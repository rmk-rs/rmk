mod behavior;
#[cfg(feature = "_ble")]
mod ble_battery;
mod device;
#[cfg(feature = "rynk")]
mod lock;
mod positional;
mod storage;
mod vial;

pub use behavior::{
    AutoMouseLayerConfig, BehaviorConfig, CombosConfig, ForksConfig, KeyboardMacrosConfig, MorsesConfig,
    MouseKeyConfig, OneShotConfig, OneShotModifiersConfig, StickyKeyConfig, StickyKeyHoldDuration, StickyKeyProfile,
    StickyKeyReleaseMode, TapConfig,
};
#[cfg(feature = "_ble")]
pub use ble_battery::BleBatteryConfig;
pub use device::{DeviceConfig, RMK_BUILD_INFO};
#[cfg(feature = "rynk")]
pub use lock::LockConfig;
pub use positional::{Hand, PositionalConfig};
pub use storage::StorageConfig;
pub use vial::VialConfig;

/// Internal configurations for RMK keyboard.
#[cfg_attr(not(feature = "rynk"), derive(Default))]
pub struct RmkConfig<'a> {
    pub device_config: DeviceConfig<'a>,
    #[cfg(feature = "vial")]
    pub vial_config: VialConfig<'a>,
    #[cfg(feature = "rynk")]
    pub lock_config: LockConfig,
    /// Opaque, compressed physical-layout blob served over rynk's `GetLayout`.
    /// Baked at build time from `[layout].map`; empty when there's no layout.
    #[cfg(feature = "rynk")]
    pub layout_blob: &'a [u8],
    #[cfg(feature = "storage")]
    pub storage_config: StorageConfig,
    #[cfg(feature = "_ble")]
    pub ble_battery_config: BleBatteryConfig<'a>,
}

#[cfg(feature = "rynk")]
impl Default for RmkConfig<'_> {
    fn default() -> Self {
        Self {
            device_config: DeviceConfig::default(),
            #[cfg(feature = "vial")]
            vial_config: VialConfig::default(),
            lock_config: LockConfig::default(),
            layout_blob: rmk_types::constants::LAYOUT_BLOB,
            #[cfg(feature = "storage")]
            storage_config: StorageConfig::default(),
            #[cfg(feature = "_ble")]
            ble_battery_config: BleBatteryConfig::default(),
        }
    }
}
