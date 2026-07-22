use rmk_types::action::LightAction;

use super::effect::EffectSample;
use super::topology::LedSlot;

/// One source contribution. Transparent samples may carry a deadline because
/// a currently invisible effect can become opaque later.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Contribution<C> {
    Transparent { next_change_ms: Option<u64> },
    Opaque(EffectSample<C>),
}

/// Inputs common to all sources in one render transaction.
#[derive(Copy, Clone, Debug)]
pub struct RenderInput<'a, Context> {
    pub now_ms: u64,
    pub context: &'a Context,
}

/// Allocation-free pull interface for sparse or dense sources.
///
/// Targets are exposed separately so the transaction can validate the whole
/// source before changing the frame. That makes an invalid target atomic.
pub trait LightingSource<C, Context> {
    fn len(&self, input: &RenderInput<'_, Context>) -> usize;
    fn slot(&self, index: usize, input: &RenderInput<'_, Context>) -> LedSlot;
    /// Sample one previously validated target.
    ///
    /// `len` and `slot` must be pure for the duration of this call. Sampling
    /// is mutable so cached, RNG-backed, and otherwise stateful effects do not
    /// require interior mutability.
    fn contribution(&mut self, index: usize, input: &RenderInput<'_, Context>) -> Contribution<C>;

    fn is_empty(&self, input: &RenderInput<'_, Context>) -> bool {
        self.len(input) == 0
    }

    /// Claim a `LightAction` ahead of the engine's built-in handling.
    ///
    /// The standard engine offers each incoming action to its extension
    /// source first, so animated sources can own the RGB mode/value/speed
    /// keys instead of the uniform background. Return `true` to consume the
    /// action; the engine then re-renders. The default declines everything.
    fn handle_light_action(&mut self, _action: LightAction) -> bool {
        false
    }

    /// Describe this source's selectable content, if any.
    ///
    /// An animated extension source (an effect pack) advertises its effect
    /// and palette names here so hosts can render controls without
    /// compiled-in assumptions. The default — and sources that are not
    /// user-selectable — report nothing.
    fn extension_descriptor(&self) -> Option<ExtensionDescriptor> {
        None
    }

    /// Current selection and tuning, indexed against
    /// [`Self::extension_descriptor`]'s name lists.
    fn extension_state(&self) -> Option<ExtensionState> {
        None
    }

    /// Apply a host-selected state. Return `true` if accepted; the engine
    /// then re-renders and advances its revision. The default declines.
    fn apply_extension_state(&mut self, _state: ExtensionState) -> bool {
        false
    }
}

/// Static description of an extension source's selectable content.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ExtensionDescriptor {
    pub effects: &'static [&'static str],
    pub palettes: &'static [&'static str],
}

/// Runtime-adjustable state of an extension source. `effect` and `palette`
/// index into the descriptor's name lists; `value` and `speed` are 0-255.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ExtensionState {
    pub effect: u8,
    pub palette: u8,
    pub value: u8,
    pub speed: u8,
}

/// User-level transform applied after composition but before changed
/// detection. Hard electrical limits remain the output driver's job.
pub trait OutputTransform<C> {
    fn transform(&mut self, slot: LedSlot, color: C) -> C;

    fn next_change_ms(&self, _slot: LedSlot, _before: C, _after: C, source_next_change_ms: Option<u64>) -> Option<u64> {
        source_next_change_ms
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub struct IdentityTransform;

impl<C> OutputTransform<C> for IdentityTransform {
    fn transform(&mut self, _slot: LedSlot, color: C) -> C {
        color
    }
}

/// Standard dense logical frame. Its slot order has semantic meaning only in
/// conjunction with a validated topology; it is never physical chain order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogicalFrame<C, const N: usize> {
    pixels: [C; N],
}

impl<C: Copy, const N: usize> LogicalFrame<C, N> {
    pub const fn new(fill: C) -> Self {
        Self { pixels: [fill; N] }
    }

    pub const fn as_array(&self) -> &[C; N] {
        &self.pixels
    }

    pub fn as_mut_array(&mut self) -> &mut [C; N] {
        &mut self.pixels
    }

