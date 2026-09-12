use embassy_futures::select::{Either, select};

use super::SplitMessage;
use super::driver::{PeripheralManager, SplitReader, SplitWriter};
use crate::event::{DfuCmdEvent, publish_event};

/// Defines how the central decides whether to flash a peripheral.
#[derive(Clone, Copy)]
pub enum UpdatePolicy {
    /// Compare the firmware hash — only flash when it differs.
    MatchHash,
    /// Always flash the firmware regardless of the current version.
    Force,
}

impl<T: SplitReader + SplitWriter> PeripheralManager<T> {
    /// Handle a proactive `FirmwareHashResponse` received in the main event
    /// loop (after the initial `check_firmware_update` may have timed out
    /// because the peripheral was not yet booted).
    pub(crate) async fn handle_proactive_hash(&mut self, hash: u32) {
        let (firmware, expected_hash) = match crate::dfu::get_firmware_update_data(self.id) {
            Some(d) => d,
            None => {
                info!(
                    "dfu_split: no firmware data set for peripheral {}, skipping proactive hash",
                    self.id
                );
                return;
            }
        };
        info!("dfu_split: proactive hash from peripheral ({:#x}), checking...", hash);
        if hash == expected_hash {
            info!("dfu_split: hash matches ({:#x}), no update needed", hash);
            return;
        }
        info!("dfu_split: hash mismatch, starting update ({} bytes)", firmware.len());
        self.send_firmware_update(firmware, expected_hash).await;
    }

