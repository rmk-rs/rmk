//! Morse key types shared between firmware and protocol layers.
//!
//! This module contains all morse-related types:
//! - [`MorseMode`] / [`MorseProfile`] — timing and behavior configuration
//! - [`MorsePattern`] — tap/hold pattern encoding (up to 15 steps in a u16)
//! - [`Morse`] — full morse key definition (profile + pattern→action map)

use heapless::LinearMap;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use crate::action::KeyAction;
use crate::constants::MORSE_SIZE;

// ---------------------------------------------------------------------------
// MorseMode & MorseProfile — timing/behavior configuration
// ---------------------------------------------------------------------------

/// Mode for morse key behavior
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum MorseMode {
    /// Same as QMK's permissive hold: <https://docs.qmk.fm/tap_hold#tap-or-hold-decision-modes>
    /// When another key is pressed and released during the current morse key is held,
    /// the hold action of current morse key will be triggered
    PermissiveHold,
    /// Trigger hold immediately if any other non-morse key is pressed when the current morse key is held
    HoldOnOtherPress,
    /// Normal mode, the decision is made when timeout
    Normal,
}

/// Configuration for morse, tap dance and tap-hold.
/// Manually packed into 64 bits to save RAM.
///
/// Bit layout of the inner `u64`:
/// ```text
/// 63        46 | 45      | 44           32 | 31  30 | 29       17 | 16  15 | 14  13   | 12       0
/// reserved     | qt_set  | quick_tap_tm    | mode   | gap_timeout | uni_tap| flow_tap | hold_timeout
///   (18b)      |  (1b)   |   (13b ms)      |  (2b)  |   (13b ms)  |  (2b)  |   (2b)   |  (13b ms)
/// ```
///
/// - `qt_set` (bit 45): when set, `quick_tap_timeout` is explicitly configured
///   (even if 0, which means "disabled"). When clear, the field is unset and
///   callers should fall back to the global default.
/// - `quick_tap_timeout` (bits 44-32): quick-tap timeout in ms (max 8191).
/// - `mode` (bits 31-30): `00` = None, `01` = PermissiveHold, `10` = HoldOnOtherPress, `11` = Normal
/// - `flow_tap` (bits 14, 13): `00`/`01` = None, `10` = Some(false), `11` = Some(true)
/// - `gap_timeout` (bits 29-17): gap timeout in ms (0 = None, max 8191)
/// - `uni_tap` (bits 16-15): `00`/`01` = None, `10` = Some(false), `11` = Some(true)
/// - `hold_timeout` (bits 12-0): hold timeout in ms (0 = None, max 8191)
#[derive(PartialEq, Eq, Clone, Copy, Debug, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MorseProfile(u64);

const TIMEOUT_MASK: u64 = 0x1FFF;
const TIMEOUT_MAX_MS: u16 = TIMEOUT_MASK as u16;
const GAP_TIMEOUT_SHIFT: u32 = 17;
const HOLD_TIMEOUT_MASK: u64 = TIMEOUT_MASK;
const GAP_TIMEOUT_MASK: u64 = TIMEOUT_MASK << GAP_TIMEOUT_SHIFT;
const UNI_TAP_LOW_BIT: u64 = 0x0000_8000;
const UNI_TAP_HIGH_BIT: u64 = 0x0001_0000;
const UNI_TAP_MASK: u64 = UNI_TAP_LOW_BIT | UNI_TAP_HIGH_BIT;
const FLOW_TAP_LOW_BIT: u64 = 0x0000_2000;
const FLOW_TAP_HIGH_BIT: u64 = 0x0000_4000;
const FLOW_TAP_MASK: u64 = FLOW_TAP_LOW_BIT | FLOW_TAP_HIGH_BIT;
const MODE_MASK: u64 = 0xC000_0000;
const QT_VALUE_MASK: u64 = TIMEOUT_MASK << 32;
const QT_SET_BIT: u64 = 1 << 45;

const fn encode_timeout_ms(t: u16) -> u64 {
    if t > TIMEOUT_MAX_MS {
        TIMEOUT_MAX_MS as u64
    } else {
        t as u64
    }
}

impl MorseProfile {
    pub const fn const_default() -> Self {
        Self(0)
    }

