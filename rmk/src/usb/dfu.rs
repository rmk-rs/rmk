use core::sync::atomic::Ordering;

use embassy_usb::Handler;
use embassy_usb::class::dfu::consts::{DfuAttributes, Status};
use embassy_usb::class::dfu::dfu_mode::{self, DfuState};
use embassy_usb::control::{InResponse, OutResponse, Recipient, Request, RequestType};
use embassy_usb::driver::Direction;
use embassy_usb::types::{InterfaceNumber, StringIndex};
use rmk_types::dfu::DfuStatus;
use static_cell::StaticCell;

use crate::dfu::{BLOCK_SIZE_DFU, DFU_WRITE_FAILED, DfuCmd, MAX_DFU_ALTS};
use crate::event::{DfuCmdEvent, DfuStatusEvent, publish_event};

/// Synchronous DFU handler for every alternate setting (alt 0 = central,
/// alt 1..N = split peripherals).
///
/// Runs inside the USB interrupt. It never touches flash: every download
/// `start`/`write`/`finish`/`system_reset` is forwarded to the async
/// [`FlashDfuHandler`](crate::dfu::FlashDfuHandler) updater task through
/// the event system ([`DfuCmdEvent`](crate::event::DfuCmdEvent)). The DFU
/// lock gate (if enabled) is checked here so every DFU start path shares
/// one place.
struct ProxyUsbDfuHandler {
    target: crate::dfu::DfuTarget,
    /// Running byte offset — each `Write` advances by the block size.
    written: u32,
}

impl dfu_mode::Handler for ProxyUsbDfuHandler {
    fn start(&mut self) -> Result<(), Status> {
        crate::dfu::dfu_lock_check()?;
        self.written = 0;
        info!("dfu: DFU download started ({:?})", self.target);
        publish_event(DfuCmdEvent(DfuCmd::Start(self.target)));
        publish_event(DfuStatusEvent::new(DfuStatus::Started));
        Ok(())
    }

    fn write(&mut self, data: &[u8]) -> Result<(), Status> {
        if DFU_WRITE_FAILED.load(Ordering::Acquire) {
            DFU_WRITE_FAILED.store(false, Ordering::Release);
            return Err(Status::ErrWrite);
        }
        let mut buf: heapless::Vec<u8, { BLOCK_SIZE_DFU }> = heapless::Vec::new();
        buf.extend_from_slice(data).map_err(|_| Status::ErrUnknown)?;
        let offset = self.written;
        self.written = self.written.checked_add(data.len() as u32).ok_or(Status::ErrAddress)?;
        publish_event(DfuCmdEvent(DfuCmd::Write(self.target, offset, buf)));
        publish_event(DfuStatusEvent::new(DfuStatus::Downloading));
        Ok(())
    }

    fn finish(&mut self) -> Result<(), Status> {
        if DFU_WRITE_FAILED.load(Ordering::Acquire) {
            DFU_WRITE_FAILED.store(false, Ordering::Release);
            return Err(Status::ErrWrite);
        }
        publish_event(DfuCmdEvent(DfuCmd::Finish(self.target)));
        publish_event(DfuStatusEvent::new(DfuStatus::Finished));
        info!("dfu: DFU download complete");
        Ok(())
    }

    fn system_reset(&mut self) {
        publish_event(DfuCmdEvent(DfuCmd::SystemReset(self.target)));
    }
}

/// Owner of every DFU alternate setting registered on a single USB interface.
///
/// Alt 0 is the device's own DFU download (forwarded by [`ProxyUsbDfuHandler`]
/// with `DfuTarget::Central` to the async updater); alt 1..N are split
/// peripheral slots (requires `dfu_split`), forwarded with
/// `DfuTarget::ForwardPeripheral(n)`. Routes by the current alternate setting and
/// injects adaptive host-side flow control (`dfuDNBUSY`) while commands are
/// still in flight.
struct UsbDfuIface {
    handlers: [Option<DfuState<ProxyUsbDfuHandler>>; MAX_DFU_ALTS],
    current_alt: u8,
}

impl Handler for UsbDfuIface {
    fn set_alternate_setting(&mut self, _iface: InterfaceNumber, alternate_setting: u8) {
        if (alternate_setting as usize) < self.handlers.len() {
            self.current_alt = alternate_setting;
        }
    }

    fn control_out(&mut self, req: Request, data: &[u8]) -> Option<OutResponse> {
        const DFU_DNLOAD: u8 = 1;
        const DFU_CLRSTATUS: u8 = 4;

        let alt = self.current_alt as usize;
        // When block 0 arrives with data, it starts a new
        // download session. If the DfuState machine is stale from a previous
        // session (next_block_num > 0), the block-num check will reject it.
        // Inject a DFU_CLRSTATUS first to reset next_block_num
        // and state to DfuIdle.  ClrStatus on an already-idle machine is a
        // harmless no-op.
        if req.request == DFU_DNLOAD && req.value == 0 && !data.is_empty() {
            if let Some(handler) = self.handlers[alt].as_mut() {
                handler.control_out(
                    Request {
                        direction: Direction::Out,
                        request_type: RequestType::Class,
                        recipient: Recipient::Interface,
                        request: DFU_CLRSTATUS,
                        value: 0,
                        index: self.current_alt as u16,
                        length: 0,
                    },
                    &[],
                );
            }
        }
        self.handlers[alt].as_mut()?.control_out(req, data)
    }

    fn control_in<'a>(&'a mut self, req: Request, buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        const DFU_GETSTATUS: u8 = 3;

