//! Connection related events
//!
//! Single event published whenever the `ConnectionStatus` changes

use rmk_macro::event;
pub use rmk_types::connection::{ConnectionStatus, ConnectionType};

/// `ConnectionStatus` changed event. Fires from `state::update_status` whenever
/// the connection status updates
#[event(channel_size = crate::CONNECTION_STATUS_CHANGE_EVENT_CHANNEL_SIZE, pubs = crate::CONNECTION_STATUS_CHANGE_EVENT_PUB_SIZE, subs = crate::CONNECTION_STATUS_CHANGE_EVENT_SUB_SIZE)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ConnectionStatusChangeEvent(pub ConnectionStatus);

impl_payload_wrapper!(ConnectionStatusChangeEvent, ConnectionStatus);
