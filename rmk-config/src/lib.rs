use std::collections::HashMap;
use std::path::Path;

use config::{Config, File, FileFormat};
use serde::{Deserialize, de};
use serde_inline_default::serde_inline_default;

/// Event channel default configuration
const EVENT_DEFAULT_CONFIG: &str = include_str!("default_config/event_default.toml");

pub(crate) mod chip;
pub(crate) mod communication;
pub mod resolved;
#[rustfmt::skip]
pub mod usb_interrupt_map;
pub(crate) mod behavior;
pub(crate) mod board;
pub(crate) mod dfu;
pub(crate) mod display;
pub(crate) mod host;
pub(crate) mod keycode_alias;
pub(crate) mod keymap;
pub mod layout;
pub use layout::{STOCK_WIDTHS, layout_blob_from_toml, layout_info_from_toml};
pub(crate) mod light;
pub(crate) mod storage;

/// Protocol-level capacity ceilings for wire-format Vec sizes.
///
/// These define the maximum values any firmware may use for protocol
/// Vec capacities (`COMBO_SIZE`, `MORSE_SIZE`, etc.). The host tool compiles
/// against these as upper bounds. Any firmware with `rynk` enabled
/// must satisfy `value <= ceiling` at compile time.
///
/// Constant names mirror the generated constants with a `MAX_` prefix:
/// `COMBO_SIZE` is bounded by `MAX_COMBO_SIZE`, etc.
pub mod protocol_limits {
    /// Max keys in a combo trigger — ceiling for `COMBO_SIZE`
    pub const MAX_COMBO_SIZE: usize = 16;
    /// Max pattern entries per morse key — ceiling for `MORSE_SIZE`
    pub const MAX_MORSE_SIZE: usize = 32;
    /// Max bytes per macro data chunk — ceiling for `MACRO_DATA_SIZE`
    pub const MAX_MACRO_DATA_SIZE: usize = 256;
    /// Max key positions in an unlock challenge.
    pub const MAX_UNLOCK_KEYS_SIZE: usize = 4;
}

pub(crate) fn validate_unlock_keys(
    section: &str,
    unlock_keys: &[[u8; 2]],
    layout: Option<&LayoutTomlConfig>,
) -> Result<(), String> {
    if unlock_keys.len() > protocol_limits::MAX_UNLOCK_KEYS_SIZE {
        return Err(format!(
            "{section}.unlock_keys has {} entries, the max is {}",
            unlock_keys.len(),
            protocol_limits::MAX_UNLOCK_KEYS_SIZE
        ));
    }

    if let Some(layout) = layout {
        for key in unlock_keys {
            let (row, col) = (key[0], key[1]);
            if row >= layout.rows || col >= layout.cols {
                return Err(format!(
                    "{section}.unlock_keys position ({row}, {col}) is outside the {}x{} matrix",
                    layout.rows, layout.cols
                ));
            }
        }
    }

    Ok(())
}

/// Configurations for RMK keyboard.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(unused)]
pub struct KeyboardTomlConfig {
    /// Basic keyboard info
    keyboard: Option<KeyboardInfo>,
    /// Matrix of the keyboard, only for non-split keyboards
    matrix: Option<MatrixConfig>,
    // Aliases for key maps
    aliases: Option<HashMap<String, String>>,
    /// Keymap config: layer count and the per-layer key actions (`[[keymap.layer]]`).
    keymap: Option<KeymapTomlConfig>,
    /// Layout config: the physical key arrangement (`map`) plus the rendered layout.
    /// For split keyboards, the total row/col is defined in this section.
    layout: Option<LayoutTomlConfig>,
    /// Behavior config
    behavior: Option<BehaviorConfig>,
    /// Light config
    light: Option<LightConfig>,
    /// Storage config
    storage: Option<StorageConfig>,
    /// DFU partition config (embassy-boot)
    dfu: Option<DfuTomlConfig>,
    /// Ble config
    pub(crate) ble: Option<BleConfig>,
    /// Chip-specific configs (e.g., [chip.nrf52840])
    chip: Option<HashMap<String, ChipConfig>>,
    /// Dependency config
    dependency: Option<DependencyConfig>,
    /// Split config
    split: Option<SplitConfig>,
    /// Input device config
    input_device: Option<InputDeviceConfig>,
    /// Display config
    display: Option<DisplayConfig>,
    /// Output Pin config
    output: Option<Vec<OutputConfig>>,
    /// Set host configurations
    pub(crate) host: Option<HostConfig>,
    /// RMK config constants
    #[serde(default)]
    pub(crate) rmk: RmkConstantsConfig,
    /// Event channel configuration
    /// Default values are loaded from event_default.toml in new_from_toml_path()
    /// build.rs also loads event defaults via new_from_toml_path_with_event_defaults()
    #[serde(default)]
    pub(crate) event: EventConfig,
    /// Whether the user explicitly set a [storage] section in keyboard.toml.
    #[serde(skip)]
    pub(crate) storage_user_set: bool,
    /// Whether the user explicitly set `[storage]` `start_addr`/`num_sectors`
    /// in keyboard.toml (chip defaults don't count).
    #[serde(skip)]
    pub(crate) storage_start_addr_user_set: bool,
    #[serde(skip)]
    pub(crate) storage_num_sectors_user_set: bool,
    /// Whether the user explicitly wrote a [dfu] section in keyboard.toml
    /// (chip defaults contain an empty [dfu] too).
    #[serde(skip)]
    pub(crate) dfu_user_set: bool,
}

impl KeyboardTomlConfig {
    fn parse_from_toml_path<P: AsRef<Path>>(config_toml_path: P, chip_default_config: Option<&str>) -> Self {
        let path = config_toml_path.as_ref();
        let path_str = path
            .to_str()
            .unwrap_or_else(|| panic!("Config path is not valid UTF-8: {:?}", path));

        let mut builder = Config::builder().add_source(File::from_str(EVENT_DEFAULT_CONFIG, FileFormat::Toml));
        if let Some(default_config) = chip_default_config {
            builder = builder.add_source(File::from_str(default_config, FileFormat::Toml));
        }
        builder
            .add_source(File::with_name(path_str))
            .build()
            .unwrap_or_else(|e| panic!("Parse {:?} error: {}", path, e))
            .try_deserialize()
            .unwrap_or_else(|e| panic!("Deserialize {:?} error: {}", path, e))
    }

