#[cfg(feature = "_dfu")]
use core::sync::atomic::Ordering;

use embassy_futures::join::join5;
use embassy_futures::select::{Either, select};
use embassy_sync::signal::Signal;
#[cfg(feature = "usb_log")]
use embassy_usb::class::cdc_acm::CdcAcmClass;
#[cfg(feature = "_dfu")]
use embassy_usb::class::dfu::consts::{DfuAttributes, Status};
#[cfg(feature = "_dfu")]
use embassy_usb::class::dfu::dfu_mode::{self, DfuState};
use embassy_usb::class::hid::{HidProtocolMode, HidReader, HidReaderWriter, HidWriter, ReportId, RequestHandler};
use embassy_usb::control::OutResponse;
#[cfg(feature = "_dfu")]
use embassy_usb::control::{InResponse, Recipient, Request, RequestType};
#[cfg(feature = "_dfu")]
use embassy_usb::driver::Direction;
use embassy_usb::driver::{Driver, EndpointError};
#[cfg(feature = "_dfu")]
use embassy_usb::types::{InterfaceNumber, StringIndex};
use embassy_usb::{Builder, Handler, UsbDevice};
use rmk_types::connection::{ConnectionType, UsbState};
#[cfg(feature = "_dfu")]
use rmk_types::dfu::DfuStatus;
use static_cell::StaticCell;
use usbd_hid::descriptor::AsInputReport;

use crate::RawMutex;
use crate::channel::USB_REPORT_CHANNEL;
use crate::config::DeviceConfig;
use crate::core_traits::Runnable;
#[cfg(feature = "dfu_lock")]
use crate::dfu::DFU_STARTED;
#[cfg(feature = "_dfu")]
use crate::dfu::DFU_WRITE_FAILED;
#[cfg(feature = "_dfu")]
use crate::dfu::MAX_DFU_ALTS;
#[cfg(feature = "_dfu")]
use crate::dfu::{BLOCK_SIZE_DFU, DFU_CHANNEL, DfuCmd};
#[cfg(feature = "_dfu")]
use crate::event::{DfuStatusEvent, publish_event};
#[cfg(feature = "steno")]
use crate::hid::StenoReport;
use crate::hid::{
    CompositeReport, CompositeReportType, HidError, HidWriterTrait, KeyboardReport, Report, run_led_reader,
};
use crate::light::UsbLedReader;
use crate::state::{current_usb_state, set_usb_state};

// The Rynk vendor interface serves the keyboard's Rynk session and the dongle's router.
#[cfg(any(feature = "rynk", all(feature = "dongle", not(feature = "vial"))))]
pub(crate) mod rynk;
#[cfg(feature = "vial")]
pub(crate) mod vial;

// A build has at most one host interface — Vial's HID report pair or the Rynk
// vendor bulk pair, and the protocols are mutually exclusive. A dongle relays
// its keyboard's protocol: Rynk unless `vial` says otherwise. The two modules
// expose the same names, so the rest of the file only talks to `host_usb`.
#[cfg(any(feature = "rynk", all(feature = "dongle", not(feature = "vial"))))]
use rynk as host_usb;
#[cfg(feature = "vial")]
use vial as host_usb;

pub(crate) static USB_REMOTE_WAKEUP: Signal<RawMutex, ()> = Signal::new();

/// Serves one framed session over the USB byte stream: the keyboard's Vial or
/// Rynk service, the dongle's router, or `()` in a build that serves none.
/// Which one is a type parameter of [`UsbTransport`], so only the attached one
/// reaches the image.
pub(crate) trait HostSession {
    async fn serve<R: embedded_io_async::Read, W: embedded_io_async::Write>(&self, rx: &mut R, tx: &mut W);
}

impl HostSession for () {
    async fn serve<R: embedded_io_async::Read, W: embedded_io_async::Write>(&self, _rx: &mut R, _tx: &mut W) {
        core::future::pending().await
    }
}

/// Borrowed view over the USB HID IN endpoints used by the report writer task.
///
/// `UsbTransport` owns the USB device, readers, writers, host interface, and
/// optional logger; `run` borrows those fields separately so they can run
/// concurrently without moving the whole transport into one task.
pub(crate) struct UsbKeyboardWriter<'a, 'd, D: Driver<'d>> {
    pub(crate) keyboard_writer: &'a mut HidWriter<'d, D, 8>,
    pub(crate) other_writer: &'a mut HidWriter<'d, D, 9>,
    #[cfg(feature = "steno")]
    pub(crate) steno_writer: &'a mut HidWriter<'d, D, 9>,
}

