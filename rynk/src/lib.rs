//! Runtime-free Rynk host protocol client.
//!
//! A session is a [`Client`] plus a [`Driver`], created by
//! [`RynkDevice::connect`] — the one entry point, implemented per transport
//! over a byte link's read/write halves. The [`Client`] carries the
//! protocol surface — typed requests and the topic stream, both `&self` so
//! they run full-duplex; the [`Driver`] pumps bytes both ways and returns
//! when the link dies. This crate does not depend on an async runtime, and
//! without default features it is `no_std` and allocation-free (frames live
//! in buffers sized by the firmware's `rynk_buffer_size`).
//!
//! ## Driving a session
//!
//! Run [`Driver::run`] in the same `select` as everything that awaits on the
//! [`Client`]. There is no in-band death signal: when the driver returns, the
//! `select` exits and drops the session, cancelling any parked typed request
//! or [`next_topic`](Client::next_topic) call.
//!
//! ```no_run
//! # async fn run<D: rynk::RynkDevice>(device: D) -> Result<(), Box<dyn std::error::Error>> {
//! use embassy_futures::select::select3;
//!
//! let (client, mut driver) = device.connect().await?;
//! select3(
//!     driver.run(&client),                     // returns when the link dies
//!     async {
//!         loop {
//!             let event = client.next_topic().await;
//!             println!("topic: {event:?}");
//!         }
//!     },
//!     async {
//!         let layer = client.get_current_layer().await?;
//!         println!("active layer: {layer}");
//!         Ok::<_, rynk::RynkHostError>(())
//!     },
//! )
//! .await;
//! # Ok(()) }
//! ```
//!
//! Two more topologies build on the same API:
//!
//! - **Spawned driver**: put the [`Client`] in a `StaticCell`/`Box::leak` for
//!   `&'static`, spawn `driver.run(client)` as its own task, and have the main
//!   loop watch that task's completion alongside its own awaits — without the
//!   watch, a parked call would outlive the link silently.
//! - **Driver-lock select** (wasm): with no resident task, each in-flight call
//!   races its client future against locking the driver, and the lock winner
//!   pumps for every parked call — see `rynk-wasm`'s `RynkClient` for the
//!   mechanism.
//!
//! Requests can run concurrently — the [`Client`] has a slot pool and matches
//! replies back by SEQ, so callers past the pool wait for a slot instead of
//! flooding the device — but only one task should consume topics, since the
//! channels behind the [`Client`] are competitively consumed.
//!
//! Overlapping a `read_all_*` pager with a bulk write is correct but slow: the
//! pending write crowds the firmware's frame buffer, its replies page short,
//! and the pager spends extra round trips covering what they left out.
//!
//! ## Transport contract
//!
//! A [`RynkDevice`]'s `open()` hands out the link's read/write halves,
//! implementing the [`io`] re-export's traits so the trait version always
//! matches this crate. The pump relies on:
//!
//! - **Writes deliver without flush** — the driver never calls
//!   [`flush`](io::Write::flush). A successful `write` MUST commit the returned
//!   bytes; on a lossy medium (e.g. BLE) use acknowledged writes, since a lost
//!   chunk desyncs the firmware's reassembler with no mid-frame resync.
//! - `read` may return arbitrary chunk boundaries; the driver reassembles
//!   frames. `Ok(0)` means the link is gone and surfaces as
//!   [`RynkHostError::Disconnected`] from [`Driver::run`].
//! - Reads must be cancel-safe: a read cancelled before completion consumes
//!   nothing (the session `select`, and the wasm per-call pattern, cancel the
//!   pump freely).
//! - Writes must be cancel-safe too: a cancelled `write` whose bytes are still
//!   going out must finish before the next one starts, or the two interleave
//!   into a frame the firmware can only discard.
//!
//! ## Multi-version dispatch
//!
//! [`RynkDevice::connect`] rejects only a protocol **major** mismatch. To
//! support several majors at once, link one `rynk` build per major (cargo
//! `package` renames) and probe with the newest first; the probe
//! (`GetVersion`) is frozen across all majors by the protocol ICD,
//! [`rmk_types::protocol::rynk`].

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod api;
mod device;
mod driver;
#[cfg(feature = "alloc")]
pub mod layout;
#[cfg(feature = "vial")]
pub mod vial;

pub use device::RynkDevice;
pub use driver::{Client, Driver, RynkHostError};
pub use embedded_io_async as io;
#[cfg(feature = "alloc")]
pub use layout::LayoutInfo;
pub use rmk_types;
/// The decoded topic union returned by [`Client::next_topic`]
pub use rmk_types::protocol::rynk::TopicEvent;
