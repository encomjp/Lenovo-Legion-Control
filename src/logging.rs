//! Shared logging bootstrap for daemon, CLI, and settings.
//!
//! Filter with `LEGION_LOG` (env_logger syntax). Examples:
//! - `LEGION_LOG=info` (default)
//! - `LEGION_LOG=debug`
//! - `LEGION_LOG=legion_core=debug,legion_settings=info`
//! - `LEGION_LOG=json`  → compact JSON to stderr
//!
//! Additional knobs (daemon only):
//! - `LEGION_LOG_FILE=1`      → write rotated log file under ~/.local/share/legion-control/
//! - `LEGION_LOG_RING=500`    → in-memory ring buffer size (default 500)

use log::{LevelFilter, Record};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

const DEFAULT_RING_SIZE: usize = 500;
const MAX_RING_SIZE: usize = 2000;
const FILE_RETENTION_DAYS: u64 = 7;

// ── internal self-diagnostics ─────────────────────────────────────────────
// Events raised BY the logging implementation (ring pushes, file writes,
// rotation) must NEVER go through the `log` facade: every facade call is
// dispatched straight back into `Logger::log`, so a `log::*!` call on that
// path would recurse until the stack blows ("logging about logging").
// Instead, self-events are written directly to stderr and gated on the
// global max level — trace-only visibility with zero recursion.

/// Emit one logging-subsystem self-event (see block comment above).
fn emit_self_trace(args: std::fmt::Arguments<'_>) {
    if log::max_level() < LevelFilter::Trace {
        return;
    }
    let ts = chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let _ = writeln!(io::stderr(), "{ts} TRACE [legion-log] {args}");
}

macro_rules! self_trace {
    ($($arg:tt)*) => {
        emit_self_trace(format_args!($($arg)*))
    };
}

/// One log line, serialisable for JSON / ring buffer / file.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub ts: String,
    pub level: String,
    pub target: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub message: String,
}

/// Thread-safe ring buffer of recent log lines.
struct RingBuffer {
    buf: VecDeque<LogEntry>,
    capacity: usize,
}

impl RingBuffer {
    fn push(&mut self, entry: LogEntry) {
        // NOTE: runs inside `Logger::log` — self_trace! only, never log::*!.
        // (Trace-mode trade-off: the stderr write happens while the ring
        // mutex is held; acceptable for an opt-in diagnostic level.)
        let evicted = self.buf.len() >= self.capacity;
        let level = entry.level.clone();
        if evicted {
            self.buf.pop_front();
        }
        self.buf.push_back(entry);
        let util_pct = (self.buf.len() * 100) / self.capacity.max(1);
        self_trace!(
            "ring push level={level} util={util_pct}% entries={}/{}{}",
            self.buf.len(),
            self.capacity,
            if evicted { " (evicted oldest)" } else { "" }
        );
    }
    fn tail(&self, n: usize) -> Vec<LogEntry> {
        self.buf
            .iter()
            .skip(self.buf.len().saturating_sub(n))
            .cloned()
            .collect()
    }
}

/// Global logger state.
/// Escape a string for embedding in a JSON string literal: handles quotes,
/// backslashes, and control characters (the old code escaped only `\"`,
/// producing malformed JSON for messages containing `\` or newlines).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

struct Logger {
    json: AtomicBool,
    ring: Mutex<RingBuffer>,
    file: Mutex<Option<FileState>>,
    component: String,
}

struct FileState {
    path: PathBuf,
    file: File,
    date: String,
    writes: usize,
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let ts = chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let level = record.level().to_string();
        let target = record.target().to_string();
        let message = format!("{}", record.args());

        let (file, line) = if record.level() <= log::Level::Debug {
            (record.file().map(|s| s.to_string()), record.line())
        } else {
            (None, None)
        };

        let entry = LogEntry {
            ts,
            level,
            target,
            file,
            line,
            message,
        };

        // ── ring buffer with poison recovery ──
        {
            let mut ring = match self.ring.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            ring.push(entry.clone());
        }

