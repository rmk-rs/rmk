// The same check `#[rmk_keyboard]` emits for `[behavior.macro]`.
use rmk_types::keyboard_macros::{MacroOp, validate_default_macros};

const MACROS: &[&[MacroOp]] = &[&[MacroOp::PauseForRelease, MacroOp::Char(b'a'), MacroOp::PauseForRelease]];
const _: () = assert!(validate_default_macros(MACROS), "invalid macros");

fn main() {}
