//! Physical-presence unlock gate shared by the Vial and Rynk host services.

use core::cell::Cell;

use embassy_time::{Duration, Instant};
#[cfg(feature = "rynk")]
use rmk_types::protocol::rynk::LockStatus;

use crate::keymap::KeyMap;

/// QMK's `VIAL_UNLOCK_COUNTER_MAX`: how many polls Vial needs before it commits
/// the unlock. The host uses the remaining count to draw its progress bar.
const VIAL_UNLOCK_COUNTER_MAX: u8 = 50;
/// QMK only consumes a counter tick after this much time has passed, so a host
/// polling faster cannot speed up the ramp.
const VIAL_UNLOCK_TICK_INTERVAL: Duration = Duration::from_millis(100);

/// Unlock gate. Interior mutability lets protocol handlers update it through a
/// shared reference without holding a borrow across an `.await` point.
pub(crate) struct HostLock<'a> {
    unlocked: Cell<bool>,
    unlocking: Cell<bool>,
    last_poll: Cell<Instant>,
    /// Ticks left in the Vial hold-to-unlock ramp (QMK's
    /// `vial_unlock_counter`). Reaching zero commits the unlock.
    vial_counter: Cell<u8>,
    /// When the last Vial tick was consumed, or when the attempt started.
    vial_last_tick: Cell<Instant>,
    /// Start (and stay) unlocked — development escape hatch.
    insecure: bool,
    /// How long an armed attempt survives without a refreshing poll. Vial
    /// passes 100 ms; Rynk passes 500 ms to tolerate a BLE WebHID round trip.
    window: Duration,
    unlock_keys: &'a [(u8, u8)],
    keymap: &'a KeyMap<'a>,
}

impl<'a> HostLock<'a> {
    pub fn new(unlock_keys: &'a [(u8, u8)], keymap: &'a KeyMap<'a>, insecure: bool, window: Duration) -> Self {
        Self {
            unlocked: Cell::new(insecure),
            unlocking: Cell::new(false),
            last_poll: Cell::new(Instant::MIN),
            vial_counter: Cell::new(VIAL_UNLOCK_COUNTER_MAX),
            vial_last_tick: Cell::new(Instant::MIN),
            insecure,
            window,
            unlock_keys,
            keymap,
        }
    }

    pub fn is_unlocking(&self) -> bool {
        self.update_unlocking_state();
        self.unlocking.get()
    }

    /// `insecure` forces unlocked regardless of the `unlocked` cell.
    pub fn is_unlocked(&self) -> bool {
        self.insecure || self.unlocked.get()
    }

    pub fn unlocking(&self) {
        self.unlocking.set(true);
        self.last_poll.set(Instant::now());
    }

    pub fn unlock(&self) {
        if self.unlocking.get() {
            self.unlocked.set(true);
            self.unlocking.set(false);
        }
    }

    /// Arm a fresh Vial attempt (`vial_unlock_start`) and reset the hold ramp.
    pub fn vial_unlock_start(&self) {
        self.unlocking();
        self.vial_counter.set(VIAL_UNLOCK_COUNTER_MAX);
        self.vial_last_tick.set(Instant::now());
    }

    /// Vial `unlock_poll`: refresh the attempt, advance the hold ramp, and
    /// return the ticks left for the host's progress bar.
    ///
    /// This mirrors QMK's `vial_unlock_poll`: one tick is consumed only while
    /// every challenge key stays held and at least
    /// [`VIAL_UNLOCK_TICK_INTERVAL`] has passed. Releasing a key restarts the
    /// ramp, and the unlock commits when the counter reaches zero. Rynk's
    /// `poll` keeps its own `remaining_keys` semantics and does not use this
    /// ramp.
    pub fn vial_unlock_poll(&self) -> u8 {
        self.unlocking();

        if self.unlock_keys.is_empty() {
            warn!("No unlock keys provided");
            return 1;
        }

        if self.remaining_held() == 0 && self.vial_last_tick.get().elapsed() > VIAL_UNLOCK_TICK_INTERVAL {
            self.vial_last_tick.set(Instant::now());
            let remaining = self.vial_counter.get().saturating_sub(1);
            self.vial_counter.set(remaining);
            if remaining == 0 {
                self.unlock();
            }
        } else {
            // Not a full tick of continuous holding: restart the ramp, and
            // require a fresh interval before the next tick can count.
            self.vial_counter.set(VIAL_UNLOCK_COUNTER_MAX);
            self.vial_last_tick.set(Instant::now());
        }

        self.vial_counter.get()
    }

