//! Allocation-free descriptions of logical lights and physical routing.

use core::ops::{BitOr, BitOrAssign};

pub use crate::physical_layout::{Coordinate as Coord, KeyPosition as MatrixPosition, PhysicalLayout, Point3};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct LedId(pub u16);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct LedSlot(pub u16);

impl LedSlot {
    pub const fn from_index(index: usize) -> Self {
        assert!(index <= u16::MAX as usize, "LED slot exceeds u16 capacity");
        Self(index as u16)
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct ZoneId(pub u8);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct LightingNodeId(pub u8);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct OutputId(pub u8);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatrixSize {
    pub rows: u8,
    pub cols: u8,
}

impl MatrixSize {
    pub const fn new(rows: u8, cols: u8) -> Self {
        Self { rows, cols }
    }

    pub const fn contains(self, position: MatrixPosition) -> bool {
        position.row < self.rows && position.col < self.cols
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ZoneSpan {
    pub start: u16,
    pub len: u8,
}

impl ZoneSpan {
    pub const EMPTY: Self = Self { start: 0, len: 0 };

    pub const fn new(start: u16, len: u8) -> Self {
        Self { start, len }
    }

    fn range(self, total: usize) -> Option<core::ops::Range<usize>> {
        let start = self.start as usize;
        let end = start.checked_add(self.len as usize)?;
        (end <= total).then_some(start..end)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ZoneMetadata<'a> {
    pub id: ZoneId,
    pub name: &'a str,
}

/// A semantic emitter. `key` records a real logical relationship; an explicit
/// position overrides the associated key center for spatial effects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LedMetadata {
    pub id: LedId,
    pub key: Option<MatrixPosition>,
    pub position: Option<Point3>,
    pub zones: ZoneSpan,
}

#[derive(Clone, Copy, Debug)]
pub struct LightingTopology<'a> {
    pub matrix: MatrixSize,
    pub keys: &'a [MatrixPosition],
    pub physical_layout: PhysicalLayout<'a>,
    pub leds: &'a [LedMetadata],
    pub zones: &'a [ZoneMetadata<'a>],
    pub zone_memberships: &'a [ZoneId],
}

impl<'a> LightingTopology<'a> {
    pub const fn len(&self) -> usize {
        self.leds.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.leds.is_empty()
    }

    pub fn slot(&self, id: LedId) -> Option<LedSlot> {
        self.leds
            .iter()
            .position(|led| led.id == id)
            .and_then(|index| u16::try_from(index).ok())
            .map(LedSlot)
    }

    pub fn led(&self, slot: LedSlot) -> Option<&'a LedMetadata> {
        self.leds.get(slot.index())
    }

    pub fn led_by_id(&self, id: LedId) -> Option<(LedSlot, &'a LedMetadata)> {
        let slot = self.slot(id)?;
        Some((slot, self.led(slot)?))
    }

    pub fn has_key(&self, matrix: MatrixPosition) -> bool {
        self.keys.contains(&matrix)
    }

    pub fn effective_position(&self, slot: LedSlot) -> Option<Point3> {
        let led = self.led(slot)?;
        led.position.or_else(|| {
            led.key
                .and_then(|key| self.physical_layout.key(key))
                .map(|key| key.center)
        })
    }

    pub fn zones_for(&self, slot: LedSlot) -> Option<&'a [ZoneId]> {
        let range = self.led(slot)?.zones.range(self.zone_memberships.len())?;
        Some(&self.zone_memberships[range])
    }

    pub fn has_zone(&self, slot: LedSlot, zone: ZoneId) -> bool {
        self.zones_for(slot).is_some_and(|zones| zones.contains(&zone))
    }

