//! The abstracted driver layer of the split keyboard.
//!
use core::cell::Cell;

use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use futures::FutureExt;
use rmk_types::battery::BatteryStatus;
#[cfg(feature = "rynk")]
use rmk_types::protocol::rynk::PeripheralStatus;

use super::{PeripheralMatrixConfig, SplitMessage};
#[cfg(feature = "_ble")]
use crate::event::{BatteryStatusEvent, PeripheralBatteryEvent};
use crate::event::{
    KeyboardEvent, KeyboardEventPos, PeripheralConnectedEvent, SubscribableEvent, publish_event, publish_event_async,
};

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum SplitDriverError {
    SerialError,
    EmptyMessage,
    DeserializeError,
    SerializeError,
    BleError(u8),
    Disconnected,
}

/// Split message reader from other split devices
pub(crate) trait SplitReader {
    async fn read(&mut self) -> Result<SplitMessage, SplitDriverError>;
}

/// Split message writer to other split devices
pub(crate) trait SplitWriter {
    async fn write(&mut self, message: &SplitMessage) -> Result<usize, SplitDriverError>;
}

/// Live per-peripheral status. Latched here in the transport-agnostic split
/// layer so host services can read a current snapshot at any time, even when
/// no host session was active when the change happened. Wired peripherals
/// never report a battery, so theirs stays `Unavailable`.
#[derive(Copy, Clone, PartialEq, Eq)]
struct PeripheralSlot {
    connected: bool,
    battery: BatteryStatus,
}

static PERIPHERAL_SLOTS: BlockingMutex<crate::RawMutex, Cell<[PeripheralSlot; crate::SPLIT_PERIPHERALS_NUM]>> =
    BlockingMutex::new(Cell::new(
        [PeripheralSlot {
            connected: false,
            battery: BatteryStatus::Unavailable,
        }; crate::SPLIT_PERIPHERALS_NUM],
    ));

/// Read-modify-write peripheral `id`'s slot. Returns `false` when `id` is out
/// of range or the slot didn't change, so callers skip publishing.
fn update_slot(id: usize, f: impl FnOnce(&mut PeripheralSlot)) -> bool {
    PERIPHERAL_SLOTS.lock(|slots| {
        let mut all = slots.get();
        let Some(slot) = all.get_mut(id) else {
            return false;
        };
        let prev = *slot;
        f(slot);
        if *slot == prev {
            return false;
        }
        slots.set(all);
        true
    })
}

/// Latch peripheral `id`'s connected state and broadcast the change.
pub(crate) fn set_peripheral_connected(id: usize, connected: bool) {
    if update_slot(id, |s| s.connected = connected) {
        publish_event(PeripheralConnectedEvent { id, connected });
    }
}

/// Latch peripheral `id`'s battery status and broadcast the change.
#[cfg(feature = "_ble")]
pub(crate) fn set_peripheral_battery(id: usize, battery: BatteryStatus) {
    if update_slot(id, |s| s.battery = battery) {
        publish_event(PeripheralBatteryEvent {
            id,
            state: BatteryStatusEvent(battery),
        });
    }
}

/// Latest battery status reported by peripheral `id`.
#[cfg(feature = "_ble")]
pub(crate) fn current_peripheral_battery_status(id: usize) -> Option<BatteryStatus> {
    PERIPHERAL_SLOTS.lock(|slots| slots.get().get(id).map(|slot| slot.battery))
}

/// Latest snapshot for peripheral `id`, or `None` when `id` is out of range.
#[cfg(feature = "rynk")]
pub(crate) fn current_peripheral_status(id: usize) -> Option<PeripheralStatus> {
    PERIPHERAL_SLOTS.lock(|slots| {
        slots.get().get(id).map(|s| PeripheralStatus {
            connected: s.connected,
            battery: s.battery,
        })
    })
}

#[cfg(all(test, feature = "_ble"))]
mod tests {
    use rmk_types::battery::ChargeState;

    use super::{current_peripheral_battery_status, set_peripheral_battery};

    #[test]
    fn caches_latest_peripheral_battery_status() {
        let status = rmk_types::battery::BatteryStatus::Available {
            charge_state: ChargeState::Discharging,
            level: Some(73),
        };

        set_peripheral_battery(0, status);

        assert_eq!(current_peripheral_battery_status(0), Some(status));
        assert_eq!(current_peripheral_battery_status(crate::SPLIT_PERIPHERALS_NUM), None);
    }
}

