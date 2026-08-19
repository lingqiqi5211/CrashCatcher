use std::{fmt, str::FromStr};

use cch_model::CrashKind;
use serde::{Deserialize, Serialize};

use crate::WireError;

/// Page size used when a client omits `limit` or sends `0`.
pub const DEFAULT_PAGE_LIMIT: u32 = 50;

/// Hard ceiling on a page.
///
/// Enforced by the daemon, not trusted from the client: an unbounded page is how
/// the reference implementation ended up shipping its whole history in one
/// message and blowing the transport limit.
pub const MAX_PAGE_LIMIT: u32 = 200;

/// Which groups or records a query is asking for.
///
/// Every field defaults to "no restriction", and an empty list means *all* — not
/// *none*. Expressing "nothing" through an empty list would make an
/// unintentionally-empty UI filter silently return zero rows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrashFilter {
    pub packages: Vec<String>,
    pub kinds: Vec<CrashKind>,
    pub user_ids: Vec<i32>,
    /// Inclusive lower bound of the half-open range `[from, to)`, in ms.
    pub time_from_ms: Option<i64>,
    /// Exclusive upper bound, in ms.
    pub time_to_ms: Option<i64>,
    pub include_system_apps: bool,
    pub only_main_process: bool,
    /// Only crashes the app swallowed itself — the class of event the reference
    /// implementation cannot see at all.
    pub only_self_handled: bool,
    /// Substring match against package name, summary class and summary text.
    ///
    /// Deliberately *not* a full-text search over stack traces: that needs an
    /// FTS5 index roughly the size of the corpus.
    pub query: Option<String>,
}

impl CrashFilter {
    /// Rejects a filter that cannot match anything by construction.
    pub fn validate(&self) -> Result<(), WireError> {
        if let (Some(from), Some(to)) = (self.time_from_ms, self.time_to_ms)
            && from > to
        {
            return Err(WireError::invalid_request(format!(
                "time_from_ms ({from}) is after time_to_ms ({to})"
            )));
        }
        Ok(())
    }

    /// True when nothing is restricted, so the store can skip building predicates.
    #[must_use]
    pub fn is_unrestricted(&self) -> bool {
        self.packages.is_empty()
            && self.kinds.is_empty()
            && self.user_ids.is_empty()
            && self.time_from_ms.is_none()
            && self.time_to_ms.is_none()
            && self.include_system_apps
            && !self.only_main_process
            && !self.only_self_handled
            && self.query.as_deref().unwrap_or("").is_empty()
    }
}

/// Ordering for a page of groups.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortKey {
    #[default]
    LastSeenDesc,
    FirstSeenDesc,
    OccurrenceDesc,
    PackageAsc,
}

impl SortKey {
    /// Whether the anchor for this ordering is text rather than an integer.
    ///
    /// Shared with the store so a cursor can never be built with an anchor of the
    /// wrong type for its column.
    #[must_use]
    pub const fn anchor_is_text(self) -> bool {
        matches!(self, Self::PackageAsc)
    }

    /// Whether the ordering runs descending.
    #[must_use]
    pub const fn is_descending(self) -> bool {
        matches!(
            self,
            Self::LastSeenDesc | Self::FirstSeenDesc | Self::OccurrenceDesc
        )
    }
}

/// The sort-column value a cursor is positioned at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorAnchor {
    Int(i64),
    Text(String),
}

/// An opaque position in a result set.
///
/// **Clients echo this back verbatim and never construct one.** On the wire it is a
/// single string, which is what makes that contract enforceable: there is no field
/// structure for a client to be tempted to build by hand, and no cross-language
/// mapping of a tagged enum to get subtly wrong. The daemon validates that `sort`
/// still matches the request and answers [`ErrorCode::CursorInvalidated`] otherwise.
///
/// Keyset rather than offset: `OFFSET` re-walks every skipped row, so deep
/// scrolling degrades linearly. Anchoring on `(sort column, group id)` keeps each
/// page the same cost as the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub sort: SortKey,
    pub anchor: CursorAnchor,
    /// Tiebreaker for rows sharing an anchor value; a group id or a record id.
    pub tiebreak: String,
}

/// Separator for the cursor's text encoding.
///
/// Safe because no field can contain it: the sort key is from a fixed set, the
/// anchor is either a decimal integer or a package name, and the tiebreak is hex or
/// Crockford base32.
const CURSOR_SEPARATOR: char = '|';
const CURSOR_VERSION: &str = "1";

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sort = match self.sort {
            SortKey::LastSeenDesc => "last_seen_desc",
            SortKey::FirstSeenDesc => "first_seen_desc",
            SortKey::OccurrenceDesc => "occurrence_desc",
            SortKey::PackageAsc => "package_asc",
        };
        let (kind, anchor) = match &self.anchor {
            CursorAnchor::Int(value) => ("i", value.to_string()),
            CursorAnchor::Text(value) => ("t", value.clone()),
        };
        write!(
            f,
            "{CURSOR_VERSION}{sep}{sort}{sep}{kind}{sep}{anchor}{sep}{tiebreak}",
            sep = CURSOR_SEPARATOR,
            tiebreak = self.tiebreak,
        )
    }
}

