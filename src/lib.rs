//! Legion Core — hardware abstraction layer for Lenovo Legion laptops.
//!
//! Reads sensors via sysfs/hwmon, controls fans via WMI, and manages
//! keyboard RGB via USB HID. Designed for the Legion Pro 7 Gen 10 (83RU)
//! and compatible with all 2020-2026 Legion models.

pub mod audio;
pub mod battery;
pub mod comms;
pub mod config;
pub mod cpu;
pub mod cpu_percore;
pub mod device;
pub mod dgpu;
pub mod diagnostics;
pub mod fans;
pub mod intel;
pub mod intel_msr;
pub mod keyboard;
pub mod logging;
pub mod models;
pub mod profile;
pub mod rgb_panic;
pub mod selftest;
pub mod sensors;
pub mod thermal;
pub mod undervolt;
