//! rmk's only test target, named `integration` after this directory.
//!
//! [`simulator`] is the harness every case runs on. `run_tests!` expands each
//! `scenarios/*.toml` into a `mod` of keyboard-behavior tests;
//! `scenarios/README.md` documents their syntax. [`rynk`], [`vial`],
//! [`ble_profile`], and [`storage`] hold what a scenario file cannot express:
//! wire-protocol writes interleaved with matrix input, what the BLE profile task
//! received, and what a keyboard restores from flash.

// The harness offers the whole step vocabulary, and each feature row plays a
// subset of it — so per-row dead code is expected, not a finding.
#![allow(dead_code)]

mod simulator;

#[cfg(feature = "_ble")]
mod ble_profile;
#[cfg(feature = "rynk")]
mod rynk;
#[cfg(feature = "storage")]
mod storage;
#[cfg(feature = "vial")]
mod vial;

rmk_macro::run_tests!("tests/scenarios");
