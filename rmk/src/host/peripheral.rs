//! USB update commands for a split peripheral, independent of its central.

use core::cell::RefCell;

use embassy_time::Duration;
use embedded_io_async::{Read, Write};
use rmk_types::protocol::vial::ViaCommand;

use super::lock::{HostLock, UnlockKeyState};
use super::via::system::process_system_packet;
use crate::config::VialConfig;
use crate::event::{KeyboardEvent, publish_event_async};
use crate::hid::ViaReport;
use crate::input_device::InputDevice;
use crate::matrix::MatrixState;
use crate::split::PeripheralMatrixConfig;

/// Serves Vial identity, unlock and bootloader commands without a keymap.
/// Run `run_matrix` alongside the USB service so unlock works without a split link.
pub struct PeripheralService {
    config: VialConfig<'static>,
    matrix: RefCell<MatrixState>,
    matrix_config: PeripheralMatrixConfig,
}

impl PeripheralService {
    /// `config` uses shared-layout positions; `matrix_config` identifies this side's local region.
    pub fn new(config: VialConfig<'static>, matrix_config: PeripheralMatrixConfig) -> Self {
        Self {
            config,
            matrix: RefCell::new(MatrixState::new(
                matrix_config.rows as usize,
                matrix_config.cols as usize,
            )),
            matrix_config,
        }
    }

    /// Run instead of the matrix's normal task. Coordinates are local to this side.
    pub async fn run_matrix<M: InputDevice<Event = KeyboardEvent>>(&self, matrix: &mut M) -> ! {
        loop {
            let event = matrix.read_event().await;
            self.matrix.borrow_mut().update(&event);
            publish_event_async(event).await;
        }
    }

    /// A new transport session always starts locked unless explicitly insecure.
    pub async fn run_session<R: Read, W: Write>(&self, rx: &mut R, tx: &mut W) {
        let locker = HostLock::new(
            self.config.unlock_keys,
            self,
            self.config.insecure,
            Duration::from_millis(100),
        );
        let mut buf = [0u8; 32];
        loop {
            if rx.read_exact(&mut buf).await.is_err() {
                return;
            }
            let mut report = ViaReport {
                input_data: buf,
                output_data: buf,
            };
            if !process_system_packet(&mut report, &self.config, &locker) {
                report.input_data[0] = ViaCommand::Unhandled as u8;
            }
            if tx.write_all(&report.input_data).await.is_err() {
                return;
            }
        }
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
    use core::sync::atomic::Ordering;

    use embassy_futures::block_on;

    use super::*;

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

    fn requests(commands: &[[u8; 2]]) -> std::vec::Vec<u8> {
        commands
            .iter()
            .flat_map(|cmd| {
                let mut packet = [0u8; 32];
                packet[..2].copy_from_slice(cmd);
                packet
            })
            .collect()
    }

    #[test]
    fn peripheral_unlock_is_local_and_expires_with_the_session() {
        let first = PeripheralService::new(CONFIG, MATRIX);
        let second = PeripheralService::new(CONFIG, MATRIX);
        first.matrix.borrow_mut().update(&KeyboardEvent::key(0, 0, true));
        first.matrix.borrow_mut().update(&KeyboardEvent::key(1, 1, true));
        let input = requests(&[
            [0x0b, 0],
            [0xfe, 6],
            [0xfe, 7],
            [0xfe, 5],
            [0x0b, 0],
            [0xfe, 8],
            [0x0b, 0],
        ]);
        let mut replies = [0u8; 32 * 7];
        block_on(first.run_session(&mut input.as_slice(), &mut replies.as_mut_slice()));
        assert_eq!(
            replies[0], 0xff,
            "jump is refused before unlock even while keys are held"
        );
        assert_eq!(replies[3 * 32], 1, "local keys unlock this side");
        assert_eq!(
            &replies[3 * 32 + 2..3 * 32 + 6],
            &[2, 3, 3, 4],
            "GUI positions match the shared definition"
        );
        assert_eq!(replies[6 * 32], 0xff, "Lock closes the bootloader gate");
        assert_eq!(crate::boot::BOOTLOADER_JUMPS.load(Ordering::Relaxed), 1);

        let input = requests(&[[0xfe, 6], [0xfe, 7], [0xfe, 5], [0x0b, 0]]);
        let mut replies = [0u8; 32 * 4];
        block_on(second.run_session(&mut input.as_slice(), &mut replies.as_mut_slice()));
        assert_eq!(replies[2 * 32], 0, "the other side remains locked");
        assert_eq!(replies[3 * 32], 0xff);

        let input = requests(&[[0xfe, 6], [0xfe, 7]]);
        block_on(first.run_session(&mut input.as_slice(), &mut [0u8; 64].as_mut_slice()));
        let input = requests(&[[0xfe, 5], [0x0b, 0]]);
        let mut replies = [0u8; 64];
        block_on(first.run_session(&mut input.as_slice(), &mut replies.as_mut_slice()));
        assert_eq!(
            replies[0], 0,
            "a new connection starts locked even if keys are still held"
        );
        assert_eq!(replies[32], 0xff);
        assert_eq!(crate::boot::BOOTLOADER_JUMPS.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn local_matrix_unlocks_without_a_central_and_keeps_forwarding_keys() {
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
        let service = PeripheralService::new(CONFIG, MATRIX);
        let mut subscriber = KeyboardEvent::subscriber();
        let update = async {
            for event in events {
                assert_eq!(subscriber.next_message_pure().await, event);
            }
            let input = requests(&[[0xfe, 6], [0xfe, 7], [0xfe, 5]]);
            let mut replies = [0u8; 96];
            service
                .run_session(&mut input.as_slice(), &mut replies.as_mut_slice())
                .await;
            assert_eq!(replies[64], 1);
        };
        block_on(embassy_futures::select::select(service.run_matrix(&mut matrix), update));
    }

    #[test]
    fn peripheral_answers_identity_and_refuses_keymap_writes() {
        let service = PeripheralService::new(CONFIG, MATRIX);
        let input = requests(&[[1, 0], [0xfe, 0], [0xfe, 1], [0xfe, 2], [5, 0]]);
        let mut replies = [0u8; 32 * 5];
        block_on(service.run_session(&mut input.as_slice(), &mut replies.as_mut_slice()));
        assert_eq!(replies[0], 1);
        assert_eq!(&replies[36..44], CONFIG.vial_keyboard_id);
        assert_eq!(u32::from_le_bytes(replies[64..68].try_into().unwrap()), 4);
        assert_eq!(&replies[96..100], CONFIG.vial_keyboard_def);
        assert_eq!(replies[128], 0xff);
    }
}
