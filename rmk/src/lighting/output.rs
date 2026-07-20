//! Allocation-free routing from semantic lighting frames to physical outputs.
//!
//! [`LogicalFrame`] order is the dense [`LedSlot`] order defined by a
//! [`LightingTopology`]. It is deliberately unrelated to electrical chain
//! order. This module joins a logical frame with validated [`LightingRouting`]
//! and visits pixels in physical output order without depending on a HAL or
//! concrete driver.

use super::color::Rgb8;
use super::compositor::{LogicalFrame, OutputTransform};
use super::topology::{
    LedId, LedSlot, LightingNodeId, LightingRouting, LightingTopology, OutputCapabilities, OutputId, OutputMetadata,
    ValidationError, validate,
};

/// A topology and routing pair whose complete structural contract has been
/// validated.
///
/// Construction checks that every semantic slot has exactly one route, every
/// physical address is unique and in bounds, and complete outputs have no
/// holes. Keeping this proof object separate prevents frame delivery from
/// repeating quadratic validation on every write.
#[derive(Clone, Copy, Debug)]
pub struct ValidatedRouting<'a> {
    topology: LightingTopology<'a>,
    routing: LightingRouting<'a>,
}

impl<'a> ValidatedRouting<'a> {
    pub fn new(topology: LightingTopology<'a>, routing: LightingRouting<'a>) -> Result<Self, ValidationError> {
        validate(&topology, &routing)?;
        Ok(Self { topology, routing })
    }

    pub const fn topology(&self) -> &LightingTopology<'a> {
        &self.topology
    }

    pub const fn routing(&self) -> &LightingRouting<'a> {
        &self.routing
    }

    /// Visit a standard logical frame in deterministic physical order.
    pub fn visit_frame<C: Copy, const N: usize, S: RoutedFrameSink<C>>(
        &self,
        frame: &LogicalFrame<C, N>,
        selection: OutputSelection,
        sink: &mut S,
    ) -> Result<VisitSummary, RouteError<S::Error>> {
        self.visit_slice(frame.as_slice(), selection, sink)
    }

    /// Visit any logical slice whose indices use this topology's slot order.
    ///
    /// Outputs are visited in `LightingRouting::outputs` order and pixels in
    /// ascending physical index. Route table order and semantic slot order do
    /// not affect delivery. Sparse-output holes simply produce no pixel call.
    /// The frame length is checked before the first sink callback.
    pub fn visit_slice<C: Copy, S: RoutedFrameSink<C>>(
        &self,
        frame: &[C],
        selection: OutputSelection,
        sink: &mut S,
    ) -> Result<VisitSummary, RouteError<S::Error>> {
        let expected = self.topology.len();
        if frame.len() != expected {
            return Err(RouteError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }

        let mut summary = VisitSummary::default();
        for output in self.routing.outputs {
            if !selection.matches(output.capabilities) {
                continue;
            }

            sink.begin_output(*output).map_err(RouteError::Sink)?;
            summary.outputs += 1;

            for physical_index in 0..output.pixel_count {
                let Some(route) = self.routing.routes.iter().find(|route| {
                    route.node == output.node && route.output == output.id && route.physical_index == physical_index
                }) else {
                    // Validated sparse outputs may intentionally contain holes.
                    continue;
                };
                let slot_index = route.slot.index();
                let led = self
                    .topology
                    .led(route.slot)
                    .expect("validated routes always reference a topology slot");
                sink.write_pixel(RoutedPixel {
                    slot: route.slot,
                    led_id: led.id,
                    node: output.node,
                    output: output.id,
                    physical_index,
                    capabilities: output.capabilities,
                    value: frame[slot_index],
                })
                .map_err(RouteError::Sink)?;
                summary.pixels += 1;
            }

            sink.end_output(*output).map_err(RouteError::Sink)?;
        }

        Ok(summary)
    }
}

/// Capability filter applied before an output is presented to a sink.
///
/// `required` bits must all be present. When `any` is non-empty, at least one
/// of those bits must also be present. This supports, for example, selecting
/// addressable RGB/RGBW outputs separately from binary or intensity outputs
/// while retaining one heterogeneous route table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputSelection {
    required: OutputCapabilities,
    any: OutputCapabilities,
}

impl OutputSelection {
    pub const ALL: Self = Self {
        required: OutputCapabilities::NONE,
        any: OutputCapabilities::NONE,
    };

    pub const fn requiring(required: OutputCapabilities) -> Self {
        Self {
            required,
            any: OutputCapabilities::NONE,
        }
    }

