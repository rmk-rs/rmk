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

#[cfg(all(test, feature = "vial"))]
mod tests {
    use embassy_futures::block_on;
    use embassy_time::Duration;

    use super::*;
    use crate::config::VialConfig;
    use crate::host::lock::HostLock;

    const CONFIG: VialConfig<'static> = VialConfig {
        vial_keyboard_id: &[1; 8],
        vial_keyboard_def: &[2; 4],
        unlock_keys: &[(2, 3), (3, 4)],
        insecure: false,
    };

    const MATRIX: PeripheralMatrixConfig = PeripheralMatrixConfig {
        rows: 2,
        cols: 2,
        row_offset: 2,
        col_offset: 3,
    };

    fn peripheral() -> PeripheralService {
        let config = RmkConfig {
            vial_config: CONFIG,
            ..Default::default()
        };
        PeripheralService::new(config, MATRIX)
    }

    #[test]
    fn local_matrix_unlocks_only_this_half_and_keeps_forwarding_keys() {
        use crate::core_traits::Runnable;
        use crate::event::SubscribableEvent;
        struct Matrix(std::collections::VecDeque<KeyboardEvent>);
        impl Runnable for Matrix {
            async fn run(&mut self) -> ! {
                panic!("the service must run the local scanner");
            }
        }
        impl InputDevice for Matrix {
            type Event = KeyboardEvent;
            async fn read_event(&mut self) -> KeyboardEvent {
                match self.0.pop_front() {
                    Some(event) => event,
                    None => core::future::pending().await,
                }
            }
        }
        let events = [KeyboardEvent::key(0, 0, true), KeyboardEvent::key(1, 1, true)];
        let mut matrix = Matrix(events.into());
        let service = peripheral();
        let other = peripheral();
        let mut subscriber = KeyboardEvent::subscriber();
        let update = async {
            for event in events {
                assert_eq!(subscriber.next_message_pure().await, event);
            }
            let local = HostLock::new(CONFIG.unlock_keys, &service, false, Duration::from_millis(100));
            let remote = HostLock::new(CONFIG.unlock_keys, &other, false, Duration::from_millis(100));
            local.unlocking();
            remote.unlocking();
            assert_eq!(local.check_unlock(), 0);
            assert_eq!(remote.check_unlock(), 2);
            assert!(local.is_unlocked());
            assert!(!remote.is_unlocked(), "the other half cannot use these held keys");
        };
        block_on(embassy_futures::select::select(service.run_matrix(&mut matrix), update));
    }
}