    /// Load keyboard.toml with event defaults but without chip-specific defaults.
    ///
    /// This is used in build.rs to resolve compile-time constants without
    /// requiring `[keyboard.board]`/`[keyboard.chip]`.
    pub fn new_from_toml_path_with_event_defaults<P: AsRef<Path>>(config_toml_path: P) -> Self {
        let mut config = Self::parse_from_toml_path(config_toml_path, None);
        let storage = config.storage;
        config.set_storage_user_flags(storage.as_ref());
        config.dfu_user_set = config.dfu.is_some();
        config.auto_calculate_parameters();
        config
    }

    pub fn new_from_toml_path<P: AsRef<Path>>(config_toml_path: P) -> Self {
        let path = config_toml_path.as_ref();

        // First pass: load user config with event defaults to get chip model.
        // This allows user's keyboard.toml to omit [event] section.
        let user_config = Self::parse_from_toml_path(path, None);

        let default_config_str = user_config
            .get_chip_model()
            .and_then(|chip| chip.get_default_config_str())
            .unwrap_or_else(|e| panic!("❌ keyboard.toml error: {e}"));

        // Second pass: load with all three config sources
        // Config priority (later sources override earlier ones):
        // 1. Event default config (lowest priority)
        // 2. Chip-specific default config
        // 3. User config (highest priority)
        let mut config = Self::parse_from_toml_path(path, Some(default_config_str));
        config.set_storage_user_flags(user_config.storage.as_ref());
        config.dfu_user_set = user_config.dfu.is_some();

        config.auto_calculate_parameters();

        config
    }

    /// Record which `[storage]` keys the user explicitly set, so DFU-related
    /// checks can distinguish them from chip default values.
    fn set_storage_user_flags(&mut self, user_storage: Option<&StorageConfig>) {
        self.storage_user_set = user_storage.is_some_and(|s| s.start_addr.is_some() || s.num_sectors.is_some());
        self.storage_start_addr_user_set = user_storage.is_some_and(|s| s.start_addr.is_some());
        self.storage_num_sectors_user_set = user_storage.is_some_and(|s| s.num_sectors.is_some());
    }

    /// Detect a `[storage]`/`[dfu]` conflict in the user's keyboard.toml.
    ///
    /// While DFU is enabled, the storage region is a partition fixed by the
    /// bootloader's linker script (`rmk-memory.x`, generated by rmk-boot's
    /// build.rs): `start_addr` is overridden to the partition start and
    /// `num_sectors` must match the partition size. Explicit `[storage]`
    /// values have no effect there, so return which keys the user set.
    pub fn dfu_storage_conflict(&self) -> Option<DfuStorageConflict> {
        if !self.dfu_user_set || !self.storage_user_set {
            return None;
        }
        let conflict = DfuStorageConflict {
            start_addr_set: self.storage_start_addr_user_set,
            num_sectors_set: self.storage_num_sectors_user_set,
        };
        (conflict.start_addr_set || conflict.num_sectors_set).then_some(conflict)
    }

    /// Auto calculate some parameters in toml:
    /// - Update morse_max_num to fit all configured morses
    /// - Update max_patterns_per_key to fit the max number of configured (pattern, action) pairs per morse key
    /// - Update peripheral number based on the number of split boards
    /// - TODO: Update controller number based on the number of split boards
    pub(crate) fn auto_calculate_parameters(&mut self) {
        // Update the number of peripherals
        if let Some(split) = &self.split
            && split.peripheral.len() > self.rmk.split_peripherals_num
        {
            // eprintln!(
            //     "The number of split peripherals is updated to {} from {}",
            //     split.peripheral.len(),
            //     self.rmk.split_peripherals_num
            // );
            self.rmk.split_peripherals_num = split.peripheral.len();
        }

        if let Some(behavior) = &self.behavior {
            // Update the max_patterns_per_key
            if let Some(morse) = &behavior.morse
                && let Some(morses) = &morse.morses
            {
                let mut max_required_patterns = self.rmk.max_patterns_per_key;

                for morse in morses {
                    let tap_actions_len = morse.tap_actions.as_ref().map(|v| v.len()).unwrap_or(0);
                    let hold_actions_len = morse.hold_actions.as_ref().map(|v| v.len()).unwrap_or(0);

                    let n = tap_actions_len.max(hold_actions_len);
                    if n > 15 {
                        panic!("The number of taps per morse is too large, the max number of taps is 15, got {n}");
                    }

                    let morse_actions_len = morse.morse_actions.as_ref().map(|v| v.len()).unwrap_or(0);

                    max_required_patterns =
                        max_required_patterns.max(tap_actions_len + hold_actions_len + morse_actions_len);
                }
                self.rmk.max_patterns_per_key = max_required_patterns;

                // Update the morse_max_num
                self.rmk.morse_max_num = self.rmk.morse_max_num.max(morses.len());
            }

            let auto_mouse_layers = behavior.auto_mouse_layer.as_deref().unwrap_or_default();
            self.rmk.auto_mouse_layer_max_num.get_or_insert(auto_mouse_layers.len());
        } else {
            self.rmk.auto_mouse_layer_max_num.get_or_insert(0);
        }
    }
}