        // ── stderr (text or JSON) ──
        let mut stderr = io::stderr();
        if self.json.load(Ordering::Relaxed) {
            let json_line = match (&entry.file, entry.line) {
                (Some(f), Some(l)) => format!(
                    r#"{{"ts":"{}","level":"{}","target":"{}","file":"{f}","line":{l},"msg":"{}"}}"#,
                    entry.ts,
                    entry.level,
                    entry.target,
                    json_escape(&entry.message),
                ),
                _ => format!(
                    r#"{{"ts":"{}","level":"{}","target":"{}","msg":"{}"}}"#,
                    entry.ts,
                    entry.level,
                    entry.target,
                    json_escape(&entry.message),
                ),
            };
            let _ = writeln!(stderr, "{json_line}");
        } else {
            let loc = match (&entry.file, entry.line) {
                (Some(f), Some(l)) => format!(" [{f}:{l}]"),
                _ => String::new(),
            };
            let _ = writeln!(
                stderr,
                "{} {:<5} [{}]{} {}",
                entry.ts, entry.level, entry.target, loc, entry.message
            );
        }

        // ── rotated file with poison recovery ──
        {
            let mut guard = match self.file.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if let Some(state) = guard.as_mut() {
                let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                if state.date != today {
                    // Self-events, not log::*! — this runs inside Logger::log.
                    self_trace!("file rotation triggered: {} → {today}", state.date);
                    match rotate_file(state, &self.component) {
                        Ok(()) => self_trace!("file rotated to {}", state.path.display()),
                        Err(e) => self_trace!("file rotation failed: {e}"),
                    }
                    state.date = today;
                }
                let json_line = match (&entry.file, entry.line) {
                    (Some(f), Some(l)) => format!(
                        r#"{{"ts":"{}","level":"{}","target":"{}","file":"{f}","line":{l},"msg":"{}"}}"#,
                        entry.ts,
                        entry.level,
                        entry.target,
                        json_escape(&entry.message),
                    ),
                    _ => format!(
                        r#"{{"ts":"{}","level":"{}","target":"{}","msg":"{}"}}"#,
                        entry.ts,
                        entry.level,
                        entry.target,
                        json_escape(&entry.message),
                    ),
                };
                match writeln!(state.file, "{json_line}") {
                    Ok(_) => {
                        state.writes += 1;
                        self_trace!("file write ok");
                        if state.writes % 1000 == 0 {
                            self_trace!(
                                "retention sweep triggered (>{FILE_RETENTION_DAYS}d old logs)"
                            );
                            // Self-events only — this runs inside Logger::log.
                            match cleanup_old_logs(&state.path, FILE_RETENTION_DAYS) {
                                Ok(removed) => {
                                    self_trace!(
                                        "retention sweep removed {removed} old log file(s)"
                                    );
                                }
                                Err(e) => self_trace!("retention sweep failed: {e}"),
                            }
                        }
                    }
                    Err(e) => self_trace!("file write failed: {e}"),
                }
            }
        }
    }

    fn flush(&self) {
        let mut guard = match self.file.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if let Some(state) = guard.as_mut() {
            let _ = state.file.flush();
        }
        let _ = io::stderr().flush();
    }
}

fn rotate_file(state: &mut FileState, component: &str) -> io::Result<()> {
    state.file.flush()?;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let new_path = state
        .path
        .parent()
        .unwrap_or(PathBuf::from(".").as_path())
        .join(format!("{}-{}.log", component, today));
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&new_path)?;
    state.path = new_path;
    state.file = file;
    state.writes = 0;
    Ok(())
}

fn cleanup_old_logs(base: &Path, days: u64) -> io::Result<usize> {
    let dir = base.parent().unwrap_or(Path::new("."));
    let cutoff = SystemTime::now() - Duration::from_secs(days * 86400);
    let mut removed = 0usize;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("log") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(mtime) = meta.modified() {
                if mtime < cutoff {
                    // NOTE: called from inside `Logger::log` — self_trace! only,
                    // never log::*!.
                    match fs::remove_file(&path) {
                        Ok(()) => removed += 1,
                        Err(e) => {
                            self_trace!("retention sweep: cannot remove {}: {e}", path.display())
                        }
                    }
                }
            }
        }
    }
    Ok(removed)
}

