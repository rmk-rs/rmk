#[allow(unused_imports)]
use super::*;
use crate::lighting::Rgb8;
use crate::lighting::compositor::{Contribution, LightingSource, RenderInput as SourceRenderInput};
use crate::lighting::context::{LayerState, LightingContext, LightingContextProvider};
use crate::lighting::effect::{BuiltinEffect, LightingEffect};
use crate::lighting::source::LayerPolicy;
use crate::lighting::topology::LedSlot;

/// One durable scene cell: an effect bound to a local slot on one layer.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SceneTableCell {
    pub layer: u8,
    pub slot: LedSlot,
    pub effect: BuiltinEffect,
}

pub(super) const EMPTY_SCENE_CELL: SceneTableCell = SceneTableCell {
    layer: 0,
    slot: LedSlot(0),
    effect: BuiltinEffect::Solid { color: Rgb8::BLACK },
};

/// Fixed-capacity, owned scene chunk suitable for a bounded async mailbox.
/// One chunk is both a page of readback and a replacement-staging step.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SceneChunk {
    cells: [SceneTableCell; SCENE_CHUNK_SIZE],
    len: usize,
}

impl Default for SceneChunk {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneChunk {
    pub const fn new() -> Self {
        Self {
            cells: [EMPTY_SCENE_CELL; SCENE_CHUNK_SIZE],
            len: 0,
        }
    }

    pub fn push(&mut self, cell: SceneTableCell) -> Result<(), StandardError> {
        if self.len == SCENE_CHUNK_SIZE {
            return Err(StandardError::InvalidSceneRequest);
        }
        self.cells[self.len] = cell;
        self.len += 1;
        Ok(())
    }

    pub fn as_slice(&self) -> &[SceneTableCell] {
        &self.cells[..self.len]
    }
}

/// Distinct effects one [`StylePool`] can hold at once.
///
/// This is the only capacity interning adds, and it is a far looser bound than
/// a per-cell one: a keyboard paints many keys from a small set of colours, so
/// a config with hundreds of painted keys typically names a handful of effects.
pub const SCENE_STYLE_CAP: usize = 64;

/// Effects held once and referenced by index, so a scene cell can carry one
/// byte instead of a whole effect.
///
/// An inline [`BuiltinEffect`] is 16 bytes because blink and breathe each carry
/// two millisecond timers, so a solid colour pays for animation fields it never
/// uses and a cell costs 20 bytes. Interning moves the effect here and leaves
/// the cell at 4, which matters more than it looks: the same capacity sizes the
/// live tables, the replace staging buffer, the replica snapshot and the queued
/// mailbox commands, so every byte saved per cell is saved several times over.
///
/// Freeing needs no reference counts. When a reference drops or is overwritten
/// the caller reports it, the table is scanned for any remaining use, and the
/// entry is freed if there is none -- cheaper to run on a host-driven operation
/// than a set of counters is to keep correct. Occupancy is a bitmask, so a
/// freed entry leaves a hole the next allocation reuses and entries never move,
/// which means no cell's index ever needs rewriting.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct StylePool {
    styles: [BuiltinEffect; SCENE_STYLE_CAP],
    used: u64,
}

impl Default for StylePool {
    fn default() -> Self {
        Self::new()
    }
}

impl StylePool {
    pub const fn new() -> Self {
        Self {
            styles: [BuiltinEffect::Solid { color: Rgb8::BLACK }; SCENE_STYLE_CAP],
            used: 0,
        }
    }

    /// Distinct effects currently held. Bounded by [`SCENE_STYLE_CAP`].
    pub const fn len(&self) -> usize {
        self.used.count_ones() as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.used == 0
    }

    /// Index for `effect`, reusing an entry when it is already held. `None`
    /// only when a *new* effect would overflow, and then nothing is mutated.
    pub fn intern(&mut self, effect: BuiltinEffect) -> Option<u8> {
        if let Some(held) = self.find(effect) {
            return Some(held);
        }
        let free = (0..SCENE_STYLE_CAP).find(|&index| !self.holds(index))?;
        self.styles[free] = effect;
        self.used |= 1 << free;
        Some(free as u8)
    }

    pub fn get(&self, style: u8) -> Option<BuiltinEffect> {
        let index = style as usize;
        (index < SCENE_STYLE_CAP && self.holds(index)).then(|| self.styles[index])
    }

    /// Free `style` unless it is still referenced. The caller does the check,
    /// so the pool's borrow stays independent of whatever holds the references.
    pub fn release(&mut self, style: u8, still_referenced: bool) {
        if (style as usize) < SCENE_STYLE_CAP && !still_referenced {
            self.used &= !(1 << style as usize);
        }
    }

