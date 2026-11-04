use core::cell::Cell;

use embassy_sync::watch::Watch;
use rmk_types::ble::{BleState, BleStatus};
use rmk_types::connection::{ConnectionStatus, ConnectionType, UsbState};

use crate::RawMutex;
#[cfg(feature = "_ble")]
use crate::event::publish_event;

/// Authoritative transport state; mutations below also update output routing and LEDs.
pub(crate) static CONNECTION_STATUS: Watch<
    RawMutex,
    ConnectionStatus,
    { crate::CONNECTION_STATUS_CHANGE_EVENT_SUB_SIZE },
> = Watch::new_with(ConnectionStatus::new());

pub(crate) fn active_transport() -> Option<ConnectionType> {
    current_connection_status().decide_active()
}

/// Snapshot of the current connection status.
///
/// For change notifications, subscribe before reading this snapshot.
/// A newly created subscriber does not replay earlier notifications.
pub fn current_connection_status() -> ConnectionStatus {
    CONNECTION_STATUS.try_get().unwrap_or_default()
}

pub(crate) fn current_usb_state() -> UsbState {
    current_connection_status().usb
}

/// Current central sleep state for host polling. Sourced from the BLE sleep
/// manager's `SLEEPING_STATE`; always `false` in builds without BLE.
pub(crate) fn current_sleep_state() -> bool {
    #[cfg(feature = "_ble")]
    {
        crate::ble::sleep::SLEEPING_STATE.load(core::sync::atomic::Ordering::Acquire)
    }
    #[cfg(not(feature = "_ble"))]
    {
        false
    }
}

/// The BLE half of [`current_connection_status`], with the same subscribe-first rule.
#[cfg(feature = "_ble")]
pub fn current_ble_status() -> BleStatus {
    current_connection_status().ble
}

/// Read-modify-write the connection status atomically.
pub(crate) fn update_status(f: impl Fn(&mut ConnectionStatus)) {
    let change = Cell::new(None);
    CONNECTION_STATUS.sender().send_if_modified(|c| {
        let c = c.get_or_insert(ConnectionStatus::new());
        let prev = *c;
        f(c);
        let new = *c;
        if prev == new {
            return false;
        }
        change.set(Some((prev, new)));
        true
    });
    let Some((prev, new)) = change.get() else {
        return;
    };

    let prev_active = prev.decide_active();
    let new_active = new.decide_active();

    if prev_active != new_active
        && let Some(prev_active) = prev_active
    {
        // Drain after the commit so any producer racing past the mutex reads
        // the new state and routes to the new channel rather than the one
        // about to be cleared.
        crate::channel::clear_and_release_report_channel(prev_active);
    }

    #[cfg(feature = "_ble")]
    if prev.ble.profile != new.ble.profile {
        crate::channel::LED_SIGNAL.reset();
        if prev_active != Some(ConnectionType::Usb) || new_active != Some(ConnectionType::Usb) {
            crate::keyboard::LOCK_LED_STATES.store(0, core::sync::atomic::Ordering::Relaxed);
            publish_event(crate::event::LedIndicatorEvent::new(Default::default()));
        }
    }
}

pub fn set_usb_state(s: UsbState) {
    update_status(|c| c.usb = s);
}

pub(crate) fn set_ble_state(s: BleState) {
    update_status(|c| c.ble.state = s);
}

/// Change the selected profile and its readiness atomically, so an already
/// encrypted target does not briefly hand input to another transport.
pub(crate) fn set_ble_status(status: BleStatus) {
    update_status(|c| c.ble = status);
}

/// `bonded` always describes `c.ble.profile`, so call this only for a change to the
/// currently active profile. Use [`set_ble_status`] to move both at once.
pub(crate) fn set_ble_bonded(bonded: bool) {
    update_status(|c| c.ble.bonded = bonded);
}

/// Persistence is the caller's responsibility: `storage::store` the new
/// `ConnectionType`.
pub(crate) fn set_preferred_connection(t: ConnectionType) {
    update_status(|c| c.preferred = t);
}

