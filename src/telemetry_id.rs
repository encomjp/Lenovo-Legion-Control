//! Hardware-derived pseudonymous telemetry id.
//!
//! A random id changes on every reinstall / config reset, so one laptop shows
//! up as several fleet hosts. The id is therefore derived from the device:
//!
//! ```text
//! SHA-256( "legion-control/telemetry-id/v1" 00 "product_uuid" 00 <product_uuid> 00 <product_name> 00 )
//! ```
//!
//! The first 16 bytes are formatted as a UUID (version nibble 5, RFC 4122
//! variant). When DMI `product_uuid` is unusable the same construction runs
//! over `/etc/machine-id` (tag `"machine-id"`); the last resort is a random
//! v4 id. Neither `product_uuid` nor `/etc/machine-id` ever leave the
//! machine: only the one-way digest does.
//!
//! `product_uuid` is root-readable only, so ONLY the root daemon derives the
//! id and writes it to the shared store; the unprivileged GUI/CLI read it, so
//! every process reports the same id. Existing installs migrate once: the old
//! id is parked as "pending previous" and sent once as `previous_machine_id`.

use std::path::{Path, PathBuf};

/// Fixed domain-separation salt. Part of the id contract: changing it
/// re-identifies the whole fleet.
pub const SALT: &str = "legion-control/telemetry-id/v1";

/// Shared (root-written, world-readable) store directory.
pub const STORE_DIR: &str = "/var/lib/legion-control";

/// Where the id came from. Hardware sources are sticky once adopted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    ProductUuid,
    MachineId,
    Random,
    /// Pre-hardware-id install (marker line absent).
    Legacy,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProductUuid => "product_uuid",
            Self::MachineId => "machine-id",
            Self::Random => "random",
            Self::Legacy => "legacy",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "product_uuid" => Self::ProductUuid,
            "machine-id" => Self::MachineId,
            "random" => Self::Random,
            _ => Self::Legacy,
        }
    }
    fn is_hardware(self) -> bool {
        matches!(self, Self::ProductUuid | Self::MachineId)
    }
}

// ─── SHA-256 (no crypto crate in the dependency tree) ───

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// Format 16 bytes as a canonical UUID string.
fn format_uuid(b: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13],
        b[14], b[15]
    )
}

