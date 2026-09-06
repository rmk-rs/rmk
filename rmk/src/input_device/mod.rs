//! Input device module for RMK
//!
//! This module defines the `InputDevice` trait, `Runnable` trait and several macros for running input devices and processors.
//! The `InputDevice` trait provides the interface for individual input devices, and the macros facilitate their concurrent execution.

use crate::core_traits::Runnable;

pub mod adc;
#[cfg(feature = "_ble")]
pub mod battery;
pub mod iqs5xx;
pub mod joystick;
pub mod pmw33xx;
pub mod pmw3610;
pub mod pointing;
pub mod rotary_encoder;

/// The trait for input devices.
///
/// This trait defines the interface for input devices in RMK.
/// Use the `#[input_device]` macro to automatically implement this trait.
///
/// # Example
/// ```rust
/// // For single-event devices, use the macro:
/// #[input_device(publish = BatteryEvent)]
/// struct MyInputDevice;
///
/// impl MyInputDevice {
///     // You MUST implement this read method for the published event.
///     // The method name follows the pattern: read_{event_name}_event
///     async fn read_battery_event(&mut self) -> BatteryEvent {
///         // Implementation for reading an event
///     }
/// }
///
/// // For multi-event devices, a derived enum should be used.
/// // **Note**: Wrapper enums only implement publish traits, not subscribe traits.
/// // This is because wrapper enums route events to their concrete type channels,
/// // and you should subscribe to the individual event types instead.
/// #[derive(Event)]
/// enum MultiDeviceEvent {
///     Battery(BatteryEvent),
///     Pointing(PointingEvent),
/// }
///
/// #[input_device(publish = MultiDeviceEvent)]
/// struct MyInputDevice;
///
/// impl MyInputDevice {
///     // Returns the `MultiDeviceEvent`
///     async fn read_multi_device_event(&mut self) -> MultiDeviceEvent {
///         // Implementation for reading multiple types of events
///     }
/// }
/// ```
pub trait InputDevice: Runnable {
    /// The event type produced by this input device
    type Event;

    /// Read the raw input event
    async fn read_event(&mut self) -> Self::Event;
}

/// Run multiple tasks concurrently by calling each task's `run` method.
///
/// The `Runnable` trait is brought into scope for input devices and
/// processors. Types with an inherent `run` method, such as `BleTransport`,
/// can be included without implementing `Runnable`.
///
/// # Example
/// ```rust
/// // Define your runnables
/// let mut device = MyInputDevice::new();
/// let mut processor = MyProcessor::new();
///
/// // Run all runnables concurrently
/// run_all!(device, processor);
/// ```
#[macro_export]
macro_rules! run_all {
    ($( $dev:ident ),* $(,)*) => {{
        use $crate::core_traits::Runnable as _;
        $crate::join_all!(
            $(
                $dev.run()
            ),*
        )
    }};
}