/// PeripheralManager runs in central.
/// It reads split message from peripheral and updates key matrix cache of the peripheral.
///
/// When the central scans the matrix, the scanning thread sends sync signal and gets key state cache back.
///
pub(crate) struct PeripheralManager<T: SplitReader + SplitWriter> {
    /// Receiver
    transceiver: T,
    /// Peripheral id
    id: usize,
    /// This peripheral's matrix size and placement in the central's keymap
    matrix_config: PeripheralMatrixConfig,
    #[cfg(feature = "dfu_split")]
    passthrough_crc: crate::crc32::Crc32,
    /// Whether to skip hash comparison and always flash firmware.
    #[cfg(feature = "dfu_split")]
    policy: UpdatePolicy,
}

/// Defines how the central decides whether to flash a peripheral.
#[cfg(feature = "dfu_split")]
#[derive(Clone, Copy)]
pub enum UpdatePolicy {
    /// Compare the firmware hash — only flash when it differs.
    MatchHash,
    /// Always flash the firmware regardless of the current version.
    Force,
}

impl<T: SplitReader + SplitWriter> PeripheralManager<T> {
    pub(crate) fn new(
        transceiver: T,
        id: usize,
        matrix_config: PeripheralMatrixConfig,
        #[cfg(feature = "dfu_split")] policy: UpdatePolicy,
    ) -> Self {
        Self {
            transceiver,
            matrix_config,
            id,
            #[cfg(feature = "dfu_split")]
            passthrough_crc: crate::crc32::Crc32::new(),
            #[cfg(feature = "dfu_split")]
            policy,
        }
    }

    /// Send a message to the peripheral, returning Err on disconnect.
    async fn send(&mut self, msg: &SplitMessage) -> Result<(), ()> {
        debug!("Sending message to peripheral {}: {:?}", self.id, msg);
        match self.transceiver.write(msg).await {
            Ok(_) => Ok(()),
            Err(SplitDriverError::Disconnected) => Err(()),
            Err(e) => {
                error!("SplitDriver write error: {:?}", e);
                Ok(())
            }
        }
    }

    /// Run the manager.
    ///
    /// The manager receives from the peripheral and publishes input events.
    /// It also syncs the central's `ConnectionStatus` to the peripheral on every
    /// change as an informational signal
    pub(crate) async fn run(mut self) {
        use crate::event::EventSubscriber;

        let mut indicator_sub = crate::event::LedIndicatorEvent::subscriber();
        let mut layer_sub = crate::event::LayerChangeEvent::subscriber();
        // Subscribe before the initial send so any change racing past the
        // snapshot is still delivered to us.
        let mut connection_sub = crate::event::ConnectionStatusChangeEvent::subscriber();
        #[cfg(feature = "_ble")]
        let mut clear_peer_sub = crate::event::ClearPeerEvent::subscriber();
        #[cfg(feature = "display")]
        let mut wpm_sub = crate::event::WpmUpdateEvent::subscriber();
        #[cfg(feature = "display")]
        let mut modifier_sub = crate::event::ModifierEvent::subscriber();
        #[cfg(feature = "display")]
        let mut sleep_sub = crate::event::SleepStateEvent::subscriber();

        // Send the current state once on startup so the peripheral matches us
        // even when no transition has happened since the central booted.
        if self
            .send(&SplitMessage::ConnectionStatus(
                crate::state::current_connection_status(),
            ))
            .await
            .is_err()
        {
            return;
        }

        #[cfg(feature = "dfu_split")]
        self.check_firmware_update().await;

        loop {
            #[cfg(feature = "dfu_split")]
            if crate::dfu::passthrough_pending(self.id) {
                self.handle_passthrough().await;
                continue;
            }

            // Use select_biased_with_feature to handle feature-gated subscriber arms
            let next_event_to_peri = async {
                crate::select_biased_with_feature! {
                    e = indicator_sub.next_event().fuse() => SplitMessage::KeyboardIndicator(e.0.into_bits()),
                    e = layer_sub.next_event().fuse() => SplitMessage::Layer(e.0),
                    e = connection_sub.next_event().fuse() => SplitMessage::ConnectionStatus(e.0),
                    with_feature("_ble"): _ = clear_peer_sub.next_event().fuse() => {
                        #[cfg(feature = "storage")]
                        {
                            use {crate::channel::FLASH_CHANNEL, crate::split::ble::PeerAddress, crate::storage::FlashOperationMessage};
                            FLASH_CHANNEL
                                .send(FlashOperationMessage::PeerAddress(PeerAddress::new(self.id as u8, false, [0; 6])))
                                .await;
                        }
                        SplitMessage::ClearPeer
                    },
                    with_feature("display"): e = wpm_sub.next_event().fuse() => SplitMessage::Wpm(e.0),
                    with_feature("display"): e = modifier_sub.next_event().fuse() => SplitMessage::Modifier(e.modifier.into_bits()),
                    with_feature("display"): e = sleep_sub.next_event().fuse() => SplitMessage::SleepState(e.0),
                }
            };

            #[cfg(feature = "dfu_split")]
            let event_or_signal = select(next_event_to_peri, crate::dfu::PASSTHROUGH_SIGNAL.wait());
            #[cfg(not(feature = "dfu_split"))]
            let event_or_signal = next_event_to_peri;

            match select(self.transceiver.read(), event_or_signal).await {
                Either::First(read_result) => match read_result {
                    #[cfg(feature = "dfu_split")]
                    Ok(SplitMessage::FirmwareHashResponse(hash)) => {
                        self.handle_proactive_hash(hash).await;
                    }
                    Ok(split_message) => self.process_peripheral_message(split_message).await,
                    Err(e) => error!("Peripheral message read error: {:?}", e),
                },
                #[cfg(feature = "dfu_split")]
                Either::Second(result) => match result {
                    Either::First(msg) => {
                        if self.send(&msg).await.is_err() {
                            return;
                        }
                    }
                    Either::Second(_) => {}
                },
                #[cfg(not(feature = "dfu_split"))]
                Either::Second(msg) => {
                    if self.send(&msg).await.is_err() {
                        return;
                    }
                }
            }
        }
    }