impl<'d, D: Driver<'d>> UsbKeyboardWriter<'_, 'd, D> {
    pub(crate) async fn run_writer(&mut self) -> ! {
        loop {
            let report = USB_REPORT_CHANNEL.receive().await;

            // EndpointError::Disabled never fires on non-OTG STM32/GD32
            // peripherals during suspend, so signal wakeup proactively when a
            // USB report is pending and the bus is suspended.
            if current_usb_state() == UsbState::Suspended {
                USB_REMOTE_WAKEUP.signal(());
                continue;
            }

            if let Err(e) = self.write_report(&report).await {
                error!("Failed to send report: {:?}", e);

                // Belt-and-braces for OTG peripherals where Disabled is the
                // correct suspend indicator: signal wakeup, give the host a
                // moment, then retry the same report once.
                if let HidError::UsbEndpointError(EndpointError::Disabled) = e {
                    USB_REMOTE_WAKEUP.signal(());
                    embassy_time::Timer::after_millis(500).await;
                    if let Err(e) = self.write_report(&report).await {
                        error!("Failed to send report after wakeup: {:?}", e);
                    }
                }
            }
        }
    }

    async fn write_composite<R: AsInputReport>(
        &mut self,
        kind: CompositeReportType,
        report: &R,
    ) -> Result<usize, HidError> {
        let mut buf = [0u8; 9];
        buf[0] = kind as u8;
        let n = report
            .serialize(&mut buf[1..])
            .map_err(|_| HidError::ReportSerializeError)?;
        self.other_writer
            .write(&buf[0..n + 1])
            .await
            .map_err(HidError::UsbEndpointError)?;
        Ok(n)
    }
}

impl<'d, D: Driver<'d>> HidWriterTrait for UsbKeyboardWriter<'_, 'd, D> {
    type ReportType = Report;

    async fn write_report(&mut self, report: &Self::ReportType) -> Result<usize, HidError> {
        match report {
            Report::KeyboardReport(keyboard_report) => {
                let mut buf: [u8; 8] = [0; 8];
                let n: usize = keyboard_report
                    .serialize(&mut buf)
                    .map_err(|_| HidError::ReportSerializeError)?;
                self.keyboard_writer
                    .write(&buf[0..n])
                    .await
                    .map_err(HidError::UsbEndpointError)?;
                Ok(n)
            }
            Report::MouseReport(r) => self.write_composite(CompositeReportType::Mouse, r).await,
            Report::MediaKeyboardReport(r) => self.write_composite(CompositeReportType::Media, r).await,
            Report::SystemControlReport(r) => self.write_composite(CompositeReportType::System, r).await,
            #[cfg(feature = "steno")]
            Report::StenoReport(steno_report) => {
                let mut buf: [u8; 9] = [0; 9];
                let n = steno_report
                    .serialize(&mut buf)
                    .map_err(|_| HidError::ReportSerializeError)?;

                // Cap on how long a steno report write is allowed to block. The host only
                // drains the steno IN endpoint while Plover is running; without this cap the
                // writer task stalls indefinitely (and starves keyboard reports) whenever
                // Plover is absent.
                match embassy_time::with_timeout(
                    embassy_time::Duration::from_millis(5),
                    self.steno_writer.write(&buf[0..n]),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => return Err(HidError::UsbEndpointError(e)),
                    Err(_) => {} // Plover not reading; drop this report and continue
                }
                Ok(n)
            }
        }
    }
}

/// Extra interfaces (usb_log, steno, dfu, rynk) overflow the 128-byte buffer.
const DEFAULT_CONFIG_DESC_SIZE: usize = if cfg!(any(
    feature = "usb_log",
    feature = "steno",
    feature = "_dfu",
    feature = "rynk",
    all(feature = "dongle", not(feature = "vial"))
)) {
    256
} else {
    128
};

fn default_config_descriptor() -> &'static mut [u8] {
    static CONFIG_DESC: StaticCell<[u8; DEFAULT_CONFIG_DESC_SIZE]> = StaticCell::new();
    &mut CONFIG_DESC.init([0; DEFAULT_CONFIG_DESC_SIZE])[..]
}

