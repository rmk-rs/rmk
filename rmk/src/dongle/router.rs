//! The dongle's USB session: a byte relay between the host and the keyboard.
//!
//! Frames cross unparsed, so payload types can change without a dongle firmware
//! update. The relay reads a header only to answer `NotReady` on the request's
//! own CMD and SEQ — whether the keyboard is reachable is all it knows.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_futures::select::select;
use embassy_sync::pipe::Pipe;
use embassy_sync::signal::Signal;
use embedded_io_async::{Read, Write};
use rmk_types::constants::RYNK_BUFFER_SIZE;
use rmk_types::protocol::rynk::{RYNK_BLE_CHUNK_SIZE, RynkError, RynkHeader, encode_frame};

use crate::RawMutex;

/// One max-size frame plus two notifies of slack: any less and a full frame in
/// flight leaves no room for the notify behind it, costing a frame per bulk read.
const TO_HOST_SIZE: usize = RYNK_BUFFER_SIZE + 2 * RYNK_BLE_CHUNK_SIZE;

/// Serves the Rynk USB interface for a dongle binary, as `RynkService` does for
/// a keyboard. `main` owns one and lends it to both sides:
/// [`crate::usb::rynk::run_host_usb`] drives a session per connection, and the
/// [`super::Dongle`] task relays what the session queues.
pub struct DongleRouter {
    /// Raw bytes waiting for the keyboard's `output_data` writes.
    pub(super) to_keyboard: Pipe<RawMutex, RYNK_BUFFER_SIZE>,
    /// Raw bytes waiting for the host: the keyboard's `input_data` notifies plus
    /// the relay's own replies.
    pub(super) to_host: Pipe<RawMutex, TO_HOST_SIZE>,
    /// The dongle task has a keyboard connected and is draining `to_keyboard`.
    link_connected: AtomicBool,
    /// Raised when a link stops relaying: only a live link drains `to_keyboard`,
    /// so a waiting [`Self::host_to_keyboard`] has to give up.
    link_dropped: Signal<RawMutex, ()>,
}

impl DongleRouter {
    pub const fn new() -> Self {
        Self {
            to_keyboard: Pipe::new(),
            to_host: Pipe::new(),
            link_connected: AtomicBool::new(false),
            link_dropped: Signal::new(),
        }
    }

    /// Called by the dongle task once it is relaying: open the host→keyboard path.
    pub(super) fn link_up(&self) {
        // `link_dropped` latches; clear it and the queue before the first forward.
        self.link_dropped.reset();
        self.to_keyboard.clear();
        self.link_connected.store(true, Ordering::Relaxed);
    }

    /// Called by the dongle task when the link drops: fail waiting forwards and
    /// drop the queue, so a keyboard connecting later can't replay a stale request.
    pub(super) fn link_down(&self) {
        self.link_connected.store(false, Ordering::Relaxed);
        self.link_dropped.signal(());
        self.to_keyboard.clear();
        // Close off whatever frame the link left mid-notify: glued to the next
        // link's first response, the two can still decode as a bogus frame.
        let _ = self.to_host.try_write(&[0]);
    }

    pub async fn run_session<R: Read, T: Write>(&self, rx: &mut R, tx: &mut T) {
        self.to_host.clear();
        self.to_keyboard.clear();
        // Concurrent, not turn by turn: a host frame can wait milliseconds for the
        // dongle task, and `to_host` overflows if notifies stop draining meanwhile.
        select(self.host_to_keyboard(rx), self.keyboard_to_host(tx)).await;
    }

    /// Host -> keyboard: bytes cross as they arrive. The only state is the head of
    /// the frame in flight — all [`RynkHeader::peek`] needs to answer an absent
    /// keyboard on the request's own CMD and SEQ.
    async fn host_to_keyboard<R: Read>(&self, rx: &mut R) {
        let mut buf = [0u8; RYNK_BLE_CHUNK_SIZE];
        let mut head: heapless::Vec<u8, 8> = heapless::Vec::new();
        loop {
            let n = match rx.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            // Each piece ends at a delimiter, except a trailing partial frame.
            for piece in buf[..n].split_inclusive(|&b| b == 0) {
                let ends_frame = piece.ends_with(&[0]);
                let body = if ends_frame { &piece[..piece.len() - 1] } else { piece };
                head.extend_from_slice(&body[..body.len().min(head.capacity() - head.len())])
                    .ok();

                // `link_dropped` is polled first so it wins the tie when the link dies
                // mid-write; the keyboard's deframer resyncs at the next delimiter.
                if self.link_connected.load(Ordering::Relaxed) {
                    let _ = select(self.link_dropped.wait(), self.to_keyboard.write_all(piece)).await;
                }
                if !ends_frame {
                    continue;
                }
                // Whole frame seen. Nothing drains the queue while the keyboard is
                // away and the host has no timeout of its own, so answer here; a
                // bare delimiter carries no request to answer.
                if !head.is_empty() && !self.link_connected.load(Ordering::Relaxed) {
                    if let Some(header) = RynkHeader::peek(&head) {
                        let mut reply = [0u8; 16];
                        match encode_frame(&mut reply[1..], header, &Err::<(), RynkError>(RynkError::NotReady)) {
                            // `reply[0]` is a bare delimiter, closing off any partial frame in the stream.
                            Ok(n) => self.to_host.write_all(&reply[..n + 1]).await,
                            Err(e) => warn!("[dongle] reply encode failed: {:?}", e),
                        }
                    } else {
                        warn!("[dongle] undecodable host frame dropped");
                    }
                }
                head.clear();
            }
        }
    }

