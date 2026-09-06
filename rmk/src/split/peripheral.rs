#[cfg(feature = "_ble")]
#[cfg(all(feature = "_ble", feature = "subrating"))]
use bt_hci::{cmd::le::LeSetHostFeature, controller::ControllerCmdSync};
use embassy_futures::select::{Either, select};
#[cfg(not(feature = "_ble"))]
use embedded_io_async::{Read, Write};
#[cfg(all(feature = "dfu_split", not(feature = "_ble")))]
use embedded_storage_async::nor_flash::NorFlash;
use futures::FutureExt;
#[cfg(all(feature = "_ble", feature = "storage"))]
use {super::ble::PeerAddress, crate::channel::FLASH_CHANNEL};
#[cfg(feature = "_ble")]
use {
    crate::event::{BatteryStatusEvent, ChargingStateEvent, EventSubscriber},
    rmk_types::battery::BatteryStatus,
    trouble_host::prelude::*,
};

use super::SplitMessage;
use super::driver::{SplitReader, SplitWriter};
use crate::event::{
    KeyboardEvent, LayerChangeEvent, LedIndicatorEvent, PointingEvent, SleepStateEvent, SubscribableEvent,
    publish_event,
};
#[cfg(feature = "display")]
use crate::event::{ModifierEvent, WpmUpdateEvent};
#[cfg(not(feature = "_ble"))]
use crate::split::serial::SerialSplitDriver;
use crate::state::update_status;

/// Run the split peripheral service. On BLE builds this owns the peripheral's
/// whole BLE stack, sized to its single link (the central) — nothing else
/// uses it.
///
/// On serial (`dfu_split`) builds the peripheral also takes its DFU download
/// and boot state partitions so over-the-split-link firmware updates can
/// write to them directly (no global registry).
///
/// # Arguments
///
/// * `id` - (optional) The id of the peripheral
/// * `controller` - (optional) The BLE controller
/// * `address` - (optional) The BLE address of this peripheral
/// * `serial` - (optional) serial port used to send peripheral split message. This argument is enabled only for serial split now
/// * `dfu_partition`/`state_partition` - (optional) the peripheral's DFU download and boot state partitions
pub async fn run_rmk_split_peripheral<
    #[cfg(all(feature = "_ble", feature = "subrating"))] C: Controller + ControllerCmdSync<LeSetHostFeature>,
    #[cfg(all(feature = "_ble", not(feature = "subrating")))] C: Controller,
    #[cfg(not(feature = "_ble"))] S: Write + Read,
    #[cfg(all(feature = "dfu_split", not(feature = "_ble")))] DFU: NorFlash + Clone,
    #[cfg(all(feature = "dfu_split", not(feature = "_ble")))] STATE: NorFlash + Clone,
>(
    #[cfg(feature = "_ble")] id: usize,
    #[cfg(feature = "_ble")] controller: C,
    #[cfg(feature = "_ble")] address: [u8; 6],
    #[cfg(not(feature = "_ble"))] serial: S,
    #[cfg(all(feature = "dfu_split", not(feature = "_ble")))] dfu_partition: DFU,
    #[cfg(all(feature = "dfu_split", not(feature = "_ble")))] state_partition: STATE,
) {
    #[cfg(not(feature = "_ble"))]
    {
        let mut peripheral = SplitPeripheral::new(SerialSplitDriver::new(serial));

        #[cfg(all(feature = "dfu_split", not(feature = "_ble")))]
        let mut dfu_handler = crate::dfu::FlashDfuHandler::new(dfu_partition, state_partition);
        #[cfg(all(feature = "dfu_split", not(feature = "_ble")))]
        dfu_handler.mark_booted().await;

        loop {
            peripheral
                .run(
                    #[cfg(all(feature = "dfu_split", not(feature = "_ble")))]
                    &mut dfu_handler,
                )
                .await;
        }
    }

    #[cfg(feature = "_ble")]
    {
        // Exactly one link — the central.
        let mut resources: HostResources<DefaultPacketPool, 1, 4> = HostResources::new();
        let stack = trouble_host::new(controller, &mut resources)
            .set_random_address(Address::random(address))
            .build();
        crate::split::ble::peripheral::initialize_nrf_ble_split_peripheral_and_run(id, &stack).await;
    }
}

