//! Fork handlers.

use rmk_types::fork::Fork;
use rmk_types::protocol::rynk::command::{GetFork, SetFork};
use rmk_types::protocol::rynk::{RynkError, SetForkRequest};

use super::super::RynkService;
use super::Handle;

impl Handle<GetFork> for RynkService<'_> {
    async fn handle(&self, idx: u8) -> Result<Fork, RynkError> {
        self.ctx.get_fork(idx).ok_or(RynkError::Invalid)
    }
}

impl Handle<SetFork> for RynkService<'_> {
    async fn handle(&self, r: SetForkRequest) -> Result<(), RynkError> {
        match self.ctx.set_fork(r.index, r.config).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(RynkError::Invalid),
            Err(()) => Err(RynkError::StorageFault),
        }
    }
}
