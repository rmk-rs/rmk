//! The dongle's USB session: a byte relay between the host and the keyboard.
//!
//! Frames cross unparsed in both directions, so payload types can change
//! without a dongle firmware update. The relay reads a header for one reason
//! only: a request it cannot deliver is answered `NotReady` on that request's
//! own CMD and SEQ, because whether the keyboard is reachable is the one thing
//! the relay alone knows.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_futures::select::{Either, select};
use embassy_sync::channel::Channel;
use embassy_sync::pipe::Pipe;
use embassy_sync::signal::Signal;
use embedded_io_async::{Read, Write};
use rmk_types::constants::RYNK_BUFFER_SIZE;
use rmk_types::protocol::rynk::{RynkError, RynkHeader, encode_frame};

use crate::RawMutex;

/// Whole encoded frames (delimiter included) from the router to the keyboard's
/// `output_data` writes.
pub(crate) type RouterFrame = heapless::Vec<u8, RYNK_BUFFER_SIZE>;
pub(crate) static ROUTER_TX: Channel<RawMutex, RouterFrame, 1> = Channel::new();

/// Raised when a link stops relaying. Only a live link drains [`ROUTER_TX`], so
/// a waiting `forward_frame` must give up rather than outlive the link.
pub(crate) static LINK_DOWN: Signal<RawMutex, ()> = Signal::new();

/// Raw keyboard→host bytes from the keyboard's `input_data` notifies. Sized for
/// a few MTUs of slack so a briefly stalled host doesn't drop bytes.
pub(crate) static ROUTER_RX: Pipe<RawMutex, 1024> = Pipe::new();

/// Frames the relay answers for itself, queued for the one task that owns the
/// host transport so the two directions never write to it at once.
type ReplyFrame = heapless::Vec<u8, 16>;
static REPLY_TX: Channel<RawMutex, ReplyFrame, 2> = Channel::new();

static SESSION_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Called by the link task for every `input_data` notify: forward the raw bytes
/// when a host session is open, drop them otherwise. Never blocks — the typing
/// path shares the link's notification queue.
pub(crate) fn forward_to_host(data: &[u8]) {
    if !SESSION_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    match ROUTER_RX.try_write(data) {
        Ok(n) if n == data.len() => {}
        // A partial write corrupts the frame; the host's deframer resyncs at
        // the next delimiter and the exchange fails visibly instead of hanging.
        _ => warn!("[dongle] host config stream overflow, dropping bytes"),
    }
}

/// Serves the Rynk USB interface for a dongle binary, as `RynkService` does for
/// a keyboard; [`crate::usb::rynk::run_host_usb`] drives one session per connection.
pub struct DongleRouter;

impl DongleRouter {
    pub async fn run_session<R: Read, T: Write>(&self, rx: &mut R, tx: &mut T) {
        ROUTER_RX.clear();
        ROUTER_TX.clear();
        REPLY_TX.clear();
        SESSION_ACTIVE.store(true, Ordering::Relaxed);
        // The directions run concurrently, not turn by turn: a host frame can
        // wait milliseconds for the link task to take it, and the keyboard's
        // notifies have to keep draining meanwhile or `ROUTER_RX` overflows and
        // the response it is carrying loses bytes. Either side ending ends the
        // session.
        select(to_keyboard(rx), to_host(tx)).await;
        SESSION_ACTIVE.store(false, Ordering::Relaxed);
    }
}

/// Host→keyboard: deframe what the host sends and hand whole frames over.
async fn to_keyboard<R: Read>(rx: &mut R) {
    // Sized like the keyboard's own session buffer, so anything a keyboard
    // could parse fits here too.
    let mut buf = [0u8; RYNK_BUFFER_SIZE];
    let mut len = 0;
    let mut discard = false;

    loop {
        let n = match rx.read(&mut buf[len..]).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        len += n;
        let mut start = 0;
        while let Some(pos) = buf[start..len].iter().position(|&b| b == 0) {
            let end = start + pos + 1;
            if discard {
                discard = false; // the oversized frame's delimiter: resync
            } else if end - start > 1 {
                forward_frame(&buf[start..end]).await;
            }
            start = end;
        }
        buf.copy_within(start..len, 0);
        len -= start;
        if len == buf.len() {
            // No delimiter in a full buffer: drop and drain to the next one.
            warn!("[dongle] oversized host frame dropped");
            len = 0;
            discard = true;
        }
    }
}