    pub const fn any_of(any: OutputCapabilities) -> Self {
        Self {
            required: OutputCapabilities::NONE,
            any,
        }
    }

    pub const fn requiring_any(required: OutputCapabilities, any: OutputCapabilities) -> Self {
        Self { required, any }
    }

    pub const fn required(self) -> OutputCapabilities {
        self.required
    }

    pub const fn any(self) -> OutputCapabilities {
        self.any
    }

    pub const fn matches(self, capabilities: OutputCapabilities) -> bool {
        capabilities.contains(self.required) && (self.any.bits() == 0 || capabilities.intersects(self.any))
    }
}

impl Default for OutputSelection {
    fn default() -> Self {
        Self::ALL
    }
}

/// One logical value annotated with both semantic and physical identity.
///
/// A heterogeneous sink can choose RGB, RGBW, intensity, or binary conversion
/// from `capabilities` and store the result in board-owned output buffers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoutedPixel<C> {
    pub slot: LedSlot,
    pub led_id: LedId,
    pub node: LightingNodeId,
    pub output: OutputId,
    pub physical_index: u16,
    pub capabilities: OutputCapabilities,
    pub value: C,
}

/// Hardware-independent consumer of physically addressed frame values.
///
/// Implementations normally fill caller-owned fixed arrays or forward each
/// output to a board-specific adapter. The core does not prescribe a common
/// physical pixel type: the output metadata and every pixel carry capability
/// information so the sink owns conversion policy.
pub trait RoutedFrameSink<C> {
    type Error;

    fn begin_output(&mut self, _output: OutputMetadata) -> Result<(), Self::Error> {
        Ok(())
    }

    fn write_pixel(&mut self, pixel: RoutedPixel<C>) -> Result<(), Self::Error>;

    fn end_output(&mut self, _output: OutputMetadata) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VisitSummary {
    pub outputs: usize,
    pub pixels: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteError<E> {
    FrameLength { expected: usize, actual: usize },
    Sink(E),
}

/// User brightness applied after composition and before changed detection.
///
/// This is intentionally not a hardware safety limit. Drivers must still
/// enforce their immutable channel/current policy after routing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrightnessTransform {
    level: u8,
}

impl BrightnessTransform {
    pub const OFF: Self = Self::new(0);
    pub const FULL: Self = Self::new(u8::MAX);

    pub const fn new(level: u8) -> Self {
        Self { level }
    }

    pub const fn level(self) -> u8 {
        self.level
    }

    pub fn set_level(&mut self, level: u8) {
        self.level = level;
    }
}

impl Default for BrightnessTransform {
    fn default() -> Self {
        Self::FULL
    }
}

impl OutputTransform<Rgb8> for BrightnessTransform {
    fn transform(&mut self, _slot: LedSlot, color: Rgb8) -> Rgb8 {
        color.scale(self.level)
    }

    fn next_change_ms(
        &self,
        _slot: LedSlot,
        _before: Rgb8,
        _after: Rgb8,
        source_next_change_ms: Option<u64>,
    ) -> Option<u64> {
        // Zero brightness makes every possible source value visibly black,
        // so source animation cannot change the transformed frame.
        (self.level != 0).then_some(source_next_change_ms).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::super::compositor::{Compositor, Contribution, LightingSource, RenderInput};
    use super::super::effect::EffectSample;
    use super::super::topology::{LedMetadata, MatrixSize, OutputCoverage, PhysicalLayout, PhysicalRoute, ZoneSpan};
    use super::*;

    const RGB_ADDRESSABLE: OutputCapabilities = OutputCapabilities::RGB.union(OutputCapabilities::ADDRESSABLE);

    static LEDS: [LedMetadata; 4] = [
        LedMetadata {
            id: LedId(100),
            key: None,
            position: None,
            zones: ZoneSpan::EMPTY,
        },
        LedMetadata {
            id: LedId(200),
            key: None,
            position: None,
            zones: ZoneSpan::EMPTY,
        },
        LedMetadata {
            id: LedId(300),
            key: None,
            position: None,
            zones: ZoneSpan::EMPTY,
        },
        LedMetadata {
            id: LedId(400),
            key: None,
            position: None,
            zones: ZoneSpan::EMPTY,
        },
    ];

    static OUTPUTS: [OutputMetadata; 3] = [
        OutputMetadata {
            node: LightingNodeId(0),
            id: OutputId(0),
            pixel_count: 2,
            capabilities: RGB_ADDRESSABLE,
            coverage: OutputCoverage::Complete,
        },
        OutputMetadata {
            node: LightingNodeId(0),
            id: OutputId(1),
            pixel_count: 1,
            capabilities: OutputCapabilities::BINARY,
            coverage: OutputCoverage::Complete,
        },
        OutputMetadata {
            node: LightingNodeId(1),
            id: OutputId(0),
            pixel_count: 1,
            capabilities: OutputCapabilities::INTENSITY,
            coverage: OutputCoverage::Complete,
        },
    ];

    // Deliberately neither semantic-slot nor physical-output order.
    static ROUTES: [PhysicalRoute; 4] = [
        PhysicalRoute {
            slot: LedSlot(0),
            node: LightingNodeId(1),
            output: OutputId(0),
            physical_index: 0,
        },
        PhysicalRoute {
            slot: LedSlot(2),
            node: LightingNodeId(0),
            output: OutputId(1),
            physical_index: 0,
        },
        PhysicalRoute {
            slot: LedSlot(1),
            node: LightingNodeId(0),
            output: OutputId(0),
            physical_index: 1,
        },
        PhysicalRoute {
            slot: LedSlot(3),
            node: LightingNodeId(0),
            output: OutputId(0),
            physical_index: 0,
        },
    ];

    fn topology() -> LightingTopology<'static> {
        LightingTopology {
            matrix: MatrixSize::new(0, 0),
            keys: &[],
            physical_layout: PhysicalLayout::EMPTY,
            leds: &LEDS,
            zones: &[],
            zone_memberships: &[],
        }
    }

