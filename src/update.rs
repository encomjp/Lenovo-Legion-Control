//! Release checker and in-app updater for Legion Control.
//!
//! Queries the GitHub releases API via `curl` (no extra HTTP crate), compares
//! semver tags, and installs the matching asset for this copy: AppImage,
//! `.deb`, `.rpm`, Arch `.pkg.tar.zst`, or a portable `x86_64` tarball.
//!
//! Every downloaded asset must carry a minisign signature (`<asset>.minisig`)
//! made with the release key embedded from `data/release-signing.pub`. The
//! signed trusted comment binds the file name and version
//! (`file:<name> version:<tag>`), so a validly signed asset cannot be swapped
//! for another one or for an older release. There is no unsigned fallback:
//! a missing or bad signature, or an unconfigured (placeholder) key, aborts
//! the update. Root-side installs never read the user-writable download
//! directly: the pkexec'd script copies it into a fresh root-owned 0700
//! directory, re-checks the sha256 computed over the verified bytes, and only
//! then installs.

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
const MAX_SIGNATURE_BYTES: u64 = 4096;

/// Minisign public key that every release asset must be signed with.
/// Kept in a separate file so key changes are easy to review.
const RELEASE_SIGNING_PUBKEY: &str = include_str!("../data/release-signing.pub");
/// Marker in the shipped placeholder key file. While present, updates fail closed.
const PUBKEY_PLACEHOLDER: &str = "REPLACE_WITH_MINISIGN_PUBLIC_KEY";

/// curl flags that pin every request to HTTPS (also across redirects).
const CURL_SECURE_ARGS: [&str; 5] = ["--proto", "=https", "--proto-redir", "=https", "--tlsv1.2"];

/// Knobs for [`apply_update_with_options`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpdateOptions {
    /// Install even when the release is not newer than the running version.
    pub allow_downgrade: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex, no `sha256:` prefix. Informational only: it comes
    /// from the same unauthenticated API response as `url`.
    pub sha256: Option<String>,
    /// Download URL of the detached `<name>.minisig` signature, if published.
    #[serde(default)]
    pub minisig_url: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdatePhase {
    Downloading,
    Verifying,
    Building,
    Installing,
    BuildingLog(String),
}

impl UpdatePhase {
    pub fn is_building(&self) -> bool {
        matches!(self, Self::Building | Self::BuildingLog(_))
    }
    pub fn building_tail(&self) -> Option<&str> {
        match self {
            Self::BuildingLog(s) => Some(s),
            _ => None,
        }
    }
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
            Self::Source => {
                "Fast-forwards your source tree to the release tag, rebuilds, \
                 and installs with one password prompt. Then restart this app."
            }
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
        // install.sh stages here. Treat as a tarball only when a portable
        // archive is how this copy is meant to update; otherwise keep it as
        // a source tree so git pull + rebuild still works.
        if source_tree().is_some() {
            return InstallKind::Source;
        }
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
        || (matches!(
            detect_install_kind(),
            InstallKind::Tarball | InstallKind::Source
        ) && source_tree().is_some())
}

/// First useful changelog line for the compact update card.
pub fn changelog_headline(body: &str) -> String {
    body.lines()
        .map(|line| line.trim().trim_start_matches('#').trim())
        .filter(|line| !line.is_empty() && !line.starts_with('|'))
        .map(|line| line.trim_start_matches(['-', '*']).trim())
        .find(|line| {
            let lower = line.to_ascii_lowercase();
            !lower.starts_with("changelog")
                && lower != "fixes"
                && lower != "notes"
                && lower != "new"
                && lower != "fixed"
        })
        .filter(|line| !line.is_empty())
        .map(|line| line.chars().take(160).collect())
        .unwrap_or_else(|| "A new version of Legion Control is available.".into())
}

/// Full notes for the What's new tab (tables stripped).
/// Short plain-text highlights from release notes: the bold title of each
/// top-level bullet (or the bullet text without markdown), at most `n`.
pub fn release_highlights(body: &str, n: usize) -> Vec<String> {
    body.lines()
        .filter(|l| l.starts_with("- ") || l.starts_with("* "))
        .map(|l| {
            let t = l[2..].trim();
            let t = match (t.strip_prefix("**"), t.find("**:")) {
                (Some(rest), Some(_)) => rest.split("**").next().unwrap_or(rest),
                _ => t,
            };
            t.replace("**", "").replace('`', "").trim().trim_end_matches(':').to_string()
        })
        .filter(|t| !t.is_empty())
        .take(n)
        .collect()
}

pub fn changelog_notes(body: &str) -> String {
    let notes = body
        .lines()
        .filter(|line| !line.trim_start().starts_with('|'))
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    notes.trim().to_string()
}

fn looks_like_source_tree(path: &Path) -> bool {
    path.join("Cargo.toml").is_file() && path.join("install.sh").is_file()
}

fn source_stamp_paths() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("/var/lib/legion-control/source-tree")];
    if let Some(config) = dirs::config_dir() {
        paths.push(config.join("legion-control").join("source-tree"));
    }
    paths
}

