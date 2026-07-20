//! Protocol driver for Rynk: the [`Client`] with its request and topic
//! methods, and the [`Driver`] that moves the bytes. Sessions are created by
//! [`RynkDevice::connect`](crate::RynkDevice::connect).
//!
//! ## Frame flow
//!
//! A frame is a 3-byte header (`CMD u16 LE | SEQ u8`) plus a postcard
//! payload. On the wire it is COBS-encoded and ends with a `0x00`, so after
//! any garbage the stream finds the next frame boundary again on its own.
//! [`encode_frame`] builds outgoing frames, a [`Deframer`] cuts incoming
//! ones back out of the byte stream, and the driver routes each frame by the
//! topic bit in its CMD:
//!
//! ```text
//! request()    encode → message ─→ Driver: write_all
//! Driver: read → deframe → route by the CMD topic bit:
//!          topic frame → decode → topics ─→ next_topic()
//!          reply frame → SEQ-matched slot ─→ request(): decode
//! ```
//!
//! ## Session lifecycle
//!
//! [`Driver::run`] returns when the link dies. There is no in-band death
//! signal: a call waiting on a dead session never finishes on its own. So run
//! the driver in the same `select` as everything that awaits on the
//! [`Client`] — see the crate docs for how to set this up.

#[cfg(feature = "alloc")]
use alloc::{string::String, vec, vec::Vec};
use core::sync::atomic::{AtomicU8, Ordering};

use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Channel, TrySendError};
use embassy_sync::signal::Signal;
use embedded_io_async::{Error as _, ErrorKind, Read, Write};
use rmk_types::protocol::rynk::command::Endpoint;
use rmk_types::protocol::rynk::{
    Cmd, Deframer, DeviceCapabilities, LightingError, RYNK_HEADER_SIZE, RynkError, RynkHeader, TopicEvent, encode_frame, max_wire_size,
};
use serde::Serialize;
use thiserror::Error;

type CS = CriticalSectionRawMutex;

/// One whole frame as owned bytes: either a COBS-encoded request waiting for
/// the writer, or a decoded reply handed back to the requester. The no-alloc
/// capacity is the firmware's frame-buffer size, so any frame the firmware
/// can send or accept fits.
#[cfg(feature = "alloc")]
type FrameBytes = Vec<u8>;
#[cfg(not(feature = "alloc"))]
type FrameBytes = heapless::Vec<u8, { rmk_types::constants::RYNK_BUFFER_SIZE }>;

/// How many topic events can queue up before the oldest is dropped.
const TOPIC_QUEUE_CAPACITY: usize = 8;

/// Errors from Rynk host.
#[derive(Debug, Error)]
pub enum RynkHostError {
    #[error("transport disconnected")]
    Disconnected,
    #[error("io error: {0:?}")]
    Io(ErrorKind),
    /// A transport step (GATT attach, port open, …) failed. The detail string
    /// is what a device picker or GUI shows when the chosen device can't be
    /// reached.
    #[cfg(feature = "alloc")]
    #[error("transport {0} failed: {1}")]
    Transport(&'static str, String),
    #[cfg(feature = "alloc")]
    #[error("device not found: {0}")]
    DeviceNotFound(String),

    #[error(
        "protocol major version mismatch — firmware speaks v{firmware_major}.{firmware_minor}, this tool speaks \
         v{host_major}.x (currently v{host_major}.{host_max_minor}). Use a tool matching major {firmware_major}, or \
         flash firmware that matches this one."
    )]
    VersionMismatch {
        firmware_major: u8,
        firmware_minor: u8,
        host_major: u8,
        host_max_minor: u8,
    },

    /// The firmware received the request but answered with an error.
    #[error("device rejected {0:?}")]
    Rejected(RynkError),
    /// Rynk framing succeeded, but the lighting service rejected the command
    /// with domain-specific detail.
    #[error("lighting command rejected: {0:?}")]
    LightingRejected(LightingError),
    /// The request failed to encode or exceeds the device's advertised
    /// `max_payload_size`.
    #[error("request {0:?} does not fit the device buffer (or failed to encode)")]
    Encode(Cmd),
    #[error("response decode failed for {cmd:?}: {source}")]
    Deserialize { cmd: Cmd, source: postcard::Error },
    /// Decompressing or decoding the `GetLayout` blob failed.
    #[cfg(feature = "alloc")]
    #[error("layout blob decode failed: {0}")]
    Layout(String),
    #[error("response for {cmd:?} had trailing bytes")]
    TrailingBytes { cmd: Cmd },
    #[error("response cmd mismatch: sent {sent:?}, got {got:?}")]
    CmdMismatch { sent: Cmd, got: Cmd },
    /// The device's capabilities lack this command, so nothing was sent.
    #[error("device does not support {0:?}: {1}")]
    Unsupported(Cmd, &'static str),
}

