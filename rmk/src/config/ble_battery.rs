/// Controls BLE battery-level notifications.
///
/// Configure charger GPIOs through `ChargingStateReader` and `BatteryLedProcessor`,
/// then run them with a `BatteryProcessor` alongside the transport using `run_all!`.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(feature = "_nrf_ble"), derive(Default))]
pub struct BleBatteryConfig {
    /// Enables battery-level notifications to the connected host.
    pub enabled: bool,
}

impl BleBatteryConfig {
    /// Disables BLE battery-level notifications.
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
