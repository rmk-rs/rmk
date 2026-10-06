// The same check `#[rmk_keyboard]` emits for `[behavior.macro]`.
use rmk_types::constants::MACRO_SPACE_SIZE;
use rmk_types::keyboard_macros::{validate_default_macros, MacroOp};

const MACROS: &[&[MacroOp]] = &[&[MacroOp::Char(b'a'); MACRO_SPACE_SIZE]];
const _: () = assert!(validate_default_macros(MACROS), "invalid macros");

fn main() {}