fn log_dir() -> PathBuf {
    let base = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("legion-control");
    let _ = fs::create_dir_all(&base);
    base
}

static LOGGER: OnceLock<Logger> = OnceLock::new();
static ORIGINAL_FILTER: OnceLock<String> = OnceLock::new();

/// Initialise once. Safe to call from every binary.
pub fn init(component: &str) {
    let env_raw = std::env::var("LEGION_LOG").unwrap_or_else(|_| "info".into());
    let json_mode = env_raw.eq_ignore_ascii_case("json");
    let env_filter = if json_mode {
        "info".into()
    } else {
        env_raw.clone()
    };

    let _ = ORIGINAL_FILTER.set(env_filter.clone());

    // Bootstrap diagnostics run before the logger exists — stderr directly
    // (same convention as the log-file fallback below).
    let ring_raw = std::env::var("LEGION_LOG_RING").ok();
    let ring_size = match ring_raw.as_deref().map(str::parse::<usize>) {
        Some(Ok(n)) => n.clamp(100, MAX_RING_SIZE),
        Some(Err(_)) => {
            eprintln!(
                "legion-log: LEGION_LOG_RING={ring_raw:?} is not a number — using default {DEFAULT_RING_SIZE}"
            );
            DEFAULT_RING_SIZE
        }
        None => DEFAULT_RING_SIZE,
    };

    let file_enabled =
        std::env::var("LEGION_LOG_FILE").is_ok_and(|s| s == "1" || s.eq_ignore_ascii_case("true"));

    let file_state = if file_enabled {
        let dir = log_dir();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let path = dir.join(format!("{}-{}.log", component, today));
        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => Some(FileState {
                path,
                file,
                date: today,
                writes: 0,
            }),
            Err(e) => {
                eprintln!("legion-log: cannot open log file {}: {e}", path.display());
                None
            }
        }
    } else {
        None
    };

    let file_desc = match &file_state {
        Some(s) => format!("enabled ({})", s.path.display()),
        None => "disabled".to_string(),
    };

    let logger = Logger {
        json: AtomicBool::new(json_mode),
        ring: Mutex::new(RingBuffer {
            buf: VecDeque::with_capacity(ring_size),
            capacity: ring_size,
        }),
        file: Mutex::new(file_state),
        component: component.to_string(),
    };

    let _ = LOGGER.set(logger);

    let max_level = parse_level(&env_filter);
    let _ = log::set_logger(LOGGER.get().unwrap());
    log::set_max_level(max_level);

    // Bootstrap events — safe here: the sink is installed and none of these
    // paths re-enter it.
    log::debug!("init: LEGION_LOG={env_raw:?} → filter {max_level:?}");
    log::debug!("init: json_mode={json_mode} ring_size={ring_size}");
    log::debug!("init: file output {file_desc}");

    log::info!(
        "{component} v{} logging ready (max={max_level:?}, json={json_mode}, ring={ring_size}, file={file_enabled})",
        env!("CARGO_PKG_VERSION")
    );
}

/// Parse env_logger-style filter string (e.g. "info,legion_core=debug").
fn parse_level(s: &str) -> LevelFilter {
    let s = s.trim();
    if s.eq_ignore_ascii_case("json") {
        return LevelFilter::Info;
    }
    let first = s.split(',').next().unwrap_or("info").trim();
    match first.to_ascii_lowercase().as_str() {
        "off" => LevelFilter::Off,
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Info,
    }
}

/// Change the global max log level at runtime (e.g. from a GUI toggle).
pub fn set_max_level(filter: LevelFilter) {
    let previous = log::max_level();
    log::set_max_level(filter);
    if previous != filter {
        log::info!("log level {previous:?} → {filter:?}");
    } else {
        log::debug!("log level unchanged ({filter:?})");
    }
}

/// Return the last `n` log entries from the in-memory ring buffer.
pub fn recent_logs(n: usize) -> Vec<LogEntry> {
    LOGGER
        .get()
        .map(|l| {
            let ring = match l.ring.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            ring.tail(n)
        })
        .unwrap_or_default()
}

