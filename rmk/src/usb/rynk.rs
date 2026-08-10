//! Rynk over a vendor-specific USB bulk interface.

use embassy_usb::driver::{Driver, Endpoint, EndpointError, EndpointIn, EndpointOut};
use embassy_usb::{Builder, msos};
use embedded_io_async::{ErrorType, Read, Write};
use rmk_types::protocol::rynk::{RYNK_USB_INTERFACE_CLASS, RYNK_USB_INTERFACE_PROTOCOL, RYNK_USB_INTERFACE_SUBCLASS};

/// One framed session over the Rynk byte stream. Served by the keyboard's
/// `RynkService` and the dongle's `DongleRouter`; a binary attaches one of
/// them to [`crate::usb::UsbTransport`], so both can coexist in one build.
pub(crate) trait RynkUsbService {
    async fn serve<R: Read, W: Write>(&self, rx: &mut R, tx: &mut W);
}

#[cfg(feature = "rynk")]
impl RynkUsbService for crate::host::rynk::RynkService<'_> {
    async fn serve<R: Read, W: Write>(&self, rx: &mut R, tx: &mut W) {
        self.run_session(rx, tx).await
    }
}

#[cfg(feature = "dongle")]
impl RynkUsbService for crate::dongle::DongleRouter {
    async fn serve<R: Read, W: Write>(&self, rx: &mut R, tx: &mut W) {
        self.run_session(rx, tx).await
    }
}

#[cfg(feature = "_usb_high_speed")]
const RYNK_USB_MAX_PACKET_SIZE: usize = 512;
#[cfg(not(feature = "_usb_high_speed"))]
const RYNK_USB_MAX_PACKET_SIZE: usize = 64;

/// bRequest value Windows sends to fetch the MS OS 2.0 descriptor set.
const MSOS_VENDOR_CODE: u8 = 0x52;

/// GUID WinUSB registers the Rynk interface under; Windows hosts open the
/// device node by it.
const DEVICE_INTERFACE_GUID: &str = "{CE60F742-A8DB-43C4-8B97-7C41B43CD4AA}";

pub(crate) struct HostUsbReader<D: Driver<'static>> {
    ep: D::EndpointOut,
    buf: [u8; RYNK_USB_MAX_PACKET_SIZE],
    pos: usize,
    len: usize,
}

/// Writer half of the Rynk USB transport.
pub(crate) struct HostUsbWriter<D: Driver<'static>> {
    ep: D::EndpointIn,
}

/// Build the Rynk vendor bulk interface and its WinUSB binding.
pub fn build_host_usb<D: Driver<'static>>(builder: &mut Builder<'static, D>) -> (HostUsbReader<D>, HostUsbWriter<D>) {
    builder.msos_descriptor(msos::windows_version::WIN8_1, MSOS_VENDOR_CODE);
    let mut function = builder.function(
        RYNK_USB_INTERFACE_CLASS,
        RYNK_USB_INTERFACE_SUBCLASS,
        RYNK_USB_INTERFACE_PROTOCOL,
    );
    function.msos_feature(msos::CompatibleIdFeatureDescriptor::new("WINUSB", ""));
    function.msos_feature(msos::RegistryPropertyFeatureDescriptor::new(
        "DeviceInterfaceGUIDs",
        msos::PropertyData::RegMultiSz(&[DEVICE_INTERFACE_GUID]),
    ));
    let mut interface = function.interface();
    let mut alt = interface.alt_setting(
        RYNK_USB_INTERFACE_CLASS,
        RYNK_USB_INTERFACE_SUBCLASS,
        RYNK_USB_INTERFACE_PROTOCOL,
        None,
    );
    let ep_out = alt.endpoint_bulk_out(None, RYNK_USB_MAX_PACKET_SIZE as u16);
    let ep_in = alt.endpoint_bulk_in(None, RYNK_USB_MAX_PACKET_SIZE as u16);
    (
        HostUsbReader {
            ep: ep_out,
            buf: [0; RYNK_USB_MAX_PACKET_SIZE],
            pos: 0,
            len: 0,
        },
        HostUsbWriter { ep: ep_in },
    )
}

/// Rynk session loop
pub(crate) async fn run_host_usb<D: Driver<'static>, S: RynkUsbService>(
    receiver: &mut HostUsbReader<D>,
    sender: &mut HostUsbWriter<D>,
    service: &S,
) -> ! {
    loop {
        receiver.ep.wait_enabled().await;
        // A bus reset voids any half-consumed packet from the last session.
        receiver.pos = 0;
        receiver.len = 0;
        service.serve(&mut *receiver, &mut *sender).await;
    }
}

impl<D: Driver<'static>> ErrorType for HostUsbReader<D> {
    type Error = EndpointError;
}

impl<D: Driver<'static>> Read for HostUsbReader<D> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.pos < self.len {
                let n = (self.len - self.pos).min(buf.len());
                buf[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }
            // Zero-length packets are transfer delimiters, not data — and not
            // EOF, which is what returning `Ok(0)` would mean. Read on.
            if buf.len() >= RYNK_USB_MAX_PACKET_SIZE {
                let n = self.ep.read(buf).await?;
                if n > 0 {
                    return Ok(n);
                }
            } else {
                self.pos = 0;
                self.len = self.ep.read(&mut self.buf).await?;
            }
        }
    }
}

impl<D: Driver<'static>> ErrorType for HostUsbWriter<D> {
    type Error = EndpointError;
}

/// Sends one frame per `write`, then a zero-length packet when the frame
/// fills the last bulk-IN packet. A bulk IN transfer completes on the host
/// only at a packet shorter than the max packet size, so a frame whose length
/// is a multiple of it would otherwise hang the host read (hit at Full-Speed's
/// 64-byte packets; masked at High-Speed's 512). `run_session` writes each
/// frame with a single `write_all`, so `buf` is one whole frame.
impl<D: Driver<'static>> Write for HostUsbWriter<D> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        for packet in buf.chunks(RYNK_USB_MAX_PACKET_SIZE) {
            self.ep.write(packet).await?;
        }
        if !buf.is_empty() && buf.len().is_multiple_of(RYNK_USB_MAX_PACKET_SIZE) {
            self.ep.write(&[]).await?;
        }
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        // `write` hands packets straight to the endpoint; nothing is buffered.
        Ok(())
    }
}
