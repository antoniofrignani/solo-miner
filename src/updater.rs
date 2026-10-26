use anyhow::{Context, Result, bail, ensure};
use bitcoin::hashes::{Hash, sha256};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Cursor, Read},
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const REPO: &str = "https://github.com/antoniofrignani/solo-miner";
const API: &str = "https://api.github.com/repos/antoniofrignani/solo-miner/releases/latest";
const ARCHIVE: &str = "solo-miner-macos-arm64.zip";
const APP: &str = "Solo Miner.app";
const BUNDLE_ID: &str = "it.solo-miner.app";
const MAX_DOWNLOAD: u64 = 64 * 1024 * 1024;
const MAX_EXPANDED: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub page: String,
    archive: String,
    checksum: String,
    size: u64,
    digest: Option<String>,
}
pub struct Prepared {
    pub release: Release,
    staging: TempDir,
    target: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Plan {
    target: PathBuf,
    version: String,
    previous: String,
    parent_pid: u32,
    resume: bool,
    result: PathBuf,
}

pub fn check() -> Result<Option<Release>> {
    let body = fetch(API, 1024 * 1024, &mut |_, _| {}, 0)?;
    parse_release(&serde_json::from_slice(&body)?, env!("CARGO_PKG_VERSION"))
}
fn parse_release(value: &serde_json::Value, installed: &str) -> Result<Option<Release>> {
    ensure!(
        value["draft"] == false && value["prerelease"] == false,
        "No stable release available"
    );
    let tag = value["tag_name"].as_str().context("Missing release tag")?;
    let version = Version::parse(tag.strip_prefix('v').context("Invalid release tag")?)?;
    ensure!(
        version.pre.is_empty() && version.build.is_empty(),
        "Invalid stable release version"
    );
    if version.cmp_precedence(&Version::parse(installed)?).is_le() {
        return Ok(None);
    }
    let assets = value["assets"]
        .as_array()
        .context("Missing release assets")?;
    let asset = |name: &str| -> Result<&serde_json::Value> {
        let matches: Vec<_> = assets.iter().filter(|a| a["name"] == name).collect();
        ensure!(
            matches.len() == 1,
            "Release must contain exactly one {name}"
        );
        Ok(matches[0])
    };
    let url = |a: &serde_json::Value, name: &str| -> Result<String> {
        let expected = format!("{REPO}/releases/download/{tag}/{name}");
        ensure!(
            a["browser_download_url"] == expected,
            "Unexpected update download URL"
        );
        Ok(expected)
    };
    let archive = asset(ARCHIVE)?;
    let checksum_name = "solo-miner-macos-arm64.sha256";
    let checksum = asset(checksum_name)?;
    let size = archive["size"].as_u64().context("Missing archive size")?;
    ensure!(
        (1..=MAX_DOWNLOAD).contains(&size),
        "Update archive is too large"
    );
    let digest = archive["digest"]
        .as_str()
        .map(|d| d.strip_prefix("sha256:").unwrap_or("").to_owned());
    if let Some(digest) = &digest {
        validate_digest(digest)?;
    }
    Ok(Some(Release {
        version: version.to_string(),
        page: format!("{REPO}/releases/tag/{tag}"),
        archive: url(archive, ARCHIVE)?,
        checksum: url(checksum, checksum_name)?,
        size,
        digest,
    }))
}
fn validate_digest(digest: &str) -> Result<()> {
    ensure!(
        digest.len() == 64 && digest.bytes().all(|c| c.is_ascii_hexdigit()),
        "Invalid SHA-256 checksum"
    );
    Ok(())
}
fn fetch(
    url: &str,
    limit: u64,
    progress: &mut impl FnMut(u64, u64),
    total: u64,
) -> Result<Vec<u8>> {
    let mut child = Command::new("/usr/bin/curl")
        .args([
            "--disable",
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "10",
            "--max-time",
            if limit <= 1024 * 1024 { "30" } else { "180" },
            "--user-agent",
        ])
        .arg(concat!("Solo-Miner/", env!("CARGO_PKG_VERSION")))
        .args([
            "--header",
            "Accept: application/vnd.github+json",
            "--url",
            url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Start HTTPS download")?;
    let result = (|| -> Result<Vec<u8>> {
        let mut output = child.stdout.take().context("Download output unavailable")?;
        let mut bytes = Vec::new();
        let mut buffer = [0; 65536];
        loop {
            let n = output.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            ensure!(
                bytes.len() as u64 + n as u64 <= limit,
                "Download exceeds size limit"
            );
            bytes.extend_from_slice(&buffer[..n]);
            progress(bytes.len() as u64, total);
        }
        Ok(bytes)
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let mut error = String::new();
    if let Some(stderr) = child.stderr.take() {
        stderr.take(4096).read_to_string(&mut error)?;
    }
    let bytes = result?;
    ensure!(status.success(), "GitHub download failed: {}", error.trim());
    Ok(bytes)
}
pub fn current_bundle() -> Result<PathBuf> {
    let executable = std::env::current_exe()?.canonicalize()?;
    let macos = executable.parent().context("Missing executable folder")?;
    let contents = macos.parent().context("Missing Contents folder")?;
    let bundle = contents.parent().context("Missing app bundle")?;
    ensure!(
        macos.file_name().is_some_and(|n| n == "MacOS")
            && contents.file_name().is_some_and(|n| n == "Contents")
            && bundle.extension().is_some_and(|n| n == "app"),
        "Updates require an .app bundle"
    );
    Ok(bundle.to_owned())
}
pub fn prepare(release: Release, mut progress: impl FnMut(u64, u64)) -> Result<Prepared> {
    let target = current_bundle()?;
    prepare_at(release, target, &mut progress)
}
fn prepare_at(
    release: Release,
    target: PathBuf,
    progress: &mut impl FnMut(u64, u64),
) -> Result<Prepared> {
    let parent = target.parent().context("Missing app parent")?;
    let staging = tempfile::Builder::new().prefix(".solo-miner-update-").tempdir_in(parent)
        .context("App folder is not writable. Move Solo Miner to Applications or another writable folder")?;
    fs::set_permissions(staging.path(), fs::Permissions::from_mode(0o700))?;
    let checksum = fetch(&release.checksum, 4096, &mut |_, _| {}, 0)?;
    let expected = checksum_text(&checksum)?;
    if let Some(digest) = &release.digest {
        ensure!(
            digest.eq_ignore_ascii_case(&expected),
            "GitHub digest and checksum disagree"
        );
    }
    let bytes = fetch(&release.archive, release.size, progress, release.size)?;
    ensure!(
        bytes.len() as u64 == release.size,
        "Incomplete update download"
    );
    ensure!(
        sha256::Hash::hash(&bytes)
            .to_string()
            .eq_ignore_ascii_case(&expected),
        "Update checksum mismatch"
    );
    extract(&bytes, staging.path())?;
    verify_bundle(&staging.path().join(APP), &release.version)?;
    Ok(Prepared {
        release,
        staging,
        target,
    })
}
fn checksum_text(bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes)?;
    let words: Vec<_> = text.split_whitespace().collect();
    ensure!(
        words.len() == 2 && words[1].trim_start_matches('*') == ARCHIVE,
        "Unexpected checksum file"
    );
    validate_digest(words[0])?;
    Ok(words[0].to_owned())
}
fn extract(bytes: &[u8], destination: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    ensure!(archive.len() <= 2048, "Too many update archive entries");
    let mut expanded = 0u64;
    let mut seen = HashSet::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let path = entry
            .enclosed_name()
            .context("Unsafe update archive path")?;
        ensure!(
            path.components().all(|c| matches!(c, Component::Normal(_))),
            "Unsafe update archive path"
        );
        ensure!(seen.insert(path.clone()), "Duplicate update archive path");
        let first = path.components().next().context("Empty archive path")?;
        ensure!(
            first.as_os_str() == APP || first.as_os_str() == "__MACOSX",
            "Unexpected update archive root"
        );
        ensure!(
            entry.unix_mode().unwrap_or(0) & 0o170000 != 0o120000,
            "Update archive contains symlinks"
        );
        expanded = expanded
            .checked_add(entry.size())
            .context("Update size overflow")?;
        ensure!(
            expanded <= MAX_EXPANDED,
            "Expanded update exceeds size limit"
        );
        if first.as_os_str() == "__MACOSX" {
            continue;
        }
        let output = destination.join(&path);
        if entry.is_dir() {
            fs::create_dir_all(output)?;
            continue;
        }
        fs::create_dir_all(output.parent().context("Missing archive parent")?)?;
        let mut file = File::options().write(true).create_new(true).open(&output)?;
        let written = std::io::copy(&mut entry.by_ref().take(MAX_EXPANDED + 1), &mut file)?;
        ensure!(written == entry.size(), "Invalid update entry size");
        let mode = if entry.unix_mode().unwrap_or(0) & 0o111 != 0 {
            0o755
        } else {
            0o644
        };
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        file.sync_all()?;
    }
    Ok(())
}
fn plist(bundle: &Path, key: &str) -> Result<String> {
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Print :{key}")])
        .arg(bundle.join("Contents/Info.plist"))
        .output()?;
    ensure!(output.status.success(), "Invalid app metadata: {key}");
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
fn bundle_version(bundle: &Path) -> Result<String> {
    ensure!(
        plist(bundle, "CFBundleIdentifier")? == BUNDLE_ID,
        "Unexpected application identity"
    );
    ensure!(
        plist(bundle, "CFBundleExecutable")? == "solo-miner",
        "Unexpected app executable"
    );
    let version = plist(bundle, "CFBundleShortVersionString")?;
    Version::parse(&version)?;
    Ok(version)
}
fn verify_bundle(bundle: &Path, version: &str) -> Result<()> {
    ensure!(
        bundle_version(bundle)? == version,
        "Downloaded app version differs from release"
    );
    let mut header = [0; 8];
    File::open(bundle.join("Contents/MacOS/solo-miner"))?.read_exact(&mut header)?;
    ensure!(
        header[..4] == [0xcf, 0xfa, 0xed, 0xfe] && header[4..] == [12, 0, 0, 1],
        "Update is not an Apple Silicon executable"
    );
    let output = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict"])
        .arg(bundle)
        .output()?;
    ensure!(
        output.status.success(),
        "Downloaded app signature is invalid: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}
impl Prepared {
    pub fn launch(self, resume: bool, result: PathBuf) -> Result<()> {
        let previous = bundle_version(&self.target)?;
        ensure!(
            Version::parse(&self.release.version)?
                .cmp_precedence(&Version::parse(&previous)?)
                .is_gt(),
            "Update is not newer than installed app"
        );
        let plan = Plan {
            target: self.target.clone(),
            version: self.release.version.clone(),
            previous,
            parent_pid: std::process::id(),
            resume,
            result,
        };
        let plan_path = self.staging.path().join("plan.json");
        fs::write(&plan_path, serde_json::to_vec(&plan)?)?;
        let helper = self.staging.path().join("installer");
        fs::copy(std::env::current_exe()?, &helper)?;
        let log = File::create(self.staging.path().join("installer.log"))?;
        Command::new(helper)
            .arg("--apply-update")
            .arg(&plan_path)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()
            .context("Start update installer")?;
        let _ = self.staging.keep(); // The helper owns cleanup once the old app exits.
        Ok(())
    }
}
fn replace_app(
    target: &Path,
    staged: &Path,
    backup: &Path,
    activate: impl FnOnce() -> Result<()>,
) -> Result<()> {
    swap_paths(target, staged)
        .context("Atomically install update; app folder must support rename exchange")?;
    if let Err(error) = fs::rename(staged, backup) {
        swap_paths(target, staged).context("Restore previous app")?;
        return Err(error.into());
    }
    if let Err(error) = activate() {
        swap_paths(target, backup).context("Restore previous app")?;
        let _ = fs::rename(backup, staged);
        return Err(error);
    }
    Ok(())
}
fn swap_paths(a: &Path, b: &Path) -> Result<()> {
    let a = std::ffi::CString::new(a.as_os_str().as_bytes())?;
    let b = std::ffi::CString::new(b.as_os_str().as_bytes())?;
    // Both paths exist on the same filesystem. Neither app path is absent
    // during exchange, including if the helper is interrupted.
    let result = unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
fn open_app(plan: &Plan, confirmation: Option<&Path>) -> Result<()> {
    let mut command = Command::new("/usr/bin/open");
    command.arg("-n").arg(&plan.target).arg("--args");
    if plan.resume {
        command.arg("--resume-mining");
    }
    if let Some(path) = confirmation {
        command.arg("--update-confirm").arg(path);
    }
    ensure!(command.status()?.success(), "Launch updated app failed");
    Ok(())
}
pub fn apply_update(plan_path: &Path) -> Result<()> {
    let outcome = apply_update_inner(plan_path);
    if let Err(error) = &outcome
        && let Ok(bytes) = fs::read(plan_path)
        && let Ok(plan) = serde_json::from_slice::<Plan>(&bytes)
    {
        let _ = fs::write(&plan.result, format!("Update failed: {error:#}"));
        if !process_alive(plan.parent_pid)
            && bundle_version(&plan.target).is_ok_and(|v| v == plan.previous)
        {
            let _ = open_app(&plan, None);
        }
    }
    outcome
}
fn process_alive(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as i32, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}
fn apply_update_inner(plan_path: &Path) -> Result<()> {
    let plan: Plan = serde_json::from_slice(&fs::read(plan_path)?)?;
    let staging = plan_path
        .parent()
        .context("Missing update folder")?
        .canonicalize()?;
    let target = plan.target.canonicalize()?;
    ensure!(
        target == plan.target
            && staging.parent() == target.parent()
            && staging
                .file_name()
                .is_some_and(|s| s.to_string_lossy().starts_with(".solo-miner-update-")),
        "Invalid installation paths"
    );
    ensure!(
        std::env::current_exe()?.canonicalize()?.parent() == Some(staging.as_path())
            && (2..=i32::MAX as u32).contains(&plan.parent_pid)
            && plan.parent_pid != std::process::id(),
        "Invalid installer process"
    );
    let deadline = Instant::now();
    while process_alive(plan.parent_pid) {
        ensure!(
            deadline.elapsed() < Duration::from_secs(60),
            "App did not exit; update not installed"
        );
        thread::sleep(Duration::from_millis(100));
    }
    let staged = staging.join(APP);
    verify_bundle(&staged, &plan.version)?;
    ensure!(
        bundle_version(&target)? == plan.previous,
        "Installed app changed during download"
    );
    ensure!(
        Version::parse(&plan.version)?
            .cmp_precedence(&Version::parse(&plan.previous)?)
            .is_gt(),
        "Update must be newer than installed app"
    );
    let backup = staging.join("previous.app");
    let ready = staging.join("ready");
    let result = replace_app(&target, &staged, &backup, || {
        let _ = fs::write(&plan.result, format!("Updated to {}", plan.version));
        let mut command = Command::new(target.join("Contents/MacOS/solo-miner"));
        if plan.resume {
            command.arg("--resume-mining");
        }
        let mut child = command
            .arg("--update-confirm")
            .arg(&ready)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(30) {
            if fs::read_to_string(&ready).is_ok_and(|v| v == plan.version) {
                return Ok(());
            }
            match child.try_wait() {
                Ok(Some(_)) => bail!("Updated app exited before confirming startup"),
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error.into());
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        bail!("Updated app did not confirm startup")
    });
    let message = match &result {
        Ok(()) => format!("Updated to {}", plan.version),
        Err(e) => format!("Update failed; previous version restored: {e:#}"),
    };
    let _ = fs::write(&plan.result, &message);
    // Retain the backup until a successful new app startup; failed updates also
    // leave their diagnostic folder available for inspection.
    if result.is_ok() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}
pub fn confirm_launch() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if let Some(pair) = args.windows(2).find(|a| a[0] == "--update-confirm") {
        let ready = PathBuf::from(&pair[1]);
        let parent = ready
            .parent()
            .context("Missing update confirmation folder")?;
        let plan: Plan = serde_json::from_slice(&fs::read(parent.join("plan.json"))?)?;
        ensure!(
            ready.file_name().is_some_and(|s| s == "ready")
                && plan.target == current_bundle()?
                && plan.version == env!("CARGO_PKG_VERSION"),
            "Unexpected update confirmation"
        );
        fs::write(ready, env!("CARGO_PKG_VERSION"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    fn release(version: &str) -> serde_json::Value {
        let tag = format!("v{version}");
        json!({"draft":false,"prerelease":false,"tag_name":tag,"assets":[
            {"name":ARCHIVE,"browser_download_url":format!("{REPO}/releases/download/{tag}/{ARCHIVE}"),"size":42},
            {"name":"solo-miner-macos-arm64.sha256","browser_download_url":format!("{REPO}/releases/download/{tag}/solo-miner-macos-arm64.sha256")}]})
    }
    #[test]
    fn versions_and_release_origin() -> Result<()> {
        assert!(parse_release(&release("0.10.0"), "0.9.0")?.is_some());
        assert!(parse_release(&release("0.7.0"), "0.8.0")?.is_none());
        assert!(parse_release(&release("0.8.0"), "0.8.0")?.is_none());
        let mut bad = release("0.9.0");
        bad["assets"][0]["browser_download_url"] = json!("https://example.com/update.zip");
        assert!(parse_release(&bad, "0.8.0").is_err());
        assert!(parse_release(&release("0.9.0-beta.1"), "0.8.0").is_err());
        Ok(())
    }
    #[test]
    fn checksum_rejects_missing_and_wrong_files() {
        assert!(checksum_text(format!("{}  {ARCHIVE}\n", "a".repeat(64)).as_bytes()).is_ok());
        assert!(checksum_text(format!("{}  other.zip\n", "a".repeat(64)).as_bytes()).is_err());
        assert!(checksum_text(b"abcd  solo-miner-macos-arm64.zip").is_err());
    }
    #[test]
    fn failed_install_and_failed_launch_restore_previous_app() -> Result<()> {
        for fail_rename in [false, true] {
            let root = tempfile::tempdir()?;
            let target = root.path().join("app");
            let staged = root.path().join("new");
            let backup = root.path().join("backup");
            fs::create_dir(&target)?;
            fs::write(target.join("version"), "old")?;
            if !fail_rename {
                fs::create_dir(&staged)?;
                fs::write(staged.join("version"), "new")?;
            }
            assert!(replace_app(&target, &staged, &backup, || bail!("Launch failed")).is_err());
            assert_eq!(fs::read_to_string(target.join("version"))?, "old");
        }
        Ok(())
    }
    #[test]
    fn successful_install_retains_backup_until_confirmation() -> Result<()> {
        let root = tempfile::tempdir()?;
        let target = root.path().join("app");
        let staged = root.path().join("new");
        let backup = root.path().join("backup");
        fs::create_dir(&target)?;
        fs::write(target.join("version"), "old")?;
        fs::create_dir(&staged)?;
        fs::write(staged.join("version"), "new")?;
        replace_app(&target, &staged, &backup, || Ok(()))?;
        assert_eq!(fs::read_to_string(target.join("version"))?, "new");
        assert_eq!(fs::read_to_string(backup.join("version"))?, "old");
        Ok(())
    }
    #[test]
    fn archive_rejects_traversal_symlinks_and_unexpected_roots() -> Result<()> {
        for (name, symlink) in [
            ("../outside", false),
            ("/outside", false),
            ("other.app/file", false),
            ("Solo Miner.app/link", true),
        ] {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let options = zip::write::SimpleFileOptions::default();
            if symlink {
                writer.add_symlink(name, "../../outside", options)?;
            } else {
                writer.start_file(name, options)?;
                writer.write_all(b"test")?;
            }
            let bytes = writer.finish()?.into_inner();
            let directory = tempfile::tempdir()?;
            assert!(
                extract(&bytes, directory.path()).is_err(),
                "Accepted unsafe entry {name}"
            );
        }
        Ok(())
    }
    #[test]
    fn archive_preserves_executable_mode() -> Result<()> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file(
            "Solo Miner.app/Contents/MacOS/solo-miner",
            zip::write::SimpleFileOptions::default().unix_permissions(0o755),
        )?;
        writer.write_all(b"test")?;
        let bytes = writer.finish()?.into_inner();
        let directory = tempfile::tempdir()?;
        extract(&bytes, directory.path())?;
        let file = directory
            .path()
            .join("Solo Miner.app/Contents/MacOS/solo-miner");
        assert_eq!(fs::read(&file)?, b"test");
        assert_eq!(fs::metadata(file)?.permissions().mode() & 0o777, 0o755);
        Ok(())
    }
    #[test]
    #[ignore = "Downloads the current GitHub release and verifies its real packaged app; never installs it"]
    fn published_release_download_and_bundle_verification() -> Result<()> {
        let body = fetch(API, 1024 * 1024, &mut |_, _| {}, 0)?;
        let release =
            parse_release(&serde_json::from_slice(&body)?, "0.0.0")?.context("No release")?;
        let root = tempfile::tempdir()?;
        let prepared = prepare_at(release, root.path().join(APP), &mut |_, _| {})?;
        assert!(prepared.staging.path().join(APP).is_dir());
        Ok(())
    }
    #[test]
    #[ignore = "Installer helper subprocess entry; invoked only by installer_subprocess_commit_and_rollback"]
    fn installer_child_entry() -> Result<()> {
        let plan =
            std::env::var_os("SOLO_MINER_INSTALL_TEST_PLAN").context("No test install plan")?;
        apply_update(Path::new(&plan))
    }
    #[test]
    #[ignore = "Requires macOS clang/codesign; installs isolated fixture apps and never changes the real app"]
    fn installer_subprocess_commit_and_rollback() -> Result<()> {
        for fail in [false, true] {
            let root = tempfile::tempdir()?;
            let root_path = root.path().canonicalize()?;
            let staging = tempfile::Builder::new()
                .prefix(".solo-miner-update-")
                .tempdir_in(&root_path)?;
            let target = root_path.join(APP);
            let staged = staging.path().join(APP);
            for (bundle, version, is_new) in [(&target, "0.0.1", false), (&staged, "0.0.2", true)] {
                fs::create_dir_all(bundle.join("Contents/MacOS"))?;
                fs::write(
                    bundle.join("Contents/Info.plist"),
                    format!(
                        r#"<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string><key>CFBundleExecutable</key><string>solo-miner</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleShortVersionString</key><string>{version}</string></dict></plist>"#
                    ),
                )?;
                let source = root_path.join("fixture.c");
                let code = if is_new && !fail {
                    r#"#include <stdio.h>
#include <string.h>
int main(int n,char **a){for(int i=1;i+1<n;i++){if(strcmp(a[i],"--update-confirm")==0){FILE*f=fopen(a[i+1],"w");if(!f)return 2;fputs("0.0.2",f);fclose(f);}}return 0;}"#
                } else {
                    "int main(void){return 1;}"
                };
                fs::write(&source, code)?;
                ensure!(
                    Command::new("/usr/bin/clang")
                        .arg(&source)
                        .arg("-o")
                        .arg(bundle.join("Contents/MacOS/solo-miner"))
                        .status()?
                        .success(),
                    "Compile fixture failed"
                );
                ensure!(
                    Command::new("/usr/bin/codesign")
                        .args(["--force", "--sign", "-"])
                        .arg(bundle)
                        .status()?
                        .success(),
                    "Sign fixture failed"
                );
            }
            let mut parent = Command::new("/usr/bin/true").spawn()?;
            let parent_pid = parent.id();
            parent.wait()?;
            let result_path = root_path.join("result.txt");
            let plan = Plan {
                target: target.clone(),
                version: "0.0.2".into(),
                previous: "0.0.1".into(),
                parent_pid,
                resume: false,
                result: result_path.clone(),
            };
            let plan_path = staging.path().join("plan.json");
            fs::write(&plan_path, serde_json::to_vec(&plan)?)?;
            let helper = staging.path().join("installer");
            let production = std::env::var_os("SOLO_MINER_INSTALLER_BINARY");
            fs::copy(
                production
                    .clone()
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_exe()?),
                &helper,
            )?;
            let mut command = Command::new(&helper);
            if production.is_some() {
                command.arg("--apply-update").arg(&plan_path);
            } else {
                command
                    .args([
                        "--ignored",
                        "--exact",
                        "updater::tests::installer_child_entry",
                    ])
                    .env("SOLO_MINER_INSTALL_TEST_PLAN", &plan_path);
            }
            let output = command.output()?;
            assert_eq!(
                output.status.success(),
                !fail,
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert_eq!(
                bundle_version(&target)?,
                if fail { "0.0.1" } else { "0.0.2" }
            );
            let report = fs::read_to_string(result_path)?;
            assert!(report.contains(if fail {
                "Update failed"
            } else {
                "Updated to 0.0.2"
            }));
        }
        Ok(())
    }
}
