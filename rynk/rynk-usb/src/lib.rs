//! Rynk over the vendor HID interface, using the platform HID driver.
//!
//! Discovery matches the Rynk usage page and usage, independently of VID/PID.
//! Frames use fixed-size, unnumbered HID reports with zero padding. Opening a
//! session leaves the ordinary keyboard interface attached to the OS.

use async_hid::{AsyncHidRead, AsyncHidWrite, Device, DeviceId, DeviceReader, DeviceWriter, HidBackend};
use futures_util::StreamExt;
use rynk::io::{ErrorType, Read, Write};
use rynk::rmk_types::protocol::rynk::RYNK_HID_REPORT_SIZE;
use rynk::{RynkDevice, RynkHostError};

const RYNK_USAGE_PAGE: u16 = 0xFF14;
const RYNK_USAGE: u16 = 0x61;

/// A Rynk HID device available to the current user.
pub struct UsbDevice {
    device: Device,
}

impl UsbDevice {
    pub async fn discover() -> Result<Vec<Self>, RynkHostError> {
        let backend = HidBackend::default();
        let mut devices = backend
            .enumerate()
            .await
            .map_err(|e| RynkHostError::Transport("enumerate_hid", e.to_string()))?;
        let mut found = Vec::new();
        while let Some(device) = devices.next().await {
            if device.usage_page == RYNK_USAGE_PAGE && device.usage_id == RYNK_USAGE {
                found.push(Self { device });
            }
        }
        Ok(found)
    }

    /// Platform HID identity for matching successive discovery results.
    pub fn id(&self) -> DeviceId {
        self.device.id.clone()
    }
}

impl RynkDevice for UsbDevice {
    type Read = UsbReader;
    type Write = UsbWriter;

    fn label(&self) -> String {
        if self.device.name.is_empty() {
            format!("USB {:04x}:{:04x}", self.device.vendor_id, self.device.product_id)
        } else {
            self.device.name.clone()
        }
    }

    async fn open(self) -> Result<(UsbReader, UsbWriter), RynkHostError> {
        let (reader, writer) = self
            .device
            .open()
            .await
            .map_err(|e| RynkHostError::Transport("open_hid", e.to_string()))?;
        Ok((
            UsbReader {
                reader,
                report: [0; RYNK_HID_REPORT_SIZE],
                pos: 0,
                end: 0,
            },
            UsbWriter {
                writer,
                pending: [0; RYNK_HID_REPORT_SIZE + 1],
                len: 0,
            },
        ))
    }
}

/// Buffers a complete HID report so small stream reads cannot truncate it.
pub struct UsbReader<R = DeviceReader> {
    reader: R,
    report: [u8; RYNK_HID_REPORT_SIZE],
    pos: usize,
    end: usize,
}

impl<R> ErrorType for UsbReader<R> {
    type Error = std::io::Error;
}

impl<R: AsyncHidRead> Read for UsbReader<R> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.pos == self.end {
            self.end = self
                .reader
                .read_input_report(&mut self.report)
                .await
                .map_err(std::io::Error::other)?;
            self.pos = 0;
            if self.end == 0 {
                return Ok(0);
            }
        }
        let n = buf.len().min(self.end - self.pos);
        buf[..n].copy_from_slice(&self.report[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Writes frame fragments as fixed-size HID output reports.
pub struct UsbWriter<W = DeviceWriter> {
    writer: W,
    pending: [u8; RYNK_HID_REPORT_SIZE + 1],
    len: usize,
}

impl<W> ErrorType for UsbWriter<W> {
    type Error = std::io::Error;
}

impl<W: AsyncHidWrite> Write for UsbWriter<W> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        let remaining = RYNK_HID_REPORT_SIZE - self.len;
        let n = buf
            .iter()
            .position(|byte| *byte == 0)
            .map_or(buf.len(), |index| index + 1)
            .min(remaining);
        self.pending[1 + self.len..1 + self.len + n].copy_from_slice(&buf[..n]);
        self.len += n;
        if self.len == RYNK_HID_REPORT_SIZE || buf[n - 1] == 0 {
            // Byte zero is the report ID; padding is added only at frame boundaries.
            self.writer
                .write_output_report(&self.pending)
                .await
                .map_err(std::io::Error::other)?;
            self.pending.fill(0);
            self.len = 0;
        }
        Ok(n)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        if self.len != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "incomplete Rynk frame",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use async_hid::HidResult;

    use super::*;

    struct Reports(VecDeque<Vec<u8>>);

    impl AsyncHidRead for Reports {
        async fn read_input_report(&mut self, buf: &mut [u8]) -> HidResult<usize> {
            let Some(report) = self.0.pop_front() else { return Ok(0) };
            assert!(buf.len() >= report.len());
            buf[..report.len()].copy_from_slice(&report);
            Ok(report.len())
        }
    }

    impl AsyncHidWrite for Reports {
        async fn write_output_report(&mut self, buf: &[u8]) -> HidResult<()> {
            self.0.push_back(buf.to_vec());
            Ok(())
        }
    }

    #[tokio::test]
    async fn complete_frames_survive_report_fragmentation_and_small_reads() {
        // Nonzero COBS body bytes, followed by the frame delimiter.
        for len in [1, 31, 32, 33, 63, 64, 65, 257] {
            let mut frame = vec![7; len];
            *frame.last_mut().unwrap() = 0;
            let mut writer = UsbWriter {
                writer: Reports(VecDeque::new()),
                pending: [0; RYNK_HID_REPORT_SIZE + 1],
                len: 0,
            };
            for fragment in frame.chunks(3) {
                writer.write_all(fragment).await.unwrap();
            }
            writer.flush().await.unwrap();
            for report in &writer.writer.0 {
                assert_eq!(report.len(), RYNK_HID_REPORT_SIZE + 1);
                assert_eq!(report[0], 0);
            }
            let reports = writer.writer.0.into_iter().map(|report| report[1..].to_vec()).collect();
            let mut reader = UsbReader {
                reader: Reports(reports),
                report: [0; RYNK_HID_REPORT_SIZE],
                pos: 0,
                end: 0,
            };
            assert_eq!(reader.read(&mut []).await.unwrap(), 0);
            let mut received = Vec::new();
            let mut small = [0; 3];
            loop {
                let n = reader.read(&mut small).await.unwrap();
                if n == 0 {
                    break;
                }
                received.extend_from_slice(&small[..n]);
            }
            assert_eq!(&received[..len], frame);
            assert!(received[len..].iter().all(|byte| *byte == 0));
        }
    }
    struct Disconnected;

    impl AsyncHidRead for Disconnected {
        async fn read_input_report(&mut self, _: &mut [u8]) -> HidResult<usize> {
            Err(async_hid::HidError::Disconnected)
        }
    }

    impl AsyncHidWrite for Disconnected {
        async fn write_output_report(&mut self, _: &[u8]) -> HidResult<()> {
            Err(async_hid::HidError::Disconnected)
        }
    }

    #[tokio::test]
    async fn transport_errors_reach_the_driver() {
        let mut reader = UsbReader {
            reader: Disconnected,
            report: [0; RYNK_HID_REPORT_SIZE],
            pos: 0,
            end: 0,
        };
        assert!(reader.read(&mut [0; 1]).await.is_err());
        let mut writer = UsbWriter {
            writer: Disconnected,
            pending: [0; RYNK_HID_REPORT_SIZE + 1],
            len: 0,
        };
        assert!(writer.write(&[1, 0]).await.is_err());
    }
}
