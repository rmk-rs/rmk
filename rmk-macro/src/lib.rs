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

/// The macro ops that type an ASCII string: `text!("ab")` is
/// `[MacroOp::Char(b'a'), MacroOp::Char(b'b')]`.
#[proc_macro]
pub fn text(input: TokenStream) -> TokenStream {
    let text = parse_macro_input!(input as syn::LitStr)
        .value()
        .into_bytes();
    quote::quote! { [#(::rmk::types::keyboard_macros::MacroOp::Char(#text)),*] }.into()
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

/// Keeps a `Runnable` implementation provided by the user or another macro.
///
/// Place this marker below `#[processor]` or `#[input_device]`. The preceding
/// macro still generates its device or processor traits, but skips `Runnable`.
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

/// Implements event handling and timer scheduling for a struct.
///
/// The macro generates `Processor` and `Runnable`, plus `PollingProcessor` when
/// polling is selected. Implement `DeadlineProcessor` yourself for deadlines.
/// Construct an instance and run it with `run_all!`,
/// or return it from a `#[register_processor]` function in a keyboard module.
///
/// # Options
///
/// - `subscribe = [EventType, ...]`: provide an async `on_<event_name>_event` handler
///   for each event. Omit this option or use `[]` for a timer-only processor.
/// - `poll_interval = N`: provide an async `poll()` method. `N` must be a positive
///   interval in milliseconds.
/// - `deadline`: implement `DeadlineProcessor::deadline()` and `on_deadline()`.
///   The macro generates the deadline run loop, not this trait implementation.
///   Clear or advance the deadline after it fires.
///
/// Options can be combined. Callbacks run serially, with ready deadlines before
/// polling ticks and events. Events do not restart the polling interval.
///
/// At least one event or timer source is required. To supply a custom `Runnable`,
/// place `#[runnable_generated]` below this attribute; it suppresses the generated
/// run loop and permits a bare `#[processor]`.
///
/// # Example
///
/// ```rust,ignore
/// use rmk::event::LayerChangeEvent;
/// use rmk::macros::processor;
///
/// #[processor(subscribe = [LayerChangeEvent])]
/// #[derive(Default)]
/// struct LayerTracker {
///     layer: u8,
/// }
///
/// impl LayerTracker {
///     async fn on_layer_change_event(&mut self, event: LayerChangeEvent) {
///         self.layer = event.0;
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn processor(attr: TokenStream, item: TokenStream) -> TokenStream {
    processor::processor_impl(attr, item)
}
