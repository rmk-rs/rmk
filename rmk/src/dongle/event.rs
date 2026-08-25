//! The keyboard's dongle event service.

use postcard::experimental::max_size::MaxSize;
use rmk_macro::Event;
use serde::{Deserialize, Serialize};
#[cfg(feature = "host")]
use trouble_host::prelude::{DefaultPacketPool, GattConnection};

#[cfg(feature = "host")]
use crate::ble::ble_server::Server;
use crate::event::{ActionEvent, BatteryStatusEvent, LayerChangeEvent, ModifierEvent, SleepStateEvent, WpmUpdateEvent};

pub(crate) const DONGLE_EVENT_SERVICE_UUID: u128 = 0x11b64cc4_93a2_470f_8311_c44fdc48c43c;
pub(crate) const DONGLE_EVENT_CHAR_UUID: u128 = 0xd171ca7c_971b_41a4_b717_dad40b9582e3;

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
}

/// Stream keyboard events to the dongle until the connection drops.
#[cfg(feature = "host")]
pub(crate) async fn run(server: &Server<'_>, conn: &GattConnection<'_, '_, DefaultPacketPool>) {
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;
    use embassy_sync::channel::Channel;
    use futures::FutureExt;

    use crate::event::{EventSubscriber, SubscribableEvent};

    let queue: Channel<NoopRawMutex, DongleEvent, 8> = Channel::new();
    let mut action = ActionEvent::subscriber();
    let mut modifier = ModifierEvent::subscriber();
    let mut layer = LayerChangeEvent::subscriber();
    let mut wpm = WpmUpdateEvent::subscriber();
    let mut sleep = SleepStateEvent::subscriber();
    let mut battery = BatteryStatusEvent::subscriber();

    embassy_futures::join::join(
        async {
            loop {
                let event = futures::select_biased! {
                    e = action.next_event().fuse() => DongleEvent::Action(e),
                    e = modifier.next_event().fuse() => DongleEvent::Modifier(e),
                    e = layer.next_event().fuse() => DongleEvent::Layer(e),
                    e = wpm.next_event().fuse() => DongleEvent::Wpm(e),
                    e = sleep.next_event().fuse() => DongleEvent::Sleep(e),
                    e = battery.next_event().fuse() => DongleEvent::Battery(e),
                };
                let _ = queue.try_send(event);
            }
        },
        async {
            let mut buf = [0u8; DONGLE_EVENT_MAX];
            loop {
                let event = queue.receive().await;
                if let Ok(encoded) = postcard::to_slice(&event, &mut buf) {
                    let _ = server.dongle_event_service.event.notify_raw(conn, encoded, false).await;
                }
            }
        },
    )
    .await;
}
