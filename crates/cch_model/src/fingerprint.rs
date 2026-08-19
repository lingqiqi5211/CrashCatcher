use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{CrashKind, FINGERPRINT_FRAME_COUNT};

/// Length of a `group_id` in characters (16 bytes of SHA-256, hex-encoded).
pub const GROUP_ID_LEN: usize = 32;

/// Package name prefixes treated as framework noise.
///
/// Used for two jobs on purpose, so they can never disagree: picking the frames
/// that identify a crash site, and deciding which frames the detail screen folds
/// away by default.
const FRAMEWORK_PREFIXES: &[&str] = &[
    "android.",
    "androidx.",
    "com.android.internal.",
    "com.android.server.",
    "dalvik.",
    "java.",
    "javax.",
    "kotlin.",
    "kotlinx.coroutines.",
    "libcore.",
    "sun.",
    "art_",
];

/// True when a stack frame belongs to the platform rather than to app code.
///
/// Accepts either a raw logcat line (`\tat com.foo.Bar.baz(Bar.kt:1)`) or an
/// already-normalized frame — but the caller must have established that the line
/// *is* a frame. It answers "does this package belong to the platform", nothing
/// more: an exception header such as `java.lang.IllegalStateException: …` would
/// match on the `java.` prefix, which is correct for the callers here (they only
/// ever pass frames) and wrong for anything that walks a whole trace. The manager's
/// fold logic therefore shares this prefix list but wraps it in its own is-a-frame
/// test.
#[must_use]
pub fn is_framework_frame(frame: &str) -> bool {
    let frame = strip_java_frame_prefix(frame);
    FRAMEWORK_PREFIXES
        .iter()
        .any(|prefix| frame.starts_with(prefix))
}

fn strip_java_frame_prefix(frame: &str) -> &str {
    let frame = frame.trim();
    frame.strip_prefix("at ").unwrap_or(frame).trim_start()
}

/// Normalizes one Java stack frame so cosmetic churn does not split a group.
///
/// Drops the `at ` prefix and the `:line` suffix inside the source-location
/// parentheses. The line number is exactly the part that shifts when the app is
/// recompiled, while the method and file still identify the site.
#[must_use]
pub fn normalize_java_frame(frame: &str) -> String {
    let frame = strip_java_frame_prefix(frame);
    let Some(open) = frame.rfind('(') else {
        return frame.to_owned();
    };
    let Some(close) = frame[open..].find(')').map(|offset| open + offset) else {
        return frame.to_owned();
    };
    let inside = &frame[open + 1..close];
    let location = inside.split(':').next().unwrap_or(inside);
    let mut out = String::with_capacity(frame.len());
    out.push_str(&frame[..=open]);
    out.push_str(location);
    out.push(')');
    out
}

/// Normalizes one native backtrace frame.
///
/// Keeps the library basename and the symbol, drops the program counter, the
/// `+offset`, and the `(BuildId: …)` trailer — all of which move between builds
/// and between runs of the same build.
#[must_use]
pub fn normalize_native_frame(frame: &str) -> String {
    let frame = frame.trim();
    let mut library = String::new();
    let mut symbol = String::new();

    for token in frame.split_whitespace() {
        if token.contains('/') && (token.contains(".so") || token.contains(".apk")) {
            let basename = token.rsplit('/').next().unwrap_or(token);
            library = basename.to_owned();
        }
    }

    if let Some(open) = frame.find('(')
        && let Some(close) = frame[open..].find(')').map(|offset| open + offset)
    {
        let inside = &frame[open + 1..close];
        if !inside.starts_with("BuildId:") {
            symbol = inside.split('+').next().unwrap_or(inside).to_owned();
        }
    }

    match (library.is_empty(), symbol.is_empty()) {
        (true, true) => frame.to_owned(),
        (false, true) => library,
        (true, false) => symbol,
        (false, false) => format!("{library} ({symbol})"),
    }
}

/// The inputs that decide which group an occurrence belongs to.
///
/// Collectors fill this in; [`GroupKey::group_id`] turns it into the stable id
/// the store and the UI use. Grouping by content rather than by pid is what
/// keeps a record from being attributed to an app that merely reused the pid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub kind: CrashKind,
    /// Exception class for Java/WTF, signal name for native, ANR reason otherwise.
    pub primary: String,
    /// Leading frames, already normalized and framework-filtered.
    pub frames: Vec<String>,
}

