//! Connection handlers — USB/BLE preferred-transport read, BLE profile management.

#[cfg(feature = "_ble")]
use core::cell::Cell;

#[cfg(feature = "_ble")]
use rmk_types::ble::BleStatus;
use rmk_types::connection::{ConnectionStatus, ConnectionType};
use rmk_types::protocol::rynk::RynkError;
#[cfg(feature = "_ble")]
use rmk_types::protocol::rynk::command::{ClearBleProfile, GetBleStatus, SwitchBleProfile};
use rmk_types::protocol::rynk::command::{GetConnectionStatus, GetConnectionType};

use super::super::RynkService;
use super::Handle;
#[cfg(feature = "_ble")]
use crate::ble::profile::BleProfileAction;

impl Handle<GetConnectionType> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<ConnectionType, RynkError> {
        Ok(self.ctx.preferred_connection())
    }
}

/// `Cmd::GetConnectionStatus` — the same payload the `ConnectionChange`
/// topic pushes, so a host can recover a missed push.
impl Handle<GetConnectionStatus> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<ConnectionStatus, RynkError> {
        Ok(self.ctx.connection_status())
    }
}

#[cfg(feature = "_ble")]
impl Handle<GetBleStatus> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<BleStatus, RynkError> {
        Ok(self.ctx.connection_status().ble)
    }
}

#[cfg(feature = "_ble")]
impl Handle<SwitchBleProfile> for Cell<Option<BleProfileAction>> {
    async fn handle(&self, slot: u8) -> Result<(), RynkError> {
        check_profile_request(slot)?;
        self.set(Some(BleProfileAction::Switch(slot)));
        Ok(())
    }
}

#[cfg(feature = "_ble")]
impl Handle<ClearBleProfile> for Cell<Option<BleProfileAction>> {
    async fn handle(&self, slot: u8) -> Result<(), RynkError> {
        check_profile_request(slot)?;
        self.set(Some(BleProfileAction::ClearSlot(slot)));
        Ok(())
    }
}

#[cfg(feature = "_ble")]
fn check_profile_request(slot: u8) -> Result<(), RynkError> {
    if usize::from(slot) >= crate::NUM_BLE_PROFILE {
        return Err(RynkError::Invalid);
    }
    if crate::channel::BLE_PROFILE_CHANNEL.is_full() {
        return Err(RynkError::NotReady);
    }
    Ok(())
}
