//! Keycode definitions including HID keycodes, media keys, and system control keycodes.

mod ascii;
mod consumer;
mod hid;
mod system_control;

pub use ascii::{from_ascii, to_ascii};
pub use consumer::ConsumerKey;
pub use hid::HidKeyCode;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};
pub use system_control::SystemControlKey;

/// Key codes which are not in the HID spec, but still commonly used
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "_codegen", derive(strum::VariantNames))]
#[non_exhaustive]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum SpecialKey {
    /// Sends `Esc`, or `` ` `` when any modifier is held
    GraveEscape,
    /// Repeats the last pressed key
    Repeat,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum KeyCode {
    Hid(HidKeyCode),
    Consumer(ConsumerKey),
    SystemControl(SystemControlKey),
}

impl KeyCode {
    pub fn is_basic_keyboard_key(&self) -> bool {
        matches!(self,
            KeyCode::Hid(hid) if hid.process_as_consumer().is_none()
                && hid.process_as_system_control().is_none()
                && !hid.is_mouse_key()
        )
    }
}