pub(crate) fn new_usb_builder<'d, D: Driver<'d>>(
    driver: D,
    keyboard_config: DeviceConfig<'d>,
    config_descriptor: &'d mut [u8],
) -> Builder<'d, D> {
    let mut usb_config = embassy_usb::Config::new(keyboard_config.vid, keyboard_config.pid);
    usb_config.manufacturer = Some(keyboard_config.manufacturer);
    usb_config.product = Some(keyboard_config.product_name);
    // Informational tag (visible in `lsusb` & co); host discovery keys on the
    // Rynk vendor interface triple, not the serial.
    #[cfg(any(feature = "rynk", all(feature = "dongle", not(feature = "vial"))))]
    let serial_number = {
        static SERIAL: StaticCell<heapless::String<64>> = StaticCell::new();
        let s = SERIAL.init(heapless::String::new());
        let _ = s.push_str(rmk_types::protocol::rynk::RYNK_MAGIC);
        let _ = s.push_str(keyboard_config.serial_number);
        s.as_str()
    };
    #[cfg(not(any(feature = "rynk", all(feature = "dongle", not(feature = "vial")))))]
    let serial_number = keyboard_config.serial_number;
    usb_config.serial_number = Some(serial_number);
    usb_config.max_power = 450;
    usb_config.supports_remote_wakeup = true;

    // Required for windows compatibility.
    usb_config.max_packet_size_0 = 64;
    usb_config.device_class = 0xEF;
    usb_config.device_sub_class = 0x02;
    usb_config.device_protocol = 0x01;
    usb_config.composite_with_iads = true;

    // Control buffer must be large enough for the largest DFU transfer block.
    #[cfg(feature = "_dfu")]
    const CONTROL_BUF_SIZE: usize = crate::dfu::BLOCK_SIZE_DFU;
    #[cfg(not(feature = "_dfu"))]
    const CONTROL_BUF_SIZE: usize = DEFAULT_CONFIG_DESC_SIZE;

    // The rynk MS OS 2.0 descriptor set (WinUSB binding) takes ~178 bytes, and
    // its BOS platform capability another 28 on top of the 5-byte BOS header.
    const RYNK_INTERFACE: bool = cfg!(any(feature = "rynk", all(feature = "dongle", not(feature = "vial"))));
    const BOS_BUF_SIZE: usize = if RYNK_INTERFACE { 64 } else { 16 };
    const MSOS_BUF_SIZE: usize = if RYNK_INTERFACE { 256 } else { 16 };

    static BOS_DESC: StaticCell<[u8; BOS_BUF_SIZE]> = StaticCell::new();
    static MSOS_DESC: StaticCell<[u8; MSOS_BUF_SIZE]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; CONTROL_BUF_SIZE]> = StaticCell::new();

    let mut builder = Builder::new(
        driver,
        usb_config,
        config_descriptor,
        &mut BOS_DESC.init([0; BOS_BUF_SIZE])[..],
        &mut MSOS_DESC.init([0; MSOS_BUF_SIZE])[..],
        &mut CONTROL_BUF.init([0; CONTROL_BUF_SIZE])[..],
    );

    static device_handler: StaticCell<UsbDeviceHandler> = StaticCell::new();
    builder.handler(device_handler.init(UsbDeviceHandler::new()));

    builder
}

/// Synchronous DFU handler for every alternate setting (alt 0 = central,
/// alt 1..N = split peripherals).
///
/// Runs inside the USB interrupt. It never touches flash: every download
/// `start`/`write`/`finish`/`system_reset` is forwarded to the async
/// [`FlashDfuHandler`](crate::dfu::FlashDfuHandler) updater task through
/// the command channel ([`DFU_CHANNEL`](crate::dfu::DFU_CHANNEL)). The DFU
/// lock gate (if enabled) is checked here so every DFU start path shares
/// one place.
#[cfg(feature = "_dfu")]
struct ProxyUsbDfuHandler {
    target: crate::dfu::DfuTarget,
    /// Running byte offset — each `Write` advances by the block size.
    written: u32,
}

#[cfg(feature = "_dfu")]
impl ProxyUsbDfuHandler {
    fn signal_peripheral(&self) {
        #[cfg(feature = "dfu_split")]
        if let crate::dfu::DfuTarget::Peripheral(id) = self.target {
            crate::dfu::DFU_PERIPH_SIGNALS[id as usize].signal(());
        }
    }
}

#[cfg(feature = "_dfu")]
impl dfu_mode::Handler for ProxyUsbDfuHandler {
    fn start(&mut self) -> Result<(), Status> {
        crate::dfu::dfu_lock_check()?;
        self.written = 0;
        info!("dfu: DFU download started ({:?})", self.target);
        DFU_CHANNEL
            .try_send(DfuCmd::Start(self.target))
            .map_err(|_| Status::ErrUnknown)?;
        #[cfg(feature = "dfu_lock")]
        DFU_STARTED.store(true, Ordering::Release);
        publish_event(DfuStatusEvent::new(DfuStatus::Started));
        self.signal_peripheral();
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
        self.written += data.len() as u32;
        DFU_CHANNEL
            .try_send(DfuCmd::Write(self.target, offset, buf))
            .map_err(|_| Status::ErrUnknown)?;
        publish_event(DfuStatusEvent::new(DfuStatus::Downloading));
        self.signal_peripheral();
        Ok(())
    }

    fn finish(&mut self) -> Result<(), Status> {
        if DFU_WRITE_FAILED.load(Ordering::Acquire) {
            DFU_WRITE_FAILED.store(false, Ordering::Release);
            return Err(Status::ErrWrite);
        }
        if DFU_CHANNEL.try_send(DfuCmd::Finish(self.target)).is_err() {
            error!("dfu: DFU command queue full at finish");
            publish_event(DfuStatusEvent::new(DfuStatus::Error));
            return Err(Status::ErrUnknown);
        }
        self.signal_peripheral();
        publish_event(DfuStatusEvent::new(DfuStatus::Finished));
        info!("dfu: DFU download complete");
        Ok(())
    }

    fn system_reset(&mut self) {
        if DFU_CHANNEL.try_send(DfuCmd::SystemReset(self.target)).is_err() {
            error!("dfu: DFU command queue full at system_reset");
            publish_event(DfuStatusEvent::new(DfuStatus::Error));
            return;
        }
        self.signal_peripheral();
    }
}

