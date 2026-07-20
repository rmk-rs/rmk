//! Allocation-free physical key geometry shared by lighting, display, and
//! host-facing layout consumers.

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KeyPosition {
    pub row: u8,
    pub col: u8,
}

impl KeyPosition {
    pub const fn new(row: u8, col: u8) -> Self {
        Self { row, col }
    }
}

/// Signed Q8.8 coordinate measured in key-pitch units.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Coordinate(i16);

impl Coordinate {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(256);

    pub const fn from_raw(raw: i16) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> i16 {
        self.0
    }
}

/// Positive Q8.8 extent measured in key-pitch units.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Extent(u16);

impl Extent {
    pub const ONE: Self = Self(256);

    pub const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Rotation(i16);

impl Rotation {
    pub const ZERO: Self = Self(0);

    pub const fn from_centidegrees(value: i16) -> Self {
        Self(value)
    }

    pub const fn centidegrees(self) -> i16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Point3 {
    pub x: Coordinate,
    pub y: Coordinate,
    pub z: Coordinate,
}

impl Point3 {
    pub const fn new(x: Coordinate, y: Coordinate, z: Coordinate) -> Self {
        Self { x, y, z }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct KeySize {
    pub width: Extent,
    pub height: Extent,
}

impl KeySize {
    pub const ONE: Self = Self::new(Extent::ONE, Extent::ONE);

    pub const fn new(width: Extent, height: Extent) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicalKey {
    pub matrix: KeyPosition,
    pub center: Point3,
    pub size: KeySize,
    pub rotation: Rotation,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PhysicalLayout<'a> {
    pub keys: &'a [PhysicalKey],
}

impl<'a> PhysicalLayout<'a> {
    pub const EMPTY: Self = Self { keys: &[] };

    pub const fn new(keys: &'a [PhysicalKey]) -> Self {
        Self { keys }
    }

    pub fn key(&self, matrix: KeyPosition) -> Option<&'a PhysicalKey> {
        self.keys.iter().find(|key| key.matrix == matrix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_geometry_is_lossless() {
        let key = PhysicalKey {
            matrix: KeyPosition::new(1, 2),
            center: Point3::new(Coordinate::from_raw(-128), Coordinate::ONE, Coordinate::ZERO),
            size: KeySize::ONE,
            rotation: Rotation::from_centidegrees(-750),
        };
        let layout = PhysicalLayout::new(core::slice::from_ref(&key));
        assert_eq!(layout.key(KeyPosition::new(1, 2)).unwrap(), &key);
        assert_eq!(key.center.x.raw(), -128);
    }
}
