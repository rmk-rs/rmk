//! Host-side semantic key topology and lowering into lighting emitters.

use alloc::vec::Vec;

pub use rmk_types::key::KeyId;
use rmk_types::protocol::rynk::{LightingLed, LightingLedId, LightingMatrixPosition, LightingZone, LightingZoneId};

/// One canonical logical key. More semantic identifiers and attributes can be
/// added here without changing the low-level matrix or LED protocols.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogicalKey {
    pub id: KeyId,
    pub matrix: LightingMatrixPosition,
}

/// Anything that can select keys from the advertised topology.
///
/// The trait is deliberately open: board-aware tools can implement their own
/// names, regions, or aliases and still lower the result through the common
/// topology path.
pub trait KeySelector {
    fn matches(&self, key: &LogicalKey) -> bool;
}

impl KeySelector for KeyId {
    fn matches(&self, key: &LogicalKey) -> bool {
        *self == key.id
    }
}

impl KeySelector for LightingMatrixPosition {
    fn matches(&self, key: &LogicalKey) -> bool {
        *self == key.matrix
    }
}

impl<F> KeySelector for F
where
    F: Fn(&LogicalKey) -> bool,
{
    fn matches(&self, key: &LogicalKey) -> bool {
        self(key)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TopologyError {
    TooManyKeys,
    DuplicateMatrix(LightingMatrixPosition),
    DuplicateLed(LightingLedId),
    UnknownLedKey(LightingMatrixPosition),
    DuplicateZone(LightingZoneId),
    InvalidZoneSpan(LightingLedId),
    UnknownZoneMembership(LightingZoneId),
}

/// A revision-pinned semantic view assembled from Rynk's existing key and LED
/// metadata pages.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyTopology {
    pub topology_revision: u32,
    pub keys: Vec<LogicalKey>,
    pub leds: Vec<LightingLed>,
    pub zones: Vec<LightingZone>,
    pub zone_memberships: Vec<LightingZoneId>,
}

impl KeyTopology {
    pub fn new(
        topology_revision: u32,
        matrices: Vec<LightingMatrixPosition>,
        leds: Vec<LightingLed>,
        zones: Vec<LightingZone>,
        zone_memberships: Vec<LightingZoneId>,
    ) -> Result<Self, TopologyError> {
        let mut keys = Vec::with_capacity(matrices.len());
        for (index, matrix) in matrices.into_iter().enumerate() {
            let id = u16::try_from(index).map_err(|_| TopologyError::TooManyKeys)?;
            if keys.iter().any(|key: &LogicalKey| key.matrix == matrix) {
                return Err(TopologyError::DuplicateMatrix(matrix));
            }
            keys.push(LogicalKey { id: KeyId(id), matrix });
        }
        for (index, led) in leds.iter().enumerate() {
            if leds[..index].iter().any(|previous| previous.id == led.id) {
                return Err(TopologyError::DuplicateLed(led.id));
            }
            if let Some(matrix) = led.key
                && !keys.iter().any(|key| key.matrix == matrix)
            {
                return Err(TopologyError::UnknownLedKey(matrix));
            }
            let zone_end = usize::from(led.zone_start)
                .checked_add(usize::from(led.zone_len))
                .ok_or(TopologyError::InvalidZoneSpan(led.id))?;
            if zone_end > zone_memberships.len() {
                return Err(TopologyError::InvalidZoneSpan(led.id));
            }
        }
        for (index, zone) in zones.iter().enumerate() {
            if zones[..index].iter().any(|previous| previous.id == zone.id) {
                return Err(TopologyError::DuplicateZone(zone.id));
            }
        }
        for zone in &zone_memberships {
            if !zones.iter().any(|candidate| candidate.id == *zone) {
                return Err(TopologyError::UnknownZoneMembership(*zone));
            }
        }
        Ok(Self {
            topology_revision,
            keys,
            leds,
            zones,
            zone_memberships,
        })
    }

    pub fn select<'a, S>(&'a self, selector: &'a S) -> impl Iterator<Item = &'a LogicalKey> + 'a
    where
        S: KeySelector + ?Sized,
    {
        self.keys.iter().filter(move |key| selector.matches(key))
    }

    /// Lower every selected key to all associated semantic LED IDs.
    pub fn resolve_leds<S>(&self, selector: &S) -> Vec<LightingLedId>
    where
        S: KeySelector + ?Sized,
    {
        let mut resolved = Vec::new();
        for key in self.select(selector) {
            for led in &self.leds {
                if led.key == Some(key.matrix) && !resolved.contains(&led.id) {
                    resolved.push(led.id);
                }
            }
        }
        resolved
    }

    pub fn has_zone(&self, zone: LightingZoneId) -> bool {
        self.zones.iter().any(|candidate| candidate.id == zone)
    }

    pub fn resolve_zone_leds(&self, zone: LightingZoneId) -> Vec<LightingLedId> {
        self.leds
            .iter()
            .filter(|led| {
                let start = usize::from(led.zone_start);
                let end = start + usize::from(led.zone_len);
                self.zone_memberships
                    .get(start..end)
                    .is_some_and(|memberships| memberships.contains(&zone))
            })
            .map(|led| led.id)
            .collect()
    }

    pub fn all_leds(&self) -> Vec<LightingLedId> {
        self.leds.iter().map(|led| led.id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix(row: u8, col: u8) -> LightingMatrixPosition {
        LightingMatrixPosition { row, col }
    }

    fn led(id: u16, key: Option<LightingMatrixPosition>) -> LightingLed {
        LightingLed {
            id: LightingLedId(id),
            key,
            position: None,
            zone_start: 0,
            zone_len: 0,
        }
    }

    fn zone(id: u8, name: &str) -> LightingZone {
        LightingZone {
            id: LightingZoneId(id),
            name: name.try_into().unwrap(),
        }
    }

    #[test]
    fn resolves_ids_matrix_positions_and_open_selectors_to_emitters() {
        let topology = KeyTopology::new(
            7,
            vec![matrix(0, 0), matrix(0, 2)],
            vec![
                led(34, Some(matrix(0, 0))),
                led(80, Some(matrix(0, 0))),
                led(22, Some(matrix(0, 2))),
                led(99, None),
            ],
            vec![],
            vec![],
        )
        .unwrap();

        assert_eq!(
            topology.resolve_leds(&KeyId(0)),
            vec![LightingLedId(34), LightingLedId(80)]
        );
        assert_eq!(topology.resolve_leds(&matrix(0, 2)), vec![LightingLedId(22)]);
        assert_eq!(
            topology.resolve_leds(&|key: &LogicalKey| key.matrix.col >= 1),
            vec![LightingLedId(22)]
        );
    }

    #[test]
    fn validates_relational_metadata() {
        assert_eq!(
            KeyTopology::new(1, vec![matrix(0, 0), matrix(0, 0)], vec![], vec![], vec![]),
            Err(TopologyError::DuplicateMatrix(matrix(0, 0)))
        );
        assert_eq!(
            KeyTopology::new(1, vec![matrix(0, 0)], vec![led(1, None), led(1, None)], vec![], vec![],),
            Err(TopologyError::DuplicateLed(LightingLedId(1)))
        );
        assert_eq!(
            KeyTopology::new(1, vec![matrix(0, 0)], vec![led(1, Some(matrix(1, 0)))], vec![], vec![],),
            Err(TopologyError::UnknownLedKey(matrix(1, 0)))
        );
    }

    #[test]
    fn resolves_zones_and_all_emitters() {
        let mut first = led(34, Some(matrix(0, 0)));
        first.zone_len = 1;
        let mut second = led(99, None);
        second.zone_start = 1;
        second.zone_len = 1;
        let topology = KeyTopology::new(
            7,
            vec![matrix(0, 0)],
            vec![first, second],
            vec![zone(1, "keys"), zone(2, "status")],
            vec![LightingZoneId(1), LightingZoneId(2)],
        )
        .unwrap();

        assert!(topology.has_zone(LightingZoneId(1)));
        assert_eq!(topology.resolve_zone_leds(LightingZoneId(2)), vec![LightingLedId(99)]);
        assert_eq!(topology.all_leds(), vec![LightingLedId(34), LightingLedId(99)]);
    }

    #[test]
    fn validates_zone_metadata() {
        let mut invalid_span = led(1, None);
        invalid_span.zone_len = 1;
        assert_eq!(
            KeyTopology::new(1, vec![], vec![invalid_span], vec![], vec![]),
            Err(TopologyError::InvalidZoneSpan(LightingLedId(1)))
        );
        assert_eq!(
            KeyTopology::new(1, vec![], vec![], vec![zone(1, "one"), zone(1, "duplicate")], vec![],),
            Err(TopologyError::DuplicateZone(LightingZoneId(1)))
        );
        assert_eq!(
            KeyTopology::new(1, vec![], vec![], vec![], vec![LightingZoneId(1)]),
            Err(TopologyError::UnknownZoneMembership(LightingZoneId(1)))
        );
    }
}
