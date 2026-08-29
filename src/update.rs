//! Release checker and in-app updater for Legion Control.
//!
//! Queries the GitHub releases API via `curl` (no extra HTTP crate), compares
//! semver tags, and installs the matching asset for this copy: AppImage,
//! `.deb`, `.rpm`, Arch `.pkg.tar.zst`, or a portable `x86_64` tarball.

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub const GITHUB_REPO: &str = "encomjp/Lenovo-Legion-Control";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const USER_AGENT: &str = concat!("legion-control-updater/", env!("CARGO_PKG_VERSION"));
const MIN_DOWNLOAD_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex, no `sha256:` prefix.
    pub sha256: Option<String>,
}

pub type AppImageAsset = ReleaseAsset;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub version: String,
    pub name: String,
    pub body: String,
    pub html_url: String,
    pub published_at: String,
    pub is_newer: bool,
    pub appimage: Option<ReleaseAsset>,
    pub deb: Option<ReleaseAsset>,
    pub rpm: Option<ReleaseAsset>,
    pub arch: Option<ReleaseAsset>,
    pub tarball: Option<ReleaseAsset>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatePhase {
    Downloading,
    Verifying,
    Installing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    AppImage,
    Deb,
    Rpm,
    Arch,
    Tarball,
    Source,
}

#[derive(Debug, Clone)]
pub struct ApplyOutcome {
    pub relaunch: PathBuf,
    pub needs_daemon_restage: bool,
}

impl InstallKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::AppImage => "AppImage",
            Self::Deb => "Debian package",
            Self::Rpm => "RPM package",
            Self::Arch => "Arch package",
            Self::Tarball => "binary archive",
            Self::Source => "source install",
        }
    }

    pub fn apply_blurb(self) -> &'static str {
        match self {
            Self::AppImage => {
                "The AppImage will download and replace itself. After restart, \
                 one password prompt refreshes the background service."
            }
            Self::Deb | Self::Rpm | Self::Arch => {
                "The matching package will download and install with one password \
                 prompt. The background service restarts as part of the package. \
                 Then restart this app."
            }
            Self::Tarball => {
                "The binary archive will download and install under /usr/local \
                 with one password prompt. Then restart this app."
            }
            Self::Source => "",
        }
    }

    pub fn needs_daemon_restage(self) -> bool {
        matches!(self, Self::AppImage)
    }
}

/// Path of the running AppImage, if this process was launched by the runtime.
pub fn running_appimage_path() -> Option<PathBuf> {
    let raw = std::env::var_os("APPIMAGE")?;
    let path = PathBuf::from(raw);
    path.is_file().then_some(path)
}

pub fn detect_install_kind() -> InstallKind {
    if running_appimage_path().is_some() {
        return InstallKind::AppImage;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    if let Some(kind) = package_owning(&exe) {
        return kind;
    }
    if exe.starts_with("/usr/local") {
        return InstallKind::Tarball;
    }
    InstallKind::Source
}

fn package_owning(exe: &Path) -> Option<InstallKind> {
    if exe.as_os_str().is_empty() {
        return None;
    }
    let exe_s = exe.to_string_lossy();
    if cmd_stdout_contains("dpkg", &["-S", &exe_s], "legion-control") {
        return Some(InstallKind::Deb);
    }
    if cmd_stdout_contains("rpm", &["-qf", &exe_s], "legion-control") {
        return Some(InstallKind::Rpm);
    }
    if cmd_stdout_contains("pacman", &["-Qo", &exe_s], "legion-control") {
        return Some(InstallKind::Arch);
    }
    None
}

fn cmd_stdout_contains(cmd: &str, args: &[&str], needle: &str) -> bool {
    Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).contains(needle))
}

/// Asset that matches this install, if the release shipped one.
pub fn selected_asset(info: &ReleaseInfo) -> Option<&ReleaseAsset> {
    match detect_install_kind() {
        InstallKind::AppImage => info.appimage.as_ref(),
        InstallKind::Deb => info.deb.as_ref(),
        InstallKind::Rpm => info.rpm.as_ref(),
        InstallKind::Arch => info.arch.as_ref(),
        InstallKind::Tarball => info.tarball.as_ref(),
        InstallKind::Source => None,
    }
}

pub fn can_apply(info: &ReleaseInfo) -> bool {
    selected_asset(info).is_some()
}

