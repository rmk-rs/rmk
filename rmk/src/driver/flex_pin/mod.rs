use embedded_hal::digital::{ErrorType, InputPin, OutputPin};

#[cfg(feature = "_nrf")]
pub mod nrf;
#[cfg(feature = "rp2040")]
pub mod rp;

/// Pin that can be switched between input and output.
pub trait FlexPin: ErrorType + InputPin + OutputPin {
    fn set_as_input(&mut self) -> ();

    fn set_as_output(&mut self) -> ();
}
