use core::cell::{Cell, RefCell};

use embassy_sync::blocking_mutex::Mutex;
use rmk_types::ble::BleState;
#[cfg(feature = "_ble")]
use rmk_types::ble::BleStatus;
use rmk_types::connection::{ConnectionStatus, ConnectionType, UsbState};
use rmk_types::modifier::ModifierCombination;

use crate::RawMutex;
use crate::event::{ConnectionStatusChangeEvent, publish_event};

/// Final modifier bitmap used by the most recently resolved keyboard report.
///
/// This is intentionally distinct from `Keyboard::held_modifiers`: one-shot,
/// macro, with-modifier, and fork processing all contribute to the value that
/// is actually sent to the host.
static MODIFIER_STATE: Mutex<RawMutex, Cell<ModifierCombination>> = Mutex::new(Cell::new(ModifierCombination::new()));

pub(crate) fn current_modifier_state() -> ModifierCombination {
    MODIFIER_STATE.lock(Cell::get)
}

pub(crate) fn set_modifier_state(modifiers: ModifierCombination) {
    let changed = MODIFIER_STATE.lock(|state| {
        let changed = state.get() != modifiers;
        state.set(modifiers);
        changed
    });
    if changed {
        publish_event(crate::event::ModifierEvent { modifier: modifiers });
    }
}

/// Authoritative device sleep state shared by display, lighting, host
/// readback, battery management, and split forwarding. Events invalidate this
/// value; they are not a second owner of it.
static SLEEPING: Mutex<RawMutex, Cell<bool>> = Mutex::new(Cell::new(false));

pub(crate) fn current_sleeping() -> bool {
    SLEEPING.lock(Cell::get)
}

/// Compatibility spelling used by the existing Rynk status handler.
pub(crate) fn current_sleep_state() -> bool {
    current_sleeping()
}

pub(crate) fn set_sleeping(sleeping: bool) {
    let changed = SLEEPING.lock(|state| {
        let changed = state.get() != sleeping;
        state.set(sleeping);
        changed
    });
    if changed {
        publish_event(crate::event::SleepStateEvent::new(sleeping));
    }
}

/// Single source of truth for transport state and routing. All writes go
/// through the mutator helpers below so the active-output cascade runs and
/// change events fire on every transition.
static CONNECTION_STATUS: Mutex<RawMutex, RefCell<ConnectionStatus>> =
    Mutex::new(RefCell::new(ConnectionStatus::new()));

pub(crate) fn active_transport() -> Option<ConnectionType> {
    CONNECTION_STATUS.lock(|c| c.borrow().decide_active())
}

/// Snapshot of the current connection status.
///
/// Nothing is retained on [`ConnectionStatusChangeEvent`]: subscribe before reading
/// this, or a transition landing in between is lost.
pub fn current_connection_status() -> ConnectionStatus {
    CONNECTION_STATUS.lock(|c| c.borrow().clone())
}

pub(crate) fn current_usb_state() -> UsbState {
    current_connection_status().usb
}

/// The selected BLE profile's status, with the same subscribe-first rule as [`current_connection_status`].
#[cfg(feature = "_ble")]
pub fn current_ble_status() -> BleStatus {
    current_connection_status().selected_profile()
}

/// Waits until `f` holds for the connection status, then returns that status.
#[cfg(feature = "_ble")]
pub(crate) async fn wait_for_connection_status(f: impl Fn(&ConnectionStatus) -> bool) -> ConnectionStatus {
    use crate::event::{EventSubscriber, SubscribableEvent};

    // Subscribe first, so a change made after the check below still wakes us.
    let mut changes = ConnectionStatusChangeEvent::subscriber();
    loop {
        let status = current_connection_status();
        if f(&status) {
            return status;
        }
        changes.next_event().await;
    }
}

