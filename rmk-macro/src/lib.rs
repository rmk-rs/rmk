mod codegen;
mod event;
mod event_macros;
mod processor;
mod utils;

use codegen::split::peripheral::parse_split_peripheral_mod;
use darling::FromMeta;
use darling::ast::NestedMeta;
use proc_macro::TokenStream;
use syn::parse_macro_input;

use crate::codegen::parse_keyboard_mod;

/// Expand a directory of simulator scenario TOMLs into `#[test]` fns targeting
/// rmk's `tests/integration/simulator` harness. Test-only; see
/// `rmk/tests/scenarios/README.md`.
#[cfg(feature = "_simulator")]
#[doc(hidden)]
#[proc_macro]
pub fn run_tests(input: TokenStream) -> TokenStream {
    let dir = parse_macro_input!(input as syn::LitStr);
    codegen::simulator::expand_run_tests(dir).into()
}

#[proc_macro_attribute]
pub fn rmk_keyboard(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_mod = parse_macro_input!(item as syn::ItemMod);
    parse_keyboard_mod(item_mod).into()
}

#[proc_macro_attribute]
pub fn rmk_central(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_mod = parse_macro_input!(item as syn::ItemMod);
    parse_keyboard_mod(item_mod).into()
}

/// Attribute for `rmk_peripheral` macro
#[derive(Debug, FromMeta)]
struct PeripheralAttr {
    #[darling(default)]
    id: usize,
}

#[proc_macro_attribute]
pub fn rmk_peripheral(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_mod = parse_macro_input!(item as syn::ItemMod);
    let attr_args = match NestedMeta::parse_meta_list(attr.clone().into()) {
        Ok(v) => v,
        Err(e) => {
            return TokenStream::from(darling::Error::from(e).write_errors());
        }
    };

    let peripheral_id = match PeripheralAttr::from_list(&attr_args) {
        Ok(v) => v.id,
        Err(e) => {
            return TokenStream::from(e.write_errors());
        }
    };

    parse_split_peripheral_mod(peripheral_id, attr, item_mod).into()
}

/// Emit the flash-resident physical layout and `[lighting]` statics from
/// `KEYBOARD_TOML_PATH` for firmware with a hand-written main function.
///
/// `#[rmk_keyboard]` emits the same statics as part of full main generation;
/// boards whose hardware does not fit the generated main (custom matrix
/// drivers, nonstandard USB bring-up) invoke this at module scope instead and
/// wire the lighting engine themselves.
#[proc_macro]
pub fn rmk_lighting_config(_item: TokenStream) -> TokenStream {
    codegen::lighting::expand_standalone_lighting_config().into()
}

/// Marker attribute for coordinating Runnable generation between macros.
/// Do not use directly.
#[doc(hidden)]
#[proc_macro_attribute]
pub fn runnable_generated(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item // Pass through unchanged
}

/// Derive macro for multi-event enums that generates automatic event dispatch.
///
/// This macro generates:
/// - `{EnumName}Publisher` struct implementing `AsyncEventPublisher` and `EventPublisher`
/// - `PublishableEvent` and `AsyncPublishableEvent` trait implementations
/// - `From<VariantType>` impls for each variant
///
/// Each variant is forwarded to its underlying event channel when published.
///
/// **Note**: You cannot subscribe to wrapper enums directly. Subscribe to the individual
/// concrete event types (e.g., `BatteryEvent`, `PointingEvent`) instead.
///
/// # Example
///
/// ```rust,ignore
/// #[derive(Event)]
/// pub enum MultiSensorEvent {
///     Battery(BatteryEvent),
///     Pointing(PointingEvent),
/// }
///
/// // Publishing: events are routed to their concrete type channels
/// publish_event_async(MultiSensorEvent::Battery(event)).await;
/// ```
#[proc_macro_derive(Event)]
pub fn event_derive(item: TokenStream) -> TokenStream {
    event_macros::input_event_derive::event_derive_impl(item)
}

/// Macro for defining input devices that publish events.
///
/// This macro generates `InputDevice` and `Runnable` implementations for single-event devices.
/// For multi-event devices, use `#[derive(Event)]` on a user-defined enum instead.
///
/// # Parameters
///
/// - `publish`: The event type to publish (single event type only)
///
/// # Example
///
/// ```rust,ignore
/// #[input_device(publish = BatteryEvent)]
/// pub struct BatteryReader { ... }
///
/// impl BatteryReader {
///     // User implements this inherent method
///     async fn read_battery_event(&mut self) -> BatteryEvent {
///         // Wait and return single event
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn input_device(attr: TokenStream, item: TokenStream) -> TokenStream {
    event_macros::input_device::input_device_impl(attr, item)
}

/// Unified macro for defining events with static channels.
///
/// Generates `PublishableEvent`, `SubscribableEvent`, and `AsyncPublishableEvent`
/// trait implementations.
///
/// # Parameters
///
/// - `channel_size`: Buffer size of the channel (default: 8 for MPSC, 1 for PubSub)
/// - `subs`: Max subscribers (triggers PubSub mode, default: 4)
/// - `pubs`: Max publishers (triggers PubSub mode, default: 1)
///
/// If `subs` or `pubs` is specified, PubSub channel is used; otherwise MPSC channel.
///
/// # Examples
///
/// ```rust,ignore
/// // MPSC channel (single consumer)
/// #[event(channel_size = 16)]
/// #[derive(Clone, Copy, Debug)]
/// pub struct KeyboardEvent { /* ... */ }
///
/// // PubSub channel (multiple subscribers)
/// #[event(channel_size = 4, subs = 8, pubs = 2)]
/// #[derive(Clone, Copy, Debug)]
/// pub struct LedIndicatorEvent { /* ... */ }
/// ```
#[proc_macro_attribute]
pub fn event(attr: TokenStream, item: TokenStream) -> TokenStream {
    event::event_impl(attr, item)
}

/// Unified macro for defining event processors.
///
/// Generates `Processor` and optional `PollingProcessor` implementations.
///
/// # Parameters
///
/// - `subscribe`: Array of event types to subscribe to (required)
/// - `poll_interval`: Optional polling interval in milliseconds
///
/// # Examples
///
/// ```rust,ignore
/// // Event-driven processor
/// #[processor(subscribe = [LedIndicatorEvent])]
/// struct LedController { /* ... */ }
///
/// impl LedController {
///     async fn on_led_indicator_event(&mut self, event: LedIndicatorEvent) {
///         // Handle event
///     }
/// }
///
/// // Polling processor
/// #[processor(subscribe = [BatteryStatusEvent], poll_interval = 1000)]
/// struct BatteryMonitor { /* ... */ }
///
/// impl BatteryMonitor {
///     async fn on_battery_status_event(&mut self, event: BatteryStatusEvent) {
///         // Handle event
///     }
///
///     async fn poll(&mut self) {
///         // Called every 1000ms
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn processor(attr: TokenStream, item: TokenStream) -> TokenStream {
    processor::processor_impl(attr, item)
}
