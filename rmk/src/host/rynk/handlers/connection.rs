//! Connection handlers — USB/BLE preferred-transport read, BLE profile management.

#[cfg(feature = "_ble")]
use rmk_types::ble::BleStatus;
use rmk_types::connection::{ConnectionStatus, ConnectionType};
use rmk_types::protocol::rynk::RynkError;
#[cfg(feature = "_ble")]
use rmk_types::protocol::rynk::command::GetBleStatus;
use rmk_types::protocol::rynk::command::{GetConnectionStatus, GetConnectionType};

use super::super::RynkService;
use super::Handle;

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
