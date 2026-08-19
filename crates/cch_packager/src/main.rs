use std::{
    env, fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
};

use cch_auth::ManagerPin;
use clap::{Args, Parser, Subcommand, ValueEnum};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use thiserror::Error;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const DEFAULT_API: u32 = 29;

#[derive(Debug, Parser)]
#[command(name = "cch-packager", version, about = "Build CrashCatcher artifacts")]
struct Cli {
    #[command(subcommand)]
    command: PackageCommand,
}

#[derive(Debug, Subcommand)]
enum PackageCommand {
    /// Builds the three daemon ABIs, bridge DEX and root-module zip.
    Module(ModuleArgs),
    /// Builds and copies the Kotlin + Compose manager APK independently.
    ManagerApk(ManagerApkArgs),
    /// Compiles only the app_process Java bridge DEX.
    Bridge(BridgeArgs),
}

#[derive(Debug, Args)]
struct ModuleArgs {
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    #[arg(long, default_value = "module")]
    template: PathBuf,
    #[arg(long, default_value = "dist/crashcatcher-module.zip")]
    output: PathBuf,
    #[arg(long)]
    manager_apk: Option<PathBuf>,
    #[arg(long, conflicts_with = "manager_apk")]
    manager_cert_sha256: Option<String>,
    #[arg(long)]
    android_sdk: Option<PathBuf>,
    #[arg(long)]
    android_ndk: Option<PathBuf>,
    #[arg(long, default_value_t = DEFAULT_API)]
    api: u32,
    #[arg(long, value_enum, value_delimiter = ',', default_values_t = Abi::ALL)]
    abi: Vec<Abi>,
}

#[derive(Debug, Args)]
struct ManagerApkArgs {
    #[arg(long, default_value = "apps/manager")]
    project: PathBuf,
    #[arg(long, default_value = "dist/crashcatcher.apk")]
    output: PathBuf,
    #[arg(long, default_value = "release")]
    variant: String,
}

#[derive(Debug, Args)]
struct BridgeArgs {
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    #[arg(long)]
    android_sdk: Option<PathBuf>,
    #[arg(long, default_value_t = DEFAULT_API)]
    min_api: u32,
    #[arg(long, default_value = "dist/cch_bridge.dex")]
    output: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Abi {
    Arm64V8a,
    ArmeabiV7a,
    X86_64,
}

impl Abi {
    const ALL: [Self; 3] = [Self::Arm64V8a, Self::ArmeabiV7a, Self::X86_64];

    const fn android_name(self) -> &'static str {
        match self {
            Self::Arm64V8a => "arm64-v8a",
            Self::ArmeabiV7a => "armeabi-v7a",
            Self::X86_64 => "x86_64",
        }
    }

    const fn rust_target(self) -> &'static str {
        match self {
            Self::Arm64V8a => "aarch64-linux-android",
            Self::ArmeabiV7a => "armv7-linux-androideabi",
            Self::X86_64 => "x86_64-linux-android",
        }
    }

    const fn clang_prefix(self) -> &'static str {
        match self {
            Self::Arm64V8a => "aarch64-linux-android",
            Self::ArmeabiV7a => "armv7a-linux-androideabi",
            Self::X86_64 => "x86_64-linux-android",
        }
    }
}

fn main() -> Result<(), PackagerError> {
    match Cli::parse().command {
        PackageCommand::Module(arguments) => build_module(arguments),
        PackageCommand::ManagerApk(arguments) => build_manager_apk(arguments),
        PackageCommand::Bridge(arguments) => {
            let sdk = android_sdk(arguments.android_sdk.as_deref())?;
            compile_bridge(
                &arguments.workspace,
                &sdk,
                arguments.min_api,
                &arguments.output,
            )
        }
    }
}

