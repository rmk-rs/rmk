//! Vial over BLE GATT.
//!
//! Only the active profile's connection may write the global
//! [`VIAL_BLE_RX_CHANNEL`]; a service-wide mutex serialises the one BLE session
//! across profile switches. Background writes are dropped without an ATT error.

use embassy_futures::select::select;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::watch::Receiver;
use embedded_io_async::{ErrorType, Read, Write};
use trouble_host::prelude::*;

use super::HostWriteOutcome;
use crate::ble::ACTIVE_RECEIVERS;
use crate::ble::ble_server::Server;
use crate::channel::VIAL_BLE_RX_CHANNEL;
use crate::host::transport::HostTransportError;
use crate::host::via::VialService;

/// Per-connection GATT write dispatcher for Vial over HID.
pub(crate) struct HostGattHandler<'a> {
    output_handle: u16,
    input_cccd_handle: u16,
    control_point_handle: u16,
    active: Receiver<'a, NoopRawMutex, u8, ACTIVE_RECEIVERS>,
    profile: u8,
}

impl<'a> HostGattHandler<'a> {
    pub(crate) fn new(
        server: &Server<'_>,
        active: Receiver<'a, NoopRawMutex, u8, ACTIVE_RECEIVERS>,
        profile: u8,
    ) -> Self {
        Self {
            output_handle: server.vial_service.output_data.handle,
            input_cccd_handle: server
                .vial_service
                .input_data
                .cccd_handle
                .expect("No CCCD for Vial input"),
            control_point_handle: server.vial_service.hid_control_point.handle,
            active,
            profile,
        }
    }

    pub(crate) async fn handle_write(&mut self, handle: u16, data: &[u8], _encrypted: bool) -> HostWriteOutcome {
        if handle == self.output_handle {
            if data.len() == 32 {
                debug!("Got Vial packet: {:?}", data);
                if self.active.try_get_and(|&p| p == self.profile).is_some() {
                    let mut message = [0u8; 32];
                    message.copy_from_slice(data);
                    // A switch drops a chunk that could not be queued yet; the
                    // host retries once the profile is active again.
                    let _ = select(
                        self.active.changed_and(|&p| p != self.profile),
                        VIAL_BLE_RX_CHANNEL.send(message),
                    )
                    .await;
                }
            } else {
                warn!("Wrong Vial packet data: {:?}", data);
            }
            HostWriteOutcome::Handled
        } else if handle == self.input_cccd_handle {
            HostWriteOutcome::CccdUpdated
        } else if handle == self.control_point_handle {
            HostWriteOutcome::ControlPoint
        } else {
            HostWriteOutcome::Unhandled
        }
    }

    /// Run the one BLE Vial session. The mutex prevents the old and new active
    /// connections from draining the global channel at the same time.
    pub(crate) async fn run<'stack, 'server, P: PacketPool>(
        server: &'server Server<'_>,
        conn: &GattConnection<'stack, 'server, P>,
        service: &VialService<'_>,
        mut active: Receiver<'_, NoopRawMutex, u8, ACTIVE_RECEIVERS>,
        profile: u8,
        vial_lock: &Mutex<NoopRawMutex, ()>,
    ) {
        loop {
            active.get_and(|&p| p == profile).await;
            let _guard = vial_lock.lock().await;
            // A stale waiter must not clear chunks the new active connection
            // already queued.
            if active.try_get_and(|&p| p == profile).is_none() {
                continue;
            }
            VIAL_BLE_RX_CHANNEL.clear();
            let mut rx = VialBleRx;
            let mut tx = VialBleTx {
                input_data: server.vial_service.input_data,
                conn,
            };
            select(
                active.changed_and(|&p| p != profile),
                service.run_session(&mut rx, &mut tx),
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
    use embassy_sync::watch::Watch;

    use super::*;

    #[test]
    fn classifies_vial_gatt_handles() {
        use crate::test_support::test_block_on as block_on;

        let server = Server::new_default("rmk").unwrap();
        let watch = Watch::<NoopRawMutex, u8, ACTIVE_RECEIVERS>::new();
        let active = watch.receiver().unwrap();
        let mut handler = HostGattHandler::new(&server, active, 0);

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

    #[test]
    fn only_the_active_profile_queues_chunks() {
        use crate::test_support::test_block_on as block_on;

        let server = Server::new_default("rmk").unwrap();
        let watch = Watch::<NoopRawMutex, u8, ACTIVE_RECEIVERS>::new();
        watch.sender().send(0);
        let handle = server.vial_service.output_data.handle;
        while VIAL_BLE_RX_CHANNEL.try_receive().is_ok() {}

        let mut background = HostGattHandler::new(&server, watch.receiver().unwrap(), 1);
        block_on(background.handle_write(handle, &[0xAA; 32], true));
        assert!(VIAL_BLE_RX_CHANNEL.try_receive().is_err());

        let mut active = HostGattHandler::new(&server, watch.receiver().unwrap(), 0);
        block_on(active.handle_write(handle, &[0xAA; 32], true));
        assert_eq!(VIAL_BLE_RX_CHANNEL.try_receive().unwrap(), [0xAA; 32]);
    }
}
