//! Connection state snapshots.

use embassy_sync::watch::Receiver;
pub use rmk_types::connection::{ConnectionStatus, ConnectionType};

/// Latest connection status after a change. Intermediate updates may be coalesced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ConnectionStatusChangeEvent(pub ConnectionStatus);

impl_payload_wrapper!(ConnectionStatusChangeEvent, ConnectionStatus);

pub struct ConnectionStatusSubscriber {
    receiver: Receiver<'static, crate::RawMutex, ConnectionStatus, { crate::CONNECTION_STATUS_CHANGE_EVENT_SUB_SIZE }>,
}

impl super::EventSubscriber for ConnectionStatusSubscriber {
    type Event = ConnectionStatusChangeEvent;

    async fn next_event(&mut self) -> Self::Event {
        ConnectionStatusChangeEvent(self.receiver.changed().await)
    }
}

impl super::SubscribableEvent for ConnectionStatusChangeEvent {
    type Subscriber = ConnectionStatusSubscriber;

    fn subscriber() -> Self::Subscriber {
        ConnectionStatusSubscriber {
            receiver: crate::state::subscribe_connection_status(),
        }
    }
}