    /// Process a single message from the peripheral.
    async fn process_peripheral_message(&self, split_message: SplitMessage) {
        trace!("Got message from peripheral: {:?}", split_message);
        match split_message {
            SplitMessage::Key(e) => match e.pos {
                KeyboardEventPos::Key(key_pos) => {
                    // Verify the row/col
                    if key_pos.row >= self.matrix_config.rows || key_pos.col >= self.matrix_config.cols {
                        error!("Invalid peripheral row/col: {} {}", key_pos.row, key_pos.col);
                        return;
                    }
                    publish_event_async(KeyboardEvent::key(
                        key_pos.row + self.matrix_config.row_offset,
                        key_pos.col + self.matrix_config.col_offset,
                        e.pressed,
                    ))
                    .await;
                }
                _ => publish_event_async(e).await,
            },
            // Non-key events are drop-on-full to keep the split read loop responsive.
            SplitMessage::Pointing(e) => publish_event(e),
            #[cfg(feature = "_ble")]
            SplitMessage::BatteryStatus(state) => set_peripheral_battery(self.id, state.0),
            #[cfg(feature = "dfu_split")]
            SplitMessage::FirmwareHashResponse(hash) => {
                info!("dfu_split: stale hash response ({:#x}) in event loop", hash);
            }
            #[cfg(feature = "dfu_split")]
            SplitMessage::FirmwareChunkAck { offset, crc: _ } => {
                info!("dfu_split: stale chunk ack (offset {}) in event loop, ignoring", offset);
            }
            #[cfg(feature = "dfu_split")]
            SplitMessage::FirmwareUpdateConfirm => {
                info!("dfu_split: stale update confirm in event loop, ignoring");
            }
            _ => warn!("{:?} should not come from peripheral", split_message),
        }
    }

