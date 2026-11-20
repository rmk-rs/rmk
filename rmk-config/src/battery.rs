use serde::Deserialize;

use crate::{KeyboardTomlConfig, PinConfig, SplitBoardConfig};

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
        let configured = match (&self.split, peripheral) {
            (Some(split), Some(id)) => split
                .peripheral
                .get(id)
                .ok_or("Invalid battery peripheral index")?
                .battery
                .as_ref(),
            (Some(split), None) => split.central.battery.as_ref(),
            (None, Some(_)) => return Err("Unibody keyboard has no peripherals".into()),
            (None, None) => self.battery.as_ref(),
        };
        let mut config = configured.cloned().unwrap_or_default();
        if configured.is_none()
            && self.split.is_some()
            && let Some(keyboard) = &self.keyboard
            && (keyboard.board.is_some() || keyboard.chip.is_some())
        {
            let preset = self.get_chip_model()?.get_default_config_str()?;
            let mut preset: toml::Table = toml::from_str(preset).map_err(|e| format!("Invalid board preset: {e}"))?;
            if let Some(battery) = preset.remove("battery") {
                config = battery
                    .try_into()
                    .map_err(|e| format!("Invalid board battery preset: {e}"))?;
            }
        }
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

/// Bluetooth SIG presentation description and the human-readable GATT label.
pub(crate) struct BatteryDescriptor {
    description: u16,
    pub name: String,
}

const MAIN: u16 = 0x0106;
const LEFT: u16 = 0x010D;
const RIGHT: u16 = 0x010E;

impl BatteryDescriptor {
    fn new(description: u16, label: Option<String>) -> Self {
        let name = label.unwrap_or_else(|| match description {
            MAIN => "Central".into(),
            LEFT => "Left".into(),
            RIGHT => "Right".into(),
            ordinal => format!("Peripheral {}", ordinal - 1),
        });
        Self { description, name }
    }

    pub(crate) fn is_hand(&self) -> bool {
        matches!(self.description, LEFT | RIGHT)
    }

    pub(crate) fn format(&self) -> [u8; 7] {
        let [lo, hi] = self.description.to_le_bytes();
        // GATT 0x2904: uint8, exponent 0, percentage (0x27AD), Bluetooth SIG namespace, description.
        [0x04, 0, 0xAD, 0x27, 1, lo, hi]
    }
}

impl KeyboardTomlConfig {
    pub(crate) fn battery_descriptors(
        &self,
        split_enabled: bool,
    ) -> Result<(BatteryDescriptor, Vec<(usize, BatteryDescriptor)>), String> {
        let central = self.resolve_battery_config(None)?;
        let mut central_description = MAIN;
        let mut peripherals = Vec::new();
        if let Some(split) = self.split.as_ref().filter(|_| split_enabled) {
            for (id, board) in split.peripheral.iter().enumerate() {
                let config = self.resolve_battery_config(Some(id))?;
                if config.battery_adc_pin.is_none() {
                    continue;
                }
                let description = if split.peripheral.len() == 1
                    && central.battery_adc_pin.is_some()
                    && let Some([central_side, peripheral_side]) = horizontal_pair(&split.central, board)
                {
                    central_description = central_side;
                    peripheral_side
                } else {
                    u16::from(u8::try_from(id + 1).map_err(|_| "BLE battery peripheral ordinal exceeds 255")?)
                };
                peripherals.push((id, BatteryDescriptor::new(description, config.battery_user_description)));
            }
        }
        Ok((
            BatteryDescriptor::new(central_description, central.battery_user_description),
            peripherals,
        ))
    }
}

fn horizontal_pair(a: &SplitBoardConfig, b: &SplitBoardConfig) -> Option<[u16; 2]> {
    let first_row = a.row_offset.max(b.row_offset);
    let end_row = a
        .row_offset
        .saturating_add(a.rows)
        .min(b.row_offset.saturating_add(b.rows));
    if first_row >= end_row || a.cols == 0 || b.cols == 0 {
        return None;
    }
    if a.col_offset.saturating_add(a.cols) <= b.col_offset {
        Some([LEFT, RIGHT])
    } else if b.col_offset.saturating_add(b.cols) <= a.col_offset {
        Some([RIGHT, LEFT])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SplitConfig;

    #[test]
    fn battery_descriptors_follow_the_matrix_only_for_a_two_board_pair() {
        let board = |col_offset| SplitBoardConfig {
            rows: 3,
            cols: 4,
            col_offset,
            battery: Some(BatteryTomlConfig {
                battery_adc_pin: Some("vddh".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let resolve = |central, peripheral| {
            let mut config: KeyboardTomlConfig = toml::from_str("").unwrap();
            config.split = Some(SplitConfig {
                central,
                peripheral,
                ..Default::default()
            });
            config.battery_descriptors(true).unwrap()
        };
        let left = [4, 0, 0xAD, 0x27, 1, 0x0D, 1];
        let right = [4, 0, 0xAD, 0x27, 1, 0x0E, 1];
        let main = [4, 0, 0xAD, 0x27, 1, 0x06, 1];
        for (central_col, peripheral_col, a, b) in [
            (0, 4, (left, "Left"), (right, "Right")),
            (4, 0, (right, "Right"), (left, "Left")),
        ] {
            let (central, others) = resolve(board(central_col), vec![board(peripheral_col)]);
            assert!(central.is_hand());
            assert_eq!((central.format(), central.name.as_str()), a);
            assert_eq!((others[0].1.format(), others[0].1.name.as_str()), b);
        }
        let mut lower = board(0);
        lower.row_offset = 3;
        assert_eq!(resolve(board(0), vec![lower]).0.format(), main);
        let mut unpowered = board(8);
        unpowered.battery = None;
        let (central, others) = resolve(board(0), vec![unpowered, board(4)]);
        assert!(!central.is_hand());
        assert_eq!(central.format(), main);
        let [(id, numbered)] = others.as_slice() else {
            panic!("one measured peripheral")
        };
        assert_eq!(
            (*id, numbered.name.as_str(), numbered.format()),
            (1, "Peripheral 1", [4, 0, 0xAD, 0x27, 1, 2, 0])
        );
    }
}