/// Keyboard constants configuration for performance and hardware limits
#[serde_inline_default]
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RmkConstantsConfig {
    /// Mouse key interval (ms) - controls mouse movement speed
    #[serde_inline_default(20)]
    pub mouse_key_interval: u16,
    /// Mouse wheel interval (ms) - controls scrolling speed
    #[serde_inline_default(80)]
    pub mouse_wheel_interval: u16,
    /// Maximum number of combos keyboard can store
    #[serde_inline_default(8)]
    #[serde(deserialize_with = "check_combo_max_num")]
    pub combo_max_num: usize,
    /// Maximum number of keys pressed simultaneously in a combo
    #[serde_inline_default(4)]
    pub combo_max_length: usize,
    /// Maximum number of forks for conditional key actions
    #[serde_inline_default(8)]
    #[serde(deserialize_with = "check_fork_max_num")]
    pub fork_max_num: usize,
    /// Maximum number of morses keyboard can store
    #[serde_inline_default(8)]
    #[serde(deserialize_with = "check_morse_max_num")]
    pub morse_max_num: usize,
    /// Capacity of the morse profile table (named profiles in `[behavior.morse.profiles]`)
    #[serde_inline_default(16)]
    #[serde(deserialize_with = "check_morse_profile_max_num")]
    pub morse_profile_max_num: usize,
    /// Maximum number of patterns a morse key can handle
    #[serde_inline_default(8)]
    #[serde(deserialize_with = "check_max_patterns_per_key")]
    pub max_patterns_per_key: usize,
    /// Macro space size in bytes for storing sequences
    #[serde_inline_default(256)]
    pub macro_space_size: usize,
    /// Default debounce time in ms
    #[serde_inline_default(20)]
    pub debounce_time: u16,
    /// Report channel size
    #[serde_inline_default(16)]
    pub report_channel_size: usize,
    /// Vial channel size
    #[serde_inline_default(4)]
    pub vial_channel_size: usize,
    /// Flash channel size
    #[serde_inline_default(4)]
    pub flash_channel_size: usize,
    /// The number of the split peripherals
    #[serde_inline_default(0)]
    pub split_peripherals_num: usize,
    /// The number of available BLE profiles
    #[serde_inline_default(3)]
    pub ble_profiles_num: usize,
    /// BLE Split Central sleep timeout in seconds (0 = disabled)
    #[serde_inline_default(0)]
    pub split_central_sleep_timeout_seconds: u32,
    /// Maximum macro data chunk size for protocol transfers (bytes).
    /// Smaller values reduce firmware RAM usage but require more round-trips.
    #[serde_inline_default(64)]
    pub protocol_macro_chunk_size: usize,
    /// Maximum number of auto mouse layer entries; auto-derived from `[[behavior.auto_mouse_layer]]` if unset.
    #[serde(default)]
    pub auto_mouse_layer_max_num: Option<usize>,
    /// Exact RAM of each Rynk RX/TX frame buffer (bytes), payload capacity and bulk counts derive from it.
    /// Default 488 fills exactly two BLE notifications.
    #[serde_inline_default(488)]
    pub rynk_buffer_size: usize,
    /// Length of one dongle pairing window in seconds: repeated while no
    /// keyboard is bonded, opened once at power-on otherwise (dongle firmware only)
    #[serde_inline_default(30)]
    pub dongle_pairing_window_secs: u32,
}

fn check_combo_max_num<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = Deserialize::deserialize(deserializer)?;
    if value > u8::MAX as usize {
        return Err(de::Error::custom(format!(
            "combo_max_num must be between 0 and 255, got {value}"
        )));
    }
    Ok(value)
}

fn check_morse_max_num<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = Deserialize::deserialize(deserializer)?;
    if value > u8::MAX as usize {
        return Err(de::Error::custom(format!(
            "morse_max_num must be between 0 and 255, got {value}"
        )));
    }
    Ok(value)
}

/// The profile index is a `u8` in `KeyAction::TapHold` and an index with no
/// table entry means "use the default profile", so the table may never cover
/// the full `u8` range: capacity ≤ 255 keeps at least one index always vacant.
fn check_morse_profile_max_num<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = Deserialize::deserialize(deserializer)?;
    if value > 255 {
        panic!("❌ Parse `keyboard.toml` error: morse_profile_max_num must be between 0 and 255, got {value}");
    }
    Ok(value)
}

fn check_max_patterns_per_key<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = Deserialize::deserialize(deserializer)?;
    if !(4..=65536).contains(&value) {
        return Err(de::Error::custom(format!(
            "max_patterns_per_key must be between 4 and 65536, got {value}"
        )));
    }
    Ok(value)
}

fn check_fork_max_num<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = Deserialize::deserialize(deserializer)?;
    if value > u8::MAX as usize {
        return Err(de::Error::custom(format!(
            "fork_max_num must be between 0 and 255, got {value}"
        )));
    }
    Ok(value)
}

/// This separate Default impl is needed when `[rmk]` section is not set in keyboard.toml
impl Default for RmkConstantsConfig {
    fn default() -> Self {
        Self {
            mouse_key_interval: 20,
            mouse_wheel_interval: 80,
            combo_max_num: 8,
            combo_max_length: 4,
            fork_max_num: 8,
            morse_max_num: 8,
            morse_profile_max_num: 16,
            max_patterns_per_key: 8,
            macro_space_size: 256,
            debounce_time: 20,
            report_channel_size: 16,
            vial_channel_size: 4,
            flash_channel_size: 4,
            split_peripherals_num: 0,
            ble_profiles_num: 3,
            split_central_sleep_timeout_seconds: 0,
            protocol_macro_chunk_size: 64,
            auto_mouse_layer_max_num: None,
            rynk_buffer_size: 488,
            dongle_pairing_window_secs: 30,
        }
    }
}

/// Event channel configuration for a single event type
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EventChannelConfig {
    /// Channel buffer size
    pub channel_size: usize,
    /// Number of publishers
    pub pubs: usize,
    /// Number of subscribers
    pub subs: usize,
}

impl Default for EventChannelConfig {
    fn default() -> Self {
        Self {
            channel_size: 1,
            pubs: 1,
            subs: 1,
        }
    }
}

/// Macro to define EventConfig and related code without repetition
macro_rules! define_event_config {
    ($($field:ident),* $(,)?) => {
        /// Event configuration for all controller events
        /// Default values are loaded from event_default.toml
        #[derive(Clone, Debug, Deserialize)]
        #[serde(deny_unknown_fields, default)]
        pub(crate) struct EventConfig {
            $(pub $field: EventChannelConfig,)*
        }

        /// Cached default EventConfig parsed from event_default.toml
        static EVENT_CONFIG_DEFAULTS: std::sync::LazyLock<EventConfig> = std::sync::LazyLock::new(|| {
            #[derive(Deserialize)]
            struct Inner { $($field: EventChannelConfig,)* }
            #[derive(Deserialize)]
            struct Wrapper { event: Inner }
            let w: Wrapper = toml::from_str(EVENT_DEFAULT_CONFIG).expect("Failed to parse event_default.toml");
            EventConfig { $($field: w.event.$field,)* }
        });

        impl Default for EventConfig {
            fn default() -> Self {
                EVENT_CONFIG_DEFAULTS.clone()
            }
        }
    };
}

