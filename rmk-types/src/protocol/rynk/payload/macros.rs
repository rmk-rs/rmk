//! Macro endpoint types. `GetMacro` takes the macro index and answers with the
//! whole [`Macro`](crate::keyboard_macros::Macro).

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use crate::keyboard_macros::Macro;

/// Request payload for `SetMacro`: replace macro `index` with `macro_ops`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct SetMacroRequest {
    pub index: u8,
    #[cfg_attr(feature = "wasm", tsify(type = "MacroOp[]"))]
    pub macro_ops: Macro,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::keyboard_macros::MacroOp;
    use crate::keycode::{HidKeyCode, KeyCode};
    use crate::protocol::rynk::tests::{assert_max_size_bound, round_trip};

    #[test]
    fn round_trip_set_macro_request() {
        round_trip(&SetMacroRequest {
            index: 0,
            macro_ops: Macro::default(),
        });
        let macro_ops = Macro::from_slice(&[
            MacroOp::Press(Action::Key(KeyCode::Hid(HidKeyCode::LShift))),
            MacroOp::Char(b'a'),
            MacroOp::Delay(500),
            MacroOp::PauseForRelease,
            MacroOp::Release(Action::Key(KeyCode::Hid(HidKeyCode::LShift))),
        ])
        .unwrap();
        round_trip(&SetMacroRequest { index: 3, macro_ops });

        // Max capacity, every op at its widest encoding.
        let mut macro_ops = Macro::new();
        while macro_ops.push(MacroOp::Delay(u16::MAX)).is_ok() {}
        let full = SetMacroRequest {
            index: u8::MAX,
            macro_ops,
        };
        round_trip(&full);
        assert_max_size_bound(&full);
    }
}