/// Load the preferred connection type at startup.
///
/// With the `storage` feature, reads the persisted `ConnectionType` from flash;
/// otherwise falls back to a build-time default — `Ble` when USB is disabled, `Usb` otherwise.
#[cfg(feature = "_ble")]
pub(crate) async fn load_preferred_connection() -> ConnectionType {
    #[cfg(feature = "storage")]
    if let Ok(Some(crate::storage::StorageValue::ConnectionType(c))) =
        crate::storage::read(crate::storage::StorageKey::ConnectionType).await
    {
        return c;
    }
    if cfg!(feature = "_no_usb") {
        ConnectionType::Ble
    } else {
        ConnectionType::Usb
    }
}

#[cfg(all(feature = "_ble", not(feature = "_no_usb")))]
pub(crate) async fn toggle_preferred() {
    let new = Cell::new(ConnectionType::Usb);
    update_status(|c| {
        c.preferred = match c.preferred {
            ConnectionType::Usb => ConnectionType::Ble,
            ConnectionType::Ble => ConnectionType::Usb,
        };
        new.set(c.preferred);
    });
    let new = new.get();
    info!("Switching preferred transport to: {:?}", new);
    #[cfg(feature = "storage")]
    crate::storage::store_unchecked(crate::storage::StorageItem::ConnectionType(new)).await;
}