        if !crate::event::DfuCmdEvent::empty()
            && req.request == DFU_GETSTATUS
            && req.request_type == RequestType::Class
            && req.recipient == Recipient::Interface
        {
            // Short-circuit: return dfuDNBUSY directly without
            // advancing the DfuState machine. The state stays in
            // DlSync so the next real GETSTATUS (after the queue
            // drains) correctly transitions to Download.
            //
            // GETSTATUS response (DFU 1.1, Table A.3):
            let resp: [u8; 6] = [
                0x00, // bmAttributes
                0x0A, 0x00, 0x00, // bwPollTimeout = 10 ms (3 bytes LE)
                4,    // bState = DlBusy
                0x00, // iString (none)
            ];
            buf[..6].copy_from_slice(&resp);
            return Some(InResponse::Accepted(&buf[..6]));
        }
        self.handlers[self.current_alt as usize].as_mut()?.control_in(req, buf)
    }
}

/// Provides the DFU product string for the DFU interface's alt settings.
///
/// DFU hosts (e.g. dfu-util) show this string in place of the raw index; it is
/// parked alongside [`UsbDfuIface`] so it lives for the USB device's lifetime.
struct DfuStringProvider {
    string_idx: StringIndex,
    string_val: &'static str,
}

impl Handler for DfuStringProvider {
    fn control_out(&mut self, _req: Request, _data: &[u8]) -> Option<OutResponse> {
        None
    }
    fn control_in<'a>(&'a mut self, _req: Request, _buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        None
    }
    fn get_string(&mut self, index: StringIndex, _lang_id: u16) -> Option<&'static str> {
        (index == self.string_idx).then_some(self.string_val)
    }
}

/// Register a DFU interface on the USB builder.
///
/// Alt 0 is the device's own DFU download partition; `num_peripherals` more
/// alts (up to [`MAX_DFU_ALTS`](crate::dfu::MAX_DFU_ALTS)) become split
/// peripheral slots (requires `dfu_split`). The parked proxy ([`UsbDfuIface`])
/// does all routing and never touches flash — downloads flow through the
/// command channel to the [`FlashDfuHandler`](crate::dfu::FlashDfuHandler)
/// updater task.
pub(crate) fn register_dfu_iface<D: embassy_usb::driver::Driver<'static>>(
    builder: &mut embassy_usb::Builder<'static, D>,
    product_name: &'static str,
    #[cfg(feature = "dfu_split")] num_peripherals: usize,
) {
    let central_attrs = DfuAttributes::CAN_DOWNLOAD | DfuAttributes::WILL_DETACH;
    let string_idx = builder.string();

    let mut func = builder.function(0x00, 0x00, 0x00); // class/subclass/protocol deferred to interface
    let mut iface = func.interface();
    let mut alt = iface.alt_setting(0xFE, 0x01, 0x02, Some(string_idx)); // class=AppSpecific, sub=DFU, proto=DFU mode
    alt.descriptor(
        0x21, // DFU FUNCTIONAL descriptor type
        &[
            central_attrs.bits(), // bmAttributes
            0xc4,
            0x09,                                 // wDetachTimeout = 2500 ms (LE)
            (BLOCK_SIZE_DFU & 0xff) as u8,        // wTransferSize LSB
            ((BLOCK_SIZE_DFU >> 8) & 0xff) as u8, // wTransferSize MSB
            0x10,
            0x01, // bcdDFUVersion = 1.1 (LE)
        ],
    );

    #[cfg(feature = "dfu_split")]
    let num_split = num_peripherals.min(MAX_DFU_ALTS - 1);
    #[cfg(feature = "dfu_split")]
    for _ in 0..num_split {
        let mut alt = iface.alt_setting(0xFE, 0x01, 0x02, Some(string_idx)); // class=AppSpecific, sub=DFU, proto=DFU mode
        alt.descriptor(
            0x21, // DFU FUNCTIONAL descriptor type
            &[
                DfuAttributes::CAN_DOWNLOAD.bits(), // bmAttributes
                0xc4,
                0x09,                                 // wDetachTimeout = 2500 ms (LE)
                (BLOCK_SIZE_DFU & 0xff) as u8,        // wTransferSize LSB
                ((BLOCK_SIZE_DFU >> 8) & 0xff) as u8, // wTransferSize MSB
                0x10,
                0x01, // bcdDFUVersion = 1.1 (LE)
            ],
        );
    }
    drop(func);

    static DFU_IFACE: StaticCell<UsbDfuIface> = StaticCell::new();
    let dfu_iface = DFU_IFACE.init(UsbDfuIface {
        handlers: {
            let mut slots: [Option<DfuState<ProxyUsbDfuHandler>>; MAX_DFU_ALTS] = Default::default();
            // Alt 0: central
            slots[0] = Some(DfuState::new(
                ProxyUsbDfuHandler {
                    target: crate::dfu::DfuTarget::Central,
                    written: 0,
                },
                central_attrs,
            ));
            // Alt 1..N: split peripherals
            #[cfg(feature = "dfu_split")]
            for id in 0..num_split {
                slots[id + 1] = Some(DfuState::new(
                    ProxyUsbDfuHandler {
                        target: crate::dfu::DfuTarget::ForwardPeripheral(id as u8),
                        written: 0,
                    },
                    DfuAttributes::CAN_DOWNLOAD,
                ));
            }
            slots
        },
        current_alt: 0,
    });
    builder.handler(dfu_iface);

    static STRING_PROVIDER: StaticCell<DfuStringProvider> = StaticCell::new();
    let string_provider = STRING_PROVIDER.init(DfuStringProvider {
        string_idx,
        string_val: product_name,
    });
    builder.handler(string_provider);
}