fn build_module(arguments: ModuleArgs) -> Result<(), PackagerError> {
    if arguments.abi.is_empty() {
        return Err(PackagerError::Arguments(
            "at least one ABI is required".to_owned(),
        ));
    }
    let workspace = canonical(&arguments.workspace)?;
    let template = resolve(&workspace, &arguments.template);
    let output = resolve(&workspace, &arguments.output);
    let sdk = android_sdk(arguments.android_sdk.as_deref())?;
    let ndk = android_ndk(arguments.android_ndk.as_deref(), &sdk)?;
    let pin = manager_pin(
        arguments
            .manager_apk
            .as_deref()
            .map(|path| resolve(&workspace, path)),
        arguments.manager_cert_sha256.as_deref(),
    )?;

    let staging = TempDir::new()?;
    copy_tree(&template, staging.path())?;
    stamp_version(&workspace, &staging.path().join("module.prop"))?;
    let config = staging.path().join("config");
    fs::create_dir_all(&config)?;
    fs::write(
        config.join("manager_signing_cert.sha256"),
        format!("{pin}\n"),
    )?;

    let dex = staging.path().join("dex/cch_bridge.dex");
    compile_bridge(&workspace, &sdk, arguments.api, &dex)?;
    fs::write(
        staging.path().join("dex/cch_bridge.dex.sha256"),
        format!("{}  cch_bridge.dex\n", sha256_file(&dex)?),
    )?;

    let binaries = build_daemons(&workspace, &ndk, arguments.api, &arguments.abi)?;
    for (abi, binary) in binaries {
        let destination = staging
            .path()
            .join("bin")
            .join(abi.android_name())
            .join("catcherd");
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(binary, destination)?;
    }
    create_zip(staging.path(), &output)?;
    println!("module: {}", output.display());
    Ok(())
}

fn build_manager_apk(arguments: ManagerApkArgs) -> Result<(), PackagerError> {
    let project = canonical(&arguments.project)?;
    let variant = arguments.variant.trim();
    if variant.is_empty() || !variant.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(PackagerError::Arguments(
            "variant must be alphanumeric".to_owned(),
        ));
    }
    let task = format!(":app:assemble{}", upper_first(variant));
    let wrapper = if cfg!(windows) {
        project.join("gradlew.bat")
    } else {
        project.join("gradlew")
    };
    run(Command::new(wrapper).current_dir(&project).arg(task))?;

    let apk_dir = project.join("app/build/outputs/apk").join(variant);
    let apk = newest_file_with_extension(&apk_dir, "apk")?
        .ok_or_else(|| PackagerError::MissingArtifact(apk_dir.clone()))?;
    let output = if arguments.output.is_absolute() {
        arguments.output
    } else {
        env::current_dir()?.join(arguments.output)
    };
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(apk, &output)?;
    println!("manager APK: {}", output.display());
    Ok(())
}

