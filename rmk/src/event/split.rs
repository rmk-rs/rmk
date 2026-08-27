//! Split keyboard events

use postcard::experimental::max_size::MaxSize;
use rmk_macro::event;
use serde::{Deserialize, Serialize};

use super::battery::BatteryStatusEvent;

/// Peripheral connected state changed event
#[event(channel_size = crate::PERIPHERAL_CONNECTED_EVENT_CHANNEL_SIZE, pubs = crate::PERIPHERAL_CONNECTED_EVENT_PUB_SIZE, subs = crate::PERIPHERAL_CONNECTED_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PeripheralConnectedEvent {
    pub id: usize,
    pub connected: bool,
}

/// Connected to central state changed event
#[event(channel_size = crate::CENTRAL_CONNECTED_EVENT_CHANNEL_SIZE, pubs = crate::CENTRAL_CONNECTED_EVENT_PUB_SIZE, subs = crate::CENTRAL_CONNECTED_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CentralConnectedEvent {
    pub connected: bool,
}

/// Peripheral battery status changed event
#[event(channel_size = crate::PERIPHERAL_BATTERY_EVENT_CHANNEL_SIZE, pubs = crate::PERIPHERAL_BATTERY_EVENT_PUB_SIZE, subs = crate::PERIPHERAL_BATTERY_EVENT_SUB_SIZE)]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PeripheralBatteryEvent {
    pub id: usize,
    pub state: BatteryStatusEvent,
}

/// Clear BLE peer information event
#[cfg(feature = "_ble")]
#[event(channel_size = crate::CLEAR_PEER_EVENT_CHANNEL_SIZE, pubs = crate::CLEAR_PEER_EVENT_PUB_SIZE, subs = crate::CLEAR_PEER_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ClearPeerEvent;
