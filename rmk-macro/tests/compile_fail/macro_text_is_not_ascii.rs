// The same check `#[rmk_keyboard]` emits for `[behavior.macro]`.
use rmk_types::keyboard_macros::{MacroOp, validate_default_macros};

const MACROS: &[&[MacroOp]] = &[&[MacroOp::Char(0xE4)]];
const _: () = assert!(validate_default_macros(MACROS), "invalid macros");

fn main() {}