define_event_config!(
    // Connection events
    connection_status_change,
    // Input events
    modifier,
    keyboard,
    // Keyboard state events
    layer_change,
    wpm_update,
    led_indicator,
    sleep_state,
    // Power events
    battery_status,
    battery_adc,
    charging_state,
    // Pointing device events
    pointing,
    // Split events
    peripheral_connected,
    central_connected,
    peripheral_battery,
    clear_peer,
    // DFU events
    dfu_status,
    // Action events
    action,
);

/// The `[layout]` section: the physical key arrangement plus the rendered layout.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(unused)]
pub(crate) struct LayoutTomlConfig {
    pub rows: u8,
    pub cols: u8,
    /// The physical arrangement: an ordered map of `(row,col)` positions with
    /// optional hand, shape (`@2u`), gaps (`[1.5]`), row-steps (`[y=]`), and
    /// encoders (`(e,0)`). Its order also defines the order of `[[keymap.layer]]`.
    pub map: Option<String>,
    // Rendered-layout fields.
    pub default_variant: Option<String>,
    pub shapes: Option<HashMap<String, ShapeToml>>,
    pub variant: Option<Vec<VariantToml>>,
}

/// A named shape from `[layout.shapes]`. Every field optional; widths/
/// heights default to 1u, nudges/rotation to 0, and `w2/h2/x2/y2` are an
/// optional second rectangle for L-shaped caps.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShapeToml {
    pub w: Option<f32>,
    pub h: Option<f32>,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub r: Option<f32>,
    pub w2: Option<f32>,
    pub h2: Option<f32>,
    pub x2: Option<f32>,
    pub y2: Option<f32>,
}

/// One `[[layout.variant]]` render overlay: reshape some keys, hide others.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VariantToml {
    pub name: String,
    pub shapes: Option<HashMap<String, String>>,
    pub hidden: Option<Vec<String>>,
}

/// The `[keymap]` section: layer count plus the per-layer key actions.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(unused)]
pub(crate) struct KeymapTomlConfig {
    /// Total layer count. Optional — defaults to the number of `[[keymap.layer]]`
    /// blocks; set it larger to reserve extra empty layers (e.g. for Vial/Rynk).
    pub layers: Option<u8>,
    /// Per-layer key actions: `[[keymap.layer]]`.
    #[serde(default)]
    pub layer: Vec<LayerTomlConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(unused)]
pub(crate) struct LayerTomlConfig {
    pub name: Option<String>,
    pub keys: String,
    pub encoders: Option<Vec<[String; 2]>>,
}

/// Configurations for keyboard info
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyboardInfo {
    /// Keyboard name
    pub name: String,
    /// Vender id
    pub vendor_id: u16,
    /// Product id
    pub product_id: u16,
    /// Manufacturer
    pub manufacturer: Option<String>,
    /// Product name, if not set, it will use `name` as default
    pub product_name: Option<String>,
    /// Serial number
    pub serial_number: Option<String>,
    /// Board name(if a supported board is used)
    pub board: Option<String>,
    /// Chip model
    pub chip: Option<String>,
    /// enable usb
    pub usb_enable: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub enum MatrixType {
    #[default]
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "direct_pin")]
    DirectPin,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DebouncerType {
    #[default]
    Default,
    Fast,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatrixConfig {
    #[serde(default)]
    pub matrix_type: MatrixType,
    pub row_pins: Option<Vec<String>>,
    pub col_pins: Option<Vec<String>>,
    pub direct_pins: Option<Vec<Vec<String>>>,
    #[serde(default = "default_true")]
    pub direct_pin_low_active: bool,
    #[serde(default = "default_false")]
    pub row2col: bool,
    #[serde(default)]
    pub debouncer: DebouncerType,
    pub bootmagic: Option<(u8, u8)>,
}

/// Which `[storage]` keys the user explicitly set while `[dfu]` is enabled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DfuStorageConflict {
    pub start_addr_set: bool,
    pub num_sectors_set: bool,
}

/// Config for storage
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StorageConfig {
    /// Start address of local storage, MUST BE start of a sector.
    /// If start_addr is set to 0(this is the default value), the last `num_sectors` sectors will be used.
    pub start_addr: Option<usize>,
    // Number of sectors used for storage, >= 2.
    pub num_sectors: Option<u8>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    // Clear on the storage at reboot, set this to true if you want to reset the keymap
    pub clear_storage: Option<bool>,
    // Clear on the layout at reboot, set this to true if you want to reset the layout
    pub clear_layout: Option<bool>,
}

/// Config for DFU (embassy-boot).
///
/// Offsets come from `rmk-memory.x` linker symbols. This section only
/// configures DFU behaviour (LED, unlock keys, page size).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DfuTomlConfig {
    /// Flash page size in bytes (e.g. 4096 for RP2040).
    pub page_size: Option<u32>,
    /// Optional DFU activity LED pin, e.g. `"PIN_16"`. When set, the LED
    /// is lit while a DFU download is in progress.
    pub led: Option<String>,
    /// Unlock keys for DFU lock (optional)
    pub unlock_keys: Option<Vec<[u8; 2]>>,
}

#[derive(Clone, Default, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BleConfig {
    pub enabled: bool,
    pub battery_adc_pin: Option<String>,
    /// User-facing description for the Battery Level characteristic.
    pub battery_user_description: Option<String>,
    pub charge_state: Option<PinConfig>,
    pub charge_led: Option<PinConfig>,
    pub adc_divider_measured: Option<u32>,
    pub adc_divider_total: Option<u32>,
    pub default_tx_power: Option<i8>,
    pub use_2m_phy: Option<bool>,
    pub passkey_entry: Option<bool>,
    pub passkey_entry_timeout: Option<u32>,
}

/// Default passkey entry timeout in seconds.
pub const DEFAULT_PASSKEY_ENTRY_TIMEOUT_SECS: u32 = 120;

/// Minimum passkey entry timeout in seconds.
pub const MIN_PASSKEY_ENTRY_TIMEOUT_SECS: u32 = 30;

/// nRF52840 DCDC REG0 output voltage
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub enum DcdcReg0Voltage {
    #[serde(rename = "3V3")]
    V3_3,
    #[serde(rename = "1V8")]
    V1_8,
}

