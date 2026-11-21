//! Behavior-config handlers (combo timeout, one-shot timeout, tap intervals,
//! default morse profile, flow-tap window).

use rmk_types::protocol::rynk::command::{GetBehaviorConfig, SetBehaviorConfig};
use rmk_types::protocol::rynk::{BehaviorConfig, RynkError};

use super::Handle;
use crate::host::context::KeyboardContext;

impl Handle<GetBehaviorConfig> for KeyboardContext<'_> {
    async fn handle(&self, _: ()) -> Result<BehaviorConfig, RynkError> {
        Ok(BehaviorConfig {
            combo_timeout_ms: self.combo_timeout().as_millis() as u16,
            oneshot_timeout_ms: self.one_shot_timeout().as_millis() as u16,
            tap_interval_ms: self.tap_interval(),
            tap_capslock_interval_ms: self.tap_capslock_interval(),
            morse_default_profile: self.morse_default_profile(),
            morse_prior_idle_time_ms: self.morse_prior_idle_time().as_millis() as u16,
        })
    }
}

impl Handle<SetBehaviorConfig> for KeyboardContext<'_> {
    async fn handle(&self, cfg: BehaviorConfig) -> Result<(), RynkError> {
        self.set_behavior_config(cfg).await.or(Err(RynkError::StorageFault))
    }
}
