//! Exposed channels which can be used to share data across devices & processors

use core::future::poll_fn;

use embassy_sync::channel::{Channel, TrySendError};
#[cfg(feature = "_ble")]
use embassy_sync::signal::Signal;
pub use embassy_sync::{blocking_mutex, channel, pubsub, zerocopy_channel};
use rmk_types::connection::ConnectionType;
use usbd_hid::descriptor::{MediaKeyboardReport, SystemControlReport};
#[cfg(feature = "_ble")]
use {crate::ble::profile::BleProfileAction, rmk_types::led_indicator::LedIndicator};

#[cfg(all(feature = "vial", feature = "_ble"))]
use crate::VIAL_CHANNEL_SIZE;
#[cfg(feature = "steno")]
use crate::hid::StenoReport;
use crate::hid::{KeyboardReport, MouseReport, Report};
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

/// Drains queued reports for `transport` and leaves an all-up report of every
/// type for its writer. Called on active-transport flips so the previous host
/// releases any pressed keys and buttons without replaying stale queued reports later.
pub(crate) fn clear_and_release_report_channel(transport: ConnectionType) {
    if let Some(ch) = report_channel(transport) {
        ch.clear();
        for report in [
            Report::KeyboardReport(KeyboardReport::default()),
            Report::MouseReport(MouseReport::default()),
            Report::MediaKeyboardReport(MediaKeyboardReport { usage_id: 0 }),
            Report::SystemControlReport(SystemControlReport { usage_id: 0 }),
            #[cfg(feature = "steno")]
            Report::StenoReport(StenoReport::default()),
        ] {
            let _ = ch.try_send(report);
        }
    }
}

#[cfg(feature = "_ble")]
pub(crate) static BLE_PROFILE_CHANNEL: Channel<RawMutex, BleProfileAction, 1> = Channel::new();

/// Vial RX from BLE GATT `output_data` writes — one 32-byte chunk per write.
/// Only the active profile's connection may send; one BLE session drains it.
#[cfg(all(feature = "vial", feature = "_ble"))]
pub(crate) static VIAL_BLE_RX_CHANNEL: Channel<RawMutex, [u8; 32], VIAL_CHANNEL_SIZE> = Channel::new();
