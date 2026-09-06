//! Resolved hardware types for the public API of `rmk-config`.
//!
//! Leaf types are re-exported directly from the TOML configuration types
//! Only types with genuine structural transformation are defined here.

pub use crate::board::{BoardConfig, UniBodyConfig};
pub use crate::chip::{ChipModel, ChipSeries};
pub use crate::communication::{CommunicationConfig, UsbInfo};
use crate::validate_unlock_keys;
pub use crate::{
    BleConfig, ChipConfig, CommunicationProtocol, DependencyConfig, DfuTomlConfig, DisplayConfig, DisplayDriver,
    EncoderConfig, EncoderResolution, ExternalFlashDriver, ExternalFlashTomlConfig, I2cConfig, InputDeviceConfig,
    Iqs5xxConfig, Iqs5xxI2cConfig, JoystickConfig, KeyInfo, LightConfig, MatrixConfig, MatrixType, OutputConfig,
    PinConfig, Pmw33xxConfig, Pmw33xxType, Pmw3610Config, PointingDeviceConfig, SerialConfig, SpiConfig,
    SplitBoardConfig, SplitConfig,
};

/// Resolved storage hardware config
pub struct Storage {
    pub start_addr: usize,
    pub num_sectors: u8,
    pub clear_storage: bool,
    pub clear_layout: bool,
}

/// Resolved DFU partition config
pub struct DfuConfig {
    pub led: Option<PinConfig>,
    pub unlock_keys: Vec<[u8; 2]>,
    pub external_flash: Option<ExternalFlashConfig>,
}

/// Resolved config for external SPI flash used as DFU partition.
pub struct ExternalFlashConfig {
    pub driver: ExternalFlashDriver,
    pub flash_size: u32,
    /// Size of the DFU download partition; defaults to `flash_size` when unset.
    pub dfu_partition_size: u32,
    pub init_fn: Option<String>,
    pub spi: SpiConfig,
}

/// Complete hardware configuration for init code generation.
pub struct Hardware {
    pub chip: ChipModel,
    pub chip_config: ChipConfig,
    pub communication: CommunicationConfig,
    pub board: BoardConfig,
    pub storage: Option<Storage>,
    pub dfu: Option<DfuConfig>,
    pub light: LightConfig,
    pub display: Option<DisplayConfig>,
    pub output: Vec<OutputConfig>,
    pub dependency: DependencyConfig,
}

impl crate::KeyboardTomlConfig {
    /// Resolve hardware configuration from TOML config.
    pub fn hardware(&self) -> Result<Hardware, String> {
        let chip = self.get_chip_model()?;
        let chip_config = self.get_chip_config();
        let communication = self.get_communication_config()?;
        let board = self.get_board_config()?;
        let storage_toml = self.get_storage_config();
        let storage = if storage_toml.enabled {
            Some(Storage {
                start_addr: storage_toml.start_addr.unwrap_or(0),
                num_sectors: if self.get_dfu_config().is_some() {
                    if self.storage_user_set {
                        storage_toml.num_sectors.unwrap_or(8)
                    } else {
                        8
                    }
                } else {
                    storage_toml.num_sectors.unwrap_or(2)
                },
                clear_storage: storage_toml.clear_storage.unwrap_or(false),
                clear_layout: storage_toml.clear_layout.unwrap_or(false),
            })
        } else {
            None
        };
        let dfu = self.split_side_dfu(None)?;
        let light = self.get_light_config();
        let display = self.get_display_config();
        let output = self.get_output_config()?;
        let dependency = self.get_dependency_config();
        Ok(Hardware {
            chip,
            chip_config,
            communication,
            board,
            storage,
            dfu,
            light,
            display,
            output,
            dependency,
        })
    }

    /// Resolve a raw TOML DFU section into the resolved [`DfuConfig`].
    ///
    /// `section` names the source for error messages (e.g. `"[dfu]"` or
    /// `"[split.peripheral[0].dfu]"`).
    fn resolve_dfu(&self, dfu: Option<&DfuTomlConfig>, section: &str) -> Result<Option<DfuConfig>, String> {
        match dfu {
            Some(d) => {
                let unlock_keys = d.unlock_keys.clone().unwrap_or_default();
                validate_unlock_keys(section, &unlock_keys, self.layout.as_ref())?;
                let external_flash = if let Some(ef) = &d.external_flash {
                    let init_fn = if matches!(ef.driver, ExternalFlashDriver::Custom) {
                        let path = ef
                            .init_fn
                            .as_ref()
                            .ok_or("[dfu.external_flash] init_fn is required when driver = \"custom\"")?;
                        Some(path.clone())
                    } else {
                        ef.init_fn.clone()
                    };
                    let dfu_partition_size = ef.dfu_partition_size.unwrap_or(ef.flash_size);
                    validate_dfu_partition_size(ef.flash_size, dfu_partition_size)?;
                    Some(ExternalFlashConfig {
                        driver: ef.driver.clone(),
                        flash_size: ef.flash_size,
                        dfu_partition_size,
                        init_fn,
                        spi: ef.spi.clone(),
                    })
                } else {
                    None
                };
                Ok(Some(DfuConfig {
                    led: d.led.clone().map(|pin| PinConfig { pin, low_active: false }),
                    unlock_keys,
                    external_flash,
                }))
            }
            None => Ok(None),
        }
    }