    /// Count challenge keys not currently held, committing the unlock when all
    /// are held. Returns nonzero forever on an empty challenge (warn-and-refuse).
    pub fn check_unlock(&self) -> u8 {
        if self.unlock_keys.is_empty() {
            warn!("No unlock keys provided");
            return 1;
        }
        let counter = self.remaining_held();
        if counter == 0 {
            self.unlock();
        }
        counter
    }

    pub fn lock(&self) {
        self.unlocked.set(false);
    }

    /// Challenge keys not currently held — no arm, no commit.
    fn remaining_held(&self) -> u8 {
        let mut counter = self.unlock_keys.len() as u8;
        for (row, col) in self.unlock_keys {
            if self.keymap.read_matrix_key(*row, *col) {
                counter -= 1;
            }
        }
        counter
    }

    fn update_unlocking_state(&self) {
        if self.last_poll.get().elapsed() > self.window {
            self.unlocking.set(false);
        }
    }
}

#[cfg(feature = "rynk")]
impl HostLock<'_> {
    /// Side-effect-free snapshot for `GetLockStatus`: reports the current lock
    /// state and challenge without arming an attempt (lazy window-expiry aside).
    pub fn status(&self) -> LockStatus {
        let unlocking = self.is_unlocking();
        LockStatus {
            locked: !self.is_unlocked(),
            unlocking,
            // Live progress only while an attempt is armed; otherwise the full count.
            remaining_keys: if unlocking {
                self.remaining_held()
            } else {
                self.unlock_keys.len() as u8
            },
            key_positions: self.unlock_keys.iter().copied().collect(),
        }
    }

    /// Arm/refresh the attempt, sample held keys, and unlock when all are held.
    /// Idempotent — the first call arms, every call refreshes and re-samples.
    pub fn poll(&self) -> LockStatus {
        if self.unlock_keys.is_empty() {
            warn!("Rynk unlock polled but no unlock_keys configured — permanently locked");
            return LockStatus {
                locked: !self.is_unlocked(),
                unlocking: false,
                remaining_keys: 0,
                key_positions: heapless::Vec::new(),
            };
        }
        self.unlocking();
        let remaining = self.check_unlock();
        LockStatus {
            locked: !self.is_unlocked(),
            unlocking: self.is_unlocking(),
            remaining_keys: remaining,
            key_positions: self.unlock_keys.iter().copied().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use embassy_time::MockDriver;
    use rmk_types::action::KeyAction;

    use super::*;
    use crate::config::{BehaviorConfig, PositionalConfig};
    use crate::event::KeyboardEvent;
    use crate::keymap::KeymapData;
    use crate::test_support::test_block_on as block_on;

    /// A 2×2, single-layer keymap — enough to hold challenge keys at (0,0)/(1,1).
    macro_rules! keymap {
        ($km:ident) => {
            let mut behavior = BehaviorConfig::default();
            let positional: PositionalConfig<2, 2> = PositionalConfig::default();
            let mut data: KeymapData<2, 2, 1, 0> =
                KeymapData::new([[[KeyAction::No, KeyAction::No], [KeyAction::No, KeyAction::No]]]);
            let $km = block_on(KeyMap::new(&mut data, &mut behavior, &positional));
        };
    }

    const WINDOW: Duration = Duration::from_millis(500);

    #[test]
    fn window_arms_refreshes_and_expires() {
        keymap!(keymap);
        let keys = [(0u8, 0u8)];
        let lock = HostLock::new(&keys, &keymap, false, WINDOW);

        MockDriver::get().reset();
        assert!(!lock.is_unlocking(), "no attempt armed initially");

        lock.unlocking();
        assert!(lock.is_unlocking(), "armed");

        MockDriver::get().advance(Duration::from_millis(300));
        lock.unlocking(); // refresh before the window lapses
        MockDriver::get().advance(Duration::from_millis(300));
        assert!(lock.is_unlocking(), "refresh kept it alive past the original 500 ms");

        MockDriver::get().advance(Duration::from_millis(501));
        assert!(!lock.is_unlocking(), "expired after the window with no refresh");
    }

    #[test]
    fn partial_then_full_hold_unlocks() {
        keymap!(keymap);
        let keys = [(0u8, 0u8), (1u8, 1u8)];
        let lock = HostLock::new(&keys, &keymap, false, WINDOW);

        lock.unlocking();
        keymap.update_matrix_state(&KeyboardEvent::key(0, 0, true));
        assert_eq!(lock.check_unlock(), 1, "one key still needed");
        assert!(!lock.is_unlocked());

        keymap.update_matrix_state(&KeyboardEvent::key(1, 1, true));
        assert_eq!(lock.check_unlock(), 0, "all held");
        assert!(lock.is_unlocked(), "unlocked once all keys held");
    }

    #[test]
    fn vial_unlock_requires_continuous_hold() {
        keymap!(keymap);
        let keys = [(0u8, 0u8), (1u8, 1u8)];
        let lock = HostLock::new(&keys, &keymap, false, WINDOW);

        MockDriver::get().reset();
        lock.vial_unlock_start();
        keymap.update_matrix_state(&KeyboardEvent::key(0, 0, true));
        keymap.update_matrix_state(&KeyboardEvent::key(1, 1, true));

        // A poll before the tick interval elapses does not advance the ramp.
        assert_eq!(lock.vial_unlock_poll(), VIAL_UNLOCK_COUNTER_MAX);

        MockDriver::get().advance(Duration::from_millis(101));
        assert_eq!(lock.vial_unlock_poll(), VIAL_UNLOCK_COUNTER_MAX - 1);
        assert!(!lock.is_unlocked());

        // Releasing a key restarts the ramp.
        keymap.update_matrix_state(&KeyboardEvent::key(1, 1, false));
        MockDriver::get().advance(Duration::from_millis(101));
        assert_eq!(lock.vial_unlock_poll(), VIAL_UNLOCK_COUNTER_MAX);
        assert!(!lock.is_unlocked());

        // Holding both keys through every remaining tick commits the unlock.
        keymap.update_matrix_state(&KeyboardEvent::key(1, 1, true));
        for _ in 0..VIAL_UNLOCK_COUNTER_MAX {
            MockDriver::get().advance(Duration::from_millis(101));
            lock.vial_unlock_poll();
        }
        assert!(lock.is_unlocked());
    }

    #[test]
    fn empty_unlock_keys_never_unlocks() {
        keymap!(keymap);
        let keys: [(u8, u8); 0] = [];
        let lock = HostLock::new(&keys, &keymap, false, WINDOW);

        lock.unlocking();
        assert_ne!(lock.check_unlock(), 0, "empty challenge refuses");
        assert!(!lock.is_unlocked());
    }

    #[test]
    fn insecure_starts_and_stays_unlocked() {
        keymap!(keymap);
        let keys = [(0u8, 0u8)];
        let lock = HostLock::new(&keys, &keymap, true, WINDOW);

        assert!(lock.is_unlocked(), "insecure starts unlocked");
        lock.lock();
        assert!(lock.is_unlocked(), "lock() does not override insecure mode");
    }

    #[cfg(feature = "rynk")]
    #[test]
    fn poll_reports_progress_then_unlocks() {
        keymap!(keymap);
        let keys = [(0u8, 0u8), (1u8, 1u8)];
        let lock = HostLock::new(&keys, &keymap, false, WINDOW);

        let s = lock.poll();
        assert!(s.locked && s.unlocking);
        assert_eq!(s.remaining_keys, 2);
        assert_eq!(s.key_positions.len(), 2, "challenge advertised");

        keymap.update_matrix_state(&KeyboardEvent::key(0, 0, true));
        assert_eq!(lock.poll().remaining_keys, 1);

        keymap.update_matrix_state(&KeyboardEvent::key(1, 1, true));
        let s = lock.poll();
        assert!(!s.locked, "unlocked once both held");
        assert!(!s.unlocking);
        assert_eq!(s.remaining_keys, 0);
    }

    #[cfg(feature = "rynk")]
    #[test]
    fn status_is_side_effect_free() {
        keymap!(keymap);
        let keys = [(0u8, 0u8)];
        let lock = HostLock::new(&keys, &keymap, false, WINDOW);

        let s = lock.status();
        assert!(s.locked);
        assert!(!s.unlocking, "status must not arm an attempt");
        assert_eq!(s.remaining_keys, 1, "full count when no attempt is armed");
        assert_eq!(s.key_positions.len(), 1);
        assert!(!lock.is_unlocking(), "still no armed attempt after status()");
    }
}