/// Convert a host error into a JS `Error` whose `name` is a stable kind
/// string, so JS code can match on it.
#[cfg(feature = "wasm")]
impl From<RynkHostError> for wasm_bindgen::JsValue {
    fn from(e: RynkHostError) -> Self {
        let kind = match &e {
            RynkHostError::Disconnected => "Disconnected",
            RynkHostError::Io(_) | RynkHostError::Transport(..) => "TransportError",
            RynkHostError::DeviceNotFound(_) => "DeviceNotFound",
            RynkHostError::Rejected(_) => "Rejected",
            RynkHostError::LightingRejected(_) => "LightingRejected",
            RynkHostError::Unsupported(..) => "Unsupported",
            RynkHostError::VersionMismatch { .. } => "VersionMismatch",
            RynkHostError::Encode(_) => "RequestEncodeError",
            RynkHostError::Deserialize { .. } => "ResponseDecodeError",
            RynkHostError::Layout(_) => "LayoutDecodeError",
            RynkHostError::TrailingBytes { .. } => "ResponseTrailingBytes",
            RynkHostError::CmdMismatch { .. } => "ResponseCommandMismatch",
        };
        let err = js_sys::Error::new(&e.to_string());
        err.set_name(kind);
        err.into()
    }
}

/// How many requests can be in flight at once. Callers beyond this wait on
/// the free-list until a slot opens. No-alloc builds hold a full frame buffer
/// per slot, so they stay at one slot instead of paying RAM for four.
#[cfg(feature = "alloc")]
pub(crate) const MAX_IN_FLIGHT: usize = 4;
#[cfg(not(feature = "alloc"))]
pub(crate) const MAX_IN_FLIGHT: usize = 1;

/// One in-flight request: the SEQ it waits for (0 means the slot is free;
/// real SEQs cycle `1..=255`) and the signal its reply arrives on.
struct Slot {
    seq: AtomicU8,
    resp: Signal<CS, FrameBytes>,
}

/// Frees the slot on drop, so a cancelled request's late reply is dropped as
/// unmatched instead of reaching the slot's next user.
struct SlotGuard<'a> {
    client: &'a Client,
    idx: usize,
}

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        let slot = &self.client.slots[self.idx];
        slot.seq.store(0, Ordering::Release);
        slot.resp.reset();
        // Capacity equals the slot count, so the free-list always has room.
        let _ = self.client.free.try_send(self.idx);
    }
}

/// The Rynk protocol surface: typed requests plus the topic stream. Both
/// take `&self`, so one shared client can serve request calls and a topic
/// loop at the same time. Up to `MAX_IN_FLIGHT` requests can run
/// concurrently; replies are matched back by SEQ, so they may complete in
/// any order. Moving the actual bytes is [`Driver::run`]'s job.
pub struct Client {
    /// Client → Driver: request frames waiting to be written.
    message: Channel<CS, FrameBytes, 1>,
    /// In-flight requests; the driver delivers each reply here by SEQ.
    slots: [Slot; MAX_IN_FLIGHT],
    /// Free slot indices. Receiving an index claims that slot, so callers
    /// beyond [`MAX_IN_FLIGHT`] wait here instead of flooding the device.
    free: Channel<CS, usize, MAX_IN_FLIGHT>,
    /// Driver → topic consumer, decoded as they arrive. When full the oldest
    /// is dropped: topics are best-effort by contract, and a missed push can
    /// be recovered with the matching `get_*` call.
    topics: Channel<CS, TopicEvent, TOPIC_QUEUE_CAPACITY>,
    /// Request SEQ, cycling through `1..=255`.
    next_seq: AtomicU8,
    /// Capability snapshot from the connect handshake;
    /// [`RynkDevice::connect`](crate::RynkDevice::connect) fills it in before
    /// the client is shared.
    pub(crate) capabilities: DeviceCapabilities,
}

impl Client {
    pub(crate) fn new() -> Self {
        let free = Channel::new();
        for idx in 0..MAX_IN_FLIGHT {
            let _ = free.try_send(idx);
        }
        Self {
            message: Channel::new(),
            slots: core::array::from_fn(|_| Slot {
                seq: AtomicU8::new(0),
                resp: Signal::new(),
            }),
            free,
            topics: Channel::new(),
            next_seq: AtomicU8::new(1),
            capabilities: DeviceCapabilities::default(),
        }
    }

