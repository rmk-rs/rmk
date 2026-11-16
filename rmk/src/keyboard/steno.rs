//! Plover HID stenography live-state reporter.
//!
//! The Plover HID protocol sends the full bitmap of currently-held steno
//! keys on every state change (press or release). Chord detection happens
//! on the host side: Plover accumulates pressed keys and fires the chord
//! according to its own policy (all-up by default, or first-up when
//! configured).
//!
//! Wire-format ordering matches the Plover HID spec: chart index
//! 0 (`StenoKey::S1`) is the most significant bit of byte 1 of the report,
//! chart index 63 (`StenoKey::X26`) is the least significant bit of byte 8.

use rmk_types::steno::StenoKey;

use crate::hid::{Report, StenoReport};

#[derive(Debug)]
pub(crate) struct StenoChord {
    /// Live bitmap of currently-held steno keys. Stored such that
    /// `state.to_be_bytes()` already has the wire-format bit order:
    /// chart index 0 = MSB of the first byte.
    state: u64,
}

impl StenoChord {
    pub(crate) const fn new() -> Self {
        Self { state: 0 }
    }

    /// Update live state for a steno key event. Plover accumulates the bitmap
    /// itself, so every press and release produces a report.
    pub(crate) fn on_event(&mut self, key: StenoKey, pressed: bool) -> Report {
        let mask = 1u64 << (63 - key.chart_index());
        if pressed {
            self.state |= mask;
        } else {
            self.state &= !mask;
        }
        Report::StenoReport(StenoReport {
            keys: self.state.to_be_bytes(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_bytes(report: Report) -> [u8; 8] {
        match report {
            Report::StenoReport(r) => r.keys,
            _ => panic!("expected StenoReport"),
        }
    }

    fn bitmap(report: Report) -> u64 {
        u64::from_be_bytes(report_bytes(report))
    }

    fn msb_bit(key: StenoKey) -> u64 {
        1u64 << (63 - key.chart_index())
    }

    #[test]
    fn press_sends_live_state() {
        let mut s = StenoChord::new();
        let b = bitmap(s.on_event(StenoKey::S1, true));
        assert_eq!(b, msb_bit(StenoKey::S1));

        let b = bitmap(s.on_event(StenoKey::T, true));
        assert_eq!(b, msb_bit(StenoKey::S1) | msb_bit(StenoKey::T));
    }

    #[test]
    fn release_clears_bit() {
        let mut s = StenoChord::new();
        s.on_event(StenoKey::S1, true);
        s.on_event(StenoKey::T, true);

        let b = bitmap(s.on_event(StenoKey::T, false));
        assert_eq!(b, msb_bit(StenoKey::S1), "T bit should be cleared");

        let b = bitmap(s.on_event(StenoKey::S1, false));
        assert_eq!(b, 0, "all bits should be cleared");
    }

    #[test]
    fn s1_is_msb_of_first_byte() {
        let mut s = StenoChord::new();
        let bytes = report_bytes(s.on_event(StenoKey::S1, true));
        assert_eq!(bytes[0], 0x80);
        assert_eq!(&bytes[1..], &[0; 7]);
    }

    #[test]
    fn x26_is_lsb_of_last_byte() {
        let mut s = StenoChord::new();
        let bytes = report_bytes(s.on_event(StenoKey::X26, true));
        assert_eq!(&bytes[..7], &[0; 7]);
        assert_eq!(bytes[7], 0x01);
    }

    #[test]
    fn repeated_chord_produces_zero_between() {
        let mut s = StenoChord::new();
        for _ in 0..3 {
            s.on_event(StenoKey::S1, true);
            s.on_event(StenoKey::T, true);
            s.on_event(StenoKey::T, false);
            let b = bitmap(s.on_event(StenoKey::S1, false));
            assert_eq!(b, 0, "all-up must produce zero bitmap");
        }
    }

    /// Press every key in turn, then release them all; returns the live-state
    /// bitmap with every key held.
    fn stroke(s: &mut StenoChord, keys: &[StenoKey]) -> u64 {
        let mut peak = 0u64;
        for &k in keys {
            peak = bitmap(s.on_event(k, true));
        }
        for &k in keys {
            s.on_event(k, false);
        }
        peak
    }

    /// "hello world" in Plover's default dictionary is three strokes:
    ///   HEL  (H + E + -L)
    ///   HRO  (H + R + O)
    ///   WORLD (W + O + -R + -L + -D)
    #[test]
    fn hello_world_strokes() {
        let mut s = StenoChord::new();

        let hel = stroke(&mut s, &[StenoKey::H, StenoKey::RE, StenoKey::RL]);
        assert_eq!(
            hel,
            msb_bit(StenoKey::H) | msb_bit(StenoKey::RE) | msb_bit(StenoKey::RL)
        );

        let hro = stroke(&mut s, &[StenoKey::H, StenoKey::R, StenoKey::O]);
        assert_eq!(hro, msb_bit(StenoKey::H) | msb_bit(StenoKey::R) | msb_bit(StenoKey::O));

        let world = stroke(
            &mut s,
            &[StenoKey::W, StenoKey::O, StenoKey::RR, StenoKey::RL, StenoKey::RD],
        );
        assert_eq!(
            world,
            msb_bit(StenoKey::W)
                | msb_bit(StenoKey::O)
                | msb_bit(StenoKey::RR)
                | msb_bit(StenoKey::RL)
                | msb_bit(StenoKey::RD)
        );
    }
}
