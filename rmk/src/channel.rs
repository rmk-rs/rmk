//! Exposed channels which can be used to share data across devices & processors

use core::future::{Future, poll_fn};
use core::pin::pin;
use core::task::Poll;

use embassy_sync::channel::Channel;
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
pub(crate) async fn send_hid_report(report: Report) {
    let Some((transport, ch)) = active_report_channel() else {
        return;
    };
    #[cfg(feature = "_ble")]
    let profile = crate::state::current_profile();

    let mut send = pin!(ch.send(report));
    poll_fn(|cx| {
        if crate::state::active_transport() != Some(transport) {
            return Poll::Ready(());
        }
        #[cfg(feature = "_ble")]
        if transport == ConnectionType::Ble && crate::state::current_profile() != profile {
            return Poll::Ready(());
        }
        send.as_mut().poll(cx)
    })
    .await;
}

/// Never waits for queue space, so a steno send cannot stall the keyboard
/// task. A dropped report can lose or merge a stroke on the host; the next
/// report still restates every held key because steno reports are absolute.
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
/// Pushed by `gatt_events_task`, drained by [`crate::ble::host::HostGattHandler::run`].
#[cfg(all(feature = "vial", feature = "_ble"))]
pub(crate) static VIAL_BLE_RX_CHANNEL: Channel<RawMutex, [u8; 32], VIAL_CHANNEL_SIZE> = Channel::new();
