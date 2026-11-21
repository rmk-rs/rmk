//! Connection handlers — USB/BLE preferred-transport read, BLE profile management.

use rmk_types::connection::{ConnectionStatus, ConnectionType};
use rmk_types::protocol::rynk::RynkError;
#[cfg(feature = "_ble")]
use rmk_types::protocol::rynk::command::{ClearBleProfile, DisconnectBleProfile, SwitchBleProfile};
use rmk_types::protocol::rynk::command::{GetConnectionStatus, GetConnectionType};

use super::Handle;
use crate::host::context::KeyboardContext;

impl Handle<GetConnectionType> for KeyboardContext<'_> {
    async fn handle(&self, _: ()) -> Result<ConnectionType, RynkError> {
        Ok(self.preferred_connection())
    }
}

/// `Cmd::GetConnectionStatus` — the same payload the `ConnectionChange`
/// topic pushes, so a host can recover a missed push.
impl Handle<GetConnectionStatus> for KeyboardContext<'_> {
    async fn handle(&self, _: ()) -> Result<ConnectionStatus, RynkError> {
        Ok(self.connection_status())
    }
}

/// `Cmd::SwitchBleProfile` — payload is the profile index. `try_send` is
/// used so a host hammering this Cmd while the previous switch is still
/// running observes the queue-full error rather than blocking the
/// dispatch loop.
#[cfg(feature = "_ble")]
impl Handle<SwitchBleProfile> for KeyboardContext<'_> {
    async fn handle(&self, profile: u8) -> Result<(), RynkError> {
        Self::check_ble_profile(profile)?;
        crate::channel::BLE_PROFILE_CHANNEL
            .try_send(crate::ble::profile::BleProfileAction::Switch(profile))
            .map_err(|_| RynkError::NotReady)
    }
}

/// `Cmd::ClearBleProfile` — disconnects the given profile and wipes its bond
/// without requiring a prior switch.
#[cfg(feature = "_ble")]
impl Handle<ClearBleProfile> for KeyboardContext<'_> {
    async fn handle(&self, profile: u8) -> Result<(), RynkError> {
        Self::check_ble_profile(profile)?;
        crate::channel::BLE_PROFILE_CHANNEL
            .try_send(crate::ble::profile::BleProfileAction::ClearBond(Some(profile)))
            .map_err(|_| RynkError::NotReady)
    }
}

/// `Cmd::DisconnectBleProfile` — disconnects the given profile and keeps its bond;
/// the selected profile then waits for a new host.
#[cfg(feature = "_ble")]
impl Handle<DisconnectBleProfile> for KeyboardContext<'_> {
    async fn handle(&self, profile: u8) -> Result<(), RynkError> {
        Self::check_ble_profile(profile)?;
        crate::channel::BLE_PROFILE_CHANNEL
            .try_send(crate::ble::profile::BleProfileAction::Disconnect(profile))
            .map_err(|_| RynkError::NotReady)
    }
}

#[cfg(feature = "_ble")]
impl KeyboardContext<'_> {
    /// `Invalid` for a profile past the host and dongle profiles.
    fn check_ble_profile(profile: u8) -> Result<(), RynkError> {
        if (profile as usize) >= crate::TOTAL_BLE_PROFILES {
            Err(RynkError::Invalid)
        } else {
            Ok(())
        }
    }
}
