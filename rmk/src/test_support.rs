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
pub const MACRO_SPACE_SIZE: usize = crate::MACRO_SPACE_SIZE;

#[cfg(feature = "vial")]
pub fn to_via_keycode(action: rmk_types::action::KeyAction) -> u16 {
    rmk_types::protocol::vial_keycode::to_via_keycode(action)
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
    crate::channel::FLASH_CHANNEL.clear();
}

#[cfg(feature = "storage")]
pub fn reset_flash_operation() {
    crate::storage::FLASH_OPERATION_FINISHED.reset();
}

#[cfg(feature = "storage")]
pub async fn flash_operation_finished() -> bool {
    crate::storage::FLASH_OPERATION_FINISHED.wait().await
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
