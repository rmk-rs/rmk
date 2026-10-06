use embedded_io_async::{Read, Write};

use super::driver::SplitDriverError;
use crate::split::driver::{PeripheralManager, SplitReader, SplitWriter, set_peripheral_connected};
use crate::split::{SPLIT_MESSAGE_MAX_SIZE, SplitMessage};

/// Receive split message from peripheral via serial and process it
///
/// Generic parameters:
/// - `const ROW`: row number of the peripheral's matrix
/// - `const COL`: column number of the peripheral's matrix
/// - `const ROW_OFFSET`: row offset of the peripheral's matrix in the whole matrix
/// - `const COL_OFFSET`: column offset of the peripheral's matrix in the whole matrix
/// - `S`: a serial port that implements `Read` and `Write` trait in embedded-io-async
pub(crate) async fn run_serial_peripheral_manager<S: Read + Write>(
    id: usize,
    receiver: S,
    matrix_config: crate::split::PeripheralMatrixConfig,
    #[cfg(feature = "dfu_split")] policy: crate::split::dfu::UpdatePolicy,
) {
    let split_serial_driver: SerialSplitDriver<S> = SerialSplitDriver::new(receiver);
    let peripheral_manager = PeripheralManager::new(
        split_serial_driver,
        id,
        matrix_config,
        #[cfg(feature = "dfu_split")]
        policy,
    );
    info!("Running peripheral manager {}", id);

    // A wired peripheral is connected for as long as its manager runs.
    set_peripheral_connected(id, true);
    peripheral_manager.run().await;
}

/// Serial driver for BOTH split central and peripheral
pub(crate) struct SerialSplitDriver<S> {
    serial: S,
    buffer: [u8; SPLIT_MESSAGE_MAX_SIZE],
    n_bytes_part: usize,
}

impl<S> SerialSplitDriver<S> {
    pub(crate) fn new(serial: S) -> Self {
        Self {
            serial,
            buffer: [0_u8; SPLIT_MESSAGE_MAX_SIZE],
            n_bytes_part: 0,
        }
    }
}

impl<S: Read> SplitReader for SerialSplitDriver<S> {
    async fn read(&mut self) -> Result<SplitMessage, SplitDriverError> {
        const SENTINEL: u8 = 0x00;
        // Check the buffer *before* reading: a prior read() call may have
        // pulled in more than one complete message, and the next one is
        // already waiting in `self.buffer[..self.n_bytes_part]`.
        while !self.buffer[..self.n_bytes_part].contains(&SENTINEL) && self.n_bytes_part < self.buffer.len() {
            let n_bytes = self
                .serial
                .read(&mut self.buffer[self.n_bytes_part..])
                .await
                .map_err(|_e| {
                    self.n_bytes_part = 0;
                    SplitDriverError::SerialError
                })?;
            if n_bytes == 0 {
                return Err(SplitDriverError::EmptyMessage);
            }
            debug_assert!(self.n_bytes_part + n_bytes <= self.buffer.len());
            self.n_bytes_part += n_bytes;
        }

        let (result, n_bytes_unused) =
            match postcard::take_from_bytes_cobs::<SplitMessage>(&mut self.buffer[..self.n_bytes_part]) {
                Ok((message, unused_bytes)) => (Ok(message), unused_bytes.len()),
                Err(e) => {
                    error!("Postcard deserialize split message error: {}", e);
                    let n_bytes_unused = self.buffer[..self.n_bytes_part]
                        .iter()
                        .position(|&x| x == SENTINEL)
                        .map_or(0, |index| self.n_bytes_part - index - 1);
                    (Err(SplitDriverError::SerializeError), n_bytes_unused)
                }
            };

        self.buffer
            .copy_within(self.n_bytes_part - n_bytes_unused..self.n_bytes_part, 0);
        self.n_bytes_part = n_bytes_unused;

        result
    }
}

