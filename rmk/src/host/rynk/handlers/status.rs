//! Status handlers — current layer, matrix bitmap, battery, peripheral status,
//! plus the live getters for WPM / sleep / LED. Each value is read from its
//! producer-owned current-value accessor, not a host-side cache.

#[cfg(feature = "_ble")]
use rmk_types::battery::BatteryStatus;
use rmk_types::led_indicator::LedIndicator;
use rmk_types::modifier::ModifierCombination;
#[cfg(feature = "split")]
use rmk_types::protocol::rynk::PeripheralStatus;
#[cfg(feature = "_ble")]
use rmk_types::protocol::rynk::command::GetBatteryStatus;
#[cfg(feature = "split")]
use rmk_types::protocol::rynk::command::GetPeripheralStatus;
use rmk_types::protocol::rynk::command::{
    GetCurrentLayer, GetLayerState, GetLedIndicator, GetMatrixState, GetModifierState, GetSleepState, GetWpm,
};
use rmk_types::protocol::rynk::{LayerState, MATRIX_BITMAP_SIZE, MatrixState, RynkError};

use super::super::RynkService;
use super::Handle;

impl Handle<GetCurrentLayer> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<u8, RynkError> {
        Ok(self.ctx.active_layer())
    }
}

impl Handle<GetLayerState> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<LayerState, RynkError> {
        Ok(self.ctx.layer_state())
    }
}

impl Handle<GetModifierState> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<ModifierCombination, RynkError> {
        Ok(crate::state::current_modifier_state())
    }
}

impl Handle<GetMatrixState> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<MatrixState, RynkError> {
        // Sized for the maximum supported matrix — host slices it down
        // using num_rows / num_cols from `DeviceCapabilities`.
        let mut bitmap: heapless::Vec<u8, MATRIX_BITMAP_SIZE> = heapless::Vec::new();
        bitmap.resize_default(MATRIX_BITMAP_SIZE).expect("bitmap size matches");

        // The dispatch gate prevents locked matrix reads.
        self.ctx.read_matrix_state(&mut bitmap);

        Ok(MatrixState { pressed_bitmap: bitmap })
    }
}

#[cfg(feature = "_ble")]
impl Handle<GetBatteryStatus> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<BatteryStatus, RynkError> {
        Ok(self.ctx.battery_status())
    }
}

/// `Cmd::GetPeripheralStatus` — payload is a peripheral slot id. The
/// snapshot is owned by the split driver layer
/// ([`current_peripheral_status`](crate::split::driver::current_peripheral_status)),
/// fed at the `PeripheralConnectedEvent` / `PeripheralBatteryEvent` publish sites.
#[cfg(feature = "split")]
impl Handle<GetPeripheralStatus> for RynkService<'_> {
    async fn handle(&self, id: u8) -> Result<PeripheralStatus, RynkError> {
        crate::split::driver::current_peripheral_status(id as usize).ok_or(RynkError::Invalid)
    }
}

impl Handle<GetWpm> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<u16, RynkError> {
        Ok(crate::processor::builtin::wpm::current_wpm())
    }
}

impl Handle<GetSleepState> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<bool, RynkError> {
        Ok(crate::state::current_sleep_state())
    }
}

impl Handle<GetLedIndicator> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<LedIndicator, RynkError> {
        Ok(self.ctx.led_indicator())
    }
}
