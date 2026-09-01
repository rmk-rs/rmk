use core::cell::RefCell;

use embassy_boot::BlockingFirmwareState;
use embassy_embedded_hal::flash::partition::BlockingPartition;
use embassy_rp::Peri;
use embassy_rp::flash::{Blocking, Flash};
use embassy_rp::peripherals::FLASH;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::once_lock::OnceLock;
use static_cell::StaticCell;

use super::DfuFlashManager;

/// Total flash size passed to the embassy-rp Flash const generic.
///
/// Set to 16 MB (the maximum common RP2040 flash size) so that the same
/// binary works on boards with 2, 4, 8 or 16 MB flash.  `new_blocking()`
/// ignores this value at runtime — it is only used for software bounds
/// checking inside embassy-rp.  Because all flash access goes through
/// `BlockingPartition` (which has its own partition-sized bounds checks),
/// overshooting the const generic is safe.
pub const FLASH_SIZE: usize = 16 * 1024 * 1024;

/// Flash write granularity — 1 for RP2040.
pub const DFU_WRITE_SIZE: usize = 1;

pub(super) type FlashType = Flash<'static, FLASH, Blocking, FLASH_SIZE>;
pub(super) type MutexType = Mutex<CriticalSectionRawMutex, RefCell<FlashType>>;
pub(super) type PartitionType = BlockingPartition<'static, CriticalSectionRawMutex, FlashType>;

static FLASH_CELL: StaticCell<MutexType> = StaticCell::new();
static MANAGER: OnceLock<DfuFlashManager> = OnceLock::new();

/// Initialize the blocking flash, create the DFU manager and store it globally.
pub fn init_flash(
    flash_peri: Peri<'static, FLASH>,
    storage_offset: u32,
    storage_size: u32,
    state_offset: u32,
    state_size: u32,
    dfu_offset: u32,
    dfu_size: u32,
) -> PartitionType {
    let raw_flash = Flash::<_, Blocking, FLASH_SIZE>::new_blocking(flash_peri);

    let flash_mutex: &'static MutexType = FLASH_CELL.init(Mutex::new(RefCell::new(raw_flash)));
    let mgr = DfuFlashManager::new(
        flash_mutex,
        storage_offset,
        storage_size,
        state_offset,
        state_size,
        dfu_offset,
        dfu_size,
    );
    let partition = mgr.storage_partition();
    MANAGER.init(mgr).ok();
    partition
}

/// Mark firmware boot as successful so the bootloader doesn't revert on next reset.
pub fn mark_booted() {
    if let Some(mgr) = get_manager() {
        let state_part = mgr.state_partition();
        static ALIGNED: StaticCell<[u8; DFU_WRITE_SIZE]> = StaticCell::new();
        let aligned: &'static mut [u8] = ALIGNED.init([0; DFU_WRITE_SIZE]);
        let mut state = BlockingFirmwareState::new(state_part, aligned);
        state.mark_booted().ok();
    }
}

/// Get a reference to the global DFU flash manager.
pub fn get_manager() -> Option<&'static DfuFlashManager> {
    MANAGER.try_get()
}

/// Initialize flash using partition offsets from the linker (rmk-memory.x).
///
/// Requires `rmk-memory.x` to be linked into the firmware binary
/// (e.g. `-Trmk-memory.x` in the linker flags). It reads the same
/// `__bootloader_*` symbols that embassy-boot's
/// `from_linkerfile_blocking()` uses; `__bootloader_storage_*` is an RMK
/// extension.
///
/// # Safety
///
/// Reads linker-defined absolute symbols. The symbols must be present in the
/// linked binary or the firmware will not link.
pub fn init_flash_from_linkerscript(flash_peri: Peri<'static, FLASH>) -> PartitionType {
    unsafe extern "C" {
        static __bootloader_state_start: u8;
        static __bootloader_state_end: u8;
        static __bootloader_dfu_start: u8;
        static __bootloader_dfu_end: u8;
        static __bootloader_storage_start: u8;
        static __bootloader_storage_end: u8;
    }
    // SAFETY: linker-defined symbols — reading their addresses is safe.
    init_flash(
        flash_peri,
        core::ptr::addr_of!(__bootloader_storage_start) as usize as u32,
        core::ptr::addr_of!(__bootloader_storage_end) as usize as u32
            - core::ptr::addr_of!(__bootloader_storage_start) as usize as u32,
        core::ptr::addr_of!(__bootloader_state_start) as usize as u32,
        core::ptr::addr_of!(__bootloader_state_end) as usize as u32
            - core::ptr::addr_of!(__bootloader_state_start) as usize as u32,
        core::ptr::addr_of!(__bootloader_dfu_start) as usize as u32,
        core::ptr::addr_of!(__bootloader_dfu_end) as usize as u32
            - core::ptr::addr_of!(__bootloader_dfu_start) as usize as u32,
    )
}
