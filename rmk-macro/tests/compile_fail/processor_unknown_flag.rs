use rmk_macro::processor;

#[derive(Clone, Copy, Debug)]
pub struct KeyEvent;

#[processor(subscribe = [KeyEvent], dedline)]
pub struct Blinker;

fn main() {}
