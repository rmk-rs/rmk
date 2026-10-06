//! Combo handlers.

use rmk_types::combo::Combo as ComboConfig;
use rmk_types::protocol::rynk::command::{GetCombo, GetComboBulk, SetCombo, SetComboBulk};
use rmk_types::protocol::rynk::{GetComboBulkRequest, RynkError, RynkMessage, SetComboRequest, bulk_item_capacity};

use super::super::RynkService;
use super::bulk::{bulk_page, take_bulk, take_element};
use super::{Handle, HandleBulk};

impl Handle<GetCombo> for RynkService<'_> {
    async fn handle(&self, idx: u8) -> Result<ComboConfig, RynkError> {
        // Empty in-range slots return the empty config; OOR is an error.
        self.ctx.with_combos(|combos| {
            if (idx as usize) >= combos.len() {
                return Err(RynkError::Invalid);
            }
            Ok(combos[idx as usize]
                .as_ref()
                .map(|c| c.config.clone())
                .unwrap_or_else(ComboConfig::empty))
        })
    }
}

impl Handle<SetCombo> for RynkService<'_> {
    async fn handle(&self, r: SetComboRequest) -> Result<(), RynkError> {
        match self.ctx.set_combo(r.index, r.config).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(RynkError::Invalid),
            Err(()) => Err(RynkError::StorageFault),
        }
    }
}

impl HandleBulk<GetComboBulk> for RynkService<'_> {
    async fn handle_bulk(&self, msg: &mut RynkMessage<'_>) -> Result<(), RynkError> {
        let req = msg.decode_request::<GetComboBulkRequest>()?;
        let cap = bulk_item_capacity(msg.capacity());
        // Empty slots read back as the empty config, same as the single Get; an
        // out-of-range `start_index` yields an empty page.
        self.ctx.with_combos(|combos| {
            let page = bulk_page(req.start_index as usize, cap, combos.len())?;
            msg.encode_bulk(page.map(|i| {
                combos[i]
                    .as_ref()
                    .map(|c| c.config.clone())
                    .unwrap_or_else(ComboConfig::empty)
            }))
        })
    }
}

impl HandleBulk<SetComboBulk> for RynkService<'_> {
    async fn handle_bulk(&self, msg: &mut RynkMessage<'_>) -> Result<(), RynkError> {
        let mut cursor = msg.payload();
        let start_index = take_element::<u8>(&mut cursor)? as usize;
        let num_combos = self.ctx.with_combos(|combos| combos.len());
        for (idx, config) in take_bulk::<ComboConfig>(&mut cursor, start_index, num_combos)? {
            self.ctx
                .set_combo(idx as u8, config)
                .await
                .or(Err(RynkError::StorageFault))?;
        }
        msg.encode_response(&())
    }
}
