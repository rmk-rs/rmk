//! Layout handler — serves the opaque blob over the built-in `GetLayout`.

use heapless::Vec;
use rmk_types::protocol::rynk::command::GetLayout;
use rmk_types::protocol::rynk::{LayoutChunk, RYNK_BLE_CHUNK_SIZE, RynkError};

use super::Handle;

impl Handle<GetLayout> for &[u8] {
    async fn handle(&self, offset: u32) -> Result<LayoutChunk, RynkError> {
        let blob = self;
        let total_len = blob.len() as u32;
        let start = (offset as usize).min(blob.len());
        let end = (start + RYNK_BLE_CHUNK_SIZE).min(blob.len());
        let bytes = Vec::from_slice(&blob[start..end]).map_err(|_| RynkError::Internal)?;
        Ok(LayoutChunk { total_len, bytes })
    }
}
