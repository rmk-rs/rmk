/// Device-independent, linear RGB sample used by RMK's standard compositor.
///
/// A driver or output transform is responsible for channel order, gamma,
/// RGBW/mono conversion, brightness, and electrical safety limits.
#[derive(Copy, Clone, Default, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Rgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb8 {
    pub const BLACK: Self = Self::new(0, 0, 0);

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const fn scale(self, level: u8) -> Self {
        const fn channel(value: u8, level: u8) -> u8 {
            ((value as u16 * level as u16) / 255) as u8
        }
        Self::new(channel(self.r, level), channel(self.g, level), channel(self.b, level))
    }
}
