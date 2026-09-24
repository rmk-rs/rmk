//! A compact deadline registry backing the keyboard task's single wait point.
//!
//! `DeadlineSet` holds one optional deadline per key, so every timeout source
//! has a stable, named slot. The keyboard task races [`DeadlineSet::next`],
//! the earliest armed deadline, against its event subscriber, and fires the
//! due slots when the wait times out.

use core::marker::PhantomData;
use core::num::NonZeroU64;

use embassy_time::Instant;

/// A deadline timestamp in compact form: the instant's ticks forced non-zero,
/// so `Option<Deadline>` occupies a single word. `Instant` itself has no
/// niche, which would make every `Option<Instant>` 16 bytes.
///
/// Tick 0 never occurs in practice: it is the boot instant, and deadlines are
/// armed as `now + duration`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Deadline(NonZeroU64);

impl Deadline {
    /// Pack an instant into the compact form, clamping tick 0 to tick 1.
    pub(crate) fn new(at: Instant) -> Self {
        Self(NonZeroU64::new(at.as_ticks().max(1)).expect("ticks clamped to non-zero"))
    }

    /// Unpack back into an instant.
    pub(crate) fn instant(self) -> Instant {
        Instant::from_ticks(self.0.get())
    }
}

/// Identifies one slot of a [`DeadlineSet`]. The slot count is a const
/// generic on the set, not on this trait, because an associated const cannot
/// size the slot array.
pub trait DeadlineKey: Copy {
    /// The slot this key arms; must be unique per key and less than the set's
    /// slot count.
    fn slot(self) -> usize;
}

/// One optional deadline per key. The wait loop races [`Self::next`] against
/// the event subscriber; the fire loop expires individual slots.
pub struct DeadlineSet<K: DeadlineKey, const SLOTS: usize> {
    slots: [Option<Deadline>; SLOTS],
    // `K` only appears in method signatures; this ties it to the set.
    _key: PhantomData<K>,
}

impl<K: DeadlineKey, const SLOTS: usize> Default for DeadlineSet<K, SLOTS> {
    fn default() -> Self {
        Self {
            slots: [None; SLOTS],
            _key: PhantomData,
        }
    }
}

impl<K: DeadlineKey, const SLOTS: usize> DeadlineSet<K, SLOTS> {
    /// Create an empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Arm (or overwrite) the deadline for `key`.
    pub fn set(&mut self, key: K, at: Instant) {
        self.slots[key.slot()] = Some(Deadline::new(at));
    }

    /// Clear the deadline for `key`.
    pub fn clear(&mut self, key: K) {
        self.slots[key.slot()] = None;
    }

    /// Arm the deadline for `key`, keeping any later existing one.
    pub fn extend(&mut self, key: K, at: Instant) {
        if self.get(key).is_some_and(|current| at <= current) {
            return;
        }
        self.set(key, at);
    }

    /// Mirror a source-owned deadline into the table: set when the source
    /// reports one, clear when it reports none.
    pub fn set_or_clear(&mut self, key: K, at: Option<Instant>) {
        match at {
            Some(at) => self.set(key, at),
            None => self.clear(key),
        }
    }

    /// Get the armed deadline for `key`, if any.
    pub fn get(&self, key: K) -> Option<Instant> {
        self.slots[key.slot()].map(Deadline::instant)
    }

    /// Whether `key` is armed and its deadline has passed. A deadline at
    /// exactly `now` counts as due.
    pub fn is_due(&self, key: K, now: Instant) -> bool {
        self.get(key).is_some_and(|at| at <= now)
    }

    /// The earliest armed deadline, if any.
    pub fn next(&self) -> Option<Instant> {
        self.slots.iter().flatten().min().map(|d| d.instant())
    }
}

impl<K: DeadlineKey, const SLOTS: usize> core::fmt::Debug for DeadlineSet<K, SLOTS> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut armed = f.debug_list();
        for (slot, deadline) in self.slots.iter().enumerate() {
            if let Some(deadline) = deadline {
                armed.entry(&(slot, deadline.instant().as_ticks()));
            }
        }
        armed.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, PartialEq)]
    enum TestKey {
        A = 0,
        B = 1,
        C = 2,
    }

    impl DeadlineKey for TestKey {
        fn slot(self) -> usize {
            self as usize
        }
    }

    type TestSet = DeadlineSet<TestKey, 3>;

    fn at(ticks: u64) -> Instant {
        Instant::from_ticks(ticks)
    }

    #[test]
    fn empty_set_has_no_deadline() {
        let set = TestSet::new();
        assert_eq!(set.next(), None);
        assert!(!set.is_due(TestKey::A, at(1000)));
    }

    #[test]
    fn set_get_clear_roundtrip() {
        let mut set = TestSet::new();
        set.set(TestKey::B, at(500));
        assert_eq!(set.get(TestKey::B), Some(at(500)));
        assert!(set.is_due(TestKey::B, at(500)));
        assert!(!set.is_due(TestKey::B, at(499)));
        set.clear(TestKey::B);
        assert_eq!(set.get(TestKey::B), None);
        assert_eq!(set.next(), None);
    }

    #[test]
    fn next_returns_minimum_across_slots() {
        let mut set = TestSet::new();
        set.set(TestKey::C, at(900));
        set.set(TestKey::A, at(300));
        set.set(TestKey::B, at(600));
        assert_eq!(set.next(), Some(at(300)));
        set.clear(TestKey::A);
        assert_eq!(set.next(), Some(at(600)));
        set.clear(TestKey::B);
        assert_eq!(set.next(), Some(at(900)));
    }

    #[test]
    fn set_overwrites_either_direction() {
        let mut set = TestSet::new();
        set.set(TestKey::A, at(500));
        set.set(TestKey::A, at(100));
        assert_eq!(set.get(TestKey::A), Some(at(100)));
    }

    #[test]
    fn extend_only_pushes_forward() {
        let mut set = TestSet::new();
        set.extend(TestKey::A, at(500));
        assert_eq!(set.get(TestKey::A), Some(at(500)));
        set.extend(TestKey::A, at(300));
        assert_eq!(set.get(TestKey::A), Some(at(500)));
        set.extend(TestKey::A, at(700));
        assert_eq!(set.get(TestKey::A), Some(at(700)));
    }

    #[test]
    fn set_or_clear_mirrors_the_source() {
        let mut set = TestSet::new();
        set.set_or_clear(TestKey::A, Some(at(500)));
        assert_eq!(set.get(TestKey::A), Some(at(500)));
        set.set_or_clear(TestKey::A, None);
        assert_eq!(set.get(TestKey::A), None);
        set.set_or_clear(TestKey::B, None);
        assert_eq!(set.get(TestKey::B), None);
    }

    #[test]
    fn zero_tick_deadline_clamps_to_one() {
        let mut set = TestSet::new();
        set.set(TestKey::A, at(0));
        assert_eq!(set.get(TestKey::A), Some(at(1)));
    }

    #[test]
    fn compact_repr_stays_a_word() {
        assert_eq!(core::mem::size_of::<Option<Deadline>>(), 8);
        assert_eq!(core::mem::size_of::<TestSet>(), 24);
    }
}