/// Git checkout used by `./install.sh`, if we can still find it.
pub fn source_tree() -> Option<PathBuf> {
    for stamp in source_stamp_paths() {
        if let Ok(raw) = fs::read_to_string(stamp) {
            let path = PathBuf::from(raw.trim());
            if looks_like_source_tree(&path) {
                return Some(path);
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut cursor = exe;
        for _ in 0..4 {
            let Some(parent) = cursor.parent() else {
                break;
            };
            cursor = parent.to_path_buf();
            if looks_like_source_tree(&cursor) {
                return Some(cursor);
            }
        }
    }
    let home = dirs::home_dir()?;
    for rel in [
        "Documents/lenovo-legion-control/lenovo-legion-tool",
        "Documents/Lenovo-Legion-Control",
        "src/lenovo-legion-tool",
        "src/Lenovo-Legion-Control",
    ] {
        let path = home.join(rel);
        if looks_like_source_tree(&path) {
            return Some(path);
        }
    }
    None
}

fn remember_source_tree(tree: &Path) {
    if let Some(dir) = dirs::config_dir() {
        let dir = dir.join("legion-control");
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(dir.join("source-tree"), tree.to_string_lossy().as_bytes());
    }
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
        InstallKind::Source => source_tree()
            .map(|tree| {
                format!(
                    "This copy was built from source at {}.\nUpdate with:\n  git pull && ./install.sh",
                    tree.display()
                )
            })
            .unwrap_or_else(|| {
                "This copy was built from source. Update with:\n  git pull && ./install.sh".into()
            }),
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
        log::debug!("update: no formal releases found on GitHub repo: current version is latest");
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
    best.map(|a| {
        let sig_name = format!("{}.minisig", a.asset_name());
        ReleaseAsset {
            name: a.asset_name().to_string(),
            url: a.asset_url().to_string(),
            size: a.asset_size(),
            sha256: parse_sha256_digest(a.asset_digest()),
            minisig_url: assets
                .iter()
                .find(|s| s.asset_name() == sig_name)
                .map(|s| s.asset_url().to_string()),
        }
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

pub fn apply_update_for_kind(
    info: &ReleaseInfo,
    kind: InstallKind,
    progress: impl FnMut(UpdatePhase, u64, Option<u64>),
) -> Result<ApplyOutcome, String> {
    apply_update_with_options(info, kind, UpdateOptions::default(), progress)
}

/// Refuse to install `target` unless it is strictly newer than `current`
/// (or the caller explicitly allowed a downgrade / reinstall).
pub fn check_not_downgrade(
    target: &str,
    current: &str,
    allow_downgrade: bool,
) -> Result<(), String> {
    if allow_downgrade || is_version_newer(target, current) {
        Ok(())
    } else {
        Err(format!(
            "Refusing to install v{} because it is not newer than the running v{}. \
             Use `legion-cli check-update --apply --allow-downgrade` to override.",
            target.trim_start_matches('v'),
            current.trim_start_matches('v')
        ))
    }
}

/// Like [`apply_update_for_kind`], with explicit [`UpdateOptions`].
pub fn apply_update_with_options(
    info: &ReleaseInfo,
    kind: InstallKind,
    opts: UpdateOptions,
    mut progress: impl FnMut(UpdatePhase, u64, Option<u64>),
) -> Result<ApplyOutcome, String> {
    check_not_downgrade(&info.version, CURRENT_VERSION, opts.allow_downgrade)?;
    match kind {
        InstallKind::AppImage => {
            let path = apply_appimage_update(info, &mut progress)?;
            Ok(ApplyOutcome {
                relaunch: path,
                needs_daemon_restage: true,
            })
        }
        k @ (InstallKind::Deb | InstallKind::Rpm | InstallKind::Arch) => {
            apply_package_update(info, k, &mut progress)
        }
        InstallKind::Tarball => {
            if info.tarball.is_some() {
                apply_tarball_update(info, &mut progress)
            } else {
                apply_source_update(info, &mut progress)
            }
        }
        InstallKind::Source => apply_source_update(info, &mut progress),
    }
}

/// Download and install the asset that matches this copy.
pub fn apply_update(
    info: &ReleaseInfo,
    progress: impl FnMut(UpdatePhase, u64, Option<u64>),
) -> Result<ApplyOutcome, String> {
    apply_update_for_kind(info, detect_install_kind(), progress)
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
    let sig_text = fetch_signature(asset)?;
    progress(UpdatePhase::Downloading, 0, Some(asset.size));
    download_file(&asset.url, &partial, asset.size, &mut progress)?;

    progress(UpdatePhase::Verifying, asset.size, Some(asset.size));
    verify_download(
        &partial,
        asset,
        InstallKind::AppImage,
        &info.version,
        &sig_text,
    )?;

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
    let install = package_install_script(kind)?;
    let (dest, sha256) = download_verified(asset, kind, &info.version, &mut progress)?;
    progress(UpdatePhase::Installing, asset.size, Some(asset.size));
    let script = format!("{ROOT_STAGE_PRELUDE}\n{install}\n");
    let result = pkexec_staged(&script, &dest, &sha256);
    let _ = fs::remove_file(&dest);
    result?;
    // Permanent fix for v0.2.6 daemon-stale telemetry (83RU CachyOS):
    // Package hooks (Arch post_upgrade try-restart, Debian postinst) are
    // unreliable under pkexec's minimal env / when the service was idle.
    // Force a daemon restart here so telemetry app_version flips immediately.
    let _ = Command::new("pkexec")
        .args([
            "/bin/sh",
            "-c",
            "systemctl daemon-reload >/dev/null 2>&1; systemctl try-restart legion-control.service >/dev/null 2>&1 || systemctl restart legion-control.service >/dev/null 2>&1 || true",
        ])
        .status();
    let relaunch =
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/bin/legion-settings"));
    Ok(ApplyOutcome {
        relaunch,
        needs_daemon_restage: false,
    })
}

/// Root-side prelude shared by every pkexec install of a downloaded asset.
///
/// Arguments (never interpolated into the script text):
///   `$1` the verified download in the user's cache dir,
///   `$2` the sha256 the unprivileged side computed over the bytes whose
///        minisign signature it verified.
///
/// The file is copied into a fresh root-owned 0700 directory under /run and
/// re-hashed there, so swapping the user-writable copy while the PolicyKit
/// prompt is open cannot change what gets installed. Sets `$stage` and
/// `$pkg` (the staged copy, keeping the asset's file name so apt/pacman/dnf
/// recognise the format).
const ROOT_STAGE_PRELUDE: &str = r#"set -eu
umask 077
src=$1
want=$2
case $want in
    *[!0-9a-f]*|'') echo "legion-update: bad expected sha256" >&2; exit 1 ;;
esac
if [ ${#want} -ne 64 ]; then echo "legion-update: bad expected sha256" >&2; exit 1; fi
stage=$(mktemp -d /run/legion-control-update.XXXXXXXXXX)
trap 'rm -rf -- "$stage"' EXIT
trap 'exit 1' HUP INT TERM
chmod 0700 "$stage"
name=$(basename -- "$src")
pkg="$stage/$name"
cat -- "$src" > "$pkg"
got=$(sha256sum -- "$pkg" | cut -d' ' -f1)
if [ "$got" != "$want" ]; then
    echo "legion-update: staged copy does not match the verified sha256; refusing to install" >&2
    exit 1
fi"#;

fn package_install_script(kind: InstallKind) -> Result<&'static str, String> {
    match kind {
        InstallKind::Deb => Ok("DEBIAN_FRONTEND=noninteractive apt-get install -y \"$pkg\""),
        InstallKind::Rpm => {
            if Path::new("/usr/bin/dnf").is_file() {
                Ok("dnf upgrade -y \"$pkg\" || dnf install -y \"$pkg\"")
            } else if Path::new("/usr/bin/zypper").is_file() {
                Ok("zypper --non-interactive install \"$pkg\"")
            } else if Path::new("/usr/bin/rpm").is_file() {
                Ok("rpm -Uvh \"$pkg\"")
            } else {
                Err("No RPM installer found (dnf, zypper, or rpm)".into())
            }
        }
        InstallKind::Arch => {
            if Path::new("/usr/bin/pacman").is_file() {
                Ok("pacman -U --noconfirm \"$pkg\"")
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
    let (archive, sha256) =
        download_verified(asset, InstallKind::Tarball, &info.version, &mut progress)?;
    progress(UpdatePhase::Installing, asset.size, Some(asset.size));

    // Extraction happens root-side, inside the root-owned staging dir, so no
    // user-writable file is ever installed.
    let script = format!("{ROOT_STAGE_PRELUDE}\n{TARBALL_INSTALL_SCRIPT}\n");
    let result = pkexec_staged(&script, &archive, &sha256);
    let _ = fs::remove_file(&archive);
    result?;

    let relaunch = PathBuf::from("/usr/local/bin/legion-settings");
    Ok(ApplyOutcome {
        relaunch,
        needs_daemon_restage: false,
    })
}

/// Runs after [`ROOT_STAGE_PRELUDE`]; `$pkg` is the staged, re-hashed archive.
const TARBALL_INSTALL_SCRIPT: &str = r#"x="$stage/extract"
mkdir -- "$x"
tar -xzf "$pkg" -C "$x" --no-same-owner --no-same-permissions
find_one() { find "$x" -maxdepth 5 -type f -name "$1" -print -quit; }
cli=$(find_one legion-cli)
daemon=$(find_one legion-daemon)
settings=$(find_one legion-settings)
helper=$(find_one legion-control-setup)
if [ -z "$cli" ] || [ -z "$daemon" ] || [ -z "$settings" ]; then
    echo "legion-update: archive is missing legion-cli, legion-daemon, or legion-settings" >&2
    exit 1
fi
install -Dm755 "$cli" /usr/local/bin/legion-cli
install -Dm755 "$daemon" /usr/local/bin/legion-daemon
install -Dm755 "$settings" /usr/local/bin/legion-settings
if [ -n "$helper" ]; then install -Dm755 "$helper" /usr/local/libexec/legion-control-setup; fi
systemctl try-restart legion-control.service >/dev/null 2>&1 || true"#;

fn apply_source_update(
    info: &ReleaseInfo,
    mut progress: impl FnMut(UpdatePhase, u64, Option<u64>),
) -> Result<ApplyOutcome, String> {
    let tree = source_tree().ok_or_else(manual_update_hint)?;
    progress(UpdatePhase::Downloading, 0, None);
    run_in_tree(
        &tree,
        "git",
        &["fetch", "--tags", "--force", "origin"],
        "Cannot fetch the source tree",
    )?;
    let tag = format!("v{}", info.version);
    let tagged = Command::new("git")
        .current_dir(&tree)
        .args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])
        .status()
        .ok()
        .is_some_and(|status| status.success());
    if tagged {
        // Fast-forward the current branch onto the release tag. Do not
        // `checkout` the tag — that detaches HEAD in the user's working tree.
        run_in_tree(
            &tree,
            "git",
            &["merge", "--ff-only", &tag],
            &format!("Cannot fast-forward to {tag} (commit or stash local changes first)"),
        )?;
    } else {
        run_in_tree(
            &tree,
            "git",
            &["pull", "--ff-only"],
            "Cannot fast-forward the source tree",
        )?;
    }

    progress(UpdatePhase::Building, 0, None);
    run_cargo_build_streaming(&tree, &mut progress)?;

    let cli = tree.join("target/release/legion-cli");
    let daemon = tree.join("target/release/legion-daemon");
    let settings = tree.join("target/release/legion-settings");
    let helper = tree.join("target/release/legion-control-setup");
    for bin in [&cli, &daemon, &settings, &helper] {
        if !bin.is_file() {
            return Err(format!("Build did not produce {}", bin.display()));
        }
    }

    progress(UpdatePhase::Installing, 0, None);
    let script = "install -Dm755 \"$1\" /usr/local/bin/legion-cli \
                  && install -Dm755 \"$2\" /usr/local/bin/legion-daemon \
                  && install -Dm755 \"$3\" /usr/local/bin/legion-settings \
                  && install -Dm755 \"$4\" /usr/local/libexec/legion-control-setup \
                  && mkdir -p /var/lib/legion-control \
                  && printf '%s\\n' \"$5\" > /var/lib/legion-control/source-tree \
                  && systemctl try-restart legion-control.service >/dev/null 2>&1 || true";
    let output = Command::new("pkexec")
        .args(["/bin/sh", "-c", script, "legion-update"])
        .arg(&cli)
        .arg(&daemon)
        .arg(&settings)
        .arg(&helper)
        .arg(&tree)
        .output()
        .map_err(|e| format!("Cannot start PolicyKit install: {e}"))?;
    if !output.status.success() {
        return Err(pkexec_error(&output));
    }
    remember_source_tree(&tree);
    Ok(ApplyOutcome {
        relaunch: PathBuf::from("/usr/local/bin/legion-settings"),
        needs_daemon_restage: false,
    })
}

fn cargo_bin() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".cargo/bin/cargo"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("cargo"))
}

fn run_cargo_build_streaming(
    tree: &Path,
    progress: &mut impl FnMut(UpdatePhase, u64, Option<u64>),
) -> Result<(), String> {
    use std::io::{BufRead, BufReader};
    let cargo = cargo_bin();
    let mut child = Command::new(&cargo)
        .current_dir(tree)
        .args(["build", "--release", "--locked"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Release build failed: {e}"))?;
    let stderr = child.stderr.take();
    if let Some(pipe) = stderr {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            let t = line.trim().to_string();
            if t.is_empty() {
                continue;
            }
            let tail = if t.len() > 72 {
                format!("{}…", &t[..72])
            } else {
                t.clone()
            };
            if tail.contains("Compiling")
                || tail.contains("Finished")
                || tail.contains("error")
                || tail.contains("warning")
            {
                progress(UpdatePhase::BuildingLog(tail), 0, None);
            } else {
                progress(UpdatePhase::Building, 0, None);
            }
        }
    }
    let status = child
        .wait()
        .map_err(|e| format!("Release build failed: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Release build failed ({status})"))
    }
}

fn run_in_tree(
    tree: &Path,
    program: impl AsRef<std::ffi::OsStr>,
    args: &[&str],
    context: &str,
) -> Result<(), String> {
    let output = Command::new(program)
        .current_dir(tree)
        .args(args)
        .output()
        .map_err(|e| format!("{context}: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if err.is_empty() {
            Err(format!("{context} ({})", output.status))
        } else {
            Err(format!("{context}: {err}"))
        }
    }
}

/// Download `asset` and its signature, verify both. Returns the download
/// path and the sha256 of the exact bytes whose signature was verified.
fn download_verified(
    asset: &ReleaseAsset,
    kind: InstallKind,
    version: &str,
    progress: &mut impl FnMut(UpdatePhase, u64, Option<u64>),
) -> Result<(PathBuf, String), String> {
    let dir = cache_updates_dir()?;
    let dest = dir.join(safe_filename(&asset.name)?);
    let _ = fs::remove_file(&dest);
    let sig_text = fetch_signature(asset)?;
    progress(UpdatePhase::Downloading, 0, Some(asset.size));
    download_file(&asset.url, &dest, asset.size, progress)?;
    progress(UpdatePhase::Verifying, asset.size, Some(asset.size));
    let sha256 = verify_download(&dest, asset, kind, version, &sig_text)?;
    Ok((dest, sha256))
}

/// Fail closed before downloading anything when the build has no real key
/// or the release did not publish a signature for this asset.
fn fetch_signature(asset: &ReleaseAsset) -> Result<String, String> {
    release_public_key(RELEASE_SIGNING_PUBKEY)?;
    let url = asset.minisig_url.as_deref().ok_or_else(|| {
        format!(
            "Release asset {} has no {}.minisig signature. Unsigned updates are refused.",
            asset.name, asset.name
        )
    })?;
    let bytes = curl_fetch_small(url, MAX_SIGNATURE_BYTES)?;
    String::from_utf8(bytes).map_err(|_| "Release signature is not valid text".to_string())
}

/// True when this build embeds a real release signing key.
pub fn signing_configured() -> bool {
    release_public_key(RELEASE_SIGNING_PUBKEY).is_ok()
}

fn release_public_key(text: &str) -> Result<minisign_verify::PublicKey, String> {
    if text.contains(PUBKEY_PLACEHOLDER) {
        return Err(
            "This build has no release signing key configured (placeholder in \
                    data/release-signing.pub), so in-app updates are disabled. \
                    Download the new release manually from GitHub."
                .into(),
        );
    }
    let b64 = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
        .ok_or_else(|| "Release signing key is empty".to_string())?;
    minisign_verify::PublicKey::from_base64(b64)
        .map_err(|e| format!("Release signing key is invalid: {e}"))
}

/// Verify a minisign signature over `data` and check that the signed
/// trusted comment names exactly `expected_file` and `expected_version`.
pub fn verify_release_signature(
    pubkey_text: &str,
    data: &[u8],
    sig_text: &str,
    expected_file: &str,
    expected_version: &str,
) -> Result<(), String> {
    let pk = release_public_key(pubkey_text)?;
    let sig = minisign_verify::Signature::decode(sig_text)
        .map_err(|e| format!("Release signature for {expected_file} is malformed: {e}"))?;
    // allow_legacy = false: only prehashed (Ed25519 over BLAKE2b) signatures,
    // the default for minisign >= 0.10 and rsign2.
    pk.verify(data, &sig, false)
        .map_err(|e| format!("Release signature check failed for {expected_file}: {e}"))?;
    check_trusted_comment(sig.trusted_comment(), expected_file, expected_version)
}

/// The trusted comment is covered by the signature; it must read
/// `file:<asset name> version:<tag>` for this exact asset and release.
fn check_trusted_comment(
    comment: &str,
    expected_file: &str,
    expected_version: &str,
) -> Result<(), String> {
    let mut file = None;
    let mut version = None;
    for tok in comment.split_whitespace() {
        let (slot, value) = if let Some(v) = tok.strip_prefix("file:") {
            (&mut file, v)
        } else if let Some(v) = tok.strip_prefix("version:") {
            (&mut version, v)
        } else {
            continue;
        };
        if slot.replace(value).is_some() {
            return Err("Release signature has an ambiguous trusted comment".into());
        }
    }
    let norm = |v: &str| v.trim().trim_start_matches('v').to_string();
    if file != Some(expected_file) {
        return Err(format!(
            "Release signature was made for a different file ({}), not {expected_file}",
            file.unwrap_or("none")
        ));
    }
    match version {
        Some(v) if !v.is_empty() && norm(v) == norm(expected_version) => Ok(()),
        other => Err(format!(
            "Release signature was made for version {}, not {expected_version}",
            other.unwrap_or("none")
        )),
    }
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

/// Run a root script built on [`ROOT_STAGE_PRELUDE`] with the file and its
/// verified sha256 passed as positional arguments (no interpolation).
fn pkexec_staged(script: &str, file: &Path, sha256: &str) -> Result<(), String> {
    if !is_sha256_hex(sha256) {
        return Err("Internal error: invalid sha256 for staged install".into());
    }
    let output = Command::new("pkexec")
        .args(["/bin/sh", "-c", script, "legion-update"])
        .arg(file)
        .arg(sha256)
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
    require_https(url)?;
    let mut child = Command::new("curl")
        .args(curl_download_args(url, dest))
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
                        .map(|mut s| {
                            let mut buf = String::new();
                            let _ = s.read_to_string(&mut buf);
                            buf
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

/// Verify a downloaded asset: magic bytes, minisign signature + trusted
/// comment, and (if GitHub supplied one) the API digest. Returns the sha256
/// of the verified bytes. Deletes the file on any failure.
fn verify_download(
    path: &Path,
    asset: &ReleaseAsset,
    kind: InstallKind,
    version: &str,
    sig_text: &str,
) -> Result<String, String> {
    let result = (|| {
        // Read once: the signature and the sha256 handed to the root side
        // are computed over the same in-memory bytes.
        let data = fs::read(path).map_err(|e| format!("Cannot read download: {e}"))?;
        if !magic_ok(&data[..data.len().min(8)], kind) {
            return Err(format!("Downloaded file is not a valid {}", kind.label()));
        }
        verify_release_signature(
            RELEASE_SIGNING_PUBKEY,
            &data,
            sig_text,
            &asset.name,
            version,
        )?;
        let got = sha256_bytes(&data)?;
        if let Some(expected) = asset.sha256.as_deref() {
            if got != expected {
                return Err("Checksum mismatch (sha256). The download was discarded.".into());
            }
        }
        log::info!(
            "update: minisign signature verified for {} (sha256 {got})",
            asset.name
        );
        Ok(got)
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
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

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// sha256 of in-memory bytes via coreutils `sha256sum` (no extra crate).
fn sha256_bytes(data: &[u8]) -> Result<String, String> {
    let mut child = Command::new("sha256sum")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Need sha256sum to verify the download: {e}"))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Cannot write to sha256sum".to_string())?;
        stdin
            .write_all(data)
            .map_err(|e| format!("Cannot write to sha256sum: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("sha256sum failed: {e}"))?;
    if !output.status.success() {
        return Err("Checksum command failed".into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace()
        .next()
        .map(str::to_ascii_lowercase)
        .filter(|tok| is_sha256_hex(tok))
        .ok_or_else(|| "Could not parse checksum output".to_string())
}

fn require_https(url: &str) -> Result<(), String> {
    if url.starts_with("https://") {
        Ok(())
    } else {
        Err(format!("Refusing non-HTTPS update URL: {url}"))
    }
}

fn curl_get_args(url: &str, timeout_secs: u64) -> Vec<String> {
    let mut args: Vec<String> = CURL_SECURE_ARGS.iter().map(|s| s.to_string()).collect();
    args.extend([
        "-sL".into(),
        "-S".into(),
        "--max-time".into(),
        timeout_secs.to_string(),
        "-A".into(),
        USER_AGENT.into(),
        "-H".into(),
        "Accept: application/vnd.github+json".into(),
        url.into(),
    ]);
    args
}

fn curl_download_args(url: &str, dest: &Path) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = CURL_SECURE_ARGS.iter().map(Into::into).collect();
    for a in [
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
    ] {
        args.push(a.into());
    }
    args.push(dest.into());
    args.push(url.into());
    args
}

fn curl_get(url: &str, timeout_secs: u64) -> Result<Vec<u8>, String> {
    require_https(url)?;
    let output = Command::new("curl")
        .args(curl_get_args(url, timeout_secs))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Failed to spawn curl: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("curl error (exit {}): {err}", output.status));
    }
    Ok(output.stdout)
}

/// HTTPS GET of a small file (signatures); fails on HTTP errors.
fn curl_fetch_small(url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    require_https(url)?;
    let output = Command::new("curl")
        .args(CURL_SECURE_ARGS)
        .args([
            "-fsSL",
            "--max-time",
            "30",
            "-A",
            USER_AGENT,
            "--max-filesize",
        ])
        .arg(max_bytes.to_string())
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Failed to spawn curl: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Cannot download the release signature (curl {}): {}",
            output.status,
            err.trim()
        ));
    }
    if output.stdout.len() as u64 > max_bytes {
        return Err("Release signature is too large".into());
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
                name: "legion-control_0.2.0_amd64.deb.minisig",
                url: "https://example/deb.minisig",
                size: 300,
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

    #[test]
    fn changelog_headline_skips_heading_noise() {
        let body = "# Changelog - 0.2.3 (2026-08-30)\n\n## Fixes\n\n### LOQ / IdeaPad fan RPM at idle\n\n- Prefer yogafan\n";
        assert_eq!(changelog_headline(body), "LOQ / IdeaPad fan RPM at idle");
    }

    // ── Signed updates ────────────────────────────────────────────────
    // Fixtures were made with rsign2 and a throwaway, test-only key
    // (secret key discarded). See tests/fixtures/update-signing/.
    const TEST_PUB: &str = include_str!("../tests/fixtures/update-signing/test-signing.pub");
    const PAYLOAD: &[u8] = include_bytes!("../tests/fixtures/update-signing/payload-9.9.9.bin");
    // trusted comment: "file:legion-control_9.9.9_amd64.deb version:v9.9.9"
    const PAYLOAD_SIG: &str =
        include_str!("../tests/fixtures/update-signing/payload-9.9.9.minisig");
    const OLD_PAYLOAD: &[u8] = include_bytes!("../tests/fixtures/update-signing/payload-0.0.1.bin");
    // trusted comment: "file:legion-control_0.0.1_amd64.deb version:v0.0.1"
    const OLD_PAYLOAD_SIG: &str =
        include_str!("../tests/fixtures/update-signing/payload-0.0.1.minisig");
    const NAME: &str = "legion-control_9.9.9_amd64.deb";

    #[test]
    fn signature_happy_path() {
        verify_release_signature(TEST_PUB, PAYLOAD, PAYLOAD_SIG, NAME, "9.9.9").unwrap();
        verify_release_signature(TEST_PUB, PAYLOAD, PAYLOAD_SIG, NAME, "v9.9.9").unwrap();
    }

    #[test]
    fn signature_rejects_tampered_file() {
        let mut tampered = PAYLOAD.to_vec();
        tampered[0] ^= 1;
        assert!(verify_release_signature(TEST_PUB, &tampered, PAYLOAD_SIG, NAME, "9.9.9").is_err());
        let mut longer = PAYLOAD.to_vec();
        longer.push(b'x');
        assert!(verify_release_signature(TEST_PUB, &longer, PAYLOAD_SIG, NAME, "9.9.9").is_err());
    }

    #[test]
    fn signature_rejects_wrong_trusted_comment() {
        // Valid signature, but for a different file name or version.
        let err = verify_release_signature(
            TEST_PUB,
            PAYLOAD,
            PAYLOAD_SIG,
            "legion-control-9.9.9-x86_64.AppImage",
            "9.9.9",
        )
        .unwrap_err();
        assert!(err.contains("different file"), "{err}");
        let err =
            verify_release_signature(TEST_PUB, PAYLOAD, PAYLOAD_SIG, NAME, "9.9.10").unwrap_err();
        assert!(err.contains("version"), "{err}");

        // Editing the trusted comment breaks the global signature.
        let forged = PAYLOAD_SIG.replace("version:v9.9.9", "version:v9.9.8");
        assert_ne!(forged, PAYLOAD_SIG);
        assert!(verify_release_signature(TEST_PUB, PAYLOAD, &forged, NAME, "9.9.8").is_err());
    }

    #[test]
    fn signature_rejects_swapped_older_signed_asset() {
        // An attacker serving a genuine, older signed release under the
        // newer name/version must be rejected via the trusted comment.
        assert!(
            verify_release_signature(TEST_PUB, OLD_PAYLOAD, OLD_PAYLOAD_SIG, NAME, "9.9.9")
                .is_err()
        );
        // Signature for one payload over another payload's bytes.
        assert!(
            verify_release_signature(TEST_PUB, OLD_PAYLOAD, PAYLOAD_SIG, NAME, "9.9.9").is_err()
        );
    }

    #[test]
    fn signature_rejects_wrong_key_and_garbage() {
        // Real key id differs: use the minisign-verify crate's own test key.
        let other =
            "untrusted comment: other\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3\n";
        assert!(verify_release_signature(other, PAYLOAD, PAYLOAD_SIG, NAME, "9.9.9").is_err());
        assert!(
            verify_release_signature(TEST_PUB, PAYLOAD, "not a signature", NAME, "9.9.9").is_err()
        );
        assert!(verify_release_signature(TEST_PUB, PAYLOAD, "", NAME, "9.9.9").is_err());
    }

    #[test]
    fn placeholder_key_fails_closed() {
        let placeholder = format!("untrusted comment: placeholder\n{PUBKEY_PLACEHOLDER}\n");
        let err = release_public_key(&placeholder).unwrap_err();
        assert!(err.contains("no release signing key"), "{err}");
        // Even a perfectly valid signature is refused under the placeholder.
        assert!(
            verify_release_signature(&placeholder, PAYLOAD, PAYLOAD_SIG, NAME, "9.9.9").is_err()
        );
    }

    #[test]
    fn embedded_key_is_placeholder_or_valid() {
        let res = release_public_key(RELEASE_SIGNING_PUBKEY);
        if RELEASE_SIGNING_PUBKEY.contains(PUBKEY_PLACEHOLDER) {
            assert!(res.is_err());
            assert!(!signing_configured());
        } else {
            res.expect("data/release-signing.pub must be a valid minisign public key");
            assert!(signing_configured());
        }
    }

    #[test]
    fn trusted_comment_parsing_is_strict() {
        assert!(check_trusted_comment("file:a.deb version:v1.0.0", "a.deb", "1.0.0").is_ok());
        assert!(check_trusted_comment("version:1.0.0 file:a.deb", "a.deb", "v1.0.0").is_ok());
        assert!(check_trusted_comment("file:a.deb", "a.deb", "1.0.0").is_err());
        assert!(check_trusted_comment("version:1.0.0", "a.deb", "1.0.0").is_err());
        assert!(check_trusted_comment("file:a.deb version:", "a.deb", "").is_err());
        assert!(
            check_trusted_comment("file:a.deb file:b.deb version:1.0.0", "a.deb", "1.0.0").is_err()
        );
        assert!(check_trusted_comment("file:a.deb.x version:1.0.0", "a.deb", "1.0.0").is_err());
    }

    #[test]
    fn downgrade_refused_unless_allowed() {
        assert!(check_not_downgrade("0.2.17", "0.2.16", false).is_ok());
        let err = check_not_downgrade("0.2.15", "0.2.16", false).unwrap_err();
        assert!(err.contains("--allow-downgrade"), "{err}");
        assert!(check_not_downgrade("0.2.16", "0.2.16", false).is_err());
        assert!(check_not_downgrade("v0.2.16", "0.2.16", false).is_err());
        assert!(check_not_downgrade("0.2.15", "0.2.16", true).is_ok());
    }

    #[test]
    fn apply_refuses_downgrade_before_any_download() {
        let mut info = empty_release("0.0.1");
        info.deb = pick_asset(&sample_assets(), AssetKind::Deb);
        let err = apply_update_with_options(
            &info,
            InstallKind::Deb,
            UpdateOptions::default(),
            |_, _, _| panic!("must not start downloading"),
        )
        .unwrap_err();
        assert!(err.contains("not newer"), "{err}");
    }

    #[test]
    fn curl_args_force_https() {
        fn has_pair<S: AsRef<std::ffi::OsStr>>(args: &[S], a: &str, b: &str) -> bool {
            args.windows(2)
                .any(|w| w[0].as_ref() == a && w[1].as_ref() == b)
        }
        let get = curl_get_args("https://api.github.com/x", 8);
        assert!(has_pair(&get, "--proto", "=https"));
        assert!(has_pair(&get, "--proto-redir", "=https"));
        assert!(get.iter().any(|a| a == "--tlsv1.2"));
        let dl = curl_download_args("https://github.com/x", Path::new("/tmp/x"));
        assert!(has_pair(&dl, "--proto", "=https"));
        assert!(has_pair(&dl, "--proto-redir", "=https"));
        assert!(dl.iter().any(|a| a == "--tlsv1.2"));
        assert!(require_https("http://github.com/x").is_err());
        assert!(require_https("file:///etc/passwd").is_err());
        assert!(require_https("https://github.com/x").is_ok());
    }

    #[test]
    fn pick_asset_finds_detached_signature() {
        let assets = sample_assets();
        let deb = pick_asset(&assets, AssetKind::Deb).unwrap();
        assert_eq!(
            deb.minisig_url.as_deref(),
            Some("https://example/deb.minisig")
        );
        // The .minisig itself is never picked as a package.
        assert_eq!(deb.name, "legion-control_0.2.0_amd64.deb");
        assert!(pick_asset(&assets, AssetKind::Rpm)
            .unwrap()
            .minisig_url
            .is_none());
    }

    #[test]
    fn sha256_bytes_matches_known_vector() {
        assert_eq!(
            sha256_bytes(b"abc").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// Runs the real root prelude unprivileged, with /run swapped for a temp
    /// dir, to check it stages a private copy and enforces the sha256.
    #[test]
    fn root_stage_prelude_rehashes_staged_copy() {
        let base = std::env::temp_dir().join(format!("legion-stage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let src = base.join("legion-control_9.9.9_amd64.deb");
        fs::write(&src, PAYLOAD).unwrap();
        let good = sha256_bytes(PAYLOAD).unwrap();
        let prelude = ROOT_STAGE_PRELUDE.replace("/run/", &format!("{}/", base.to_string_lossy()));
        assert_ne!(prelude, ROOT_STAGE_PRELUDE);
        let script =
            format!("{prelude}\nstat -c %a \"$stage\"; basename -- \"$pkg\"; cat -- \"$pkg\"");
        let run = |sha: &str| {
            Command::new("/bin/sh")
                .args(["-c", &script, "legion-update"])
                .arg(&src)
                .arg(sha)
                .output()
                .unwrap()
        };
        let ok = run(&good);
        assert!(
            ok.status.success(),
            "{}",
            String::from_utf8_lossy(&ok.stderr)
        );
        let out = String::from_utf8_lossy(&ok.stdout);
        assert!(
            out.starts_with("700\nlegion-control_9.9.9_amd64.deb\n"),
            "{out}"
        );
        assert!(out.ends_with(std::str::from_utf8(PAYLOAD).unwrap()));

        let bad = run(&"0".repeat(64));
        assert!(!bad.status.success());
        assert!(String::from_utf8_lossy(&bad.stderr).contains("refusing to install"));
        // Shell metacharacters in the expected hash are rejected outright.
        assert!(!run("$(id)").status.success());
        // Staging dirs are cleaned up on exit.
        let leftovers = fs::read_dir(&base)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("legion-control-update.")
            })
            .count();
        assert_eq!(leftovers, 0);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn root_scripts_parse() {
        let deb = package_install_script(InstallKind::Deb).unwrap();
        for body in [deb, TARBALL_INSTALL_SCRIPT] {
            let script = format!("{ROOT_STAGE_PRELUDE}\n{body}\n");
            let ok = Command::new("/bin/sh")
                .args(["-n", "-c", &script])
                .status()
                .unwrap();
            assert!(ok.success(), "script failed to parse:\n{script}");
        }
    }

    #[test]
    fn looks_like_source_tree_requires_cargo_and_installer() {
        let dir = std::env::temp_dir().join(format!("legion-src-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(!looks_like_source_tree(&dir));
        fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
        assert!(!looks_like_source_tree(&dir));
        fs::write(dir.join("install.sh"), "#!/bin/sh\n").unwrap();
        assert!(looks_like_source_tree(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_highlights_strip_markdown() {
        let body = "## v1\n- **GPU mode (MUX)**: switch between `hybrid` and dGPU\n  - nested detail\n- Plain `item`\n| table |";
        assert_eq!(
            super::release_highlights(body, 4),
            vec!["GPU mode (MUX)".to_string(), "Plain item".to_string()]
        );
    }
}
