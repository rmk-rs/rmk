//! Morse handlers.

use rmk_types::morse::Morse;
use rmk_types::protocol::rynk::command::{GetMorse, GetMorseBulk, SetMorse, SetMorseBulk};
use rmk_types::protocol::rynk::{GetMorseBulkRequest, RynkError, RynkMessage, SetMorseRequest, bulk_item_capacity};

use super::bulk::{bulk_page, take_bulk, take_element};
use super::{Handle, HandleBulk};
use crate::host::context::KeyboardContext;

impl Handle<GetMorse> for KeyboardContext<'_> {
    async fn handle(&self, idx: u8) -> Result<Morse, RynkError> {
        self.get_morse(idx).ok_or(RynkError::Invalid)
    }
}

impl Handle<SetMorse> for KeyboardContext<'_> {
    async fn handle(&self, r: SetMorseRequest) -> Result<(), RynkError> {
        if (r.index as usize) >= self.morses_len() {
            return Err(RynkError::Invalid);
        }
        self.update_morse(r.index, |m| *m = r.config)
            .await
            .or(Err(RynkError::StorageFault))
    }
}

impl HandleBulk<GetMorseBulk> for KeyboardContext<'_> {
    async fn handle_bulk(&self, msg: &mut RynkMessage<'_>) -> Result<(), RynkError> {
        let req = msg.decode_request::<GetMorseBulkRequest>()?;
        let cap = bulk_item_capacity(msg.capacity());
        let page = bulk_page(req.start_index as usize, cap, self.morses_len())?;
        msg.encode_bulk(page.map(|idx| self.get_morse(idx as u8).unwrap_or_default()))
    }
}

impl HandleBulk<SetMorseBulk> for KeyboardContext<'_> {
    async fn handle_bulk(&self, msg: &mut RynkMessage<'_>) -> Result<(), RynkError> {
        let mut cursor = msg.payload();
        let start_index = take_element::<u8>(&mut cursor)? as usize;
        for (idx, config) in take_bulk::<Morse>(&mut cursor, start_index, self.morses_len())? {
            self.update_morse(idx as u8, |m| *m = config)
                .await
                .or(Err(RynkError::StorageFault))?;
        }
        msg.encode_response(&())
    }
}
