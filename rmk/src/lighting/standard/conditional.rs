#[allow(unused_imports)]
use super::*;
use crate::lighting::Rgb8;
use crate::lighting::compositor::{Contribution, LightingSource, RenderInput as SourceRenderInput};
use crate::lighting::context::LightingContextProvider;
use crate::lighting::effect::{BuiltinEffect, LightingEffect};
use crate::lighting::source::{BatteryStatusProvider, ConditionSet, OutputMode};
use crate::lighting::topology::LedSlot;

/// One ordered, runtime-authored conditional rule.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RuntimeConditionalSceneCell {
    pub conditions: ConditionSet,
    pub slot: LedSlot,
    pub effect: BuiltinEffect,
}

pub(super) const EMPTY_CONDITIONAL_SCENE_CELL: RuntimeConditionalSceneCell = RuntimeConditionalSceneCell {
    conditions: ConditionSet {
        layer: None,
        battery: None,
        connection: None,
        output_mode: None,
    },
    slot: LedSlot(0),
    effect: BuiltinEffect::Solid { color: Rgb8::BLACK },
};

/// A conditional rule as stored: conditions and target stay with the rule,
/// while the effect is interned in the table's [`StylePool`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct InternedRuntimeConditionalSceneCell {
    conditions: ConditionSet,
    slot: LedSlot,
    style: u8,
}

const EMPTY_INTERNED_CONDITIONAL_SCENE_CELL: InternedRuntimeConditionalSceneCell =
    InternedRuntimeConditionalSceneCell {
        conditions: ConditionSet {
            layer: None,
            battery: None,
            connection: None,
            output_mode: None,
        },
        slot: LedSlot(0),
        style: 0,
    };

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RuntimeConditionalSceneChunk {
    cells: [RuntimeConditionalSceneCell; CONDITIONAL_SCENE_CHUNK_SIZE],
    len: usize,
}

impl RuntimeConditionalSceneChunk {
    pub const fn new() -> Self {
        Self {
            cells: [EMPTY_CONDITIONAL_SCENE_CELL; CONDITIONAL_SCENE_CHUNK_SIZE],
            len: 0,
        }
    }

    pub fn push(&mut self, cell: RuntimeConditionalSceneCell) -> Result<(), StandardError> {
        if self.len == CONDITIONAL_SCENE_CHUNK_SIZE {
            return Err(StandardError::InvalidConditionalSceneRequest);
        }
        self.cells[self.len] = cell;
        self.len += 1;
        Ok(())
    }

    pub fn as_slice(&self) -> &[RuntimeConditionalSceneCell] {
        &self.cells[..self.len]
    }
}

impl Default for RuntimeConditionalSceneChunk {
    fn default() -> Self {
        Self::new()
    }
}

/// Fixed-capacity ordered conditional source. Order is semantic: later
/// matching rules override earlier rules at the same compositor priority.
#[derive(Copy, Clone, Debug)]
pub struct RuntimeConditionalSceneTable<const CAP: usize> {
    cells: [InternedRuntimeConditionalSceneCell; CAP],
    len: usize,
    pub(super) pool: StylePool,
}

