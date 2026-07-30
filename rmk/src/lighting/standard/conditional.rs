#[allow(unused_imports)]
use super::*;
use crate::lighting::Rgb8;
use crate::lighting::compositor::{Contribution, LightingSource, RenderInput as SourceRenderInput};
use crate::lighting::context::LightingContextProvider;
use crate::lighting::effect::{BuiltinEffect, LightingEffect};
use crate::lighting::source::{BatteryStatusProvider, ConditionSet};
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
    },
    slot: LedSlot(0),
    effect: BuiltinEffect::Solid { color: Rgb8::BLACK },
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
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RuntimeConditionalSceneTable<const CAP: usize> {
    cells: [RuntimeConditionalSceneCell; CAP],
    len: usize,
}

impl<const CAP: usize> RuntimeConditionalSceneTable<CAP> {
    pub const fn new() -> Self {
        Self {
            cells: [EMPTY_CONDITIONAL_SCENE_CELL; CAP],
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_slice(&self) -> &[RuntimeConditionalSceneCell] {
        &self.cells[..self.len]
    }

    pub fn push(&mut self, cell: RuntimeConditionalSceneCell) -> Result<(), StandardError> {
        if self.len == CAP {
            return Err(StandardError::ConditionalSceneFull { capacity: CAP });
        }
        self.cells[self.len] = cell;
        self.len += 1;
        Ok(())
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn page(&self, offset: u16) -> RuntimeConditionalSceneChunk {
        let start = (offset as usize).min(self.len);
        let end = (start + CONDITIONAL_SCENE_CHUNK_SIZE).min(self.len);
        let mut chunk = RuntimeConditionalSceneChunk::new();
        for cell in &self.cells[start..end] {
            chunk.push(*cell).expect("page is chunk-bounded");
        }
        chunk
    }
}

impl<const CAP: usize> Default for RuntimeConditionalSceneTable<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) struct RuntimeConditionalSource<'a, Batteries: ?Sized, const CAP: usize> {
    pub(super) table: &'a RuntimeConditionalSceneTable<CAP>,
    pub(super) batteries: &'a Batteries,
}

impl<Context, Batteries, const CAP: usize> LightingSource<Rgb8, Context>
    for RuntimeConditionalSource<'_, Batteries, CAP>
where
    Context: LightingContextProvider,
    Batteries: BatteryStatusProvider + ?Sized,
{
    fn len(&self, input: &SourceRenderInput<'_, Context>) -> usize {
        self.table
            .as_slice()
            .iter()
            .filter(|cell| cell.conditions.matches(input.context, self.batteries))
            .count()
    }

    fn slot(&self, index: usize, input: &SourceRenderInput<'_, Context>) -> LedSlot {
        self.table
            .as_slice()
            .iter()
            .filter(|cell| cell.conditions.matches(input.context, self.batteries))
            .nth(index)
            .expect("LightingSource index must be below len")
            .slot
    }

    fn contribution(&mut self, index: usize, input: &SourceRenderInput<'_, Context>) -> Contribution<Rgb8> {
        let cell = self
            .table
            .as_slice()
            .iter()
            .filter(|cell| cell.conditions.matches(input.context, self.batteries))
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