/// Owner of every DFU alternate setting registered on a single USB interface.
///
/// Alt 0 is the device's own DFU download (forwarded by [`ProxyUsbDfuHandler`]
/// with `DfuTarget::Central` to the async updater); alt 1..N are split
/// peripheral slots (requires `dfu_split`), forwarded with
/// `DfuTarget::Peripheral(n)`. Routes by the current alternate setting and
/// injects adaptive host-side flow control (`dfuDNBUSY`) while the command
/// queue is non-empty.
#[cfg(feature = "_dfu")]
struct UsbDfuIface {
    handlers: [Option<DfuState<ProxyUsbDfuHandler>>; MAX_DFU_ALTS],
    current_alt: u8,
}

#[cfg(feature = "_dfu")]
impl Handler for UsbDfuIface {
    fn set_alternate_setting(&mut self, _iface: InterfaceNumber, alternate_setting: u8) {
        self.current_alt = alternate_setting.min(self.handlers.len() as u8 - 1);
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

        if !DFU_CHANNEL.is_empty() && req.request == DFU_GETSTATUS {
            // Short-circuit: return dfuDNBUSY directly without
            // advancing the DfuState machine. The state stays in
            // DlSync so the next real GETSTATUS (after the queue
            // drains) correctly transitions to Download.
            //
            // GETSTATUS response (DFU 1.1, Table A.3):
            let resp: [u8; 6] = [
                0x00, // bmAttributes
                0x0A, 0x00, 0x00, // bwPollTimeout = 10 ms (3 bytes LE)
                4,    // bState = DlSync
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
#[cfg(feature = "_dfu")]
struct DfuStringProvider {
    string_idx: StringIndex,
    string_val: &'static str,
}

#[cfg(feature = "_dfu")]
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
#[cfg(feature = "_dfu")]
fn register_dfu_iface<D: Driver<'static>>(
    builder: &mut Builder<'static, D>,
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
                        target: crate::dfu::DfuTarget::Peripheral(id as u8),
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

/// USB transport. Owns the embassy-usb device + every HID reader/writer
/// pair and runs them concurrently for the lifetime of the program.
///
/// `S` is whatever serves the host interface in this binary — a keyboard's
/// Vial or Rynk service, a dongle's `DongleRouter`, or `()` for a build that
/// serves none. Picking it at compile time keeps the others out of the image.
pub struct UsbTransport<'a, D: Driver<'static>, S = ()> {
    device: UsbDevice<'static, D>,
    keyboard_reader: HidReader<'static, D, 1>,
    keyboard_writer: HidWriter<'static, D, 8>,
    other_writer: HidWriter<'static, D, 9>,
    #[cfg(feature = "steno")]
    steno_writer: HidWriter<'static, D, 9>,
    /// Taken by `run`: the logger future consumes the CDC class.
    #[cfg(feature = "usb_log")]
    logger: Option<embassy_usb::class::cdc_acm::CdcAcmClass<'static, D>>,
    #[cfg(any(feature = "host", feature = "dongle"))]
    host_reader: host_usb::HostUsbReader<D>,
    #[cfg(any(feature = "host", feature = "dongle"))]
    host_writer: host_usb::HostUsbWriter<D>,
    /// Serves the host interface; `&()` until a binary attaches its own.
    session: &'a S,
}

impl<'a, D: Driver<'static>> UsbTransport<'a, D> {
    pub fn new(
        driver: D,
        device_config: DeviceConfig<'static>,
        #[cfg(feature = "dfu_split")] num_peripherals: usize,
    ) -> Self {
        UsbTransportBuilder::new(
            driver,
            device_config,
            default_config_descriptor(),
            #[cfg(feature = "dfu_split")]
            num_peripherals,
        )
        .build()
    }

    /// Start a USB stack the caller finishes, for binaries serving USB classes of
    /// their own alongside the keyboard.
    ///
    /// ```rust,ignore
    /// let mut builder = UsbTransport::builder(driver, device_config);
    /// let mut cdc = CdcAcmClass::new(builder.usb_builder(), CDC_STATE.init(State::new()), 64);
    /// let mut usb_transport = builder.build().with_host_service(&host_service);
    /// ```
    pub fn builder(driver: D, device_config: DeviceConfig<'static>) -> UsbTransportBuilder<D> {
        // A CDC ACM function costs ~66 descriptor bytes, an extra HID interface ~40.
        const SIZE: usize = DEFAULT_CONFIG_DESC_SIZE + 256;
        static CONFIG_DESC: StaticCell<[u8; SIZE]> = StaticCell::new();
        UsbTransportBuilder::new(
            driver,
            device_config,
            &mut CONFIG_DESC.init([0; SIZE])[..],
            #[cfg(feature = "dfu_split")]
            crate::SPLIT_PERIPHERALS_NUM,
        )
    }
}

/// A [`UsbTransport`] mid-construction. See [`UsbTransport::builder`].
pub struct UsbTransportBuilder<D: Driver<'static>> {
    builder: Builder<'static, D>,
    keyboard_rw: HidReaderWriter<'static, D, 1, 8>,
    other_writer: HidWriter<'static, D, 9>,
    #[cfg(feature = "steno")]
    steno_writer: HidWriter<'static, D, 9>,
    #[cfg(feature = "usb_log")]
    logger: CdcAcmClass<'static, D>,
    #[cfg(any(feature = "host", feature = "dongle"))]
    host_reader: host_usb::HostUsbReader<D>,
    #[cfg(any(feature = "host", feature = "dongle"))]
    host_writer: host_usb::HostUsbWriter<D>,
}

impl<D: Driver<'static>> UsbTransportBuilder<D> {
    // Without `always`, opt-level="z" moves the whole struct between the two: +300 bytes.
    #[inline(always)]
    fn new(
        driver: D,
        device_config: DeviceConfig<'static>,
        config_descriptor: &'static mut [u8],
        #[cfg(feature = "dfu_split")] num_peripherals: usize,
    ) -> Self {
        // nRF chips don't have a stable USB serial number unless one is derived
        // from the FICR. Override here so user code doesn't have to know.
        #[cfg(feature = "_nrf_ble")]
        let device_config = {
            let mut device_config = device_config;
            device_config.serial_number = crate::ble::nrf::get_serial_number();
            device_config
        };
        let mut builder: Builder<'static, D> = new_usb_builder(driver, device_config, config_descriptor);
        // Linux's usbhid driver auto-enables power/wakeup when it probes a
        // boot-protocol keyboard, so advertise Boot/Keyboard on the primary
        // HID interface.
        let keyboard_rw = add_usb_reader_writer!(
            &mut builder,
            KeyboardReport,
            1,
            8,
            8,
            ::embassy_usb::class::hid::HidSubclass::Boot,
            ::embassy_usb::class::hid::HidBootProtocol::Keyboard
        );
        let other_writer = add_usb_writer!(&mut builder, CompositeReport, 9, 16);
        #[cfg(feature = "steno")]
        let steno_writer = add_usb_writer!(&mut builder, StenoReport, 9, 16);
        #[cfg(feature = "usb_log")]
        let logger = add_usb_logger!(&mut builder);

        #[cfg(feature = "_dfu")]
        register_dfu_iface(
            &mut builder,
            device_config.product_name,
            #[cfg(feature = "dfu_split")]
            num_peripherals,
        );

        #[cfg(any(feature = "host", feature = "dongle"))]
        let (host_reader, host_writer) = host_usb::build_host_usb(&mut builder);

        Self {
            builder,
            keyboard_rw,
            other_writer,
            #[cfg(feature = "steno")]
            steno_writer,
            #[cfg(feature = "usb_log")]
            logger,
            #[cfg(any(feature = "host", feature = "dongle"))]
            host_reader,
            #[cfg(any(feature = "host", feature = "dongle"))]
            host_writer,
        }
    }

    /// RMK's interfaces are already registered, so the keyboard keeps interface 0.
    pub fn usb_builder(&mut self) -> &mut Builder<'static, D> {
        &mut self.builder
    }

