//! Fork handlers.

use rmk_types::fork::Fork;
use rmk_types::protocol::rynk::command::{GetFork, SetFork};
use rmk_types::protocol::rynk::{RynkError, SetForkRequest};

use super::Handle;
use crate::host::context::KeyboardContext;

impl Handle<GetFork> for KeyboardContext<'_> {
    async fn handle(&self, idx: u8) -> Result<Fork, RynkError> {
        self.get_fork(idx).ok_or(RynkError::Invalid)
    }
}

impl Handle<SetFork> for KeyboardContext<'_> {
    async fn handle(&self, r: SetForkRequest) -> Result<(), RynkError> {
        match self.set_fork(r.index, r.config).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(RynkError::Invalid),
            Err(()) => Err(RynkError::StorageFault),
        }
    }
}