    /// If the previous key is on the same "hand", the current key will be determined as a tap
    pub fn unilateral_tap(self) -> Option<bool> {
        match (self.0 & UNI_TAP_MASK) >> 15 {
            3 => Some(true),
            2 => Some(false),
            _ => None,
        }
    }

    pub const fn with_unilateral_tap(self, b: Option<bool>) -> Self {
        Self(
            (self.0 & !UNI_TAP_MASK)
                | match b {
                    Some(true) => UNI_TAP_MASK,
                    Some(false) => UNI_TAP_HIGH_BIT,
                    None => 0,
                },
        )
    }

    /// Per-profile override for flow tap. `None` falls back to the default
    /// profile's bit, then the config-level `enable_flow_tap` switch.
    pub fn enable_flow_tap(self) -> Option<bool> {
        match (self.0 & FLOW_TAP_MASK) >> 13 {
            3 => Some(true),
            2 => Some(false),
            _ => None,
        }
    }

    pub const fn with_enable_flow_tap(self, b: Option<bool>) -> Self {
        Self(
            (self.0 & !FLOW_TAP_MASK)
                | match b {
                    Some(true) => FLOW_TAP_MASK,
                    Some(false) => FLOW_TAP_HIGH_BIT,
                    None => 0,
                },
        )
    }

    /// The decision mode of the morse/tap-hold key
    pub fn mode(self) -> Option<MorseMode> {
        match self.0 & MODE_MASK {
            MODE_MASK => Some(MorseMode::Normal),
            0x8000_0000 => Some(MorseMode::HoldOnOtherPress),
            0x4000_0000 => Some(MorseMode::PermissiveHold),
            _ => None,
        }
    }

    pub const fn with_mode(self, m: Option<MorseMode>) -> Self {
        Self(
            (self.0 & !MODE_MASK)
                | match m {
                    Some(MorseMode::Normal) => MODE_MASK,
                    Some(MorseMode::HoldOnOtherPress) => 0x8000_0000,
                    Some(MorseMode::PermissiveHold) => 0x4000_0000,
                    None => 0,
                },
        )
    }

    /// If the key is pressed longer than this, it is accepted as `hold` (in milliseconds)
    pub fn hold_timeout_ms(self) -> Option<u16> {
        let t = (self.0 & HOLD_TIMEOUT_MASK) as u16;
        if t == 0 { None } else { Some(t) }
    }

    pub const fn with_hold_timeout_ms(self, t: Option<u16>) -> Self {
        if let Some(t) = t {
            Self((self.0 & !HOLD_TIMEOUT_MASK) | encode_timeout_ms(t))
        } else {
            Self(self.0 & !HOLD_TIMEOUT_MASK)
        }
    }

    pub const fn set_hold_timeout_ms(&mut self, t: u16) {
        self.0 = (self.0 & !HOLD_TIMEOUT_MASK) | encode_timeout_ms(t)
    }

    pub const fn set_gap_timeout_ms(&mut self, t: u16) {
        self.0 = (self.0 & !GAP_TIMEOUT_MASK) | (encode_timeout_ms(t) << GAP_TIMEOUT_SHIFT)
    }

    /// The time elapsed from the last release of a key is longer than this, it will break the morse pattern (in milliseconds)
    pub fn gap_timeout_ms(self) -> Option<u16> {
        let t = ((self.0 & GAP_TIMEOUT_MASK) >> GAP_TIMEOUT_SHIFT) as u16;
        if t == 0 { None } else { Some(t) }
    }

    pub const fn with_gap_timeout_ms(self, t: Option<u16>) -> Self {
        if let Some(t) = t {
            Self((self.0 & !GAP_TIMEOUT_MASK) | (encode_timeout_ms(t) << GAP_TIMEOUT_SHIFT))
        } else {
            Self(self.0 & !GAP_TIMEOUT_MASK)
        }
    }

    pub const fn quick_tap_timeout_ms(self) -> Option<u16> {
        if self.0 & QT_SET_BIT != 0 {
            Some(((self.0 >> 32) & TIMEOUT_MASK) as u16)
        } else {
            None
        }
    }