impl<S: Write> SplitWriter for SerialSplitDriver<S> {
    async fn write(&mut self, message: &SplitMessage) -> Result<usize, SplitDriverError> {
        let mut buf = [0_u8; SPLIT_MESSAGE_MAX_SIZE];
        let bytes = postcard::to_slice_cobs(message, &mut buf).map_err(|e| {
            error!("Postcard serialize split message error: {}", e);
            SplitDriverError::SerializeError
        })?;
        let mut remaining_bytes = bytes.len();
        while remaining_bytes > 0 {
            let sent_bytes = self
                .serial
                .write(&bytes[bytes.len() - remaining_bytes..])
                .await
                .map_err(|_e| SplitDriverError::SerialError)?;
            remaining_bytes -= sent_bytes;
        }
        Ok(bytes.len())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::convert::Infallible;

    use embassy_futures::block_on;
    use embedded_io_async::ErrorType;

    use super::*;

    /// Fake `embedded_io_async::Read`: each `serial.read()` call returns the
    /// next scripted chunk. Panics if the driver calls `read()` more times
    /// than we scripted — that is itself a useful assertion, since #801 is
    /// about the driver making a `read()` call it should not have made.
    struct FakeSerial {
        chunks: VecDeque<Vec<u8>>,
        read_calls: usize,
    }

    impl FakeSerial {
        fn new<I: IntoIterator<Item = Vec<u8>>>(chunks: I) -> Self {
            Self {
                chunks: chunks.into_iter().collect(),
                read_calls: 0,
            }
        }
    }

    impl ErrorType for FakeSerial {
        type Error = Infallible;
    }

    impl Read for FakeSerial {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            self.read_calls += 1;
            let chunk = self
                .chunks
                .pop_front()
                .expect("SerialSplitDriver made an unexpected underlying read() call");
            assert!(
                chunk.len() <= buf.len(),
                "scripted chunk larger than driver's read slice"
            );
            buf[..chunk.len()].copy_from_slice(&chunk);
            Ok(chunk.len())
        }
    }

    fn encode(msg: &SplitMessage) -> Vec<u8> {
        let mut buf = [0u8; SPLIT_MESSAGE_MAX_SIZE];
        let encoded = postcard::to_slice_cobs(msg, &mut buf).unwrap();
        encoded.to_vec()
    }

    #[test]
    fn read_single_message_in_one_chunk() {
        let fake = FakeSerial::new([encode(&SplitMessage::LedState(true))]);
        let mut drv = SerialSplitDriver::new(fake);

        let msg = block_on(drv.read()).expect("read should succeed");
        assert!(matches!(msg, SplitMessage::LedState(true)));
        assert_eq!(drv.serial.read_calls, 1);
    }

    #[test]
    fn read_message_split_across_chunks() {
        let bytes = encode(&SplitMessage::LedState(true));
        let (a, b) = bytes.split_at(bytes.len() / 2);
        let fake = FakeSerial::new([a.to_vec(), b.to_vec()]);
        let mut drv = SerialSplitDriver::new(fake);

        let msg = block_on(drv.read()).expect("read should succeed");
        assert!(matches!(msg, SplitMessage::LedState(true)));
        assert_eq!(drv.serial.read_calls, 2);
    }

    /// Regression test for https://github.com/rmk-rs/rmk/issues/801: when
    /// two complete messages arrive in a single underlying read, the driver
    /// must deliver both without issuing a second underlying read.
    #[test]
    fn two_bundled_messages_do_not_trigger_extra_read() {
        let status = rmk_types::connection::ConnectionStatus::new();
        let mut bundled = encode(&SplitMessage::LedState(true));
        bundled.extend_from_slice(&encode(&SplitMessage::ConnectionStatus(status)));

        let fake = FakeSerial::new([bundled]);
        let mut drv = SerialSplitDriver::new(fake);

        let m1 = block_on(drv.read()).expect("first read should succeed");
        assert!(matches!(m1, SplitMessage::LedState(true)));

        let m2 = block_on(drv.read()).expect("second read should not touch serial");
        match m2 {
            SplitMessage::ConnectionStatus(s) => assert_eq!(s, status),
            other => panic!("expected ConnectionStatus, got {:?}", other),
        }

        assert_eq!(drv.serial.read_calls, 1);
    }

    /// Complete message followed by the prefix of a second in one chunk; the
    /// suffix of the second arrives later. The driver should deliver the
    /// first immediately, then assemble the second from the carried-over
    /// prefix plus the next chunk.
    #[test]
    fn trailing_partial_message_is_carried_over() {
        let status = rmk_types::connection::ConnectionStatus::new();
        let full1 = encode(&SplitMessage::LedState(true));
        let full2 = encode(&SplitMessage::ConnectionStatus(status));
        let (prefix, suffix) = full2.split_at(full2.len() / 2);

        let mut first_chunk = full1;
        first_chunk.extend_from_slice(prefix);

        let fake = FakeSerial::new([first_chunk, suffix.to_vec()]);
        let mut drv = SerialSplitDriver::new(fake);

        let m1 = block_on(drv.read()).expect("first read should succeed");
        assert!(matches!(m1, SplitMessage::LedState(true)));

        let m2 = block_on(drv.read()).expect("second read should succeed");
        match m2 {
            SplitMessage::ConnectionStatus(s) => assert_eq!(s, status),
            other => panic!("expected ConnectionStatus, got {:?}", other),
        }

        assert_eq!(drv.serial.read_calls, 2);
    }
}
