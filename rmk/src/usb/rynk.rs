//! Rynk over USB HID.
//!
//! Rynk frames are fragmented into the same fixed 32-byte reports used by
//! Rynk's BLE WebHID transport. Report padding is zero bytes, which the COBS
//! deframer inside `RynkService` treats as inter-frame delimiters.

use embassy_usb::Builder;
use embassy_usb::class::hid::{HidReader, HidWriter};
use embassy_usb::driver::Driver;
use embedded_io_async::{ErrorType, Read, Write};
use rmk_types::protocol::rynk::RYNK_HID_REPORT_SIZE;

use super::HostSession;
use crate::hid::RynkHidReport;
use crate::host::transport::HostTransportError;
use crate::usb::add_usb_reader_writer;

#[cfg(feature = "rynk")]
impl HostSession for crate::host::rynk::RynkService<'_> {
    async fn serve<R: Read, W: Write>(&self, rx: &mut R, tx: &mut W) {
        self.run_session(rx, tx).await
    }
}

#[cfg(feature = "dongle")]
impl HostSession for crate::dongle::DongleRouter {
    async fn serve<R: Read, W: Write>(&self, rx: &mut R, tx: &mut W) {
        self.run_session(rx, tx).await
    }
}

pub(crate) type HostUsbReader<D> = HidReader<'static, D, RYNK_HID_REPORT_SIZE>;
pub(crate) type HostUsbWriter<D> = HidWriter<'static, D, RYNK_HID_REPORT_SIZE>;

/// Build the Rynk vendor-HID interface.
pub(crate) fn build_host_usb<D: Driver<'static>>(
    builder: &mut Builder<'static, D>,
) -> (HostUsbReader<D>, HostUsbWriter<D>) {
    add_usb_reader_writer!(builder, RynkHidReport, RYNK_HID_REPORT_SIZE, RYNK_HID_REPORT_SIZE, 32).split()
}

/// Run one Rynk session for each USB connection.
pub(crate) async fn run_host_usb<D: Driver<'static>, S: HostSession>(
    reader: &mut HostUsbReader<D>,
    writer: &mut HostUsbWriter<D>,
    service: &S,
) -> ! {
    loop {
        reader.ready().await;
        let mut rx = RynkUsbRx::new(&mut *reader);
        let mut tx = RynkUsbTx { writer: &mut *writer };
        service.serve(&mut rx, &mut tx).await;
    }
}

struct RynkUsbRx<'a, D: Driver<'static>> {
    reader: &'a mut HostUsbReader<D>,
    report: [u8; RYNK_HID_REPORT_SIZE],
    pos: usize,
    end: usize,
}

impl<'a, D: Driver<'static>> RynkUsbRx<'a, D> {
    fn new(reader: &'a mut HostUsbReader<D>) -> Self {
        Self {
            reader,
            report: [0; RYNK_HID_REPORT_SIZE],
            pos: 0,
            end: 0,
        }
    }
}

impl<D: Driver<'static>> ErrorType for RynkUsbRx<'_, D> {
    type Error = HostTransportError;
}

impl<D: Driver<'static>> Read for RynkUsbRx<'_, D> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.pos < self.end {
                let n = (self.end - self.pos).min(buf.len());
                buf[..n].copy_from_slice(&self.report[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }

            let n = self
                .reader
                .read(&mut self.report)
                .await
                .map_err(|_| HostTransportError)?;
            self.pos = 0;
            self.end = n;
        }
    }
}

struct RynkUsbTx<'a, D: Driver<'static>> {
    writer: &'a mut HostUsbWriter<D>,
}

impl<D: Driver<'static>> ErrorType for RynkUsbTx<'_, D> {
    type Error = HostTransportError;
}

impl<D: Driver<'static>> Write for RynkUsbTx<'_, D> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        for chunk in buf.chunks(RYNK_HID_REPORT_SIZE) {
            let mut report = [0u8; RYNK_HID_REPORT_SIZE];
            report[..chunk.len()].copy_from_slice(chunk);
            self.writer.write(&report).await.map_err(|_| HostTransportError)?;
        }
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
