#[cfg(all(feature = "dfu_split", any(feature = "dfu_rp", feature = "dfu_nrf")))]
use core::sync::atomic::Ordering;
#[cfg(feature = "dfu_lock")]
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "dfu_lock")]
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
#[cfg(feature = "dfu_lock")]
use embassy_sync::signal::Signal;
use embassy_usb::control::{InResponse, OutResponse, Request};
use embassy_usb::driver::Driver;
use embassy_usb::types::{InterfaceNumber, StringIndex};
use embassy_usb::{Builder, Handler};
#[cfg(any(feature = "dfu_rp", feature = "dfu_nrf"))]
use embassy_usb_dfu::{ResetImmediate, dfu::FirmwareHandler};
use static_cell::StaticCell;

#[cfg(feature = "dfu_lock")]
use crate::core_traits::Runnable;

// ---------------------------------------------------------------------------
// Chip modules
// ---------------------------------------------------------------------------

#[cfg(feature = "dfu_nrf")]
mod nrf;
#[cfg(feature = "dfu_rp")]
mod rp;

#[cfg(feature = "dfu_nrf")]
pub use self::nrf::{DFU_WRITE_SIZE, get_manager, init_flash, init_flash_from_linkerscript, mark_booted};
#[cfg(feature = "dfu_rp")]
pub use self::rp::{DFU_WRITE_SIZE, get_manager, init_flash, init_flash_from_linkerscript, mark_booted};

// ---------------------------------------------------------------------------
// Chip-specific type aliases
// ---------------------------------------------------------------------------

/// DFU transfer block size in bytes. Larger values speed up firmware
/// downloads. Must match the USB control buffer size used by the host.
pub const BLOCK_SIZE_DFU: usize = 512;

#[cfg(any(feature = "dfu_rp", feature = "dfu_nrf"))]
use embassy_embedded_hal::flash::partition::BlockingPartition;

#[cfg(feature = "dfu_nrf")]
use self::nrf::{MutexType, PartitionType};
#[cfg(feature = "dfu_rp")]
use self::rp::{MutexType, PartitionType};

// ---------------------------------------------------------------------------
// dfu_split sub-module (behind feature flag)
// ---------------------------------------------------------------------------

#[cfg(feature = "dfu_split")]
mod split;
#[cfg(feature = "dfu_split")]
use self::split::PassthroughDfuHandler;
#[cfg(feature = "dfu_split")]
pub(crate) use self::split::{
    PASSTHROUGH_SIGNAL, PASSTHROUGH_TARGET, PassthroughCommand, passthrough_done_if_empty, passthrough_pending,
    passthrough_take_command,
};
#[cfg(feature = "dfu_split")]
pub use self::split::{
    SplitDfuHandler, get_firmware_update_data, read_embedded_firmware_hash, set_firmware_update_data,
};

// ---------------------------------------------------------------------------
// DfuFlashManager — shared by RP2040 and nRF
// ---------------------------------------------------------------------------

#[cfg(any(feature = "dfu_rp", feature = "dfu_nrf"))]
pub struct DfuFlashManager {
    flash_mutex: &'static MutexType,
    state_offset: u32,
    state_size: u32,
    dfu_offset: u32,
    dfu_size: u32,
    storage_offset: u32,
    storage_size: u32,
}

#[cfg(any(feature = "dfu_rp", feature = "dfu_nrf"))]
impl DfuFlashManager {
    pub(super) fn new(
        flash_mutex: &'static MutexType,
        storage_offset: u32,
        storage_size: u32,
        state_offset: u32,
        state_size: u32,
        dfu_offset: u32,
        dfu_size: u32,
    ) -> Self {
        Self {
            flash_mutex,
            state_offset,
            state_size,
            dfu_offset,
            dfu_size,
            storage_offset,
            storage_size,
        }
    }

    pub fn state_partition(&self) -> PartitionType {
        BlockingPartition::new(self.flash_mutex, self.state_offset, self.state_size)
    }

    pub fn dfu_partition(&self) -> PartitionType {
        BlockingPartition::new(self.flash_mutex, self.dfu_offset, self.dfu_size)
    }

    pub fn storage_partition(&self) -> PartitionType {
        BlockingPartition::new(self.flash_mutex, self.storage_offset, self.storage_size)
    }
}

// ---------------------------------------------------------------------------
// DfuStringProvider
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// DFU lock state
// ---------------------------------------------------------------------------

