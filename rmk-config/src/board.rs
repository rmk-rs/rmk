use crate::{
    InputDeviceConfig, KeyboardTomlConfig, LayoutTomlConfig, MatrixConfig, MatrixType, SplitConfig, SplitConnection,
};

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum BoardConfig {
    Split(SplitConfig),
    UniBody(UniBodyConfig),
}

#[derive(Clone, Debug, Default)]
pub struct UniBodyConfig {
    pub matrix: MatrixConfig,
    pub input_device: InputDeviceConfig,
}

impl Default for BoardConfig {
    fn default() -> Self {
        BoardConfig::UniBody(UniBodyConfig::default())
    }
}

impl BoardConfig {
    /// Get number of peripherals
    pub fn get_num_peripheral(&self) -> usize {
        match self {
            BoardConfig::Split(split_config) => split_config.peripheral.len(),
            BoardConfig::UniBody(_) => 0,
        }
    }
    /// Get the number of encoders for each board
    ///
    /// - If the board is the unibody board, the returned vector has only one element.
    /// - If the board is the split board, the number of elements is the number of peripherals + 1 (central),
    ///   where the first element is the number of encoders on the central.
    pub fn get_num_encoder(&self) -> Vec<usize> {
        let mut num_encoder = Vec::new();
        match self {
            BoardConfig::Split(split) => {
                // Central's encoders
                num_encoder.push(
                    split
                        .central
                        .input_device
                        .clone()
                        .unwrap_or_default()
                        .encoder
                        .unwrap_or(Vec::new())
                        .len(),
                );

                // Peripheral's encoders
                for peri in &split.peripheral {
                    num_encoder.push(
                        peri.input_device
                            .clone()
                            .unwrap_or_default()
                            .encoder
                            .unwrap_or(Vec::new())
                            .len(),
                    );
                }
            }
            BoardConfig::UniBody(uni_body_config) => {
                num_encoder.push(uni_body_config.input_device.encoder.clone().unwrap_or(Vec::new()).len());
            }
        };
        num_encoder
    }
}

impl KeyboardTomlConfig {
    /// Board-wide encoder count, without resolving the board.
    ///
    /// Split halves declare their own encoders; every other shape — unibody, or a
    /// hardware-less config such as a simulator scenario — uses the top-level
    /// `[input_device]`.
    pub(crate) fn total_encoders(&self) -> usize {
        let count =
            |device: &Option<InputDeviceConfig>| device.as_ref().and_then(|d| d.encoder.as_ref()).map_or(0, Vec::len);
        match &self.split {
            Some(split) => {
                count(&split.central.input_device)
                    + split.peripheral.iter().map(|p| count(&p.input_device)).sum::<usize>()
            }
            None => count(&self.input_device),
        }
    }
}

