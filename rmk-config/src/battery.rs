use serde::Deserialize;

use crate::{KeyboardTomlConfig, PinConfig};

/// Battery wiring shared by unibody, central and peripheral boards.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatteryTomlConfig {
    pub battery_adc_pin: Option<String>,
    pub adc_divider_measured: Option<u32>,
    pub adc_divider_total: Option<u32>,
    pub charge_state: Option<PinConfig>,
    pub charge_led: Option<PinConfig>,
    pub battery_user_description: Option<String>,
}

impl KeyboardTomlConfig {
    /// Resolve the battery table for a unibody/central board or a peripheral.
    pub fn resolve_battery_config(&self, peripheral: Option<usize>) -> Result<BatteryTomlConfig, String> {
        if self.split.is_some() && self.battery.is_some() {
            return Err("Use [split.central.battery] and [split.peripheral.battery] for split keyboards".into());
        }
        let config = match (&self.split, peripheral) {
            (Some(split), Some(id)) => split
                .peripheral
                .get(id)
                .ok_or("Invalid battery peripheral index")?
                .battery
                .as_ref(),
            (Some(split), None) => split.central.battery.as_ref(),
            (None, Some(_)) => return Err("Unibody keyboard has no peripherals".into()),
            (None, None) => self.battery.as_ref(),
        }
        .cloned()
        .unwrap_or_default();
        if config.battery_adc_pin.is_none()
            && (config.adc_divider_measured.is_some() || config.adc_divider_total.is_some())
        {
            return Err("battery: ADC divider requires battery_adc_pin in the same table".into());
        }
        if config.charge_led.is_some() && config.battery_adc_pin.is_none() && config.charge_state.is_none() {
            return Err("battery.charge_led requires battery_adc_pin or charge_state on the same board".into());
        }
        if let Some(id) = peripheral
            && config.battery_user_description.is_some()
            && config.battery_adc_pin.is_none()
        {
            return Err(format!(
                "keyboard.toml: [[split.peripheral]] at index {} requires battery_adc_pin when battery_user_description is set",
                id
            ));
        }
        if config.battery_adc_pin.as_deref().is_some_and(|pin| pin != "vddh") {
            for (field, value) in [
                ("adc_divider_measured", config.adc_divider_measured),
                ("adc_divider_total", config.adc_divider_total),
            ] {
                if value == Some(0) {
                    return Err(format!("battery.{field} must be greater than zero"));
                }
            }
        }
        Ok(config)
    }
}
