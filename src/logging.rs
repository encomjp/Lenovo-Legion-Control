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
use std::time::{Duration, Instant, SystemTime};

const DEFAULT_RING_SIZE: usize = 500;
const MAX_RING_SIZE: usize = 2000;
const FILE_RETENTION_DAYS: u64 = 7;

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
        if self.buf.len() >= self.capacity {
            self.buf.pop_front();
        }
        self.buf.push_back(entry);
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
    start: Instant,
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

        // ── ring buffer ──
        {
            let mut ring = self.ring.lock().unwrap();
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

        // ── rotated file ──
        {
            let mut guard = self.file.lock().unwrap();
            if let Some(state) = guard.as_mut() {
                let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                if state.date != today {
                    let _ = rotate_file(state, &self.component);
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
                let _ = writeln!(state.file, "{json_line}");
                state.writes += 1;
                if state.writes % 1000 == 0 {
                    let _ = cleanup_old_logs(&state.path, FILE_RETENTION_DAYS);
                }
            }
        }
    }

    fn flush(&self) {
        let mut guard = self.file.lock().unwrap();
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

fn cleanup_old_logs(base: &Path, days: u64) -> io::Result<()> {
    let dir = base.parent().unwrap_or(Path::new("."));
    let cutoff = SystemTime::now() - Duration::from_secs(days * 86400);
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("log") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(mtime) = meta.modified() {
                if mtime < cutoff {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }
    Ok(())
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

    let ring_size = std::env::var("LEGION_LOG_RING")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_RING_SIZE)
        .clamp(100, MAX_RING_SIZE);

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
                eprintln!("legion-log: cannot open log file: {e}");
                None
            }
        }
    } else {
        None
    };

    let logger = Logger {
        json: AtomicBool::new(json_mode),
        ring: Mutex::new(RingBuffer {
            buf: VecDeque::with_capacity(ring_size),
            capacity: ring_size,
        }),
        file: Mutex::new(file_state),
        start: Instant::now(),
        component: component.to_string(),
    };

    let _ = LOGGER.set(logger);

    let max_level = parse_level(&env_filter);
    let _ = log::set_logger(LOGGER.get().unwrap());
    log::set_max_level(max_level);

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
    log::set_max_level(filter);
    log::info!("log level changed to {filter:?}");
}

/// Retrieve the original `LEGION_LOG` string so it can be restored later.
pub fn original_filter() -> Option<&'static str> {
    ORIGINAL_FILTER.get().map(|s| s.as_str())
}

/// Return the last `n` log entries from the in-memory ring buffer.
pub fn recent_logs(n: usize) -> Vec<LogEntry> {
    LOGGER
        .get()
        .map(|l| {
            let ring = l.ring.lock().unwrap();
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
    if let Ok(val) = std::env::var("LEGION_LOG") {
        let lvl = parse_level(&val);
        set_max_level(lvl);
    }
}

/// Number of log entries currently in the ring buffer.
pub fn ring_len() -> usize {
    LOGGER
        .get()
        .map(|l| l.ring.lock().unwrap().buf.len())
        .unwrap_or(0)
}

/// Uptime since logger init, as a human-readable string.
pub fn uptime() -> String {
    let secs = LOGGER
        .get()
        .map(|l| l.start.elapsed().as_secs())
        .unwrap_or(0);
    format!("{}m {}s", secs / 60, secs % 60)
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
}
