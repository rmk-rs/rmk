//! The dongle's USB session for a Vial keyboard: a 32-byte report relay.
//!
//! Vial is strict request/response with no framing, so whole reports cross 1:1
//! in both directions. The relay reads nothing but the first byte — and only to
//! echo the request back as `Unhandled` when there is no keyboard to ask.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_futures::select::{Either, select};
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embedded_io_async::{Read, Write};
use rmk_types::protocol::vial::{VIAL_EP_SIZE, ViaCommand};

use crate::RawMutex;

/// One Vial request or reply, always exactly one HID report.
pub(super) type VialReport = [u8; VIAL_EP_SIZE];

/// Serves the Vial USB interface for a dongle binary, as `VialService` does for
/// a keyboard; [`crate::usb::vial::run_host_usb`] drives one session per
/// connection. The binary's `main` owns one and lends it to both sides that meet
/// here: the [`crate::usb::UsbTransport`] running the sessions, and the
/// [`super::Dongle`] whose dongle task relays what they queue.
pub struct DongleRouter {
    /// Requests waiting for the keyboard's `output_data` writes.
    pub(super) to_keyboard: Channel<RawMutex, VialReport, 1>,
    /// Replies the keyboard notified, plus the relay's own answers. Vial is
    /// turn-by-turn so one slot would do; the rest is headroom.
    pub(super) to_host: Channel<RawMutex, VialReport, 4>,
    /// The dongle task has a keyboard connected and is draining `to_keyboard`.
    link_connected: AtomicBool,
    /// Raised when a link stops relaying: only a live link drains `to_keyboard`,
    /// so a waiting [`Self::forward_report`] has to give up.
    link_dropped: Signal<RawMutex, ()>,
}

impl DongleRouter {
    pub const fn new() -> Self {
        Self {
            to_keyboard: Channel::new(),
            to_host: Channel::new(),
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
    }

    pub async fn run_session<R: Read, T: Write>(&self, rx: &mut R, tx: &mut T) {
        self.to_host.clear();
        self.to_keyboard.clear();
        // Concurrent, not turn by turn: a request can wait milliseconds for the
        // dongle task, and replies must keep draining meanwhile.
        select(self.host_to_keyboard(rx), self.keyboard_to_host(tx)).await;
    }

    async fn host_to_keyboard<R: Read>(&self, rx: &mut R) {
        loop {
            let mut report = [0u8; VIAL_EP_SIZE];
            if rx.read_exact(&mut report).await.is_err() {
                return;
            }
            self.forward_report(report).await;
        }
    }

    async fn keyboard_to_host<T: Write>(&self, tx: &mut T) {
        loop {
            let report = self.to_host.receive().await;
            if tx.write_all(&report).await.is_err() {
                return;
            }
        }
    }

    /// Hand one request to the keyboard, or answer it here if there is no
    /// keyboard to hand it to.
    async fn forward_report(&self, mut report: VialReport) {
        // `link_dropped` is polled first so it wins the tie when the link dies mid-wait.
        if self.link_connected.load(Ordering::Relaxed)
            && let Either::Second(()) = select(self.link_dropped.wait(), self.to_keyboard.send(report)).await
        {
            return;
        }

        // Queueing it would strand it — nothing drains the queue while the keyboard
        // is away — and the host has no timeout of its own, so answer it here with
        // the protocol's unknown-command echo.
        report[0] = ViaCommand::Unhandled as u8;
        self.to_host.send(report).await;
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

    fn request(cmd: u8) -> VialReport {
        let mut report = [0u8; VIAL_EP_SIZE];
        report[0] = cmd;
        report[1] = 0xAB; // marker: replies must echo the request's tail
        report
    }

    fn run(router: &DongleRouter, chunks: VecDeque<Vec<u8>>, idle_reads: usize) -> Vec<u8> {
        let mut rx = ChunkRead { chunks, idle_reads };
        let mut tx = VecWrite { captured: Vec::new() };
        block_on(router.run_session(&mut rx, &mut tx));
        tx.captured
    }

    #[test]
    fn a_connected_keyboard_gets_the_report_byte_for_byte() {
        let router = DongleRouter::new();
        router.link_up();
        let req = request(0x01);
        let captured = run(&router, VecDeque::from([req.to_vec()]), 0);

        assert!(captured.is_empty(), "forwarded requests get no local reply");
        let forwarded = router.to_keyboard.try_receive().expect("report routed to the keyboard");
        assert_eq!(forwarded, req, "byte-for-byte pass-through");
    }

    #[test]
    fn an_absent_keyboard_answers_the_unhandled_echo() {
        let router = DongleRouter::new();
        // Idle reads let the host-transport task drain the queued answer before
        // EOF ends the session.
        let captured = run(&router, VecDeque::from([request(0x01).to_vec()]), 2);

        assert_eq!(captured.len(), VIAL_EP_SIZE, "exactly one reply");
        assert_eq!(captured[0], ViaCommand::Unhandled as u8);
        assert_eq!(&captured[1..], &request(0x01)[1..], "the request's tail is echoed");
        assert!(
            router.to_keyboard.try_receive().is_err(),
            "and nothing is parked for later"
        );
    }

    #[test]
    fn a_forward_waiting_on_a_dying_link_answers_instead_of_replaying() {
        let router = DongleRouter::new();
        router.link_up();
        // Occupy the single slot so the forward has to wait for room.
        router.to_keyboard.try_send([0u8; VIAL_EP_SIZE]).unwrap();

        block_on(join(router.forward_report(request(0x02)), async {
            yield_now().await;
            // The link drops with the forward still waiting.
            router.link_down();
        }));

        let reply = router.to_host.try_receive().expect("a reply is queued");
        assert_eq!(reply[0], ViaCommand::Unhandled as u8);
        assert_eq!(
            &reply[1..],
            &request(0x02)[1..],
            "seqless protocol: the echo is the match"
        );
        assert!(
            router.to_keyboard.try_receive().is_err(),
            "and the request is not parked for whichever keyboard reconnects next"
        );
    }

    #[test]
    fn a_keyboard_reply_reaches_the_host_intact() {
        let router = DongleRouter::new();
        let mut reply = [0u8; VIAL_EP_SIZE];
        reply[0] = 0x01;
        reply[31] = 0xEE;
        let mut rx = ChunkRead {
            chunks: VecDeque::new(),
            idle_reads: 4,
        };
        let mut tx = VecWrite { captured: Vec::new() };

        block_on(join(router.run_session(&mut rx, &mut tx), async {
            yield_now().await;
            router.link_up();
            router.to_host.try_send(reply).unwrap();
            yield_now().await;
        }));

        assert_eq!(tx.captured, reply.to_vec(), "one whole report per write");
    }
}
