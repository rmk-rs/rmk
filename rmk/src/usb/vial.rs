//! Vial over USB HID (32-byte IN/OUT reports).

use embassy_usb::Builder;
use embassy_usb::class::hid::{HidReader, HidWriter};
use embassy_usb::driver::Driver;
use embedded_io_async::{ErrorType, Read, Write};

use crate::hid::ViaReport;
use crate::host::transport::HostTransportError;
use crate::host::via::VialService;
use crate::usb::add_usb_reader_writer;

/// Reader/writer halves of the Vial USB transport (32-byte HID reports).
pub(crate) type HostUsbReader<D> = HidReader<'static, D, 32>;
pub(crate) type HostUsbWriter<D> = HidWriter<'static, D, 32>;

/// Build the Vial HID interface (32-byte input + 32-byte output reports).
pub(crate) fn build_host_usb<D: Driver<'static>>(
    builder: &mut Builder<'static, D>,
) -> (HostUsbReader<D>, HostUsbWriter<D>) {
    let rw = add_usb_reader_writer!(builder, ViaReport, 32, 32, 32);
    rw.split()
}

impl super::HostSession for VialService<'_> {
    async fn serve<R: Read, W: Write>(&self, rx: &mut R, tx: &mut W) {
        self.run_session(rx, tx).await
    }
}

/// Vial session loop.
pub(crate) async fn run_host_usb<D: Driver<'static>, S: super::HostSession>(
    reader: &mut HostUsbReader<D>,
    writer: &mut HostUsbWriter<D>,
    session: &S,
) -> ! {
    loop {
        reader.ready().await;
        let mut rx = VialUsbRx { reader: &mut *reader };
        let mut tx = VialUsbTx { writer: &mut *writer };
        session.serve(&mut rx, &mut tx).await;
    }
}

/// Vial USB reader, implements embedded-io `Read` trait.
struct VialUsbRx<'a, D: Driver<'static>> {
    reader: &'a mut HidReader<'static, D, 32>,
}

impl<D: Driver<'static>> ErrorType for VialUsbRx<'_, D> {
    type Error = HostTransportError;
}

impl<D: Driver<'static>> Read for VialUsbRx<'_, D> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.len() < 32 {
            error!("VialUsbRx::read called with buf.len() = {} < 32", buf.len());
            return Err(HostTransportError);
        }
        match self.reader.read(&mut buf[..32]).await {
            Ok(n) => Ok(n),
            Err(e) => {
                error!("USB host read error: {:?}", e);
                Err(HostTransportError)
            }
        }
    }
}

/// Vial USB writer, implements embedded-io `Write` trait.
/// Sends one 32-byte HID report per `write` call.
struct VialUsbTx<'a, D: Driver<'static>> {
    writer: &'a mut HidWriter<'static, D, 32>,
}

impl<D: Driver<'static>> ErrorType for VialUsbTx<'_, D> {
    type Error = HostTransportError;
}

impl<D: Driver<'static>> Write for VialUsbTx<'_, D> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        // Reject non-32-byte HID reports instead of desyncing the reply stream.
        if buf.len() != 32 {
            error!("Vial reply must be exactly 32 bytes, got {}", buf.len());
            return Err(HostTransportError);
        }
        match self.writer.write(buf).await {
            Ok(()) => Ok(32),
            Err(e) => {
                error!("USB host write error: {:?}", e);
                Err(HostTransportError)
            }
        }
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