/// The split peripheral instance.
pub(crate) struct SplitPeripheral<S: SplitWriter + SplitReader> {
    split_driver: S,
}

impl<S: SplitWriter + SplitReader> SplitPeripheral<S> {
    pub(crate) fn new(split_driver: S) -> Self {
        Self { split_driver }
    }

    /// Run the peripheral keyboard service.
    ///
    /// The peripheral uses the general matrix, does scanning and sends key events through `SplitWriter`.
    /// It also receives split messages from the central through `SplitReader`.
    pub(crate) async fn run<
        #[cfg(all(feature = "dfu_split", not(feature = "_ble")))] DFU: NorFlash + Clone,
        #[cfg(all(feature = "dfu_split", not(feature = "_ble")))] STATE: NorFlash + Clone,
    >(
        &mut self,
        #[cfg(all(feature = "dfu_split", not(feature = "_ble")))] dfu_handler: &mut crate::dfu::FlashDfuHandler<
            DFU,
            STATE,
        >,
    ) {
        // Proactively announce our firmware hash so the central can detect
        // us even when it booted first and already gave up waiting for a query response.
        #[cfg(feature = "dfu_split")]
        {
            let hash = crate::dfu::read_embedded_firmware_hash();
            info!("dfu_split: announcing firmware hash {:#x}", hash);
            self.split_driver
                .write(&SplitMessage::FirmwareHashResponse(hash))
                .await
                .ok();
        }

        let mut key_sub = KeyboardEvent::subscriber();
        #[cfg(feature = "_ble")]
        let mut charging_state_sub = ChargingStateEvent::subscriber();
        let mut pointing_sub = PointingEvent::subscriber();
        #[cfg(feature = "_ble")]
        let mut battery_sub = BatteryStatusEvent::subscriber();

        loop {
            let read_message_to_send = async {
                crate::select_biased_with_feature! {
                    e = key_sub.next_message_pure().fuse() => SplitMessage::Key(e),
                    with_feature("_ble"): e = charging_state_sub.next_message_pure().fuse() => {
                        SplitMessage::BatteryStatus(BatteryStatus::Available {
                            charge_state: e.charging.into(),
                            level: None,
                        }.into())
                    },
                    e = pointing_sub.next_message_pure().fuse() => SplitMessage::Pointing(e),
                    with_feature("_ble"): e = battery_sub.next_event().fuse() => SplitMessage::BatteryStatus(e),
                }
            };

            match select(self.split_driver.read(), read_message_to_send).await {
                Either::First(m) => match m {
                    // Process split messages from the central
                    Ok(split_message) => match split_message {
                        SplitMessage::ConnectionStatus(status) => {
                            trace!("Received central connection status: {:?}", status);
                            update_status(|c| *c = status);
                            // The central sends this only after subscribing to split notifications.
                            #[cfg(feature = "_ble")]
                            self.split_driver
                                .write(&SplitMessage::BatteryStatus(
                                    crate::input_device::battery::current_battery_status().into(),
                                ))
                                .await
                                .ok();
                        }
                        #[cfg(all(feature = "_ble", feature = "storage"))]
                        SplitMessage::ClearPeer => {
                            // Clear the peer address
                            FLASH_CHANNEL
                                .send(crate::storage::FlashOperationMessage::PeerAddress(PeerAddress::new(
                                    0, false, [0; 6],
                                )))
                                .await;
                        }
                        SplitMessage::KeyboardIndicator(indicator) => {
                            // Publish KeyboardIndicator event
                            publish_event(LedIndicatorEvent::new(
                                rmk_types::led_indicator::LedIndicator::from_bits(indicator),
                            ));
                        }
                        SplitMessage::Layer(layer) => {
                            // Publish Layer event
                            publish_event(LayerChangeEvent::new(layer));
                        }
                        #[cfg(feature = "display")]
                        SplitMessage::Wpm(wpm) => publish_event(WpmUpdateEvent::new(wpm)),
                        #[cfg(feature = "display")]
                        SplitMessage::Modifier(bits) => {
                            publish_event(ModifierEvent {
                                modifier: rmk_types::modifier::ModifierCombination::from_bits(bits),
                            });
                        }
                        SplitMessage::SleepState(sleeping) => {
                            publish_event(SleepStateEvent::new(sleeping));
                        }
                        // --- dfu_split: firmware update handlers ---
                        #[cfg(feature = "dfu_split")]
                        SplitMessage::FirmwareHashQuery => {
                            let hash = crate::dfu::read_embedded_firmware_hash();
                            info!("dfu_split: hash query, responding with {:#x}", hash);
                            self.split_driver
                                .write(&SplitMessage::FirmwareHashResponse(hash))
                                .await
                                .ok();
                        }
                        #[cfg(feature = "dfu_split")]
                        SplitMessage::FirmwareChunk { offset, len, data } => {
                            let actual_len = len as usize;
                            let chunk_data = &data.0[..actual_len];
                            match dfu_handler.write_chunk(offset as u32, chunk_data).await {
                                Ok(()) => {
                                    debug!("dfu_split: wrote {} bytes at offset {}", actual_len, offset);
                                    let ack = SplitMessage::FirmwareChunkAck {
                                        offset,
                                        crc: crate::crc32::crc32(chunk_data),
                                    };
                                    self.split_driver.write(&ack).await.ok();
                                }
                                Err(()) => error!("dfu_split: write error at offset {}", offset),
                            }
                        }
                        #[cfg(feature = "dfu_split")]
                        SplitMessage::FirmwareUpdateComplete => {
                            let dfu_crc = match dfu_handler.compute_dfu_crc().await {
                                Ok(crc) => crc,
                                Err(()) => {
                                    // No CRC report: the central's verification
                                    // times out and aborts the update, so a flash
                                    // that cannot be read back never gets booted.
                                    error!("dfu_split: reading back DFU partition failed, aborting verification");
                                    continue;
                                }
                            };
                            info!("dfu_split: DFU partition CRC: {:#010x}", dfu_crc);
                            let crc_msg = SplitMessage::FirmwareCrcReport(dfu_crc);
                            self.split_driver.write(&crc_msg).await.ok();
                            info!("dfu_split: CRC report sent");

                            let deadline = embassy_time::Instant::now() + embassy_time::Duration::from_secs(5);
                            let ok = loop {
                                match select(self.split_driver.read(), embassy_time::Timer::at(deadline)).await {
                                    Either::First(Ok(SplitMessage::FirmwareCrcOk)) => {
                                        info!("dfu_split: central confirmed CRC, resetting");
                                        break true;
                                    }
                                    Either::First(Ok(SplitMessage::FirmwareCrcFail)) => {
                                        warn!("dfu_split: central rejected CRC, stopping update");
                                        break false;
                                    }
                                    Either::First(Ok(_)) => {}
                                    Either::First(Err(e)) => {
                                        error!("read error: {:?}", e);
                                        break false;
                                    }
                                    Either::Second(_) => {
                                        error!("timeout");
                                        break false;
                                    }
                                }
                            };

                            if ok {
                                self.split_driver.write(&SplitMessage::FirmwareUpdateConfirm).await.ok();
                                embassy_time::Timer::after_millis(50).await;
                                dfu_handler.mark_updated_and_reset().await.ok();
                            }
                        }
                        #[cfg(feature = "dfu_split")]
                        SplitMessage::SystemReset => {
                            info!("dfu_split: received system reset from central");
                            cortex_m::peripheral::SCB::sys_reset();
                        }
                        _ => (),
                    },
                    Err(e) => {
                        error!("Split message read error: {:?}", e);
                        if let crate::split::driver::SplitDriverError::Disconnected = e {
                            break;
                        }
                    }
                },
                Either::Second(e) => {
                    debug!("Writing split message {:?} to central", e);
                    self.split_driver.write(&e).await.ok();
                }
            }
        }
    }
}
