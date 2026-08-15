//! System-level protocol types.
//!
//! Types for protocol handshake, device discovery, security, and global configuration.

use heapless::{String, Vec};
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use crate::morse::MorseProfile;

/// Maximum byte length of each `DeviceInfo` string field.
pub const DEVICE_INFO_STRING_SIZE: usize = 32;

/// Protocol version advertised during the connection handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct ProtocolVersion {
    pub major: u8,
    pub minor: u8,
}

impl ProtocolVersion {
    /// Current protocol version for this firmware release.
    /// Now the protocol is still being developed, so the version is v0.1
    pub const CURRENT: Self = Self { major: 0, minor: 1 };
}

/// Device capabilities discovered during the connection handshake.
///
/// The host reads this once after connecting to learn the firmware's layout,
/// feature set, and protocol limits.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct DeviceCapabilities {
    // -- Layout --
    pub num_layers: u8,
    pub num_rows: u8,
    pub num_cols: u8,

    // -- Input devices --
    pub num_encoders: u8,
    pub max_combos: u8,
    pub max_combo_keys: u8,
    /// Byte size of the flat macro region. `0` disables macro data endpoints.
    pub macro_space_size: u16,
    pub max_morse: u8,
    pub max_patterns_per_key: u8,
    pub max_forks: u8,

    // -- Feature flags --
    pub storage_enabled: bool,
    pub lighting_enabled: bool,

    // -- Connectivity --
    pub is_split: bool,
    pub num_split_peripherals: u8,
    pub ble_enabled: bool,
    pub num_ble_profiles: u8,

    // -- Protocol limits --
    pub max_payload_size: u16,
    /// Worst-case keys per `GetKeymapBulk` page; paces pipelined reads.
    pub max_bulk_keys: u8,
    /// Worst-case combos/morses per `GetComboBulk`/`GetMorseBulk` page; paces
    /// pipelined reads. Separate from `max_bulk_keys` because these items are
    /// far larger than keys. Writes pack by encoded size up to `max_payload_size`.
    pub max_bulk_items: u8,
    pub macro_chunk_size: u16,
    pub bulk_transfer_supported: bool,
}

/// Version of the `rmk` crate baked into the firmware, so hosts can key
/// version-specific behavior off the library release, not the user's app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct FirmwareVersion {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
}

/// Device identity for display and per-device host profiles.
///
/// Complements [`DeviceCapabilities`]: capabilities answer "what can you do"
/// for feature gating, identity answers "which device is this".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct DeviceInfo {
    pub rmk_version: FirmwareVersion,
    pub vendor_id: u16,
    pub product_id: u16,
    #[cfg_attr(feature = "wasm", tsify(type = "string"))]
    pub manufacturer: String<DEVICE_INFO_STRING_SIZE>,
    #[cfg_attr(feature = "wasm", tsify(type = "string"))]
    pub product_name: String<DEVICE_INFO_STRING_SIZE>,
    #[cfg_attr(feature = "wasm", tsify(type = "string"))]
    pub serial_number: String<DEVICE_INFO_STRING_SIZE>,
}

impl MaxSize for DeviceInfo {
    // A str encodes as varint length + UTF-8 bytes — the same wire shape as
    // `Vec<u8, N>`, so the Vec bound covers each string field.
    const POSTCARD_MAX_SIZE: usize = FirmwareVersion::POSTCARD_MAX_SIZE
        + 2 * u16::POSTCARD_MAX_SIZE
        + 3 * crate::heapless_vec_max_size::<u8, DEVICE_INFO_STRING_SIZE>();
}

/// Current lock/unlock state of this Rynk session, returned by `GetLockStatus`
/// and `UnlockPoll`. The `Lock` endpoint returns `()`.
///
/// Loses `Copy` and derived `MaxSize` (both forbidden by the `heapless::Vec`
/// field): handlers return it by value, and the bound is hand-written below.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct LockStatus {
    pub locked: bool,
    /// An unlock attempt is armed (host is polling; window not yet lapsed).
    pub unlocking: bool,
    /// Challenge keys not currently held; `== key_positions.len()` when no
    /// attempt is armed.
    pub remaining_keys: u8,
    /// The challenge itself: physical `(row, col)` the user must hold. Empty
    /// while `locked` ⇒ permanently locked (no `unlock_keys` configured).
    #[cfg_attr(feature = "wasm", tsify(type = "[number, number][]"))]
    pub key_positions: Vec<(u8, u8), 4>,
}

// `#[derive(MaxSize)]` doesn't support `heapless::Vec` (see `crate` root), so
// hand-write the bound: two bools + one u8 + the key-position vec.
impl MaxSize for LockStatus {
    const POSTCARD_MAX_SIZE: usize =
        2 * bool::POSTCARD_MAX_SIZE + u8::POSTCARD_MAX_SIZE + crate::heapless_vec_max_size::<(u8, u8), 4>();
}

/// Storage reset mode for the `StorageReset` endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum StorageResetMode {
    /// Reset all stored data — including keymap and BLE bonds.
    Full,
    /// Reset only the layout/keymap data, preserving BLE bonds.
    LayoutOnly,
}