#[cfg(feature = "dfu_lock")]
static DFU_LOCKED: AtomicBool = AtomicBool::new(true);
#[cfg(feature = "dfu_lock")]
static DFU_STARTED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "dfu_lock")]
static DFU_UNLOCK_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

#[cfg(feature = "dfu_lock")]
pub fn is_dfu_unlocked() -> bool {
    !DFU_LOCKED.load(Ordering::Acquire)
}

// ---------------------------------------------------------------------------
// RmkDfuHandler
// ---------------------------------------------------------------------------

#[cfg(any(feature = "dfu_rp", feature = "dfu_nrf"))]
use embassy_usb::class::dfu::dfu_mode::DfuState;
#[cfg(feature = "dfu")]
use embassy_usb::class::dfu::{
    consts::Status,
    dfu_mode::{self},
};
#[cfg(any(feature = "dfu", feature = "dfu_lock"))]
use rmk_types::dfu::DfuStatus;

/// DFU handler wrapper that blinks an LED during transfer and checks the
/// DFU lock (if `dfu_lock` feature is enabled).
#[cfg(any(feature = "dfu", feature = "dfu_lock"))]
use crate::event::publish_event;

#[cfg(feature = "dfu")]
struct RmkDfuHandler<H> {
    inner: H,
    target_id: Option<usize>,
}

#[cfg(feature = "dfu")]
impl<H: dfu_mode::Handler> dfu_mode::Handler for RmkDfuHandler<H> {
    fn start(&mut self) -> Result<(), Status> {
        #[cfg(feature = "dfu_lock")]
        if !is_dfu_unlocked() {
            DFU_UNLOCK_SIGNAL.signal(());
            info!("dfu_lock: DFU download rejected — keys not unlocked");
            return Err(Status::ErrVendor);
        }
        #[cfg(feature = "dfu_lock")]
        DFU_STARTED.store(true, Ordering::Release);
        match self.target_id {
            Some(id) => info!("dfu: DFU download started (passthrough peripheral {})", id),
            None => info!("dfu: DFU download started (central)"),
        }
        publish_event(crate::event::DfuStatusEvent::new(DfuStatus::Started));
        self.inner.start()
    }

    fn write(&mut self, data: &[u8]) -> Result<(), Status> {
        publish_event(crate::event::DfuStatusEvent::new(DfuStatus::Downloading));
        self.inner.write(data)
    }

    fn finish(&mut self) -> Result<(), Status> {
        let res = self.inner.finish();
        publish_event(crate::event::DfuStatusEvent::new(if res.is_ok() {
            DfuStatus::Finished
        } else {
            DfuStatus::Error
        }));
        res
    }

    fn system_reset(&mut self) {
        self.inner.system_reset()
    }
}

// ---------------------------------------------------------------------------
// RmkDfuInterface — single USB handler for all DFU alt settings
// ---------------------------------------------------------------------------

/// Max passthrough alt settings supported on a single DFU interface.
#[cfg(feature = "dfu_split")]
const MAX_PASSTHROUGH_ALTS: usize = 4;

/// Single USB `Handler` that owns all DFU alternate settings.
///
/// Alt 0 is always the central's own DFU flash.  Alt 1..N are passthrough
/// slots for split peripherals (requires `dfu_split`).
#[cfg(any(feature = "dfu_rp", feature = "dfu_nrf"))]
struct RmkDfuInterface {
    central:
        DfuState<RmkDfuHandler<FirmwareHandler<'static, PartitionType, PartitionType, ResetImmediate, BLOCK_SIZE_DFU>>>,
    #[cfg(feature = "dfu_split")]
    passthrough: [Option<DfuState<RmkDfuHandler<PassthroughDfuHandler>>>; MAX_PASSTHROUGH_ALTS],
    #[cfg(feature = "dfu_split")]
    num_passthrough: usize,
    current_alt: u8,
}

#[cfg(any(feature = "dfu_rp", feature = "dfu_nrf"))]
impl Handler for RmkDfuInterface {
    fn set_alternate_setting(&mut self, _iface: InterfaceNumber, alternate_setting: u8) {
        self.current_alt = alternate_setting;
    }

    fn control_out(&mut self, req: Request, data: &[u8]) -> Option<OutResponse> {
        match self.current_alt {
            0 => self.central.control_out(req, data),
            #[cfg(feature = "dfu_split")]
            n => {
                let idx = (n as usize).saturating_sub(1);
                self.passthrough_slots(idx).and_then(|s| s.control_out(req, data))
            }
            #[cfg(not(feature = "dfu_split"))]
            _ => None,
        }
    }

