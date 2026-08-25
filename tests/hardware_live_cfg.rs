//! Live self-test: config corruption handling — needs its OWN process so the
//! `config` crate-internal OnceLock store initializes fresh against the
//! corrupt file (in-process it would already be initialized by sibling tests).
//!
//! ```bash
//! cargo test --test hardware_live_cfg -- --ignored
//! ```

use legion_core::config;

#[test]
#[ignore = "live self-test"]
fn f24_config_corruption_preserved_not_lost() {
    let dir = std::env::temp_dir().join(format!("legion-selftest-corrupt-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("legion-control")).unwrap();
    // SAFETY: this binary has a single test; no parallel env readers.
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };

    std::fs::write(dir.join("legion-control/settings.json"), "{ broken json").unwrap();

    // First load must survive the corrupt file: defaults + preserved backup.
    let loaded = config::get();
    assert_eq!(loaded.brightness, 9, "expected schema-default brightness");
    assert_eq!(
        loaded.charge_limit, 100,
        "expected schema-default charge limit"
    );

    let kept_corrupt = std::fs::read_dir(dir.join("legion-control"))
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().contains(".corrupt-"));
    assert!(kept_corrupt, "corrupt settings.json was not preserved");

    unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
    let _ = std::fs::remove_dir_all(&dir);
}