    pub const fn with_quick_tap_timeout_ms(self, t: Option<u16>) -> Self {
        if let Some(t) = t {
            Self((self.0 & !(QT_VALUE_MASK | QT_SET_BIT)) | (encode_timeout_ms(t) << 32) | QT_SET_BIT)
        } else {
            Self(self.0 & !(QT_VALUE_MASK | QT_SET_BIT))
        }
    }

    pub const fn set_quick_tap_timeout_ms(&mut self, t: u16) {
        self.0 = (self.0 & !(QT_VALUE_MASK | QT_SET_BIT)) | (encode_timeout_ms(t) << 32) | QT_SET_BIT;
    }

    pub const fn new(
        unilateral_tap: Option<bool>,
        mode: Option<MorseMode>,
        hold_timeout_ms: Option<u16>,
        gap_timeout_ms: Option<u16>,
    ) -> Self {
        let mut v = 0u64;
        if let Some(t) = hold_timeout_ms {
            v = encode_timeout_ms(t);
        }
        if let Some(t) = gap_timeout_ms {
            v |= encode_timeout_ms(t) << GAP_TIMEOUT_SHIFT;
        }
        if let Some(b) = unilateral_tap {
            v |= if b { UNI_TAP_MASK } else { UNI_TAP_HIGH_BIT };
        }
        if let Some(m) = mode {
            v |= match m {
                MorseMode::Normal => MODE_MASK,
                MorseMode::HoldOnOtherPress => 0x8000_0000,
                MorseMode::PermissiveHold => 0x4000_0000,
            };
        }
        MorseProfile(v)
    }
}

impl Default for MorseProfile {
    fn default() -> Self {
        MorseProfile::const_default()
    }
}

impl From<u64> for MorseProfile {
    fn from(v: u64) -> Self {
        MorseProfile(v)
    }
}

impl From<MorseProfile> for u64 {
    fn from(val: MorseProfile) -> Self {
        val.0
    }
}

// Wire stays packed; human-readable serializers expose named fields.
impl Serialize for MorseProfile {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            #[derive(Serialize)]
            struct Repr {
                unilateral_tap: Option<bool>,
                enable_flow_tap: Option<bool>,
                mode: Option<MorseMode>,
                hold_timeout_ms: Option<u16>,
                gap_timeout_ms: Option<u16>,
                quick_tap_timeout_ms: Option<u16>,
            }
            Repr {
                unilateral_tap: self.unilateral_tap(),
                enable_flow_tap: self.enable_flow_tap(),
                mode: self.mode(),
                hold_timeout_ms: self.hold_timeout_ms(),
                gap_timeout_ms: self.gap_timeout_ms(),
                quick_tap_timeout_ms: self.quick_tap_timeout_ms(),
            }
            .serialize(serializer)
        } else {
            serializer.serialize_u64(self.0)
        }
    }
}

impl<'de> Deserialize<'de> for MorseProfile {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            #[derive(Deserialize)]
            struct Repr {
                unilateral_tap: Option<bool>,
                enable_flow_tap: Option<bool>,
                mode: Option<MorseMode>,
                hold_timeout_ms: Option<u16>,
                gap_timeout_ms: Option<u16>,
                quick_tap_timeout_ms: Option<u16>,
            }
            let r = Repr::deserialize(deserializer)?;
            Ok(
                MorseProfile::new(r.unilateral_tap, r.mode, r.hold_timeout_ms, r.gap_timeout_ms)
                    .with_enable_flow_tap(r.enable_flow_tap)
                    .with_quick_tap_timeout_ms(r.quick_tap_timeout_ms),
            )
        } else {
            Ok(MorseProfile(u64::deserialize(deserializer)?))
        }
    }
}

// TS shape mirrors the `Repr` above; `Option<T>` renders as `T | undefined`.
#[cfg(feature = "wasm")]
const _: () = {
    #[::wasm_bindgen::prelude::wasm_bindgen(typescript_custom_section)]
    const TS_APPEND_CONTENT: &'static str = "export type MorseProfile = { unilateral_tap: boolean | undefined; enable_flow_tap: boolean | undefined; mode: MorseMode | undefined; hold_timeout_ms: number | undefined; gap_timeout_ms: number | undefined; quick_tap_timeout_ms: number | undefined; };";
};
crate::wasm_object_abi!(MorseProfile, "MorseProfile");

