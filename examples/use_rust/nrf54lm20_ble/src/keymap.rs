use rmk::types::action::{EncoderAction, KeyAction};
use rmk::{a, encoder, k, layer, mo};

pub(crate) const COL: usize = 8;
pub(crate) const ROW: usize = 5;
pub(crate) const NUM_LAYER: usize = 2;
pub(crate) const NUM_ENCODER: usize = 1;

#[rustfmt::skip]
pub const fn get_default_keymap() -> [[[KeyAction; COL]; ROW]; NUM_LAYER] {
    [
        layer!([
            [k!(Escape),   k!(Kc1),  k!(Kc2),  k!(Kc3),  k!(Kc4),   k!(Kc5),    k!(Kc6),        k!(Kc7)],
            [k!(Tab),      k!(Q),    k!(W),    k!(E),    k!(R),     k!(T),      k!(Y),          k!(U)],
            [k!(CapsLock), k!(A),    k!(S),    k!(D),    k!(F),     k!(G),      k!(H),          k!(J)],
            [k!(LShift),   k!(Z),    k!(X),    k!(C),    k!(V),     k!(B),      k!(N),          k!(M)],
            [k!(LCtrl),    k!(LGui), k!(LAlt), mo!(1),   k!(Space), k!(Enter),  k!(Backspace),  k!(Delete)]
        ]),
        layer!([
            [k!(Grave),    k!(F1),   k!(F2),   k!(F3),   k!(F4),    k!(F5),     k!(F6),         k!(F7)],
            [a!(Transparent), a!(No), a!(No),  k!(Up),   a!(No),    a!(No),     a!(No),         a!(No)],
            [a!(Transparent), a!(No), k!(Left), k!(Down), k!(Right), a!(No),    a!(No),         a!(No)],
            [a!(Transparent), a!(No), a!(No),  a!(No),   a!(No),    a!(No),     a!(No),         a!(No)],
            [a!(Transparent), a!(Transparent), a!(Transparent), mo!(1), a!(Transparent), k!(AudioMute), k!(KbVolumeDown), k!(KbVolumeUp)]
        ]),
    ]
}

/// The board carries one 4 mm scroll-wheel encoder on `J1`.
#[rustfmt::skip]
pub const fn get_default_encoder_map() -> [[EncoderAction; NUM_ENCODER]; NUM_LAYER] {
    [
        [encoder!(k!(MouseWheelUp), k!(MouseWheelDown))],
        [encoder!(k!(KbVolumeUp),   k!(KbVolumeDown))],
    ]
}
