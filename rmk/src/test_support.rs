//! Test-only helpers, never compiled into firmware.
//!
//! Serves two consumers: `#[cfg(test)]` modules under `src/`, and the simulator
//! harness in `tests/integration/simulator`. The accessors below are wrappers
//! rather than `pub use`, which can't widen `pub(crate)` visibility.

use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use embassy_time::{Duration, MockDriver};

pub const COMBO_MAX_LENGTH: usize = crate::COMBO_MAX_LENGTH;

#[cfg(feature = "vial")]
pub fn to_via_keycode(action: rmk_types::action::KeyAction) -> u16 {
    crate::host::via::keycode_convert::to_via_keycode(action)
}

#[cfg(all(feature = "_no_usb", feature = "_ble"))]
pub fn set_ble_state(state: rmk_types::ble::BleState) {
    crate::state::set_ble_state(state);
}

#[cfg(any(not(feature = "_no_usb"), feature = "_ble"))]
pub fn reset_connection_status() {
    crate::state::CONNECTION_STATUS.lock(|c| c.set(rmk_types::connection::ConnectionStatus::default()));
}

#[cfg(feature = "storage")]
pub fn clear_flash_channel() {
    crate::storage::clear_flash_channel();
}

/// Stand-in for the storage task when a simulation has no flash: every write
/// lands, every read is absent, so nothing blocks on a never-serviced queue.
pub async fn drain_flash_channel() {
    #[cfg(feature = "storage")]
    crate::storage::drain_flash_channel().await;
    #[cfg(not(feature = "storage"))]
    core::future::pending::<()>().await
}

const STEP: Duration = Duration::from_micros(100);
const MAX_ITERS: usize = 600_000; // 60 s of virtual time

/// Drop-in replacement for `embassy_futures::block_on` that advances
/// `embassy-time`'s mock clock.
pub fn test_block_on<F: Future>(fut: F) -> F::Output {
    require_nextest();
    MockDriver::get().reset();

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    let mut fut = pin!(fut);
    for _ in 0..MAX_ITERS {
        if let Poll::Ready(out) = fut.as_mut().poll(&mut cx) {
            return out;
        }
        MockDriver::get().advance(STEP);
    }
    panic!(
        "test_block_on: future did not resolve within {} iterations ({} s of virtual time)",
        MAX_ITERS,
        (MAX_ITERS as u64 * STEP.as_micros()) / 1_000_000,
    );
}

// `embassy-time`'s MockDriver is a process-global singleton, so running the
// suite under plain `cargo test` lets tests race on it and hang at the 60 s
// virtual-time kill switch above. Fail the first mock-clock test with a pointer
// to the right runner instead of making the user wait for that timeout.
fn require_nextest() {
    if std::env::var_os("NEXTEST").is_none() {
        panic!(
            "\nrmk tests must run under cargo-nextest (embassy-time's MockDriver \
             is a process-global singleton and needs per-test process isolation).\n\
             \n  cargo install cargo-nextest --locked\n\n\
             Then from rmk/:\n\n  \
             cargo nextest run --no-default-features \
             --features=split,vial,storage,async_matrix,_ble\n\n\
             Or for the behavioral suite: `bash scripts/test_all.sh` from the repo root.\n"
        );
    }
}

fn noop_waker() -> Waker {
    // Safety: every vtable function is a true no-op; no state is ever
    // dereferenced through the data pointer.
    unsafe { Waker::from_raw(RAW) }
}

const RAW: RawWaker = RawWaker::new(core::ptr::null(), &VTABLE);

const VTABLE: RawWakerVTable = RawWakerVTable::new(
    |_| RAW, // clone
    |_| {},  // wake
    |_| {},  // wake_by_ref
    |_| {},  // drop
);

/// Test-only stand-in for the BLE profile task: records the `Debug` form of every
/// received action in `sink`, so a test can check what a profile-key gesture sent.
pub async fn drain_ble_profile_channel(sink: &mut std::vec::Vec<std::string::String>) {
    #[cfg(feature = "_ble")]
    loop {
        sink.push(std::format!(
            "{:?}",
            crate::channel::BLE_PROFILE_CHANNEL.receive().await
        ));
    }
    #[cfg(not(feature = "_ble"))]
    {
        let _ = sink;
        core::future::pending::<()>().await
    }
}