/// Check a matrix's pin counts against its declared `rows`/`cols`. `row2col` only flips
/// scan direction (In/Out pins), never the pin-list lengths, so there is no swap here.
fn validate_matrix_dims(matrix: &MatrixConfig, rows: usize, cols: usize, ctx: &str) -> Result<(), String> {
    match matrix.matrix_type {
        MatrixType::Normal => {
            let row_pins = matrix.row_pins.as_ref().map_or(0, |v| v.len());
            let col_pins = matrix.col_pins.as_ref().map_or(0, |v| v.len());
            if row_pins != rows {
                return Err(format!(
                    "keyboard.toml: {ctx} has {row_pins} row_pins but rows = {rows}"
                ));
            }
            if col_pins != cols {
                return Err(format!(
                    "keyboard.toml: {ctx} has {col_pins} col_pins but cols = {cols}"
                ));
            }
        }
        MatrixType::DirectPin => {
            let direct = matrix.direct_pins.as_ref();
            let n_rows = direct.map_or(0, |v| v.len());
            if n_rows != rows {
                return Err(format!(
                    "keyboard.toml: {ctx} direct_pins has {n_rows} rows but rows = {rows}"
                ));
            }
            if let Some(direct) = direct {
                for (r, row) in direct.iter().enumerate() {
                    if row.len() != cols {
                        return Err(format!(
                            "keyboard.toml: {ctx} direct_pins row {r} has {} pins but cols = {cols}",
                            row.len()
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Split invariants that per-board dimension checks don't cover: the transport
/// fields must be consistent with `split.connection`, and every board's region
/// of the unified matrix must fit `[layout]` without overlapping another board.
fn validate_split_config(split: &SplitConfig, layout: Option<&LayoutTomlConfig>) -> Result<(), String> {
    let boards = || {
        std::iter::once((&split.central, "[split.central]".to_string())).chain(
            split
                .peripheral
                .iter()
                .enumerate()
                .map(|(i, p)| (p, format!("[[split.peripheral]] #{i}"))),
        )
    };

    match split.connection {
        SplitConnection::Serial => {
            for (board, ctx) in boards() {
                if board.ble_addr.is_some() {
                    return Err(format!(
                        "keyboard.toml: {ctx} sets `ble_addr`, but split.connection = \"serial\""
                    ));
                }
            }
            let central_ports = split.central.serial.as_ref().map_or(0, |s| s.len());
            if central_ports < split.peripheral.len() {
                return Err(format!(
                    "keyboard.toml: [split.central] defines {central_ports} serial port(s) for {} peripheral(s) — one port per peripheral is required, in peripheral order",
                    split.peripheral.len()
                ));
            }
            for (i, peri) in split.peripheral.iter().enumerate() {
                let n = peri.serial.as_ref().map_or(0, |s| s.len());
                if n != 1 {
                    return Err(format!(
                        "keyboard.toml: [[split.peripheral]] #{i} must define exactly 1 serial port, got {n}"
                    ));
                }
            }
        }
        SplitConnection::Ble => {
            for (board, ctx) in boards() {
                if board.serial.is_some() {
                    return Err(format!(
                        "keyboard.toml: {ctx} sets `serial`, but split.connection = \"ble\""
                    ));
                }
            }
        }
    }

    let regions: Vec<_> = boards()
        .map(|(b, ctx)| {
            (
                b.row_offset,
                b.row_offset + b.rows,
                b.col_offset,
                b.col_offset + b.cols,
                ctx,
            )
        })
        .collect();
    if let Some(layout) = layout {
        let (rows, cols) = (layout.rows as usize, layout.cols as usize);
        for (r0, r1, c0, c1, ctx) in &regions {
            if *r1 > rows || *c1 > cols {
                return Err(format!(
                    "keyboard.toml: {ctx} occupies rows {r0}..{r1}, cols {c0}..{c1}, which exceeds [layout] ({rows} rows x {cols} cols)"
                ));
            }
        }
    }
    for (i, a) in regions.iter().enumerate() {
        for b in &regions[i + 1..] {
            if a.0 < b.1 && b.0 < a.1 && a.2 < b.3 && b.2 < a.3 {
                return Err(format!(
                    "keyboard.toml: {} and {} overlap in the unified matrix — adjust row_offset/col_offset",
                    a.4, b.4
                ));
            }
        }
    }
    Ok(())
}

impl KeyboardTomlConfig {
    pub(crate) fn get_board_config(&self) -> Result<BoardConfig, String> {
        let matrix = self.matrix.clone();
        let split = self.split.clone();
        let input_device = self.input_device.clone();
        match (matrix, split) {
            (None, Some(s)) => {
                validate_split_config(&s, self.layout.as_ref())?;
                validate_matrix_dims(&s.central.matrix, s.central.rows, s.central.cols, "[split.central]")?;
                for (i, peri) in s.peripheral.iter().enumerate() {
                    validate_matrix_dims(&peri.matrix, peri.rows, peri.cols, &format!("[[split.peripheral]] #{i}"))?;
                }
                Ok(BoardConfig::Split(s))
            },
            (Some(m), None) => {
                match m.matrix_type {
                    MatrixType::Normal => {
                        if m.row_pins.is_none() || m.col_pins.is_none() {
                            return Err("`row_pins` and `col_pins` is required for normal matrix".to_string());
                        }
                    },
                    MatrixType::DirectPin => {
                        if m.direct_pins.is_none() {
                            return Err("`direct_pins` is required for direct pin matrix".to_string());
                        }
                    },
                }
                if let Some(layout) = &self.layout {
                    validate_matrix_dims(&m, layout.rows as usize, layout.cols as usize, "[matrix]")?;
                }
                // Top-level input_device applies only to unibody configs.
                Ok(BoardConfig::UniBody(UniBodyConfig{matrix: m, input_device: input_device.unwrap_or_default()}))
            },
            (None, None) => Err("[matrix] section in keyboard.toml is required for non-split keyboard".to_string()),
            _ => Err("Use at most one of [matrix] or [split] in your keyboard.toml!\n-> [matrix] is used to define a normal matrix of non-split keyboard\n-> [split] is used to define a split keyboard\n".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_matrix_dims, validate_split_config};
    use crate::{
        LayoutTomlConfig, MatrixConfig, MatrixType, SerialConfig, SplitBoardConfig, SplitConfig, SplitConnection,
    };

    fn normal(rows: &[&str], cols: &[&str]) -> MatrixConfig {
        MatrixConfig {
            matrix_type: MatrixType::Normal,
            row_pins: Some(rows.iter().map(|s| s.to_string()).collect()),
            col_pins: Some(cols.iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        }
    }

    #[test]
    fn normal_matrix_dims_must_match_pins() {
        let m = normal(&["r0", "r1"], &["c0", "c1", "c2"]);
        assert!(validate_matrix_dims(&m, 2, 3, "[matrix]").is_ok());
        assert!(validate_matrix_dims(&m, 3, 3, "[matrix]").is_err()); // too many rows
        assert!(validate_matrix_dims(&m, 2, 2, "[matrix]").is_err()); // too few cols
    }

    #[test]
    fn direct_pin_dims_must_match_grid() {
        let m = MatrixConfig {
            matrix_type: MatrixType::DirectPin,
            direct_pins: Some(vec![vec!["a".into(), "b".into()], vec!["c".into(), "d".into()]]),
            ..Default::default()
        };
        assert!(validate_matrix_dims(&m, 2, 2, "[matrix]").is_ok());
        assert!(validate_matrix_dims(&m, 3, 2, "[matrix]").is_err());
        assert!(validate_matrix_dims(&m, 2, 3, "[matrix]").is_err());
    }

    #[test]
    fn direct_pin_rejects_jagged_rows() {
        let m = MatrixConfig {
            matrix_type: MatrixType::DirectPin,
            direct_pins: Some(vec![vec!["a".into(), "b".into()], vec!["c".into()]]),
            ..Default::default()
        };
        assert!(validate_matrix_dims(&m, 2, 2, "[matrix]").is_err());
    }

    fn board(rows: usize, cols: usize, row_offset: usize, col_offset: usize) -> SplitBoardConfig {
        SplitBoardConfig {
            rows,
            cols,
            row_offset,
            col_offset,
            ..Default::default()
        }
    }

    fn ble_split() -> SplitConfig {
        SplitConfig {
            connection: SplitConnection::Ble,
            central: board(2, 2, 0, 0),
            peripheral: vec![board(2, 1, 2, 2)],
        }
    }

    fn layout_4x3() -> LayoutTomlConfig {
        LayoutTomlConfig {
            rows: 4,
            cols: 3,
            map: None,
            regions: None,
            default_variant: None,
            shapes: None,
            variant: None,
        }
    }

    #[test]
    fn ble_split_without_addr_is_valid() {
        // Dongle-style setups omit ble_addr entirely
        assert!(validate_split_config(&ble_split(), Some(&layout_4x3())).is_ok());
    }

    #[test]
    fn transport_must_match_connection() {
        let mut split = ble_split();
        split.peripheral[0].serial = Some(vec![SerialConfig::default()]);
        let err = validate_split_config(&split, None).unwrap_err();
        assert!(err.contains("sets `serial`"), "{err}");

        let mut split = ble_split();
        split.connection = SplitConnection::Serial;
        split.central.serial = Some(vec![SerialConfig::default()]);
        split.peripheral[0].serial = Some(vec![SerialConfig::default()]);
        split.peripheral[0].ble_addr = Some([0; 6]);
        let err = validate_split_config(&split, None).unwrap_err();
        assert!(err.contains("sets `ble_addr`"), "{err}");
    }

    #[test]
    fn serial_ports_must_cover_peripherals() {
        let mut split = ble_split();
        split.connection = SplitConnection::Serial;
        split.central.serial = Some(vec![SerialConfig::default()]);
        split.peripheral = vec![board(2, 1, 2, 2), board(2, 1, 2, 0)];
        split.peripheral[0].serial = Some(vec![SerialConfig::default()]);
        split.peripheral[1].serial = Some(vec![SerialConfig::default()]);
        let err = validate_split_config(&split, None).unwrap_err();
        assert!(err.contains("1 serial port(s) for 2 peripheral(s)"), "{err}");

        // Extra central ports beyond the peripheral count are fine
        split.central.serial = Some(vec![SerialConfig::default(); 3]);
        assert!(validate_split_config(&split, None).is_ok());
    }

    #[test]
    fn peripheral_needs_exactly_one_serial_port() {
        let mut split = ble_split();
        split.connection = SplitConnection::Serial;
        split.central.serial = Some(vec![SerialConfig::default()]);
        let err = validate_split_config(&split, None).unwrap_err();
        assert!(err.contains("exactly 1 serial port, got 0"), "{err}");
    }

    #[test]
    fn regions_must_fit_layout() {
        let mut split = ble_split();
        split.peripheral[0].row_offset = 3; // rows 3..5 exceeds 4
        let err = validate_split_config(&split, Some(&layout_4x3())).unwrap_err();
        assert!(err.contains("exceeds [layout]"), "{err}");
    }

    #[test]
    fn overlapping_regions_are_rejected() {
        let mut split = ble_split();
        split.peripheral.push(board(2, 1, 2, 2)); // identical to peripheral #0
        let err = validate_split_config(&split, Some(&layout_4x3())).unwrap_err();
        assert!(err.contains("overlap"), "{err}");
    }
}