    fn validated() -> ValidatedRouting<'static> {
        ValidatedRouting::new(
            topology(),
            LightingRouting {
                outputs: &OUTPUTS,
                routes: &ROUTES,
            },
        )
        .unwrap()
    }

    #[derive(Default)]
    struct RecordingSink<C> {
        begun: std::vec::Vec<(LightingNodeId, OutputId)>,
        pixels: std::vec::Vec<RoutedPixel<C>>,
        ended: std::vec::Vec<(LightingNodeId, OutputId)>,
    }

    impl<C> RoutedFrameSink<C> for RecordingSink<C> {
        type Error = core::convert::Infallible;

        fn begin_output(&mut self, output: OutputMetadata) -> Result<(), Self::Error> {
            self.begun.push((output.node, output.id));
            Ok(())
        }

        fn write_pixel(&mut self, pixel: RoutedPixel<C>) -> Result<(), Self::Error> {
            self.pixels.push(pixel);
            Ok(())
        }

        fn end_output(&mut self, output: OutputMetadata) -> Result<(), Self::Error> {
            self.ended.push((output.node, output.id));
            Ok(())
        }
    }

    #[test]
    fn physical_visit_order_is_independent_of_semantic_and_route_order() {
        let frame = LogicalFrame::new(0u16);
        let mut values = frame;
        *values.as_mut_array() = [10, 20, 30, 40];
        let mut sink = RecordingSink::default();

        let summary = validated()
            .visit_frame(&values, OutputSelection::ALL, &mut sink)
            .unwrap();

        assert_eq!(summary, VisitSummary { outputs: 3, pixels: 4 });
        assert_eq!(
            sink.begun,
            [
                (LightingNodeId(0), OutputId(0)),
                (LightingNodeId(0), OutputId(1)),
                (LightingNodeId(1), OutputId(0))
            ]
        );
        assert_eq!(
            sink.pixels
                .iter()
                .map(|pixel| (pixel.slot, pixel.led_id, pixel.physical_index, pixel.value))
                .collect::<std::vec::Vec<_>>(),
            [
                (LedSlot(3), LedId(400), 0, 40),
                (LedSlot(1), LedId(200), 1, 20),
                (LedSlot(2), LedId(300), 0, 30),
                (LedSlot(0), LedId(100), 0, 10),
            ]
        );
        assert_eq!(sink.ended, sink.begun);
    }

    #[test]
    fn capability_selection_handles_heterogeneous_outputs() {
        let frame = [10u8, 20, 30, 40];

        let mut rgb = RecordingSink::default();
        let summary = validated()
            .visit_slice(&frame, OutputSelection::requiring(OutputCapabilities::RGB), &mut rgb)
            .unwrap();
        assert_eq!(summary, VisitSummary { outputs: 1, pixels: 2 });
        assert!(
            rgb.pixels
                .iter()
                .all(|pixel| pixel.capabilities.contains(OutputCapabilities::RGB))
        );

        let mut mono = RecordingSink::default();
        let summary = validated()
            .visit_slice(
                &frame,
                OutputSelection::any_of(OutputCapabilities::BINARY.union(OutputCapabilities::INTENSITY)),
                &mut mono,
            )
            .unwrap();
        assert_eq!(summary, VisitSummary { outputs: 2, pixels: 2 });
        assert_eq!(
            mono.pixels
                .iter()
                .map(|pixel| pixel.value)
                .collect::<std::vec::Vec<_>>(),
            [30, 10]
        );
    }