/// Keyboard→host: whole frames only, so a link that dies mid-frame doesn't hand
/// the host a truncated one to resync past. Sole owner of the host transport.
async fn to_host<T: Write>(tx: &mut T) {
    let mut buf = [0u8; RYNK_BUFFER_SIZE];
    let mut len = 0;

    loop {
        let n = match select(ROUTER_RX.read(&mut buf[len..]), REPLY_TX.receive()).await {
            Either::First(n) => n,
            Either::Second(reply) => {
                if tx.write_all(&reply).await.is_err() {
                    return;
                }
                continue;
            }
        };
        len += n;
        let mut start = 0;
        while let Some(pos) = buf[start..len].iter().position(|&b| b == 0) {
            let end = start + pos + 1;
            if end - start > 1 && tx.write_all(&buf[start..end]).await.is_err() {
                return;
            }
            start = end;
        }
        buf.copy_within(start..len, 0);
        len -= start;
        if len == buf.len() {
            warn!("[dongle] oversized keyboard frame dropped");
            len = 0;
        }
    }
}

/// Hand one whole encoded frame (delimiter included) to the keyboard, or queue
/// an answer for it if there is no keyboard to hand it to.
async fn forward_frame(frame: &[u8]) {
    // LINK_DOWN is polled first so it wins the tie when the link dies mid-wait.
    if super::read_peer(|p| p.connected)
        && let Ok(copy) = RouterFrame::from_slice(frame)
        && let Either::Second(()) = select(LINK_DOWN.wait(), ROUTER_TX.send(copy)).await
    {
        return;
    }

    // Queueing it would strand it — nothing drains the queue while the keyboard
    // is away, and a keyboard that reconnects later must never replay a stale
    // request. Silence would strand the host instead, which has no timeout of
    // its own, so answer on the request's own header.
    let Some(header) = RynkHeader::peek(&frame[..frame.len() - 1]) else {
        warn!("[dongle] undecodable host frame dropped");
        return;
    };
    let mut buf = [0u8; 16];
    match encode_frame(&mut buf, header, &Err::<(), RynkError>(RynkError::NotReady)) {
        Ok(n) => match ReplyFrame::from_slice(&buf[..n]) {
            Ok(reply) => REPLY_TX.send(reply).await,
            Err(_) => warn!("[dongle] reply larger than a reply frame"),
        },
        Err(e) => warn!("[dongle] reply encode failed: {:?}", e),
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::collections::VecDeque;
    use alloc::vec::Vec;

    use embassy_futures::join::join;
    use embassy_futures::yield_now;
    use embedded_io_async::{ErrorKind, ErrorType};
    use rmk_types::protocol::rynk::{Cmd, Deframer, RYNK_HEADER_SIZE, RynkHeader};

    use super::*;
    use crate::test_support::test_block_on as block_on;

    /// Returns each chunk as one `read`; once drained, yields `idle_reads`
    /// times (so the session's other select arms get to run) and then EOF.
    struct ChunkRead {
        chunks: VecDeque<Vec<u8>>,
        idle_reads: usize,
    }

    impl ErrorType for ChunkRead {
        type Error = ErrorKind;
    }

    impl Read for ChunkRead {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            loop {
                let Some(chunk) = self.chunks.front_mut() else {
                    if self.idle_reads == 0 {
                        return Ok(0);
                    }
                    self.idle_reads -= 1;
                    yield_now().await;
                    continue;
                };
                let n = chunk.len().min(buf.len());
                buf[..n].copy_from_slice(&chunk[..n]);
                chunk.drain(..n);
                if chunk.is_empty() {
                    self.chunks.pop_front();
                }
                return Ok(n);
            }
        }
    }

    struct VecWrite {
        captured: Vec<u8>,
    }

    impl ErrorType for VecWrite {
        type Error = ErrorKind;
    }

    impl Write for VecWrite {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.captured.extend_from_slice(buf);
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    fn frame(cmd: Cmd, seq: u8, payload: &impl serde::Serialize) -> Vec<u8> {
        let mut buf = [0u8; 128];
        let n = encode_frame(&mut buf, RynkHeader { cmd, seq }, payload).unwrap();
        buf[..n].to_vec()
    }

    /// Decode a captured reply stream into `(cmd, seq, payload)` frames.
    fn decode_frames(bytes: &[u8]) -> Vec<(u16, u8, Vec<u8>)> {
        let mut work = bytes.to_vec();
        work.resize(work.len().max(8), 0);
        let mut df = Deframer::new();
        df.commit(bytes.len());
        let mut out = Vec::new();
        while let Some(n) = df.next(&mut work) {
            out.push((
                u16::from_le_bytes([work[0], work[1]]),
                work[2],
                work[RYNK_HEADER_SIZE..n].to_vec(),
            ));
        }
        out
    }

    fn run(chunks: VecDeque<Vec<u8>>, idle_reads: usize) -> Vec<u8> {
        let mut rx = ChunkRead { chunks, idle_reads };
        let mut tx = VecWrite { captured: Vec::new() };
        block_on(DongleRouter.run_session(&mut rx, &mut tx));
        tx.captured
    }

    #[test]
    fn a_connected_keyboard_gets_the_frame_byte_for_byte() {
        super::super::update_peer(|p| p.connected = true);
        let request = frame(Cmd::GetVersion, 7, &());
        let captured = run(VecDeque::from([request.clone()]), 0);

        assert!(captured.is_empty(), "forwarded frames get no local reply");
        let forwarded = ROUTER_TX.try_receive().expect("frame routed to the keyboard");
        assert_eq!(&forwarded[..], &request[..], "byte-for-byte pass-through");
    }

    #[test]
    fn a_forward_waiting_on_a_dying_link_answers_instead_of_replaying() {
        super::super::update_peer(|p| p.connected = true);
        LINK_DOWN.reset();
        ROUTER_TX.clear();
        // Occupy the single slot so the forward has to wait for room.
        ROUTER_TX.try_send(RouterFrame::new()).unwrap();

        let request = frame(Cmd::GetVersion, 11, &());
        REPLY_TX.clear();
        block_on(join(forward_frame(&request), async {
            yield_now().await;
            // The link drops with the forward still waiting.
            super::super::update_peer(|p| p.connected = false);
            LINK_DOWN.signal(());
            ROUTER_TX.clear();
        }));

        let queued = REPLY_TX.try_receive().expect("the host is answered, not left waiting");
        let resp = decode_frames(&queued);
        assert_eq!(resp.len(), 1, "the host is answered rather than left waiting");
        assert_eq!(resp[0].1, 11, "seq echo");
        assert_eq!(
            postcard::from_bytes::<Result<(), RynkError>>(&resp[0].2).unwrap(),
            Err(RynkError::NotReady),
        );
        assert!(
            ROUTER_TX.try_receive().is_err(),
            "and the request is not parked for whichever keyboard reconnects next"
        );
    }

    #[test]
    fn an_absent_keyboard_answers_not_ready_on_the_host_s_header() {
        super::super::update_peer(|p| p.connected = false);
        // Idle reads let the host-transport task drain the queued answer before
        // EOF ends the session.
        let resp = decode_frames(&run(VecDeque::from([frame(Cmd::GetVersion, 9, &())]), 2));

        assert_eq!(resp.len(), 1);
        assert_eq!(resp[0].0, Cmd::GetVersion.raw(), "cmd echo");
        assert_eq!(resp[0].1, 9, "seq echo, so the host can match the reply");
        assert_eq!(
            postcard::from_bytes::<Result<(), RynkError>>(&resp[0].2).unwrap(),
            Err(RynkError::NotReady),
        );
        assert!(ROUTER_TX.try_receive().is_err(), "and nothing is parked for later");
    }
}
