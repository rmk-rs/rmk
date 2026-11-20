//! Vial over USB HID (32-byte IN/OUT reports).

use embassy_usb::Builder;
use embassy_usb::class::hid::device::HidWriter;
#[cfg(not(feature = "vial_kvm"))]
use embassy_usb::class::hid::device::{HidReader, HidReaderWriter};
use embassy_usb::class::hid::{HidBootProtocol, HidSubclass};
use embassy_usb::driver::Driver;
use embedded_io_async::{ErrorType, Read, Write};
use rmk_types::protocol::vial::VIAL_EP_SIZE;

#[cfg(feature = "vial_kvm")]
use self::control::Reports as ControlHandler;
#[cfg(not(feature = "vial_kvm"))]
use super::UsbRequestHandler as ControlHandler;
use crate::hid::ViaReport;
use crate::host::transport::HostTransportError;
use crate::host::via::VialService;

#[cfg(feature = "vial_kvm")]
pub(super) mod control;

pub(crate) struct HostUsb<D: Driver<'static>> {
    #[cfg(not(feature = "vial_kvm"))]
    reader: HidReader<'static, D, VIAL_EP_SIZE>,
    writer: HidWriter<'static, D, VIAL_EP_SIZE>,
}

pub(crate) fn build_host_usb<D: Driver<'static>>(builder: &mut Builder<'static, D>) -> HostUsb<D> {
    static HANDLER: static_cell::StaticCell<ControlHandler> = static_cell::StaticCell::new();
    let (state, config) = super::usb_hid_state_and_config!(
        ViaReport,
        VIAL_EP_SIZE as u16,
        HidSubclass::No,
        HidBootProtocol::None,
        HANDLER.init_with(Default::default)
    );
    #[cfg(not(feature = "vial_kvm"))]
    let (reader, writer) = HidReaderWriter::new(builder, state, config).split();
    #[cfg(feature = "vial_kvm")]
    let writer = HidWriter::new(builder, state, config);
    HostUsb {
        #[cfg(not(feature = "vial_kvm"))]
        reader,
        writer,
    }
}

impl super::HostSession for VialService<'_> {
    async fn serve<R: Read, W: Write>(&self, rx: &mut R, tx: &mut W) {
        self.run(rx, tx).await
    }
}

#[cfg(feature = "dongle")]
impl super::HostSession for crate::dongle::DongleRouter {
    async fn serve<R: Read, W: Write>(&self, rx: &mut R, tx: &mut W) {
        self.run_session(rx, tx).await
    }
}

/// Vial session loop.
pub(crate) async fn run_host_usb<D: Driver<'static>, S: super::HostSession>(usb: &mut HostUsb<D>, session: &S) -> ! {
    let mut tx = VialUsbTx {
        writer: &mut usb.writer,
    };
    loop {
        tx.writer.ready().await;
        #[cfg(not(feature = "vial_kvm"))]
        let mut rx = VialUsbRx {
            reader: &mut usb.reader,
        };
        #[cfg(feature = "vial_kvm")]
        let mut rx = control::Reports;
        let serve = session.serve(&mut rx, &mut tx);
        #[cfg(not(feature = "vial_kvm"))]
        serve.await;
        #[cfg(feature = "vial_kvm")]
        embassy_futures::select::select(control::SESSION_ENDED.wait(), serve).await;
    }
}

/// Vial USB reader, implements embedded-io `Read` trait.
#[cfg(not(feature = "vial_kvm"))]
struct VialUsbRx<'a, D: Driver<'static>> {
    reader: &'a mut HidReader<'static, D, 32>,
}

#[cfg(not(feature = "vial_kvm"))]
impl<D: Driver<'static>> ErrorType for VialUsbRx<'_, D> {
    type Error = HostTransportError;
}

#[cfg(not(feature = "vial_kvm"))]
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
