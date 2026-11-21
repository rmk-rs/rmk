#[cfg(feature = "host_lock")]
use core::cell::Cell;

use byteorder::{BigEndian, ByteOrder, LittleEndian};
use embassy_time::Instant;
use embedded_io_async::{Read, Write};
use rmk_types::protocol::vial::{VIA_FIRMWARE_VERSION, VIA_PROTOCOL_VERSION, ViaCommand, ViaKeyboardInfo};
use vial::process_vial;

use crate::config::{RmkConfig, VialConfig};
use crate::hid::ViaReport;
use crate::host::context::KeyboardContext;
use crate::host::via::keycode_convert::{from_via_keycode, to_via_keycode};
use crate::keymap::KeyMap;
use crate::{MACRO_SPACE_SIZE, MAX_MACROS};

pub(crate) mod keycode_convert;
mod vial;

/// The const flag lets peripheral firmware omit keymap handlers from its image.
pub struct VialService<'a, const HAS_KEYMAP: bool = true> {
    ctx: Option<KeyboardContext<'a>>,
    #[cfg(feature = "host_lock")]
    keys: &'a dyn crate::host::lock::UnlockKeyState,
    #[cfg(feature = "host_lock")]
    unlocked: Cell<bool>,
    vial_config: VialConfig<'static>,
}

impl<'a> VialService<'a> {
    pub fn new(keymap: &'a KeyMap<'a>, config: &RmkConfig<'static>) -> Self {
        Self {
            ctx: Some(KeyboardContext::new(keymap)),
            #[cfg(feature = "host_lock")]
            keys: keymap,
            #[cfg(feature = "host_lock")]
            unlocked: Cell::new(false),
            vial_config: config.vial_config,
        }
    }
}

#[cfg(all(feature = "host_lock", feature = "split", any(not(feature = "_no_usb"), test)))]
impl<'a> VialService<'a, false> {
    pub(crate) fn new_peripheral(keys: &'a dyn crate::host::lock::UnlockKeyState, config: &RmkConfig<'static>) -> Self {
        Self {
            ctx: None,
            keys,
            unlocked: Cell::new(false),
            vial_config: config.vial_config,
        }
    }
}