impl Fingerprint {
    /// Builds a fingerprint from raw frames, applying the normalization and
    /// framework filtering appropriate to `kind`.
    ///
    /// When every frame is framework code the filter is skipped rather than
    /// yielding an empty fingerprint — a crash entirely inside the platform still
    /// needs to group by *something*.
    #[must_use]
    pub fn from_raw_frames(kind: CrashKind, primary: impl Into<String>, raw: &[String]) -> Self {
        let normalize: fn(&str) -> String = match kind {
            CrashKind::NativeCrash => normalize_native_frame,
            CrashKind::JavaException | CrashKind::Anr | CrashKind::Wtf => normalize_java_frame,
        };

        let normalized: Vec<String> = raw.iter().map(|frame| normalize(frame)).collect();
        let app_frames: Vec<String> = normalized
            .iter()
            .filter(|frame| !is_framework_frame(frame))
            .take(FINGERPRINT_FRAME_COUNT)
            .cloned()
            .collect();

        let frames = if app_frames.is_empty() {
            normalized
                .into_iter()
                .take(FINGERPRINT_FRAME_COUNT)
                .collect()
        } else {
            app_frames
        };

        Self {
            kind,
            primary: primary.into(),
            frames,
        }
    }
}

/// Everything that distinguishes one crash group from another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupKey<'a> {
    pub package_name: &'a str,
    pub process_name: &'a str,
    pub user_id: i32,
    pub fingerprint: &'a Fingerprint,
}

