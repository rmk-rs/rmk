//! Macro handlers: one whole macro per request.

use rmk_types::keyboard_macros::Macro;
use rmk_types::protocol::rynk::command::{GetMacro, SetMacro};
use rmk_types::protocol::rynk::{RynkError, SetMacroRequest};

use super::super::RynkService;
use super::Handle;
use crate::MACRO_MAX_NUM;

impl Handle<GetMacro> for RynkService<'_> {
    async fn handle(&self, idx: u8) -> Result<Macro, RynkError> {
        if idx as usize >= MACRO_MAX_NUM {
            return Err(RynkError::Invalid);
        }
        self.ctx
            .keymap
            .macros(|m| Macro::from_bytes(m.slot(idx)))
            .ok_or(RynkError::StorageFault)
    }
}

impl Handle<SetMacro> for RynkService<'_> {
    async fn handle(&self, r: SetMacroRequest) -> Result<(), RynkError> {
        // A macro past the buffer never decodes: `serve` answers `Malformed` first.
        if r.index as usize >= MACRO_MAX_NUM {
            return Err(RynkError::Invalid);
        }
        #[cfg(feature = "storage")]
        {
            let changed = self
                .ctx
                .keymap
                .macros(|m| m.set_slot(r.index, r.macro_ops.as_bytes()))
                .ok_or(RynkError::Invalid)?;
            crate::keyboard::macros::persist(self.ctx.keymap, changed)
                .await
                .map_err(|()| RynkError::StorageFault)
        }
        #[cfg(not(feature = "storage"))]
        {
            Err(RynkError::Unimplemented)
        }
    }
}