    /// Receive the next topic push. Topics the driver did not recognize were
    /// already skipped when they arrived.
    ///
    /// If the link dies this never finishes on its own; the surrounding
    /// `select` must cancel it (see the module docs).
    pub async fn next_topic(&self) -> TopicEvent {
        self.topics.receive().await
    }

    /// One typed request/response round trip for endpoint `E`.
    ///
    /// No built-in timeout (this crate has no async runtime): a silent peer
    /// or a dead link keeps this pending until the surrounding `select`
    /// cancels it; callers that need a deadline use their runtime's timeout.
    /// Dropping the call is safe: [`SlotGuard`] frees the slot and the late
    /// reply is dropped as unmatched.
    pub(crate) async fn request<E: Endpoint>(&self, req: &E::Request) -> Result<E::Response, RynkHostError> {
        let cmd = E::CMD;
        let idx = self.free.receive().await;
        let _guard = SlotGuard { client: self, idx };
        let slot = &self.slots[idx];
        let seq = self.alloc_seq();
        // Claim the SEQ before the frame can reach the wire, so the reply
        // always finds its slot; `reset` clears anything a previous request
        // left behind.
        slot.resp.reset();
        slot.seq.store(seq, Ordering::Release);
        self.send_frame(cmd, seq, req).await?;
        let bytes = slot.resp.wait().await;
        // The Deframer never yields a frame shorter than the header, so this
        // slice cannot panic.
        let header = RynkHeader::parse(bytes[..RYNK_HEADER_SIZE].try_into().unwrap());
        if header.cmd != cmd {
            return Err(RynkHostError::CmdMismatch {
                sent: cmd,
                got: header.cmd,
            });
        }
        // Trailing bytes mean the firmware sent a different type than we
        // expect; reject instead of silently accepting a prefix.
        match postcard::take_from_bytes::<Result<E::Response, RynkError>>(&bytes[RYNK_HEADER_SIZE..]) {
            Err(source) => Err(RynkHostError::Deserialize { cmd, source }),
            Ok((_, rest)) if !rest.is_empty() => Err(RynkHostError::TrailingBytes { cmd }),
            Ok((env, _)) => env.map_err(RynkHostError::Rejected),
        }
    }

    /// Send one request without waiting for a reply — for commands whose
    /// effect prevents one (reboot, bootloader jump). `Ok` only means the
    /// frame is queued for the writer; keep the driver running long enough to
    /// write it out. If the device does not actually reset, its reply matches
    /// no slot and is dropped.
    pub(crate) async fn send_no_reply<E: Endpoint>(&self, req: &E::Request) -> Result<(), RynkHostError> {
        self.send_frame(E::CMD, self.alloc_seq(), req).await
    }

    /// Return the next request SEQ, cycling through `1..=255`. 0 marks a free
    /// slot, so it is never handed out.
    fn alloc_seq(&self) -> u8 {
        // `fetch_update` cannot fail because the closure always returns `Some`.
        let (Ok(seq) | Err(seq)) = self.next_seq.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |s| {
            Some(if s == u8::MAX { 1 } else { s + 1 })
        });
        seq
    }

    /// Encode one request into an owned frame and queue it for the writer.
    async fn send_frame<Req: Serialize>(&self, cmd: Cmd, seq: u8, req: &Req) -> Result<(), RynkHostError> {
        // The buffer fits the largest allowed request in COBS-encoded form,
        // so an oversized request fails to encode and never reaches the link.
        let limit = max_wire_size(RYNK_HEADER_SIZE + self.capabilities.max_payload_size as usize);
        #[cfg(feature = "alloc")]
        let mut buf: FrameBytes = vec![0; limit];
        #[cfg(not(feature = "alloc"))]
        let mut buf: FrameBytes = {
            let mut b = FrameBytes::new();
            // The `min` keeps the resize within capacity, so it cannot fail;
            // an oversized request fails at `encode_frame` below instead.
            let _ = b.resize(limit.min(b.capacity()), 0);
            b
        };
        let frame_len = encode_frame(&mut buf, RynkHeader { cmd, seq }, req).map_err(|_| RynkHostError::Encode(cmd))?;
        buf.truncate(frame_len);
        self.message.send(buf).await;
        Ok(())
    }
}

