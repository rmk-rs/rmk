//! Dongle events (`dongle` feature). A dongle answers no protocol of its own, so
//! nothing else in RMK reports what its BLE central is doing; these do.

use rmk_macro::event;

/// What the dongle's BLE central is doing. Declared in the order a link walks
/// through; everything past [`Searching`](Self::Searching) falls back to it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DongleState {
    /// Powered up, waiting for the BLE stack to come up. The state before the
    /// dongle task has run at all, hence the `Default`.
    #[default]
    Starting,
    /// Scanning for the bonded keyboard to ask for the dongle. Steady state
    /// while the keyboard is away or asleep.
    Searching,
    /// The pairing window is open: any keyboard set seeking can bond now. Only
    /// reached at boot, or when no bond is stored.
    Pairing,
    /// A keyboard was picked; the link is being established.
    Connecting,
    /// Linked, but not yet usable: pairing or encryption, then GATT discovery.
    Securing,
    /// Relaying the keyboard to the USB host. The only state in which a
    /// keystroke reaches the host.
    Connected,
}

/// The dongle's link to its keyboard: the state plus what is known about the peer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DongleStatus {
    pub state: DongleState,
    /// The bonded keyboard's address, or the candidate being connected. Little-
    /// endian as it comes off the wire, so print it reversed.
    pub peer: Option<[u8; 6]>,
    /// Signal strength from the peer, dBm. `None` until it has been seen.
    pub rssi: Option<i8>,
}

/// The keys the relayed keyboard is holding: HID usage codes exactly as they
/// crossed the relay, `0` for an empty slot, six being the rollover limit.
/// Decode with [`HidKeyCode::from_repr`](rmk_types::keycode::HidKeyCode::from_repr).
#[event(channel_size = crate::DONGLE_KEYS_EVENT_CHANNEL_SIZE, pubs = crate::DONGLE_KEYS_EVENT_PUB_SIZE, subs = crate::DONGLE_KEYS_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DongleKeysEvent {
    /// The keys held right now.
    pub keys: [u8; 6],
    /// Keys and modifiers that have gone *down* since boot, wrapping. Held keys
    /// cannot answer "was one just pressed" — a subscriber slower than the
    /// typing sees `[A]` either side of a repeat, and never sees a modifier.
    pub presses: u8,
}

/// The dongle's link status changed.
#[event(channel_size = crate::DONGLE_STATUS_EVENT_CHANNEL_SIZE, pubs = crate::DONGLE_STATUS_EVENT_PUB_SIZE, subs = crate::DONGLE_STATUS_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DongleStatusEvent(pub DongleStatus);

impl_payload_wrapper!(DongleStatusEvent, DongleStatus);
