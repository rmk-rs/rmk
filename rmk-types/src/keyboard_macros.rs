//! Keyboard macros: a macro is a sequence of [`MacroOp`]s.
//!
//! Default macros are compiled into rodata as `&[&[MacroOp]]`. The firmware keeps
//! every macro in one buffer of `MACRO_SPACE_SIZE` bytes they share, in the
//! encoding of the host that edits it; over Rynk a macro travels as a [`Macro`].

use core::fmt;

use heapless::CapacityError;
use postcard::experimental::max_size::MaxSize;
use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::action::Action;
use crate::constants::{MACRO_MAX_NUM, MACRO_SPACE_SIZE};

/// One step of a macro.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum MacroOp {
    /// Press and release the action.
    Tap(Action),
    /// Press the action and leave it pressed.
    Press(Action),
    /// Release the action.
    Release(Action),
    /// Wait this many milliseconds before the next op.
    Delay(u16),
    /// Type one ASCII character with its own shift state, ignoring held modifiers.
    Char(u8),
    /// The ops before the first one run when the macro key is pressed, the ops
    /// after it when the macro key is released.
    PauseForRelease,
}

/// A whole macro, held as the postcard encoding of its ops back to back: at most
/// `MACRO_SPACE_SIZE` bytes, a text character two, a key step three to five. It
/// serializes as the list of its ops, the same bytes as a `Vec<MacroOp>`.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Macro(heapless::Vec<u8, MACRO_SPACE_SIZE>);

impl Macro {
    pub const fn new() -> Self {
        Self(heapless::Vec::new())
    }

    /// `Err` when `ops` take more than `MACRO_SPACE_SIZE` bytes.
    pub fn from_slice(ops: &[MacroOp]) -> Result<Self, CapacityError> {
        let mut macro_ops = Self::new();
        ops.iter().try_for_each(|op| macro_ops.push(*op))?;
        Ok(macro_ops)
    }

    /// Append `op`, or leave the macro as it was when `op` doesn't fit.
    pub fn push(&mut self, op: MacroOp) -> Result<(), CapacityError> {
        let mut buf = [0; MacroOp::POSTCARD_MAX_SIZE];
        let bytes = postcard::to_slice(&op, &mut buf).map_err(|_| CapacityError::default())?;
        self.0.extend_from_slice(bytes)
    }

    pub fn ops(&self) -> impl Iterator<Item = MacroOp> + '_ {
        let mut rest = self.0.as_slice();
        core::iter::from_fn(move || {
            let (op, tail) = postcard::take_from_bytes(rest).ok()?;
            rest = tail;
            Some(op)
        })
    }

    /// The encoded ops, as the firmware's buffer keeps them.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The macro whose encoded ops are `bytes`, `None` unless they are whole ops
    /// that fit.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut rest = bytes;
        while !rest.is_empty() {
            rest = postcard::take_from_bytes::<MacroOp>(rest).ok()?.1;
        }
        heapless::Vec::from_slice(bytes).ok().map(Self)
    }
}

impl fmt::Debug for Macro {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.ops()).finish()
    }
}

impl Serialize for Macro {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.ops().count()))?;
        self.ops().try_for_each(|op| seq.serialize_element(&op))?;
        seq.end()
    }
}

impl<'de> Deserialize<'de> for Macro {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Ops;

        impl<'de> Visitor<'de> for Ops {
            type Value = Macro;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "macro ops taking at most {MACRO_SPACE_SIZE} bytes")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Macro, A::Error> {
                let mut macro_ops = Macro::new();
                while let Some(op) = seq.next_element()? {
                    macro_ops
                        .push(op)
                        .map_err(|_| de::Error::invalid_length(macro_ops.0.len(), &self))?;
                }
                Ok(macro_ops)
            }
        }

        deserializer.deserialize_seq(Ops)
    }
}

impl MaxSize for Macro {
    // Every op takes at least a byte, so the op count never passes the byte count.
    const POSTCARD_MAX_SIZE: usize = crate::varint_max_size(MACRO_SPACE_SIZE) + MACRO_SPACE_SIZE;
}

