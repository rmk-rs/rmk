/// Rynk lock-gate configuration, emitted by the macro from `[host]` in
/// keyboard.toml. `unlock_keys` empty ⇒ dangerous ops are permanently locked.
#[derive(Clone, Copy, Debug)]
pub struct LockConfig {
    /// Physical `(row, col)` keys held simultaneously to unlock. Empty ⇒ no
    /// unlock possible.
    pub unlock_keys: &'static [(u8, u8)],
    /// Start (and stay) unlocked — development escape hatch.
    pub insecure: bool,
    /// Move the config-write tier (`SetKeyAction`, `SetMacro`, …) into the
    /// locked set.
    pub write_requires_unlock: bool,
    /// Gate central and split-peripheral bootloader entry behind the physical
    /// unlock challenge. Defaults to true; boards with a trusted host can opt
    /// out without disabling the gate for storage reset or matrix reads.
    pub bootloader_requires_unlock: bool,
}

impl Default for LockConfig {
    fn default() -> Self {
        Self {
            unlock_keys: &[],
            insecure: false,
            write_requires_unlock: false,
            bootloader_requires_unlock: true,
        }
    }
}