fn build_daemons(
    workspace: &Path,
    ndk: &Path,
    api: u32,
    abis: &[Abi],
) -> Result<Vec<(Abi, PathBuf)>, PackagerError> {
    let toolchain = ndk
        .join("toolchains/llvm/prebuilt")
        .join(ndk_host_tag())
        .join("bin");
    let target_dir = workspace.join("target");
    thread::scope(|scope| {
        abis.iter()
            .copied()
            .map(|abi| {
                let toolchain = toolchain.clone();
                let workspace = workspace.to_path_buf();
                let target_dir = target_dir.clone();
                scope.spawn(move || {
                    let clang =
                        toolchain.join(ndk_tool(&format!("{}{}-clang", abi.clang_prefix(), api)));
                    if !clang.is_file() {
                        return Err(PackagerError::MissingArtifact(clang));
                    }
                    let linker_variable = format!(
                        "CARGO_TARGET_{}_LINKER",
                        abi.rust_target().replace('-', "_").to_ascii_uppercase()
                    );
                    let cc_variable = format!("CC_{}", abi.rust_target().replace('-', "_"));
                    let ar_variable = format!("AR_{}", abi.rust_target().replace('-', "_"));
                    let mut command = Command::new("cargo");
                    command
                        .current_dir(&workspace)
                        .env(linker_variable, &clang)
                        .env(cc_variable, &clang)
                        .env(ar_variable, toolchain.join(llvm_tool("llvm-ar")))
                        .arg("build")
                        .arg("--release")
                        .arg("--locked")
                        .arg("--package")
                        .arg("cch_daemond")
                        .arg("--bin")
                        .arg("catcherd")
                        .arg("--target")
                        .arg(abi.rust_target());
                    run(&mut command)?;
                    let binary = target_dir
                        .join(abi.rust_target())
                        .join("release")
                        .join("catcherd");
                    if !binary.is_file() {
                        return Err(PackagerError::MissingArtifact(binary));
                    }
                    Ok((abi, binary))
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().map_err(|_| PackagerError::Thread)?)
            .collect::<Result<Vec<_>, _>>()
    })
}

fn compile_bridge(
    workspace: &Path,
    sdk: &Path,
    min_api: u32,
    output: &Path,
) -> Result<(), PackagerError> {
    let android_jar = newest_android_jar(sdk)?;
    let d8 = newest_build_tool(sdk, "d8")?;
    let temporary = TempDir::new()?;
    let classes = temporary.path().join("classes");
    let dex_output = temporary.path().join("dex");
    fs::create_dir_all(&classes)?;
    fs::create_dir_all(&dex_output)?;
    let sources = files_with_extension(&workspace.join("bridge/src"), "java")?;
    if sources.is_empty() {
        return Err(PackagerError::MissingArtifact(workspace.join("bridge/src")));
    }
    let mut javac = Command::new("javac");
    javac
        .arg("-encoding")
        .arg("UTF-8")
        .arg("--release")
        .arg("21")
        .arg("-classpath")
        .arg(android_jar)
        .arg("-d")
        .arg(&classes)
        .args(sources);
    run(&mut javac)?;

    let class_files = files_with_extension(&classes, "class")?;
    let mut d8_command = Command::new(d8);
    d8_command
        .arg("--min-api")
        .arg(min_api.to_string())
        .arg("--output")
        .arg(&dex_output)
        .args(class_files);
    run(&mut d8_command)?;
    let dex = dex_output.join("classes.dex");
    if !dex.is_file() {
        return Err(PackagerError::MissingArtifact(dex));
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(dex, output)?;
    println!("bridge DEX: {}", output.display());
    Ok(())
}

fn manager_pin(apk: Option<PathBuf>, explicit: Option<&str>) -> Result<String, PackagerError> {
    if let Some(apk) = apk {
        let digest = cch_apk_sig::certificate_sha256(&apk)?
            .into_iter()
            .next()
            .ok_or_else(|| {
                PackagerError::Arguments("manager APK has no v2/v3 signer".to_owned())
            })?;
        return Ok(hex::encode(digest));
    }
    let explicit = explicit.ok_or_else(|| {
        PackagerError::Arguments(
            "provide --manager-apk or --manager-cert-sha256 for module authentication".to_owned(),
        )
    })?;
    let parsed = ManagerPin::parse(explicit)?;
    Ok(hex::encode(parsed.digest()))
}

fn android_sdk(explicit: Option<&Path>) -> Result<PathBuf, PackagerError> {
    explicit
        .map(Path::to_path_buf)
        .or_else(|| env::var_os("ANDROID_SDK_ROOT").map(PathBuf::from))
        .or_else(|| env::var_os("ANDROID_HOME").map(PathBuf::from))
        .filter(|path| path.is_dir())
        .ok_or_else(|| PackagerError::Arguments("Android SDK path was not found".to_owned()))
}

fn android_ndk(explicit: Option<&Path>, sdk: &Path) -> Result<PathBuf, PackagerError> {
    if let Some(path) = explicit
        .map(Path::to_path_buf)
        .or_else(|| env::var_os("ANDROID_NDK_HOME").map(PathBuf::from))
        .or_else(|| env::var_os("ANDROID_NDK_ROOT").map(PathBuf::from))
        .filter(|path| path.is_dir())
    {
        return Ok(path);
    }
    newest_directory(&sdk.join("ndk"))?
        .ok_or_else(|| PackagerError::Arguments("Android NDK path was not found".to_owned()))
}

fn newest_android_jar(sdk: &Path) -> Result<PathBuf, PackagerError> {
    let platform = newest_directory(&sdk.join("platforms"))?
        .ok_or_else(|| PackagerError::MissingArtifact(sdk.join("platforms")))?;
    let jar = platform.join("android.jar");
    if jar.is_file() {
        Ok(jar)
    } else {
        Err(PackagerError::MissingArtifact(jar))
    }
}

fn newest_build_tool(sdk: &Path, name: &str) -> Result<PathBuf, PackagerError> {
    let directory = newest_directory(&sdk.join("build-tools"))?
        .ok_or_else(|| PackagerError::MissingArtifact(sdk.join("build-tools")))?;
    let tool = directory.join(android_build_tool(name));
    if tool.is_file() {
        Ok(tool)
    } else {
        Err(PackagerError::MissingArtifact(tool))
    }
}

fn newest_directory(root: &Path) -> Result<Option<PathBuf>, PackagerError> {
    if !root.is_dir() {
        return Ok(None);
    }
    let mut directories = fs::read_dir(root)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    directories.sort();
    Ok(directories.pop())
}

fn create_zip(root: &Path, output: &Path) -> Result<(), PackagerError> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::File::create(output)?;
    let mut writer = ZipWriter::new(file);
    let mut files = all_files(root)?;
    files.sort();
    for path in files {
        if path == output {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| PackagerError::UnsafePath(path.clone()))?;
        let name = relative.to_string_lossy().replace('\\', "/");
        if name.starts_with('/') || name.split('/').any(|part| part == "..") {
            return Err(PackagerError::UnsafePath(relative.to_path_buf()));
        }
        let executable = name.ends_with(".sh") || name.ends_with("/catcherd");
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(if executable { 0o755 } else { 0o644 });
        writer.start_file(name, options)?;
        let mut input = fs::File::open(path)?;
        io::copy(&mut input, &mut writer)?;
    }
    writer.finish()?.sync_all()?;
    Ok(())
}

/// Writes the repository's version into the staged `module.prop`.
///
/// The template carries whatever was last committed, which is one more place for the module
/// and the manager to disagree — and a root manager showing 0.1.0 next to an about page
/// showing 0.2.0 gives the user no way to tell which half is stale. Both halves read
/// `version.properties` for the name, and both derive `versionCode` from the commit count,
/// so the two numbers are the same number.
fn stamp_version(workspace: &Path, module_prop: &Path) -> Result<(), PackagerError> {
    let version = repository_version(workspace)?;
    let code = commit_count(workspace);

    let original = fs::read_to_string(module_prop)?;
    let rewritten: String = original
        .lines()
        .map(|line| match line.split_once('=') {
            Some(("version", _)) => format!("version={version}"),
            Some(("versionCode", _)) => format!("versionCode={code}"),
            _ => line.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(module_prop, format!("{rewritten}\n"))?;
    Ok(())
}

/// The commit count, which is what both halves use as `versionCode`.
///
/// Falls back to 1 when git cannot answer — a source tarball with no history, say. **A
/// shallow clone answers 1 too**, which is why CI checks out with full history; without it
/// every build would ship as the first one.
fn commit_count(workspace: &Path) -> u32 {
    Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(workspace)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(1)
}

/// `version` from the repository's `version.properties`.
fn repository_version(workspace: &Path) -> Result<String, PackagerError> {
    let path = workspace.join("version.properties");
    let text = fs::read_to_string(&path)?;
    text.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == "version")
        .map(|(_, value)| value.trim().to_owned())
        .ok_or_else(|| PackagerError::Arguments(format!("no `version` in {}", path.display())))
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), PackagerError> {
    if !source.is_dir() {
        return Err(PackagerError::MissingArtifact(source.to_path_buf()));
    }
    for path in all_files(source)? {
        let relative = path
            .strip_prefix(source)
            .map_err(|_| PackagerError::UnsafePath(path.clone()))?;
        let target = destination.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(path, target)?;
    }
    Ok(())
}

fn all_files(root: &Path) -> Result<Vec<PathBuf>, PackagerError> {
    let mut output = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(PackagerError::UnsafePath(path));
            }
            if file_type.is_dir() {
                pending.push(path);
            } else if file_type.is_file() {
                output.push(path);
            }
        }
    }
    Ok(output)
}

fn files_with_extension(root: &Path, extension: &str) -> Result<Vec<PathBuf>, PackagerError> {
    Ok(all_files(root)?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|value| value == extension))
        .collect())
}