impl GroupKey<'_> {
    /// Stable 32-character hex id for this group.
    ///
    /// Every field is length-prefixed rather than separated by a delimiter. A
    /// delimiter is not injective: with a `\u{1f}` separator, a `primary` of
    /// `"a\u{1f}b"` and no frames hashes exactly the same bytes as a `primary` of
    /// `"a"` with one frame `"b"`, silently merging two different crashes into one
    /// group. Length prefixes make the encoding unambiguous.
    #[must_use]
    pub fn group_id(&self) -> String {
        let mut hasher = Sha256::new();
        let mut feed = |value: &str| {
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value.as_bytes());
        };

        feed(self.package_name);
        feed(self.process_name);
        feed(&self.user_id.to_string());
        feed(&self.fingerprint.kind.as_i64().to_string());
        feed(&self.fingerprint.primary);
        feed(&self.fingerprint.frames.len().to_string());
        for frame in &self.fingerprint.frames {
            feed(frame);
        }

        let digest = hasher.finalize();
        hex::encode(&digest[..GROUP_ID_LEN / 2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn java_frame_normalization_drops_line_number() {
        assert_eq!(
            normalize_java_frame("\tat com.example.app.MainActivity.onCreate(MainActivity.kt:37)"),
            "com.example.app.MainActivity.onCreate(MainActivity.kt)"
        );
    }

    #[test]
    fn java_frame_normalization_tolerates_missing_location() {
        assert_eq!(
            normalize_java_frame("at com.example.Foo.bar(Unknown Source)"),
            "com.example.Foo.bar(Unknown Source)"
        );
        assert_eq!(normalize_java_frame("at com.example.Foo.bar"), "com.example.Foo.bar");
        assert_eq!(normalize_java_frame("garbage ((("), "garbage (((");
    }

    #[test]
    fn native_frame_normalization_drops_pc_offset_and_build_id() {
        let raw = "      #00 pc 0000000000001ac4  /data/app/~~AbC==/com.example.app-xY==/lib/arm64/libnative.so (Java_com_example_NativeLib_processData+132) (BuildId: 1f2e3d)";
        assert_eq!(
            normalize_native_frame(raw),
            "libnative.so (Java_com_example_NativeLib_processData)"
        );
    }

    #[test]
    fn native_frame_normalization_without_symbol_keeps_library() {
        let raw = "      #01 pc 000000000031f0d8  /apex/com.android.art/lib64/libart.so";
        assert_eq!(normalize_native_frame(raw), "libart.so");
    }

    #[test]
    fn native_frame_normalization_ignores_build_id_only_parens() {
        let raw = "      #02 pc 0000000000012345  /system/lib64/libc.so (BuildId: abcdef)";
        assert_eq!(normalize_native_frame(raw), "libc.so");
    }

    #[test]
    fn framework_frames_are_recognized_before_and_after_normalization() {
        assert!(is_framework_frame("\tat android.app.Activity.performCreate(Activity.java:8595)"));
        assert!(is_framework_frame("java.lang.Thread.run(Thread.java)"));
        assert!(!is_framework_frame("at com.example.app.Repo.load(Repo.kt:88)"));
    }

    #[test]
    fn fingerprint_keeps_app_frames_and_skips_framework_noise() {
        let fingerprint = Fingerprint::from_raw_frames(
            CrashKind::JavaException,
            "java.lang.IllegalStateException",
            &frames(&[
                "at android.app.Activity.performCreate(Activity.java:8595)",
                "at com.example.app.MainActivity.onCreate(MainActivity.kt:37)",
                "at com.example.app.Repo.load(Repo.kt:88)",
            ]),
        );
        assert_eq!(
            fingerprint.frames,
            frames(&[
                "com.example.app.MainActivity.onCreate(MainActivity.kt)",
                "com.example.app.Repo.load(Repo.kt)",
            ])
        );
    }

    #[test]
    fn fingerprint_falls_back_when_every_frame_is_framework() {
        let fingerprint = Fingerprint::from_raw_frames(
            CrashKind::JavaException,
            "java.lang.NullPointerException",
            &frames(&[
                "at android.app.Activity.performCreate(Activity.java:8595)",
                "at android.os.Looper.loop(Looper.java:288)",
            ]),
        );
        assert_eq!(fingerprint.frames.len(), 2);
    }

    #[test]
    fn fingerprint_is_capped_at_the_configured_frame_count() {
        let raw: Vec<String> = (0..20)
            .map(|index| format!("at com.example.app.Deep{index}.call(Deep.kt:{index})"))
            .collect();
        let fingerprint =
            Fingerprint::from_raw_frames(CrashKind::JavaException, "java.lang.Error", &raw);
        assert_eq!(fingerprint.frames.len(), FINGERPRINT_FRAME_COUNT);
    }

    #[test]
    fn line_number_churn_does_not_split_a_group() {
        let before = Fingerprint::from_raw_frames(
            CrashKind::JavaException,
            "java.lang.IllegalStateException",
            &frames(&["at com.example.app.MainActivity.onCreate(MainActivity.kt:37)"]),
        );
        let after = Fingerprint::from_raw_frames(
            CrashKind::JavaException,
            "java.lang.IllegalStateException",
            &frames(&["at com.example.app.MainActivity.onCreate(MainActivity.kt:52)"]),
        );

        let key = |fingerprint: &Fingerprint| {
            GroupKey {
                package_name: "com.example.app",
                process_name: "com.example.app",
                user_id: 0,
                fingerprint,
            }
            .group_id()
        };

        assert_eq!(key(&before), key(&after));
    }

    #[test]
    fn group_id_is_stable_length_and_separates_users_and_processes() {
        let fingerprint = Fingerprint::from_raw_frames(
            CrashKind::JavaException,
            "java.lang.IllegalStateException",
            &frames(&["at com.example.app.MainActivity.onCreate(MainActivity.kt:37)"]),
        );
        let base = GroupKey {
            package_name: "com.example.app",
            process_name: "com.example.app",
            user_id: 0,
            fingerprint: &fingerprint,
        };
        let other_user = GroupKey { user_id: 10, ..base.clone() };
        let other_process = GroupKey {
            process_name: "com.example.app:remote",
            ..base.clone()
        };

        assert_eq!(base.group_id().len(), GROUP_ID_LEN);
        assert_ne!(base.group_id(), other_user.group_id());
        assert_ne!(base.group_id(), other_process.group_id());
    }

    #[test]
    fn separator_cannot_be_forged_by_field_contents() {
        let left = Fingerprint::from_raw_frames(CrashKind::Wtf, "a\u{1f}b", &[]);
        let right = Fingerprint::from_raw_frames(CrashKind::Wtf, "a", &frames(&["b"]));
        let key = |fingerprint: &Fingerprint| {
            GroupKey {
                package_name: "p",
                process_name: "p",
                user_id: 0,
                fingerprint,
            }
            .group_id()
        };
        assert_ne!(key(&left), key(&right));
    }
}