    #[inline(always)]
    pub fn build<'a>(self) -> UsbTransport<'a, D> {
        let (keyboard_reader, keyboard_writer) = self.keyboard_rw.split();

        UsbTransport {
            device: self.builder.build(),
            keyboard_reader,
            keyboard_writer,
            other_writer: self.other_writer,
            #[cfg(feature = "steno")]
            steno_writer: self.steno_writer,
            #[cfg(feature = "usb_log")]
            logger: Some(self.logger),
            #[cfg(any(feature = "host", feature = "dongle"))]
            host_reader: self.host_reader,
            #[cfg(any(feature = "host", feature = "dongle"))]
            host_writer: self.host_writer,
            session: &(),
        }
    }
}

impl<'a, D: Driver<'static>, S> UsbTransport<'a, D, S> {
    /// Attach the host-protocol service (Vial or Rynk, picked by feature).
    #[cfg(feature = "host")]
    pub fn with_host_service(
        self,
        service: &'a crate::host::HostService<'a>,
    ) -> UsbTransport<'a, D, crate::host::HostService<'a>> {
        self.serving(service)
    }

    /// Attach the dongle's router — this is what makes a binary a dongle. The
    /// same router goes to [`crate::dongle::Dongle`], which relays through it.
    #[cfg(feature = "dongle")]
    pub fn with_dongle_router(
        self,
        router: &'a crate::dongle::DongleRouter,
    ) -> UsbTransport<'a, D, crate::dongle::DongleRouter> {
        self.serving(router)
    }

    /// Rebuild around the session that answers the host interface.
    fn serving<T>(self, session: &'a T) -> UsbTransport<'a, D, T> {
        UsbTransport {
            device: self.device,
            keyboard_reader: self.keyboard_reader,
            keyboard_writer: self.keyboard_writer,
            other_writer: self.other_writer,
            #[cfg(feature = "steno")]
            steno_writer: self.steno_writer,
            #[cfg(feature = "usb_log")]
            logger: self.logger,
            #[cfg(any(feature = "host", feature = "dongle"))]
            host_reader: self.host_reader,
            #[cfg(any(feature = "host", feature = "dongle"))]
            host_writer: self.host_writer,
            session,
        }
    }
}

