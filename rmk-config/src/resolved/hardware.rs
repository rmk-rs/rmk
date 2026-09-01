//! Resolved hardware types for the public API of `rmk-config`.
//!
//! Leaf types are re-exported directly from the TOML configuration types
//! Only types with genuine structural transformation are defined here.

pub use crate::board::{BoardConfig, UniBodyConfig};
pub use crate::chip::{ChipModel, ChipSeries};
pub use crate::communication::{CommunicationConfig, UsbInfo};
use crate::validate_unlock_keys;
pub use crate::{
    BleConfig, ChipConfig, CommunicationProtocol, DependencyConfig, DisplayConfig, DisplayDriver, EncoderConfig,
    EncoderResolution, I2cConfig, InputDeviceConfig, Iqs5xxConfig, Iqs5xxI2cConfig, JoystickConfig, KeyInfo,
    LightConfig, MatrixConfig, MatrixType, OutputConfig, PinConfig, Pmw33xxConfig, Pmw33xxType, Pmw3610Config,
    PointingDeviceConfig, SerialConfig, SpiConfig, SplitBoardConfig, SplitConfig,
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
    pub page_size: u32,
    pub led: Option<PinConfig>,
    pub unlock_keys: Vec<[u8; 2]>,
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
        let dfu = match self.get_dfu_config() {
            Some(d) => {
                let unlock_keys = d.unlock_keys.clone().unwrap_or_default();
                validate_unlock_keys("[dfu]", &unlock_keys, self.layout.as_ref())?;
                Some(DfuConfig {
                    page_size: d.page_size.unwrap_or(4096),
                    led: d.led.clone().map(|pin| PinConfig { pin, low_active: false }),
                    unlock_keys,
                })
            }
            None => None,
        };
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
}
