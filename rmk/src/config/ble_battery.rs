/// Controls battery-level reporting over BLE.
///
/// Run local battery inputs and processors alongside the transport with `run_all!`.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(feature = "_nrf_ble"), derive(Default))]
pub struct BleBatteryConfig {
    pub enabled: bool,
}

impl BleBatteryConfig {
    pub fn disabled() -> Self {
        Self { enabled: false }
    }
}

#[cfg(feature = "_nrf_ble")]
impl Default for BleBatteryConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}
