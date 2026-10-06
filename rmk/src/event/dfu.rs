//! DFU events

use rmk_macro::event;
use rmk_types::dfu::DfuStatus;

use crate::dfu::DfuCmd;

/// DFU status changed event
#[event(
    channel_size = crate::DFU_STATUS_EVENT_CHANNEL_SIZE,
    pubs = crate::DFU_STATUS_EVENT_PUB_SIZE,
    subs = crate::DFU_STATUS_EVENT_SUB_SIZE
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DfuStatusEvent(pub DfuStatus);

impl DfuStatusEvent {
    pub fn new(status: DfuStatus) -> Self {
        Self(status)
    }
}

impl_payload_wrapper!(DfuStatusEvent, DfuStatus);

/// DFU command event — published by the USB proxy (ISR context) and consumed
/// by [`FlashDfuHandler`](crate::dfu::FlashDfuHandler) (central) and
/// `PeripheralManager` (peripheral passthrough).
#[event(
    channel_size = crate::DFU_CMD_EVENT_CHANNEL_SIZE,
    pubs = crate::DFU_CMD_EVENT_PUB_SIZE,
    subs = crate::DFU_CMD_EVENT_SUB_SIZE
)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DfuCmdEvent(pub(crate) DfuCmd);
