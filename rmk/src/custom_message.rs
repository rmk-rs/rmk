//! Application-defined messages between the boards of one keyboard, routed by
//! [`CustomMessageTarget`].

use postcard::experimental::max_size::MaxSize;
use rmk_macro::event;
use serde::{Deserialize, Serialize};

use crate::event::{EventSubscriber, SubscribableEvent, publish_event};

/// Target receiver of a custom message.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CustomMessageTarget {
    Dongle,
    Central,
    Peripherals,
}

/// A custom message defined by users.
#[event(
    channel_size = crate::CUSTOM_MESSAGE_EVENT_CHANNEL_SIZE,
    pubs = crate::CUSTOM_MESSAGE_EVENT_PUB_SIZE,
    subs = crate::CUSTOM_MESSAGE_EVENT_SUB_SIZE
)]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CustomMessage {
    pub data: heapless::Vec<u8, { crate::CUSTOM_MESSAGE_MAX_SIZE }>,
    pub target: CustomMessageTarget,
}

impl CustomMessage {
    /// `Err` when `data` is longer than `custom_message_max_size`.
    pub fn new(data: &[u8], target: CustomMessageTarget) -> Result<Self, ()> {
        Ok(Self {
            data: heapless::Vec::from_slice(data).map_err(|_| ())?,
            target,
        })
    }
}

impl MaxSize for CustomMessage {
    const POSTCARD_MAX_SIZE: usize = crate::CUSTOM_MESSAGE_MAX_SIZE + 3;
}

/// A custom message on its way out. Every link subscribes and takes only what
/// its own end of the chain should carry.
///
/// Internal, and a type of its own rather than [`CustomMessage`]: a link
/// publishes what it receives, so one shared event would have the links
/// transmit it straight back where it came from. Applications reach it through
/// [`send`], which is the only way to get a message onto a link.
#[event(
    channel_size = crate::CUSTOM_MESSAGE_OUT_EVENT_CHANNEL_SIZE,
    pubs = crate::CUSTOM_MESSAGE_OUT_EVENT_PUB_SIZE,
    subs = crate::CUSTOM_MESSAGE_OUT_EVENT_SUB_SIZE
)]
#[derive(Clone, Debug)]
pub(crate) struct CustomMessageOut(pub(crate) CustomMessage);

/// Queue `message` on every link this board has.
pub fn send(message: CustomMessage) {
    publish_event(CustomMessageOut(message));
}

/// Carry queued messages over one link, until a write fails and ends the loop.
/// A `target` of `None` takes everything queued, for a board whose only link
/// carries it all.
pub(crate) async fn forward<E>(target: Option<CustomMessageTarget>, write: impl AsyncFn(&[u8]) -> Result<(), E>) {
    let mut tx = CustomMessageOut::subscriber();
    let mut buf = [0u8; CustomMessage::POSTCARD_MAX_SIZE];
    loop {
        let message = tx.next_event().await.0;
        if let Some(target) = target
            && message.target != target
        {
            continue;
        }
        let Ok(encoded) = postcard::to_slice(&message, &mut buf) else {
            continue;
        };
        if write(encoded).await.is_err() {
            break;
        }
    }
}
