//! Built-in processor module for RMK
//!
//! This module contains built-in processor implementations for output devices.

#[cfg(feature = "_ble")]
pub mod battery_led;
#[cfg(feature = "_dfu")]
pub mod dfu_led;
pub mod led_indicator;
pub mod wpm;