    pub fn clear(&mut self) {
        self.used = 0;
    }

    fn find(&self, effect: BuiltinEffect) -> Option<u8> {
        (0..SCENE_STYLE_CAP)
            .find(|&index| self.holds(index) && self.styles[index] == effect)
            .map(|index| index as u8)
    }

    const fn holds(&self, index: usize) -> bool {
        self.used & (1 << index) != 0
    }
}

/// A scene cell as stored: the effect lives in the table's [`StylePool`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct InternedSceneCell {
    layer: u8,
    slot: LedSlot,
    style: u8,
}

const EMPTY_INTERNED_CELL: InternedSceneCell = InternedSceneCell {
    layer: 0,
    slot: LedSlot(0),
    style: 0,
};

/// Fixed-capacity runtime scene table with layer-aware composition.
///
/// Cells are unique per `(layer, slot)` and grouped by layer. Order within a
/// layer is irrelevant because same-layer cells can never target the same
/// slot. `layer_offsets[layer]..layer_offsets[layer + 1]` is that layer's run.
#[derive(Copy, Clone, Debug)]
pub struct SceneTable<const CAP: usize> {
    cells: [InternedSceneCell; CAP],
    len: usize,
    layer_offsets: [u16; LayerState::CAPACITY as usize + 1],
    pub(super) pool: StylePool,
    policy: LayerPolicy,
}

