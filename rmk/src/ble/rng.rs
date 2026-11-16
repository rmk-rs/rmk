//! Random numbers for trouble-host's `embassy-crypto` backend.
//!
//! `init` must run before `trouble_host::new`, which generates the LE Secure Connections key pair.
//! The host runner isn't running yet, so only a controller that runs HCI commands directly can answer `LE Rand`.

use core::cell::RefCell;

use bt_hci::cmd::le::LeRand;
use bt_hci::controller::ControllerCmdSync;
// Link the selected software crypto drivers.
use embassy_crypto_rustcrypto as _;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use rand_chacha::ChaCha12Rng;
use rand_chacha::rand_core::{RngCore, SeedableRng};

// embassy-crypto finds its RNG driver at link time, so the generator has to be a static.
static RNG: Mutex<CriticalSectionRawMutex, RefCell<Option<ChaCha12Rng>>> = Mutex::new(RefCell::new(None));

/// ESP: the hardware RNG is true random while Bluetooth is on, which it is by now.
#[cfg(feature = "_esp_ble")]
pub(crate) async fn init<C: ControllerCmdSync<LeRand>>(_controller: &C) {
    let mut bytes = [0; 32];
    esp_hal::rng::Rng::new().read(&mut bytes);
    seed(bytes);
}

/// Pico W: the ring oscillator, because the CYW43439 only answers through the host runner.
#[cfg(feature = "pico_w_ble")]
pub(crate) async fn init<C: ControllerCmdSync<LeRand>>(_controller: &C) {
    let mut bytes = [0; 32];
    embassy_rp::clocks::RoscRng.fill_bytes(&mut bytes);
    seed(bytes);
}

/// nRF: the SoftDevice Controller owns the chip's RNG and runs `LE Rand` directly.
#[cfg(not(any(feature = "_esp_ble", feature = "pico_w_ble")))]
pub(crate) async fn init<C: trouble_host::Controller + ControllerCmdSync<LeRand>>(controller: &C) {
    let mut bytes = [0; 32];
    for chunk in bytes.as_chunks_mut::<8>().0 {
        match ControllerCmdSync::exec(controller, &LeRand::new()).await {
            Ok(random) => *chunk = random,
            Err(e) => {
                error!("[ble] failed to seed the RNG, rebooting: {:?}", e);
                embassy_time::Timer::after_millis(100).await;
                crate::boot::reboot_keyboard();
                return;
            }
        }
    }
    seed(bytes);
}

/// Seeds the generator with `bytes`. Tests have no hardware, so they pass a constant.
pub(crate) fn seed(bytes: [u8; 32]) {
    RNG.lock(|rng| *rng.borrow_mut() = Some(ChaCha12Rng::from_seed(bytes)));
}

struct RngDriver;

impl embassy_crypto::driver::Rng for RngDriver {
    fn fill_bytes(buf: &mut [u8]) {
        RNG.lock(|rng| {
            rng.borrow_mut()
                .as_mut()
                .expect("invariant: init runs before the BLE stack is built")
                .fill_bytes(buf)
        })
    }
}

// The std self-dependency must not register a second driver in the unit-test binary.
#[cfg(any(not(feature = "std"), test))]
embassy_crypto::rng_impl!(RngDriver);

#[cfg(test)]
mod tests {
    use embassy_crypto::driver::Rng;

    use super::{RngDriver, seed};

    #[test]
    fn same_seed_gives_same_bytes() {
        let mut first = [0; 32];
        seed([7; 32]);
        RngDriver::fill_bytes(&mut first);

        let mut second = [0; 32];
        seed([7; 32]);
        RngDriver::fill_bytes(&mut second);

        assert_eq!(first, second);
        assert_ne!(first, [0; 32]);
    }
}