// ---------------------------------------------------------------------------
// MorsePattern & Morse — pattern encoding and key definition
// ---------------------------------------------------------------------------

/// MorsePattern is a sequence of maximum 15 taps or holds that can be encoded into an u16:
/// 0x1 when empty, then 0 for tap or 1 for hold shifted from the right
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct MorsePattern(u16);

pub const TAP: MorsePattern = MorsePattern(0b10);
pub const HOLD: MorsePattern = MorsePattern(0b11);
pub const DOUBLE_TAP: MorsePattern = MorsePattern(0b100);
pub const HOLD_AFTER_TAP: MorsePattern = MorsePattern(0b101);

impl Default for MorsePattern {
    fn default() -> Self {
        MorsePattern(0b1) // 0b1 means empty
    }
}

impl MorsePattern {
    pub fn max_taps() -> usize {
        15 // 15 taps can be encoded on u16 bits (1 bit used to mark the start position)
    }

    /// Creates a `MorsePattern` from a raw `u16`.
    ///
    /// # Panics (debug only)
    /// Panics if `value` is 0, which is not a valid encoding
    /// (the empty pattern is `0b1`).
    pub fn from_u16(value: u16) -> Self {
        debug_assert!(value != 0, "MorsePattern 0 is invalid; the empty pattern is 0b1");
        MorsePattern(value)
    }

    pub fn to_u16(&self) -> u16 {
        self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0b1
    }

    pub fn is_full(&self) -> bool {
        (self.0 & 0b1000_0000_0000_0000) != 0
    }

    pub fn pattern_length(&self) -> usize {
        // leading_zeros() is 16 for 0, which would underflow.
        // Saturate to 0 for the (invalid) zero case.
        15usize.saturating_sub(self.0.leading_zeros() as usize)
    }

    /// Checks if this pattern starts with the given one
    pub fn starts_with(&self, pattern_start: MorsePattern) -> bool {
        let n = pattern_start.0.leading_zeros();
        let m = self.0.leading_zeros();
        m <= n && (self.0 >> (n - m) == pattern_start.0)
    }

    /// Returns `true` if the last step in the pattern is a hold.
    /// Returns `false` for empty patterns.
    pub fn last_is_hold(&self) -> bool {
        !self.is_empty() && self.0 & 0b1 == 0b1
    }

    pub fn followed_by_tap(&self) -> Self {
        // Shift the bits to the left and set the last bit to 0 (tap)
        MorsePattern(self.0 << 1)
    }

    pub fn followed_by_hold(&self) -> Self {
        // Shift the bits to the left and set the last bit to 1 (hold)
        MorsePattern((self.0 << 1) | 0b1)
    }

    /// `true` when the pattern consists only of tap steps (no holds).
    /// Returns `false` for the empty pattern (encoding `0b1`).
    pub fn is_all_taps(&self) -> bool {
        self.0 > 0b1 && self.0 & (self.0 - 1) == 0
    }
}

/// Definition of a morse key.
///
/// A morse key is a key that behaves differently according to the pattern of a tap/hold sequence.
/// The maximum number of taps is limited to 15 by the internal u16 representation of MorsePattern.
/// There is a list of (pattern, corresponding action) pairs for each morse key:
/// The number of pairs is limited by `MORSE_SIZE` (from `constants.rs`, generated at build time).
///
/// Note: `MORSE_SIZE` is a **wire-format** capacity — on firmware it equals
/// `MAX_PATTERNS_PER_KEY` (from `keyboard.toml`), on host it's a fixed upper bound.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct Morse {
    /// The profile of this morse key, which defines the timing parameters, etc.
    /// If some of its fields are filled with None, the global default value will be used.
    pub profile: MorseProfile,
    /// The list of pattern -> action pairs, which can be triggered
    #[serde(with = "morse_actions_serde")]
    #[cfg_attr(feature = "wasm", tsify(type = "[number, KeyAction][]"))]
    pub actions: LinearMap<MorsePattern, KeyAction, MORSE_SIZE>,
}

impl MaxSize for Morse {
    // The custom serializer in `morse_actions_serde` (below) emits the
    // `LinearMap` as `Vec<(u16, KeyAction), MORSE_SIZE>` on the wire — keep that
    // shape in sync with the helper type parameter here.
    const POSTCARD_MAX_SIZE: usize =
        MorseProfile::POSTCARD_MAX_SIZE + crate::heapless_vec_max_size::<(u16, KeyAction), MORSE_SIZE>();
}

