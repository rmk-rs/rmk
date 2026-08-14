//! Behavior-config handlers (combo timeout, one-shot timeout, tap intervals,
//! default morse profile, flow-tap window).

use rmk_types::protocol::rynk::command::{GetBehaviorConfig, SetBehaviorConfig};
use rmk_types::protocol::rynk::{BehaviorConfig, RynkError};

use super::super::RynkService;
use super::Handle;

impl Handle<GetBehaviorConfig> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<BehaviorConfig, RynkError> {
        Ok(BehaviorConfig {
            combo_timeout_ms: self.ctx.combo_timeout().as_millis() as u16,
            oneshot_timeout_ms: self.ctx.one_shot_timeout().as_millis() as u16,
            tap_interval_ms: self.ctx.tap_interval(),
            tap_capslock_interval_ms: self.ctx.tap_capslock_interval(),
            morse_default_profile: self.ctx.morse_default_profile(),
            morse_prior_idle_time_ms: self.ctx.morse_prior_idle_time().as_millis() as u16,
        })
    }
}

impl Handle<SetBehaviorConfig> for RynkService<'_> {
    async fn handle(&self, cfg: BehaviorConfig) -> Result<(), RynkError> {
        self.ctx.set_combo_timeout(cfg.combo_timeout_ms).await;
        self.ctx.set_one_shot_timeout(cfg.oneshot_timeout_ms).await;
        self.ctx.set_tap_interval(cfg.tap_interval_ms).await;
        self.ctx.set_tap_capslock_interval(cfg.tap_capslock_interval_ms).await;
        self.ctx.set_morse_default_profile(cfg.morse_default_profile).await;
        self.ctx.set_morse_prior_idle_time(cfg.morse_prior_idle_time_ms).await;
        Ok(())
    }
}
