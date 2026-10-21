//! Local matrix state for a split peripheral's USB host service.

use core::cell::RefCell;

use embedded_io_async::{Read, Write};

use super::lock::UnlockKeyState;
#[cfg(feature = "rynk")]
use super::rynk::RynkService as Service;
#[cfg(feature = "vial")]
use super::via::VialService as Service;
use crate::config::RmkConfig;
use crate::event::{KeyboardEvent, publish_event_async};
use crate::input_device::InputDevice;
use crate::matrix::MatrixState;
use crate::split::PeripheralMatrixConfig;

/// Records this side's keys without a keymap or a connection to the central.
pub struct PeripheralService {
    config: RmkConfig<'static>,
    matrix: RefCell<MatrixState>,
    matrix_config: PeripheralMatrixConfig,
}

impl PeripheralService {
    pub fn new(config: RmkConfig<'static>, matrix_config: PeripheralMatrixConfig) -> Self {
        Self {
            config,
            matrix: RefCell::new(MatrixState::new(
                matrix_config.rows as usize,
                matrix_config.cols as usize,
            )),
            matrix_config,
        }
    }

    pub async fn run_matrix<M: InputDevice<Event = KeyboardEvent>>(&self, matrix: &mut M) -> ! {
        loop {
            let event = matrix.read_event().await;
            self.matrix.borrow_mut().update(&event);
            publish_event_async(event).await;
        }
    }

    pub async fn run_session<R: Read, W: Write>(&self, rx: &mut R, tx: &mut W) {
        Service::<false>::new_peripheral(self, &self.config)
            .run(rx, tx)
            .await;
    }
}

impl UnlockKeyState for PeripheralService {
    fn is_pressed(&self, row: u8, col: u8) -> bool {
        let Some(row) = row.checked_sub(self.matrix_config.row_offset) else {
            return false;
        };
        let Some(col) = col.checked_sub(self.matrix_config.col_offset) else {
            return false;
        };
        self.matrix.borrow().read(row, col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_matrix_coordinates_do_not_leak_between_halves() {
        let matrix = PeripheralMatrixConfig {
            rows: 2,
            cols: 2,
            row_offset: 2,
            col_offset: 3,
        };
        let local = PeripheralService::new(RmkConfig::default(), matrix);
        let other = PeripheralService::new(RmkConfig::default(), matrix);
        local.matrix.borrow_mut().update(&KeyboardEvent::key(0, 0, true));
        local.matrix.borrow_mut().update(&KeyboardEvent::key(1, 1, true));

        assert!(local.is_pressed(2, 3));
        assert!(local.is_pressed(3, 4));
        assert!(!local.is_pressed(1, 3));
        assert!(!local.is_pressed(2, 2));
        assert!(!local.is_pressed(4, 5));
        assert!(!other.is_pressed(2, 3));
        assert!(!other.is_pressed(3, 4));
    }
}