/// Format recent logs as plain text for a GUI viewer.
pub fn recent_logs_text(n: usize) -> String {
    recent_logs(n)
        .iter()
        .map(|e| {
            let loc = match (&e.file, e.line) {
                (Some(f), Some(l)) => format!(" [{f}:{l}]"),
                _ => String::new(),
            };
            format!("{} {:<5} [{}]{loc} {}", e.ts, e.level, e.target, e.message)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Re-read `LEGION_LOG` from the environment and apply it (call after SIGHUP).
pub fn reload_from_env() {
    match std::env::var("LEGION_LOG") {
        Ok(val) => {
            let lvl = parse_level(&val);
            log::debug!("reload_from_env: LEGION_LOG={val:?} → {lvl:?}");
            set_max_level(lvl);
        }
        Err(std::env::VarError::NotPresent) => {
            log::debug!("reload_from_env: LEGION_LOG not set — keeping current level");
        }
        Err(e) => {
            log::warn!("reload_from_env: cannot read LEGION_LOG ({e}) — keeping current level");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_valid_json_object(s: &str) -> bool {
        serde_json::from_str::<serde_json::Value>(s).is_ok()
    }

    #[test]
    fn json_escape_handles_backslash_newline_and_ctrl() {
        let msg = "a\"b\\c\nd\re\tf\u{0001}g";
        let escaped = json_escape(msg);
        assert_eq!(escaped, "a\\\"b\\\\c\\nd\\re\\tf\\u0001g");
        // Must round-trip as a valid JSON string literal.
        assert!(is_valid_json_object(&format!(r#"{{"msg":"{}"}}"#, escaped)));
    }

    #[test]
    fn json_escape_leaves_plain_text_untouched() {
        assert_eq!(json_escape("hello world"), "hello world");
    }

    #[test]
    fn json_log_line_is_valid_json_even_with_hostile_message() {
        let msg = "hello\"world\\oops\nnew\tline";
        let json_line = format!(r#"{{"msg":"{}"}}"#, json_escape(msg));
        assert!(
            is_valid_json_object(&json_line),
            "produced invalid JSON: {json_line}"
        );
    }

    #[test]
    fn json_escape_empty() {
        assert_eq!(json_escape(""), "");
    }

    #[test]
    fn parse_level_variants() {
        assert_eq!(parse_level("off"), LevelFilter::Off);
        assert_eq!(parse_level("ERROR"), LevelFilter::Error);
        assert_eq!(parse_level("warn"), LevelFilter::Warn);
        assert_eq!(parse_level("info"), LevelFilter::Info);
        assert_eq!(parse_level("DEBUG"), LevelFilter::Debug);
        assert_eq!(parse_level("trace"), LevelFilter::Trace);
        assert_eq!(parse_level("json"), LevelFilter::Info);
        assert_eq!(parse_level("unknown"), LevelFilter::Info);
        // env_logger style prefix
        assert_eq!(parse_level("debug,legion_core=info"), LevelFilter::Debug);
        assert_eq!(parse_level("  info  "), LevelFilter::Info);
    }

    #[test]
    fn ring_buffer_capacity_and_tail() {
        let mut ring = RingBuffer {
            buf: VecDeque::with_capacity(3),
            capacity: 3,
        };
        for i in 0..5 {
            ring.push(LogEntry {
                ts: format!("t{i}"),
                level: "INFO".into(),
                target: "test".into(),
                file: None,
                line: None,
                message: format!("m{i}"),
            });
        }
        assert_eq!(ring.buf.len(), 3);
        // Oldest two evicted.
        assert_eq!(ring.buf[0].message, "m2");
        assert_eq!(ring.tail(2).len(), 2);
        assert_eq!(ring.tail(2)[0].message, "m3");
        assert_eq!(ring.tail(10).len(), 3);
        assert_eq!(ring.tail(0).len(), 0);
    }

    #[test]
    fn cleanup_old_logs_ignores_non_log_files() {
        let dir = std::env::temp_dir().join(format!("legion-log-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("keep.txt"), "x").unwrap();
        std::fs::write(dir.join("keep.log"), "x").unwrap();
        // Should not error and must not delete keep.txt.
        let _ = cleanup_old_logs(&dir.join("dummy.log"), 7);
        assert!(dir.join("keep.txt").exists(), "non-log file was deleted");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
