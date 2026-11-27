//! Combo handlers.

use rmk_types::combo::{Combo as ComboConfig, ComboDefinition};
use rmk_types::protocol::rynk::command::{
    GetCombo, GetComboBulk, GetComboDefinition, GetComboDefinitionBulk, SetCombo, SetComboBulk, SetComboDefinition,
    SetComboDefinitionBulk,
};
use rmk_types::protocol::rynk::{
    GetComboBulkRequest, RynkError, RynkMessage, SetComboDefinitionRequest, SetComboRequest, bulk_item_capacity,
};

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
            match combos[idx as usize].as_ref() {
                Some(combo) => combo.legacy_config().cloned().ok_or(RynkError::Invalid),
                None => Ok(ComboConfig::empty()),
            }
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
            if page
                .clone()
                .any(|i| combos[i].as_ref().is_some_and(|combo| combo.legacy_config().is_none()))
            {
                return Err(RynkError::Invalid);
            }
            msg.encode_bulk(page.map(|i| {
                combos[i]
                    .as_ref()
                    .and_then(|c| c.legacy_config().cloned())
                    .unwrap_or_else(ComboConfig::empty)
            }))
        })
    }
}

impl Handle<GetComboDefinition> for RynkService<'_> {
    async fn handle(&self, idx: u8) -> Result<ComboDefinition, RynkError> {
        self.ctx.with_combos(|combos| {
            if (idx as usize) >= combos.len() {
                return Err(RynkError::Invalid);
            }
            Ok(combos[idx as usize]
                .as_ref()
                .map(|combo| combo.definition())
                .unwrap_or_else(ComboDefinition::empty))
        })
    }
}

impl Handle<SetComboDefinition> for RynkService<'_> {
    async fn handle(&self, request: SetComboDefinitionRequest) -> Result<(), RynkError> {
        if self
            .ctx
            .set_combo_definition(request.index, request.definition)
            .await
            .map_err(|_| RynkError::StorageFault)?
        {
            Ok(())
        } else {
            Err(RynkError::Invalid)
        }
    }
}

impl HandleBulk<GetComboDefinitionBulk> for RynkService<'_> {
    async fn handle_bulk(&self, msg: &mut RynkMessage<'_>) -> Result<(), RynkError> {
        let req = msg.decode_request::<GetComboBulkRequest>()?;
        let cap = bulk_item_capacity(msg.capacity());
        self.ctx.with_combos(|combos| {
            let page = bulk_page(req.start_index as usize, cap, combos.len())?;
            msg.encode_bulk(page.map(|i| {
                combos[i]
                    .as_ref()
                    .map(|combo| combo.definition())
                    .unwrap_or_else(ComboDefinition::empty)
            }))
        })
    }
}

impl HandleBulk<SetComboDefinitionBulk> for RynkService<'_> {
    async fn handle_bulk(&self, msg: &mut RynkMessage<'_>) -> Result<(), RynkError> {
        let mut cursor = msg.payload();
        let start_index = take_element::<u8>(&mut cursor)? as usize;
        let definitions_payload = cursor;
        let num_combos = self.ctx.with_combos(|combos| combos.len());

        for (_, definition) in take_bulk::<ComboDefinition>(&mut cursor, start_index, num_combos)? {
            if !self.ctx.combo_definition_is_valid(&definition) {
                return Err(RynkError::Invalid);
            }
        }

        let mut cursor = definitions_payload;
        for (idx, definition) in take_bulk::<ComboDefinition>(&mut cursor, start_index, num_combos)? {
            if !self
                .ctx
                .set_combo_definition(idx as u8, definition)
                .await
                .map_err(|_| RynkError::StorageFault)?
            {
                return Err(RynkError::Invalid);
            }
        }
        msg.encode_response(&())
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