    fn control_in<'a>(&'a mut self, req: Request, buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        match self.current_alt {
            0 => self.central.control_in(req, buf),
            #[cfg(feature = "dfu_split")]
            n => {
                let idx = (n as usize).saturating_sub(1);
                let buf_ptr = buf.as_mut_ptr();
                let resp = self.passthrough_slots(idx).and_then(|s| s.control_in(req, buf));
                // ── Flow control: dfuDNBUSY override ──────────────────────
                // Byte 4 of the GETSTATUS response (6 bytes) is the `state`
                // field.  While PASSTHROUGH_TARGET is set (!= usize::MAX) we override
                // it with 4 (= dfuDNBUSY).  dfu-util then waits 50 ms and polls again
                // — adaptive back-pressure without a fixed timeout.
                // Uses a volatile store because `buf` is still borrowed by
                // `resp` at this point.
                if resp.is_some() && PASSTHROUGH_TARGET.load(Ordering::Acquire) != usize::MAX {
                    unsafe {
                        core::ptr::write_volatile(buf_ptr.add(4), 4u8);
                    }
                }
                resp
            }
            #[cfg(not(feature = "dfu_split"))]
            _ => None,
        }
    }
}

#[cfg(feature = "dfu_split")]
impl RmkDfuInterface {
    fn passthrough_slots(&mut self, idx: usize) -> Option<&mut DfuState<RmkDfuHandler<PassthroughDfuHandler>>> {
        self.passthrough.get_mut(idx)?.as_mut()
    }
}

#[cfg(any(feature = "dfu_rp", feature = "dfu_nrf"))]
static RMK_DFU_INTERFACE: StaticCell<RmkDfuInterface> = StaticCell::new();

// ---------------------------------------------------------------------------
// register_dfu_interface — register DFU interface with central + passthrough alts
// ---------------------------------------------------------------------------

/// Register a DFU interface on the USB builder.
///
/// Alt 0 is always the central's own DFU flash.
/// Alt 1..N are passthrough slots for split peripherals (requires `dfu_split`).
#[cfg(any(feature = "dfu_rp", feature = "dfu_nrf"))]
pub fn register_dfu_interface<D: Driver<'static>>(
    builder: &mut Builder<'static, D>,
    mgr: &'static DfuFlashManager,
    product_name: &'static str,
    #[cfg(feature = "dfu_split")] num_peripherals: usize,
) {
    use embassy_boot::{BlockingFirmwareUpdater, FirmwareUpdaterConfig};
    use embassy_usb::class::dfu::consts::DfuAttributes;

    let dfu_part = mgr.dfu_partition();
    let state_part = mgr.state_partition();
    let config = FirmwareUpdaterConfig {
        dfu: dfu_part,
        state: state_part,
    };
    static ALIGNED: StaticCell<[u8; DFU_WRITE_SIZE]> = StaticCell::new();
    let aligned: &'static mut [u8] = ALIGNED.init([0; DFU_WRITE_SIZE]);
    let updater = BlockingFirmwareUpdater::new(config, aligned);

    // Alt 0: Central flash
    let central_attrs = DfuAttributes::CAN_DOWNLOAD | DfuAttributes::WILL_DETACH;
    // embassy-usb's DfuAttributes is Copy only under defmt, so read the bits before the move
    let central_attrs_bits = central_attrs.bits();
    let central_handler = RmkDfuHandler {
        inner: FirmwareHandler::new(updater, ResetImmediate),
        target_id: None,
    };
    let central_state = DfuState::new(central_handler, central_attrs);

    // Alt 1..N: Passthrough
    #[cfg(feature = "dfu_split")]
    let passthrough_count = num_peripherals.min(MAX_PASSTHROUGH_ALTS);
    #[cfg(feature = "dfu_split")]
    let passthrough = {
        let mut arr: [Option<DfuState<RmkDfuHandler<PassthroughDfuHandler>>>; MAX_PASSTHROUGH_ALTS] =
            Default::default();
        for id in 0..passthrough_count {
            let state = DfuState::new(
                RmkDfuHandler {
                    inner: PassthroughDfuHandler {
                        target_id: id,
                        written: 0,
                    },
                    target_id: Some(id),
                },
                DfuAttributes::CAN_DOWNLOAD,
            );
            arr[id] = Some(state);
        }
        arr
    };

    let string_idx = builder.string();

    let mut func = builder.function(0x00, 0x00, 0x00);
    let mut iface = func.interface();
    let mut alt = iface.alt_setting(0xFE, 0x01, 0x02, Some(string_idx));
    alt.descriptor(
        0x21,
        &[
            central_attrs_bits,
            0xc4,
            0x09,
            (BLOCK_SIZE_DFU & 0xff) as u8,
            ((BLOCK_SIZE_DFU >> 8) & 0xff) as u8,
            0x10,
            0x01,
        ],
    );

    #[cfg(feature = "dfu_split")]
    for _ in 0..passthrough_count {
        let mut alt = iface.alt_setting(0xFE, 0x01, 0x02, Some(string_idx));
        alt.descriptor(
            0x21,
            &[
                DfuAttributes::CAN_DOWNLOAD.bits(),
                0xc4,
                0x09,
                (BLOCK_SIZE_DFU & 0xff) as u8,
                ((BLOCK_SIZE_DFU >> 8) & 0xff) as u8,
                0x10,
                0x01,
            ],
        );
    }
    drop(func);

    let iface_ref = RMK_DFU_INTERFACE.init(RmkDfuInterface {
        central: central_state,
        #[cfg(feature = "dfu_split")]
        passthrough,
        #[cfg(feature = "dfu_split")]
        num_passthrough: passthrough_count,
        current_alt: 0,
    });
    builder.handler(iface_ref);

    static STRING_PROVIDER: StaticCell<DfuStringProvider> = StaticCell::new();
    let string_provider = STRING_PROVIDER.init(DfuStringProvider {
        string_idx,
        string_val: product_name,
    });
    builder.handler(string_provider);
}

