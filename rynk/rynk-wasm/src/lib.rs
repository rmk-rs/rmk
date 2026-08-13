//! Wasm package for the Rynk host client.
//!
//! JS owns the WebSerial/WebHID link, probes the protocol version, then hands the
//! open byte link to [`client::connect`]. Native targets compile an empty crate.
//! Build with `wasm-pack build --target web`.
#![cfg(target_arch = "wasm32")]

mod catalog;
mod client;
mod transport;
#[cfg(feature = "vial")]
mod vial;

use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Debug);
}