/// True when this process can download and replace itself from a release asset.
pub fn can_apply_appimage(info: &ReleaseInfo) -> bool {
    detect_install_kind() == InstallKind::AppImage && info.appimage.is_some()
}

/// Hint for installs that cannot self-replace.
pub fn manual_update_hint() -> String {
    match detect_install_kind() {
        InstallKind::AppImage => {
            "Move the AppImage to a writable folder (for example ~/Applications) and try again."
                .into()
        }
        InstallKind::Deb => {
            "This copy is a .deb. Update with:\n  sudo apt install ./legion-control_*_amd64.deb"
                .into()
        }
        InstallKind::Rpm => {
            "This copy is an RPM. Update with:\n  sudo dnf upgrade ./legion-control-*.rpm".into()
        }
        InstallKind::Arch => {
            "This copy is an Arch package. Update with:\n  sudo pacman -U ./legion-control-*.pkg.tar.zst"
                .into()
        }
        InstallKind::Tarball => {
            "This copy lives under /usr/local. Update with the x86_64 tarball from the release, or:\n  git pull && ./install.sh"
                .into()
        }
        InstallKind::Source => {
            "This copy was built from source. Update with:\n  git pull && ./install.sh".into()
        }
    }
}

/// Check GitHub for the latest release of `encomjp/Lenovo-Legion-Control`.
/// Runs with a 8-second timeout and fails gracefully if offline.
pub fn check_latest_release() -> Result<ReleaseInfo, String> {
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    log::debug!("update: checking latest release from {url}");
    let bytes = curl_get(&url, 8)?;
    parse_release_json(&bytes)
}

fn empty_release(version: &str) -> ReleaseInfo {
    ReleaseInfo {
        tag_name: format!("v{version}"),
        version: version.to_string(),
        name: format!("Version {version}"),
        body: "Current release".into(),
        html_url: format!("https://github.com/{GITHUB_REPO}/releases"),
        published_at: "".into(),
        is_newer: false,
        appimage: None,
        deb: None,
        rpm: None,
        arch: None,
        tarball: None,
    }
}

fn parse_release_json(bytes: &[u8]) -> Result<ReleaseInfo, String> {
    #[derive(Deserialize)]
    struct GhAsset {
        name: String,
        browser_download_url: String,
        size: u64,
        digest: Option<String>,
    }

    #[derive(Deserialize)]
    struct GhRelease {
        tag_name: String,
        name: Option<String>,
        body: Option<String>,
        html_url: Option<String>,
        published_at: Option<String>,
        #[serde(default)]
        assets: Vec<GhAsset>,
    }

    let body_str = String::from_utf8_lossy(bytes);
    if body_str.contains("Not Found")
        || body_str.contains("\"message\"") && !body_str.contains("\"tag_name\"")
    {
        log::debug!("update: no formal releases found on GitHub repo — current version is latest");
        return Ok(empty_release(CURRENT_VERSION));
    }

    let gh: GhRelease = serde_json::from_slice(bytes)
        .map_err(|e| format!("Failed to parse GitHub release JSON: {e}"))?;

    let version = gh.tag_name.trim_start_matches('v').to_string();
    let is_newer = is_version_newer(&version, CURRENT_VERSION);
    let refs: Vec<GhAssetRef<'_>> = gh
        .assets
        .iter()
        .map(|a| GhAssetRef {
            name: &a.name,
            url: &a.browser_download_url,
            size: a.size,
            digest: a.digest.as_deref(),
        })
        .collect();

    Ok(ReleaseInfo {
        tag_name: gh.tag_name,
        version: version.clone(),
        name: gh.name.unwrap_or_else(|| format!("Version {version}")),
        body: gh.body.unwrap_or_default(),
        html_url: gh
            .html_url
            .unwrap_or_else(|| format!("https://github.com/{GITHUB_REPO}/releases")),
        published_at: gh.published_at.unwrap_or_default(),
        is_newer,
        appimage: pick_asset(&refs, AssetKind::AppImage),
        deb: pick_asset(&refs, AssetKind::Deb),
        rpm: pick_asset(&refs, AssetKind::Rpm),
        arch: pick_asset(&refs, AssetKind::Arch),
        tarball: pick_asset(&refs, AssetKind::Tarball),
    })
}

#[derive(Clone, Copy)]
enum AssetKind {
    AppImage,
    Deb,
    Rpm,
    Arch,
    Tarball,
}