#[cfg(feature = "_ble")]
pub(crate) fn current_profile() -> u8 {
    current_connection_status().ble.profile
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use embassy_futures::select::{Either, select};
    use embassy_sync::channel::Channel;
    use embassy_time::{Duration, Timer};
    #[cfg(feature = "_ble")]
    use rmk_types::ble::{BleState, BleStatus};

    #[cfg(all(feature = "_ble", not(feature = "_no_usb")))]
    use super::set_ble_state;
    use super::{ConnectionStatus, ConnectionType, UsbState, set_preferred_connection, set_usb_state};
    #[cfg(feature = "_ble")]
    use super::{set_ble_bonded, set_ble_status};
    use crate::event::{ConnectionStatusChangeEvent, EventSubscriber, SubscribableEvent};
    #[cfg(not(feature = "_no_usb"))]
    use crate::hid::MouseReport;
    use crate::hid::{KeyboardReport, Report};
    use crate::test_support::test_block_on as block_on;

    fn state_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn reset_state() {
        crate::test_support::set_connection_status(ConnectionStatus::default());
        #[cfg(not(feature = "_no_usb"))]
        crate::channel::USB_REPORT_CHANNEL.clear();
        #[cfg(feature = "_ble")]
        crate::channel::BLE_REPORT_CHANNEL.clear();
    }

    fn pressed_keyboard_report() -> Report {
        Report::KeyboardReport(KeyboardReport {
            modifier: 0x02,
            reserved: 0,
            leds: 0,
            keycodes: [4, 0, 0, 0, 0, 0],
        })
    }

    fn assert_all_released(ch: &Channel<crate::RawMutex, Report, { crate::REPORT_CHANNEL_SIZE }>) {
        assert!(matches!(ch.try_receive(), Ok(Report::KeyboardReport(r))
            if r.modifier == 0 && r.reserved == 0 && r.leds == 0 && r.keycodes == [0; 6]));
        assert!(matches!(ch.try_receive(), Ok(Report::MouseReport(r))
            if r.buttons == 0 && r.x == 0 && r.y == 0 && r.wheel == 0 && r.pan == 0));
        assert!(matches!(ch.try_receive(), Ok(Report::MediaKeyboardReport(r)) if r.usage_id == 0));
        assert!(matches!(ch.try_receive(), Ok(Report::SystemControlReport(r)) if r.usage_id == 0));
        #[cfg(feature = "steno")]
        assert!(matches!(ch.try_receive(), Ok(Report::StenoReport(r)) if r.keys == [0; 8]));
        assert!(ch.try_receive().is_err());
    }

    #[test]
    fn preferred_transport_change_publishes_status_event() {
        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        set_usb_state(UsbState::Configured);
        let mut sub = ConnectionStatusChangeEvent::subscriber();

        set_preferred_connection(ConnectionType::Ble);

        let event = block_on(sub.next_event());
        assert_eq!(event.0.preferred, ConnectionType::Ble);
    }

    #[test]
    fn usb_state_change_publishes_status_event() {
        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        let mut sub = ConnectionStatusChangeEvent::subscriber();

        set_usb_state(UsbState::Configured);

        let event = block_on(sub.next_event());
        assert_eq!(event.0.usb, UsbState::Configured);
    }

    #[test]
    fn status_update_initializes_an_empty_watch() {
        let _guard = state_test_lock().lock().expect("state test lock");
        reset_state();
        super::CONNECTION_STATUS.sender().clear();
        let mut sub = ConnectionStatusChangeEvent::subscriber();

        set_usb_state(UsbState::Configured);

        let expected = ConnectionStatus {
            usb: UsbState::Configured,
            ..ConnectionStatus::new()
        };
        assert_eq!(super::current_connection_status(), expected);
        assert_eq!(block_on(sub.next_event()).0, expected);
    }

    #[test]
    fn unchanged_status_does_not_publish_event() {
        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        set_usb_state(UsbState::Configured);
        let mut sub = ConnectionStatusChangeEvent::subscriber();

        // Re-setting the same value should not publish.
        set_usb_state(UsbState::Configured);

        match block_on(select(Timer::after(Duration::from_millis(1)), sub.next_event())) {
            Either::First(_) => {}
            Either::Second(event) => panic!("unexpected status change event: {:?}", event),
        }
    }

    #[cfg(feature = "_ble")]
    #[test]
    fn profile_updates_are_coalesced_for_each_subscriber() {
        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        let mut first = ConnectionStatusChangeEvent::subscriber();
        let mut second = ConnectionStatusChangeEvent::subscriber();

        for profile in [1, 2] {
            set_ble_status(BleStatus {
                profile,
                state: BleState::Connected,
                bonded: true,
            });
        }

        for subscriber in [&mut first, &mut second] {
            assert_eq!(block_on(subscriber.next_event()).0, super::current_connection_status());
        }
    }

    #[cfg(feature = "_ble")]
    #[test]
    fn ble_status_update_publishes_profile_state_and_bond_together() {
        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        let mut sub = ConnectionStatusChangeEvent::subscriber();

        set_ble_status(BleStatus {
            profile: 1,
            state: BleState::Inactive,
            bonded: true,
        });

        assert_eq!(
            block_on(sub.next_event()).0.ble,
            BleStatus {
                profile: 1,
                state: BleState::Inactive,
                bonded: true,
            }
        );

        set_ble_status(BleStatus {
            profile: 2,
            state: BleState::Inactive,
            bonded: false,
        });

        assert_eq!(
            block_on(sub.next_event()).0.ble,
            BleStatus {
                profile: 2,
                state: BleState::Inactive,
                bonded: false,
            }
        );
    }

    #[cfg(feature = "_ble")]
    #[test]
    fn ble_bond_update_publishes_status_event_and_preserves_profile_and_state() {
        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        set_ble_status(BleStatus {
            profile: 2,
            state: BleState::Connected,
            bonded: true,
        });
        let mut sub = ConnectionStatusChangeEvent::subscriber();

        set_ble_bonded(false);

        assert_eq!(
            block_on(sub.next_event()).0.ble,
            BleStatus {
                profile: 2,
                state: BleState::Connected,
                bonded: false,
            }
        );
    }

    #[cfg(not(feature = "_no_usb"))]
    #[test]
    fn flipping_away_from_active_clears_stale_reports_and_queues_all_up() {
        use crate::channel::USB_REPORT_CHANNEL;

        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        set_usb_state(UsbState::Configured);
        assert_eq!(super::active_transport(), Some(ConnectionType::Usb));

        for report in [
            pressed_keyboard_report(),
            Report::MouseReport(MouseReport {
                buttons: 1,
                ..Default::default()
            }),
            Report::MediaKeyboardReport(usbd_hid::descriptor::MediaKeyboardReport { usage_id: 0xE9 }),
        ] {
            USB_REPORT_CHANNEL.try_send(report).unwrap();
        }

        set_usb_state(UsbState::Disabled);
        assert!(super::active_transport().is_none());
        assert_all_released(&USB_REPORT_CHANNEL);
    }

    #[cfg(not(feature = "_no_usb"))]
    #[test]
    fn blocked_send_drops_report_after_transport_change() {
        use embassy_futures::join::join;

        use crate::channel::{USB_REPORT_CHANNEL, send_hid_report};

        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        set_usb_state(UsbState::Configured);

        for _ in 0..crate::REPORT_CHANNEL_SIZE {
            USB_REPORT_CHANNEL
                .try_send(pressed_keyboard_report())
                .expect("channel should have capacity while filling");
        }

        block_on(join(
            send_hid_report(Report::KeyboardReport(KeyboardReport::default())),
            async {
                Timer::after(Duration::from_millis(1)).await;
                set_usb_state(UsbState::Disabled);
            },
        ));

        assert_all_released(&USB_REPORT_CHANNEL);
    }

    #[cfg(feature = "_ble")]
    #[test]
    fn blocked_send_drops_report_after_profile_change() {
        use embassy_futures::join::join;

        use crate::channel::{BLE_REPORT_CHANNEL, send_hid_report};

        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        set_ble_status(BleStatus {
            profile: 0,
            state: BleState::Connected,
            bonded: true,
        });
        for _ in 0..crate::REPORT_CHANNEL_SIZE {
            BLE_REPORT_CHANNEL.try_send(pressed_keyboard_report()).unwrap();
        }

        block_on(join(send_hid_report(pressed_keyboard_report()), async {
            Timer::after_millis(1).await;
            set_ble_status(BleStatus {
                profile: 1,
                state: BleState::Connected,
                bonded: true,
            });
            BLE_REPORT_CHANNEL.clear();
        }));

        assert_eq!(super::active_transport(), Some(ConnectionType::Ble));
        assert!(BLE_REPORT_CHANNEL.is_empty());
    }

    #[cfg(all(not(feature = "_no_usb"), feature = "_ble"))]
    #[test]
    fn usb_preference_flip_releases_previous_ble_transport() {
        use crate::channel::BLE_REPORT_CHANNEL;

        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        set_preferred_connection(ConnectionType::Usb);
        set_ble_state(BleState::Connected);
        assert_eq!(super::active_transport(), Some(ConnectionType::Ble));

        BLE_REPORT_CHANNEL
            .try_send(pressed_keyboard_report())
            .expect("BLE report channel should have capacity for sentinel");

        set_usb_state(UsbState::Configured);
        assert_eq!(super::active_transport(), Some(ConnectionType::Usb));
        assert_all_released(&BLE_REPORT_CHANNEL);
    }

    #[cfg(all(feature = "_ble", not(feature = "_no_usb")))]
    #[test]
    fn toggling_preference_routes_and_persists_the_selected_transport() {
        use crate::config::StorageConfig;
        use crate::core_traits::Runnable;
        use crate::storage::{Storage, StorageKey, StorageValue, async_flash_wrapper, read};
        use crate::test_support::{InMemoryFlash, clear_flash_channel};

        let _guard = state_test_lock().lock().expect("state test lock");
        reset_state();
        clear_flash_channel();
        set_usb_state(UsbState::Configured);
        set_ble_state(BleState::Connected);

        block_on(async {
            let flash = async_flash_wrapper(InMemoryFlash::<16_384, 4_096, 1>::new());
            let mut storage = Box::pin(Storage::new(flash, &StorageConfig::default())).await;
            select(storage.run(), async {
                for expected in [ConnectionType::Ble, ConnectionType::Usb] {
                    super::toggle_preferred().await;
                    assert_eq!(super::active_transport(), Some(expected));
                    assert!(matches!(
                        read(StorageKey::ConnectionType).await,
                        Ok(Some(StorageValue::ConnectionType(stored))) if stored == expected
                    ));
                }
            })
            .await;
        });
    }

    #[cfg(not(feature = "_no_usb"))]
    #[test]
    fn blocked_send_enqueues_when_transport_stays_active() {
        use embassy_futures::join::join;

        use crate::channel::{USB_REPORT_CHANNEL, send_hid_report};

        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        set_usb_state(UsbState::Configured);

        for _ in 0..crate::REPORT_CHANNEL_SIZE {
            USB_REPORT_CHANNEL
                .try_send(Report::KeyboardReport(KeyboardReport::default()))
                .expect("channel should have capacity while filling");
        }

        block_on(join(
            send_hid_report(Report::KeyboardReport(KeyboardReport::default())),
            async {
                Timer::after(Duration::from_millis(1)).await;
                let _ = USB_REPORT_CHANNEL.try_receive();
            },
        ));

        assert_eq!(USB_REPORT_CHANNEL.len(), crate::REPORT_CHANNEL_SIZE);
    }
}
