//! Sticky key profiles, shared by `SK`, `OSM` and `OSL`.

use bitfield_struct::bitfield;
use postcard::experimental::max_size::MaxSize;

#[cfg(not(feature = "host"))]
type IgnoreKeys = heapless::Vec<KeyCode, STICKY_IGNORE_MAX>;
#[cfg(feature = "host")]
type IgnoreKeys = alloc::vec::Vec<KeyCode>;

#[cfg(not(feature = "host"))]
use crate::constants::STICKY_IGNORE_MAX;
use crate::keycode::KeyCode;

/// Release conditions after source release; the first match wins.
/// Waiting timeout, cancellation and hold behavior apply separately.
#[bitfield(u8, order = Lsb)]
#[derive(PartialEq, Eq, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct StickyReleaseConditions {
    /// End before the next nonignored input action executes.
    #[bits(1)]
    pub before_next_press: bool,
    /// End after that press executes, while the consuming key is still held.
    #[bits(1)]
    pub after_next_press: bool,
    /// End when the consuming input releases; unrelated releases do not end it.
    #[bits(1)]
    pub after_next_release: bool,
    /// End when a layer becomes active, including through a default-layer change.
    #[bits(1)]
    pub layer_activate: bool,
    /// End when a layer becomes inactive, including through a default-layer change.
    #[bits(1)]
    pub layer_deactivate: bool,
    #[bits(3)]
    __: u8,
}

crate::bitfield_named_serde!(StickyReleaseConditions, "StickyReleaseConditions", {
    before_next_press = with_before_next_press,
    after_next_press = with_after_next_press,
    after_next_release = with_after_next_release,
    layer_activate = with_layer_activate,
    layer_deactivate = with_layer_deactivate,
});

/// One profile referenced by [`crate::action::KeyAction::Sticky`].
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct StickyProfile {
    /// Input/layer release conditions. An empty set disables these conditions.
    pub release_on: StickyReleaseConditions,
    /// Inputs excluded from consumption and hold detection; presses restart the waiting timer.
    /// A combined action is ignored only if every key/modifier is listed.
    #[cfg_attr(feature = "wasm", tsify(type = "KeyCode[]"))]
    #[cfg_attr(all(feature = "defmt", feature = "host"), defmt(Debug2Format))]
    pub ignore: IgnoreKeys,
    /// Milliseconds to wait after source release; zero disables expiry.
    /// Stops when `after_next_release` claims an input.
    pub wait_timeout_ms: u16,
    /// Hold duration in milliseconds that makes source release end the action.
    /// Zero disables this duration check; using a nonignored input still ends it on source release.
    pub hold_timeout_ms: u16,
}

impl Default for StickyProfile {
    fn default() -> Self {
        Self {
            release_on: StickyReleaseConditions::new().with_after_next_release(true),
            ignore: IgnoreKeys::new(),
            wait_timeout_ms: crate::constants::DEFAULT_STICKY_WAIT_TIMEOUT_MS,
            hold_timeout_ms: crate::constants::DEFAULT_STICKY_HOLD_TIMEOUT_MS,
        }
    }
}

/// Default profile shared by `SK`, `OSM` and `OSL`.
pub const STICKY_PROFILE_DEFAULT: u8 = u8::MAX;

#[cfg(not(feature = "host"))]
impl MaxSize for StickyProfile {
    const POSTCARD_MAX_SIZE: usize = StickyReleaseConditions::POSTCARD_MAX_SIZE
        + crate::heapless_vec_max_size::<KeyCode, STICKY_IGNORE_MAX>()
        + 2 * u16::POSTCARD_MAX_SIZE;
}
