//! Rynk command handlers.

use rmk_types::protocol::rynk::command::Endpoint;
use rmk_types::protocol::rynk::{RynkError, RynkMessage};

mod behavior;
mod bulk;
mod combo;
mod connection;
mod fork;
mod keymap;
mod layout;
#[cfg(feature = "lighting")]
pub(super) mod lighting;
mod macro_data;
mod morse;
mod status;
mod system;

/// Fixed-size endpoints: a request → response function. [`serve`] adds the
/// decode → handle → encode wire glue.
pub(super) trait Handle<E: Endpoint> {
    async fn handle(&self, req: E::Request) -> Result<E::Response, RynkError>;
}

/// Bulk endpoints stream a page straight through the session buffer, so no `Vec`
/// is ever materialized. Implemented instead of [`Handle`]: a bulk handler needs
/// the message itself to size its page against the remaining reply window.
pub(super) trait HandleBulk<E: Endpoint> {
    async fn handle_bulk(&self, msg: &mut RynkMessage<'_>) -> Result<(), RynkError>;
}

/// Answer `msg` in place from a fixed-size endpoint's handler.
pub(super) async fn serve<E: Endpoint, T: Handle<E>>(h: &T, msg: &mut RynkMessage<'_>) -> Result<(), RynkError> {
    let req = msg.decode_request::<E::Request>()?;
    let resp = h.handle(req).await?;
    msg.encode_response(&resp)
}

/// [`serve`] for a bulk endpoint, whose handler owns the whole decode → encode
/// path. Exists so both endpoint kinds are dispatched by the same call shape.
pub(super) async fn serve_bulk<E: Endpoint, T: HandleBulk<E>>(
    h: &T,
    msg: &mut RynkMessage<'_>,
) -> Result<(), RynkError> {
    h.handle_bulk(msg).await
}