    /// Keyboard -> host: raw bytes forwarded as they arrive.
    async fn keyboard_to_host<T: Write>(&self, tx: &mut T) {
        let mut buf = [0u8; RYNK_BLE_CHUNK_SIZE];
        loop {
            let n = self.to_host.read(&mut buf).await;
            if tx.write_all(&buf[..n]).await.is_err() {
                return;
            }
        }
    }
}

impl Default for DongleRouter {
    fn default() -> Self {
        Self::new()
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

    fn run(router: &DongleRouter, chunks: VecDeque<Vec<u8>>, idle_reads: usize) -> Vec<u8> {
        let mut rx = ChunkRead { chunks, idle_reads };
        let mut tx = VecWrite { captured: Vec::new() };
        block_on(router.run_session(&mut rx, &mut tx));
        tx.captured
    }

    #[test]
    fn a_connected_keyboard_gets_the_frame_byte_for_byte() {
        let router = DongleRouter::new();
        router.link_up();
        let request = frame(Cmd::GetVersion, 7, &());
        let captured = run(&router, VecDeque::from([request.clone()]), 0);

        assert!(captured.is_empty(), "forwarded frames get no local reply");
        let mut forwarded = [0u8; 64];
        let n = router
            .to_keyboard
            .try_read(&mut forwarded)
            .expect("bytes routed to the keyboard");
        assert_eq!(&forwarded[..n], &request[..], "byte-for-byte pass-through");
    }

    #[test]
    fn a_forward_waiting_on_a_dying_link_answers_instead_of_replaying() {
        let router = DongleRouter::new();
        router.link_up();
        // Fill the pipe so the forward has to wait for room.
        while router.to_keyboard.try_write(&[0xFF; 64]).is_ok() {}

        let request = frame(Cmd::GetVersion, 11, &());
        let mut rx = ChunkRead {
            chunks: VecDeque::from([request.clone()]),
            idle_reads: 4,
        };
        block_on(join(router.host_to_keyboard(&mut rx), async {
            yield_now().await;
            // The link drops with the forward still waiting.
            router.link_down();
        }));

        let mut queued = [0u8; 32];
        let n = router.to_host.try_read(&mut queued).expect("a reply is queued");
        let resp = decode_frames(&queued[..n]);
        assert_eq!(resp.len(), 1, "the host is answered rather than left waiting");
        assert_eq!(resp[0].1, 11, "seq echo");
        assert_eq!(
            postcard::from_bytes::<Result<(), RynkError>>(&resp[0].2).unwrap(),
            Err(RynkError::NotReady),
        );
        let mut parked = [0u8; 8];
        assert!(
            router.to_keyboard.try_read(&mut parked).is_err(),
            "and the request is not parked for whichever keyboard reconnects next"
        );
    }

    #[test]
    fn a_link_that_dies_mid_frame_does_not_swallow_the_next_response() {
        let router = DongleRouter::new();
        let response = frame(Cmd::GetVersion, 3, &());
        let mut rx = ChunkRead {
            chunks: VecDeque::new(),
            idle_reads: 8,
        };
        let mut tx = VecWrite { captured: Vec::new() };

        block_on(join(router.run_session(&mut rx, &mut tx), async {
            yield_now().await;
            router.link_up();
            // A response the keyboard only got halfway through notifying.
            router.to_host.try_write(&response[..3]).unwrap();
            yield_now().await;
            router.link_down();
            yield_now().await;
            router.link_up();
            router.to_host.try_write(&response).unwrap();
            yield_now().await;
        }));

        let resp = decode_frames(&tx.captured);
        assert_eq!(
            resp.len(),
            1,
            "the truncated frame is dropped, not merged with the next"
        );
        assert_eq!(resp[0].1, 3, "and the next response reaches the host intact");
    }

    /// The relay keeps no frame buffer, so a request split across reads must still
    /// be answered exactly once — on the header it started with.
    #[test]
    fn a_request_split_across_reads_is_answered_once() {
        let router = DongleRouter::new();
        let request = frame(Cmd::GetVersion, 21, &());
        let (head, tail) = request.split_at(2);
        let captured = run(&router, VecDeque::from([head.to_vec(), tail.to_vec()]), 2);

        let resp = decode_frames(&captured);
        assert_eq!(resp.len(), 1, "one request, one answer");
        assert_eq!(resp[0].1, 21, "seq echo survives the split");
    }

    #[test]
    fn an_absent_keyboard_answers_not_ready_on_the_host_s_header() {
        let router = DongleRouter::new();
        // Idle reads let the host-transport task drain the queued answer before
        // EOF ends the session.
        let captured = run(&router, VecDeque::from([frame(Cmd::GetVersion, 9, &())]), 2);
        let resp = decode_frames(&captured);

        assert_eq!(resp.len(), 1);
        assert_eq!(resp[0].0, Cmd::GetVersion.raw(), "cmd echo");
        assert_eq!(resp[0].1, 9, "seq echo, so the host can match the reply");
        assert_eq!(
            postcard::from_bytes::<Result<(), RynkError>>(&resp[0].2).unwrap(),
            Err(RynkError::NotReady),
        );
        let mut parked = [0u8; 8];
        assert!(
            router.to_keyboard.try_read(&mut parked).is_err(),
            "and nothing is parked for later"
        );
    }
}