fn pick_asset<A>(assets: &[A], kind: AssetKind) -> Option<ReleaseAsset>
where
    A: AssetLike,
{
    let mut best: Option<&A> = None;
    let mut best_score = u8::MAX;
    for asset in assets {
        let Some(score) = asset_score(asset.asset_name(), kind) else {
            continue;
        };
        if score < best_score {
            best_score = score;
            best = Some(asset);
        }
    }
    best.map(|a| ReleaseAsset {
        name: a.asset_name().to_string(),
        url: a.asset_url().to_string(),
        size: a.asset_size(),
        sha256: parse_sha256_digest(a.asset_digest()),
    })
}

fn asset_score(name: &str, kind: AssetKind) -> Option<u8> {
    let n = name.to_ascii_lowercase();
    let arch = n.contains("x86_64") || n.contains("amd64");
    match kind {
        AssetKind::AppImage => {
            if n.ends_with(".appimage") && !n.contains(".zsync") {
                Some(if arch { 0 } else { 1 })
            } else {
                None
            }
        }
        AssetKind::Deb => n.ends_with(".deb").then_some(if arch { 0 } else { 1 }),
        AssetKind::Rpm => {
            if n.ends_with(".rpm") && !n.contains(".src.rpm") && !n.ends_with(".src.rpm") {
                Some(if arch { 0 } else { 1 })
            } else {
                None
            }
        }
        AssetKind::Arch => n
            .ends_with(".pkg.tar.zst")
            .then_some(if arch { 0 } else { 1 }),
        AssetKind::Tarball => {
            // Portable binaries: legion-control-<ver>-x86_64.tar.gz
            // Skip source archives that lack an architecture in the name.
            if n.ends_with(".tar.gz") && arch && !n.contains(".pkg.tar") {
                Some(0)
            } else {
                None
            }
        }
    }
}

trait AssetLike {
    fn asset_name(&self) -> &str;
    fn asset_url(&self) -> &str;
    fn asset_size(&self) -> u64;
    fn asset_digest(&self) -> Option<&str>;
}

impl AssetLike for GhAssetRef<'_> {
    fn asset_name(&self) -> &str {
        self.name
    }
    fn asset_url(&self) -> &str {
        self.url
    }
    fn asset_size(&self) -> u64 {
        self.size
    }
    fn asset_digest(&self) -> Option<&str> {
        self.digest
    }
}

/// Tiny stand-in so pickers can be tested without the serde types.
struct GhAssetRef<'a> {
    name: &'a str,
    url: &'a str,
    size: u64,
    digest: Option<&'a str>,
}

fn parse_sha256_digest(digest: Option<&str>) -> Option<String> {
    let d = digest?.trim();
    if d.is_empty() {
        return None;
    }
    let hex = d
        .strip_prefix("sha256:")
        .or_else(|| d.strip_prefix("SHA256:"))
        .unwrap_or(d)
        .trim()
        .to_ascii_lowercase();
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hex)
}

/// Download and install the asset that matches this copy.
pub fn apply_update(
    info: &ReleaseInfo,
    progress: impl FnMut(UpdatePhase, u64, Option<u64>),
) -> Result<ApplyOutcome, String> {
    match detect_install_kind() {
        InstallKind::AppImage => {
            let path = apply_appimage_update(info, progress)?;
            Ok(ApplyOutcome {
                relaunch: path,
                needs_daemon_restage: true,
            })
        }
        kind @ (InstallKind::Deb | InstallKind::Rpm | InstallKind::Arch) => {
            apply_package_update(info, kind, progress)
        }
        InstallKind::Tarball => apply_tarball_update(info, progress),
        InstallKind::Source => Err(manual_update_hint()),
    }
}

