use rmk::types::action::KeyAction;
use rmk::{a, k, layer, mo, user};

pub(crate) const COL: usize = 15;
pub(crate) const ROW: usize = 5;
pub(crate) const NUM_LAYER: usize = 2;

/// The whole 5x15 keymap: columns 0..7 are the left half (the central), columns
/// 7..15 are the right half (the peripheral).
#[rustfmt::skip]
pub const fn get_default_keymap() -> [[[KeyAction; COL]; ROW]; NUM_LAYER] {
    [
        layer!([
            [k!(Escape),   k!(Kc1),  k!(Kc2),  k!(Kc3),  k!(Kc4),   k!(Kc5),  k!(Kc6), k!(Kc7),   k!(Kc8),  k!(Kc9),   k!(Kc0), k!(Minus),     k!(Equal),       k!(Backspace),    a!(No)],
            [k!(Tab),      k!(Q),    k!(W),    k!(E),    k!(R),     k!(T),    a!(No),  k!(Y),     k!(U),    k!(I),     k!(O),   k!(P),         k!(LeftBracket), k!(RightBracket), k!(Backslash)],
            [k!(CapsLock), k!(A),    k!(S),    k!(D),    k!(F),     k!(G),    a!(No),  k!(H),     k!(J),    k!(K),     k!(L),   k!(Semicolon), k!(Quote),       a!(No),           k!(Enter)],
            [k!(LShift),   k!(Z),    k!(X),    k!(C),    k!(V),     k!(B),    a!(No),  k!(N),     k!(M),    k!(Comma), k!(Dot), k!(Slash),     k!(Up),          a!(No),           mo!(1)],
            [k!(LCtrl),    k!(LGui), k!(LAlt), a!(No),   k!(Space), a!(No),   a!(No),  k!(Space), a!(No),   a!(No),    a!(No),  a!(No),        k!(Left),        k!(Down),         k!(Right)]
        ]),
        layer!([
            [k!(Grave),    k!(F1),   k!(F2),   k!(F3),   k!(F4),    k!(F5),   k!(F6),  k!(F7),    k!(F8),   k!(F9),    k!(F10), k!(F11),       k!(F12),         k!(Delete),       a!(No)],
            [a!(No),       user!(0), user!(1), user!(2), user!(3),  user!(4), a!(No),  user!(5),  user!(6), user!(7),  a!(No),  a!(No),        a!(No),          a!(No),           a!(No)],
            [a!(No),       a!(No),   a!(No),   a!(No),   a!(No),    a!(No),   a!(No),  a!(No),    a!(No),   a!(No),    a!(No),  a!(No),        a!(No),          a!(No),           a!(No)],
            [a!(No),       a!(No),   a!(No),   a!(No),   a!(No),    a!(No),   a!(No),  a!(No),    a!(No),   a!(No),    a!(No),  a!(No),        a!(No),          a!(No),           a!(No)],
            [a!(No),       a!(No),   a!(No),   a!(No),   a!(No),    a!(No),   a!(No),  a!(No),    a!(No),   a!(No),    a!(No),  a!(No),        a!(No),          a!(No),           a!(No)]
        ]),
    ]
}
