//! The keyboard's dongle event service.

use postcard::experimental::max_size::MaxSize;
use rmk_macro::Event;
use serde::{Deserialize, Serialize};
use trouble_host::prelude::{DefaultPacketPool, GattConnection};

use crate::ble::ble_server::Server;
#[cfg(feature = "split")]
use crate::event::PeripheralBatteryEvent;
use crate::event::{ActionEvent, BatteryStatusEvent, LayerChangeEvent, ModifierEvent, SleepStateEvent, WpmUpdateEvent};

pub(crate) const DONGLE_EVENT_SERVICE_UUID: u128 = 0x11b64cc4_93a2_470f_8311_c44fdc48c43c;
pub(crate) const DONGLE_EVENT_CHAR_UUID: u128 = 0xd171ca7c_971b_41a4_b717_dad40b9582e3;
#[cfg(feature = "custom_message")]
pub(crate) const CUSTOM_TO_DONGLE_UUID: u128 = 0x5f2a7c16_9b3e_4a51_8d76_2c1e4b8a6f03;
#[cfg(feature = "custom_message")]
pub(crate) const CUSTOM_TO_KEYBOARD_UUID: u128 = 0x5f2a7c17_9b3e_4a51_8d76_2c1e4b8a6f03;

pub(crate) const DONGLE_EVENT_MAX: usize = DongleEvent::POSTCARD_MAX_SIZE;

/// Keyboard events which are sent from the keyboard to the dongle.
#[derive(Event, Serialize, Deserialize, Clone, Copy, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum DongleEvent {
    Action(ActionEvent),
    Modifier(ModifierEvent),
    Layer(LayerChangeEvent),
    Wpm(WpmUpdateEvent),
    Sleep(SleepStateEvent),
    Battery(BatteryStatusEvent),
    #[cfg(feature = "split")]
    PeripheralBattery(PeripheralBatteryEvent),
}

/// Stream keyboard events to the dongle until the connection drops.
pub(crate) async fn run(server: &Server<'_>, conn: &GattConnection<'_, '_, DefaultPacketPool>) {
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;
    use embassy_sync::channel::Channel;
    use futures::FutureExt;

    #[cfg(feature = "custom_message")]
    use crate::custom_message::CustomMessageTarget;
    use crate::event::{EventSubscriber, SubscribableEvent};

    let queue: Channel<NoopRawMutex, DongleEvent, 8> = Channel::new();
    let mut action = ActionEvent::subscriber();
    let mut modifier = ModifierEvent::subscriber();
    let mut layer = LayerChangeEvent::subscriber();
    let mut wpm = WpmUpdateEvent::subscriber();
    let mut sleep = SleepStateEvent::subscriber();
    let mut battery = BatteryStatusEvent::subscriber();
    #[cfg(feature = "split")]
    let mut peripheral_battery = PeripheralBatteryEvent::subscriber();

    let queue_events = async {
        loop {
            let next_peripheral_battery = async {
                #[cfg(feature = "split")]
                {
                    DongleEvent::PeripheralBattery(peripheral_battery.next_event().await)
                }
                #[cfg(not(feature = "split"))]
                core::future::pending::<DongleEvent>().await
            };
            let event = futures::select_biased! {
                e = action.next_event().fuse() => DongleEvent::Action(e),
                e = modifier.next_event().fuse() => DongleEvent::Modifier(e),
                e = layer.next_event().fuse() => DongleEvent::Layer(e),
                e = wpm.next_event().fuse() => DongleEvent::Wpm(e),
                e = sleep.next_event().fuse() => DongleEvent::Sleep(e),
                e = battery.next_event().fuse() => DongleEvent::Battery(e),
                e = next_peripheral_battery.fuse() => e,
            };
            let _ = queue.try_send(event);
        }
    };

    let notify_events = async {
        let mut buf = [0u8; DONGLE_EVENT_MAX];
        loop {
            let event = queue.receive().await;
            if let Ok(encoded) = postcard::to_slice(&event, &mut buf) {
                let _ = server.dongle_event_service.event.notify_raw(conn, encoded, false).await;
            }
        }
    };

    // Characteristics of their own: a `DongleEvent` variant would size every
    // event's buffer to the largest message and crowd out layer and battery.
    #[cfg(not(feature = "custom_message"))]
    embassy_futures::join::join(queue_events, notify_events).await;
    #[cfg(feature = "custom_message")]
    embassy_futures::select::select3(queue_events, notify_events, {
        // This link reaches the dongle and nothing else.
        let custom_to_dongle = &server.dongle_event_service.custom_to_dongle;
        crate::custom_message::forward(Some(CustomMessageTarget::Dongle), async |encoded| {
            custom_to_dongle.notify_raw(conn, encoded, false).await
        })
    })
    .await;
}