/// Config for chip-specific settings
#[derive(Clone, Default, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChipConfig {
    /// DCDC regulator 0 enabled (for nrf52840)
    pub dcdc_reg0: Option<bool>,
    /// DCDC regulator 1 enabled (for nrf52840, nrf52833)
    pub dcdc_reg1: Option<bool>,
    /// DCDC regulator 0 voltage (for nrf52840)
    pub dcdc_reg0_voltage: Option<DcdcReg0Voltage>,
}

/// Config for lights
#[derive(Clone, Default, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightConfig {
    pub capslock: Option<PinConfig>,
    pub scrolllock: Option<PinConfig>,
    pub numslock: Option<PinConfig>,
}

/// Config for a single pin
#[derive(Clone, Default, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinConfig {
    pub pin: String,
    pub low_active: bool,
}

/// Configurations for dependencies
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyConfig {
    /// Enable defmt log or not
    #[serde(default = "default_true")]
    pub defmt_log: bool,
}

impl Default for DependencyConfig {
    fn default() -> Self {
        Self { defmt_log: true }
    }
}

/// Intermediate resolved keymap grid (rows/cols/layers + per-layer actions).
/// Built once by `get_keymap_config` and unpacked into `Keymap`; never (de)serialized.
pub(crate) struct KeymapConfig {
    pub rows: u8,
    pub cols: u8,
    pub layers: u8,
    pub keymap: Vec<Vec<Vec<String>>>,
    pub encoder_map: Vec<Vec<[String; 2]>>, // Empty if there are no encoders or not configured
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyInfo {
    pub hand: char, // 'L' or 'R' or other chars
}

/// Configurations for actions behavior
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BehaviorConfig {
    pub tri_layer: Option<TriLayerConfig>,
    pub one_shot: Option<OneShotConfig>,
    pub one_shot_modifiers: Option<OneShotModifiersConfig>,
    pub combo: Option<CombosConfig>,
    #[serde(alias = "macro")]
    pub macros: Option<MacrosConfig>,
    pub fork: Option<ForksConfig>,
    pub morse: Option<MorsesConfig>,
    pub auto_mouse_layer: Option<Vec<AutoMouseLayerConfig>>,
}

/// Configurations for auto mouse layer
///
/// When motion is detected from a pointing device (e.g. PMW3610), the
/// specified `target_layer` is activated. The layer stays active until
/// `timeout` has elapsed without further motion, then it is deactivated.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AutoMouseLayerConfig {
    /// Pointing device id this entry applies to. When omitted, the entry acts as
    /// a fallback for events whose `device_id` matches no other entry.
    pub device_id: Option<u8>,
    /// Layer index to activate on cursor motion
    pub target_layer: u8,
    /// Idle time after the last cursor motion before the layer is deactivated
    /// (e.g. `"500ms"` or `"2s"`).
    pub timeout: Option<DurationMillis>,
    /// Minimum absolute axis delta required to be considered as motion.
    /// Defaults to `1` (any motion). Helpful to filter out sensor noise.
    pub threshold: Option<u16>,
    /// When `true`, non-mouse key presses deactivate `target_layer` immediately (mouse HID keys and `extra_mouse_keys` excepted).
    /// Macro-emitted keycodes, `Again`/`Repeat`, and `GraveEscape` cannot be classified and never deactivate the layer.
    pub deactivate_on_key: Option<bool>,
    /// Extra keycodes (e.g. modifiers) that do not trigger deactivation when `deactivate_on_key` is set.
    /// Modifier keycodes listed here also exempt modifier-only actions containing them.
    pub extra_mouse_keys: Option<Vec<String>>,
    /// When `true`, key presses that do NOT deactivate `target_layer` extend the timeout deadline
    /// (i.e. reset it to now + `timeout`) at the moment the key's action resolves.
    pub reset_timeout_on_key: Option<bool>,
}

/// Per Key configurations profiles for morse, tap-hold, etc.
/// overrides the defaults given in TapHoldConfig
#[derive(Clone, Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct MorseProfile {
    pub enable_flow_tap: Option<bool>,

    /// if true, tap-hold key will always send tap action when tapped with the same hand only
    pub unilateral_tap: Option<bool>,

    /// The decision mode of the morse/tap-hold key (only one of permissive_hold, hold_on_other_press and normal_mode can be true)
    /// /// if none of them is given, normal mode will be the default
    pub permissive_hold: Option<bool>,
    pub hold_on_other_press: Option<bool>,
    pub normal_mode: Option<bool>,

    /// If the key is pressed longer than this, it is accepted as `hold` (in milliseconds)
    pub hold_timeout: Option<DurationMillis>,

    /// The time elapsed from the last release of a key is longer than this, it will break the morse pattern (in milliseconds)
    pub gap_timeout: Option<DurationMillis>,

    pub quick_tap_timeout: Option<DurationMillis>,
}

/// Configurations for tri layer
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TriLayerConfig {
    pub upper: u8,
    pub lower: u8,
    pub adjust: u8,
}

/// Configurations for oneshot modifiers/layers
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OneShotConfig {
    pub timeout: Option<DurationMillis>,
}

/// Configurations for oneshot modifiers
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OneShotModifiersConfig {
    pub activate_on_keypress: Option<bool>,
    pub quick_release: Option<bool>,
}

/// Configurations for combos
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CombosConfig {
    #[serde(default)]
    pub combos: Vec<ComboConfig>,
    pub timeout: Option<DurationMillis>,
    pub prior_idle_time: Option<DurationMillis>,
}

/// Configurations for combo
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ComboConfig {
    pub actions: Vec<String>,
    pub output: String,
    pub layer: Option<u8>,
}

/// Configurations for macros
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MacrosConfig {
    pub macros: Vec<MacroConfig>,
}

/// Configurations for macro
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MacroConfig {
    pub operations: Vec<MacroOperation>,
}

/// Macro operations (TOML deserialization type — resolved equivalent is in `resolved::behavior`)
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "lowercase")]
pub(crate) enum MacroOperation {
    Tap { keycode: String },
    Down { keycode: String },
    Up { keycode: String },
    Delay { duration: DurationMillis },
    Text { text: String },
}

/// Configurations for forks
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ForksConfig {
    pub forks: Vec<ForkConfig>,
}

/// Configurations for fork
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ForkConfig {
    pub trigger: String,
    pub negative_output: String,
    pub positive_output: String,
    pub match_any: Option<String>,
    pub match_none: Option<String>,
    pub kept_modifiers: Option<String>,
    pub bindable: Option<bool>,
}