impl FromStr for Cursor {
    type Err = WireError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = || WireError::cursor_invalidated("cursor is not in a recognised format");

        let mut parts = value.splitn(5, CURSOR_SEPARATOR);
        let version = parts.next().ok_or_else(invalid)?;
        if version != CURSOR_VERSION {
            return Err(WireError::cursor_invalidated(format!(
                "cursor version {version} is not supported"
            )));
        }

        let sort = match parts.next().ok_or_else(invalid)? {
            "last_seen_desc" => SortKey::LastSeenDesc,
            "first_seen_desc" => SortKey::FirstSeenDesc,
            "occurrence_desc" => SortKey::OccurrenceDesc,
            "package_asc" => SortKey::PackageAsc,
            _ => return Err(invalid()),
        };
        let kind = parts.next().ok_or_else(invalid)?;
        let anchor_text = parts.next().ok_or_else(invalid)?;
        let tiebreak = parts.next().ok_or_else(invalid)?;

        let anchor = match kind {
            "i" => CursorAnchor::Int(anchor_text.parse::<i64>().map_err(|_| invalid())?),
            "t" => CursorAnchor::Text(anchor_text.to_owned()),
            _ => return Err(invalid()),
        };

        Ok(Self {
            sort,
            anchor,
            tiebreak: tiebreak.to_owned(),
        })
    }
}

impl Serialize for Cursor {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Cursor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl Cursor {
    #[must_use]
    pub fn new(sort: SortKey, anchor: CursorAnchor, tiebreak: impl Into<String>) -> Self {
        Self {
            sort,
            anchor,
            tiebreak: tiebreak.into(),
        }
    }

    /// Confirms the cursor belongs to `sort` and carries a well-typed anchor.
    pub fn validate_for(&self, sort: SortKey) -> Result<(), WireError> {
        if self.sort != sort {
            return Err(WireError::cursor_invalidated(format!(
                "cursor was issued for {:?} but the request sorts by {sort:?}",
                self.sort
            )));
        }
        let matches_type = match self.anchor {
            CursorAnchor::Text(_) => sort.anchor_is_text(),
            CursorAnchor::Int(_) => !sort.anchor_is_text(),
        };
        if !matches_type {
            return Err(WireError::cursor_invalidated(
                "cursor anchor type does not match the sort column",
            ));
        }
        Ok(())
    }
}

/// One page request: what to match, how to order, where to resume, how many.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PageRequest {
    pub filter: CrashFilter,
    pub sort: SortKey,
    pub cursor: Option<Cursor>,
    /// `0` means "use the default". Clamped to [`MAX_PAGE_LIMIT`] server-side.
    pub limit: u32,
}

impl PageRequest {
    /// The page size actually used, after defaulting and clamping.
    #[must_use]
    pub const fn effective_limit(&self) -> u32 {
        if self.limit == 0 {
            DEFAULT_PAGE_LIMIT
        } else if self.limit > MAX_PAGE_LIMIT {
            MAX_PAGE_LIMIT
        } else {
            self.limit
        }
    }

    /// Full server-side check: filter sanity plus cursor/sort agreement.
    pub fn validate(&self) -> Result<(), WireError> {
        self.filter.validate()?;
        if let Some(cursor) = &self.cursor {
            cursor.validate_for(self.sort)?;
        }
        Ok(())
    }
}

/// A page of results plus the cursor that continues it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// `None` when the result set is exhausted.
    pub next_cursor: Option<Cursor>,
}

impl<T> Page<T> {
    #[must_use]
    pub const fn new(items: Vec<T>, next_cursor: Option<Cursor>) -> Self {
        Self { items, next_cursor }
    }

    #[must_use]
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            next_cursor: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorCode;

    #[test]
    fn limit_is_defaulted_and_clamped() {
        let with_zero = PageRequest::default();
        assert_eq!(with_zero.effective_limit(), DEFAULT_PAGE_LIMIT);

        let oversized = PageRequest {
            limit: 100_000,
            ..PageRequest::default()
        };
        assert_eq!(oversized.effective_limit(), MAX_PAGE_LIMIT);

        let reasonable = PageRequest {
            limit: 25,
            ..PageRequest::default()
        };
        assert_eq!(reasonable.effective_limit(), 25);
    }

