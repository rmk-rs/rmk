//! Keymap and encoder handlers (encoder is part of keymap's `0x01xx` Cmd group).

use rmk_types::action::{EncoderAction, KeyAction};
use rmk_types::protocol::rynk::command::{
    GetDefaultLayer, GetEncoderAction, GetKeyAction, GetKeymapBulk, SetDefaultLayer, SetEncoderAction, SetKeyAction,
    SetKeymapBulk,
};
use rmk_types::protocol::rynk::{
    GetEncoderRequest, GetKeymapBulkRequest, KeyPosition, RynkError, RynkMessage, SetEncoderRequest, SetKeyRequest,
    bulk_key_capacity,
};

use super::super::RynkService;
use super::bulk::{bulk_page, take_bulk, take_element};
use super::{Handle, HandleBulk};

impl Handle<GetKeyAction> for RynkService<'_> {
    async fn handle(&self, pos: KeyPosition) -> Result<KeyAction, RynkError> {
        self.check_key_position(&pos)?;
        Ok(self.ctx.get_action(pos.layer, pos.row, pos.col))
    }
}

impl Handle<SetKeyAction> for RynkService<'_> {
    async fn handle(&self, set: SetKeyRequest) -> Result<(), RynkError> {
        self.check_key_position(&set.position)?;
        self.ctx
            .set_action(set.position.layer, set.position.row, set.position.col, set.action)
            .await
            .or(Err(RynkError::StorageFault))
    }
}

impl Handle<GetDefaultLayer> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<u8, RynkError> {
        Ok(self.ctx.default_layer())
    }
}

impl Handle<SetDefaultLayer> for RynkService<'_> {
    async fn handle(&self, layer: u8) -> Result<(), RynkError> {
        let (_, _, num_layers) = self.ctx.keymap_dimensions();
        if (layer as usize) >= num_layers {
            return Err(RynkError::Invalid);
        }
        self.ctx.set_default_layer(layer).await.or(Err(RynkError::StorageFault))
    }
}

impl Handle<GetEncoderAction> for RynkService<'_> {
    async fn handle(&self, r: GetEncoderRequest) -> Result<EncoderAction, RynkError> {
        self.check_encoder_bounds(r.layer, r.encoder_id)?;
        self.ctx.get_encoder(r.layer, r.encoder_id).ok_or(RynkError::Invalid)
    }
}

impl Handle<SetEncoderAction> for RynkService<'_> {
    async fn handle(&self, r: SetEncoderRequest) -> Result<(), RynkError> {
        self.check_encoder_bounds(r.layer, r.encoder_id)?;
        self.ctx
            .set_encoder(r.layer, r.encoder_id, r.action)
            .await
            .or(Err(RynkError::StorageFault))
    }
}

impl RynkService<'_> {
    /// `Invalid` for a key position outside the live keymap grid. Reads and
    /// writes share these bounds.
    fn check_key_position(&self, pos: &KeyPosition) -> Result<(), RynkError> {
        let (rows, cols, num_layers) = self.ctx.keymap_dimensions();
        if (pos.layer as usize) >= num_layers || (pos.row as usize) >= rows || (pos.col as usize) >= cols {
            return Err(RynkError::Invalid);
        }
        Ok(())
    }

    /// `Invalid` for an out-of-range encoder. Checks `layer` and `encoder_id`
    /// explicitly rather than relying on `get_encoder` returning `None`: the
    /// keymap flat-indexes encoders (`layer * num_encoder + id`), so an
    /// over-range `id` would otherwise alias into another layer's slot.
    fn check_encoder_bounds(&self, layer: u8, encoder_id: u8) -> Result<(), RynkError> {
        let (_, _, num_layers) = self.ctx.keymap_dimensions();
        if (layer as usize) >= num_layers || (encoder_id as usize) >= self.ctx.num_encoders() {
            return Err(RynkError::Invalid);
        }
        Ok(())
    }
}

impl RynkService<'_> {
    /// Validate a bulk keymap start position against the live geometry and
    /// return its flat, row-major, layer-major key index.
    fn keymap_flat_start(&self, layer: u8, start_row: u8, start_col: u8) -> Result<usize, RynkError> {
        self.check_key_position(&KeyPosition {
            layer,
            row: start_row,
            col: start_col,
        })?;
        let (rows, cols, _) = self.ctx.keymap_dimensions();
        Ok((layer as usize * rows + start_row as usize) * cols + start_col as usize)
    }
}

impl HandleBulk<GetKeymapBulk> for RynkService<'_> {
    async fn handle_bulk(&self, msg: &mut RynkMessage<'_>) -> Result<(), RynkError> {
        let req = msg.decode_request::<GetKeymapBulkRequest>()?;
        // From the start key the page reads forward through the flat keymap,
        // crossing row and layer boundaries freely, and stops at the keymap's end.
        let start = self.keymap_flat_start(req.layer, req.start_row, req.start_col)?;
        let (rows, cols, num_layers) = self.ctx.keymap_dimensions();
        let cap = bulk_key_capacity(msg.capacity());
        let page = bulk_page(start, cap, num_layers * rows * cols)?;
        msg.encode_bulk(page.map(|offset| self.ctx.get_action_flat(offset)))
    }
}

impl HandleBulk<SetKeymapBulk> for RynkService<'_> {
    async fn handle_bulk(&self, msg: &mut RynkMessage<'_>) -> Result<(), RynkError> {
        let mut cursor = msg.payload();
        let [layer, start_row, start_col] = take_element::<[u8; 3]>(&mut cursor)?;
        let start = self.keymap_flat_start(layer, start_row, start_col)?;
        let (rows, cols, num_layers) = self.ctx.keymap_dimensions();
        // Bulk order advances columns, then rows, then layers.
        for (offset, action) in take_bulk::<KeyAction>(&mut cursor, start, num_layers * rows * cols)? {
            let layer = (offset / (rows * cols)) as u8;
            let row = (offset / cols % rows) as u8;
            let col = (offset % cols) as u8;
            self.ctx
                .set_action(layer, row, col, action)
                .await
                .or(Err(RynkError::StorageFault))?;
        }
        msg.encode_response(&())
    }
}