/// Read-modify-write the connection status atomically.
pub(crate) fn update_status(f: impl FnOnce(&mut ConnectionStatus)) {
    let Some((prev, new)) = CONNECTION_STATUS.lock(|c| {
        let mut c = c.borrow_mut();
        let prev = c.clone();
        f(&mut c);
        (prev != *c).then(|| (prev, c.clone()))
    }) else {
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

    #[cfg(all(feature = "_ble", feature = "split"))]
    if prev.usb != new.usb {
        crate::split::ble::central::power_source_changed();
    }

    publish_event(ConnectionStatusChangeEvent(new));
}

pub fn set_usb_state(s: UsbState) {
    update_status(|c| c.usb = s);
}

pub(crate) fn set_ble_state(profile: u8, state: BleState) {
    update_status(|c| c.ble[profile as usize].state = state);
}

pub(crate) fn set_ble_bonded(profile: u8, bonded: bool) {
    update_status(|c| c.ble[profile as usize].bonded = bonded);
}

pub(crate) fn select_ble_profile(profile: u8) {
    update_status(|c| c.selected = profile);
}

/// Persistence is the caller's responsibility: `storage::store` the new
/// `ConnectionType`.
pub(crate) fn set_preferred_connection(t: ConnectionType) {
    update_status(|c| c.preferred = t);
}

/// Restore the persisted preferred connection at startup.
///
/// Falls back to a build-time default when nothing is stored: `Ble` when USB is
/// disabled, `Usb` otherwise.
#[cfg(feature = "_ble")]
pub(crate) async fn restore_preferred_connection() {
    if let Ok(Some(crate::storage::StorageValue::ConnectionType(c))) =
        crate::storage::read(crate::storage::StorageKey::ConnectionType).await
    {
        set_preferred_connection(c);
        return;
    }
    let default = if cfg!(feature = "_no_usb") {
        ConnectionType::Ble
    } else {
        ConnectionType::Usb
    };
    set_preferred_connection(default);
}

#[cfg(all(feature = "_ble", not(feature = "_no_usb")))]
pub(crate) async fn toggle_preferred() {
    let mut new = ConnectionType::Usb;
    update_status(|c| {
        c.preferred = match c.preferred {
            ConnectionType::Usb => ConnectionType::Ble,
            ConnectionType::Ble => ConnectionType::Usb,
        };
        new = c.preferred;
    });
    info!("Switching preferred transport to: {:?}", new);
    crate::storage::store_unchecked(crate::storage::StorageItem::ConnectionType(new)).await;
}

/// The selected BLE profile.
#[cfg(feature = "_ble")]
pub(crate) fn current_profile() -> u8 {
    CONNECTION_STATUS.lock(|c| c.borrow().selected)
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use embassy_futures::select::{Either, select};
    use embassy_sync::channel::Channel;
    use embassy_time::{Duration, Timer};
    #[cfg(feature = "_ble")]
    use rmk_types::ble::{BleState, BleStatus};
    use rmk_types::modifier::ModifierCombination;

    use super::{
        ConnectionType, MODIFIER_STATE, UsbState, current_modifier_state, set_modifier_state, set_preferred_connection,
        set_usb_state,
    };
    #[cfg(feature = "_ble")]
    use super::{select_ble_profile, set_ble_bonded, set_ble_state};
    use crate::event::{ConnectionStatusChangeEvent, EventSubscriber, ModifierEvent, SubscribableEvent};
    #[cfg(not(feature = "_no_usb"))]
    use crate::hid::MouseReport;
    use crate::hid::{KeyboardReport, Report};
    use crate::test_support::test_block_on as block_on;

    fn state_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn reset_state() {
        crate::test_support::reset_connection_status();
        MODIFIER_STATE.lock(|state| state.set(ModifierCombination::new()));
        #[cfg(not(feature = "_no_usb"))]
        crate::channel::USB_REPORT_CHANNEL.clear();
        #[cfg(feature = "_ble")]
        crate::channel::BLE_REPORT_CHANNEL.clear();
    }

    #[test]
    fn resolved_modifier_state_is_snapshotted_and_published_once() {
        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        let mut sub = ModifierEvent::subscriber();
        let modifiers = ModifierCombination::LSHIFT | ModifierCombination::RALT;

        set_modifier_state(modifiers);

        assert_eq!(current_modifier_state(), modifiers);
        assert_eq!(block_on(sub.next_event()).modifier, modifiers);

        set_modifier_state(modifiers);
        match block_on(select(Timer::after(Duration::from_millis(1)), sub.next_event())) {
            Either::First(_) => {}
            Either::Second(event) => panic!("unexpected modifier event: {:?}", event),
        }
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
    fn each_profile_update_publishes_a_status_event() {
        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        let mut sub = ConnectionStatusChangeEvent::subscriber();

        for profile in [1, 2] {
            select_ble_profile(profile);
        }

        for profile in [1, 2] {
            assert_eq!(block_on(sub.next_event()).0.selected, profile);
        }
    }

    #[cfg(feature = "_ble")]
    #[test]
    fn profile_update_changes_only_its_entry_and_keeps_the_route() {
        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        set_preferred_connection(ConnectionType::Ble);
        set_ble_state(0, BleState::Connected);
        crate::channel::BLE_REPORT_CHANNEL
            .try_send(pressed_keyboard_report())
            .unwrap();

        set_ble_state(1, BleState::Connected);
        set_ble_bonded(1, true);

        let status = super::current_connection_status();
        assert_eq!(
            status.ble[1],
            BleStatus {
                state: BleState::Connected,
                bonded: true
            }
        );
        assert_eq!(
            status.selected_profile(),
            BleStatus {
                state: BleState::Connected,
                bonded: false
            }
        );
        assert!(
            matches!(crate::channel::BLE_REPORT_CHANNEL.try_receive(), Ok(Report::KeyboardReport(r)) if r.modifier == 0x02)
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

    /// Switching between two connected profiles keeps BLE active, so the profile must cancel the send too.
    #[cfg(feature = "_ble")]
    #[test]
    fn blocked_send_drops_report_after_profile_switch() {
        use embassy_futures::join::join;

        use crate::channel::{BLE_REPORT_CHANNEL, send_hid_report};

        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        set_ble_state(0, BleState::Connected);
        set_ble_state(1, BleState::Connected);

        for _ in 0..crate::REPORT_CHANNEL_SIZE {
            BLE_REPORT_CHANNEL
                .try_send(pressed_keyboard_report())
                .expect("channel should have capacity while filling");
        }

        block_on(join(send_hid_report(pressed_keyboard_report()), async {
            Timer::after(Duration::from_millis(1)).await;
            // What `ProfileManager::switch_profile` does.
            select_ble_profile(1);
            BLE_REPORT_CHANNEL.clear();
        }));

        assert!(BLE_REPORT_CHANNEL.try_receive().is_err());
    }

    #[cfg(all(not(feature = "_no_usb"), feature = "_ble"))]
    #[test]
    fn usb_preference_flip_releases_previous_ble_transport() {
        use crate::channel::BLE_REPORT_CHANNEL;

        let _guard = state_test_lock().lock().unwrap();
        reset_state();
        set_preferred_connection(ConnectionType::Usb);
        set_ble_state(0, BleState::Connected);
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
        set_ble_state(0, BleState::Connected);

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