/// Download the release AppImage, verify it, and replace the running file.
///
/// The running process keeps its old inode until exit; the path then points
/// at the new image so the next launch (and [`spawn_relaunch`]) picks it up.
pub fn apply_appimage_update(
    info: &ReleaseInfo,
    mut progress: impl FnMut(UpdatePhase, u64, Option<u64>),
) -> Result<PathBuf, String> {
    let asset = info
        .appimage
        .as_ref()
        .ok_or_else(|| "This release has no AppImage to download".to_string())?;
    let current = running_appimage_path()
        .ok_or_else(|| "In-app update is only available when running the AppImage".to_string())?;
    let dir = current.parent().ok_or_else(|| {
        format!(
            "Cannot determine the folder that holds {}",
            current.display()
        )
    })?;
    ensure_writable_dir(dir)?;

    let partial = dir.join(format!(".{}.partial", safe_filename(&asset.name)?));
    let _ = fs::remove_file(&partial);
    progress(UpdatePhase::Downloading, 0, Some(asset.size));
    download_file(&asset.url, &partial, asset.size, &mut progress)?;

    progress(UpdatePhase::Verifying, asset.size, Some(asset.size));
    verify_download(&partial, asset, InstallKind::AppImage)?;

    progress(UpdatePhase::Installing, asset.size, Some(asset.size));
    chmod_755(&partial)?;

    fs::rename(&partial, &current).map_err(|e| {
        let _ = fs::remove_file(&partial);
        format!("Cannot replace {}: {e}", current.display())
    })?;
    mark_pending_restage();
    log::info!(
        "update: replaced {} with {} (v{})",
        current.display(),
        asset.name,
        info.version
    );
    Ok(current)
}

fn apply_package_update(
    info: &ReleaseInfo,
    kind: InstallKind,
    mut progress: impl FnMut(UpdatePhase, u64, Option<u64>),
) -> Result<ApplyOutcome, String> {
    let asset = selected_asset(info)
        .ok_or_else(|| format!("This release has no {} for this install", kind.label()))?;
    let dest = download_verified(asset, kind, &mut progress)?;
    progress(UpdatePhase::Installing, asset.size, Some(asset.size));
    let script = package_install_script(kind)?;
    pkexec_with_file(script, &dest)?;
    let _ = fs::remove_file(&dest);
    let relaunch =
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/bin/legion-settings"));
    Ok(ApplyOutcome {
        relaunch,
        needs_daemon_restage: false,
    })
}

fn package_install_script(kind: InstallKind) -> Result<&'static str, String> {
    match kind {
        InstallKind::Deb => Ok("DEBIAN_FRONTEND=noninteractive apt-get install -y \"$1\""),
        InstallKind::Rpm => {
            if Path::new("/usr/bin/dnf").is_file() {
                Ok("dnf upgrade -y \"$1\" || dnf install -y \"$1\"")
            } else if Path::new("/usr/bin/zypper").is_file() {
                Ok("zypper --non-interactive install \"$1\"")
            } else if Path::new("/usr/bin/rpm").is_file() {
                Ok("rpm -Uvh \"$1\"")
            } else {
                Err("No RPM installer found (dnf, zypper, or rpm)".into())
            }
        }
        InstallKind::Arch => {
            if Path::new("/usr/bin/pacman").is_file() {
                Ok("pacman -U --noconfirm \"$1\"")
            } else {
                Err("pacman not found".into())
            }
        }
        _ => Err("Not a native package install".into()),
    }
}

fn apply_tarball_update(
    info: &ReleaseInfo,
    mut progress: impl FnMut(UpdatePhase, u64, Option<u64>),
) -> Result<ApplyOutcome, String> {
    let asset = info
        .tarball
        .as_ref()
        .ok_or_else(|| "This release has no x86_64 tarball to download".to_string())?;
    let archive = download_verified(asset, InstallKind::Tarball, &mut progress)?;
    progress(UpdatePhase::Installing, asset.size, Some(asset.size));

    let work = archive.with_extension("extract");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| format!("Cannot create extract dir: {e}"))?;
    let status = Command::new("tar")
        .args(["-xzf"])
        .arg(&archive)
        .arg("-C")
        .arg(&work)
        .status()
        .map_err(|e| format!("Cannot run tar: {e}"))?;
    if !status.success() {
        let _ = fs::remove_dir_all(&work);
        let _ = fs::remove_file(&archive);
        return Err("Failed to extract the binary archive".into());
    }

    let cli = find_named(&work, "legion-cli");
    let daemon = find_named(&work, "legion-daemon");
    let settings = find_named(&work, "legion-settings");
    let helper = find_named(&work, "legion-control-setup");
    let (Some(cli), Some(daemon), Some(settings)) = (cli, daemon, settings) else {
        let _ = fs::remove_dir_all(&work);
        let _ = fs::remove_file(&archive);
        return Err("Archive is missing legion-cli, legion-daemon, or legion-settings".into());
    };

    // Fixed script: $1 cli, $2 daemon, $3 settings, $4 optional helper.
    let script = "install -Dm755 \"$1\" /usr/local/bin/legion-cli \
                  && install -Dm755 \"$2\" /usr/local/bin/legion-daemon \
                  && install -Dm755 \"$3\" /usr/local/bin/legion-settings \
                  && if [ -n \"$4\" ]; then install -Dm755 \"$4\" /usr/local/libexec/legion-control-setup; fi \
                  && systemctl try-restart legion-control.service >/dev/null 2>&1 || true";
    let mut cmd = Command::new("pkexec");
    cmd.args(["/bin/sh", "-c", script, "legion-update"])
        .arg(&cli)
        .arg(&daemon)
        .arg(&settings);
    if let Some(helper) = helper {
        cmd.arg(&helper);
    } else {
        cmd.arg("");
    }
    let output = cmd
        .output()
        .map_err(|e| format!("Cannot start PolicyKit install: {e}"))?;
    let _ = fs::remove_dir_all(&work);
    let _ = fs::remove_file(&archive);
    if !output.status.success() {
        return Err(pkexec_error(&output));
    }

    let relaunch = PathBuf::from("/usr/local/bin/legion-settings");
    Ok(ApplyOutcome {
        relaunch,
        needs_daemon_restage: false,
    })
}