// ---------------------------------------------------------------------------
// dfu_lock
// ---------------------------------------------------------------------------

/// DfuLock state machine that checks a physical key combination to unlock DFU.
#[cfg(feature = "dfu_lock")]
pub struct DfuLock<'a> {
    unlocked: AtomicBool,
    unlock_keys: &'a [(u8, u8)],
    keymap: &'a crate::keymap::KeyMap<'a>,
}

#[cfg(feature = "dfu_lock")]
impl<'a> DfuLock<'a> {
    pub fn new(unlock_keys: &'a [(u8, u8)], keymap: &'a crate::keymap::KeyMap<'a>) -> Self {
        Self {
            unlocked: AtomicBool::new(false),
            unlock_keys,
            keymap,
        }
    }

    pub(crate) async fn process_unlock(&self) {
        DFU_UNLOCK_SIGNAL.wait().await;

        info!("dfu_lock: DFU activity detected, unlock window open for 10 s");
        info!("dfu_lock: waiting for unlock keys");
        publish_event(crate::event::DfuStatusEvent::new(DfuStatus::LockWaiting));
        let deadline = embassy_time::Instant::now() + embassy_time::Duration::from_secs(10);
        loop {
            let all_pressed = self
                .unlock_keys
                .iter()
                .all(|(row, col)| self.keymap.read_matrix_key(*row, *col));
            if all_pressed {
                self.unlocked.store(true, Ordering::Release);
                DFU_LOCKED.store(false, Ordering::Release);
                info!("dfu_lock: unlock keys pressed, DFU unlocked for 10 s");
                publish_event(crate::event::DfuStatusEvent::new(DfuStatus::LockUnlocked));
                break;
            }
            if embassy_time::Instant::now() >= deadline {
                info!("dfu_lock: unlock window expired (10 s timeout)");
                DFU_LOCKED.store(true, Ordering::Release);
                publish_event(crate::event::DfuStatusEvent::new(DfuStatus::Idle));
                return;
            }
            embassy_time::Timer::after_millis(50).await;
        }

        info!("dfu_lock: unlocked, waiting for DFU download");
        let deadline = embassy_time::Instant::now() + embassy_time::Duration::from_secs(10);
        loop {
            if DFU_STARTED.load(Ordering::Acquire) {
                info!("dfu_lock: DFU download started, staying unlocked");
                break;
            }
            if embassy_time::Instant::now() >= deadline {
                info!("dfu_lock: unlock expired (10 s timeout)");
                DFU_LOCKED.store(true, Ordering::Release);
                self.unlocked.store(false, Ordering::Release);
                publish_event(crate::event::DfuStatusEvent::new(DfuStatus::Idle));
                break;
            }
            embassy_time::Timer::after_millis(200).await;
        }
    }
}

#[cfg(feature = "dfu_lock")]
impl<'a> Runnable for DfuLock<'a> {
    async fn run(&mut self) -> ! {
        loop {
            self.process_unlock().await;
        }
    }
}
