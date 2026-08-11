//! The dongle's USB session: a byte relay between the host and the keyboard.
//!
//! Frames cross unparsed, so payload types can change without a dongle firmware
//! update. The relay reads a header only to answer `NotReady` on the request's
//! own CMD and SEQ — whether the keyboard is reachable is all it knows.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_futures::select::{Either, select};
use embassy_sync::channel::Channel;
use embassy_sync::pipe::Pipe;
use embassy_sync::signal::Signal;
use embedded_io_async::{Read, Write};
use rmk_types::constants::RYNK_BUFFER_SIZE;
use rmk_types::protocol::rynk::{RYNK_BLE_CHUNK_SIZE, RynkError, RynkHeader, encode_frame};

use crate::RawMutex;

/// A whole encoded frame, delimiter included.
type RouterFrame = heapless::Vec<u8, RYNK_BUFFER_SIZE>;

/// A whole max-size frame, plus two notifies of slack for a briefly stalled
/// host. Anything less and one full frame in flight leaves no room for the
/// notify behind it, which costs a frame every bulk read.
const TO_HOST_SIZE: usize = RYNK_BUFFER_SIZE + 2 * RYNK_BLE_CHUNK_SIZE;

/// Serves the Rynk USB interface for a dongle binary, as `RynkService` does for
/// a keyboard; [`crate::usb::rynk::run_host_usb`] drives one session per
/// connection. The binary's `main` owns one and lends it to both sides that meet
/// here: the [`crate::usb::UsbTransport`] running the sessions, and the
/// [`super::Dongle`] whose dongle task relays what they queue.
pub struct DongleRouter {
    /// Frames waiting for the keyboard's `output_data` writes.
    pub(super) to_keyboard: Channel<RawMutex, RouterFrame, 1>,
    /// Raw bytes waiting for the host: the keyboard's `input_data` notifies plus
    /// the relay's own replies.
    pub(super) to_host: Pipe<RawMutex, TO_HOST_SIZE>,
    /// The dongle task has a keyboard connected and is draining `to_keyboard`.
    link_connected: AtomicBool,
    /// Raised when a link stops relaying: only a live link drains `to_keyboard`,
    /// so a waiting [`Self::forward_frame`] has to give up.
    link_dropped: Signal<RawMutex, ()>,
}

impl DongleRouter {
    pub const fn new() -> Self {
        Self {
            to_keyboard: Channel::new(),
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

    /// Host→keyboard: deframe what the host sends and hand whole frames over.
    async fn host_to_keyboard<R: Read>(&self, rx: &mut R) {
        let mut frames = FrameSplitter::new("host");
        loop {
            let n = match rx.read(frames.spare()).await {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            frames.commit(n);
            while let Some(frame) = frames.next_frame() {
                self.forward_frame(frame).await;
            }
        }
    }

    /// Keyboard→host: one frame per write, which is what the host transport's
    /// zero-length-packet rule is written for. Sole owner of that transport.
    async fn keyboard_to_host<T: Write>(&self, tx: &mut T) {
        let mut frames = FrameSplitter::new("keyboard");
        loop {
            let n = self.to_host.read(frames.spare()).await;
            frames.commit(n);
            while let Some(frame) = frames.next_frame() {
                if tx.write_all(frame).await.is_err() {
                    return;
                }
            }
        }
    }

    /// Hand one whole encoded frame (delimiter included) to the keyboard, or
    /// answer it here if there is no keyboard to hand it to.
    async fn forward_frame(&self, frame: &[u8]) {
        // `link_dropped` is polled first so it wins the tie when the link dies mid-wait.
        if self.link_connected.load(Ordering::Relaxed)
            && let Ok(copy) = RouterFrame::from_slice(frame)
            && let Either::Second(()) = select(self.link_dropped.wait(), self.to_keyboard.send(copy)).await
        {
            return;
        }

        // Queueing it would strand it — nothing drains the queue while the keyboard
        // is away — and the host has no timeout of its own, so answer it here.
        let Some(header) = RynkHeader::peek(&frame[..frame.len() - 1]) else {
            warn!("[dongle] undecodable host frame dropped");
            return;
        };
        let mut buf = [0u8; 16];
        match encode_frame(&mut buf[1..], header, &Err::<(), RynkError>(RynkError::NotReady)) {
            // `buf[0]` is a bare delimiter, closing off any partial frame in the stream.
            Ok(n) => self.to_host.write_all(&buf[..n + 1]).await,
            Err(e) => warn!("[dongle] reply encode failed: {:?}", e),
        }
    }
}

impl Default for DongleRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Splits a byte stream into whole delimiter-terminated frames without decoding
/// them, which rmk_types' `Deframer` can't do — it decodes in place. Sized like
/// the keyboard's session buffer, so anything a keyboard could parse fits.
struct FrameSplitter {
    buf: [u8; RYNK_BUFFER_SIZE],
    len: usize,
    /// Scan position: frames before it have been yielded.
    start: usize,
    /// Mid-oversized-frame: eat bytes until its delimiter resyncs the stream.
    discard: bool,
    /// Names the sender when an oversized frame is dropped.
    from: &'static str,
}

impl FrameSplitter {
    const fn new(from: &'static str) -> Self {
        Self {
            buf: [0; RYNK_BUFFER_SIZE],
            len: 0,
            start: 0,
            discard: false,
            from,
        }
    }

    /// Free space to read the next chunk into; never empty. Follow with `commit`.
    fn spare(&mut self) -> &mut [u8] {
        &mut self.buf[self.len..]
    }

    fn commit(&mut self, n: usize) {
        self.len += n;
    }

    /// The next whole frame (delimiter included), or `None` once the buffered
    /// bytes hold no more — which compacts, so `spare` has room again.
    fn next_frame(&mut self) -> Option<&[u8]> {
        while let Some(pos) = self.buf[self.start..self.len].iter().position(|&b| b == 0) {
            let start = self.start;
            self.start += pos + 1;
            if core::mem::take(&mut self.discard) || pos == 0 {
                continue; // an oversized frame's delimiter, or a bare one
            }
            return Some(&self.buf[start..self.start]);
        }
        self.buf.copy_within(self.start..self.len, 0);
        self.len -= self.start;
        self.start = 0;
        if self.len == self.buf.len() {
            // No delimiter in a full buffer: drop and drain to the next one.
            warn!("[dongle] oversized {} frame dropped", self.from);
            self.len = 0;
            self.discard = true;
        }
        None
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
        let forwarded = router.to_keyboard.try_receive().expect("frame routed to the keyboard");
        assert_eq!(&forwarded[..], &request[..], "byte-for-byte pass-through");
    }

    #[test]
    fn a_forward_waiting_on_a_dying_link_answers_instead_of_replaying() {
        let router = DongleRouter::new();
        router.link_up();
        // Occupy the single slot so the forward has to wait for room.
        router.to_keyboard.try_send(RouterFrame::new()).unwrap();

        let request = frame(Cmd::GetVersion, 11, &());
        block_on(join(router.forward_frame(&request), async {
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
        assert!(
            router.to_keyboard.try_receive().is_err(),
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
        assert!(
            router.to_keyboard.try_receive().is_err(),
            "and nothing is parked for later"
        );
    }
}