impl<const CAP: usize> SceneTable<CAP> {
    pub const fn new() -> Self {
        Self {
            cells: [EMPTY_INTERNED_CELL; CAP],
            len: 0,
            layer_offsets: [0; LayerState::CAPACITY as usize + 1],
            pool: StylePool::new(),
            policy: LayerPolicy::ActiveStack,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn policy(&self) -> LayerPolicy {
        self.policy
    }

    pub fn set_policy(&mut self, policy: LayerPolicy) -> bool {
        let changed = self.policy != policy;
        self.policy = policy;
        changed
    }

    /// Insert or update the cell addressed by `(layer, slot)`.
    pub fn set(&mut self, cell: SceneTableCell) -> Result<(), StandardError> {
        let occupied = self.cells[..self.len]
            .iter()
            .position(|held| held.layer == cell.layer && held.slot == cell.slot);
        if occupied.is_none() && self.len == CAP {
            return Err(StandardError::SceneFull { capacity: CAP });
        }
        // Interning first means a full pool declines before anything is
        // mutated, so a rejected set leaves the table exactly as it was.
        let style = self.pool.intern(cell.effect).ok_or(StandardError::SceneFull {
            capacity: SCENE_STYLE_CAP,
        })?;
        let interned = InternedSceneCell {
            layer: cell.layer,
            slot: cell.slot,
            style,
        };
        match occupied {
            Some(index) => {
                let replaced = self.cells[index].style;
                self.cells[index] = interned;
                if replaced != style {
                    self.release(replaced);
                }
            }
            None => {
                let layer = cell.layer as usize;
                let insert_at = if layer < LayerState::CAPACITY as usize {
                    self.layer_offsets[layer + 1] as usize
                } else {
                    self.cells[..self.len].partition_point(|held| held.layer <= cell.layer)
                };
                self.cells.copy_within(insert_at..self.len, insert_at + 1);
                self.cells[insert_at] = interned;
                self.len += 1;
                if layer < LayerState::CAPACITY as usize {
                    for offset in &mut self.layer_offsets[layer + 1..] {
                        *offset += 1;
                    }
                }
            }
        }
        Ok(())
    }

    /// Free a style that no cell references any more.
    fn release(&mut self, style: u8) {
        let referenced = self.cells[..self.len].iter().any(|held| held.style == style);
        self.pool.release(style, referenced);
    }

    /// Painted cells, with their effects resolved out of the pool.
    pub fn iter(&self) -> impl Iterator<Item = SceneTableCell> + '_ {
        self.cells[..self.len].iter().filter_map(move |held| {
            self.pool.get(held.style).map(|effect| SceneTableCell {
                layer: held.layer,
                slot: held.slot,
                effect,
            })
        })
    }

    /// Remove the cell addressed by `(layer, slot)`.
    pub fn unset(&mut self, layer: u8, slot: LedSlot) -> bool {
        if let Some(index) = self.cells[..self.len]
            .iter()
            .position(|cell| cell.layer == layer && cell.slot == slot)
        {
            let removed = self.cells[index].style;
            self.len -= 1;
            self.cells.copy_within(index + 1..self.len + 1, index);
            if (layer as usize) < LayerState::CAPACITY as usize {
                for offset in &mut self.layer_offsets[layer as usize + 1..] {
                    *offset -= 1;
                }
            }
            self.release(removed);
            true
        } else {
            false
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.layer_offsets.fill(0);
        self.pool.clear();
    }

    /// One readback page starting at `offset`, clamped at the table's end.
    pub fn page(&self, offset: u16) -> SceneChunk {
        let mut chunk = SceneChunk::new();
        for cell in self.iter().skip(offset as usize).take(SCENE_CHUNK_SIZE) {
            chunk.push(cell).expect("page is chunk-bounded");
        }
        chunk
    }

    fn layer_bounds(&self, layer: u8) -> (usize, usize) {
        let layer = layer as usize;
        if layer < LayerState::CAPACITY as usize {
            (
                self.layer_offsets[layer] as usize,
                self.layer_offsets[layer + 1] as usize,
            )
        } else {
            let start = self.cells[..self.len].partition_point(|cell| cell.layer < layer as u8);
            let end = self.cells[..self.len].partition_point(|cell| cell.layer <= layer as u8);
            (start, end)
        }
    }

    fn cell_for_layer(&self, layer: u8, wanted: &mut usize) -> Option<SceneTableCell> {
        let (start, end) = self.layer_bounds(layer);
        if *wanted >= end - start {
            *wanted -= end - start;
            return None;
        }
        let held = self.cells[start + *wanted];
        self.pool.get(held.style).map(|effect| SceneTableCell {
            layer: held.layer,
            slot: held.slot,
            effect,
        })
    }

    fn cell_at(&self, context: &LightingContext, mut wanted: usize) -> SceneTableCell {
        let effective = context.layers.effective;
        match self.policy {
            LayerPolicy::EffectiveOnly => self.cell_for_layer(effective, &mut wanted),
            LayerPolicy::ActiveStack => {
                let default = context.layers.default;
                if let Some(cell) = self.cell_for_layer(default, &mut wanted) {
                    return cell;
                }
                for layer in 0..LayerState::CAPACITY {
                    if layer != default
                        && layer != effective
                        && context.layers.is_active(layer)
                        && let Some(cell) = self.cell_for_layer(layer, &mut wanted)
                    {
                        return cell;
                    }
                }
                if effective != default {
                    self.cell_for_layer(effective, &mut wanted)
                } else {
                    None
                }
            }
        }
        .expect("LightingSource index must be below len")
    }

    fn included_len(&self, context: &LightingContext) -> usize {
        let effective = context.layers.effective;
        match self.policy {
            LayerPolicy::EffectiveOnly => {
                let (start, end) = self.layer_bounds(effective);
                end - start
            }
            LayerPolicy::ActiveStack => {
                let default = context.layers.default;
                let mut included = 0;
                for layer in 0..LayerState::CAPACITY {
                    if layer == default || layer == effective || context.layers.is_active(layer) {
                        let layer = layer as usize;
                        included += self.layer_offsets[layer + 1] as usize - self.layer_offsets[layer] as usize;
                    }
                }
                if default >= LayerState::CAPACITY {
                    let (start, end) = self.layer_bounds(default);
                    included += end - start;
                }
                if effective >= LayerState::CAPACITY && effective != default {
                    let (start, end) = self.layer_bounds(effective);
                    included += end - start;
                }
                included
            }
        }
    }
}

impl<const CAP: usize> Default for SceneTable<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAP: usize> PartialEq for SceneTable<CAP> {
    fn eq(&self, other: &Self) -> bool {
        // Cells past `len` are stale storage, not state, and a style index is
        // an allocation detail -- two tables holding the same effects can
        // number them differently -- so this compares resolved cells.
        self.policy == other.policy && self.iter().eq(other.iter())
    }
}

impl<const CAP: usize> Eq for SceneTable<CAP> {}

impl<Context, const CAP: usize> LightingSource<Rgb8, Context> for SceneTable<CAP>
where
    Context: LightingContextProvider,
{
    fn len(&self, input: &SourceRenderInput<'_, Context>) -> usize {
        self.included_len(input.context.lighting_context())
    }

    fn slot(&self, index: usize, input: &SourceRenderInput<'_, Context>) -> LedSlot {
        self.cell_at(input.context.lighting_context(), index).slot
    }

    fn contribution(&mut self, index: usize, input: &SourceRenderInput<'_, Context>) -> Contribution<Rgb8> {
        Contribution::Opaque(
            self.cell_at(input.context.lighting_context(), index)
                .effect
                .sample(input.now_ms),
        )
    }
}