    #[test]
    fn an_inverted_time_range_is_rejected() {
        let filter = CrashFilter {
            time_from_ms: Some(200),
            time_to_ms: Some(100),
            ..CrashFilter::default()
        };
        assert_eq!(
            filter.validate().map(|_| ()).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn an_open_ended_time_range_is_fine() {
        let filter = CrashFilter {
            time_from_ms: Some(200),
            ..CrashFilter::default()
        };
        assert!(filter.validate().is_ok());
    }

    #[test]
    fn a_cursor_from_another_sort_order_is_refused() {
        let cursor = Cursor::new(SortKey::LastSeenDesc, CursorAnchor::Int(10), "abc");
        assert_eq!(
            cursor
                .validate_for(SortKey::OccurrenceDesc)
                .map(|_| ())
                .unwrap_err()
                .code,
            ErrorCode::CursorInvalidated
        );
        assert!(cursor.validate_for(SortKey::LastSeenDesc).is_ok());
    }

    #[test]
    fn a_cursor_anchor_of_the_wrong_type_is_refused() {
        let text_anchor_on_int_sort =
            Cursor::new(SortKey::LastSeenDesc, CursorAnchor::Text("x".into()), "abc");
        assert_eq!(
            text_anchor_on_int_sort
                .validate_for(SortKey::LastSeenDesc)
                .map(|_| ())
                .unwrap_err()
                .code,
            ErrorCode::CursorInvalidated
        );

        let int_anchor_on_text_sort = Cursor::new(SortKey::PackageAsc, CursorAnchor::Int(1), "abc");
        assert_eq!(
            int_anchor_on_text_sort
                .validate_for(SortKey::PackageAsc)
                .map(|_| ())
                .unwrap_err()
                .code,
            ErrorCode::CursorInvalidated
        );
    }

    #[test]
    fn sort_metadata_agrees_with_the_variants() {
        assert!(SortKey::PackageAsc.anchor_is_text());
        assert!(!SortKey::PackageAsc.is_descending());
        for sort in [
            SortKey::LastSeenDesc,
            SortKey::FirstSeenDesc,
            SortKey::OccurrenceDesc,
        ] {
            assert!(!sort.anchor_is_text());
            assert!(sort.is_descending());
        }
    }

    #[test]
    fn an_empty_filter_means_everything_except_system_apps() {
        let filter = CrashFilter::default();
        // Empty lists are "no restriction"...
        assert!(filter.packages.is_empty());
        assert!(filter.kinds.is_empty());
        // ...but system apps stay hidden until asked for, so the list is not
        // drowned in platform noise on first open.
        assert!(!filter.include_system_apps);
        assert!(!filter.is_unrestricted());
    }

    #[test]
    fn a_filter_that_restricts_nothing_is_recognized() {
        let filter = CrashFilter {
            include_system_apps: true,
            ..CrashFilter::default()
        };
        assert!(filter.is_unrestricted());
    }

    #[test]
    fn omitted_fields_deserialize_to_defaults() {
        let request: PageRequest = serde_json::from_str("{}").expect("empty object is valid");
        assert_eq!(request, PageRequest::default());
        assert_eq!(request.sort, SortKey::LastSeenDesc);

        let partial: CrashFilter =
            serde_json::from_str(r#"{"packages":["com.example.app"]}"#).expect("partial is valid");
        assert_eq!(partial.packages, vec!["com.example.app".to_owned()]);
        assert!(!partial.include_system_apps);
    }

    #[test]
    fn unknown_fields_are_ignored_for_forward_compatibility() {
        let filter: CrashFilter =
            serde_json::from_str(r#"{"packages":[],"invented_by_a_newer_client":true}"#)
                .expect("unknown fields must not break an older daemon");
        assert!(filter.packages.is_empty());
    }

    #[test]
    fn cursors_round_trip_through_json() {
        for cursor in [
            Cursor::new(SortKey::LastSeenDesc, CursorAnchor::Int(-5), "g1"),
            Cursor::new(
                SortKey::PackageAsc,
                CursorAnchor::Text("com.a".into()),
                "g2",
            ),
            Cursor::new(SortKey::OccurrenceDesc, CursorAnchor::Int(0), "g3"),
        ] {
            let json = serde_json::to_string(&cursor).expect("serializes");
            let parsed: Cursor = serde_json::from_str(&json).expect("deserializes");
            assert_eq!(parsed, cursor);
        }
    }

    #[test]
    fn a_cursor_is_a_single_opaque_string_on_the_wire() {
        let cursor = Cursor::new(
            SortKey::LastSeenDesc,
            CursorAnchor::Int(1_755_440_000_123),
            "abc",
        );
        assert_eq!(
            serde_json::to_string(&cursor).expect("serializes"),
            r#""1|last_seen_desc|i|1755440000123|abc""#
        );
    }

    #[test]
    fn a_package_anchor_survives_dots_in_the_name() {
        let cursor = Cursor::new(
            SortKey::PackageAsc,
            CursorAnchor::Text("com.example.app.debug".into()),
            "0123456789abcdef0123456789abcdef",
        );
        let text = cursor.to_string();
        assert_eq!(text.parse::<Cursor>().expect("parses"), cursor);
    }

    #[test]
    fn a_malformed_cursor_is_reported_rather_than_guessed_at() {
        for text in [
            "",
            "garbage",
            "1|last_seen_desc",
            "1|invented_sort|i|1|abc",
            "1|last_seen_desc|x|1|abc",
            "1|last_seen_desc|i|not-a-number|abc",
            "2|last_seen_desc|i|1|abc",
        ] {
            let error = text.parse::<Cursor>().map(|_| ()).unwrap_err();
            assert_eq!(
                error.code,
                ErrorCode::CursorInvalidated,
                "{text:?} should be rejected as a cursor"
            );
        }
    }
}
