use rmk::types::action::{EncoderAction, KeyAction};
use rmk::{a, encoder, k, mo};
pub(crate) const COL: usize = 7;
pub(crate) const ROW: usize = 8;
pub(crate) const NUM_LAYER: usize = 4;
pub(crate) const NUM_ENCODER: usize = 2;
#[rustfmt::skip]
pub const fn get_default_keymap() -> [[[KeyAction; COL]; ROW]; NUM_LAYER] {
    [
        [
            [k!(Tab), k!(Q), k!(W), k!(E), k!(R), k!(T), a!(No)],
            [k!(CapsLock), k!(A), k!(S), k!(D), k!(F), k!(G), a!(No)],
            [k!(LShift), k!(Z), k!(X), k!(C), k!(V), k!(B), a!(No)],
            [k!(LCtrl), k!(LGui), k!(LAlt), mo!(1), mo!(3), k!(Space), a!(No)],
            [k!(Backspace), k!(P), k!(O), k!(I), k!(U), k!(Y), a!(No)],
            [k!(Enter), k!(Backslash), k!(L), k!(K), k!(J), k!(H), a!(No)],
            [k!(Slash), k!(Up), k!(Dot), k!(Comma), k!(M), k!(N), a!(No)],
            [k!(Right), k!(Down), k!(Left), mo!(2), mo!(4), k!(Space), a!(No)]
        ],
        [
            [k!(Escape), k!(Kc1), k!(Kc2), k!(Kc3), k!(Kc4), k!(Kc5), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), k!(Semicolon), k!(Minus), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), k!(Delete), k!(Kc0), k!(Kc9), k!(Kc8), k!(Kc7), k!(Kc6)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), k!(Quote), k!(Equal)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)]
        ],
        [
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)]
        ],
        [
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)]
        ],
    ]
}

pub const fn get_default_encoder_map() -> [[EncoderAction; NUM_ENCODER]; NUM_LAYER] {
    [
        [
            encoder!(k!(KbVolumeUp), k!(KbVolumeDown)),
            encoder!(k!(KbVolumeUp), k!(KbVolumeDown)),
        ],
        [
            encoder!(k!(KbVolumeUp), k!(KbVolumeDown)),
            encoder!(k!(KbVolumeUp), k!(KbVolumeDown)),
        ],
        [
            encoder!(k!(KbVolumeUp), k!(KbVolumeDown)),
            encoder!(k!(KbVolumeUp), k!(KbVolumeDown)),
        ],
        [
            encoder!(k!(KbVolumeUp), k!(KbVolumeDown)),
            encoder!(k!(KbVolumeUp), k!(KbVolumeDown)),
        ],
    ]
}
