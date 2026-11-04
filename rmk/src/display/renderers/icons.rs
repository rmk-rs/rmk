/// Shift — upward arrow (⇧)
/// ```text
/// ...##...
/// ..####..
/// .######.
/// ...##...
/// ...##...
/// ...##...
/// ...##...
/// ........
/// ```
pub const SHIFT: [u8; 8] = [0x18, 0x3C, 0x7E, 0x18, 0x18, 0x18, 0x18, 0x00];

/// Ctrl — chevron/caret (^)
/// ```text
/// ........
/// ...##...
/// ..####..
/// .##..##.
/// ##....##
/// ........
/// ........
/// ........
/// ```
pub const CTRL: [u8; 8] = [0x00, 0x18, 0x3C, 0x66, 0xC3, 0x00, 0x00, 0x00];

/// Alt — letter A
/// ```text
/// ...##...
/// ..#..#..
/// .#....#.
/// .######.
/// .#....#.
/// .#....#.
/// ........
/// ........
/// ```
pub const ALT: [u8; 8] = [0x18, 0x24, 0x42, 0x7E, 0x42, 0x42, 0x00, 0x00];

/// GUI — four squares (⊞ Windows logo)
/// ```text
/// ........
/// .##.##.
/// .##.##.
/// ........
/// .##.##.
/// .##.##.
/// ........
/// ........
/// ```
pub const GUI: [u8; 8] = [0x00, 0x6C, 0x6C, 0x00, 0x6C, 0x6C, 0x00, 0x00];

pub const ICON_SIZE: u32 = 8;

/// Bluetooth logo — 9×14, rounded rect with ᛒ rune in negative space.
/// Ported from zmk-dongle-display (englmaxi).
///
/// ```text
/// ..#####..
/// .##..###.
/// ###...###
/// ###.#..##
/// #...##..#
/// ##..#..##
/// ###...###
/// ###...###
/// ##..#..##
/// #...##..#
/// ###.#..##
/// ###...###
/// .##..###.
/// ..#####..
/// ```
#[cfg_attr(not(feature = "_ble"), allow(dead_code))]
pub const BT_ICON: [u8; 28] = [
    0x3E, 0x00, 0x67, 0x00, 0xE3, 0x80, 0xE9, 0x80, 0x8C, 0x80, 0xC9, 0x80, 0xE3, 0x80, 0xE3, 0x80, 0xC9, 0x80, 0x8C,
    0x80, 0xE9, 0x80, 0xE3, 0x80, 0x67, 0x00, 0x3E, 0x00,
];
#[cfg_attr(not(feature = "_ble"), allow(dead_code))]
pub const BT_ICON_W: u32 = 9;
#[cfg_attr(not(feature = "_ble"), allow(dead_code))]
pub const BT_ICON_H: u32 = 14;
