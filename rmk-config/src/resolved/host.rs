use crate::validate_unlock_keys;

/// Resolved host-tool configuration.
pub struct Host {
    pub vial_enabled: bool,
    pub vial_macro_count: u8,
    pub rynk_enabled: bool,
    pub unlock_keys: Vec<[u8; 2]>,
    pub insecure: bool,
    pub write_requires_unlock: bool,
}

impl crate::KeyboardTomlConfig {
    /// Resolve host-tool configuration from TOML config.
    pub fn host(&self) -> Host {
        let host_toml = self.get_host_config();
        let unlock_keys = host_toml.unlock_keys.unwrap_or_default();

        validate_unlock_keys("[host]", &unlock_keys, self.layout.as_ref())
            .unwrap_or_else(|err| panic!("❌ Parse `keyboard.toml` error: {err}"));

        Host {
            vial_enabled: host_toml.vial_enabled,
            vial_macro_count: host_toml.vial_macro_count,
            rynk_enabled: host_toml.rynk_enabled,
            unlock_keys,
            insecure: host_toml.insecure,
            write_requires_unlock: host_toml.write_requires_unlock,
        }
    }
}