/// Configurations for morse keys
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MorsesConfig {
    pub enable_flow_tap: Option<bool>, //default: false
    /// used in permissive_hold mode
    pub prior_idle_time: Option<DurationMillis>,

    /// if true, tap-hold key will always send tap action when tapped with the same hand only
    pub unilateral_tap: Option<bool>,

    /// The decision mode of the morse/tap-hold key (only one of permissive_hold, hold_on_other_press and normal_mode can be true)
    /// if none of them is given, normal mode will be the default
    pub permissive_hold: Option<bool>,
    pub hold_on_other_press: Option<bool>,
    pub normal_mode: Option<bool>,

    /// If the key is pressed longer than this, it is accepted as `hold` (in milliseconds)
    pub hold_timeout: Option<DurationMillis>,

    /// The time elapsed from the last release of a key is longer than this, it will break the morse pattern (in milliseconds)
    pub gap_timeout: Option<DurationMillis>,

    pub quick_tap_timeout: Option<DurationMillis>,

    /// these can be used to overrides the defaults given above
    pub profiles: Option<HashMap<String, MorseProfile>>,

    /// the definition of morse / tap dance keys
    pub morses: Option<Vec<MorseConfig>>,
}

/// Configurations for morse
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MorseConfig {
    // name of morse profile (to address BehaviorConfig::morse.profiles[self.profile])
    pub profile: Option<String>,

    pub tap: Option<String>,
    pub hold: Option<String>,
    pub hold_after_tap: Option<String>,
    pub double_tap: Option<String>,
    /// Array of tap actions for each tap count (0-indexed)
    pub tap_actions: Option<Vec<String>>,
    /// Array of hold actions for each tap count (0-indexed)
    pub hold_actions: Option<Vec<String>>,
    /// Array of morse patter->action pairs  count (0-indexed)
    pub morse_actions: Option<Vec<MorseActionPair>>,
}

/// Configurations for morse action pairs
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MorseActionPair {
    pub pattern: String, // for example morse code of "B": "-..." or "_..." or "1000"
    pub action: String,  // "B"
}

/// Split connection transport
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SplitConnection {
    #[default]
    Ble,
    Serial,
}

/// Configurations for split keyboards
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitConfig {
    pub connection: SplitConnection,
    pub central: SplitBoardConfig,
    pub peripheral: Vec<SplitBoardConfig>,
}

/// Configurations for each split board
///
/// The transport field must match `split.connection`: `serial` is required for
/// serial splits and forbidden for BLE splits; `ble_addr` is optional for BLE
/// splits (dongle setups omit it) and forbidden for serial splits.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitBoardConfig {
    /// Row number of the split board
    pub rows: usize,
    /// Col number of the split board
    pub cols: usize,
    /// Row offset of the split board
    pub row_offset: usize,
    /// Col offset of the split board
    pub col_offset: usize,
    /// Ble address
    pub ble_addr: Option<[u8; 6]>,
    /// Serial config, the vector length should be 1 for peripheral
    pub serial: Option<Vec<SerialConfig>>,
    /// Matrix config for the split
    pub matrix: MatrixConfig,
    /// Input device config for the split
    pub input_device: Option<InputDeviceConfig>,
    /// Display config for the split board
    pub display: Option<DisplayConfig>,
    /// Battery ADC pin for this split board
    pub battery_adc_pin: Option<String>,
    /// User-facing description for this board's Battery Level characteristic
    pub battery_user_description: Option<String>,
    /// ADC divider measured value for battery
    pub adc_divider_measured: Option<u32>,
    /// ADC divider total value for battery
    pub adc_divider_total: Option<u32>,
    /// Output Pin config for the split
    pub output: Option<Vec<OutputConfig>>,
    /// Path to the peripheral firmware binary for automatic dfu_split update.
    /// Relative to the project's `Cargo.toml`.  When set, the generated code
    /// includes the binary with `include_bytes!` and registers it via
    /// [`set_firmware_update_data`](crate::set_firmware_update_data).
    pub firmware: Option<String>,
    /// DFU update policy for this peripheral. "MatchHash" (default) only
    /// flashes when the firmware differs; "force" always flashes.
    pub update_policy: Option<String>,
}

/// Serial port config
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SerialConfig {
    pub instance: String,
    pub tx_pin: String,
    pub rx_pin: String,
}

/// Duration in milliseconds
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct DurationMillis(#[serde(deserialize_with = "parse_duration_millis")] pub u64);

const fn default_true() -> bool {
    true
}

const fn default_false() -> bool {
    false
}

const fn default_pointing_report_hz() -> u16 {
    125
}

fn parse_duration_millis<'de, D: de::Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let input: String = de::Deserialize::deserialize(deserializer)?;
    let num = input.trim_end_matches(|c: char| !c.is_numeric());
    let unit = &input[num.len()..];
    let num: u64 = num.parse().map_err(|_| {
        de::Error::custom(format!(
            "Invalid number \"{num}\" in duration: number part must be a u64"
        ))
    })?;

    match unit {
        "s" => Ok(num * 1000),
        "ms" => Ok(num),
        other => Err(de::Error::custom(format!(
            "Invalid duration unit \"{other}\": unit part must be either \"s\" or \"ms\""
        ))),
    }
}

/// Configuration for host tools
#[serde_inline_default]
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostConfig {
    /// Whether Vial is enabled
    #[serde_inline_default(true)]
    pub vial_enabled: bool,
    /// Whether the RMK-native Rynk protocol is enabled. Mutually exclusive
    /// with `vial_enabled` (the underlying Cargo features conflict).
    #[serde_inline_default(false)]
    pub rynk_enabled: bool,
    /// Physical keys (row, col) held simultaneously to unlock (optional).
    /// Shared by the Vial lock and the Rynk lock gate.
    pub unlock_keys: Option<Vec<[u8; 2]>>,
    /// Start (and stay) unlocked, bypassing the unlock-key combo (default:
    /// false). Renamed from `vial_insecure`; the old name still parses.
    #[serde(alias = "vial_insecure")]
    #[serde_inline_default(false)]
    pub insecure: bool,
    /// Move the Rynk config-write tier (`SetKeyAction`, `SetMacro`, …) into the
    /// locked set, so writes also require unlock (default: false).
    #[serde_inline_default(false)]
    pub write_requires_unlock: bool,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            vial_enabled: true,
            rynk_enabled: false,
            unlock_keys: None,
            insecure: false,
            write_requires_unlock: false,
        }
    }
}

