//! Keyboard control actions.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

/// Actions for controlling the keyboard or changing the keyboard's state, for example, enable/disable a particular function
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "_codegen", derive(strum::VariantNames))]
#[non_exhaustive]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum KeyboardAction {
    Bootloader,
    Reboot,
    DebugToggle,
    ClearEeprom,
    OutputAuto,
    OutputUsb,
    OutputBluetooth,
    ComboOn,
    ComboOff,
    ComboToggle,
    CapsWordToggle,
    AutoShiftOn,
    AutoShiftOff,
    AutoShiftToggle,
}