    pub fn as_slice(&self) -> &[C] {
        &self.pixels
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    PriorityRegression { previous: u8, attempted: u8 },
    SlotOutOfRange { slot: LedSlot, frame_len: usize },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RenderResult {
    pub changed: bool,
    /// Absolute deadline. `None` means no timer should be armed.
    pub next_wake_ms: Option<u64>,
}

/// Keeps the last frame successfully presented by the output.
///
/// [`Compositor::commit`] is deliberately separate from rendering. A service
/// commits only after a successful driver write, so failures remain dirty and
/// can be retried without waiting for an unrelated event.
pub struct Compositor<C, const N: usize> {
    committed: LogicalFrame<C, N>,
    has_committed: bool,
}

impl<C: Copy + Eq, const N: usize> Compositor<C, N> {
    pub const fn new(fill: C) -> Self {
        Self {
            committed: LogicalFrame::new(fill),
            has_committed: false,
        }
    }

    pub fn begin<'a, 'context, Context>(
        &'a self,
        now_ms: u64,
        context: &'context Context,
        fill: C,
        frame: &'a mut LogicalFrame<C, N>,
    ) -> RenderTransaction<'a, 'context, C, Context, N> {
        frame.pixels.fill(fill);
        RenderTransaction {
            compositor: self,
            frame,
            deadlines: [None; N],
            input: RenderInput { now_ms, context },
            last_priority: None,
        }
    }

    pub fn commit(&mut self, frame: &LogicalFrame<C, N>) {
        self.committed.pixels.copy_from_slice(&frame.pixels);
        self.has_committed = true;
    }

    pub fn has_committed(&self) -> bool {
        self.has_committed
    }
}

pub struct RenderTransaction<'a, 'context, C, Context, const N: usize> {
    compositor: &'a Compositor<C, N>,
    frame: &'a mut LogicalFrame<C, N>,
    deadlines: [Option<u64>; N],
    input: RenderInput<'context, Context>,
    last_priority: Option<u8>,
}

impl<C: Copy + Eq, Context, const N: usize> RenderTransaction<'_, '_, C, Context, N> {
    /// Apply a source. Priorities must be nondecreasing; equal priorities use
    /// stable call order, with the later opaque contribution winning.
    pub fn apply(&mut self, priority: u8, source: &mut impl LightingSource<C, Context>) -> Result<(), RenderError> {
        if let Some(previous) = self.last_priority
            && priority < previous
        {
            return Err(RenderError::PriorityRegression {
                previous,
                attempted: priority,
            });
        }

        // Validate every target before mutating anything.
        for index in 0..source.len(&self.input) {
            let slot = source.slot(index, &self.input);
            if slot.index() >= N {
                return Err(RenderError::SlotOutOfRange { slot, frame_len: N });
            }
        }

        for index in 0..source.len(&self.input) {
            let slot = source.slot(index, &self.input).index();
            match source.contribution(index, &self.input) {
                Contribution::Transparent { next_change_ms } => {
                    self.deadlines[slot] =
                        earliest(self.deadlines[slot], future_deadline(self.input.now_ms, next_change_ms));
                }
                Contribution::Opaque(sample) => {
                    self.frame.pixels[slot] = sample.color;
                    // Opaque replacement intentionally erases all deadlines
                    // belonging to sources hidden below this winner.
                    self.deadlines[slot] = future_deadline(self.input.now_ms, sample.next_change_ms);
                }
            }
        }
        self.last_priority = Some(priority);
        Ok(())
    }

    pub fn finish(self) -> RenderResult {
        self.finish_with(&mut IdentityTransform)
    }

    pub fn finish_with(self, transform: &mut impl OutputTransform<C>) -> RenderResult {
        let now_ms = self.input.now_ms;
        let mut next_wake_ms = None;
        for slot_index in 0..N {
            let slot = LedSlot::from_index(slot_index);
            let before = self.frame.pixels[slot_index];
            let after = transform.transform(slot, before);
            self.frame.pixels[slot_index] = after;
            let transformed_deadline = transform.next_change_ms(slot, before, after, self.deadlines[slot_index]);
            next_wake_ms = earliest(next_wake_ms, future_deadline(now_ms, transformed_deadline));
        }

        RenderResult {
            changed: !self.compositor.has_committed || self.frame.pixels != self.compositor.committed.pixels,
            next_wake_ms,
        }
    }
}

fn future_deadline(now_ms: u64, deadline: Option<u64>) -> Option<u64> {
    match deadline {
        Some(deadline) if deadline > now_ms => Some(deadline),
        Some(_) => now_ms.checked_add(1),
        None => None,
    }
}

fn earliest(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::Rgb8;
    use super::*;

    struct Source<const M: usize>([(LedSlot, Contribution<Rgb8>); M]);

    impl<Context, const M: usize> LightingSource<Rgb8, Context> for Source<M> {
        fn len(&self, _: &RenderInput<'_, Context>) -> usize {
            M
        }
        fn slot(&self, index: usize, _: &RenderInput<'_, Context>) -> LedSlot {
            self.0[index].0
        }
        fn contribution(&mut self, index: usize, _: &RenderInput<'_, Context>) -> Contribution<Rgb8> {
            self.0[index].1
        }
    }

    fn opaque(slot: usize, color: Rgb8, next: Option<u64>) -> (LedSlot, Contribution<Rgb8>) {
        (
            LedSlot::from_index(slot),
            Contribution::Opaque(EffectSample {
                color,
                next_change_ms: next,
            }),
        )
    }

    #[test]
    fn transparency_ties_and_occluded_deadlines_are_deterministic() {
        let mut compositor = Compositor::<Rgb8, 2>::new(Rgb8::BLACK);
        let mut frame = LogicalFrame::new(Rgb8::BLACK);
        let context = ();
        let mut low = Source([opaque(0, Rgb8::new(1, 0, 0), Some(10))]);
        let mut transparent = Source([(
            LedSlot(0),
            Contribution::Transparent {
                next_change_ms: Some(20),
            },
        )]);
        let mut high = Source([opaque(0, Rgb8::new(0, 1, 0), None)]);
        let mut tx = compositor.begin(0, &context, Rgb8::BLACK, &mut frame);
        tx.apply(1, &mut low).unwrap();
        tx.apply(1, &mut transparent).unwrap();
        tx.apply(2, &mut high).unwrap();
        let result = tx.finish();
        assert_eq!(frame.as_slice(), &[Rgb8::new(0, 1, 0), Rgb8::BLACK]);
        assert_eq!(result.next_wake_ms, None);
        assert!(result.changed);

        compositor.commit(&frame);
    }

    #[test]
    fn failed_apply_is_atomic_and_priority_regression_does_not_advance_order() {
        let compositor = Compositor::<Rgb8, 1>::new(Rgb8::BLACK);
        let mut frame = LogicalFrame::new(Rgb8::BLACK);
        let mut good = Source([opaque(0, Rgb8::new(3, 0, 0), None)]);
        let mut bad = Source([opaque(0, Rgb8::new(9, 0, 0), None), opaque(2, Rgb8::new(8, 0, 0), None)]);
        let mut tx = compositor.begin(0, &(), Rgb8::BLACK, &mut frame);
        assert!(matches!(tx.apply(5, &mut bad), Err(RenderError::SlotOutOfRange { .. })));
        tx.apply(4, &mut good).unwrap();
        assert_eq!(tx.finish().next_wake_ms, None);
        assert_eq!(frame.as_slice(), &[Rgb8::new(3, 0, 0)]);
    }

    #[test]
    fn uncommitted_output_remains_changed_for_driver_retry() {
        let mut compositor = Compositor::<Rgb8, 1>::new(Rgb8::BLACK);
        let mut frame = LogicalFrame::new(Rgb8::BLACK);
        let mut source = Source([opaque(0, Rgb8::new(1, 2, 3), None)]);
        for _ in 0..2 {
            let mut tx = compositor.begin(0, &(), Rgb8::BLACK, &mut frame);
            tx.apply(0, &mut source).unwrap();
            assert!(tx.finish().changed);
        }
        compositor.commit(&frame);
        let mut tx = compositor.begin(0, &(), Rgb8::BLACK, &mut frame);
        tx.apply(0, &mut source).unwrap();
        assert!(!tx.finish().changed);
    }
}
