//! Rynk over the RMK vendor bulk USB interface, using `nusb`.
//!
//! [`UsbDevice::discover`] lists keyboards by the vendor interface class
//! triple the firmware advertises, so any VID/PID matches without host-side
//! configuration. Enumeration reads cached descriptors and never opens a
//! device; the chosen device is opened once, by
//! [`RynkDevice::connect`]. Dropping the halves (with the owning session)
//! ends the Rynk **session** only: the keyboard stays connected and usable.

use embedded_io_adapters::tokio_1::FromTokio;
use nusb::io::EndpointRead;
use nusb::transfer::{Bulk, In, Out};
use nusb::{DeviceInfo, Endpoint};
use rynk::rmk_types::protocol::rynk::{
    RYNK_USB_INTERFACE_CLASS, RYNK_USB_INTERFACE_PROTOCOL, RYNK_USB_INTERFACE_SUBCLASS,
};
use rynk::{RynkDevice, RynkHostError};

/// Device→host read-transfer size: covers a whole Rynk frame per transfer and
/// is a multiple of both Full- and High-Speed bulk packet sizes.
const READ_TRANSFER_SIZE: usize = 4096;

/// The device→host half: nusb's transfer pump behind the embedded-io adapter.
/// Zero-length packets (the firmware's transfer delimiters) are absorbed by
/// `EndpointRead`, which polls on until data arrives.
pub type UsbReader = FromTokio<EndpointRead<Bulk>>;

/// A Rynk keyboard found by [`UsbDevice::discover`], for building a device
/// picker. Version and capabilities are read by
/// [`connect`](RynkDevice::connect), which is when the device is first opened.
pub struct UsbDevice {
    info: DeviceInfo,
    interface: u8,
}

impl UsbDevice {
    /// List devices carrying the Rynk vendor interface — one [`UsbDevice`]
    /// per keyboard, recognized by the interface class triple without opening
    /// anything.
    pub async fn discover() -> Result<Vec<Self>, RynkHostError> {
        let devices = nusb::list_devices()
            .await
            .map_err(|e| RynkHostError::Transport("list_devices", e.to_string()))?;
        Ok(devices
            .filter_map(|info| {
                let interface = info
                    .interfaces()
                    .find(|i| {
                        i.class() == RYNK_USB_INTERFACE_CLASS
                            && i.subclass() == RYNK_USB_INTERFACE_SUBCLASS
                            && i.protocol() == RYNK_USB_INTERFACE_PROTOCOL
                    })
                    .map(|i| i.interface_number())?;
                Some(Self { info, interface })
            })
            .collect())
    }

    /// Enumeration-stable identity of the underlying device, for matching a
    /// picked entry back to a fresh [`discover`](Self::discover) list on
    /// connect (the role the serial transport's port path used to play).
    pub fn id(&self) -> nusb::DeviceId {
        self.info.id()
    }
}

impl RynkDevice for UsbDevice {
    type Read = UsbReader;
    type Write = UsbWriter;

    /// The USB product string, falling back to the numeric ids when the
    /// descriptor carried none.
    fn label(&self) -> String {
        match self.info.product_string() {
            Some(name) => name.to_owned(),
            None => format!("USB {:04x}:{:04x}", self.info.vendor_id(), self.info.product_id()),
        }
    }

    /// Open the device and claim the vendor interface. A device unplugged
    /// since discovery surfaces as a normal [`RynkHostError`].
    async fn open(self) -> Result<(UsbReader, UsbWriter), RynkHostError> {
        let device = self
            .info
            .open()
            .await
            .map_err(|e| RynkHostError::Transport("open", e.to_string()))?;
        let interface = device
            .claim_interface(self.interface)
            .await
            .map_err(|e| RynkHostError::Transport("claim_interface", e.to_string()))?;

        let mut in_addr = None;
        let mut out_addr = None;
        let descriptor = interface
            .descriptor()
            .ok_or_else(|| RynkHostError::Transport("descriptor", "missing interface descriptor".into()))?;
        for ep in descriptor.endpoints() {
            match ep.direction() {
                nusb::transfer::Direction::In => in_addr = Some(ep.address()),
                nusb::transfer::Direction::Out => out_addr = Some(ep.address()),
            }
        }
        let (Some(in_addr), Some(out_addr)) = (in_addr, out_addr) else {
            return Err(RynkHostError::Transport(
                "endpoints",
                "vendor interface lacks a bulk endpoint pair".into(),
            ));
        };

        let ep_in: Endpoint<Bulk, In> = interface
            .endpoint(in_addr)
            .map_err(|e| RynkHostError::Transport("endpoint_in", e.to_string()))?;
        let ep_out: Endpoint<Bulk, Out> = interface
            .endpoint(out_addr)
            .map_err(|e| RynkHostError::Transport("endpoint_out", e.to_string()))?;
        Ok((
            FromTokio::new(EndpointRead::new(ep_in, READ_TRANSFER_SIZE)),
            UsbWriter { ep: ep_out },
        ))
    }
}

/// The host→device half: one bulk transfer per `write`, awaited to
/// completion. The Rynk driver never flushes, so nusb's own buffering
/// `EndpointWrite` (which submits only full transfers) would strand frames.
pub struct UsbWriter {
    ep: Endpoint<Bulk, Out>,
}

impl rynk::io::ErrorType for UsbWriter {
    type Error = std::io::Error;
}

impl rynk::io::Write for UsbWriter {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        // A transfer still pending is a cancelled `write` of these same bytes,
        // which the driver is retrying; submitting again would send them twice.
        if self.ep.pending() == 0 {
            let mut transfer = self.ep.allocate(buf.len());
            transfer.extend_from_slice(buf);
            self.ep.submit(transfer);
        }
        let completion = core::future::poll_fn(|cx| self.ep.poll_next_complete(cx)).await;
        completion.status.map_err(std::io::Error::other)?;
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        // `write` completes the transfer before returning; nothing is buffered.
        Ok(())
    }
}