    /// Process a DFU command event targeted at this peripheral.
    ///
    /// Called from the event loop when a [`DfuCmdEvent`] is received via PubSub.
    /// Forwards DFU commands targeted at this peripheral over the split link.
    ///
    /// On `Finish`, triggers end-to-end CRC verification: the peripheral
    /// reads back its DFU partition, sends the CRC-32, the central
    /// compares, and sends `FirmwareCrcOk` / `FirmwareCrcFail`.
    pub(crate) async fn handle_dfu_event(&mut self, cmd_event: DfuCmdEvent) {
        use embassy_time::{Duration, Instant, Timer};

        match cmd_event.0 {
            crate::dfu::DfuCmd::UnlockRequest => {}
            crate::dfu::DfuCmd::Start(crate::dfu::DfuTarget::ForwardPeripheral(id)) if id == self.id as u8 => {
                self.passthrough_crc = crate::crc32::Crc32::new();
                self.dfu_aborted = false;
                info!("dfu_split: DFU download started for peripheral {}", self.id);
            }
            crate::dfu::DfuCmd::Write(crate::dfu::DfuTarget::ForwardPeripheral(id), base_offset, data)
                if id == self.id as u8 =>
            {
                if self.dfu_aborted {
                    return;
                }
                const MAX_RETRIES: u32 = 3;
                for (chunk_idx, chunk) in data.chunks(crate::split::SPLIT_CHUNK_SIZE).enumerate() {
                    let mut buf = [0u8; crate::split::SPLIT_CHUNK_SIZE];
                    buf[..chunk.len()].copy_from_slice(chunk);
                    let chunk_offset = base_offset + (chunk_idx * crate::split::SPLIT_CHUNK_SIZE) as u32;
                    let chunk_crc = crate::crc32::crc32(&buf[..chunk.len()]);
                    self.passthrough_crc.update(&buf[..chunk.len()]);

                    let mut retries = 0;
                    let mut acked = false;

                    while !acked && retries < MAX_RETRIES {
                        if retries > 0 {
                            info!(
                                "dfu_split: retry {}/{} for chunk at offset {}",
                                retries + 1,
                                MAX_RETRIES,
                                chunk_offset
                            );
                        }

                        debug!(
                            "dfu_split: forwarding chunk to peripheral {} @ offset {} ({} bytes)",
                            self.id,
                            chunk_offset,
                            chunk.len()
                        );

                        let msg = SplitMessage::FirmwareChunk {
                            offset: chunk_offset,
                            len: chunk.len() as u16,
                            data: super::FirmwareChunkData(buf),
                        };
                        if self.send(&msg).await.is_err() {
                            error!("dfu_split: disconnected during chunk send");
                            return;
                        }

                        let deadline = Instant::now() + Duration::from_secs(5);
                        let got = loop {
                            match select(self.transceiver.read(), Timer::at(deadline)).await {
                                Either::First(Ok(SplitMessage::FirmwareChunkAck {
                                    offset: ack_offset,
                                    crc: ack_crc,
                                })) => {
                                    if ack_offset == chunk_offset {
                                        if ack_crc == chunk_crc {
                                            break true;
                                        }
                                        warn!(
                                            "dfu_split: per-chunk CRC mismatch at offset {} (peripheral={:#010x}, central={:#010x})",
                                            chunk_offset, ack_crc, chunk_crc
                                        );
                                        break false;
                                    }
                                    info!(
                                        "dfu_split: got ack for offset {}, waiting for {}",
                                        ack_offset, chunk_offset
                                    );
                                }
                                Either::First(Ok(_)) => {}
                                Either::First(Err(e)) => {
                                    error!("dfu_split: read error: {:?}", e);
                                    break false;
                                }
                                Either::Second(_) => {
                                    error!("dfu_split: timeout waiting for chunk ack");
                                    break false;
                                }
                            }
                        };
                        acked = got;
                        retries += 1;
                    }

                    if !acked {
                        error!(
                            "dfu_split: chunk at offset {} failed after {} retries, giving up on peripheral {}",
                            chunk_offset, MAX_RETRIES, self.id
                        );
                        self.dfu_aborted = true;
                        crate::dfu::DFU_WRITE_FAILED.store(true, core::sync::atomic::Ordering::Release);
                        publish_event(crate::event::DfuStatusEvent::new(rmk_types::dfu::DfuStatus::Error));
                        return;
                    }
                }
            }
            crate::dfu::DfuCmd::Finish(crate::dfu::DfuTarget::ForwardPeripheral(id)) if id == self.id as u8 => {
                if self.dfu_aborted {
                    return;
                }
                info!("dfu_split: DFU download complete, starting end-to-end verification");

                if self.send(&SplitMessage::FirmwareUpdateComplete).await.is_err() {
                    error!("dfu_split: disconnected during finish");
                    return;
                }

                let deadline = Instant::now() + Duration::from_secs(5);
                let crc = loop {
                    match select(self.transceiver.read(), Timer::at(deadline)).await {
                        Either::First(Ok(SplitMessage::FirmwareCrcReport(crc))) => break Some(crc),
                        Either::First(Ok(_)) => {}
                        Either::First(Err(e)) => {
                            error!("dfu_split: read error: {:?}", e);
                            break None;
                        }
                        Either::Second(_) => {
                            error!("dfu_split: timeout waiting for CRC");
                            break None;
                        }
                    }
                };

                let Some(peripheral_crc) = crc else {
                    error!("dfu_split: CRC verification failed");
                    self.passthrough_crc = crate::crc32::Crc32::new();
                    self.send(&SplitMessage::FirmwareCrcFail).await.ok();
                    publish_event(crate::event::DfuStatusEvent::new(rmk_types::dfu::DfuStatus::Error));
                    return;
                };

                let central_crc = self.passthrough_crc.finalize();
                self.passthrough_crc = crate::crc32::Crc32::new();

                if central_crc != peripheral_crc {
                    error!(
                        "dfu_split: CRC mismatch (central={:#010x}, peripheral={:#010x})",
                        central_crc, peripheral_crc
                    );
                    self.send(&SplitMessage::FirmwareCrcFail).await.ok();
                    publish_event(crate::event::DfuStatusEvent::new(rmk_types::dfu::DfuStatus::Error));
                    return;
                }

                info!("dfu_split: CRC OK, confirming update");
                if self.send(&SplitMessage::FirmwareCrcOk).await.is_err() {
                    error!("dfu_split: disconnected during CRC OK");
                    return;
                }

                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    match select(self.transceiver.read(), Timer::at(deadline)).await {
                        Either::First(Ok(SplitMessage::FirmwareUpdateConfirm)) => {
                            info!("dfu_split: peripheral confirmed, update complete");
                            publish_event(crate::event::DfuStatusEvent::new(rmk_types::dfu::DfuStatus::Finished));
                            break;
                        }
                        Either::First(Ok(_)) => {}
                        Either::First(Err(e)) => {
                            error!("dfu_split: FirmwareUpdateConfirm error {:?}", e);
                            break;
                        }
                        Either::Second(_) => {
                            error!("dfu_split: FirmwareUpdateConfirm timeout on confirm");
                            publish_event(crate::event::DfuStatusEvent::new(rmk_types::dfu::DfuStatus::Error));
                            break;
                        }
                    }
                }
            }
            crate::dfu::DfuCmd::SystemReset(crate::dfu::DfuTarget::ForwardPeripheral(id)) if id == self.id as u8 => {
                self.dfu_aborted = false;
                info!("dfu_split: forwarding system reset to peripheral {}", self.id);
                if self.send(&SplitMessage::SystemReset).await.is_err() {
                    error!("dfu_split: disconnected during system reset");
                }
            }
            _ => {} // Central command or other peripheral — skip
        }
    }

    /// Check if the peripheral's firmware is up to date and update if needed.
    ///
    /// Called once at connection start.  Depending on [`UpdatePolicy`]:
    ///
    /// * `MatchHash` — sends a `FirmwareHashQuery`, compares the
    ///   peripheral's response against the expected CRC-32, and only
    ///   flashes when they differ.
    /// * `Force` — skips the hash query entirely and always flashes.
    pub(crate) async fn check_firmware_update(&mut self) {
        use embassy_time::{Duration, Instant, Timer};

        let (firmware, expected_hash) = match crate::dfu::get_firmware_update_data(self.id) {
            Some(d) => d,
            None => {
                info!("dfu_split: no firmware data for peripheral {}", self.id);
                return;
            }
        };

        match self.policy {
            UpdatePolicy::Force => {
                info!("dfu_split: force update enabled, sending {} bytes", firmware.len());
                self.send_firmware_update(firmware, expected_hash).await;
                return;
            }
            UpdatePolicy::MatchHash => {}
        }

        info!("dfu_split: checking peripheral firmware...");
        if self.send(&SplitMessage::FirmwareHashQuery).await.is_err() {
            error!("dfu_split: disconnected during hash query");
            return;
        }

        let deadline = Instant::now() + Duration::from_secs(2);
        let hash = loop {
            match select(self.transceiver.read(), Timer::at(deadline)).await {
                Either::First(Ok(SplitMessage::FirmwareHashResponse(h))) => break Some(h),
                Either::First(Ok(_)) => {}
                Either::First(Err(e)) => {
                    error!("read error: {:?}", e);
                    break None;
                }
                Either::Second(_) => break None,
            }
        };

        let peripheral_hash = match hash {
            Some(h) => h,
            None => {
                info!("dfu_split: no hash, starting update");
                self.send_firmware_update(firmware, expected_hash).await;
                return;
            }
        };

        if peripheral_hash == expected_hash {
            info!("dfu_split: hash matches, no update needed");
            return;
        }

        info!("dfu_split: hash mismatch, starting update ({} bytes)", firmware.len());
        self.send_firmware_update(firmware, expected_hash).await;
    }

    /// Send the full firmware binary to the peripheral in SPLIT_CHUNK_SIZE-byte chunks.
    ///
    /// Each chunk is checked with per-chunk CRC-32 verification.  If a
    /// chunk fails (CRC mismatch or timeout) it is retried up to 3 times.
    /// The entire transfer is retried up to 3 attempts on failure.
    ///
    /// On success, the peripheral confirms and resets into the new
    /// firmware.
    async fn send_firmware_update(&mut self, firmware: &[u8], expected_hash: u32) {
        use embassy_time::{Duration, Instant, Timer};
        const MAX_RETRIES: u32 = 3;
        const MAX_ATTEMPTS: u32 = 3;

        for attempt in 1..=MAX_ATTEMPTS {
            info!("dfu_split: update attempt {}/{}", attempt, MAX_ATTEMPTS);
            publish_event(crate::event::DfuStatusEvent::new(rmk_types::dfu::DfuStatus::Started));

            let mut central_crc = crate::crc32::Crc32::new();
            let mut all_acked = true;

            for (offset, chunk) in firmware.chunks(crate::split::SPLIT_CHUNK_SIZE).enumerate() {
                let offset_bytes = (offset * crate::split::SPLIT_CHUNK_SIZE) as u32;
                let mut data = [0u8; crate::split::SPLIT_CHUNK_SIZE];
                data[..chunk.len()].copy_from_slice(chunk);
                let chunk_crc = crate::crc32::crc32(&data[..chunk.len()]);
                central_crc.update(&data[..chunk.len()]);

                let mut retries = 0;
                let mut acked = false;

                while !acked && retries < MAX_RETRIES {
                    if retries > 0 {
                        info!(
                            "dfu_split: retry {}/{} for chunk at offset {}",
                            retries + 1,
                            MAX_RETRIES,
                            offset_bytes
                        );
                    }

                    if self
                        .send(&SplitMessage::FirmwareChunk {
                            offset: offset_bytes,
                            len: chunk.len() as u16,
                            data: super::FirmwareChunkData(data),
                        })
                        .await
                        .is_err()
                    {
                        error!("dfu_split: disconnected during chunk send");
                        return;
                    }
                    publish_event(crate::event::DfuStatusEvent::new(
                        rmk_types::dfu::DfuStatus::Downloading,
                    ));

                    let deadline = Instant::now() + Duration::from_secs(2);
                    let got = loop {
                        match select(self.transceiver.read(), Timer::at(deadline)).await {
                            Either::First(Ok(SplitMessage::FirmwareChunkAck {
                                offset: ack_offset,
                                crc: ack_crc,
                            })) => {
                                if ack_offset == offset_bytes {
                                    if ack_crc == chunk_crc {
                                        break true;
                                    }
                                    warn!(
                                        "dfu_split: per-chunk CRC mismatch at offset {} (peripheral={:#010x}, central={:#010x})",
                                        offset_bytes, ack_crc, chunk_crc
                                    );
                                    break false;
                                }
                                info!(
                                    "dfu_split: got ack for offset {}, waiting for {}",
                                    ack_offset, offset_bytes
                                );
                            }
                            Either::First(Ok(other)) => warn!("dfu_split: unexpected message: {:?}", other),
                            Either::First(Err(e)) => {
                                error!("dfu_split: FirmwareChunkAck error {:?}", e);
                                break false;
                            }
                            Either::Second(_) => break false,
                        }
                    };
                    acked = got;
                    retries += 1;
                }

                if !acked {
                    error!(
                        "dfu_split: chunk at offset {} failed after {} retries",
                        offset_bytes, MAX_RETRIES
                    );
                    all_acked = false;
                    break;
                }
            }

            if !all_acked {
                continue;
            }

            let local_crc = central_crc.finalize();
            if local_crc != expected_hash {
                error!("dfu_split: central CRC mismatch — aborting");
                publish_event(crate::event::DfuStatusEvent::new(rmk_types::dfu::DfuStatus::Error));
                return;
            }

            if self.send(&SplitMessage::FirmwareUpdateComplete).await.is_err() {
                return;
            }

            let deadline = Instant::now() + Duration::from_secs(5);
            let peripheral_crc = loop {
                match select(self.transceiver.read(), Timer::at(deadline)).await {
                    Either::First(Ok(SplitMessage::FirmwareCrcReport(crc))) => break Some(crc),
                    Either::First(Ok(_)) => {}
                    Either::First(Err(e)) => {
                        error!("dfu_split: FirmwareCrcReport error {:?}", e);
                        break None;
                    }
                    Either::Second(_) => break None,
                }
            };

            let Some(dfu_crc) = peripheral_crc else {
                continue;
            };

            if dfu_crc == expected_hash {
                info!("dfu_split: end-to-end CRC matches, confirming");
                self.send(&SplitMessage::FirmwareCrcOk).await.ok();
                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    match select(self.transceiver.read(), Timer::at(deadline)).await {
                        Either::First(Ok(SplitMessage::FirmwareUpdateConfirm)) => {
                            info!("dfu_split: peripheral confirmed CRC, complete");
                            publish_event(crate::event::DfuStatusEvent::new(rmk_types::dfu::DfuStatus::Finished));
                            return;
                        }
                        Either::First(Ok(_)) => {}
                        Either::First(Err(e)) => {
                            error!("dfu_split: FirmwareCrcOk error {:?}", e);
                            return;
                        }
                        Either::Second(_) => {
                            error!("dfu_split: FirmwareCrcOk timeout");
                            publish_event(crate::event::DfuStatusEvent::new(rmk_types::dfu::DfuStatus::Error));
                            return;
                        }
                    }
                }
            } else {
                warn!("dfu_split: end-to-end CRC mismatch, retrying");
                self.send(&SplitMessage::FirmwareCrcFail).await.ok();
                Timer::after(Duration::from_millis(100)).await;
            }
        }

        error!("dfu_split: all {} update attempts failed", MAX_ATTEMPTS);
        publish_event(crate::event::DfuStatusEvent::new(rmk_types::dfu::DfuStatus::Error));
    }
}
