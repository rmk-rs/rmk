//! Vial over BLE GATT.
//!
//! Only the active profile's connection may write the global
//! [`VIAL_BLE_RX_CHANNEL`]; a service-wide mutex serialises the one BLE session
//! across profile switches. Background writes are dropped without an ATT error.

use embassy_futures::select::select;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embedded_io_async::{ErrorType, Read, Write};
use trouble_host::prelude::*;

use super::HostWriteOutcome;
use crate::ble::ble_server::{Server, VialGattService};
use crate::channel::VIAL_BLE_RX_CHANNEL;
use crate::host::transport::HostTransportError;
use crate::host::via::VialService;
use crate::state::{CONNECTION_STATUS, current_profile};

pub(crate) struct BleConfig<'s> {
    service: &'s VialService<'s>,
    lock: Mutex<NoopRawMutex, ()>,
}

impl<'s> BleConfig<'s> {
    pub(crate) fn new(service: &'s VialService<'s>) -> Self {
        Self {
            service,
            lock: Mutex::new(()),
        }
    }
}

/// Per-connection GATT write dispatcher for Vial over HID.
pub(crate) struct HostGattHandler<'a, 's> {
    hid: &'a VialGattService,
    profile: u8,
    config: Option<&'a BleConfig<'s>>,
}

impl<'a, 's> HostGattHandler<'a, 's> {
    pub(crate) fn new(server: &'a Server<'_>, profile: u8, config: Option<&'a BleConfig<'s>>) -> Self {
        Self {
            hid: &server.vial_service,
            profile,
            config,
        }
    }

    pub(crate) async fn handle_write(&self, handle: u16, data: &[u8], _encrypted: bool) -> HostWriteOutcome {
        if handle == self.hid.output_data.handle {
            if let Ok(message) = data.try_into() {
                debug!("Got Vial packet: {:?}", data);
                let mut status = CONNECTION_STATUS
                    .receiver()
                    .expect("connection status subscriber capacity exceeded");
                // A switch drops a chunk that could not be queued yet; the
                // host retries once the profile is active again.
                let _ = select(
                    status.get_and(|s| s.ble.profile != self.profile),
                    VIAL_BLE_RX_CHANNEL.send(message),
                )
                .await;
            } else {
                warn!("Wrong Vial packet data: {:?}", data);
            }
            HostWriteOutcome::Handled
        } else if handle == self.hid.input_data.cccd_handle.expect("No CCCD for Vial input") {
            HostWriteOutcome::CccdUpdated
        } else if handle == self.hid.hid_control_point.handle {
            HostWriteOutcome::ControlPoint
        } else {
            HostWriteOutcome::Unhandled
        }
    }

    /// Run the one BLE Vial session. The mutex prevents the old and new active
    /// connections from draining the global channel at the same time.
    pub(crate) async fn run<P: PacketPool>(&self, conn: &GattConnection<'_, '_, P>) {
        let Some(config) = self.config else {
            return core::future::pending().await;
        };
        let mut status = CONNECTION_STATUS
            .receiver()
            .expect("connection status subscriber capacity exceeded");
        loop {
            status.get_and(|s| s.ble.profile == self.profile).await;
            let _guard = config.lock.lock().await;
            // A stale waiter must not clear chunks the new active connection
            // already queued.
            if current_profile() != self.profile {
                continue;
            }
            VIAL_BLE_RX_CHANNEL.clear();
            let mut rx = VialBleRx;
            let mut tx = VialBleTx {
                input_data: self.hid.input_data,
                conn,
            };
            select(
                status.get_and(|s| s.ble.profile != self.profile),
                config.service.run_session(&mut rx, &mut tx),
            )
            .await;
        }
    }
}

struct VialBleRx;

impl ErrorType for VialBleRx {
    type Error = HostTransportError;
}

impl Read for VialBleRx {
    /// Vial chunks are always 32 bytes. Callers drive this via
    /// `read_exact(&mut [u8; 32])`; smaller buffers are rejected.
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let chunk = VIAL_BLE_RX_CHANNEL.receive().await;
        if buf.len() < chunk.len() {
            error!(
                "VialBleRx::read called with buf.len() = {} < chunk.len() = {}",
                buf.len(),
                chunk.len()
            );
            return Err(HostTransportError);
        }
        buf[..chunk.len()].copy_from_slice(&chunk);
        Ok(chunk.len())
    }
}

struct VialBleTx<'a, 'b, 'c, P: PacketPool> {
    input_data: Characteristic<[u8; 32]>,
    conn: &'a GattConnection<'b, 'c, P>,
}

impl<P: PacketPool> ErrorType for VialBleTx<'_, '_, '_, P> {
    type Error = HostTransportError;
}

impl<P: PacketPool> Write for VialBleTx<'_, '_, '_, P> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        // The GATT input_data characteristic is fixed-size [u8; 32]; expect
        // exactly 32 bytes per write call.
        let arr: &[u8; 32] = buf.try_into().map_err(|_| {
            error!("Vial reply must be exactly 32 bytes, got {}", buf.len());
            HostTransportError
        })?;
        if let Err(e) = self.input_data.notify(self.conn, arr, true).await {
            error!("Failed to notify Vial reply: {:?}", e);
            return Err(HostTransportError);
        }
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn classifies_vial_gatt_handles() {
        use crate::test_support::test_block_on as block_on;

        let server = Server::new_default("rmk").unwrap();
        let handler = HostGattHandler::new(&server, 0, None);

        assert_eq!(
            block_on(handler.handle_write(server.vial_service.output_data.handle, &[], true)),
            HostWriteOutcome::Handled
        );
        assert_eq!(
            block_on(handler.handle_write(server.vial_service.input_data.cccd_handle.unwrap(), &[], true,)),
            HostWriteOutcome::CccdUpdated
        );
        assert_eq!(
            block_on(handler.handle_write(server.vial_service.hid_control_point.handle, &[0], true,)),
            HostWriteOutcome::ControlPoint
        );
        assert_eq!(
            block_on(handler.handle_write(u16::MAX, &[], true)),
            HostWriteOutcome::Unhandled
        );
    }
}
