// `NrfAdc` is built on `embassy_nrf::saadc`, which embassy-nrf compiles out for
// parts that have no SAADC (nRF52820, nRF5340-net, nRF51). Gating only on
// `_nrf_ble` therefore makes RMK itself fail to build for those chips;
// `_no_saadc` (set by `nrf52820_ble`) opts them out.
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
