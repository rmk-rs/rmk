// embassy-nrf compiles `saadc` out for parts that lack it (nRF52820, nRF5340-net,
// nRF51), so this module can't build on `_no_saadc` chips.
#[cfg(all(feature = "_nrf_ble", not(feature = "_no_saadc")))]
pub mod nrf;

#[cfg(all(feature = "_nrf_ble", not(feature = "_no_saadc")))]
pub use nrf::*;

pub enum AnalogEventType {
    Joystick(u8),
    Battery,
}

#[derive(PartialEq)]
pub enum AdcState {
    Active,
    LightSleep,
    // DeepSleep,
}