fn download_verified(
    asset: &ReleaseAsset,
    kind: InstallKind,
    progress: &mut impl FnMut(UpdatePhase, u64, Option<u64>),
) -> Result<PathBuf, String> {
    let dir = cache_updates_dir()?;
    let dest = dir.join(safe_filename(&asset.name)?);
    let _ = fs::remove_file(&dest);
    progress(UpdatePhase::Downloading, 0, Some(asset.size));
    download_file(&asset.url, &dest, asset.size, progress)?;
    progress(UpdatePhase::Verifying, asset.size, Some(asset.size));
    verify_download(&dest, asset, kind)?;
    Ok(dest)
}

fn cache_updates_dir() -> Result<PathBuf, String> {
    let dir = dirs::cache_dir()
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .ok_or_else(|| "Cannot locate cache directory".to_string())?
        .join("legion-control")
        .join("updates");
    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}

fn safe_filename(name: &str) -> Result<String, String> {
    if name.is_empty()
        || name.len() > 160
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
    {
        return Err("Release asset has an unsafe file name".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err("Release asset has an unsafe file name".into());
    }
    Ok(name.to_string())
}

fn ensure_writable_dir(dir: &Path) -> Result<(), String> {
    let probe = dir.join(format!(".legion-update-write-test-{}", std::process::id()));
    match File::create(&probe) {
        Ok(mut f) => {
            let _ = f.write_all(b"ok");
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&probe);
            Err(format!(
                "Cannot write to {} ({e}). Move the file to a folder you own, such as ~/Applications.",
                dir.display()
            ))
        }
    }
}

fn chmod_755(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .map_err(|e| format!("Cannot stat download: {e}"))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)
            .map_err(|e| format!("Cannot mark the new file executable: {e}"))?;
    }
    let _ = path;
    Ok(())
}

fn find_named(root: &Path, name: &str) -> Option<PathBuf> {
    fn walk(dir: &Path, name: &str, depth: u8) -> Option<PathBuf> {
        if depth == 0 {
            return None;
        }
        let entries = fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = walk(&path, name, depth - 1) {
                    return Some(found);
                }
            } else if path.file_name().is_some_and(|n| n == name) {
                return Some(path);
            }
        }
        None
    }
    walk(root, name, 5)
}