/// Receive buffer size on alloc builds — the largest frame that can be
/// received. If a frame overflows it, or a flood of bytes never contains a
/// `0x00` delimiter, the [`Deframer`] throws the data away and resyncs at
/// the next delimiter.
#[cfg(feature = "alloc")]
const RX_BUFFER_SIZE: usize = 128 * 1024;

/// Moves the bytes for one session. Owns the transport's read and write
/// halves plus the receive state: incoming bytes land in the tail of `buf`,
/// and the [`Deframer`] cuts whole COBS frames back out of it in place.
/// No-alloc builds size `buf` to the firmware's frame buffer.
pub struct Driver<R: Read, W: Write> {
    reader: R,
    writer: W,
    #[cfg(feature = "alloc")]
    buf: Vec<u8>,
    #[cfg(not(feature = "alloc"))]
    buf: [u8; rmk_types::constants::RYNK_BUFFER_SIZE],
    df: Deframer,
}

impl<R: Read, W: Write> Driver<R, W> {
    pub(crate) fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            #[cfg(feature = "alloc")]
            buf: vec![0; RX_BUFFER_SIZE],
            #[cfg(not(feature = "alloc"))]
            buf: [0; rmk_types::constants::RYNK_BUFFER_SIZE],
            df: Deframer::new(),
        }
    }

    /// Move bytes in both directions until the link dies, then return the
    /// error that ended it.
    ///
    /// Takes `&mut self` so it can be called again after being cancelled: the
    /// receive state lives in the struct, so a cancelled run (a `select`
    /// exit, wasm's call-at-a-time pumping) loses no bytes.
    pub async fn run(&mut self, client: &Client) -> RynkHostError {
        let Self {
            reader,
            writer,
            buf,
            df,
        } = self;

        let rx_loop = async {
            loop {
                // No await between `read` and `commit`, so cancelling `run`
                // cannot lose bytes that were already read.
                let n = match reader.read(df.tail(buf)).await {
                    Ok(0) => break RynkHostError::Disconnected,
                    Ok(n) => n,
                    Err(e) => break RynkHostError::Io(e.kind()),
                };
                df.commit(n);
                // Cut out every complete frame this read finished. The
                // Deframer decodes in place and skips past any garbage on its
                // own.
                while let Some(len) = df.next(buf) {
                    let frame = &buf[..len];
                    let header = RynkHeader::parse(frame[..RYNK_HEADER_SIZE].try_into().unwrap());
                    if header.cmd.is_topic() {
                        let Some(event) = TopicEvent::decode(header.cmd, &frame[RYNK_HEADER_SIZE..]) else {
                            log::debug!("rynk: unknown topic {:?}, skipped", header.cmd);
                            continue;
                        };
                        if let Err(TrySendError::Full(event)) = client.topics.try_send(event) {
                            // The read loop must never block; drop the oldest
                            // topic instead.
                            let _ = client.topics.try_receive();
                            log::debug!("rynk: topic queue full, dropped oldest");
                            let _ = client.topics.try_send(event);
                        }
                    } else {
                        #[cfg(feature = "alloc")]
                        let bytes = FrameBytes::from(frame);
                        // A decoded frame is never larger than the receive
                        // buffer, so this always fits.
                        #[cfg(not(feature = "alloc"))]
                        let bytes = FrameBytes::try_from(frame).unwrap();
                        // Deliver the reply to the slot waiting for this SEQ.
                        // If no slot waits, it is a cancelled request's late
                        // reply or a fire-and-forget command's echo — drop it.
                        // SEQ 0 would match a free slot, so those frames are
                        // dropped outright.
                        let waiter = if header.seq == 0 {
                            None
                        } else {
                            client
                                .slots
                                .iter()
                                .find(|s| s.seq.load(Ordering::Acquire) == header.seq)
                        };
                        match waiter {
                            Some(slot) => slot.resp.signal(bytes),
                            None => log::debug!("rynk: unmatched reply {:?} seq {}, dropped", header.cmd, header.seq),
                        }
                    }
                }
            }
        };

        let tx_loop = async {
            // Send a lone `0x00` first so stale bytes in the peer's receive
            // buffer (an OS port probe, a prior session's half-frame) end as
            // garbage instead of merging with our first request.
            if let Err(e) = writer.write_all(&[0]).await {
                return RynkHostError::Io(e.kind());
            }
            loop {
                let frame = client.message.receive().await;
                if let Err(e) = writer.write_all(&frame).await {
                    break RynkHostError::Io(e.kind());
                }
            }
        };

        match select(tx_loop, rx_loop).await {
            Either::First(e) | Either::Second(e) => e,
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
