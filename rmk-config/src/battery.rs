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
    /// New battery tables replace the corresponding legacy settings as a whole.
    /// This also serves build.rs, which need not resolve the board's transport.
    pub fn battery_config(&self, peripheral: Option<usize>) -> Result<BatteryTomlConfig, String> {
        if self.split.is_some() && self.battery.is_some() {
            return Err("Use [split.central.battery] and [split.peripheral.battery] for split keyboards".into());
        }
        let side = match (&self.split, peripheral) {
            (Some(split), Some(id)) => Some(split.peripheral.get(id).ok_or("Invalid battery peripheral index")?),
            (Some(split), None) => Some(&split.central),
            (None, Some(_)) => return Err("Unibody keyboard has no peripherals".into()),
            (None, None) => None,
        };
        let explicit = side.and_then(|board| board.battery.as_ref()).or(self.battery.as_ref());
        let config = if let Some(config) = explicit {
            if config.battery_adc_pin.is_none()
                && (config.adc_divider_measured.is_some() || config.adc_divider_total.is_some())
            {
                return Err("battery: ADC divider requires battery_adc_pin in the same table".into());
            }
            config.clone()
        } else {
            let fallback = self.ble.as_ref().filter(|_| peripheral.is_none());
            let own_adc = side.filter(|board| board.battery_adc_pin.is_some());
            let (pin, measured, total) = if let Some(board) = own_adc {
                (
                    board.battery_adc_pin.clone(),
                    board.adc_divider_measured,
                    board.adc_divider_total,
                )
            } else if let Some(ble) = fallback {
                (
                    ble.battery_adc_pin.clone(),
                    ble.adc_divider_measured,
                    ble.adc_divider_total,
                )
            } else {
                (None, None, None)
            };
            BatteryTomlConfig {
                battery_adc_pin: pin,
                adc_divider_measured: measured,
                adc_divider_total: total,
                charge_state: side
                    .and_then(|b| b.charge_state.clone())
                    .or_else(|| fallback.and_then(|b| b.charge_state.clone())),
                charge_led: side
                    .and_then(|b| b.charge_led.clone())
                    .or_else(|| fallback.and_then(|b| b.charge_led.clone())),
                battery_user_description: side
                    .and_then(|b| b.battery_user_description.clone())
                    .or_else(|| fallback.and_then(|b| b.battery_user_description.clone())),
            }
        };
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