    #[test]
    fn frame_length_is_rejected_before_sink_observes_output() {
        let mut sink = RecordingSink::default();
        assert_eq!(
            validated().visit_slice(&[1u8, 2, 3], OutputSelection::ALL, &mut sink),
            Err(RouteError::FrameLength { expected: 4, actual: 3 })
        );
        assert!(sink.begun.is_empty());
        assert!(sink.pixels.is_empty());
    }

    #[test]
    fn invalid_routing_cannot_construct_proof_object() {
        let duplicate = [ROUTES[0], ROUTES[0], ROUTES[2], ROUTES[3]];
        assert!(matches!(
            ValidatedRouting::new(
                topology(),
                LightingRouting {
                    outputs: &OUTPUTS,
                    routes: &duplicate,
                },
            ),
            Err(ValidationError::DuplicateRouteForSlot { .. })
        ));
    }

    #[test]
    fn sparse_physical_holes_are_skipped_without_placeholder_values() {
        let leds = [LEDS[0]];
        let outputs = [OutputMetadata {
            node: LightingNodeId(9),
            id: OutputId(4),
            pixel_count: 3,
            capabilities: OutputCapabilities::INTENSITY,
            coverage: OutputCoverage::Sparse,
        }];
        let routes = [PhysicalRoute {
            slot: LedSlot(0),
            node: LightingNodeId(9),
            output: OutputId(4),
            physical_index: 2,
        }];
        let routing = ValidatedRouting::new(
            LightingTopology {
                matrix: MatrixSize::new(0, 0),
                keys: &[],
                physical_layout: PhysicalLayout::EMPTY,
                leds: &leds,
                zones: &[],
                zone_memberships: &[],
            },
            LightingRouting {
                outputs: &outputs,
                routes: &routes,
            },
        )
        .unwrap();
        let mut sink = RecordingSink::default();
        assert_eq!(
            routing.visit_slice(&[77u8], OutputSelection::ALL, &mut sink).unwrap(),
            VisitSummary { outputs: 1, pixels: 1 }
        );
        assert_eq!(sink.pixels[0].physical_index, 2);
    }

    struct AnimatedSource;

    impl<Context> LightingSource<Rgb8, Context> for AnimatedSource {
        fn len(&self, _: &RenderInput<'_, Context>) -> usize {
            1
        }

        fn slot(&self, _: usize, _: &RenderInput<'_, Context>) -> LedSlot {
            LedSlot(0)
        }

        fn contribution(&mut self, _: usize, _: &RenderInput<'_, Context>) -> Contribution<Rgb8> {
            Contribution::Opaque(EffectSample {
                color: Rgb8::new(255, 128, 64),
                next_change_ms: Some(25),
            })
        }
    }

    #[test]
    fn brightness_is_applied_before_diffing_and_zero_suppresses_deadlines() {
        let compositor = Compositor::<Rgb8, 1>::new(Rgb8::BLACK);
        let mut frame = LogicalFrame::new(Rgb8::BLACK);
        let mut half = BrightnessTransform::new(128);
        let mut source = AnimatedSource;
        let mut tx = compositor.begin(0, &(), Rgb8::BLACK, &mut frame);
        tx.apply(0, &mut source).unwrap();
        let result = tx.finish_with(&mut half);
        assert_eq!(frame.as_slice(), &[Rgb8::new(128, 64, 32)]);
        assert_eq!(result.next_wake_ms, Some(25));

        let mut off = BrightnessTransform::OFF;
        let mut tx = compositor.begin(0, &(), Rgb8::BLACK, &mut frame);
        tx.apply(0, &mut source).unwrap();
        let result = tx.finish_with(&mut off);
        assert_eq!(frame.as_slice(), &[Rgb8::BLACK]);
        assert_eq!(result.next_wake_ms, None);
    }

    #[test]
    fn brightness_level_is_runtime_mutable() {
        let color = Rgb8::new(255, 100, 1);
        let mut brightness = BrightnessTransform::OFF;
        assert_eq!(brightness.transform(LedSlot(0), color), Rgb8::BLACK);
        brightness.set_level(u8::MAX);
        assert_eq!(brightness.level(), u8::MAX);
        assert_eq!(brightness.transform(LedSlot(0), color), color);
    }
}