#[cfg(feature = "defmt")]
impl defmt::Format for Morse {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "profile: MorseProfile({:?}), ", self.profile);
        defmt::write!(f, "actions: [");
        for item in self.actions.iter() {
            defmt::write!(f, "{:?},", item);
        }
        defmt::write!(f, "]");
    }
}

impl PartialEq for Morse {
    fn eq(&self, other: &Self) -> bool {
        if self.profile != other.profile || self.actions.len() != other.actions.len() {
            return false;
        }
        self.actions
            .iter()
            .all(|(k, v)| other.actions.get(k).is_some_and(|other| same_output(*v, *other)))
    }
}

impl Eq for Morse {}

fn same_output(left: KeyAction, right: KeyAction) -> bool {
    left == right
        && match (left, right) {
            (KeyAction::Sticky(_, a), KeyAction::Sticky(_, b))
            | (KeyAction::TapHold(_, _, a), KeyAction::TapHold(_, _, b)) => a == b,
            _ => true,
        }
}

/// Wire format note: `Morse` uses a custom serde impl for the `LinearMap`
/// of actions. The on-wire shape is `(MorseProfile, Vec<(u16, KeyAction)>)`.
/// The `morse_wire_format` test below pins this contract.
impl Morse {
    pub fn new_from_vial(
        tap: KeyAction,
        hold: KeyAction,
        hold_after_tap: KeyAction,
        double_tap: KeyAction,
        profile: MorseProfile,
    ) -> Self {
        let mut result = Self {
            profile,
            ..Default::default()
        };

        for (pattern, action) in [
            (TAP, tap),
            (HOLD, hold),
            (DOUBLE_TAP, double_tap),
            (HOLD_AFTER_TAP, hold_after_tap),
        ] {
            let _ = result.put(pattern, action);
        }
        result
    }

    pub fn new_with_actions(
        tap_actions: heapless::Vec<KeyAction, MORSE_SIZE>,
        hold_actions: heapless::Vec<KeyAction, MORSE_SIZE>,
        profile: MorseProfile,
    ) -> Self {
        let mut result = Self {
            profile,
            ..Default::default()
        };

        for (actions, hold) in [(tap_actions, 0), (hold_actions, 1)] {
            let mut pattern = 0b10u16;
            for action in actions {
                let _ = result.put(MorsePattern::from_u16(pattern | hold), action);
                pattern <<= 1;
            }
        }

        result
    }

    pub fn max_pattern_length(&self) -> usize {
        let mut max_length = 0;
        for pair in self.actions.iter() {
            max_length = max_length.max(pair.0.pattern_length());
        }
        max_length
    }

    pub fn try_predict_final_action(&self, pattern_start: MorsePattern) -> Option<KeyAction> {
        if !self.actions.contains_key(&pattern_start) {
            return None;
        }
        for (pattern, _) in self.actions.iter() {
            if *pattern != pattern_start && pattern.starts_with(pattern_start) {
                return None;
            }
        }
        self.actions.get(&pattern_start).copied()
    }

    pub fn can_fire_early(&self, pattern: MorsePattern) -> bool {
        let Some(current_action) = self.actions.get(&pattern) else {
            return false;
        };
        if self.actions.contains_key(&pattern.followed_by_tap()) {
            return false;
        }
        self.actions
            .get(&pattern.followed_by_hold())
            .is_some_and(|a| same_output(*a, *current_action))
    }

    pub fn has_pattern_or_continuation(&self, pattern: MorsePattern) -> bool {
        self.actions.iter().any(|(p, _)| p.starts_with(pattern))
    }

    pub fn get(&self, pattern: MorsePattern) -> Option<KeyAction> {
        self.actions.get(&pattern).copied()
    }

    /// Insert or update an action for the given pattern.
    ///
    /// An empty action removes the pattern. Returns `Err((pattern, action))` if the map is full.
    pub fn put(&mut self, pattern: MorsePattern, action: KeyAction) -> Result<(), (MorsePattern, KeyAction)> {
        if !action.is_empty() {
            self.actions.insert(pattern, action).map(|_| ())
        } else {
            let _ = self.actions.remove(&pattern);
            Ok(())
        }
    }
}

