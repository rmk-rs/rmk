//! Morse handlers.

use rmk_types::constants::MORSE_PROFILE_MAX_NUM;
use rmk_types::morse::Morse;
use rmk_types::protocol::rynk::command::{
    GetMorse, GetMorseBulk, GetMorseHoldTriggerPositions, SetMorse, SetMorseBulk, SetMorseHoldTriggerPositions,
};
use rmk_types::protocol::rynk::{
    GetMorseBulkRequest, MorseHoldTriggerPositionState, RynkError, RynkMessage, SetMorseHoldTriggerPositionsRequest,
    SetMorseRequest, bulk_item_capacity,
};

use super::super::RynkService;
use super::bulk::{bulk_page, take_bulk, take_element};
use super::{Handle, HandleBulk};

impl Handle<GetMorse> for RynkService<'_> {
    async fn handle(&self, idx: u8) -> Result<Morse, RynkError> {
        self.ctx.get_morse(idx).ok_or(RynkError::Invalid)
    }
}

impl Handle<SetMorse> for RynkService<'_> {
    async fn handle(&self, r: SetMorseRequest) -> Result<(), RynkError> {
        if (r.index as usize) >= self.ctx.morses_len() {
            return Err(RynkError::Invalid);
        }
        self.ctx
            .update_morse(r.index, |m| {
                *m = r.config;
            })
            .await;
        Ok(())
    }
}

impl HandleBulk<GetMorseBulk> for RynkService<'_> {
    async fn handle_bulk(&self, msg: &mut RynkMessage<'_>) -> Result<(), RynkError> {
        let req = msg.decode_request::<GetMorseBulkRequest>()?;
        let cap = bulk_item_capacity(msg.capacity());
        let page = bulk_page(req.start_index as usize, cap, self.ctx.morses_len())?;
        msg.encode_bulk(page.map(|idx| self.ctx.get_morse(idx as u8).unwrap_or_default()))
    }
}

impl HandleBulk<SetMorseBulk> for RynkService<'_> {
    async fn handle_bulk(&self, msg: &mut RynkMessage<'_>) -> Result<(), RynkError> {
        let mut cursor = msg.payload();
        let start_index = take_element::<u8>(&mut cursor)? as usize;
        for (idx, config) in take_bulk::<Morse>(&mut cursor, start_index, self.ctx.morses_len())? {
            self.ctx.update_morse(idx as u8, |m| *m = config).await;
        }
        msg.encode_response(&())
    }
}

impl Handle<GetMorseHoldTriggerPositions> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<MorseHoldTriggerPositionState, RynkError> {
        Ok(self.ctx.morse_hold_trigger_positions())
    }
}

impl Handle<SetMorseHoldTriggerPositions> for RynkService<'_> {
    async fn handle(&self, request: SetMorseHoldTriggerPositionsRequest) -> Result<(), RynkError> {
        let (rows, cols, _) = self.ctx.keymap_dimensions();
        if request.positions.iter().any(|position| {
            (position.profile != u8::MAX && position.profile as usize >= MORSE_PROFILE_MAX_NUM)
                || position.row as usize >= rows
                || position.col as usize >= cols
        }) {
            return Err(RynkError::Invalid);
        }
        self.ctx.set_morse_hold_trigger_positions(request).await;
        Ok(())
    }
}