/// The most bytes `ops` take of the firmware's macro buffer, in Vial's encoding:
/// each op's bytes, then a terminator.
#[cfg(feature = "vial")]
const fn macro_size(ops: &[MacroOp]) -> usize {
    let (mut len, mut i) = (1, 0);
    while i < ops.len() {
        len += match ops[i] {
            MacroOp::Char(_) => 1,
            // Vial's encoding has no pause.
            MacroOp::PauseForRelease => 0,
            // Vial splits a delay past its two-byte range in two.
            MacroOp::Delay(ms) if ms > 254 * 255 + 254 => 8,
            MacroOp::Delay(_) => 4,
            MacroOp::Tap(action) | MacroOp::Press(action) | MacroOp::Release(action) => match action {
                Action::Key(_) => 3,
                Action::KeyWithModifier(_, modifiers) if modifiers.into_packed_bits() == 0 => 3,
                _ => 4,
            },
        };
        i += 1;
    }
    len
}

/// The most bytes `ops` take of the firmware's macro buffer, in postcard: the
/// ops, behind their length.
#[cfg(not(feature = "vial"))]
const fn macro_size(ops: &[MacroOp]) -> usize {
    let (mut len, mut i) = (0, 0);
    while i < ops.len() {
        // The size of MacroOp's postcard tag(1) + its payload's size
        len += 1 + match ops[i] {
            MacroOp::Char(_) => 1,
            MacroOp::PauseForRelease => 0,
            MacroOp::Delay(ms) => crate::varint_max_size(ms as usize),
            _ => Action::POSTCARD_MAX_SIZE,
        };
        i += 1;
    }
    len + crate::varint_max_size(len)
}