    /// Resolve the effective DFU config for a split side.
    ///
    /// `None` (the central side) uses `[split.central.dfu]` when present,
    /// otherwise the global `[dfu]` section. A peripheral's own
    /// `[split.peripheral[i].dfu]` section, when present, **completely
    /// replaces** the global one; otherwise the peripheral falls back to the
    /// global section too. A side's own section never merges with the global
    /// one.
    pub fn split_side_dfu(&self, side: Option<usize>) -> Result<Option<DfuConfig>, String> {
        match side {
            Some(id) => match self
                .split
                .as_ref()
                .and_then(|s| s.peripheral.get(id))
                .and_then(|p| p.dfu.as_ref())
            {
                Some(d) => self.resolve_dfu(Some(d), &format!("[split.peripheral[{id}].dfu]")),
                None => self.resolve_dfu(self.get_dfu_config().as_ref(), "[dfu]"),
            },
            None => match self.split.as_ref().and_then(|s| s.central.dfu.as_ref()) {
                Some(d) => self.resolve_dfu(Some(d), "[split.central.dfu]"),
                None => self.resolve_dfu(self.get_dfu_config().as_ref(), "[dfu]"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::KeyboardTomlConfig;

    fn config_from_toml(toml: &str) -> KeyboardTomlConfig {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("rmk-config-hw-{}-{}.toml", std::process::id(), unique));
        fs::write(&path, toml).unwrap();
        let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
        let _ = fs::remove_file(&path);
        config
    }

    fn external_flash_toml(dfu_partition_size: Option<u32>) -> String {
        format!(
            r#"
[keyboard]
name = "test"
product_name = "test"
vendor_id = 0x4c4b
product_id = 0x4643
manufacturer = "RMK"
chip = "rp2040"
usb_enable = true

[dfu.external_flash]
driver = "w25q"
flash_size = 8388608
{partition_line}
[dfu.external_flash.spi]
instance = "SPI0"
sck = "PIN_18"
mosi = "PIN_19"
miso = "PIN_16"

[layout]
rows = 1
cols = 1

[matrix]
row_pins = ["PIN_9"]
col_pins = ["PIN_10"]
"#,
            partition_line = dfu_partition_size
                .map(|s| format!("dfu_partition_size = {s}\n"))
                .unwrap_or_default(),
        )
    }

    #[test]
    fn dfu_partition_size_defaults_to_full_flash() {
        let config = config_from_toml(&external_flash_toml(None));
        let dfu = config.hardware().unwrap().dfu.unwrap();
        assert_eq!(dfu.external_flash.unwrap().dfu_partition_size, 8388608);
    }

    #[test]
    fn dfu_partition_size_accepts_sector_aligned_subset() {
        let config = config_from_toml(&external_flash_toml(Some(2097152)));
        let dfu = config.hardware().unwrap().dfu.unwrap();
        assert_eq!(dfu.external_flash.unwrap().dfu_partition_size, 2097152);
    }

    #[test]
    fn dfu_partition_size_rejects_unaligned_size() {
        let config = config_from_toml(&external_flash_toml(Some(12345)));
        let err = config.hardware().err().expect("expected validation error");
        assert!(err.contains("multiple of 4096"), "unexpected error: {err}");
    }

    #[test]
    fn dfu_partition_size_rejects_size_beyond_flash() {
        for size in [0, 8388609, u32::MAX] {
            let config = config_from_toml(&external_flash_toml(Some(size)));
            let err = config.hardware().err().expect("expected validation error");
            assert!(err.contains("between 1 and"), "unexpected error for {size}: {err}");
        }
    }
}

/// The DFU download partition must fit the flash and respect the sector erase
/// granularity, matching what [`crate::resolved`] hands to codegen and what
/// the W25Q driver can actually erase.
fn validate_dfu_partition_size(flash_size: u32, dfu_partition_size: u32) -> Result<(), String> {
    if dfu_partition_size == 0 || dfu_partition_size > flash_size {
        return Err(format!(
            "[dfu.external_flash] dfu_partition_size ({dfu_partition_size}) must be between 1 and flash_size ({flash_size})"
        ));
    }
    if !dfu_partition_size.is_multiple_of(4096) {
        return Err(format!(
            "[dfu.external_flash] dfu_partition_size ({dfu_partition_size}) must be a multiple of 4096 (sector erase size)"
        ));
    }
    Ok(())
}