/// Protocol-facing behavior configuration (global timing settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct BehaviorConfig {
    pub combo_timeout_ms: u16,
    pub oneshot_timeout_ms: u16,
    pub tap_interval_ms: u16,
    pub tap_capslock_interval_ms: u16,
    /// Default profile for morse/tap-hold keys; per-key profiles override it.
    /// A `None` field falls back to a firmware default the host can't read back —
    /// for `enable_flow_tap`, the compiled `[behavior.morse]` switch.
    pub morse_default_profile: MorseProfile,
    /// Flow-tap window: a morse key pressed within this time of the previous
    /// key press is forced to its tap action.
    pub morse_prior_idle_time_ms: u16,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::rynk::tests::{assert_max_size_bound, round_trip};

    #[test]
    fn round_trip_protocol_version() {
        round_trip(&ProtocolVersion { major: 1, minor: 0 });
        round_trip(&ProtocolVersion { major: 255, minor: 255 });
    }

    #[test]
    fn round_trip_device_capabilities() {
        // Populated and all-zero edge cases.
        round_trip(&DeviceCapabilities {
            num_layers: 4,
            num_rows: 6,
            num_cols: 14,
            num_encoders: 2,
            max_combos: 16,
            max_combo_keys: 4,
            macro_space_size: 2048,
            max_morse: 8,
            max_patterns_per_key: 8,
            max_forks: 4,
            storage_enabled: true,
            lighting_enabled: false,
            is_split: false,
            num_split_peripherals: 0,
            ble_enabled: true,
            num_ble_profiles: 4,
            max_payload_size: 256,
            max_bulk_keys: 32,
            max_bulk_items: 8,
            macro_chunk_size: 64,
            bulk_transfer_supported: true,
        });
        round_trip(&DeviceCapabilities {
            num_layers: 0,
            num_rows: 0,
            num_cols: 0,
            num_encoders: 0,
            max_combos: 0,
            max_combo_keys: 0,
            macro_space_size: 0,
            max_morse: 0,
            max_patterns_per_key: 0,
            max_forks: 0,
            storage_enabled: false,
            lighting_enabled: false,
            is_split: false,
            num_split_peripherals: 0,
            ble_enabled: false,
            num_ble_profiles: 0,
            max_payload_size: 0,
            max_bulk_keys: 0,
            max_bulk_items: 0,
            macro_chunk_size: 0,
            bulk_transfer_supported: false,
        });
    }

    #[test]
    fn round_trip_device_info() {
        // Fill strings and ids so varints take their full width.
        let full: String<DEVICE_INFO_STRING_SIZE> = String::try_from("🦀🦀🦀🦀🦀🦀🦀🦀").unwrap();
        assert_eq!(full.len(), DEVICE_INFO_STRING_SIZE);
        let info = DeviceInfo {
            rmk_version: FirmwareVersion {
                major: 255,
                minor: 255,
                patch: 255,
            },
            vendor_id: u16::MAX,
            product_id: u16::MAX,
            manufacturer: full.clone(),
            product_name: full.clone(),
            serial_number: full,
        };
        round_trip(&info);
        assert_max_size_bound(&info);
    }

    #[test]
    fn round_trip_lock_status() {
        // Locked, no attempt armed, challenge advertised.
        let mut kp = Vec::new();
        kp.push((1, 2)).unwrap();
        kp.push((3, 4)).unwrap();
        round_trip(&LockStatus {
            locked: true,
            unlocking: false,
            remaining_keys: 2,
            key_positions: kp,
        });
        // Unlocked / no challenge configured.
        round_trip(&LockStatus {
            locked: false,
            unlocking: true,
            remaining_keys: 0,
            key_positions: Vec::new(),
        });

        // Max-capacity case: every (u8, u8) at u8::MAX so each varint takes its
        // full width and the hand-written `MaxSize` bound is genuinely exercised.
        let mut full = Vec::new();
        while full.push((u8::MAX, u8::MAX)).is_ok() {}
        let status = LockStatus {
            locked: true,
            unlocking: true,
            remaining_keys: u8::MAX,
            key_positions: full,
        };
        round_trip(&status);
        assert_max_size_bound(&status);
    }

    #[test]
    fn round_trip_storage_reset_mode() {
        round_trip(&StorageResetMode::Full);
        round_trip(&StorageResetMode::LayoutOnly);
    }

    #[test]
    fn round_trip_behavior_config() {
        round_trip(&BehaviorConfig {
            combo_timeout_ms: 50,
            oneshot_timeout_ms: 500,
            tap_interval_ms: 200,
            tap_capslock_interval_ms: 20,
            morse_default_profile: MorseProfile::new(
                Some(true),
                Some(crate::morse::MorseMode::PermissiveHold),
                Some(260),
                Some(180),
            )
            .with_quick_tap_timeout_ms(Some(120)),
            morse_prior_idle_time_ms: 130,
        });

        // Max-width case. The profile's setters top out at bit 45, so it is built
        // from raw bits: a host decodes `MorseProfile` from a bare u64, and only
        // the reserved high bits reach the 10-byte varint `MaxSize` counts.
        let cfg = BehaviorConfig {
            combo_timeout_ms: u16::MAX,
            oneshot_timeout_ms: u16::MAX,
            tap_interval_ms: u16::MAX,
            tap_capslock_interval_ms: u16::MAX,
            morse_default_profile: MorseProfile::from(u64::MAX),
            morse_prior_idle_time_ms: u16::MAX,
        };
        round_trip(&cfg);
        assert_max_size_bound(&cfg);
    }
}
