//! Vial over BLE GATT, with one incoming queue per connection.

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embedded_io_async::{ErrorType, Read, Write};
use trouble_host::prelude::*;

use super::GattWriteResult;
use crate::ble::ble_server::{Server, VialGattService};
use crate::host::transport::HostTransportError;
use crate::host::via::VialService;

/// Per-connection GATT write dispatcher for Vial over HID.
pub(crate) struct HostConnection<'a> {
    hid: &'a VialGattService,
    incoming: Channel<NoopRawMutex, [u8; 32], { crate::VIAL_CHANNEL_SIZE }>,
}

impl<'a> HostConnection<'a> {
    pub(crate) fn new(server: &'a Server<'_>) -> Self {
        Self {
            hid: &server.vial_service,
            incoming: Channel::new(),
        }
    }

    pub(crate) async fn handle_write(&self, handle: u16, data: &[u8], encrypted: bool) -> GattWriteResult {
        if handle == self.hid.output_data.handle {
            if !encrypted {
                return GattWriteResult::Handled;
            }
            if let Ok(message) = data.try_into() {
                debug!("Got Vial packet: {:?}", data);
                self.incoming.send(message).await;
            } else {
                warn!("Wrong Vial packet data: {:?}", data);
            }
            GattWriteResult::Handled
        } else if self.hid.input_data.cccd_handle == Some(handle) {
            GattWriteResult::CccdWrite
        } else if handle == self.hid.hid_control_point.handle {
            GattWriteResult::HidControlPoint
        } else {
            GattWriteResult::Unhandled
        }
    }

    pub(crate) async fn run<P: PacketPool>(&self, conn: &GattConnection<'_, '_, P>, service: &VialService<'_>) {
        let mut rx = VialBleRx(&self.incoming);
        let mut tx = VialBleTx { hid: self.hid, conn };
        service.run(&mut rx, &mut tx).await;
    }
}

struct VialBleRx<'a>(&'a Channel<NoopRawMutex, [u8; 32], { crate::VIAL_CHANNEL_SIZE }>);

impl ErrorType for VialBleRx<'_> {
    type Error = HostTransportError;
}

impl Read for VialBleRx<'_> {
    /// Vial chunks are always 32 bytes. Callers drive this via
    /// `read_exact(&mut [u8; 32])`; smaller buffers are rejected.
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let chunk = self.0.receive().await;
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
    hid: &'a VialGattService,
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
        if let Err(e) = self.hid.input_data.notify(self.conn, arr, true).await {
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
        let handler = HostConnection::new(&server);

        assert_eq!(
            block_on(handler.handle_write(server.vial_service.output_data.handle, &[], true)),
            GattWriteResult::Handled
        );
        assert_eq!(
            block_on(handler.handle_write(server.vial_service.input_data.cccd_handle.unwrap(), &[], true,)),
            GattWriteResult::CccdWrite
        );
        assert_eq!(
            block_on(handler.handle_write(server.vial_service.hid_control_point.handle, &[0], true,)),
            GattWriteResult::HidControlPoint
        );
        assert_eq!(
            block_on(handler.handle_write(u16::MAX, &[], true)),
            GattWriteResult::Unhandled
        );
    }
}