impl<const CAP: usize> RuntimeConditionalSceneTable<CAP> {
    pub const fn new() -> Self {
        Self {
            cells: [EMPTY_INTERNED_CONDITIONAL_SCENE_CELL; CAP],
            len: 0,
            pool: StylePool::new(),
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Installed rules in their semantic order, with effects resolved from the
    /// pool. Style indices are an allocation detail and never escape the table.
    pub fn iter(&self) -> impl Iterator<Item = RuntimeConditionalSceneCell> + '_ {
        self.cells[..self.len].iter().filter_map(move |held| {
            self.pool.get(held.style).map(|effect| RuntimeConditionalSceneCell {
                conditions: held.conditions,
                slot: held.slot,
                effect,
            })
        })
    }

    pub fn push(&mut self, cell: RuntimeConditionalSceneCell) -> Result<(), StandardError> {
        if self.len == CAP {
            return Err(StandardError::ConditionalSceneFull { capacity: CAP });
        }
        let style = self
            .pool
            .intern(cell.effect)
            .ok_or(StandardError::ConditionalSceneFull {
                capacity: SCENE_STYLE_CAP,
            })?;
        self.cells[self.len] = InternedRuntimeConditionalSceneCell {
            conditions: cell.conditions,
            slot: cell.slot,
            style,
        };
        self.len += 1;
        Ok(())
    }

    /// Mutable access to the most recently pushed rule's conditions, for
    /// stream decoders that amend a cell after its base record arrives.
    pub fn last_conditions_mut(&mut self) -> Option<&mut ConditionSet> {
        self.len
            .checked_sub(1)
            .map(move |index| &mut self.cells[index].conditions)
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.pool.clear();
    }

    pub fn page(&self, offset: u16) -> RuntimeConditionalSceneChunk {
        let start = (offset as usize).min(self.len);
        let end = (start + CONDITIONAL_SCENE_CHUNK_SIZE).min(self.len);
        let mut chunk = RuntimeConditionalSceneChunk::new();
        for cell in self.iter().skip(start).take(end - start) {
            chunk.push(cell).expect("page is chunk-bounded");
        }
        chunk
    }
}

impl<const CAP: usize> Default for RuntimeConditionalSceneTable<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAP: usize> PartialEq for RuntimeConditionalSceneTable<CAP> {
    fn eq(&self, other: &Self) -> bool {
        // Style indices may differ after independent replacement/replication;
        // only the ordered resolved rules are semantic state.
        self.iter().eq(other.iter())
    }
}

impl<const CAP: usize> Eq for RuntimeConditionalSceneTable<CAP> {}

pub(super) struct RuntimeConditionalSource<'a, Batteries: ?Sized, const CAP: usize> {
    pub(super) table: &'a RuntimeConditionalSceneTable<CAP>,
    pub(super) batteries: &'a Batteries,
    /// The engine owns the policy, so unlike the board's compiled source this
    /// one can evaluate an output-mode condition.
    pub(super) output_mode: OutputMode,
}

impl<Context, Batteries, const CAP: usize> LightingSource<Rgb8, Context>
    for RuntimeConditionalSource<'_, Batteries, CAP>
where
    Context: LightingContextProvider,
    Batteries: BatteryStatusProvider + ?Sized,
{
    fn len(&self, input: &SourceRenderInput<'_, Context>) -> usize {
        self.table
            .iter()
            .filter(|cell| {
                cell.conditions
                    .matches(input.context, self.batteries, Some(self.output_mode))
            })
            .count()
    }

    fn slot(&self, index: usize, input: &SourceRenderInput<'_, Context>) -> LedSlot {
        self.table
            .iter()
            .filter(|cell| {
                cell.conditions
                    .matches(input.context, self.batteries, Some(self.output_mode))
            })
            .nth(index)
            .expect("LightingSource index must be below len")
            .slot
    }

    fn contribution(&mut self, index: usize, input: &SourceRenderInput<'_, Context>) -> Contribution<Rgb8> {
        let cell = self
            .table
            .iter()
            .filter(|cell| {
                cell.conditions
                    .matches(input.context, self.batteries, Some(self.output_mode))
            })
            .nth(index)
            .expect("LightingSource index must be below len");
        Contribution::Opaque(cell.effect.sample(input.now_ms))
    }
}

pub(super) struct NoBatteryStatus;

impl BatteryStatusProvider for NoBatteryStatus {
    fn battery_status(&self, _node: u8) -> crate::types::battery::BatteryStatus {
        crate::types::battery::BatteryStatus::Unavailable
    }
}

pub(super) static NO_BATTERY_STATUS: NoBatteryStatus = NoBatteryStatus;
