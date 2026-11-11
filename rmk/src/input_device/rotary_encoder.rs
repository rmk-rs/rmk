//! General rotary encoder
//!
//! The rotary encoder implementation is adapted from: <https://github.com/leshow/rotary-encoder-hal/blob/master/src/lib.rs>
use embedded_hal::digital::InputPin;
#[cfg(feature = "async_matrix")]
use embedded_hal_async::digital::Wait;
use postcard::experimental::max_size::MaxSize;
use rmk_macro::input_device;
use serde::{Deserialize, Serialize};

use crate::event::KeyboardEvent;

/// Holds current/old state and both [`InputPin`](https://docs.rs/embedded-hal/latest/embedded_hal/digital/trait.InputPin.html)
#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[input_device(publish = KeyboardEvent)]
pub struct RotaryEncoder<
    #[cfg(feature = "async_matrix")] A: InputPin + Wait,
    #[cfg(not(feature = "async_matrix"))] A: InputPin,
    #[cfg(feature = "async_matrix")] B: InputPin + Wait,
    #[cfg(not(feature = "async_matrix"))] B: InputPin,
    P: Phase,
> {
    pin_a: A,
    pin_b: B,
    state: Option<u8>,
    phase: P,
    /// The index of the rotary encoder
    id: u8,
    /// The last action of the rotary encoder.
    /// When it's not `None`, the rotary encoder needs to emit a release event.
    last_action: Option<Direction>,
    /// Timestamp of the last emitted direction event, used for debounce.
    last_event_time: Option<embassy_time::Instant>,
    /// Minimum interval in milliseconds between emitted direction events.
    /// Contact bounce on mechanical encoders produces spurious quadrature edges;
    /// this suppresses them without altering the Phase/Resolution logic.
    debounce_ms: u16,
}

/// The encoder direction is either `Clockwise`, `CounterClockwise`, or `None`
#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Direction {
    /// A clockwise turn
    Clockwise,
    /// A counterclockwise turn
    CounterClockwise,
    /// No change
    None,
}

/// Allows customizing which Quadrature Phases should be considered movements
/// and in which direction or ignored.
pub trait Phase {
    /// Given the current state `s`, return the direction.
    fn direction(&mut self, s: u8) -> Direction;
}

/// Default implementation of `Phase`.
pub struct DefaultPhase;

/// The useful values of `s` are:
/// - 0b0001 | 0b0111 | 0b1000 | 0b1110
/// - 0b0010 | 0b0100 | 0b1011 | 0b1101
impl Phase for DefaultPhase {
    fn direction(&mut self, s: u8) -> Direction {
        match s {
            0b0001 | 0b0111 | 0b1000 | 0b1110 => Direction::Clockwise,
            0b0010 | 0b0100 | 0b1011 | 0b1101 => Direction::CounterClockwise,
            _ => Direction::None,
        }
    }
}

/// Phase implementation for E8H7 encoder
pub struct E8H7Phase;
impl Phase for E8H7Phase {
    fn direction(&mut self, s: u8) -> Direction {
        match s {
            0b0010 | 0b1101 => Direction::Clockwise,
            0b0001 | 0b1110 => Direction::CounterClockwise,
            _ => Direction::None,
        }
    }
}

/// Phase implementation based on configurable resolution
pub struct ResolutionPhase {
    resolution: i16,
    lut: [i8; 16],
    current_pulses: i16,
}

impl ResolutionPhase {
    pub fn new(resolution: u8, reverse: bool) -> Self {
        assert!(resolution > 0, "ResolutionPhase: resolution must be nonzero");
        // Each entry corresponds to a state transition and provides +1, -1, or 0 pulse
        let mut lut = [0, -1, 1, 0, 1, 0, 0, -1, -1, 0, 0, 1, 0, 1, -1, 0];
        if reverse {
            lut = lut.map(|x| -x);
        }
        Self {
            resolution: i16::from(resolution),
            lut,
            current_pulses: 0,
        }
    }

    pub fn new_with_detent_and_pulse(detent: u8, pulse: u8, reverse: bool) -> Self {
        assert!(detent > 0, "ResolutionPhase: detent must be nonzero");
        let resolution = u8::try_from(u16::from(pulse) * 4 / u16::from(detent))
            .expect("ResolutionPhase: pulse * 4 / detent must fit in u8");
        Self::new(resolution, reverse)
    }
}