fn newest_file_with_extension(
    root: &Path,
    extension: &str,
) -> Result<Option<PathBuf>, PackagerError> {
    let mut candidates = files_with_extension(root, extension)?;
    candidates.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
    });
    Ok(candidates.pop())
}

fn sha256_file(path: &Path) -> Result<String, PackagerError> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let length = file.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
    }
    Ok(hex::encode(hash.finalize()))
}

fn run(command: &mut Command) -> Result<(), PackagerError> {
    let display = format!("{command:?}");
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(PackagerError::Command { display, status })
    }
}

fn canonical(path: &Path) -> Result<PathBuf, PackagerError> {
    fs::canonicalize(path).map_err(PackagerError::Io)
}

fn resolve(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn android_build_tool(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.bat")
    } else {
        name.to_owned()
    }
}

fn ndk_tool(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.cmd")
    } else {
        name.to_owned()
    }
}

fn llvm_tool(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

fn ndk_host_tag() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows-x86_64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "darwin-arm64"
    } else if cfg!(target_os = "macos") {
        "darwin-x86_64"
    } else {
        "linux-x86_64"
    }
}

fn upper_first(value: &str) -> String {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

#[derive(Debug, Error)]
enum PackagerError {
    #[error("invalid arguments: {0}")]
    Arguments(String),
    #[error("io failed: {0}")]
    Io(#[from] io::Error),
    #[error("command {display} failed with {status}")]
    Command {
        display: String,
        status: std::process::ExitStatus,
    },
    #[error("missing build artifact: {0}")]
    MissingArtifact(PathBuf),
    #[error("refusing unsafe path: {0}")]
    UnsafePath(PathBuf),
    #[error("APK signature failed: {0}")]
    Apk(#[from] cch_apk_sig::ApkSigError),
    #[error("manager pin failed: {0}")]
    Pin(#[from] cch_auth::AuthError),
    #[error("zip failed: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("parallel build thread failed")]
    Thread,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_names_match_android_and_rust() {
        assert_eq!(Abi::Arm64V8a.android_name(), "arm64-v8a");
        assert_eq!(Abi::Arm64V8a.rust_target(), "aarch64-linux-android");
        assert_eq!(Abi::ArmeabiV7a.clang_prefix(), "armv7a-linux-androideabi");
    }

    #[test]
    fn explicit_manager_pin_is_normalized() {
        let value = "AB".repeat(32);
        assert_eq!(
            manager_pin(None, Some(&value)).expect("pin"),
            "ab".repeat(32)
        );
    }

    #[test]
    fn zip_preserves_module_relative_paths() {
        let directory = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(directory.path().join("service.d")).expect("directory");
        fs::write(
            directory.path().join("service.d/test.sh"),
            "#!/system/bin/sh\n",
        )
        .expect("script");
        let output = directory.path().join("module.zip");
        create_zip(directory.path(), &output).expect("zip");
        let archive = zip::ZipArchive::new(fs::File::open(output).expect("open")).expect("archive");
        assert!(archive.file_names().any(|name| name == "service.d/test.sh"));
    }
}
