/// Controls central BLE battery-level notifications.
///
/// Configure charger GPIOs through `ChargingStateReader` and `BatteryLedProcessor`,
/// then run them with a `BatteryProcessor` alongside the transport using `run_all!`.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(all(feature = "_nrf", feature = "_ble")), derive(Default))]
pub struct BleBatteryConfig {
    /// Enables battery-level notifications for the central battery to the connected host.
    pub enabled: bool,
}

impl BleBatteryConfig {
    /// Disables central BLE battery-level notifications.
    pub fn disabled() -> Self {
        Self { enabled: false }
    }
}

#[cfg(all(feature = "_nrf", feature = "_ble"))]
impl Default for BleBatteryConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}
