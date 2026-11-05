// ESP32-S31 has no calibrated ADC driver in esp-hal yet.
#[cfg(all(feature = "_esp_ble", not(feature = "esp32s31_ble")))]
pub mod esp32;
#[cfg(feature = "rp2040")]
pub mod rp2040;

// embassy-nrf compiles `saadc` out for parts that lack it (nRF52820, nRF5340-net,
// nRF51), so this module can't build on `_no_saadc` chips.
#[cfg(any(test, all(feature = "_nrf", not(feature = "_no_saadc"))))]
pub mod nrf;

#[cfg(all(feature = "_nrf", not(feature = "_no_saadc")))]
pub use nrf::*;

pub enum AnalogEventType {
    Joystick(u8),
    Battery,
}
