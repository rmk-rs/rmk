//! Sticky profile handlers.

use rmk_types::protocol::rynk::command::{GetStickyProfile, SetStickyProfile};
use rmk_types::protocol::rynk::{RynkError, SetStickyProfileRequest};
use rmk_types::sticky::StickyProfile;

use super::super::RynkService;
use super::Handle;

impl Handle<GetStickyProfile> for RynkService<'_> {
    async fn handle(&self, idx: u8) -> Result<StickyProfile, RynkError> {
        self.ctx.get_sticky_profile(idx).ok_or(RynkError::Invalid)
    }
}

impl Handle<SetStickyProfile> for RynkService<'_> {
    async fn handle(&self, req: SetStickyProfileRequest) -> Result<(), RynkError> {
        match self.ctx.set_sticky_profile(req.index, req.config).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(RynkError::Invalid),
            Err(()) => Err(RynkError::StorageFault),
        }
    }
}
