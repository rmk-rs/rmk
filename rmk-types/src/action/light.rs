//! Light control actions.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

/// Actions for controlling lights
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "_codegen", derive(strum::VariantNames))]
#[non_exhaustive]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum LightAction {
    BacklightOn,
    BacklightOff,
    BacklightToggle,
    BacklightDown,
    BacklightUp,
    BacklightStep,
    BacklightToggleBreathing,
    RgbTog,
    RgbModeForward,
    RgbModeReverse,
    RgbHui,
    RgbHud,
    RgbSai,
    RgbSad,
    RgbVai,
    RgbVad,
    RgbSpi,
    RgbSpd,
    RgbModePlain,
    RgbModeBreathe,
    RgbModeRainbow,
    RgbModeSwirl,
    RgbModeSnake,
    RgbModeKnight,
    RgbModeXmas,
    RgbModeGradient,
    // Not in vial
    RgbModeRgbtest,
    RgbModeTwinkle,
    /// Cycle standard lighting through always-on, always-off, and powered-only.
    OutputModeCycle,
}
