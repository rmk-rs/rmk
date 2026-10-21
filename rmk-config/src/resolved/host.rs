use crate::{SplitBoardConfig, validate_unlock_keys};

/// Resolved host-tool configuration.
pub struct Host {
    pub vial_enabled: bool,
    pub rynk_enabled: bool,
    pub unlock_keys: Vec<[u8; 2]>,
    pub insecure: bool,
    pub write_requires_unlock: bool,
}

impl crate::KeyboardTomlConfig {
    /// Resolve host-tool configuration from TOML config.
    pub fn host(&self) -> Host {
        let host_toml = self.host.clone().unwrap_or_default();
        let unlock_keys = host_toml.unlock_keys.unwrap_or_default();

        validate_unlock_keys("[host]", &unlock_keys, self.layout.as_ref())
            .unwrap_or_else(|err| panic!("❌ Parse `keyboard.toml` error: {err}"));

        let mut host = Host {
            vial_enabled: host_toml.vial_enabled,
            rynk_enabled: host_toml.rynk_enabled,
            unlock_keys,
            insecure: host_toml.insecure,
            write_requires_unlock: host_toml.write_requires_unlock,
        };
        if let Some(split) = &self.split
            && split.central.host.is_some()
        {
            host.apply_side(&split.central, "[split.central.host]")
                .unwrap_or_else(|err| panic!("keyboard.toml: {err}"));
        }
        host
    }

    /// Resolve the explicitly enabled USB update service of one peripheral.
    pub fn peripheral_host(&self, id: usize) -> Result<Option<Host>, String> {
        let board = self
            .split
            .as_ref()
            .and_then(|split| split.peripheral.get(id))
            .ok_or_else(|| format!("Missing split peripheral {id}"))?;
        if board.host.is_none() {
            return Ok(None);
        }
        let mut host = self.host();
        if host.vial_enabled == host.rynk_enabled {
            return Err("[split.peripheral.host] requires exactly one of Vial or Rynk".into());
        }
        host.apply_side(board, "[split.peripheral.host]")?;
        Ok(Some(host))
    }
}

impl Host {
    fn apply_side(&mut self, board: &SplitBoardConfig, section: &str) -> Result<(), String> {
        let config = board.host.as_ref().expect("side host config");
        validate_unlock_keys(section, &config.unlock_keys, None)?;
        if !config.insecure && config.unlock_keys.is_empty() {
            return Err(format!(
                "{section}.unlock_keys must not be empty unless insecure = true"
            ));
        }
        for [row, col] in &config.unlock_keys {
            if usize::from(*row) >= board.rows || usize::from(*col) >= board.cols {
                return Err(format!(
                    "{section}.unlock_keys ({row}, {col}) is outside the local matrix"
                ));
            }
            if matches!(board.matrix.matrix_type, crate::MatrixType::DirectPin)
                && board
                    .matrix
                    .direct_pins
                    .as_ref()
                    .and_then(|pins| pins.get(*row as usize))
                    .and_then(|row| row.get(*col as usize))
                    .is_none_or(|pin| pin == "_" || pin.eq_ignore_ascii_case("trns"))
            {
                return Err(format!("{section}.unlock_keys ({row}, {col}) has no physical pin"));
            }
        }
        // Host tools report positions in the shared keyboard layout.
        self.unlock_keys = config
            .unlock_keys
            .iter()
            .map(|[row, col]| {
                Ok([
                    u8::try_from(usize::from(*row) + board.row_offset)
                        .map_err(|_| format!("{section}: unlock row overflow"))?,
                    u8::try_from(usize::from(*col) + board.col_offset)
                        .map_err(|_| format!("{section}: unlock column overflow"))?,
                ])
            })
            .collect::<Result<_, String>>()?;
        self.insecure = config.insecure;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{KeyboardTomlConfig, SplitHostConfig};

    fn split_config() -> KeyboardTomlConfig {
        toml::from_str(
            r#"
[host]
insecure = true
unlock_keys = [[7, 7]]
[split]
connection = "ble"
[split.central]
rows = 2
cols = 2
row_offset = 1
col_offset = 2
[split.central.matrix]
[split.central.host]
unlock_keys = [[0, 0], [1, 1]]
[[split.peripheral]]
rows = 2
cols = 2
row_offset = 3
col_offset = 4
[split.peripheral.matrix]
[split.peripheral.host]
unlock_keys = [[0, 1], [1, 0]]
"#,
        )
        .unwrap()
    }

    #[test]
    fn split_unlock_keys_and_policy_are_local_to_each_board() {
        let mut config = split_config();
        let central = config.host();
        let peripheral = config.peripheral_host(0).unwrap().unwrap();
        assert_eq!(central.unlock_keys, [[1, 2], [2, 3]]);
        assert_eq!(peripheral.unlock_keys, [[3, 5], [4, 4]]);
        assert!(!central.insecure && !peripheral.insecure);
        config.split.as_mut().unwrap().peripheral[0].host = Some(SplitHostConfig {
            unlock_keys: vec![],
            insecure: true,
        });
        assert!(config.peripheral_host(0).unwrap().unwrap().insecure);
        assert!(!config.host().insecure);

        config.split.as_mut().unwrap().peripheral[0].host = None;
        assert!(config.peripheral_host(0).unwrap().is_none());
        config.split.as_mut().unwrap().central.host = None;
        assert_eq!(config.host().unlock_keys, [[7, 7]]);
        assert!(config.host().insecure);
    }

    #[test]
    fn rejects_unusable_local_unlock_combinations() {
        let mut config = split_config();
        for keys in [vec![], vec![[2, 0]], vec![[0, 2]], vec![[0, 0]; 5]] {
            config.split.as_mut().unwrap().peripheral[0].host = Some(SplitHostConfig {
                unlock_keys: keys,
                insecure: false,
            });
            assert!(config.peripheral_host(0).is_err());
        }
        let board = &mut config.split.as_mut().unwrap().peripheral[0];
        board.host.as_mut().unwrap().unlock_keys = vec![[0, 0]];
        board.matrix.matrix_type = crate::MatrixType::DirectPin;
        board.matrix.direct_pins = Some(vec![vec!["_".into(), "P0_01".into()]]);
        assert!(config.peripheral_host(0).is_err());
        config.split.as_mut().unwrap().peripheral[0]
            .host
            .as_mut()
            .unwrap()
            .unlock_keys = vec![[0, 1]];
        assert!(config.peripheral_host(0).is_ok());
    }
}
