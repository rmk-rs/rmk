#[allow(unused_imports)]
use super::*;
use crate::lighting::Rgb8;
use crate::lighting::compositor::{Contribution, LightingSource, RenderInput as SourceRenderInput};
use crate::lighting::effect::{BuiltinEffect, LightingEffect};
use crate::lighting::topology::LedSlot;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BackgroundMode {
    Solid,
    Breathe,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct BackgroundState {
    pub enabled: bool,
    pub hue: u8,
    pub saturation: u8,
    pub value: u8,
    pub speed: u8,
    pub mode: BackgroundMode,
}

impl Default for BackgroundState {
    fn default() -> Self {
        Self {
            enabled: true,
            hue: 0,
            saturation: 0,
            value: 32,
            speed: 128,
            mode: BackgroundMode::Solid,
        }
    }
}

/// Atomic partial update of the designated background.
///
/// Protocol adapters use this instead of a `ReadState` followed by
/// `SetBackground`: the lighting engine remains the sole mutable owner and
/// concurrent callers cannot overwrite fields changed between two mailbox
/// requests.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct BackgroundPatch {
    pub enabled: Option<bool>,
    pub hue: Option<u8>,
    pub saturation: Option<u8>,
    pub value: Option<u8>,
    pub speed: Option<u8>,
    pub mode: Option<BackgroundMode>,
}

impl BackgroundPatch {
    pub const fn apply_to(self, state: &mut BackgroundState) {
        if let Some(enabled) = self.enabled {
            state.enabled = enabled;
        }
        if let Some(hue) = self.hue {
            state.hue = hue;
        }
        if let Some(saturation) = self.saturation {
            state.saturation = saturation;
        }
        if let Some(value) = self.value {
            state.value = value;
        }
        if let Some(speed) = self.speed {
            state.speed = speed;
        }
        if let Some(mode) = self.mode {
            state.mode = mode;
        }
    }
}

/// Built-in designated background controlled by RGB/Vial-compatible fields.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct UniformBackground<const N: usize> {
    pub(super) state: BackgroundState,
}

impl<const N: usize> UniformBackground<N> {
    pub const fn new(state: BackgroundState) -> Self {
        Self { state }
    }

    pub const fn state(&self) -> BackgroundState {
        self.state
    }

    pub fn set_state(&mut self, state: BackgroundState) {
        self.state = state;
    }

    fn effect(&self) -> BuiltinEffect {
        let color = if self.state.enabled {
            hsv(self.state.hue, self.state.saturation, self.state.value)
        } else {
            Rgb8::BLACK
        };
        match self.state.mode {
            BackgroundMode::Solid => BuiltinEffect::Solid { color },
            BackgroundMode::Breathe => BuiltinEffect::Breathe {
                color,
                period_ms: 250 + ((u8::MAX - self.state.speed) as u32 * 3_750 / 255),
                phase_ms: 0,
                step_ms: 16,
            },
        }
    }
}

impl<Context, const N: usize> LightingSource<Rgb8, Context> for UniformBackground<N> {
    fn len(&self, _: &SourceRenderInput<'_, Context>) -> usize {
        N
    }

    fn slot(&self, index: usize, _: &SourceRenderInput<'_, Context>) -> LedSlot {
        LedSlot::from_index(index)
    }

    fn contribution(&mut self, _: usize, input: &SourceRenderInput<'_, Context>) -> Contribution<Rgb8> {
        Contribution::Opaque(self.effect().sample(input.now_ms))
    }
}

/// Zero-sized source used when a board does not need an extension band.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct EmptySource;

impl<C, Context> LightingSource<C, Context> for EmptySource {
    fn len(&self, _: &SourceRenderInput<'_, Context>) -> usize {
        0
    }

    fn slot(&self, _: usize, _: &SourceRenderInput<'_, Context>) -> LedSlot {
        unreachable!("EmptySource has no targets")
    }

    fn contribution(&mut self, _: usize, _: &SourceRenderInput<'_, Context>) -> Contribution<C> {
        unreachable!("EmptySource has no samples")
    }
}
