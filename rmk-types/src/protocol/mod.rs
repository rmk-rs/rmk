//! Communication protocol definitions.
//!
//! RMK supports two host-communication protocols:
//!
//! - [`vial`] — Legacy Vial/Via protocol for compatibility with the Vial GUI.
//!   Always available.
//! - `rynk` — RMK native protocol. Carries `KeyAction`, `Combo`, `Morse`,
//!   `Fork`, `EncoderAction`, `BatteryStatus`, `BleStatus` on the wire over
//!   a 3-byte fixed header + postcard payload, COBS-framed. Enabled by the `rynk` feature.
//!
//! The two protocols are mutually exclusive at the firmware level
//! (`rynk` and `vial` features cannot be enabled together).

#[cfg(feature = "rynk")]
pub mod rynk;
pub mod vial;
