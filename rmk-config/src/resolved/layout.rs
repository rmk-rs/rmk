/// Signed Q8.8 board-space point in key-pitch units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedPoint3 {
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

/// Unsigned Q8.8 key size in key-pitch units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedSize2 {
    pub width: u16,
    pub height: u16,
}

/// Fixed-point geometry for one key in the selected/default KLE variant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalKey {
    pub matrix: [u8; 2],
    pub center: FixedPoint3,
    pub size: FixedSize2,
    /// Clockwise rotation in hundredths of one degree.
    pub rotation_centidegrees: i16,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PhysicalLayout {
    pub keys: Vec<PhysicalKey>,
}

/// Resolved physical layout. `blob` preserves the complete variant-aware KLE
/// representation streamed over `GetLayout`; `physical` is the allocation-free
/// firmware geometry derived from its selected/default variant. `keys` is the
/// variant-independent set of logical matrix positions from `[layout].map`.
pub struct Layout {
    pub blob: Vec<u8>,
    pub rows: u8,
    pub cols: u8,
    pub keys: Vec<[u8; 2]>,
    pub physical: PhysicalLayout,
}

impl crate::KeyboardTomlConfig {
    /// Resolve the physical layout blob from the `[layout]` section.
    pub fn layout(&self) -> Result<Layout, String> {
        let (blob, keys, physical, rows, cols) = match &self.layout {
            Some(l) => {
                let resolved = crate::layout::build_resolved_layout(l, Some(self.total_encoders()))?;
                (resolved.blob, resolved.keys, resolved.physical, l.rows, l.cols)
            }
            None => (Vec::new(), Vec::new(), PhysicalLayout::default(), 0, 0),
        };
        Ok(Layout {
            blob,
            rows,
            cols,
            keys,
            physical,
        })
    }

    /// Resolve the `[layout]` section without consulting the board config.
    ///
    /// Firmware that wires its scan hardware by hand has no `[matrix]` or
    /// `[split]` section, so encoder counts cannot be validated against the
    /// layout map; everything else resolves as in [`Self::layout`].
    pub fn layout_standalone(&self) -> Result<Layout, String> {
        let (blob, keys, physical, rows, cols) = match &self.layout {
            Some(l) => {
                let resolved = crate::layout::build_resolved_layout(l, None)?;
                (resolved.blob, resolved.keys, resolved.physical, l.rows, l.cols)
            }
            None => (Vec::new(), Vec::new(), PhysicalLayout::default(), 0, 0),
        };
        Ok(Layout {
            blob,
            rows,
            cols,
            keys,
            physical,
        })
    }
}