impl Phase for ResolutionPhase {
    fn direction(&mut self, s: u8) -> Direction {
        // Only proceed if there was a state change
        if (s & 0xC) != (s & 0x3) {
            // Add pulse value from the lookup table
            self.current_pulses += i16::from(self.lut[s as usize & 0xF]);
            // Check if we've reached the resolution threshold
            if self.current_pulses >= self.resolution {
                self.current_pulses %= self.resolution;
                return Direction::CounterClockwise;
            } else if self.current_pulses <= -self.resolution {
                self.current_pulses %= self.resolution;
                return Direction::Clockwise;
            }
        }

        Direction::None
    }
}

impl<
    #[cfg(feature = "async_matrix")] A: InputPin + Wait,
    #[cfg(not(feature = "async_matrix"))] A: InputPin,
    #[cfg(feature = "async_matrix")] B: InputPin + Wait,
    #[cfg(not(feature = "async_matrix"))] B: InputPin,
> RotaryEncoder<A, B, DefaultPhase>
{
    /// Accepts two [`InputPin`](https://docs.rs/embedded-hal/latest/embedded_hal/digital/trait.InputPin.html)s, these will be read on every `update()`.
    pub fn new(pin_a: A, pin_b: B, id: u8) -> Self {
        Self::with_phase(pin_a, pin_b, DefaultPhase, id)
    }
}

/// Create a resolution-based rotary encoder
impl<
    #[cfg(feature = "async_matrix")] A: InputPin + Wait,
    #[cfg(not(feature = "async_matrix"))] A: InputPin,
    #[cfg(feature = "async_matrix")] B: InputPin + Wait,
    #[cfg(not(feature = "async_matrix"))] B: InputPin,
> RotaryEncoder<A, B, ResolutionPhase>
{
    /// Creates a new encoder with the specified resolution
    pub fn with_resolution(pin_a: A, pin_b: B, resolution: u8, reverse: bool, id: u8) -> Self {
        Self::with_phase(pin_a, pin_b, ResolutionPhase::new(resolution, reverse), id)
    }
}

impl<
    #[cfg(feature = "async_matrix")] A: InputPin + Wait,
    #[cfg(not(feature = "async_matrix"))] A: InputPin,
    #[cfg(feature = "async_matrix")] B: InputPin + Wait,
    #[cfg(not(feature = "async_matrix"))] B: InputPin,
    P: Phase,
> RotaryEncoder<A, B, P>
{
    /// Accepts two [`InputPin`](https://docs.rs/embedded-hal/latest/embedded_hal/digital/trait.InputPin.html)s, these will be read on every `update()`, while using `phase` to determine the direction.
    pub fn with_phase(pin_a: A, pin_b: B, phase: P, id: u8) -> Self {
        Self {
            pin_a,
            pin_b,
            state: None,
            phase,
            id,
            last_action: None,
            last_event_time: None,
            debounce_ms: 0,
        }
    }

    /// Set the debounce interval in milliseconds. Events arriving faster than
    /// this interval after the last emitted event are suppressed.
    pub fn with_debounce(mut self, debounce_ms: u16) -> Self {
        self.debounce_ms = debounce_ms;
        self
    }

    /// Sample both pins and return the rotation direction.
    /// The first successful sample and pin read errors return [`Direction::None`].
    pub fn update(&mut self) -> Direction {
        // use mask to get previous state value
        let mut s = self.state.unwrap_or(0) & 0b11;

        let (a_is_low, b_is_low) = (self.pin_a.is_low(), self.pin_b.is_low());

        // move in the new state
        match a_is_low {
            Ok(true) => s |= 0b0100,
            Ok(false) => {}
            Err(_) => return Direction::None,
        }
        match b_is_low {
            Ok(true) => s |= 0b1000,
            Ok(false) => {}
            Err(_) => return Direction::None,
        }

        // move new state in
        let previous = self.state.replace(s >> 2);

        // Use the phase implementation
        previous.map_or(Direction::None, |_| self.phase.direction(s))
    }

    /// Returns a reference to the first pin. Can be used to clear interrupt.
    pub fn pin_a(&mut self) -> &mut A {
        &mut self.pin_a
    }

    /// Returns a reference to the second pin. Can be used to clear interrupt.
    pub fn pin_b(&mut self) -> &mut B {
        &mut self.pin_b
    }

    /// Returns a reference to both pins. Can be used to clear interrupt.
    pub fn pins(&mut self) -> (&mut A, &mut B) {
        (&mut self.pin_a, &mut self.pin_b)
    }

    /// Consumes this `Rotary`, returning the underlying pins `A` and `B`.
    pub fn into_inner(self) -> (A, B) {
        (self.pin_a, self.pin_b)
    }

    /// Check whether enough time has elapsed since the last emitted event.
    /// Returns `true` if the event should pass through (first event, or
    /// debounce interval exceeded). Updates the internal timestamp when
    /// returning `true`.
    fn debounce_check(&mut self) -> bool {
        let now = embassy_time::Instant::now();
        let ok = match self.last_event_time {
            Some(last) => now.duration_since(last).as_millis() >= self.debounce_ms as u64,
            None => true,
        };
        if ok {
            self.last_event_time = Some(now);
        }
        ok
    }
}