/// True for a canonical 36-char UUID string (any version).
pub fn is_uuid_shaped(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Placeholder / junk firmware values that are not a unique device identity.
pub fn is_junk_identifier(raw: &str) -> bool {
    let t = raw.trim();
    if t.len() < 8 {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    const JUNK: [&str; 9] = [
        "default string",
        "to be filled by o.e.m.",
        "to be filled by oem",
        "not specified",
        "not settable",
        "not applicable",
        "system product name",
        "unknown",
        "n/a",
    ];
    if JUNK.iter().any(|j| lower.contains(j)) {
        return true;
    }
    // All one character once separators are stripped: all-zeros, all-F, …
    let flat: Vec<char> = lower.chars().filter(|c| *c != '-').collect();
    if flat.iter().all(|c| *c == flat[0]) {
        return true;
    }
    // Well-known vendor placeholder UUIDs.
    matches!(
        lower.as_str(),
        "03000200-0400-0500-0006-000700080009" | "12345678-1234-5678-1234-567812345678"
    )
}

fn derive(tag: &str, secret: &str, product_name: &str) -> String {
    let mut input = Vec::new();
    for part in [SALT, tag, secret.trim(), product_name.trim()] {
        input.extend_from_slice(part.as_bytes());
        input.push(0);
    }
    let digest = sha256(&input);
    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    b[6] = (b[6] & 0x0f) | 0x50; // v5-style
    b[8] = (b[8] & 0x3f) | 0x80; // RFC 4122 variant
    format_uuid(&b)
}

/// Pure: derive the hardware id from the raw inputs. `None` when neither
/// source is usable (caller falls back to a random id).
pub fn hardware_id(
    product_uuid: Option<&str>,
    product_name: &str,
    machine_id: Option<&str>,
) -> Option<(String, Source)> {
    if let Some(u) = product_uuid.filter(|u| !is_junk_identifier(u)) {
        return Some((derive("product_uuid", u, product_name), Source::ProductUuid));
    }
    if let Some(m) = machine_id.filter(|m| !is_junk_identifier(m)) {
        return Some((derive("machine-id", m, product_name), Source::MachineId));
    }
    None
}

/// Read the real inputs. `product_uuid` is None when unreadable (non-root).
pub fn hardware_id_from_system() -> Option<(String, Source)> {
    let rd = |p: &str| std::fs::read_to_string(p).ok().map(|s| s.trim().to_string());
    let uuid = rd("/sys/class/dmi/id/product_uuid");
    let name = rd("/sys/class/dmi/id/product_name").unwrap_or_default();
    let mid = rd("/etc/machine-id");
    hardware_id(uuid.as_deref(), &name, mid.as_deref())
}

/// Fresh random UUID v4 (bounded 16-byte urandom read).
pub fn random_id() -> String {
    use std::io::Read as _;
    let mut b = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut b);
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        b = (now ^ ((std::process::id() as u128) << 64)).to_be_bytes();
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format_uuid(&b)
}

// ─── Shared store ───

/// `machine-id` file: line 1 = id, optional line 2 = `source=<tag>`.
/// `previous-machine-id`: the one-shot migration hand-off.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn system() -> Self {
        Self {
            dir: PathBuf::from(STORE_DIR),
        }
    }
    pub fn at(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }
    fn id_path(&self) -> PathBuf {
        self.dir.join("machine-id")
    }
    fn prev_path(&self) -> PathBuf {
        self.dir.join("previous-machine-id")
    }

    pub fn read(&self) -> Option<(String, Source)> {
        let text = std::fs::read_to_string(self.id_path()).ok()?;
        let mut lines = text.lines();
        let id = lines.next()?.trim().to_string();
        if !is_uuid_shaped(&id) {
            return None;
        }
        let source = lines
            .next()
            .and_then(|l| l.trim().strip_prefix("source="))
            .map(Source::parse)
            .unwrap_or(Source::Legacy);
        Some((id, source))
    }

    pub fn write(&self, id: &str, source: Source) {
        self.write_file(&self.id_path(), &format!("{id}\nsource={}\n", source.as_str()));
    }

    pub fn record_previous(&self, old: &str) {
        self.write_file(&self.prev_path(), &format!("{old}\n"));
    }

    fn write_file(&self, path: &Path, body: &str) {
        if let Err(e) = std::fs::create_dir_all(&self.dir) {
            log::debug!("telemetry-id store: create_dir_all failed: {e}");
            return;
        }
        // Atomic replace so readers never see a torn file.
        let tmp = path.with_extension("tmp");
        let res = std::fs::write(&tmp, body).and_then(|_| {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644));
            std::fs::rename(&tmp, path)
        });
        if let Err(e) = res {
            log::debug!("telemetry-id store: write {} failed: {e}", path.display());
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// The pending previous id, but only if this process is able to consume
    /// it afterwards (otherwise it would be re-sent forever by whoever
    /// cannot delete it; the root daemon pushes every minute, so it wins).
    pub fn pending_previous(&self) -> Option<String> {
        let id = std::fs::read_to_string(self.prev_path())
            .ok()?
            .trim()
            .to_string();
        if !is_uuid_shaped(&id) {
            return None;
        }
        let dir = std::ffi::CString::new(self.dir.to_string_lossy().as_bytes()).ok()?;
        // SAFETY: access(2) on a valid NUL-terminated path.
        let writable = unsafe { libc::access(dir.as_ptr(), libc::W_OK) } == 0;
        writable.then_some(id)
    }

    /// Call after a report that carried the previous id was sent.
    pub fn clear_previous(&self) {
        let _ = std::fs::remove_file(self.prev_path());
    }
}

// ─── Resolution (pure) ───

/// Outcome of one resolution pass.
#[derive(Debug, PartialEq, Eq)]
pub struct Resolution {
    pub id: String,
    /// Persist to the shared store (root only).
    pub write_shared: Option<Source>,
    /// Old id to park for a one-time `previous_machine_id` (root only).
    pub previous: Option<String>,
}