// Custom serde module for LinearMap
mod morse_actions_serde {
    use serde::de::Error;
    use serde::{Deserializer, Serializer};

    use super::*;

    pub fn serialize<S>(map: &LinearMap<MorsePattern, KeyAction, MORSE_SIZE>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Convert to Vec for serialization
        let vec: heapless::Vec<(u16, KeyAction), MORSE_SIZE> = map.iter().map(|(k, v)| (k.to_u16(), *v)).collect();
        vec.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<LinearMap<MorsePattern, KeyAction, MORSE_SIZE>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let vec = heapless::Vec::<(u16, KeyAction), MORSE_SIZE>::deserialize(deserializer)?;
        let mut map = LinearMap::new();
        for (pattern, action) in vec {
            if pattern == 0 {
                return Err(D::Error::custom("MorsePattern 0 is invalid; the empty pattern is 0b1"));
            }
            map.insert(MorsePattern::from_u16(pattern), action)
                .map_err(|_| D::Error::custom("Failed to insert into LinearMap"))?;
        }
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use super::*;
    use crate::action::Action;
    use crate::action::KeyAction::Single;
    use crate::keycode::{HidKeyCode, KeyCode};

    #[test]
    fn morse_round_trips_empty_single_multiple_and_sticky_outputs() {
        let key = |code| Single(Action::Key(KeyCode::Hid(code)));
        let single = [(TAP, key(HidKeyCode::A))];
        let multiple = [
            (TAP, key(HidKeyCode::A)),
            (HOLD, key(HidKeyCode::B)),
            (DOUBLE_TAP, key(HidKeyCode::C)),
            (HOLD_AFTER_TAP, key(HidKeyCode::D)),
        ];
        let sticky = [
            (TAP, KeyAction::Sticky(Action::LayerOn(1), 0)),
            (HOLD_AFTER_TAP, KeyAction::Sticky(Action::LayerOn(1), 1)),
        ];
        let default = MorseProfile::default();
        let custom = MorseProfile::new(Some(true), Some(MorseMode::PermissiveHold), Some(200), Some(150));
        for (profile, actions) in [
            (default, &[] as &[(MorsePattern, KeyAction)]),
            (default, &single),
            (default, &multiple),
            (custom, &multiple),
            (custom, &sticky),
        ] {
            let morse = Morse {
                profile,
                actions: actions.iter().copied().collect(),
            };
            let mut buffer = [0u8; Morse::POSTCARD_MAX_SIZE];
            let bytes = postcard::to_slice(&morse, &mut buffer).unwrap();
            assert_eq!(postcard::from_bytes::<Morse>(bytes).unwrap(), morse);
        }
        let mut morse = Morse {
            actions: sticky.into_iter().collect(),
            ..Morse::default()
        };
        assert!(!morse.can_fire_early(TAP));
        let previous = morse.clone();
        morse.put(HOLD_AFTER_TAP, sticky[0].1).unwrap();
        assert!(morse.can_fire_early(TAP));
        assert_ne!(morse, previous);
    }

    #[test]
    fn morse_pattern_max_size_matches_u16() {
        // Morse actions serialize MorsePattern as u16 on the wire.
        // If MorsePattern's MaxSize ever diverges from u16, the manual
        // MaxSize impl on Morse would be wrong.
        assert_eq!(MorsePattern::POSTCARD_MAX_SIZE, u16::POSTCARD_MAX_SIZE,);
    }

    #[test]
    fn test_morse_profile_timeout_setters() {
        let mut profile = MorseProfile::new(Some(true), Some(MorseMode::PermissiveHold), Some(1000), Some(2000));

        assert_eq!(profile.hold_timeout_ms(), Some(1000));
        assert_eq!(profile.gap_timeout_ms(), Some(2000));
        assert_eq!(profile.unilateral_tap(), Some(true));
        assert_eq!(profile.mode(), Some(MorseMode::PermissiveHold));

        profile.set_hold_timeout_ms(1500);
        assert_eq!(profile.hold_timeout_ms(), Some(1500));
        assert_eq!(profile.gap_timeout_ms(), Some(2000));
        assert_eq!(profile.unilateral_tap(), Some(true));
        assert_eq!(profile.mode(), Some(MorseMode::PermissiveHold));

        profile.set_gap_timeout_ms(2500);
        assert_eq!(profile.hold_timeout_ms(), Some(1500));
        assert_eq!(profile.gap_timeout_ms(), Some(2500));
        assert_eq!(profile.unilateral_tap(), Some(true));
        assert_eq!(profile.mode(), Some(MorseMode::PermissiveHold));

        profile.set_hold_timeout_ms(0xFFFF);
        profile.set_gap_timeout_ms(0xFFFF);
        assert_eq!(profile.hold_timeout_ms(), Some(TIMEOUT_MAX_MS));
        assert_eq!(profile.gap_timeout_ms(), Some(TIMEOUT_MAX_MS));

        profile.set_hold_timeout_ms(0);
        profile.set_gap_timeout_ms(0);
        assert_eq!(profile.hold_timeout_ms(), None);
        assert_eq!(profile.gap_timeout_ms(), None);

        let p = MorseProfile::const_default().with_quick_tap_timeout_ms(Some(300));
        assert_eq!(p.quick_tap_timeout_ms(), Some(300));
        assert_eq!(p.hold_timeout_ms(), MorseProfile::const_default().hold_timeout_ms());
        assert_eq!(p.gap_timeout_ms(), MorseProfile::const_default().gap_timeout_ms());

        let mut p2 = p;
        p2.set_quick_tap_timeout_ms(0);
        assert_eq!(p2.quick_tap_timeout_ms(), Some(0), "set_*_ms(0) is explicit");

        p2.set_quick_tap_timeout_ms(0xFFFF);
        assert_eq!(p2.quick_tap_timeout_ms(), Some(TIMEOUT_MAX_MS));

        let p3 = p.with_quick_tap_timeout_ms(None);
        assert_eq!(p3.quick_tap_timeout_ms(), None, "with_*(None) clears the field");

        let p4 = MorseProfile::const_default().with_quick_tap_timeout_ms(Some(0));
        assert_eq!(p4.quick_tap_timeout_ms(), Some(0), "Some(0) is explicitly disabled");
    }

    #[test]
    fn is_all_taps_encoding_invariant() {
        let tap = MorsePattern::from_u16(0b10);
        let tap_tap = MorsePattern::from_u16(0b100);
        let tap_tap_tap = MorsePattern::from_u16(0b1000);
        let hold = MorsePattern::from_u16(0b11);
        let tap_hold = MorsePattern::from_u16(0b101);
        let hold_tap = MorsePattern::from_u16(0b110);
        let empty = MorsePattern::default();

        assert!(tap.is_all_taps());
        assert!(tap_tap.is_all_taps());
        assert!(tap_tap_tap.is_all_taps());
        assert!(!hold.is_all_taps());
        assert!(!tap_hold.is_all_taps());
        assert!(!hold_tap.is_all_taps());
        assert!(!empty.is_all_taps());

        assert_eq!(tap, MorsePattern::default().followed_by_tap());
        assert_eq!(tap_tap, tap.followed_by_tap());
        assert_eq!(hold, MorsePattern::default().followed_by_hold());
    }

    #[test]
    fn test_morse_profile_packed_layout_matches_docs() {
        let profile = MorseProfile::new(
            Some(false),
            Some(MorseMode::HoldOnOtherPress),
            Some(0x0123),
            Some(0x0456),
        )
        .with_enable_flow_tap(Some(false));
        assert_eq!(
            u64::from(profile),
            0x8000_0000 | (0x0456u64 << 17) | 0x0001_0000 | 0x0000_4000 | 0x0123
        );
        assert_eq!(profile.unilateral_tap(), Some(false));
        assert_eq!(profile.enable_flow_tap(), Some(false));

        let profile = MorseProfile::new(Some(true), Some(MorseMode::Normal), Some(0x0123), Some(0x0456))
            .with_enable_flow_tap(Some(true));
        assert_eq!(
            u64::from(profile),
            0xC000_0000 | (0x0456u64 << 17) | 0x0001_8000 | 0x0000_6000 | 0x0123
        );
        assert_eq!(profile.unilateral_tap(), Some(true));
        assert_eq!(profile.enable_flow_tap(), Some(true));
    }

    #[test]
    fn test_morse_profile_enable_flow_tap_accessors_preserve_packed_fields() {
        assert_eq!(core::mem::size_of::<MorseProfile>(), 8);
        assert_eq!(MorseProfile::POSTCARD_MAX_SIZE, u64::POSTCARD_MAX_SIZE);
        assert_eq!(MorseProfile::const_default().enable_flow_tap(), None);

        let profile = MorseProfile::new(Some(true), Some(MorseMode::PermissiveHold), Some(1000), Some(2000));
        let profile = profile.with_enable_flow_tap(Some(true));
        assert_eq!(profile.enable_flow_tap(), Some(true));
        assert_eq!(profile.hold_timeout_ms(), Some(1000));
        assert_eq!(profile.gap_timeout_ms(), Some(2000));
        assert_eq!(profile.unilateral_tap(), Some(true));
        assert_eq!(profile.mode(), Some(MorseMode::PermissiveHold));

        let profile = profile.with_enable_flow_tap(Some(false));
        assert_eq!(profile.enable_flow_tap(), Some(false));
        assert_eq!(profile.hold_timeout_ms(), Some(1000));
        assert_eq!(profile.gap_timeout_ms(), Some(2000));
        assert_eq!(profile.unilateral_tap(), Some(true));
        assert_eq!(profile.mode(), Some(MorseMode::PermissiveHold));

        let profile = profile.with_enable_flow_tap(None);
        assert_eq!(profile.enable_flow_tap(), None);
        assert_eq!(profile.hold_timeout_ms(), Some(1000));
        assert_eq!(profile.gap_timeout_ms(), Some(2000));
        assert_eq!(profile.unilateral_tap(), Some(true));
        assert_eq!(profile.mode(), Some(MorseMode::PermissiveHold));
    }

    /// The human-readable serde goes `MorseProfile` -> decoded parts -> `new()`.
    /// All 32 bits are covered by the five fields, so that path must be lossless.
    #[test]
    fn morse_profile_parts_roundtrip() {
        for p in [
            MorseProfile::new(
                Some(false),
                Some(MorseMode::HoldOnOtherPress),
                Some(TIMEOUT_MAX_MS),
                Some(1),
            )
            .with_enable_flow_tap(Some(false)),
            MorseProfile::new(Some(true), Some(MorseMode::Normal), Some(200), Some(150))
                .with_enable_flow_tap(Some(true)),
            MorseProfile::const_default(),
        ] {
            let parts = MorseProfile::new(p.unilateral_tap(), p.mode(), p.hold_timeout_ms(), p.gap_timeout_ms())
                .with_enable_flow_tap(p.enable_flow_tap());
            assert_eq!(p, parts);
        }
    }

    /// Pins the on-wire shape of `Morse`:
    ///   `(MorseProfile, Vec<(u16, KeyAction)>)`
    ///
    /// `Morse` uses a custom serde impl for the `LinearMap` of actions; this
    /// test verifies a Morse value can be reconstructed by manually
    /// deserializing those two fields from the same byte stream.
    #[test]
    fn morse_wire_format() {
        use postcard::to_slice;

        // Build a Morse with known data
        let mut morse = Morse::default();
        morse
            .actions
            .insert(MorsePattern::from_u16(0b11), KeyAction::No)
            .unwrap();

        // Serialize the whole Morse
        let mut buf = [0u8; 256];
        let bytes = to_slice(&morse, &mut buf).unwrap();

        // Now manually deserialize field-by-field in the order the Schema declares:
        // 1. profile: MorseProfile (a newtype around u64)
        let (profile, rest): (MorseProfile, &[u8]) =
            postcard::take_from_bytes(bytes).expect("should deserialize MorseProfile first");
        assert_eq!(profile, MorseProfile::const_default());

        // 2. actions: Vec<(u16, KeyAction)> — which is what the custom serde produces
        let (actions, rest): (heapless::Vec<(u16, KeyAction), MORSE_SIZE>, &[u8]) =
            postcard::take_from_bytes(rest).expect("should deserialize actions vec second");
        assert!(rest.is_empty(), "no trailing bytes should remain");

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], (0b11u16, KeyAction::No));
    }
}