    /// Handle a proactive `FirmwareHashResponse` received in the main event
    /// loop (after the initial `check_firmware_update` may have timed out
    /// because the peripheral was not yet booted).
    #[cfg(feature = "dfu_split")]
    async fn handle_proactive_hash(&mut self, hash: u32) {
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

    /// Process passthrough DFU chunks (fire-and-forget with per-chunk ack).
    ///
    /// Called from the event loop when [`passthrough_pending`] returns
    /// `true`.  Drains the entire `PASSTHROUGH_CMD` queue, forwarding
    /// each chunk over the split link and waiting for a
    /// `FirmwareChunkAck` before proceeding to the next.
    ///
    /// On `Finish`, triggers end-to-end CRC verification: the peripheral
    /// reads back its DFU partition, sends the CRC-32, the central
    /// compares, and sends `FirmwareCrcOk` / `FirmwareCrcFail`.
    #[cfg(feature = "dfu_split")]
    async fn handle_passthrough(&mut self) {
        use embassy_time::{Duration, Instant, Timer};

        while let Some(cmd) = crate::dfu::passthrough_take_command() {
            match cmd {
                crate::dfu::PassthroughCommand::Chunk(chunk) => {
                    debug!(
                        "dfu_split/passthrough: sending chunk @ offset {} ({} bytes)",
                        chunk.offset, chunk.len
                    );
                    self.passthrough_crc.update(&chunk.data[..chunk.len as usize]);
                    let msg = SplitMessage::FirmwareChunk {
                        offset: chunk.offset,
                        len: chunk.len,
                        data: super::FirmwareChunkData(chunk.data),
                    };
                    if self.send(&msg).await.is_err() {
                        error!("dfu_split/passthrough: disconnected during chunk send");
                        crate::dfu::passthrough_done_if_empty();
                        return;
                    }

                    // Wait for the peripheral to acknowledge this chunk
                    let deadline = Instant::now() + Duration::from_secs(2);
                    loop {
                        match select(self.transceiver.read(), Timer::at(deadline)).await {
                            Either::First(Ok(SplitMessage::FirmwareChunkAck { offset, .. }))
                                if offset == chunk.offset =>
                            {
                                break;
                            }
                            Either::First(Ok(_)) => {}
                            Either::First(Err(e)) => {
                                error!("dfu_split/passthrough: read error: {:?}", e);
                                break;
                            }
                            Either::Second(_) => {
                                error!("dfu_split/passthrough: timeout waiting for chunk ack");
                                break;
                            }
                        }
                    }

                    crate::dfu::passthrough_done_if_empty();
                }
                crate::dfu::PassthroughCommand::Finish => {
                    info!("dfu_split/passthrough: DFU download complete, starting end-to-end verification");

                    if self.send(&SplitMessage::FirmwareUpdateComplete).await.is_err() {
                        error!("dfu_split/passthrough: disconnected during finish");
                        crate::dfu::passthrough_done_if_empty();
                        return;
                    }

                    let deadline = Instant::now() + Duration::from_secs(5);
                    let crc = loop {
                        match select(self.transceiver.read(), Timer::at(deadline)).await {
                            Either::First(Ok(SplitMessage::FirmwareCrcReport(crc))) => break Some(crc),
                            Either::First(Ok(_)) => {}
                            Either::First(Err(e)) => {
                                error!("dfu_split/passthrough: read error: {:?}", e);
                                break None;
                            }
                            Either::Second(_) => {
                                error!("dfu_split/passthrough: timeout waiting for CRC");
                                break None;
                            }
                        }
                    };

                    let Some(peripheral_crc) = crc else {
                        error!("dfu_split/passthrough: CRC verification failed");
                        self.send(&SplitMessage::FirmwareCrcFail).await.ok();
                        crate::dfu::passthrough_done_if_empty();
                        return;
                    };

                    let central_crc = self.passthrough_crc.finalize();
                    self.passthrough_crc = crate::crc32::Crc32::new();

                    if central_crc != peripheral_crc {
                        error!(
                            "dfu_split/passthrough: CRC mismatch (central={:#010x}, peripheral={:#010x})",
                            central_crc, peripheral_crc
                        );
                        self.send(&SplitMessage::FirmwareCrcFail).await.ok();
                        crate::dfu::passthrough_done_if_empty();
                        return;
                    }

                    info!("dfu_split/passthrough: CRC OK, confirming update");
                    if self.send(&SplitMessage::FirmwareCrcOk).await.is_err() {
                        error!("dfu_split/passthrough: disconnected during CRC OK");
                        crate::dfu::passthrough_done_if_empty();
                        return;
                    }

                    let deadline = Instant::now() + Duration::from_secs(2);
                    loop {
                        match select(self.transceiver.read(), Timer::at(deadline)).await {
                            Either::First(Ok(SplitMessage::FirmwareUpdateConfirm)) => {
                                info!("dfu_split/passthrough: peripheral confirmed, update complete");
                                break;
                            }
                            Either::First(Ok(_)) => {}
                            Either::First(Err(e)) => {
                                error!("dfu_split: FirmwareUpdateConfirm error {:?}", e);
                                break;
                            }
                            Either::Second(_) => {
                                info!("dfu_split: FirmwareUpdateConfirm timeout on confirm");
                                break;
                            }
                        }
                    }

                    crate::dfu::passthrough_done_if_empty();
                }
            }
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
    #[cfg(feature = "dfu_split")]
    async fn check_firmware_update(&mut self) {
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

    /// Send the full firmware binary to the peripheral in 256-byte chunks.
    ///
    /// Each chunk is checked with per-chunk CRC-32 verification.  If a
    /// chunk fails (CRC mismatch or timeout) it is retried up to 3 times.
    /// The entire transfer is retried up to 3 attempts on failure.
    ///
    /// On success, the peripheral confirms and resets into the new
    /// firmware.
    #[cfg(feature = "dfu_split")]
    async fn send_firmware_update(&mut self, firmware: &[u8], expected_hash: u32) {
        use embassy_time::{Duration, Instant, Timer};
        const MAX_RETRIES: u32 = 3;
        const MAX_ATTEMPTS: u32 = 3;

        for attempt in 1..=MAX_ATTEMPTS {
            info!("dfu_split: update attempt {}/{}", attempt, MAX_ATTEMPTS);
            publish_event(crate::event::DfuStatusEvent::new(rmk_types::dfu::DfuStatus::Started));

            let mut central_crc = crate::crc32::Crc32::new();
            let mut all_acked = true;

            for (offset, chunk) in firmware.chunks(256).enumerate() {
                let offset_bytes = (offset * 256) as u32;
                let mut data = [0u8; 256];
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
    }
}