/// Configurations for input devices
///
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputDeviceConfig {
    pub encoder: Option<Vec<EncoderConfig>>,
    pub pointing: Option<Vec<PointingDeviceConfig>>,
    pub joystick: Option<Vec<JoystickConfig>>,
    pub pmw3610: Option<Vec<Pmw3610Config>>,
    pub pmw33xx: Option<Vec<Pmw33xxConfig>>,
    pub iqs5xx: Option<Vec<Iqs5xxConfig>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoystickConfig {
    // Name of the joystick
    pub name: String,
    /// Device id used to match this joystick with its JoystickProcessor.
    /// If omitted, ids are assigned sequentially starting from 0.
    pub id: Option<u8>,
    // Pin a of the joystick
    pub pin_x: String,
    // Pin b of the joystick
    pub pin_y: String,
    // Pin z of the joystick
    pub pin_z: String,
    pub transform: Vec<Vec<i16>>,
    pub bias: Vec<i16>,
    pub resolution: u16,
}

/// PMW3610 optical mouse sensor configuration
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pmw3610Config {
    /// Name of the sensor (used for variable naming)
    pub name: String,
    /// id of the device
    pub id: Option<u8>,
    /// SPI pins
    pub spi: SpiConfig,
    /// Optional motion interrupt pin
    pub motion: Option<String>,
    /// CPI resolution (200-3200, step 200). Optional, uses sensor default if not set.
    pub cpi: Option<u16>,
    /// Invert X axis
    #[serde(default)]
    pub invert_x: bool,
    /// Invert Y axis
    #[serde(default)]
    pub invert_y: bool,
    /// Swap X and Y axes
    #[serde(default)]
    pub swap_xy: bool,
    /// Force awake mode (disable power saving)
    #[serde(default)]
    pub force_awake: bool,
    /// Enable smart mode for better tracking on shiny surfaces
    #[serde(default)]
    pub smart_mode: bool,
    /// Report rate (Hz). Motion will be accumulated and emitted at this rate.
    #[serde(default = "default_pointing_report_hz")]
    pub report_hz: u16,
    #[serde(default)]
    pub proc_invert_x: bool,
    /// Invert Y axis
    #[serde(default)]
    pub proc_invert_y: bool,
    /// Swap X and Y axes
    #[serde(default)]
    pub proc_swap_xy: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Pmw33xxType {
    #[default]
    PMW3360,
    PMW3389,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pmw33xxConfig {
    // Name of the sensor (used for variable naming)
    pub name: String,
    // id of the device
    pub id: Option<u8>,
    // Sensor Type (3360 or 3389)
    pub sensor_type: Pmw33xxType,
    // SPI pins
    pub spi: SpiConfig,
    // Optional motion interrupt pin
    pub motion: Option<String>,
    // CPI resolution (100-12000, step 100).Optional, uses sensor default 1600 if not set.
    pub cpi: Option<u16>,
    // Rotational transform angle (-127 to 127) Optional, uses sensor default 0 if not set.
    pub rot_trans_angle: Option<i8>,
    // liftoff distance. Optional, uses sensor default 0 if not set.
    pub liftoff_dist: Option<u8>,
    // Invert X axis
    #[serde(default)]
    pub proc_invert_x: bool,
    // Invert Y axis
    #[serde(default)]
    pub proc_invert_y: bool,
    // Swap X and Y axes
    #[serde(default)]
    pub proc_swap_xy: bool,
    /// Report rate (Hz). Motion will be accumulated and emitted at this rate.
    #[serde(default = "default_pointing_report_hz")]
    pub report_hz: u16,
}

/// Azoteq IQS5xx trackpad configuration.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Iqs5xxConfig {
    /// Name of the trackpad (used for variable naming).
    pub name: String,
    /// RMK pointing-device id (0-255). Defaults to 0.
    pub id: Option<u8>,
    /// I²C bus the trackpad is connected to. The bus is dedicated to this
    /// device — sharing with other I²C peripherals (e.g. an OLED) is not yet
    /// supported via TOML.
    pub i2c: Iqs5xxI2cConfig,
    /// Optional `RDY` pin. Strongly recommended; without it the driver falls
    /// back to timed polling and may stall the bus through clock-stretching.
    pub rdy: Option<String>,
    /// Invert X in the PointingProcessor.
    #[serde(default)]
    pub proc_invert_x: bool,
    /// Invert Y in the PointingProcessor.
    #[serde(default)]
    pub proc_invert_y: bool,
    /// Swap X and Y in the PointingProcessor.
    #[serde(default)]
    pub proc_swap_xy: bool,
}

/// I²C bus configuration for the IQS5xx. Distinct from the generic `I2cConfig`
/// because the IQS5xx address is fixed (`0x74` by default; can be reprogrammed
/// at the IC, but not at runtime — exposing it would be misleading).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Iqs5xxI2cConfig {
    pub instance: String,
    pub sda: String,
    pub scl: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncoderConfig {
    // Pin a of the encoder
    pub pin_a: String,
    // Pin b of the encoder
    pub pin_b: String,
    // Phase is the working mode of the rotary encoders.
    // Available mode:
    // - default: resolution = 1
    // - e8h7: phase table tuned for E8H7 encoders
    // - resolution: customized resolution, the resolution value and reverse should be specified
    //   A typical [EC11 encoder](https://tech.alpsalpine.com/cms.media/product_catalog_ec_01_ec11e_en_611f078659.pdf)'s resolution is 2
    //   In resolution mode, you can also specify the number of detent and pulses, the resolution will be calculated by `pulse * 4 / detent`
    #[serde(default)]
    pub phase: EncoderPhase,
    // Resolution
    pub resolution: Option<EncoderResolution>,
    // The number of detent
    pub detent: Option<u8>,
    // The number of pulse
    pub pulse: Option<u8>,
    // Whether the direction of the rotary encoder is reversed.
    pub reverse: Option<bool>,
    // Use MCU's internal pull-up resistor or not, defaults to false, the external pull-up resistor is needed
    #[serde(default = "default_false")]
    pub internal_pullup: bool,
    // Debounce interval in milliseconds. Suppresses spurious events from mechanical contact bounce.
    // Defaults to 0 (disabled) if not specified.
    pub debounce_ms: Option<u16>,
}

/// Rotary encoder phase (decoding) mode
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EncoderPhase {
    #[default]
    Default,
    E8h7,
    Resolution,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, untagged)]
pub enum EncoderResolution {
    Value(u8),
    Derived { detent: u8, pulse: u8 },
}

impl Default for EncoderResolution {
    fn default() -> Self {
        Self::Value(4)
    }
}

/// Pointing device config
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointingDeviceConfig {
    pub interface: Option<CommunicationProtocol>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunicationProtocol {
    I2c(I2cConfig),
    Spi(SpiConfig),
}

/// SPI config
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpiConfig {
    pub instance: String,
    pub sck: String,
    pub mosi: String,
    pub miso: String,
    pub cs: Option<String>,
    pub cpi: Option<u32>,
    pub tx_dma: Option<String>,
    pub rx_dma: Option<String>,
}

/// I2C config
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct I2cConfig {
    pub instance: String,
    pub sda: String,
    pub scl: String,
    /// 7-bit I2C address. Defaults to 0x3C when omitted.
    #[serde(default = "default_i2c_address")]
    pub address: u8,
}

const fn default_i2c_address() -> u8 {
    0x3C
}

/// Display driver type
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayDriver {
    Ssd1306,
    Sh1106,
    Sh1107,
    Sh1108,
    Ssd1309,
}

/// Display configuration
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayConfig {
    pub driver: DisplayDriver,
    pub protocol: CommunicationProtocol,
    pub size: String,
    #[serde(default)]
    pub rotation: u16,
    pub renderer: Option<String>,
    /// Poll interval in milliseconds for periodic redraws (animations).
    /// When absent, polling is disabled — the display only redraws on events.
    pub render_interval: Option<u64>,
    /// Minimum time in milliseconds between event-driven renders.
    /// Prevents the display from being hammered by rapid events. Default: 10 ms.
    pub min_render_interval: Option<u64>,
}

/// Configuration for an output pin
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    pub pin: String,
    #[serde(default)]
    pub low_active: bool,
    #[serde(default)]
    pub initial_state_active: bool,
}

impl KeyboardTomlConfig {
    pub(crate) fn get_output_config(&self) -> Result<Vec<OutputConfig>, String> {
        let output_config = self.output.clone();
        let split = self.split.clone();
        match (output_config, split) {
            (None, Some(s)) => Ok(s.central.output.unwrap_or_default()),
            (Some(c), None) => Ok(c),
            (None, None) => Ok(Default::default()),
            _ => Err("Use [[split.output]] to define outputs for split in your keyboard.toml!".to_string()),
        }
    }

    pub(crate) fn get_dependency_config(&self) -> DependencyConfig {
        self.dependency.clone().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_config_default_values() {
        let config = EventConfig::default();

        // Check some key default values from event_default.toml
        assert_eq!(config.keyboard.channel_size, 16);
        assert_eq!(config.keyboard.pubs, 2);
        assert_eq!(config.keyboard.subs, 3);

        assert_eq!(config.modifier.channel_size, 8);
        assert_eq!(config.modifier.pubs, 1);
        assert_eq!(config.modifier.subs, 2);

        assert_eq!(config.layer_change.channel_size, 1);
        assert_eq!(config.layer_change.subs, 1);

        assert_eq!(config.led_indicator.channel_size, 2);
        assert_eq!(config.led_indicator.pubs, 2);
        assert_eq!(config.led_indicator.subs, 3);

        assert_eq!(config.pointing.channel_size, 8);
        assert_eq!(config.pointing.subs, 2);

        assert_eq!(config.action.channel_size, 16);
        assert_eq!(config.action.pubs, 1);
        assert_eq!(config.action.subs, 0);
    }

    #[test]
    fn test_event_config_user_override() {
        // Simulate user config that overrides some event settings
        let user_toml = r#"
[event.keyboard]
channel_size = 32
"#;
        // Parse with event defaults first, then user config
        let config: KeyboardTomlConfig = Config::builder()
            .add_source(File::from_str(EVENT_DEFAULT_CONFIG, FileFormat::Toml))
            .add_source(File::from_str(user_toml, FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        // User-overridden values
        assert_eq!(config.event.keyboard.channel_size, 32);
        assert_eq!(config.event.keyboard.pubs, 2);
        assert_eq!(config.event.keyboard.subs, 3);

        // Non-overridden values should use defaults
        assert_eq!(config.event.modifier.channel_size, 8);
        assert_eq!(config.event.modifier.subs, 2);
        assert_eq!(config.event.layer_change.subs, 1);
    }

    #[test]
    fn rmk_count_limits_fit_u8_capability_fields() {
        let ok: KeyboardTomlConfig = toml::from_str(
            r#"
[rmk]
combo_max_num = 255
morse_max_num = 255
fork_max_num = 255
"#,
        )
        .unwrap();
        assert_eq!(ok.rmk.combo_max_num, 255);
        assert_eq!(ok.rmk.morse_max_num, 255);
        assert_eq!(ok.rmk.fork_max_num, 255);

        for (field, message) in [
            ("combo_max_num", "combo_max_num must be between 0 and 255"),
            ("morse_max_num", "morse_max_num must be between 0 and 255"),
            ("fork_max_num", "fork_max_num must be between 0 and 255"),
        ] {
            let toml = format!("[rmk]\n{field} = 256\n");
            let err = toml::from_str::<KeyboardTomlConfig>(&toml).unwrap_err();
            assert!(err.to_string().contains(message), "{err}");
        }
    }

    #[test]
    fn test_event_config_partial_override_with_event_defaults_loader() {
        let user_toml = r#"
[event.layer_change]
subs = 2
"#;
        let path = std::env::temp_dir().join(format!(
            "rmk-event-defaults-loader-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, user_toml).unwrap();

        let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
        std::fs::remove_file(path).unwrap();

        assert_eq!(config.event.layer_change.channel_size, 1);
        assert_eq!(config.event.layer_change.pubs, 2);
        assert_eq!(config.event.layer_change.subs, 2);
    }
}
