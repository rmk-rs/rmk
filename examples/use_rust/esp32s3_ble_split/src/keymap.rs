use rmk::types::action::KeyAction;
use rmk::{a, k, layer, macros, mo, td, to};
pub(crate) const TOT_COL: usize = 14;
pub(crate) const TOT_ROW: usize = 6;
pub(crate) const NUM_LAYER: usize = 2;
pub(crate) const PERIPHERAL_ROWS: usize = 6;
pub(crate) const PERIPHERAL_COLS: usize = 7;
pub(crate) const CENTRAL_ROWS: usize = 6;
pub(crate) const CENTRAL_COLS: usize = 7;

#[rustfmt::skip]
pub const fn get_default_keymap() -> [[[KeyAction; TOT_COL]; TOT_ROW]; NUM_LAYER] {
    [
        layer!([
            [k!(F1), k!(F2), k!(F3), k!(F4), k!(F5), k!(F6), k!(LCtrl), k!(Space), k!(F7), k!(F8), k!(F9), k!(F10), k!(F11), k!(F12)],
            [td!(5), td!(6), k!(Kc1), k!(Kc2), k!(Kc3), k!(Kc4), k!(LAlt), k!(Backspace), k!(Kc5), k!(Kc6), k!(Kc7), k!(Kc8), k!(Kc9), k!(Kc0)],
            [k!(International1), td!(0), k!(Q), k!(W), k!(F), k!(P), k!(Space), k!(Escape), k!(U), k!(L), k!(J), k!(G), td!(1), k!(K)],
            [k!(NonusBackslash), k!(Tab), k!(A), k!(R), k!(S), k!(T), mo!(1), td!(9), k!(M), k!(N), k!(I), k!(E), k!(Semicolon), k!(Y)],
            [k!(CapsLock), td!(7), k!(Z), k!(X), k!(C), k!(D), k!(LGui), k!(Delete), k!(V), k!(H), k!(B), k!(O), k!(Slash), to!(1)],
            [a!(No), a!(No), td!(4), k!(Equal), k!(LShift), a!(No), a!(No), a!(No), a!(No), k!(Enter), td!(2), td!(3), a!(No), a!(No)]
        ]),
        layer!([
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), k!(Space), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), k!(Kc1), k!(Kc2), k!(Kc3), k!(Kc4), k!(LShift), a!(No), k!(Kc5), k!(Kc6), k!(Kc7), k!(Kc8), k!(Kc9), k!(Kp0)],
            [td!(8), k!(Comma), k!(Q), k!(W), k!(E), k!(R), k!(LGui), k!(Escape), k!(T), k!(Y), k!(U), k!(I), k!(O), k!(P)],
            [k!(LeftBracket), k!(LShift), k!(A), k!(S), k!(D), k!(F), a!(No), k!(Down), k!(G), k!(H), k!(J), k!(K), k!(L), a!(No)],
            [k!(Escape), k!(Tab), k!(Z), k!(X), k!(C), k!(V), k!(LAlt), k!(Up), k!(B), k!(N), k!(M), k!(Up), a!(No), to!(0)],
            [a!(No), a!(No), macros!(0), macros!(1), k!(LCtrl), a!(No), a!(No), a!(No), a!(No), a!(No), k!(Left), k!(Right), a!(No), a!(No)]
        ])
    ]
}