impl<
    #[cfg(feature = "async_matrix")] A: InputPin + Wait,
    #[cfg(not(feature = "async_matrix"))] A: InputPin,
    #[cfg(feature = "async_matrix")] B: InputPin + Wait,
    #[cfg(not(feature = "async_matrix"))] B: InputPin,
    P: Phase,
> RotaryEncoder<A, B, P>
{
    /// Read a keyboard event from the rotary encoder.
    /// This method is called by the generated InputDevice implementation.
    async fn read_keyboard_event(&mut self) -> KeyboardEvent {
        while self.state.is_none() {
            self.update();
            if self.state.is_none() {
                embassy_time::Timer::after_millis(1).await;
            }
        }
        // Read until a valid rotary encoder event is detected
        if let Some(last_action) = self.last_action {
            embassy_time::Timer::after_millis(1).await;
            self.last_action = None;
            return KeyboardEvent::rotary_encoder(self.id, last_action, false);
        }

        loop {
            #[cfg(feature = "async_matrix")]
            {
                use embassy_futures::select::select;
                let state = self.state.unwrap_or(0);
                let (pin_a, pin_b) = self.pins();
                match state {
                    0 => select(pin_a.wait_for_low(), pin_b.wait_for_low()).await,
                    1 => select(pin_a.wait_for_high(), pin_b.wait_for_low()).await,
                    2 => select(pin_a.wait_for_low(), pin_b.wait_for_high()).await,
                    _ => select(pin_a.wait_for_high(), pin_b.wait_for_high()).await,
                };
            }

            let direction = self.update();

            if direction != Direction::None && self.debounce_check() {
                self.last_action = Some(direction);
                return KeyboardEvent::rotary_encoder(self.id, direction, true);
            }

            embassy_time::Timer::after_millis(1).await;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn first_sample_sets_baseline_without_a_step() {
        use embedded_hal_mock::eh1::digital::{Mock, State, Transaction};
        let mut encoder = RotaryEncoder::new(
            Mock::new(&[Transaction::get(State::Low)]),
            Mock::new(&[Transaction::get(State::High)]),
            0,
        );
        assert_eq!(encoder.update(), Direction::None);
        encoder.pin_a.done();
        encoder.pin_b.done();
    }

    #[test]
    fn test_resolutin_phase() {
        // Check with E8H7 phase
        let mut default_phase = E8H7Phase {};
        let mut resolution_phase = ResolutionPhase::new(2, true);
        // Clockwise sequence
        for item in [0b100, 0b1101, 0b1011, 0b10] {
            let d = default_phase.direction(item);
            let d2 = resolution_phase.direction(item);
            info!("Item: {:b}, {:?} {:?}", item, d, d2);
            assert_eq!(d, d2);
        }
        // Counterclockwise sequence
        for item in [0b1000, 0b1110, 0b111, 0b1] {
            let d = default_phase.direction(item);
            let d2 = resolution_phase.direction(item);
            info!("Item: {:b}, {:?} {:?}", item, d, d2);
            assert_eq!(d, d2);
        }

        // Check with default phase
        let mut default_phase = DefaultPhase {};
        let mut resolution_phase = ResolutionPhase::new(1, false);
        for item in 0u8..16 {
            let d = default_phase.direction(item);
            let d2 = resolution_phase.direction(item);
            info!("Item: {:b}, {:?} {:?}", item, d, d2);
            assert_eq!(d, d2);
        }
    }

    #[test]
    fn resolution_above_i8_max_counts_full_steps() {
        let mut phase = ResolutionPhase::new(200, false);
        for pulse in 1..=200 {
            let d = phase.direction([0b1000, 0b1110, 0b0111, 0b0001][(pulse - 1) % 4]);
            let expected = if pulse == 200 {
                Direction::Clockwise
            } else {
                Direction::None
            };
            assert_eq!(d, expected, "pulse {pulse}");
        }
    }

    #[test]
    #[should_panic(expected = "resolution must be nonzero")]
    fn zero_resolution_is_rejected() {
        ResolutionPhase::new(0, false);
    }
}
