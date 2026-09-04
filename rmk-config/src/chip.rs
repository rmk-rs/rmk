use crate::{ChipConfig, KeyboardTomlConfig};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ChipSeries {
    Stm32,
    Nrf52,
    #[default]
    Rp2040,
    Esp32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChipModel {
    pub series: ChipSeries,
    pub chip: String,
    pub board: Option<String>,
}

impl ChipModel {
    pub fn get_default_config_str(&self) -> Result<&'static str, String> {
        if let Some(board) = self.board.clone() {
            match board.as_str() {
                "nice!nano_v2" | "nice!nano v2" => Ok(include_str!("default_config/nice_nano_v2.toml")),
                "nice!nano" | "nice!nano_v1" | "nicenano" => Ok(include_str!("default_config/nice_nano.toml")),
                "XIAO BLE" | "nrfmicro" | "bluemicro840" | "puchi_ble" => {
                    Ok(include_str!("default_config/nrf52840.toml"))
                }
                "Pi Pico W" | "Pico W" | "pi_pico_w" | "pico_w" => Ok(include_str!("default_config/pi_pico_w.toml")),
                _ => {
                    eprintln!("Fallback to use chip config for board: {}", board);
                    self.get_default_config_str_from_chip(&self.chip)
                }
            }
        } else {
            self.get_default_config_str_from_chip(&self.chip)
        }
    }

    fn get_default_config_str_from_chip(&self, chip: &str) -> Result<&'static str, String> {
        match chip {
            "nrf52840" => Ok(include_str!("default_config/nrf52840.toml")),
            "nrf52833" => Ok(include_str!("default_config/nrf52833.toml")),
            "nrf52832" => Ok(include_str!("default_config/nrf52832.toml")),
            "nrf52810" | "nrf52811" => Ok(include_str!("default_config/nrf52810.toml")),
            "rp2040" => Ok(include_str!("default_config/rp2040.toml")),
            s if s.starts_with("stm32") => Ok(include_str!("default_config/stm32.toml")),
            "esp32s3" => Ok(include_str!("default_config/esp32s3.toml")),
            "esp32s31" => Ok(include_str!("default_config/esp32s31.toml")),
            s if s.starts_with("esp32") => Ok(include_str!("default_config/esp32.toml")),
            _ => Err(format!(
                "No default chip config for {}, please report at https://github.com/rmk-rs/rmk/issues",
                self.chip
            )),
        }
    }
}

impl KeyboardTomlConfig {
    pub(crate) fn get_chip_model(&self) -> Result<ChipModel, String> {
        let keyboard = self
            .keyboard
            .as_ref()
            .ok_or_else(|| "[keyboard] section is required in keyboard.toml".to_string())?;
        if keyboard.board.is_none() == keyboard.chip.is_none() {
            return Err("Either \"board\" or \"chip\" should be set in keyboard.toml, but not both".to_string());
        }

        // Check board type
        if let Some(board) = keyboard.board.clone() {
            match board.as_str() {
                "nice!nano" | "nice!nano_v1" | "nicenano" | "nice!nano_v2" | "nice!nano v2" | "XIAO BLE"
                | "nrfmicro" | "bluemicro840" | "puchi_ble" => Ok(ChipModel {
                    series: ChipSeries::Nrf52,
                    chip: "nrf52840".to_string(),
                    board: Some(board),
                }),
                "Pi Pico W" | "Pico W" | "pi_pico_w" | "pico_w" => Ok(ChipModel {
                    series: ChipSeries::Rp2040,
                    chip: "rp2040".to_string(),
                    board: Some(board),
                }),
                _ => Err(format!(
                    "Unsupported board \"{board}\" — supported boards are nice!nano, nice!nano_v2, XIAO BLE, nrfmicro, bluemicro840, puchi_ble and Pi Pico W; for other hardware set `chip` instead"
                )),
            }
        } else if let Some(chip) = keyboard.chip.clone() {
            if chip.to_lowercase().starts_with("stm32") {
                Ok(ChipModel {
                    series: ChipSeries::Stm32,
                    chip,
                    board: None,
                })
            } else if chip.to_lowercase().starts_with("nrf52") {
                Ok(ChipModel {
                    series: ChipSeries::Nrf52,
                    chip,
                    board: None,
                })
            } else if chip.to_lowercase().starts_with("rp2040") {
                Ok(ChipModel {
                    series: ChipSeries::Rp2040,
                    chip,
                    board: None,
                })
            } else if chip.to_lowercase().starts_with("esp32") {
                Ok(ChipModel {
                    series: ChipSeries::Esp32,
                    chip,
                    board: None,
                })
            } else {
                Err(format!(
                    "Unsupported chip \"{chip}\" — supported chip families are stm32*, nrf52*, rp2040 and esp32*"
                ))
            }
        } else {
            Err("Neither board nor chip is specified".to_string())
        }
    }

    pub(crate) fn get_chip_config(&self) -> ChipConfig {
        // An unresolvable chip is reported by the resolution entry points; here it just
        // means there is no chip-specific config to look up.
        let Ok(chip_model) = self.get_chip_model() else {
            return ChipConfig::default();
        };
        self.chip
            .as_ref()
            .and_then(|chip_configs| chip_configs.get(&chip_model.chip))
            .cloned()
            .unwrap_or_default()
    }
}
