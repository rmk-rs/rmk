use super::topology::{LedId, LedSlot, LightingTopology, MatrixPosition, ZoneId};

/// Stable configuration-level selector. None of these variants exposes a
/// local frame slot or electrical chain index.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LedSelector {
    Led(LedId),
    Key(MatrixPosition),
    Zone(ZoneId),
    All,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ResolveError {
    EmptySelection,
    NoMatches(LedSelector),
    UnknownLed(LedId),
    UnknownKey(MatrixPosition),
    UnknownZone(ZoneId),
    CapacityExceeded { capacity: usize },
}

/// Deduplicated, fixed-capacity target set resolved once at configuration or
/// source installation time.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ResolvedTargets<const CAP: usize> {
    slots: [LedSlot; CAP],
    len: usize,
}

impl<const CAP: usize> ResolvedTargets<CAP> {
    pub const fn new() -> Self {
        Self {
            slots: [LedSlot(0); CAP],
            len: 0,
        }
    }

    pub fn resolve(topology: &LightingTopology<'_>, selectors: &[LedSelector]) -> Result<Self, ResolveError> {
        if selectors.is_empty() {
            return Err(ResolveError::EmptySelection);
        }
        let mut resolved = Self::new();
        for selector in selectors {
            let mut matched = false;
            match *selector {
                LedSelector::Led(id) => {
                    let slot = topology.slot(id).ok_or(ResolveError::UnknownLed(id))?;
                    matched = true;
                    resolved.insert(slot)?;
                }
                LedSelector::Key(key) => {
                    if !topology.has_key(key) {
                        return Err(ResolveError::UnknownKey(key));
                    }
                    for (slot, _) in topology.leds_for_key(key) {
                        matched = true;
                        resolved.insert(slot)?;
                    }
                }
                LedSelector::Zone(zone) => {
                    if !topology.zones.iter().any(|metadata| metadata.id == zone) {
                        return Err(ResolveError::UnknownZone(zone));
                    }
                    for index in 0..topology.len() {
                        let slot = LedSlot::from_index(index);
                        if topology.has_zone(slot, zone) {
                            matched = true;
                            resolved.insert(slot)?;
                        }
                    }
                }
                LedSelector::All => {
                    for index in 0..topology.len() {
                        matched = true;
                        resolved.insert(LedSlot::from_index(index))?;
                    }
                }
            }
            if !matched {
                return Err(ResolveError::NoMatches(*selector));
            }
        }
        Ok(resolved)
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_slice(&self) -> &[LedSlot] {
        &self.slots[..self.len]
    }

    fn insert(&mut self, slot: LedSlot) -> Result<(), ResolveError> {
        if self.as_slice().contains(&slot) {
            return Ok(());
        }
        if self.len == CAP {
            return Err(ResolveError::CapacityExceeded { capacity: CAP });
        }
        self.slots[self.len] = slot;
        self.len += 1;
        Ok(())
    }
}

impl<const CAP: usize> Default for ResolvedTargets<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::topology::{LedMetadata, LightingTopology, MatrixSize, PhysicalLayout, ZoneMetadata, ZoneSpan};
    use super::*;

    const KEY: MatrixPosition = MatrixPosition::new(0, 0);
    static KEYS: [MatrixPosition; 1] = [KEY];
    static ZONES: [ZoneMetadata<'static>; 1] = [ZoneMetadata {
        id: ZoneId(7),
        name: "thumb",
    }];
    static MEMBERSHIPS: [ZoneId; 2] = [ZoneId(7), ZoneId(7)];
    static LEDS: [LedMetadata; 3] = [
        LedMetadata {
            id: LedId(10),
            key: Some(KEY),
            position: None,
            zones: ZoneSpan::new(0, 1),
        },
        LedMetadata {
            id: LedId(20),
            key: Some(KEY),
            position: None,
            zones: ZoneSpan::new(1, 1),
        },
        LedMetadata {
            id: LedId(99),
            key: None,
            position: None,
            zones: ZoneSpan::EMPTY,
        },
    ];

    fn topology() -> LightingTopology<'static> {
        LightingTopology {
            matrix: MatrixSize::new(1, 1),
            keys: &KEYS,
            physical_layout: PhysicalLayout::EMPTY,
            leds: &LEDS,
            zones: &ZONES,
            zone_memberships: &MEMBERSHIPS,
        }
    }

    #[test]
    fn resolves_keys_to_multiple_leds_and_deduplicates_selectors() {
        let targets = ResolvedTargets::<3>::resolve(
            &topology(),
            &[
                LedSelector::Led(LedId(10)),
                LedSelector::Key(KEY),
                LedSelector::Zone(ZoneId(7)),
            ],
        )
        .unwrap();
        assert_eq!(targets.as_slice(), &[LedSlot(0), LedSlot(1)]);
    }

    #[test]
    fn resolution_failure_is_atomic() {
        assert_eq!(
            ResolvedTargets::<1>::resolve(&topology(), &[LedSelector::All]),
            Err(ResolveError::CapacityExceeded { capacity: 1 })
        );
        assert_eq!(
            ResolvedTargets::<3>::resolve(&topology(), &[LedSelector::Led(LedId(404))]),
            Err(ResolveError::UnknownLed(LedId(404)))
        );
        assert_eq!(
            ResolvedTargets::<3>::resolve(&topology(), &[]),
            Err(ResolveError::EmptySelection)
        );
    }

    #[test]
    fn known_but_empty_key_or_zone_is_rejected() {
        let empty_keys = [KEY];
        let empty_zone = [ZoneMetadata {
            id: ZoneId(8),
            name: "empty",
        }];
        let empty = LightingTopology {
            matrix: MatrixSize::new(1, 1),
            keys: &empty_keys,
            physical_layout: PhysicalLayout::EMPTY,
            leds: &[],
            zones: &empty_zone,
            zone_memberships: &[],
        };
        assert_eq!(
            ResolvedTargets::<1>::resolve(&empty, &[LedSelector::Key(KEY)]),
            Err(ResolveError::NoMatches(LedSelector::Key(KEY)))
        );
        assert_eq!(
            ResolvedTargets::<1>::resolve(&empty, &[LedSelector::Zone(ZoneId(8))]),
            Err(ResolveError::NoMatches(LedSelector::Zone(ZoneId(8))))
        );
    }
}
