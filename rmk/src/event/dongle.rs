use rmk_macro::event;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DongleState {
    /// Waiting for the bonded keyboard to connect.
    #[default]
    Searching,
    /// The pairing window is open.
    Pairing,
    /// The bonded keyboard is connected. Relaying the keyboard to the USB host.
    Connected,
}

#[event(channel_size = crate::DONGLE_STATE_EVENT_CHANNEL_SIZE, pubs = crate::DONGLE_STATE_EVENT_PUB_SIZE, subs = crate::DONGLE_STATE_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DongleStateEvent(pub DongleState);

impl_payload_wrapper!(DongleStateEvent, DongleState);