impl<D: Driver<'static>, S: HostSession> Runnable for UsbTransport<'_, D, S> {
    async fn run(&mut self) -> ! {
        let Self {
            device,
            keyboard_reader,
            keyboard_writer,
            other_writer,
            #[cfg(feature = "steno")]
            steno_writer,
            #[cfg(feature = "usb_log")]
            logger,
            #[cfg(any(feature = "host", feature = "dongle"))]
            host_reader,
            #[cfg(any(feature = "host", feature = "dongle"))]
            host_writer,
            session,
        } = self;

        let usb_device_task = async {
            loop {
                device.run_until_suspend().await;
                match select(device.wait_resume(), USB_REMOTE_WAKEUP.wait()).await {
                    Either::First(_) => continue,
                    Either::Second(_) => {
                        info!("USB remote wakeup requested");
                        if let Err(e) = device.remote_wakeup().await {
                            warn!("Remote wakeup failed: {:?}", e);
                        }
                    }
                }
            }
        };

        let mut writer = UsbKeyboardWriter {
            keyboard_writer,
            other_writer,
            #[cfg(feature = "steno")]
            steno_writer,
        };
        let writer_task = writer.run_writer();

        let mut led_reader = UsbLedReader::new(keyboard_reader);
        let led_task = run_led_reader(&mut led_reader, ConnectionType::Usb);

        #[cfg(any(feature = "host", feature = "dongle"))]
        let host_task = host_usb::run_host_usb(host_reader, host_writer, *session);
        #[cfg(not(any(feature = "host", feature = "dongle")))]
        let host_task = {
            // No host interface was built, so the session is always `()`.
            let _ = session;
            core::future::pending::<()>()
        };

        #[cfg(feature = "usb_log")]
        let logger_task = run_usb_logger(logger.take().expect("UsbTransport::run called twice"));
        #[cfg(not(feature = "usb_log"))]
        let logger_task = core::future::pending::<()>();

        join5(usb_device_task, writer_task, led_task, host_task, logger_task).await;
        unreachable!("UsbTransport sub-tasks must run forever");
    }
}

#[cfg(feature = "usb_log")]
async fn run_usb_logger<D: Driver<'static>>(logger_class: CdcAcmClass<'static, D>) {
    // Add a usb logger with log filter set to `Trace` to catch all logs.
    // The log level itself is set via the `max_level_*` feature of the log crate.
    let logger_fut =
        ::embassy_usb_logger::with_custom_style!(1024, log::LevelFilter::Trace, logger_class, |record, writer| {
            use core::fmt::Write;
            let ms = embassy_time::Instant::now().as_millis();
            let _ = write!(writer, "[{:>8}ms {:5}] {}\r\n", ms, record.level(), record.args());
        });
    logger_fut.await;
}

#[cfg(any(feature = "usb_log", feature = "_dfu"))]
pub async fn run_peripheral_usb<D: Driver<'static>>(driver: D, config: DeviceConfig<'static>) {
    let mut builder = new_usb_builder(driver, config, default_config_descriptor());

    #[cfg(feature = "usb_log")]
    let logger_fut = run_usb_logger(add_usb_logger!(&mut builder));
    #[cfg(not(feature = "usb_log"))]
    let logger_fut = ::core::future::pending::<()>();

    #[cfg(feature = "_dfu")]
    register_dfu_iface(
        &mut builder,
        config.product_name,
        #[cfg(feature = "dfu_split")]
        0,
    );

    let mut usb_device = builder.build();

    ::embassy_futures::join::join(usb_device.run(), logger_fut).await;
}

#[cfg(feature = "usb_log")]
macro_rules! add_usb_logger {
    ($usb_builder:expr) => {{
        use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
        use static_cell::StaticCell;

        // The usb logger can be only initialized once, so just use a fixed name for the state
        static LOGGER_STATE: StaticCell<State> = StaticCell::new();
        let state = LOGGER_STATE.init(State::new());
        CdcAcmClass::new($usb_builder, state, embassy_usb_logger::MAX_PACKET_SIZE as u16)
    }};
}