fn pkexec_with_file(script: &str, file: &Path) -> Result<(), String> {
    let output = Command::new("pkexec")
        .args(["/bin/sh", "-c", script, "legion-update"])
        .arg(file)
        .output()
        .map_err(|e| format!("Cannot start PolicyKit install: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(pkexec_error(&output))
    }
}

fn pkexec_error(output: &std::process::Output) -> String {
    let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if err.is_empty() {
        format!("Install was cancelled or failed ({})", output.status)
    } else {
        err
    }
}

fn download_file(
    url: &str,
    dest: &Path,
    expected: u64,
    progress: &mut impl FnMut(UpdatePhase, u64, Option<u64>),
) -> Result<(), String> {
    let mut child = Command::new("curl")
        .args([
            "-fL",
            "--retry",
            "2",
            "--retry-delay",
            "1",
            "--max-time",
            "180",
            "-A",
            USER_AGENT,
            "-o",
        ])
        .arg(dest)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn curl: {e}"))?;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let err = child
                        .stderr
                        .take()
                        .and_then(|mut s| {
                            let mut buf = String::new();
                            let _ = s.read_to_string(&mut buf);
                            Some(buf)
                        })
                        .unwrap_or_default();
                    let _ = fs::remove_file(dest);
                    return Err(format!("Download failed (curl {status}): {}", err.trim()));
                }
                break;
            }
            Ok(None) => {
                if let Ok(meta) = fs::metadata(dest) {
                    progress(UpdatePhase::Downloading, meta.len(), Some(expected));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => {
                let _ = fs::remove_file(dest);
                return Err(format!("Download failed: {e}"));
            }
        }
    }

    let got = fs::metadata(dest)
        .map(|m| m.len())
        .map_err(|e| format!("Download missing after curl: {e}"))?;
    if expected > 0 && got != expected {
        let _ = fs::remove_file(dest);
        return Err(format!(
            "Download size mismatch: got {got} bytes, expected {expected}"
        ));
    }
    if got < MIN_DOWNLOAD_BYTES {
        let _ = fs::remove_file(dest);
        return Err(format!(
            "Download is too small ({got} bytes) to be a release"
        ));
    }
    progress(UpdatePhase::Downloading, got, Some(expected));
    Ok(())
}

fn verify_download(path: &Path, asset: &ReleaseAsset, kind: InstallKind) -> Result<(), String> {
    let mut magic = [0u8; 8];
    let n = File::open(path)
        .and_then(|mut f| f.read(&mut magic))
        .map_err(|e| format!("Cannot read download: {e}"))?;
    if !magic_ok(&magic[..n], kind) {
        let _ = fs::remove_file(path);
        return Err(format!("Downloaded file is not a valid {}", kind.label()));
    }
    if let Some(expected) = asset.sha256.as_deref() {
        let got = sha256_file(path)?;
        if got != expected {
            let _ = fs::remove_file(path);
            return Err("Checksum mismatch (sha256). The download was discarded.".into());
        }
        log::debug!("update: sha256 verified ({got})");
    } else {
        log::warn!("update: release asset has no sha256 digest — magic + size check only");
    }
    Ok(())
}

fn magic_ok(bytes: &[u8], kind: InstallKind) -> bool {
    match kind {
        InstallKind::AppImage => bytes.starts_with(b"\x7fELF"),
        InstallKind::Deb => bytes.starts_with(b"!<arch>"),
        InstallKind::Rpm => bytes.starts_with(&[0xED, 0xAB, 0xEE, 0xDB]),
        InstallKind::Arch => bytes.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]), // zstd
        InstallKind::Tarball => bytes.starts_with(&[0x1F, 0x8B]),          // gzip
        InstallKind::Source => false,
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .or_else(|_| {
            Command::new("openssl")
                .args(["dgst", "-sha256"])
                .arg(path)
                .output()
        })
        .map_err(|e| format!("Need sha256sum or openssl to verify the download: {e}"))?;
    if !output.status.success() {
        return Err("Checksum command failed".into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let hex = text
        .split_whitespace()
        .find(|tok| tok.len() == 64 && tok.bytes().all(|b| b.is_ascii_hexdigit()))
        .or_else(|| {
            text.split('=')
                .last()
                .map(str::trim)
                .filter(|tok| tok.len() == 64)
        })
        .ok_or_else(|| "Could not parse checksum output".to_string())?
        .to_ascii_lowercase();
    Ok(hex)
}

fn curl_get(url: &str, timeout_secs: u64) -> Result<Vec<u8>, String> {
    let output = Command::new("curl")
        .args([
            "-sL",
            "-S",
            "--max-time",
            &timeout_secs.to_string(),
            "-A",
            USER_AGENT,
            "-H",
            "Accept: application/vnd.github+json",
            url,
        ])
        .output()
        .map_err(|e| format!("Failed to spawn curl: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("curl error (exit {}): {err}", output.status));
    }
    Ok(output.stdout)
}

/// After the current process exits, exec `exe` so GApplication is not still
/// holding the session bus name.
pub fn spawn_relaunch(exe: &Path, extra_args: &[&str]) -> Result<(), String> {
    if !exe.is_file() {
        return Err(format!("Updated binary missing at {}", exe.display()));
    }
    let pid = std::process::id();
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(
            "while kill -0 \"$LEGION_RELAUNCH_PID\" 2>/dev/null; do sleep 0.1; done; \
             sleep 0.35; exec \"$LEGION_RELAUNCH_EXE\" \"$@\"",
        )
        .arg("legion-relaunch")
        .args(extra_args)
        .env("LEGION_RELAUNCH_PID", pid.to_string())
        .env("LEGION_RELAUNCH_EXE", exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    cmd.spawn()
        .map_err(|e| format!("Cannot schedule relaunch: {e}"))?;
    Ok(())
}

fn restage_marker_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("legion-control").join("pending-daemon-restage"))
}

pub fn mark_pending_restage() {
    if let Some(path) = restage_marker_path() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&path, CURRENT_VERSION);
    }
}