impl<const HAS_KEYMAP: bool> VialService<'_, HAS_KEYMAP> {
    async fn process_via_packet(
        &self,
        report: &mut ViaReport,
        #[cfg(feature = "host_lock")] locker: &crate::host::lock::HostLock<'_>,
    ) {
        let command_id = report.output_data[0];
        match command_id.into() {
            ViaCommand::GetProtocolVersion => {
                BigEndian::write_u16(&mut report.input_data[1..3], VIA_PROTOCOL_VERSION);
                return;
            }
            ViaCommand::BootloaderJump => {
                #[cfg(feature = "host_lock")]
                if !locker.is_unlocked() {
                    report.input_data[0] = ViaCommand::Unhandled as u8;
                    return;
                }
                crate::boot::jump_to_bootloader();
                return;
            }
            ViaCommand::Vial => {
                process_vial::<HAS_KEYMAP>(
                    report,
                    &self.vial_config,
                    #[cfg(feature = "host_lock")]
                    locker,
                    self.ctx.as_ref(),
                )
                .await;
                return;
            }
            _ => {}
        }
        if !HAS_KEYMAP {
            report.input_data[0] = ViaCommand::Unhandled as u8;
            return;
        }
        let Some(ctx) = &self.ctx else {
            report.input_data[0] = ViaCommand::Unhandled as u8;
            return;
        };

        // Caller pre-fills `input_data` from `output_data`, so individual arms
        // only need to overwrite the bytes they actually change.
        match command_id.into() {
            ViaCommand::GetKeyboardValue => {
                // Check the second u8
                match report.output_data[1].try_into() {
                    Ok(v) => match v {
                        ViaKeyboardInfo::Uptime => {
                            let value = Instant::now().as_millis() as u32;
                            BigEndian::write_u32(&mut report.input_data[2..6], value);
                        }
                        ViaKeyboardInfo::LayoutOptions => {
                            let layout_option = ctx.layout_options();
                            BigEndian::write_u32(&mut report.input_data[2..6], layout_option);
                        }
                        #[cfg(not(feature = "host_lock"))]
                        ViaKeyboardInfo::SwitchMatrixState => {
                            error!("It is not secure to use matrix tester without vial lock");
                        }
                        #[cfg(feature = "host_lock")]
                        ViaKeyboardInfo::SwitchMatrixState if locker.is_unlocked() => {
                            let bitmap = &mut report.input_data[2..];
                            ctx.read_matrix_state(bitmap);
                            // Vial wants each row's bytes big-endian (QMK matrix_row_t order).
                            let (rows, cols, _) = ctx.keymap_dimensions();
                            let row_len = cols.div_ceil(8);
                            if row_len > 1 {
                                let len = (rows * row_len).min(bitmap.len());
                                for row in bitmap[..len].chunks_mut(row_len) {
                                    row.reverse();
                                }
                            }
                        }
                        ViaKeyboardInfo::FirmwareVersion => {
                            BigEndian::write_u32(&mut report.input_data[2..6], VIA_FIRMWARE_VERSION);
                        }
                        _ => (),
                    },
                    Err(e) => error!("Invalid subcommand: {} of GetKeyboardValue", e),
                }
            }
            ViaCommand::SetKeyboardValue => {
                // Check the second u8
                match report.output_data[1].try_into() {
                    Ok(v) => match v {
                        ViaKeyboardInfo::LayoutOptions => {
                            let layout_option = BigEndian::read_u32(&report.output_data[2..6]);
                            let _ = ctx.set_layout_options(layout_option).await;
                        }
                        ViaKeyboardInfo::DeviceIndication => {
                            let _device_indication = report.output_data[2];
                            warn!("SetKeyboardValue - DeviceIndication")
                        }
                        _ => (),
                    },
                    Err(e) => error!("Invalid subcommand: {} of GetKeyboardValue", e),
                }
            }
            ViaCommand::DynamicKeymapGetKeyCode => {
                let layer = report.output_data[1];
                let row = report.output_data[2];
                let col = report.output_data[3];
                let action = ctx.get_action(layer, row, col);
                let keycode = to_via_keycode(action);
                info!("Getting keycode: {:02X} at ({},{}), layer {}", keycode, row, col, layer);
                BigEndian::write_u16(&mut report.input_data[4..6], keycode);
            }
            ViaCommand::DynamicKeymapSetKeyCode => {
                let layer = report.output_data[1];
                let row = report.output_data[2];
                let col = report.output_data[3];
                let keycode = BigEndian::read_u16(&report.output_data[4..6]);
                let action = from_via_keycode(keycode);
                info!(
                    "Setting keycode: 0x{:02X} at ({},{}), layer {} as {:?}",
                    keycode, row, col, layer, action
                );
                let _ = ctx.set_action(layer, row, col, action).await;
            }
            ViaCommand::DynamicKeymapReset => {
                warn!("Dynamic keymap reset -- not supported")
            }
            ViaCommand::CustomSetValue => {
                // backlight/rgblight/rgb matrix/led matrix/audio settings here
                warn!("Custom set value -- not supported")
            }
            ViaCommand::CustomGetValue => {
                // backlight/rgblight/rgb matrix/led matrix/audio settings here
                warn!("Custom get value -- not supported")
            }
            ViaCommand::CustomSave => {
                // backlight/rgblight/rgb matrix/led matrix/audio settings here
                warn!("Custom get value -- not supported")
            }
            ViaCommand::EepromReset => {
                warn!("Resetting storage..");
                ctx.reset_storage().await;
                // TODO: Reboot after a eeprom reset?
            }
            ViaCommand::DynamicKeymapMacroGetCount => {
                report.input_data[1] = MAX_MACROS as u8;
            }
            ViaCommand::DynamicKeymapMacroGetBufferSize => {
                BigEndian::write_u16(&mut report.input_data[1..3], MACRO_SPACE_SIZE as u16);
            }
            ViaCommand::DynamicKeymapMacroGetBuffer => {
                let offset = BigEndian::read_u16(&report.output_data[1..3]) as usize;
                let size = report.output_data[3] as usize;
                // The payload is `[4..4 + size]` of a 32-byte report.
                if size <= 28 {
                    let out = &mut report.input_data[4..4 + size];
                    ctx.keymap.macros(|m| {
                        let bytes = m.bytes().get(offset..).unwrap_or(&[]);
                        let n = bytes.len().min(size);
                        out[..n].copy_from_slice(&bytes[..n]);
                        out[n..].fill(0);
                    });
                } else {
                    report.input_data[0] = 0xFF;
                }
            }
            ViaCommand::DynamicKeymapMacroSetBuffer => {
                let offset = BigEndian::read_u16(&report.output_data[1..3]) as usize;
                let size = report.output_data[3] as usize;
                if size <= 28 {
                    let changed = ctx.keymap.macros(|m| m.write(offset, &report.output_data[4..4 + size]));
                    #[cfg(feature = "storage")]
                    if crate::keyboard::macros::persist(ctx.keymap, changed).await.is_err() {
                        error!("Failed to save the macros");
                    }
                    #[cfg(not(feature = "storage"))]
                    if !changed.is_empty() {
                        warn!("Macros changed but not saved: there is no storage");
                    }
                } else {
                    report.input_data[0] = 0xFF;
                }
            }
            ViaCommand::DynamicKeymapMacroReset => {
                let changed = ctx.keymap.macros(|m| m.clear());
                #[cfg(feature = "storage")]
                if crate::keyboard::macros::persist(ctx.keymap, changed).await.is_err() {
                    error!("Failed to save the macros");
                }
                #[cfg(not(feature = "storage"))]
                if !changed.is_empty() {
                    warn!("Macros changed but not saved: there is no storage");
                }
            }
            ViaCommand::DynamicKeymapGetLayerCount => {
                report.input_data[1] = ctx.keymap_dimensions().2 as u8;
            }
            ViaCommand::DynamicKeymapGetBuffer => {
                let offset = BigEndian::read_u16(&report.output_data[1..3]);
                // size <= 28
                let size = report.output_data[3];
                debug!("Getting keymap buffer, offset: {}, size: {}", offset, size);
                let mut idx = 4;
                let start = (offset / 2) as usize;
                let count = (size / 2) as usize;
                for i in 0..count {
                    let a = ctx.get_action_flat(start + i);
                    let kc = to_via_keycode(a);
                    BigEndian::write_u16(&mut report.input_data[idx..idx + 2], kc);
                    idx += 2;
                }
            }
            ViaCommand::DynamicKeymapSetBuffer => {
                debug!("Dynamic keymap set buffer");
                let offset = BigEndian::read_u16(&report.output_data[1..3]);
                // size <= 28
                let size = report.output_data[3];
                let mut idx = 4;
                let (rows, cols, _) = ctx.keymap_dimensions();
                for i in 0..(size as usize) {
                    let via_keycode = LittleEndian::read_u16(&report.output_data[idx..idx + 2]);
                    let action = from_via_keycode(via_keycode);
                    let flat_index = offset as usize + i;
                    let (layer, in_layer) = (flat_index / (rows * cols), flat_index % (rows * cols));
                    let _ = ctx
                        .set_action(layer as u8, (in_layer / cols) as u8, (in_layer % cols) as u8, action)
                        .await;
                    idx += 2;
                }
            }
            ViaCommand::DynamicKeymapGetEncoder => {
                warn!("Keymap get encoder -- not supported");
            }
            ViaCommand::DynamicKeymapSetEncoder => {
                warn!("Keymap set encoder -- not supported");
            }
            ViaCommand::Unhandled | ViaCommand::GetProtocolVersion | ViaCommand::BootloaderJump | ViaCommand::Vial => {
                info!("Unknown cmd: {:?}", report.output_data);
                report.input_data[0] = ViaCommand::Unhandled as u8
            }
        }
    }
}