/// Per-descriptor HID `(State, Config)` pair. `paste` generates the `static`s
/// from the descriptor name so each interface keeps its own State/Handler.
/// Size `$max_packet` to the actual report to conserve Packet Memory Area on tight parts.
macro_rules! usb_hid_state_and_config {
    ($descriptor:ty, $max_packet:expr, $subclass:expr, $protocol:expr) => {{
        use usbd_hid::descriptor::SerializedDescriptor;
        paste::paste! {
            static [<$descriptor:snake:upper _STATE>]: ::static_cell::StaticCell<::embassy_usb::class::hid::State> = ::static_cell::StaticCell::new();
            static [<$descriptor:snake:upper _HANDLER>]: ::static_cell::StaticCell<$crate::usb::UsbRequestHandler> = ::static_cell::StaticCell::new();
        }

        let state = paste::paste! { [<$descriptor:snake:upper _STATE>].init(::embassy_usb::class::hid::State::new()) };
        let request_handler = paste::paste! {
            [<$descriptor:snake:upper _HANDLER>].init($crate::usb::UsbRequestHandler {
                protocol: ::embassy_usb::class::hid::HidProtocolMode::Report,
            })
        };

        let hid_config = ::embassy_usb::class::hid::Config {
            report_descriptor: <$descriptor>::desc(),
            request_handler: Some(request_handler),
            poll_ms: 1,
            max_packet_size: $max_packet,
            hid_subclass: $subclass,
            hid_boot_protocol: $protocol,
        };
        (state, hid_config)
    }};
}

macro_rules! add_usb_writer {
    ($usb_builder:expr, $descriptor:ty, $n:expr, $max_packet:expr) => {{
        let (state, hid_config) = $crate::usb::usb_hid_state_and_config!(
            $descriptor,
            $max_packet,
            ::embassy_usb::class::hid::HidSubclass::No,
            ::embassy_usb::class::hid::HidBootProtocol::None
        );
        ::embassy_usb::class::hid::HidWriter::<_, $n>::new($usb_builder, state, hid_config)
    }};
}

macro_rules! add_usb_reader_writer {
    ($usb_builder:expr, $descriptor:ty, $read_n:expr, $write_n:expr, $max_packet:expr) => {
        $crate::usb::add_usb_reader_writer!(
            $usb_builder,
            $descriptor,
            $read_n,
            $write_n,
            $max_packet,
            ::embassy_usb::class::hid::HidSubclass::No,
            ::embassy_usb::class::hid::HidBootProtocol::None
        )
    };
    ($usb_builder:expr, $descriptor:ty, $read_n:expr, $write_n:expr, $max_packet:expr, $subclass:expr, $protocol:expr) => {{
        let (state, hid_config) =
            $crate::usb::usb_hid_state_and_config!($descriptor, $max_packet, $subclass, $protocol);
        ::embassy_usb::class::hid::HidReaderWriter::<_, $read_n, $write_n>::new($usb_builder, state, hid_config)
    }};
}

#[cfg(feature = "usb_log")]
pub(crate) use add_usb_logger;
pub(crate) use add_usb_reader_writer;
pub(crate) use add_usb_writer;
pub(crate) use usb_hid_state_and_config;

pub(crate) struct UsbRequestHandler {
    pub(crate) protocol: HidProtocolMode,
}

impl RequestHandler for UsbRequestHandler {
    fn set_report(&mut self, id: ReportId, data: &[u8]) -> OutResponse {
        info!("Set report for {:?}: {:?}", id, data);
        OutResponse::Accepted
    }

    fn get_protocol(&self) -> HidProtocolMode {
        self.protocol
    }

    fn set_protocol(&mut self, protocol: HidProtocolMode) -> OutResponse {
        // KeyboardReport is already the 8-byte boot keyboard layout, so the
        // mode only changes what GET_PROTOCOL answers.
        // TODO: Return to Report on a bus reset once embassy-usb tells the
        // request handler about it (embassy-rs/embassy#6891).
        self.protocol = protocol;
        OutResponse::Accepted
    }
}

pub(crate) struct UsbDeviceHandler {
    /// State to restore on resume. Only a Configured device is ever published as
    /// Suspended (see `suspended()`), so this always holds Configured while the
    /// device is suspended; kept as a snapshot rather than a hardcoded value so
    /// resume stays correct if another pre-suspend state becomes publishable.
    pre_suspend: UsbState,
}

impl UsbDeviceHandler {
    fn new() -> Self {
        UsbDeviceHandler {
            pre_suspend: UsbState::Disabled,
        }
    }
}

impl Handler for UsbDeviceHandler {
    fn enabled(&mut self, enabled: bool) {
        if enabled {
            info!("Device enabled");
            set_usb_state(UsbState::Enabled);
        } else {
            info!("Device disabled");
            set_usb_state(UsbState::Disabled);
        }
    }

    fn reset(&mut self) {
        info!("Bus reset, the Vbus current limit is 100mA");
    }

    fn addressed(&mut self, addr: u8) {
        info!("USB address set to: {}", addr);
    }

    fn configured(&mut self, configured: bool) {
        if configured {
            set_usb_state(UsbState::Configured);
            info!("Device configured, it may now draw up to the configured current from Vbus.")
        } else {
            set_usb_state(UsbState::Enabled);
            info!("Device is no longer configured, the Vbus current limit is 100mA.");
        }
    }

