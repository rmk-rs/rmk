//! System handlers — handshake, device identity, reboot, bootloader jump, storage reset.

use rmk_types::constants;
use rmk_types::protocol::rynk::command::{
    BootloaderJump, GetBuildInfo, GetCapabilities, GetDeviceInfo, GetLockStatus, GetVersion, Lock, Reboot,
    StorageReset, UnlockPoll,
};
use rmk_types::protocol::rynk::{
    BuildInfo, DEVICE_INFO_STRING_SIZE, DeviceCapabilities, DeviceInfo, LockStatus, MAX_BULK_ITEMS, MAX_BULK_KEYS,
    ProtocolVersion, RYNK_MAX_PAYLOAD_SIZE, RynkError, StorageResetMode,
};

use super::super::{RMK_VERSION, RynkService, truncated};
use super::Handle;
use crate::host::lock::HostLock;

impl Handle<GetVersion> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<ProtocolVersion, RynkError> {
        Ok(ProtocolVersion::CURRENT)
    }
}

impl Handle<GetCapabilities> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<DeviceCapabilities, RynkError> {
        let (rows, cols, num_layers) = self.ctx.keymap_dimensions();
        Ok(DeviceCapabilities {
            // Layout (live, from the configured keymap)
            num_layers: num_layers as u8,
            num_rows: rows as u8,
            num_cols: cols as u8,

            // Input device limits (compile-time from keyboard.toml)
            num_encoders: self.ctx.num_encoders() as u8,
            max_combos: constants::COMBO_MAX_NUM as u8,
            max_combo_keys: constants::COMBO_MAX_LENGTH as u8,
            macro_space_size: constants::MACRO_SPACE_SIZE as u16,
            max_morse: constants::MORSE_MAX_NUM as u8,
            max_patterns_per_key: constants::MAX_PATTERNS_PER_KEY as u8,
            max_forks: constants::FORK_MAX_NUM as u8,

            // Feature flags
            storage_enabled: cfg!(feature = "storage"),
            #[cfg(feature = "lighting")]
            lighting_enabled: self.lighting.is_some(),
            #[cfg(not(feature = "lighting"))]
            lighting_enabled: false,

            // Connectivity
            is_split: cfg!(feature = "split"),
            num_split_peripherals: constants::SPLIT_PERIPHERALS_NUM as u8,
            ble_enabled: cfg!(feature = "_ble"),
            num_ble_profiles: constants::NUM_BLE_PROFILE as u8,

            // Protocol limits
            max_payload_size: RYNK_MAX_PAYLOAD_SIZE as u16,
            macro_chunk_size: constants::MACRO_DATA_SIZE as u16,
            max_bulk_keys: MAX_BULK_KEYS as u8,
            max_bulk_items: MAX_BULK_ITEMS as u8,
            bulk_transfer_supported: true,
        })
    }
}

impl Handle<Reboot> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<(), RynkError> {
        // Fire-and-forget: synchronous reset never returns on real hardware.
        crate::boot::reboot_keyboard();
        Ok(())
    }
}

impl Handle<BootloaderJump> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<(), RynkError> {
        // Fire-and-forget, same reasoning as `Reboot`.
        crate::boot::jump_to_bootloader();
        Ok(())
    }
}

impl Handle<StorageReset> for RynkService<'_> {
    async fn handle(&self, mode: StorageResetMode) -> Result<(), RynkError> {
        if mode != StorageResetMode::Full {
            // TODO: Reset required storage range
            return Err(RynkError::Unimplemented);
        }
        self.ctx.reset_storage().await;
        Ok(())
    }
}

// Lock endpoints are served by the session's own gate, and stay dispatchable
// while locked.

impl Handle<GetLockStatus> for HostLock<'_> {
    async fn handle(&self, _: ()) -> Result<LockStatus, RynkError> {
        Ok(self.status())
    }
}

impl Handle<UnlockPoll> for HostLock<'_> {
    async fn handle(&self, _: ()) -> Result<LockStatus, RynkError> {
        Ok(self.poll())
    }
}

impl Handle<Lock> for HostLock<'_> {
    async fn handle(&self, _: ()) -> Result<(), RynkError> {
        self.lock();
        Ok(())
    }
}

impl Handle<GetDeviceInfo> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<DeviceInfo, RynkError> {
        Ok(DeviceInfo {
            rmk_version: RMK_VERSION,
            vendor_id: self.device.vid,
            product_id: self.device.pid,
            manufacturer: truncated(self.device.manufacturer),
            product_name: truncated(self.device.product_name),
            serial_number: truncated(self.device.serial_number),
        })
    }
}

impl Handle<GetBuildInfo> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<BuildInfo, RynkError> {
        Ok(self.build_info.clone())
    }
}
