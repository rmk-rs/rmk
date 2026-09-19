/// Config for [vial](https://get.vial.today/).
///
/// You can generate automatically using [`build.rs`](https://github.com/rmk-rs/rmk/blob/main/examples/use_rust/stm32h7/build.rs).
#[derive(Clone, Copy, Debug)]
pub struct VialConfig<'a> {
    pub vial_keyboard_id: &'a [u8],
    pub vial_keyboard_def: &'a [u8],
    pub unlock_keys: &'a [(u8, u8)],
    pub insecure: bool,
    /// Number of dynamic macros reported to Vial.
    pub macro_count: u8,
}

impl<'a> VialConfig<'a> {
    pub fn new(vial_keyboard_id: &'a [u8], vial_keyboard_def: &'a [u8], unlock_keys: &'a [(u8, u8)]) -> Self {
        Self {
            vial_keyboard_id,
            vial_keyboard_def,
            unlock_keys,
            insecure: false,
            macro_count: 32,
        }
    }

    /// Set the number of dynamic macros reported to Vial.
    pub const fn with_macro_count(mut self, macro_count: u8) -> Self {
        self.macro_count = macro_count;
        self
    }
}

impl Default for VialConfig<'_> {
    fn default() -> Self {
        Self {
            vial_keyboard_id: &[],
            vial_keyboard_def: &[],
            unlock_keys: &[],
            insecure: false,
            macro_count: 32,
        }
    }
}
