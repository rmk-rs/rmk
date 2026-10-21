//! Local matrix state for a split peripheral's USB host service.

use core::cell::RefCell;

use super::lock::UnlockKeyState;
use crate::event::{KeyboardEvent, publish_event_async};
use crate::input_device::InputDevice;
use crate::matrix::MatrixState;
use crate::split::PeripheralMatrixConfig;

/// Records this side's keys without a keymap or a connection to the central.
pub struct PeripheralMatrix {
    matrix: RefCell<MatrixState>,
    matrix_config: PeripheralMatrixConfig,
}

impl PeripheralMatrix {
    pub fn new(matrix_config: PeripheralMatrixConfig) -> Self {
        Self {
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
}

impl UnlockKeyState for PeripheralMatrix {
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
pub(crate) mod tests {
    use super::*;
    use crate::event::SubscribableEvent;

    pub(crate) fn scanned_matrix(pressed: bool) -> PeripheralMatrix {
        #[rmk_macro::input_device(publish = KeyboardEvent)]
        struct Matrix(Option<KeyboardEvent>);
        impl Matrix {
            async fn read_keyboard_event(&mut self) -> KeyboardEvent {
                match self.0.take() {
                    Some(event) => event,
                    None => core::future::pending().await,
                }
            }
        }
        let keys = PeripheralMatrix::new(PeripheralMatrixConfig {
            rows: 1,
            cols: 1,
            row_offset: 2,
            col_offset: 3,
        });
        let event = KeyboardEvent::key(0, 0, pressed);
        let mut matrix = Matrix(Some(event));
        let mut subscriber = KeyboardEvent::subscriber();
        assert!(embassy_futures::poll_once(keys.run_matrix(&mut matrix)).is_pending());
        assert_eq!(subscriber.try_next_message_pure(), Some(event));
        keys
    }
}
