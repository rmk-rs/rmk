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
        SESSION_ACTIVE.store(true, Ordering::Relaxed);
        self.session(rx, tx).await;
        SESSION_ACTIVE.store(false, Ordering::Relaxed);
    }

    async fn session<R: Read, T: Write>(&self, rx: &mut R, tx: &mut T) {
        // Host→dongle frames; sized like the keyboard's own session buffer, so
        // anything a keyboard could parse fits here too.
        let mut host_buf = [0u8; RYNK_BUFFER_SIZE];
        let mut host_len = 0;
        let mut host_discard = false;
        // Keyboard→host bytes, forwarded a whole frame at a time so a link that
        // dies mid-frame doesn't hand the host a truncated one to resync past.
        let mut kb_buf = [0u8; RYNK_BUFFER_SIZE];
        let mut kb_len = 0;

        loop {
            match select(
                rx.read(&mut host_buf[host_len..]),
                ROUTER_RX.read(&mut kb_buf[kb_len..]),
            )
            .await
            {
                Either::First(Ok(0)) | Either::First(Err(_)) => return,
                Either::First(Ok(n)) => {
                    host_len += n;
                    let mut start = 0;
                    while let Some(pos) = host_buf[start..host_len].iter().position(|&b| b == 0) {
                        let end = start + pos + 1;
                        if host_discard {
                            host_discard = false; // the oversized frame's delimiter: resync
                        } else if end - start > 1 && !forward_frame(&host_buf[start..end], tx).await {
                            return;
                        }
                        start = end;
                    }
                    host_buf.copy_within(start..host_len, 0);
                    host_len -= start;
                    if host_len == host_buf.len() {
                        // No delimiter in a full buffer: drop and drain to the next one.
                        warn!("[dongle] oversized host frame dropped");
                        host_len = 0;
                        host_discard = true;
                    }
                }
                Either::Second(n) => {
                    kb_len += n;
                    let mut start = 0;
                    while let Some(pos) = kb_buf[start..kb_len].iter().position(|&b| b == 0) {
                        let end = start + pos + 1;
                        if end - start > 1 && tx.write_all(&kb_buf[start..end]).await.is_err() {
                            return;
                        }
                        start = end;
                    }
                    kb_buf.copy_within(start..kb_len, 0);
                    kb_len -= start;
                    if kb_len == kb_buf.len() {
                        warn!("[dongle] oversized keyboard frame dropped");
                        kb_len = 0;
                    }
                }
            }
        }
    }
}

/// Hand one whole encoded frame (delimiter included) to the keyboard, or answer
/// it if there is no keyboard to hand it to. Returns `false` when the host
/// transport died.
async fn forward_frame<T: Write>(frame: &[u8], tx: &mut T) -> bool {
    // LINK_DOWN is polled first so it wins the tie when the link dies mid-wait.
    if super::read_peer(|p| p.connected)
        && let Ok(copy) = RouterFrame::from_slice(frame)
        && let Either::Second(()) = select(LINK_DOWN.wait(), ROUTER_TX.send(copy)).await
    {
        return true;
    }

    // Queueing it would strand it — nothing drains the queue while the keyboard
    // is away, and a keyboard that reconnects later must never replay a stale
    // request. Silence would strand the host instead, which has no timeout of
    // its own, so answer on the request's own header.
    let Some(header) = RynkHeader::peek(&frame[..frame.len() - 1]) else {
        warn!("[dongle] undecodable host frame dropped");
        return true;
    };
    let mut buf = [0u8; 16];
    match encode_frame(&mut buf, header, &Err::<(), RynkError>(RynkError::NotReady)) {
        Ok(n) => tx.write_all(&buf[..n]).await.is_ok(),
        Err(e) => {
            warn!("[dongle] reply encode failed: {:?}", e);
            true
        }
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
        let mut tx = VecWrite { captured: Vec::new() };
        block_on(join(
            async { assert!(forward_frame(&request, &mut tx).await, "host transport stays up") },
            async {
                yield_now().await;
                // The link drops with the forward still waiting.
                super::super::update_peer(|p| p.connected = false);
                LINK_DOWN.signal(());
                ROUTER_TX.clear();
            },
        ));

        let resp = decode_frames(&tx.captured);
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
        let resp = decode_frames(&run(VecDeque::from([frame(Cmd::GetVersion, 9, &())]), 0));

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