/// Decide the id.
/// * `is_root`  — only root may derive/migrate (it alone sees `product_uuid`).
/// * `shared`   — the shared store content.
/// * `settings` — id in this user's/root's settings.json (legacy source).
/// * `hw`       — hardware derivation (`None` when unavailable / non-root).
pub fn resolve(
    is_root: bool,
    shared: Option<(String, Source)>,
    settings: Option<&str>,
    hw: Option<(String, Source)>,
    random: impl FnOnce() -> String,
) -> Resolution {
    let settings = settings.filter(|s| !s.is_empty());
    if !is_root {
        // Unprivileged: never derive (would disagree with root's id).
        let id = shared
            .map(|(id, _)| id)
            .or_else(|| settings.map(str::to_string))
            .unwrap_or_else(random);
        return Resolution {
            id,
            write_shared: None,
            previous: None,
        };
    }
    match (shared, hw) {
        // Hardware id already adopted: sticky, never switch again.
        (Some((id, src)), _) if src.is_hardware() => Resolution {
            id,
            write_shared: None,
            previous: None,
        },
        // Migration (or first run with a hardware id available).
        (shared, Some((hid, hsrc))) => {
            let old = shared
                .map(|(id, _)| id)
                .or_else(|| settings.map(str::to_string));
            Resolution {
                previous: old.filter(|o| *o != hid),
                id: hid,
                write_shared: Some(hsrc),
            }
        }
        // No hardware id derivable: keep whatever exists, else mint random.
        (Some((id, _)), None) => Resolution {
            id,
            write_shared: None,
            previous: None,
        },
        (None, None) => match settings {
            Some(s) => Resolution {
                id: s.to_string(),
                write_shared: Some(Source::Legacy),
                previous: None,
            },
            None => Resolution {
                id: random(),
                write_shared: Some(Source::Random),
                previous: None,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // 56-byte message forces an extra padding block.
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    const UUID: &str = "4c4c4544-0042-3510-8052-b4c04f4a3433";

    #[test]
    fn hardware_id_is_deterministic_and_uuid_formatted() {
        let a = hardware_id(Some(UUID), "83DG", Some("abcdef0123456789abcdef0123456789")).unwrap();
        let b = hardware_id(Some(UUID), "83DG", Some("ffffffffffff0000ffffffffffff0001")).unwrap();
        assert_eq!(a, b, "machine-id must not matter when product_uuid is good");
        assert_eq!(a.1, Source::ProductUuid);
        assert!(is_uuid_shaped(&a.0));
        let c: Vec<char> = a.0.chars().collect();
        assert_eq!(c[14], '5', "version nibble");
        assert!("89ab".contains(c[19]), "RFC 4122 variant");
        assert!(!a.0.contains(&UUID[..8]), "raw uuid must not leak");
        assert_ne!(a.0, hardware_id(Some(UUID), "82K2", None).unwrap().0);
        assert_ne!(
            a.0,
            hardware_id(Some("4c4c4544-0042-3510-8052-b4c04f4a3434"), "83DG", None)
                .unwrap()
                .0
        );
        assert_eq!(a.0, derive("product_uuid", UUID, "83DG"));
    }

    #[test]
    fn junk_uuid_falls_back_to_machine_id_then_none() {
        let mid = "abcdef0123456789abcdef0123456789";
        for junk in [
            "",
            "   ",
            "00000000-0000-0000-0000-000000000000",
            "FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF",
            "Default string",
            "To Be Filled By O.E.M.",
            "03000200-0400-0500-0006-000700080009",
            "12345678-1234-5678-1234-567812345678",
        ] {
            let r = hardware_id(Some(junk), "83DG", Some(mid)).unwrap();
            assert_eq!(r.1, Source::MachineId, "junk {junk:?}");
        }
        assert_eq!(
            hardware_id(None, "83DG", Some(mid)).unwrap().1,
            Source::MachineId
        );
        assert!(hardware_id(None, "83DG", None).is_none());
        assert!(hardware_id(Some("Default string"), "83DG", Some("")).is_none());
        assert_ne!(
            hardware_id(Some(UUID), "x", None).unwrap().0,
            hardware_id(None, "x", Some(UUID)).unwrap().0
        );
    }

    #[test]
    fn migration_switches_once_and_is_sticky() {
        let new_id = "aaaaaaaa-0000-5000-8000-000000000000";
        let hw = || Some((new_id.to_string(), Source::ProductUuid));
        let old = "11111111-1111-4111-8111-111111111111";
        let r = resolve(true, Some((old.into(), Source::Legacy)), None, hw(), random_id);
        assert_eq!(r.id, new_id);
        assert_eq!(r.previous.as_deref(), Some(old));
        assert_eq!(r.write_shared, Some(Source::ProductUuid));
        // Second pass (store now holds the hw id): nothing to do.
        let r2 = resolve(true, Some((r.id.clone(), Source::ProductUuid)), None, hw(), random_id);
        assert_eq!(r2.id, new_id);
        assert_eq!((r2.previous, r2.write_shared), (None, None));
        // Even if derivation later yields something else, the id is kept.
        let other = Some((
            "bbbbbbbb-0000-5000-8000-000000000000".to_string(),
            Source::MachineId,
        ));
        let r3 = resolve(true, Some((new_id.into(), Source::ProductUuid)), None, other, random_id);
        assert_eq!(r3.id, new_id);
        assert!(r3.previous.is_none());
        // Settings-only legacy install also links.
        let r4 = resolve(true, None, Some(old), hw(), random_id);
        assert_eq!(r4.previous.as_deref(), Some(old));
        // Fresh install: no previous.
        assert!(resolve(true, None, None, hw(), random_id).previous.is_none());
    }

    #[test]
    fn unprivileged_never_derives() {
        let hw = Some((
            "aaaaaaaa-0000-5000-8000-000000000000".to_string(),
            Source::MachineId,
        ));
        let shared = Some((
            "bbbbbbbb-0000-5000-8000-000000000000".to_string(),
            Source::ProductUuid,
        ));
        let r = resolve(false, shared, Some("settingsid"), hw.clone(), random_id);
        assert_eq!(r.id, "bbbbbbbb-0000-5000-8000-000000000000");
        assert!(r.write_shared.is_none() && r.previous.is_none());
        let r = resolve(false, None, Some("settingsid"), hw, || "rnd".into());
        assert_eq!(r.id, "settingsid");
        assert_eq!(resolve(false, None, None, None, || "rnd".into()).id, "rnd");
    }

    #[test]
    fn root_without_hardware_keeps_or_mints() {
        let r = resolve(true, None, None, None, || "rnd".into());
        assert_eq!((r.id.as_str(), r.write_shared), ("rnd", Some(Source::Random)));
        let r = resolve(true, None, Some("legacy"), None, || "rnd".into());
        assert_eq!(r.id, "legacy");
        let r = resolve(true, Some(("keep".into(), Source::Random)), None, None, || {
            "rnd".into()
        });
        assert_eq!(r.id, "keep");
    }

    #[test]
    fn previous_machine_id_is_delivered_exactly_once() {
        let dir = std::env::temp_dir().join(format!(
            "legion-tid-{}-{}",
            std::process::id(),
            random_id()
        ));
        let store = Store::at(&dir);
        let old = "11111111-1111-4111-8111-111111111111";
        assert!(store.pending_previous().is_none());
        store.record_previous(old);
        assert_eq!(store.pending_previous().as_deref(), Some(old));
        // Still pending until a send happened (failed send retries).
        assert_eq!(store.pending_previous().as_deref(), Some(old));
        store.clear_previous();
        assert!(store.pending_previous().is_none());
        // Id file round-trip incl. marker; legacy files parse as Legacy.
        store.write("aaaaaaaa-0000-5000-8000-000000000000", Source::ProductUuid);
        assert_eq!(
            store.read(),
            Some((
                "aaaaaaaa-0000-5000-8000-000000000000".into(),
                Source::ProductUuid
            ))
        );
        std::fs::write(dir.join("machine-id"), old).unwrap();
        assert_eq!(store.read(), Some((old.into(), Source::Legacy)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