pub fn has_pending_restage() -> bool {
    restage_marker_path().is_some_and(|p| p.is_file())
}

pub fn clear_pending_restage() {
    if let Some(path) = restage_marker_path() {
        let _ = fs::remove_file(path);
    }
}

/// True when a previous AppImage Enable left a host-side daemon to refresh.
pub fn daemon_was_staged() -> bool {
    Path::new("/usr/local/bin/legion-daemon").is_file()
        || Path::new("/etc/systemd/system/legion-control.service").is_file()
}

/// Compare two semver-like version strings (e.g. "0.2.0" > "0.1.0", "1.0.0" > "0.9.9").
/// Returns true if `remote` is strictly newer than `local`.
pub fn is_version_newer(remote: &str, local: &str) -> bool {
    let parse_nums = |s: &str| -> Vec<u64> {
        let clean = s.trim().trim_start_matches('v');
        clean
            .split('.')
            .filter_map(|part| {
                let num_str = part.split('-').next().unwrap_or(part);
                num_str.parse::<u64>().ok()
            })
            .collect()
    };

    let r_nums = parse_nums(remote);
    let l_nums = parse_nums(local);

    for i in 0..r_nums.len().max(l_nums.len()) {
        let r_val = r_nums.get(i).copied().unwrap_or(0);
        let l_val = l_nums.get(i).copied().unwrap_or(0);
        if r_val > l_val {
            return true;
        }
        if r_val < l_val {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_assets() -> Vec<GhAssetRef<'static>> {
        vec![
            GhAssetRef {
                name: "legion-control_0.2.0_amd64.deb",
                url: "https://example/deb",
                size: 11,
                digest: None,
            },
            GhAssetRef {
                name: "legion-control-0.2.0-1.fc42.x86_64.rpm",
                url: "https://example/rpm",
                size: 22,
                digest: None,
            },
            GhAssetRef {
                name: "legion-control-0.2.0-1-x86_64.pkg.tar.zst",
                url: "https://example/arch",
                size: 33,
                digest: None,
            },
            GhAssetRef {
                name: "legion-control-0.2.0-x86_64.tar.gz",
                url: "https://example/tar",
                size: 44,
                digest: None,
            },
            GhAssetRef {
                name: "legion-control-0.2.0.tar.gz",
                url: "https://example/src",
                size: 55,
                digest: None,
            },
            GhAssetRef {
                name: "legion-control-0.2.0-x86_64.AppImage",
                url: "https://example/appimage",
                size: 99,
                digest: Some(
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ),
            },
        ]
    }

    #[test]
    fn test_is_version_newer_comparisons() {
        assert!(is_version_newer("0.2.0", "0.1.0"));
        assert!(is_version_newer("1.0.0", "0.9.9"));
        assert!(is_version_newer("0.1.1", "0.1.0"));
        assert!(is_version_newer("v0.2.0", "0.1.0"));
        assert!(is_version_newer("0.2.0-rc1", "0.1.0"));

        assert!(!is_version_newer("0.1.0", "0.1.0"));
        assert!(!is_version_newer("0.1.0", "0.2.0"));
        assert!(!is_version_newer("0.0.9", "0.1.0"));
        assert!(!is_version_newer("v0.1.0", "0.1.0"));
    }

    #[test]
    fn test_parse_sha256_digest() {
        assert_eq!(
            parse_sha256_digest(Some(
                "sha256:6b4e192621bb98290c8f6d5087aecf016821adb34677a7d3ba2c8193372e8afb"
            ))
            .as_deref(),
            Some("6b4e192621bb98290c8f6d5087aecf016821adb34677a7d3ba2c8193372e8afb")
        );
        assert_eq!(parse_sha256_digest(Some("not-a-hash")), None);
        assert_eq!(parse_sha256_digest(None), None);
    }

    #[test]
    fn test_pick_each_package_format() {
        let assets = sample_assets();
        assert_eq!(
            pick_asset(&assets, AssetKind::AppImage).unwrap().name,
            "legion-control-0.2.0-x86_64.AppImage"
        );
        assert_eq!(
            pick_asset(&assets, AssetKind::Deb).unwrap().name,
            "legion-control_0.2.0_amd64.deb"
        );
        assert_eq!(
            pick_asset(&assets, AssetKind::Rpm).unwrap().name,
            "legion-control-0.2.0-1.fc42.x86_64.rpm"
        );
        assert_eq!(
            pick_asset(&assets, AssetKind::Arch).unwrap().name,
            "legion-control-0.2.0-1-x86_64.pkg.tar.zst"
        );
        assert_eq!(
            pick_asset(&assets, AssetKind::Tarball).unwrap().name,
            "legion-control-0.2.0-x86_64.tar.gz"
        );
    }

    #[test]
    fn test_pick_appimage_prefers_x86_64() {
        let assets = sample_assets();
        let picked = pick_asset(&assets, AssetKind::AppImage).expect("AppImage");
        assert_eq!(picked.name, "legion-control-0.2.0-x86_64.AppImage");
        assert_eq!(picked.size, 99);
        assert!(picked.sha256.is_some());
    }

    #[test]
    fn test_parse_release_json_with_assets() {
        let json = r#"{
            "tag_name": "v0.3.0",
            "name": "Legion Control 0.3.0",
            "body": "- in-app updates\n",
            "html_url": "https://github.com/encomjp/Lenovo-Legion-Control/releases/tag/v0.2.0",
            "published_at": "2026-08-30T00:00:00Z",
            "assets": [
                {
                    "name": "legion-control-0.3.0-x86_64.AppImage",
                    "browser_download_url": "https://github.com/encomjp/Lenovo-Legion-Control/releases/download/v0.3.0/legion-control-0.3.0-x86_64.AppImage",
                    "size": 4506816,
                    "digest": "sha256:6b4e192621bb98290c8f6d5087aecf016821adb34677a7d3ba2c8193372e8afb"
                },
                {
                    "name": "legion-control_0.3.0_amd64.deb",
                    "browser_download_url": "https://example/deb",
                    "size": 2430124,
                    "digest": null
                },
                {
                    "name": "legion-control-0.3.0-1.fc42.x86_64.rpm",
                    "browser_download_url": "https://example/rpm",
                    "size": 2682695,
                    "digest": null
                }
            ]
        }"#;
        let info = parse_release_json(json.as_bytes()).unwrap();
        assert_eq!(info.version, "0.3.0");
        assert!(info.is_newer);
        assert!(info.appimage.unwrap().url.contains("AppImage"));
        assert_eq!(info.deb.unwrap().name, "legion-control_0.3.0_amd64.deb");
        assert!(info.rpm.unwrap().name.ends_with(".rpm"));
        assert!(info.arch.is_none());
        assert!(info.tarball.is_none());
    }

    #[test]
    fn test_magic_ok() {
        assert!(magic_ok(b"\x7fELFrest", InstallKind::AppImage));
        assert!(magic_ok(b"!<arch>\n", InstallKind::Deb));
        assert!(magic_ok(&[0xED, 0xAB, 0xEE, 0xDB], InstallKind::Rpm));
        assert!(magic_ok(&[0x28, 0xB5, 0x2F, 0xFD], InstallKind::Arch));
        assert!(magic_ok(&[0x1F, 0x8B, 0x08], InstallKind::Tarball));
        assert!(!magic_ok(b"hello", InstallKind::Deb));
    }

    #[test]
    fn test_safe_filename() {
        assert!(safe_filename("legion-control_0.2.0_amd64.deb").is_ok());
        assert!(safe_filename("../etc/passwd").is_err());
        assert!(safe_filename("foo bar.deb").is_err());
    }
}
