//! Exposed channels which can be used to share data across devices & processors

use core::future::poll_fn;

use embassy_sync::channel::{Channel, TrySendError};
#[cfg(feature = "_ble")]
use embassy_sync::signal::Signal;
pub use embassy_sync::{blocking_mutex, channel, pubsub, zerocopy_channel};
use rmk_types::connection::ConnectionType;
#[cfg(feature = "_ble")]
use {crate::ble::profile::BleProfileAction, rmk_types::led_indicator::LedIndicator};

#[cfg(all(feature = "vial", feature = "_ble"))]
use crate::VIAL_CHANNEL_SIZE;
use crate::event::KeyboardEvent;
use crate::hid::{KeyboardReport, Report};
#[cfg(feature = "storage")]
use crate::{FLASH_CHANNEL_SIZE, storage::FlashOperationMessage};
use crate::{REPORT_CHANNEL_SIZE, RawMutex};

type ReportChannel = Channel<RawMutex, Report, REPORT_CHANNEL_SIZE>;

/// Signal for LED indicator, used in BLE keyboards only since BLE receiving is not async
#[cfg(feature = "_ble")]
pub(crate) static LED_SIGNAL: Signal<RawMutex, LedIndicator> = Signal::new();

/// Drained by the USB HID writer task. Routed through `send_hid_report`
/// from the keyboard task and ad-hoc producers (e.g. steno chord output).
#[cfg(not(feature = "_no_usb"))]
pub static USB_REPORT_CHANNEL: ReportChannel = Channel::new();

/// Drained by the BLE HID writer task. Routed through `send_hid_report`.
#[cfg(feature = "_ble")]
pub static BLE_REPORT_CHANNEL: ReportChannel = Channel::new();

fn report_channel(transport: ConnectionType) -> Option<&'static ReportChannel> {
    match transport {
        #[cfg(not(feature = "_no_usb"))]
        ConnectionType::Usb => Some(&USB_REPORT_CHANNEL),
        #[cfg(feature = "_ble")]
        ConnectionType::Ble => Some(&BLE_REPORT_CHANNEL),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

fn active_report_channel() -> Option<(ConnectionType, &'static ReportChannel)> {
    let transport = crate::state::active_transport()?;
    report_channel(transport).map(|ch| (transport, ch))
}

/// Reports generated while no transport is selected are dropped on the floor.
pub(crate) async fn send_hid_report(mut report: Report) {
    let Some((transport, ch)) = active_report_channel() else {
        return;
    };

    loop {
        match ch.try_send(report) {
            Ok(()) => return,
            Err(TrySendError::Full(r)) => report = r,
        }

        poll_fn(|cx| ch.poll_ready_to_send(cx)).await;
        if crate::state::active_transport() != Some(transport) {
            return;
        }
    }
}

/// Drops the report when the active transport's queue is full or no
/// transport is selected. Use for producers where back-pressure would block
/// the matrix scan (e.g. steno chord output).
pub(crate) fn try_send_hid_report(report: Report) {
    if let Some((_, ch)) = active_report_channel() {
        let _ = ch.try_send(report);
    }
}

/// Drains queued reports for `transport` and leaves an all-up keyboard report
/// for its writer. Called on active-transport flips so the previous host
/// releases any pressed keys without replaying stale queued reports later.
pub(crate) fn clear_and_release_report_channel(transport: ConnectionType) {
    if let Some(ch) = report_channel(transport) {
        ch.clear();
        let _ = ch.try_send(Report::KeyboardReport(KeyboardReport::default()));
    }
}

// Sync messages from server to flash
#[cfg(feature = "storage")]
pub(crate) static FLASH_CHANNEL: Channel<RawMutex, FlashOperationMessage, FLASH_CHANNEL_SIZE> = Channel::new();

/// Test-only: continuously drain [`FLASH_CHANNEL`] so host-service integration
/// tests that trigger persistence never block on a full, never-serviced flash
/// queue — the real firmware's storage task is what normally drains it.
#[cfg(feature = "std")]
#[doc(hidden)]
pub async fn drain_flash_channel_for_test() {
    #[cfg(feature = "storage")]
    loop {
        FLASH_CHANNEL.receive().await;
    }
    #[cfg(not(feature = "storage"))]
    core::future::pending::<()>().await
}
#[cfg(feature = "_ble")]
pub(crate) static BLE_PROFILE_CHANNEL: Channel<RawMutex, BleProfileAction, 1> = Channel::new();

/// Test-only stand-in for the BLE profile task: hands each action's `Debug`
/// form to `sink`, so a test can assert what a profile-key gesture sent.
#[cfg(all(feature = "std", feature = "_ble"))]
#[doc(hidden)]
pub async fn drain_ble_profile_channel_for_test(mut sink: impl FnMut(std::string::String)) {
    loop {
        sink(std::format!("{:?}", BLE_PROFILE_CHANNEL.receive().await));
    }
}

/// Vial RX from BLE GATT `output_data` writes — one 32-byte chunk per write.
/// Pushed by `gatt_events_task`, drained by [`crate::ble::host::HostGattHandler::run`].
#[cfg(all(feature = "vial", feature = "_ble"))]
pub(crate) static VIAL_BLE_RX_CHANNEL: Channel<RawMutex, [u8; 32], VIAL_CHANNEL_SIZE> = Channel::new();

/// Rynk RX from the BLE `output_data` writes. The 512 B ring is ~2× one MTU's maximal payload.
#[cfg(all(feature = "rynk", feature = "_ble"))]
pub(crate) static RYNK_BLE_RX_PIPE: embassy_sync::pipe::Pipe<RawMutex, 512> = embassy_sync::pipe::Pipe::new();

/// Macros are triggered on key press but run by the keyboard loop to avoid recursion
/// (`execute_macro` dispatches a macro's ops back through the action path).
/// Producer: the `TriggerMacro` action. Consumer: the keyboard loop.
pub(crate) static MACRO_TRIGGER_CHANNEL: Channel<RawMutex, (u8, KeyboardEvent), 4> = Channel::new();