/// An in-memory NOR flash part, `SIZE` bytes in `ERASE`-byte sectors written
/// `WRITE` bytes at a time.
///
/// Cloning shares the same bytes, so a second build over a clone reads back what
/// the first one persisted — the stand-in for a power cycle.
#[cfg(feature = "storage")]
#[derive(Clone)]
pub struct InMemoryFlash<const SIZE: usize, const ERASE: usize, const WRITE: usize> {
    data: std::rc::Rc<core::cell::RefCell<[u8; SIZE]>>,
    fail_writes: std::rc::Rc<core::cell::Cell<bool>>,
    writes: std::rc::Rc<core::cell::Cell<usize>>,
}

#[cfg(feature = "storage")]
impl<const SIZE: usize, const ERASE: usize, const WRITE: usize> InMemoryFlash<SIZE, ERASE, WRITE> {
    pub fn new() -> Self {
        Self {
            data: std::rc::Rc::new(core::cell::RefCell::new([0xFF; SIZE])),
            fail_writes: std::rc::Rc::new(core::cell::Cell::new(false)),
            writes: std::rc::Rc::new(core::cell::Cell::new(0)),
        }
    }

    /// How many writes have landed, shared by every clone.
    pub fn writes(&self) -> usize {
        self.writes.get()
    }

    /// Reject every write while set, shared by every clone — the stand-in for a
    /// flash that stops taking data.
    pub fn fail_writes(&self, fail: bool) {
        self.fail_writes.set(fail);
    }
}

#[cfg(feature = "storage")]
impl<const SIZE: usize, const ERASE: usize, const WRITE: usize> Default for InMemoryFlash<SIZE, ERASE, WRITE> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "storage")]
impl<const SIZE: usize, const ERASE: usize, const WRITE: usize> embedded_storage::nor_flash::ErrorType
    for InMemoryFlash<SIZE, ERASE, WRITE>
{
    type Error = embedded_storage::nor_flash::NorFlashErrorKind;
}

#[cfg(feature = "storage")]
impl<const SIZE: usize, const ERASE: usize, const WRITE: usize> embedded_storage::nor_flash::ReadNorFlash
    for InMemoryFlash<SIZE, ERASE, WRITE>
{
    const READ_SIZE: usize = 1;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        embedded_storage::nor_flash::check_read(self, offset, bytes.len())?;
        let offset = offset as usize;
        bytes.copy_from_slice(&self.data.borrow()[offset..offset + bytes.len()]);
        Ok(())
    }

    fn capacity(&self) -> usize {
        SIZE
    }
}

#[cfg(feature = "storage")]
impl<const SIZE: usize, const ERASE: usize, const WRITE: usize> embedded_storage::nor_flash::NorFlash
    for InMemoryFlash<SIZE, ERASE, WRITE>
{
    const WRITE_SIZE: usize = WRITE;
    const ERASE_SIZE: usize = ERASE;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        embedded_storage::nor_flash::check_erase(self, from, to)?;
        self.data.borrow_mut()[from as usize..to as usize].fill(0xFF);
        Ok(())
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        embedded_storage::nor_flash::check_write(self, offset, bytes.len())?;
        if self.fail_writes.get() {
            return Err(embedded_storage::nor_flash::NorFlashErrorKind::Other);
        }
        let mut data = self.data.borrow_mut();
        let offset = offset as usize;
        for (current, byte) in data[offset..offset + bytes.len()].iter_mut().zip(bytes) {
            // Real NOR only clears bits; writing a 1 over a 0 needs an erase first.
            if *current & *byte != *byte {
                return Err(embedded_storage::nor_flash::NorFlashErrorKind::Other);
            }
            *current &= *byte;
        }
        self.writes.set(self.writes.get() + 1);
        Ok(())
    }
}