    fn suspended(&mut self, suspended: bool) {
        if suspended {
            // Only publish Suspended when the device was configured before the
            // suspend. `usb_ready()` deliberately treats Suspended as routable
            // (a suspended host must stay reachable for remote wakeup), but that
            // only holds for a device the host has actually enumerated. A
            // never-configured device also sees bus-idle suspends — a charge-only
            // cable or wall charger leaves D+/D- idle, which e.g. on nRF52840
            // raises SUSPEND ~3 ms after enable — and publishing Suspended there
            // would route reports to endpoints that were never configured,
            // silently dropping keystrokes that BLE could have delivered.
            let live = current_usb_state();
            if live == UsbState::Configured {
                self.pre_suspend = live;
                set_usb_state(UsbState::Suspended);
                info!(
                    "Device suspended, the Vbus current limit is 500µA (or 2.5mA for high-power devices with remote wakeup enabled)."
                );
            } else if live != UsbState::Suspended {
                info!("Bus suspended before enumeration (charger or charge-only cable?), USB stays inactive");
            }
        } else {
            // Only restore from Suspended; if we're somehow not in Suspended (out-of-order
            // callbacks), don't overwrite — `configured()`/`enabled()` will resync.
            if current_usb_state() == UsbState::Suspended {
                set_usb_state(self.pre_suspend);
            }
            info!(
                "Device resumed, the Vbus current limit is 500µA (or 2.5mA for high-power devices with remote wakeup enabled)."
            );
        }
    }

    fn remote_wakeup_enabled(&mut self, enabled: bool) {
        info!("Remote wakeup enabled state: {}", enabled);
    }
}

// These tests mutate the process-global CONNECTION_STATUS; cargo-nextest's
// per-test process isolation keeps them from racing each other (plain
// `cargo test` is rejected at startup by `test_support::require_nextest`).
#[cfg(test)]
mod tests {
    use embassy_usb::Handler;
    use embassy_usb::class::hid::RequestHandler;
    use rmk_types::connection::UsbState;

    use super::{HidProtocolMode, OutResponse, UsbDeviceHandler, UsbRequestHandler};
    use crate::state::{current_usb_state, set_usb_state};

    /// A BIOS or KVM switch selects the boot protocol before it will use the
    /// keyboard; rejecting the switch strands a host that trusts the boot
    /// subclass the interface advertises.
    #[test]
    fn the_keyboard_switches_protocol_and_reports_it() {
        let mut handler = UsbRequestHandler {
            protocol: HidProtocolMode::Report,
        };
        assert_eq!(handler.get_protocol(), HidProtocolMode::Report);

        assert_eq!(handler.set_protocol(HidProtocolMode::Boot), OutResponse::Accepted);
        assert_eq!(handler.get_protocol(), HidProtocolMode::Boot);

        assert_eq!(handler.set_protocol(HidProtocolMode::Report), OutResponse::Accepted);
        assert_eq!(handler.get_protocol(), HidProtocolMode::Report);
    }

    /// A charge-only cable / wall charger enables the device (VBUS present) but
    /// never enumerates it; the bus-idle suspend that follows must not publish
    /// Suspended, otherwise `usb_ready()` would route reports to endpoints that
    /// were never configured while a BLE host could have received them.
    #[test]
    fn suspend_without_enumeration_stays_enabled() {
        let mut handler = UsbDeviceHandler::new();
        handler.enabled(true);
        assert_eq!(current_usb_state(), UsbState::Enabled);

        handler.suspended(true);
        assert_eq!(current_usb_state(), UsbState::Enabled);

        // Spurious resume (bus activity without enumeration) changes nothing.
        handler.suspended(false);
        assert_eq!(current_usb_state(), UsbState::Enabled);

        // A host showing up later still enumerates normally.
        handler.configured(true);
        assert_eq!(current_usb_state(), UsbState::Configured);
    }

    /// A genuinely suspended (previously enumerated) host keeps the Suspended
    /// state so it stays routable for remote wakeup, and resume restores
    /// Configured.
    #[test]
    fn suspend_after_configured_publishes_suspended_and_resume_restores() {
        let mut handler = UsbDeviceHandler::new();
        handler.enabled(true);
        handler.configured(true);

        handler.suspended(true);
        assert_eq!(current_usb_state(), UsbState::Suspended);

        handler.suspended(false);
        assert_eq!(current_usb_state(), UsbState::Configured);
    }

    /// A stray duplicate `suspended(true)` while already Suspended must not
    /// clobber the pre-suspend snapshot that resume restores.
    #[test]
    fn duplicate_suspend_preserves_pre_suspend_state() {
        let mut handler = UsbDeviceHandler::new();
        handler.enabled(true);
        handler.configured(true);

        handler.suspended(true);
        handler.suspended(true);
        assert_eq!(current_usb_state(), UsbState::Suspended);

        handler.suspended(false);
        assert_eq!(current_usb_state(), UsbState::Configured);
    }

    /// Out-of-order resume while not suspended must not overwrite the live
    /// state.
    #[test]
    fn resume_without_suspend_is_a_no_op() {
        let mut handler = UsbDeviceHandler::new();
        set_usb_state(UsbState::Configured);

        handler.suspended(false);
        assert_eq!(current_usb_state(), UsbState::Configured);
    }
}