impl<const HAS_KEYMAP: bool> VialService<'_, HAS_KEYMAP> {
    /// Serve 32-byte requests until the connection ends.
    pub async fn run<R: Read, T: Write>(&self, rx: &mut R, tx: &mut T) {
        #[cfg(feature = "host_lock")]
        let locker = crate::host::lock::HostLock::new(
            &self.unlocked,
            self.vial_config.unlock_keys,
            self.keys,
            self.vial_config.insecure,
            embassy_time::Duration::from_millis(100),
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
            self.process_via_packet(
                &mut report,
                #[cfg(feature = "host_lock")]
                &locker,
            )
            .await;
            if tx.write_all(&report.input_data).await.is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use embassy_futures::block_on;
    use rmk_types::action::KeyAction;

    use super::*;
    use crate::config::{BehaviorConfig, PositionalConfig};
    use crate::keymap::{KeyMap, KeymapData};

    /// Build a minimal 1x1x1 keymap + `VialService` and run `f` against it.
    fn with_service<R>(f: impl FnOnce(&mut VialService) -> R) -> R {
        let mut data = KeymapData::new([[[KeyAction::No]]]);
        let mut behavior = BehaviorConfig::default();
        let positional = PositionalConfig::<1, 1>::default();
        let keymap = block_on(KeyMap::new(&mut data, &mut behavior, &positional));
        let config = RmkConfig::default();
        let mut service = VialService::new(&keymap, &config);
        f(&mut service)
    }

    /// These tests run no storage task, so writes that wait for one would never be
    /// answered; the stand-in answers them.
    fn process(service: &mut VialService, report: &mut ViaReport) {
        block_on(embassy_futures::select::select(
            service.run(
                &mut report.output_data.as_slice(),
                &mut report.input_data.as_mut_slice(),
            ),
            crate::test_support::drain_flash_channel(),
        ));
    }

    /// A `DynamicKeymapMacroSetBuffer` (0x0F) report with `offset = 0` and the
    /// given payload `size` byte. The caller mirrors `Runnable::run` by seeding
    /// `input_data` with a copy of `output_data`.
    fn macro_set_buffer_report(size: u8) -> ViaReport {
        let mut output_data = [0u8; 32];
        output_data[0] = 0x0F; // DynamicKeymapMacroSetBuffer
        output_data[3] = size;
        ViaReport {
            input_data: output_data,
            output_data,
        }
    }

    #[cfg(feature = "host_lock")]
    #[test]
    fn unlock_survives_sessions_until_explicit_lock() {
        use core::sync::atomic::Ordering;

        use crate::event::KeyboardEvent;

        fn exchange<const HAS_KEYMAP: bool, const N: usize>(
            service: &VialService<'_, HAS_KEYMAP>,
            commands: [[u8; 2]; N],
        ) -> [[u8; 32]; N] {
            let input = commands.map(|cmd| {
                let mut packet = [0u8; 32];
                packet[..2].copy_from_slice(&cmd);
                packet
            });
            let mut replies = [[0u8; 32]; N];
            block_on(service.run(&mut input.as_flattened(), &mut replies.as_flattened_mut()));
            replies
        }

        fn check<const HAS_KEYMAP: bool>(service: &VialService<'_, HAS_KEYMAP>) {
            let before = crate::boot::BOOTLOADER_JUMPS.load(Ordering::Relaxed);
            let replies = exchange(service, [[0x0b, 0], [0xfe, 7], [0x0b, 0], [0x0b, 0]]);
            assert_eq!(replies[0][0], 0xff, "a new MCU starts locked");
            assert_eq!(replies[2][0], 0x0b);
            assert_eq!(replies[3][0], 0x0b, "protected commands retain authorization");
            let replies = exchange(service, [[0x0b, 0], [0xfe, 5]]);
            assert_eq!(replies[0][0], 0x0b, "reconnecting retains authorization");
            assert_eq!(replies[1][0], 1);
            assert_eq!(crate::boot::BOOTLOADER_JUMPS.load(Ordering::Relaxed), before + 3);
        }

        let mut data = KeymapData::new([[[KeyAction::No; 4]; 3]]);
        let mut behavior = BehaviorConfig::default();
        let positional = PositionalConfig::<3, 4>::default();
        let keymap = block_on(KeyMap::new(&mut data, &mut behavior, &positional));
        let mut config = RmkConfig::default();
        config.vial_config.unlock_keys = &[(2, 3)];
        let service = VialService::new(&keymap, &config);
        keymap.update_matrix_state(&KeyboardEvent::key(2, 3, true));
        check(&service);
        let replies = exchange(&service, [[0x02, 0x03], [0x02, 0x03]]);
        assert_eq!(replies[0][4], 8);
        assert_eq!(replies[1][4], 8, "matrix polling retains authorization");
        let rebooted = VialService::new(&keymap, &config);
        assert_eq!(exchange(&rebooted, [[0x0b, 0]])[0][0], 0xff);

        #[cfg(feature = "split")]
        let keys = crate::host::peripheral::tests::scanned_matrix();
        #[cfg(feature = "split")]
        let peripheral = VialService::<false>::new_peripheral(&keys, &config);
        #[cfg(feature = "split")]
        check(&peripheral);

        let replies = exchange(&service, [[0xfe, 8], [0x0b, 0], [0xfe, 5]]);
        assert_eq!(replies[1][0], 0xff, "explicit Lock relocks the MCU");
        assert_eq!(replies[2][0], 0);
        #[cfg(feature = "split")]
        {
            assert_eq!(
                exchange(&peripheral, [[0x0b, 0]])[0][0],
                0x0b,
                "locking the central does not relock the peripheral"
            );
            assert_eq!(exchange(&peripheral, [[0xfe, 8], [0x0b, 0]])[1][0], 0xff);
        }
    }

    // `output_data` is [u8; 32], so the handler slices `output_data[4..4 + size]`.
    // size == 28 is the largest payload that fits (writes output_data[4..32]).
    #[test]
    fn macro_set_buffer_max_size_ok() {
        with_service(|service| {
            let mut report = macro_set_buffer_report(28);
            process(service, &mut report);
        });
    }

    // size == 29 slices output_data[4..33], which is out of bounds. The sibling
    // DynamicKeymapMacroGetBuffer handler already rejects size > 28 with 0xFF;
    // SetBuffer must do the same instead of panicking.
    #[test]
    fn macro_set_buffer_oversize_rejected() {
        with_service(|service| {
            let mut report = macro_set_buffer_report(29);
            process(service, &mut report);
            assert_eq!(report.input_data[0], 0xFF);
        });
    }

    /// A `GetKeyboardValue(LayoutOptions)` (0x02 0x02) or
    /// `SetKeyboardValue(LayoutOptions)` (0x03 0x02) report carrying `value`
    /// as the big-endian payload.
    fn layout_options_report(command_id: u8, value: u32) -> ViaReport {
        let mut output_data = [0u8; 32];
        output_data[0] = command_id;
        output_data[1] = 0x02; // ViaKeyboardInfo::LayoutOptions
        BigEndian::write_u32(&mut output_data[2..6], value);
        ViaReport {
            input_data: output_data,
            output_data,
        }
    }

    // GetKeyboardValue(LayoutOptions) must return the value written by
    // SetKeyboardValue(LayoutOptions) instead of a hardcoded 0, so the Vial
    // GUI shows the user's selection again after a reconnect.
    #[test]
    fn layout_options_set_then_get_roundtrip() {
        with_service(|service| {
            let mut set = layout_options_report(0x03, 0x00C0_FFEE);
            process(service, &mut set);

            let mut get = layout_options_report(0x02, 0);
            process(service, &mut get);
            assert_eq!(BigEndian::read_u32(&get.input_data[2..6]), 0x00C0_FFEE);
        });
    }
}