    pub fn leds_for_key(&'a self, key: MatrixPosition) -> impl Iterator<Item = (LedSlot, &'a LedMetadata)> + 'a {
        self.leds.iter().enumerate().filter_map(move |(index, led)| {
            if led.key != Some(key) {
                return None;
            }
            Some((LedSlot(u16::try_from(index).ok()?), led))
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct OutputCapabilities(u8);

impl OutputCapabilities {
    pub const NONE: Self = Self(0);
    pub const BINARY: Self = Self(1 << 0);
    pub const INTENSITY: Self = Self(1 << 1);
    pub const RGB: Self = Self(1 << 2);
    pub const WHITE: Self = Self(1 << 3);
    pub const ADDRESSABLE: Self = Self(1 << 4);
    pub const RGBW: Self = Self(Self::RGB.0 | Self::WHITE.0);
    const KNOWN_BITS: u8 = Self::BINARY.0 | Self::INTENSITY.0 | Self::RGB.0 | Self::WHITE.0 | Self::ADDRESSABLE.0;
    const COLOR_BITS: u8 = Self::BINARY.0 | Self::INTENSITY.0 | Self::RGB.0 | Self::WHITE.0;

    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::KNOWN_BITS == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn has_color_capability(self) -> bool {
        self.0 & Self::COLOR_BITS != 0
    }
}

impl BitOr for OutputCapabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl BitOrAssign for OutputCapabilities {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputCoverage {
    Complete,
    Sparse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputMetadata {
    pub node: LightingNodeId,
    pub id: OutputId,
    pub pixel_count: u16,
    pub capabilities: OutputCapabilities,
    pub coverage: OutputCoverage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicalRoute {
    pub slot: LedSlot,
    pub node: LightingNodeId,
    pub output: OutputId,
    pub physical_index: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct LightingRouting<'a> {
    pub outputs: &'a [OutputMetadata],
    pub routes: &'a [PhysicalRoute],
}

impl<'a> LightingRouting<'a> {
    pub fn output(&self, node: LightingNodeId, id: OutputId) -> Option<&'a OutputMetadata> {
        self.outputs
            .iter()
            .find(|output| output.node == node && output.id == id)
    }

    pub fn route(&self, slot: LedSlot) -> Option<&'a PhysicalRoute> {
        self.routes.iter().find(|route| route.slot == slot)
    }

    pub fn capabilities_for(&self, slot: LedSlot) -> Option<OutputCapabilities> {
        let route = self.route(slot)?;
        Some(self.output(route.node, route.output)?.capabilities)
    }
}

/// A precise topology or routing validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ValidationError {
    TooManyLeds {
        count: usize,
    },
    TooManyZoneMemberships {
        count: usize,
    },
    InvalidMatrixSize {
        size: MatrixSize,
    },
    KeyOutOfBounds {
        key_index: usize,
        matrix: MatrixPosition,
    },
    DuplicateKey {
        first_index: usize,
        second_index: usize,
        matrix: MatrixPosition,
    },
    UnknownPhysicalKey {
        key_index: usize,
        matrix: MatrixPosition,
    },
    DuplicatePhysicalKey {
        first_index: usize,
        second_index: usize,
        matrix: MatrixPosition,
    },
    DuplicateLedId {
        first_slot: LedSlot,
        second_slot: LedSlot,
        id: LedId,
    },
    UnknownKey {
        slot: LedSlot,
        matrix: MatrixPosition,
    },
    InvalidZoneSpan {
        slot: LedSlot,
        span: ZoneSpan,
    },
    EmptyZoneName {
        zone_index: usize,
        id: ZoneId,
    },
    DuplicateZoneId {
        first_index: usize,
        second_index: usize,
        id: ZoneId,
    },
    DuplicateZoneName {
        first_index: usize,
        second_index: usize,
    },
    UnknownZone {
        slot: LedSlot,
        membership_index: usize,
        id: ZoneId,
    },
    DuplicateLedZone {
        slot: LedSlot,
        id: ZoneId,
    },
    DuplicateOutputId {
        first_index: usize,
        second_index: usize,
        node: LightingNodeId,
        output: OutputId,
    },
    EmptyOutput {
        output_index: usize,
    },
    OutputHasNoColorCapability {
        output_index: usize,
    },
    RouteSlotOutOfBounds {
        route_index: usize,
        slot: LedSlot,
    },
    DuplicateRouteForSlot {
        first_index: usize,
        second_index: usize,
        slot: LedSlot,
    },
    MissingRouteForSlot {
        slot: LedSlot,
    },
    UnknownOutput {
        route_index: usize,
        node: LightingNodeId,
        output: OutputId,
    },
    PhysicalIndexOutOfBounds {
        route_index: usize,
        physical_index: u16,
        pixel_count: u16,
    },
    DuplicatePhysicalAddress {
        first_index: usize,
        second_index: usize,
    },
    MissingPhysicalRoute {
        output_index: usize,
        physical_index: u16,
    },
}

/// Validate semantic identity, key/geometry references, zones, split/output
/// ownership, and the logical-to-physical routing bijection.
///
/// This function allocates nothing. Its quadratic checks are deliberate: the
/// topology is small static board data and validation normally runs at build
/// time or once during initialization.
pub fn validate(topology: &LightingTopology<'_>, routing: &LightingRouting<'_>) -> Result<(), ValidationError> {
    if topology.leds.len() > u16::MAX as usize + 1 {
        return Err(ValidationError::TooManyLeds {
            count: topology.leds.len(),
        });
    }
    if topology.zone_memberships.len() > u16::MAX as usize + 1 {
        return Err(ValidationError::TooManyZoneMemberships {
            count: topology.zone_memberships.len(),
        });
    }
    if (topology.matrix.rows == 0) != (topology.matrix.cols == 0) {
        return Err(ValidationError::InvalidMatrixSize { size: topology.matrix });
    }

    for (index, key) in topology.keys.iter().copied().enumerate() {
        if !topology.matrix.contains(key) {
            return Err(ValidationError::KeyOutOfBounds {
                key_index: index,
                matrix: key,
            });
        }
        if let Some((first_index, _)) = topology.keys[..index]
            .iter()
            .enumerate()
            .find(|(_, previous)| **previous == key)
        {
            return Err(ValidationError::DuplicateKey {
                first_index,
                second_index: index,
                matrix: key,
            });
        }
    }

    for (index, key) in topology.physical_layout.keys.iter().enumerate() {
        let matrix = key.matrix;
        if !topology.matrix.contains(matrix) {
            return Err(ValidationError::KeyOutOfBounds {
                key_index: index,
                matrix,
            });
        }
        if !topology.has_key(matrix) {
            return Err(ValidationError::UnknownPhysicalKey {
                key_index: index,
                matrix,
            });
        }
        if let Some((first_index, _)) = topology.physical_layout.keys[..index]
            .iter()
            .enumerate()
            .find(|(_, previous)| previous.matrix == matrix)
        {
            return Err(ValidationError::DuplicatePhysicalKey {
                first_index,
                second_index: index,
                matrix,
            });
        }
    }

    for (index, zone) in topology.zones.iter().enumerate() {
        if zone.name.is_empty() {
            return Err(ValidationError::EmptyZoneName {
                zone_index: index,
                id: zone.id,
            });
        }
        for (first_index, previous) in topology.zones[..index].iter().enumerate() {
            if previous.id == zone.id {
                return Err(ValidationError::DuplicateZoneId {
                    first_index,
                    second_index: index,
                    id: zone.id,
                });
            }
            if previous.name == zone.name {
                return Err(ValidationError::DuplicateZoneName {
                    first_index,
                    second_index: index,
                });
            }
        }
    }

    for (index, led) in topology.leds.iter().enumerate() {
        let slot = LedSlot(index as u16);
        if let Some((first, _)) = topology.leds[..index]
            .iter()
            .enumerate()
            .find(|(_, previous)| previous.id == led.id)
        {
            return Err(ValidationError::DuplicateLedId {
                first_slot: LedSlot(first as u16),
                second_slot: slot,
                id: led.id,
            });
        }
        if let Some(matrix) = led.key
            && !topology.has_key(matrix)
        {
            return Err(ValidationError::UnknownKey { slot, matrix });
        }
        let Some(range) = led.zones.range(topology.zone_memberships.len()) else {
            return Err(ValidationError::InvalidZoneSpan { slot, span: led.zones });
        };
        for membership_index in range.clone() {
            let id = topology.zone_memberships[membership_index];
            if !topology.zones.iter().any(|zone| zone.id == id) {
                return Err(ValidationError::UnknownZone {
                    slot,
                    membership_index,
                    id,
                });
            }
            if topology.zone_memberships[range.start..membership_index].contains(&id) {
                return Err(ValidationError::DuplicateLedZone { slot, id });
            }
        }
    }

    for (index, output) in routing.outputs.iter().enumerate() {
        if let Some((first_index, _)) = routing.outputs[..index]
            .iter()
            .enumerate()
            .find(|(_, previous)| previous.node == output.node && previous.id == output.id)
        {
            return Err(ValidationError::DuplicateOutputId {
                first_index,
                second_index: index,
                node: output.node,
                output: output.id,
            });
        }
        if output.pixel_count == 0 {
            return Err(ValidationError::EmptyOutput { output_index: index });
        }
        if !output.capabilities.has_color_capability() {
            return Err(ValidationError::OutputHasNoColorCapability { output_index: index });
        }
    }

    for (index, route) in routing.routes.iter().enumerate() {
        if route.slot.0 as usize >= topology.leds.len() {
            return Err(ValidationError::RouteSlotOutOfBounds {
                route_index: index,
                slot: route.slot,
            });
        }
        if let Some((first_index, _)) = routing.routes[..index]
            .iter()
            .enumerate()
            .find(|(_, previous)| previous.slot == route.slot)
        {
            return Err(ValidationError::DuplicateRouteForSlot {
                first_index,
                second_index: index,
                slot: route.slot,
            });
        }
        let Some(output) = routing.output(route.node, route.output) else {
            return Err(ValidationError::UnknownOutput {
                route_index: index,
                node: route.node,
                output: route.output,
            });
        };
        if route.physical_index >= output.pixel_count {
            return Err(ValidationError::PhysicalIndexOutOfBounds {
                route_index: index,
                physical_index: route.physical_index,
                pixel_count: output.pixel_count,
            });
        }
        if let Some((first_index, _)) = routing.routes[..index].iter().enumerate().find(|(_, previous)| {
            previous.node == route.node
                && previous.output == route.output
                && previous.physical_index == route.physical_index
        }) {
            return Err(ValidationError::DuplicatePhysicalAddress {
                first_index,
                second_index: index,
            });
        }
    }

    for slot in 0..topology.leds.len() {
        let slot = LedSlot(slot as u16);
        if routing.route(slot).is_none() {
            return Err(ValidationError::MissingRouteForSlot { slot });
        }
    }

    for (output_index, output) in routing.outputs.iter().enumerate() {
        if output.coverage == OutputCoverage::Sparse {
            continue;
        }
        for physical_index in 0..output.pixel_count {
            if !routing.routes.iter().any(|route| {
                route.node == output.node && route.output == output.id && route.physical_index == physical_index
            }) {
                return Err(ValidationError::MissingPhysicalRoute {
                    output_index,
                    physical_index,
                });
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_layout::*;

    #[test]
    fn keyed_emitter_falls_back_to_shared_key_center() {
        let keys = [PhysicalKey {
            matrix: KeyPosition::new(0, 0),
            center: Point3::new(Coordinate::ONE, Coordinate::ZERO, Coordinate::ZERO),
            size: KeySize::ONE,
            rotation: Rotation::ZERO,
        }];
        let leds = [LedMetadata {
            id: LedId(7),
            key: Some(KeyPosition::new(0, 0)),
            position: None,
            zones: ZoneSpan::new(0, 0),
        }];
        let topology = LightingTopology {
            matrix: MatrixSize::new(1, 1),
            keys: &[KeyPosition::new(0, 0)],
            physical_layout: PhysicalLayout::new(&keys),
            leds: &leds,
            zones: &[],
            zone_memberships: &[],
        };
        assert_eq!(topology.effective_position(LedSlot(0)), Some(keys[0].center));
    }

    #[test]
    fn validation_accepts_canonical_layout_and_semantic_route() {
        let keys = [PhysicalKey {
            matrix: KeyPosition::new(0, 0),
            center: Point3::new(Coordinate::ONE, Coordinate::ZERO, Coordinate::ZERO),
            size: KeySize::ONE,
            rotation: Rotation::ZERO,
        }];
        let leds = [LedMetadata {
            id: LedId(7),
            key: Some(KeyPosition::new(0, 0)),
            position: None,
            zones: ZoneSpan::EMPTY,
        }];
        let topology = LightingTopology {
            matrix: MatrixSize::new(1, 1),
            keys: &[KeyPosition::new(0, 0)],
            physical_layout: PhysicalLayout::new(&keys),
            leds: &leds,
            zones: &[],
            zone_memberships: &[],
        };
        let outputs = [OutputMetadata {
            node: LightingNodeId(0),
            id: OutputId(0),
            pixel_count: 1,
            capabilities: OutputCapabilities::RGB.union(OutputCapabilities::ADDRESSABLE),
            coverage: OutputCoverage::Complete,
        }];
        let routes = [PhysicalRoute {
            slot: LedSlot(0),
            node: LightingNodeId(0),
            output: OutputId(0),
            physical_index: 0,
        }];

        assert_eq!(
            validate(
                &topology,
                &LightingRouting {
                    outputs: &outputs,
                    routes: &routes,
                }
            ),
            Ok(())
        );
    }
}