/// Whether a default macro table can be compiled in: at most `MACRO_MAX_NUM`
/// macros, each pausing for release at most once and typing only ASCII, and all
/// of them, with the unused slots, fitting the firmware's macro buffer.
pub const fn validate_default_macros(macros: &[&[MacroOp]]) -> bool {
    if macros.len() > MACRO_MAX_NUM {
        return false;
    }
    let (mut total, mut m) = ((MACRO_MAX_NUM - macros.len()) * macro_size(&[]), 0);
    while m < macros.len() {
        let (ops, mut paused, mut i) = (macros[m], false, 0);
        while i < ops.len() {
            match ops[i] {
                MacroOp::Char(c) if !c.is_ascii() => return false,
                MacroOp::PauseForRelease if paused => return false,
                MacroOp::PauseForRelease => paused = true,
                _ => {}
            }
            i += 1;
        }
        total += macro_size(ops);
        m += 1;
    }
    total <= MACRO_SPACE_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keycode::{HidKeyCode, KeyCode};
    use crate::modifier::ModifierCombination;

    const A: MacroOp = MacroOp::Tap(Action::Key(KeyCode::Hid(HidKeyCode::A)));

    /// The longest text one slot holds: one byte a character with `vial`, two
    /// otherwise, plus the slot's framing.
    const LONGEST_TEXT: usize = if cfg!(feature = "vial") {
        MACRO_SPACE_SIZE - 1
    } else {
        (MACRO_SPACE_SIZE - 2) / 2
    };

    #[test]
    fn every_rule_of_validate_default_macros() {
        assert!(validate_default_macros(&[]));
        assert!(validate_default_macros(&[&[A], &[]]));
        assert!(validate_default_macros(&[&[
            MacroOp::Press(Action::KeyWithModifier(HidKeyCode::A, ModifierCombination::LCTRL)),
            MacroOp::Delay(u16::MAX),
            MacroOp::Char(b'z'),
            MacroOp::PauseForRelease,
            MacroOp::Release(Action::LayerOn(1)),
            MacroOp::Tap(Action::TriggerMacro(0)),
        ]]));
        assert!(!validate_default_macros(&[&[A], &[MacroOp::Char(0xff)]]));
        assert!(!validate_default_macros(&[&[
            MacroOp::PauseForRelease,
            A,
            MacroOp::PauseForRelease
        ]]));
        const EMPTY: &[MacroOp] = &[];
        assert!(!validate_default_macros(&[EMPTY; MACRO_MAX_NUM + 1]));
        assert!(validate_default_macros(&[EMPTY; MACRO_MAX_NUM]));

        // Every slot costs its framing, so a text that fits alone doesn't fit beside the empty slots.
        assert!(!validate_default_macros(&[&[MacroOp::Char(b'a'); LONGEST_TEXT]]));
        assert!(validate_default_macros(&[
            &[MacroOp::Char(b'a'); LONGEST_TEXT + 1 - MACRO_MAX_NUM]
        ]));
    }

    // The const fns are what `const _: () = assert!(..)` evaluates at compile time.
    const _: () = assert!(validate_default_macros(&[&[
        A,
        MacroOp::Char(b'a'),
        MacroOp::PauseForRelease
    ]]));

    /// Without `vial` the buffer holds postcard: a one-op macro takes at least
    /// the op's bytes behind a one-byte length.
    #[cfg(not(feature = "vial"))]
    #[test]
    fn macro_size_bounds_every_op_shape() {
        let ops = [
            A,
            MacroOp::Press(Action::Key(KeyCode::Hid(HidKeyCode::LShift))),
            MacroOp::Release(Action::KeyWithModifier(HidKeyCode::RGui, ModifierCombination::RGUI)),
            MacroOp::Delay(0),
            MacroOp::Delay(127),
            MacroOp::Delay(128),
            MacroOp::Delay(u16::MAX),
            MacroOp::Char(b'a'),
            MacroOp::PauseForRelease,
        ];
        for op in ops {
            let mut buf = [0u8; MacroOp::POSTCARD_MAX_SIZE];
            let len = postcard::to_slice(&op, &mut buf).unwrap().len();
            assert!(len < macro_size(&[op]), "{op:?}");
        }
        assert_eq!(MacroOp::POSTCARD_MAX_SIZE, 5, "one op is at most five wire bytes");
    }

    #[test]
    fn a_macro_encodes_exactly_like_its_op_list() {
        let ops = [A, MacroOp::Delay(300), MacroOp::Char(b'a'), MacroOp::PauseForRelease];
        let macro_ops = Macro::from_slice(&ops).unwrap();
        assert!(macro_ops.ops().eq(ops));

        let mut raw = [0u8; 32];
        let raw = postcard::to_slice(&heapless::Vec::<MacroOp, 8>::from_slice(&ops).unwrap(), &mut raw).unwrap();
        let mut wrapped = [0u8; 32];
        assert_eq!(postcard::to_slice(&macro_ops, &mut wrapped).unwrap(), raw);
        assert_eq!(raw, [4, 0, 1, 0, 4, 3, 172, 2, 4, 97, 5]);
        // The count prefix aside, the wire bytes are the stored ones.
        assert_eq!(macro_ops.as_bytes(), &raw[1..]);
        assert_eq!(postcard::from_bytes::<Macro>(raw).unwrap(), macro_ops);
        assert_eq!(Macro::from_bytes(macro_ops.as_bytes()), Some(macro_ops));
        // Bytes that aren't whole ops make no macro.
        assert_eq!(Macro::from_bytes(&[3]), None, "a truncated op");
        assert_eq!(Macro::from_bytes(&[4, 97, 0xff]), None, "a byte past the last op");
    }

    #[test]
    fn a_full_macro_round_trips_within_its_bound_and_refuses_more() {
        let mut full = Macro::new();
        while full.push(MacroOp::Char(b'a')).is_ok() {}
        assert_eq!(full.ops().count(), MACRO_SPACE_SIZE / 2);
        let mut buf = [0u8; 2 * Macro::POSTCARD_MAX_SIZE];
        let bytes = postcard::to_slice(&full, &mut buf).unwrap();
        assert!(bytes.len() <= Macro::POSTCARD_MAX_SIZE);
        assert_eq!(postcard::from_bytes::<Macro>(bytes).unwrap(), full);

        // One op more than fits never decodes.
        let too_long = [MacroOp::Char(b'a'); MACRO_SPACE_SIZE / 2 + 1];
        let bytes = postcard::to_slice(
            &heapless::Vec::<MacroOp, { MACRO_SPACE_SIZE }>::from_slice(&too_long).unwrap(),
            &mut buf,
        )
        .unwrap();
        assert!(postcard::from_bytes::<Macro>(bytes).is_err());
    }
}
