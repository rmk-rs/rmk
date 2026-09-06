use core::cell::RefCell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use crate::dfu::MAX_DFU_ALTS;

/// A firmware binary reference and its pre-computed CRC-32.
struct FirmwareSlot {
    data: &'static [u8],
    hash: u32,
}

/// Global registry: peripheral ID → (`FirmwareSlot`).
///
/// Populated via [`set_firmware_update_data`].
/// Looked up by [`PeripheralManager`] on
/// connection to decide whether an update is needed.
static FW_SLOTS: Mutex<CriticalSectionRawMutex, RefCell<heapless::Vec<(usize, FirmwareSlot), MAX_DFU_ALTS>>> =
    Mutex::new(RefCell::new(heapless::Vec::new()));

/// Register a peripheral firmware binary for automatic dfu_split updates.
///
/// The central calls this (typically at startup) so that
/// `PeripheralManager` can verify and, if needed, update the peripheral's
/// firmware when the split link is established.
///
/// `id` must match the peripheral index in `[[split.peripheral]]` (or the
/// `id` argument of `run_peripheral_manager`).  `hash` is the CRC-32 of
/// the firmware binary — typically computed via [`crate::crc32::crc32`].
/// `id` must be unique; if a slot for the same `id` already exists, it will be replaced.
/// Every peripheral has only one firmware slot, given by its unique `id`.
///
/// Returns `Err(())` if the registry is full (max `MAX_DFU_ALTS` entries).
pub fn set_firmware_update_data(id: usize, firmware: &'static [u8], hash: u32) -> Result<(), ()> {
    FW_SLOTS.lock(|cell| {
        let slots = &mut cell.borrow_mut();
        if let Some(slot) = slots.iter_mut().find(|(i, _)| *i == id) {
            *slot = (id, FirmwareSlot { data: firmware, hash });
        } else {
            slots
                .push((id, FirmwareSlot { data: firmware, hash }))
                .map_err(|_| ())?;
        }
        Ok(())
    })
}

/// Retrieve the firmware binary and its expected CRC-32 for a given
/// peripheral ID, if one has been registered.
pub fn get_firmware_update_data(id: usize) -> Option<(&'static [u8], u32)> {
    FW_SLOTS.lock(|cell| {
        let slots = cell.borrow();
        slots.iter().find(|(i, _)| *i == id).map(|(_, s)| (s.data, s.hash))
    })
}
