pub mod central;
pub mod peripheral;

use embassy_time::{Duration, with_timeout};
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};
use trouble_host::prelude::*;

use super::driver::{SplitDriverError, SplitReader, SplitWriter};
use super::{SPLIT_MESSAGE_MAX_SIZE, SplitMessage};

/// PSM of the split link's L2CAP channel. Both halves are RMK built from the
/// same config, so nothing has to be discovered: the peripheral listens on this
/// PSM and the central connects to it.
pub(crate) const SPLIT_L2CAP_PSM: u16 = 0x0080;

// An SDU is reassembled into a single pool packet, so a split message larger
// than one can never be delivered.
const _: () = core::assert!(
    SPLIT_MESSAGE_MAX_SIZE <= trouble_host::config::DEFAULT_PACKET_POOL_MTU,
    "split message exceeds the packet pool MTU; raise TROUBLE_HOST_DEFAULT_PACKET_POOL_MTU"
);

/// Backstop for a peer that stops returning credits while the link stays up.
///
/// A real disconnect is caught by the supervision timeout, 6s awake and 15s
/// asleep, and ends the session through the link watcher, so this only has to
/// cover the peer being stuck. Firing it costs a full reconnect while merely
/// blocking costs a latency spike, so keep it far longer than any credit round
/// trip, subrated ones included.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// Grant credits back once about half the window is spent. The default
/// `Every(1)` would put a credit packet on the air for every key event, and
/// granting any later leaves too little slack for the grant to travel back
/// before a fast burst exhausts the window.
const CREDIT_FLOW_POLICY: CreditFlowPolicy = CreditFlowPolicy::MinThreshold(8);

pub(crate) fn split_channel_config() -> L2capChannelConfig {
    L2capChannelConfig {
        mtu: Some(SPLIT_MESSAGE_MAX_SIZE as u16),
        flow_policy: CREDIT_FLOW_POLICY,
        ..Default::default()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PeerAddress {
    pub peer_id: u8,
    pub is_valid: bool,
    pub address: [u8; 6],
}

impl PeerAddress {
    pub(crate) fn new(peer_id: u8, is_valid: bool, address: [u8; 6]) -> Self {
        Self {
            peer_id,
            is_valid,
            address,
        }
    }
}

/// [`SplitReader`] half of the split link's L2CAP channel.
///
/// Held by the session's single read loop, which must never be raced against
/// anything: [`L2capChannelReader::receive`] is not cancellation safe. It takes
/// an SDU off the inbound queue and copies it out before awaiting the credit
/// grant, so cancelling it there loses that message, and cancelling it while
/// the grant is in flight makes the next call grant the same credits twice —
/// the peer then overruns the window, which costs the whole link.
pub(crate) struct SplitCocReader<'a, 'd, C: Controller, P: PacketPool> {
    stack: &'d Stack<'a, C, P>,
    reader: L2capChannelReader<'d, P>,
    rx: [u8; SPLIT_MESSAGE_MAX_SIZE],
}

/// [`SplitWriter`] half of the split link's L2CAP channel.
///
/// Cancelling a send is safe for a split message: it is one frame, and
/// [`CreditGrant`]'s `Drop` hands the unused credit back.
pub(crate) struct SplitCocWriter<'a, 'd, C: Controller, P: PacketPool> {
    stack: &'d Stack<'a, C, P>,
    writer: L2capChannelWriter<'d, P>,
}

/// The two halves a session runs concurrently. Splitting them is what keeps the
/// read loop free of the write loop's blocking: credits are granted from inside
/// `receive`, so a send waiting on credits must not be able to stop receiving —
/// otherwise neither half can ever make progress again.
pub(crate) fn split_coc<'a, 'd, C: Controller, P: PacketPool>(
    stack: &'d Stack<'a, C, P>,
    channel: L2capChannel<'d, P>,
) -> (SplitCocReader<'a, 'd, C, P>, SplitCocWriter<'a, 'd, C, P>) {
    let (writer, reader) = channel.split();
    (
        SplitCocReader {
            stack,
            reader,
            rx: [0; SPLIT_MESSAGE_MAX_SIZE],
        },
        SplitCocWriter { stack, writer },
    )
}

impl<C: Controller, P: PacketPool> SplitReader for SplitCocReader<'_, '_, C, P> {
    async fn read(&mut self) -> Result<SplitMessage, SplitDriverError> {
        // Every channel-level error is terminal for the session: the reconnect
        // path rebuilds the channel, so report it as a disconnect.
        let len = self.reader.receive(self.stack, &mut self.rx).await.map_err(|e| {
            #[cfg(feature = "defmt")]
            let e = defmt::Debug2Format(&e);
            error!("Split channel receive error: {:?}", e);
            SplitDriverError::Disconnected
        })?;
        postcard::from_bytes(&self.rx[..len]).map_err(|_| SplitDriverError::DeserializeError)
    }
}

impl<C: Controller, P: PacketPool> SplitWriter for SplitCocWriter<'_, '_, C, P> {
    async fn write(&mut self, message: &SplitMessage) -> Result<usize, SplitDriverError> {
        let mut buf = [0; SPLIT_MESSAGE_MAX_SIZE];
        let encoded = postcard::to_slice(message, &mut buf).map_err(|e| {
            error!("Postcard serialize split message error: {}", e);
            SplitDriverError::SerializeError
        })?;
        let len = encoded.len();
        match with_timeout(SEND_TIMEOUT, self.writer.send(self.stack, encoded)).await {
            Ok(Ok(())) => Ok(len),
            Ok(Err(e)) => {
                #[cfg(feature = "defmt")]
                let e = defmt::Debug2Format(&e);
                error!("Split channel send error: {:?}", e);
                Err(SplitDriverError::Disconnected)
            }
            Err(_) => {
                error!("Split channel send timed out waiting for credits");
                Err(SplitDriverError::Disconnected)
            }
        }
    }
}
