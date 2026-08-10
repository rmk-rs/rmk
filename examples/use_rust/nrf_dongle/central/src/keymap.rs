use rmk::types::action::KeyAction;
use rmk::{k, user};

/// Central half: the Elytra left hand, 5x7. Peripheral half: the nRF54L15
/// DK's four buttons, mapped at row offset 5. One keymap covers both halves.
pub(crate) const COL: usize = 7;
pub(crate) const ROW: usize = 7;
pub(crate) const NUM_LAYER: usize = 1;

#[rustfmt::skip]
pub const fn get_default_keymap() -> [[[KeyAction; COL]; ROW]; NUM_LAYER] {
    [
        [
            // Elytra left hand; User8 (top-left key) = SwitchToDongle with the
            // default 3 BLE profiles (hold 5s to clear the bond / authorize).
            [user!(8),    k!(Kc1),  k!(Kc2),   k!(Kc3),   k!(Kc4),        k!(Kc5),  k!(Kc6)],
            [k!(Tab),     k!(Q),    k!(W),     k!(E),     k!(R),          k!(T),    k!(Y)],
            [k!(LShift),  k!(A),    k!(S),     k!(D),     k!(F),          k!(G),    k!(H)],
            [k!(LCtrl),   k!(Z),    k!(X),     k!(C),     k!(V),          k!(B),    k!(N)],
            [k!(LGui),    k!(LAlt), k!(Space), k!(Enter), k!(Backspace),  k!(No),   k!(No)],
            // Peripheral half (2x2).
            [k!(Kc7),     k!(Kc8),  k!(No),    k!(No),    k!(No),         k!(No),   k!(No)],
            [k!(Kc9),     k!(Kc0),  k!(No),    k!(No),    k!(No),         k!(No),   k!(No)],
        ],
    ]
}
