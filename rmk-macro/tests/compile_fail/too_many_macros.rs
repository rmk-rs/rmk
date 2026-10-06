// The same check `#[rmk_keyboard]` emits for `[behavior.macro]`.
use rmk_types::constants::MACRO_MAX_NUM;

const EMPTY: &[MacroOp] = &[];
use rmk_types::keyboard_macros::{MacroOp, validate_default_macros};

const MACROS: &[&[MacroOp]] = &[EMPTY; MACRO_MAX_NUM + 1];
const _: () = assert!(validate_default_macros(MACROS), "invalid macros");

fn main() {}
