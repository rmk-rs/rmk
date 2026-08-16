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
use crate::{MACRO_SPACE_SIZE, boot};

pub(crate) mod keycode_convert;
mod vial;

pub struct VialService<'a> {
    ctx: KeyboardContext<'a>,
    vial_config: VialConfig<'static>,
    #[cfg(feature = "host_lock")]
    locker: crate::host::lock::HostLock<'a>,
}

impl<'a> VialService<'a> {
    pub fn new(keymap: &'a KeyMap<'a>, config: &RmkConfig<'static>) -> Self {
        Self {
            ctx: KeyboardContext::new(keymap),
            vial_config: config.vial_config,
            // Vial's poll cadence is ~100 ms (`VialCommand::UnlockPoll`).
            #[cfg(feature = "host_lock")]
            locker: crate::host::lock::HostLock::new(
                config.vial_config.unlock_keys,
                keymap,
                config.vial_config.insecure,
                embassy_time::Duration::from_millis(100),
            ),
        }
    }

    async fn process_via_packet(&self, report: &mut ViaReport) {
        let command_id = report.output_data[0];

        // Caller pre-fills `input_data` from `output_data`, so individual arms
        // only need to overwrite the bytes they actually change.
        match command_id.into() {
            ViaCommand::GetProtocolVersion => {
                BigEndian::write_u16(&mut report.input_data[1..3], VIA_PROTOCOL_VERSION);
            }
            ViaCommand::GetKeyboardValue => {
                // Check the second u8
                match report.output_data[1].try_into() {
                    Ok(v) => match v {
                        ViaKeyboardInfo::Uptime => {
                            let value = Instant::now().as_millis() as u32;
                            BigEndian::write_u32(&mut report.input_data[2..6], value);
                        }
                        ViaKeyboardInfo::LayoutOptions => {
                            let layout_option = self.ctx.layout_options();
                            BigEndian::write_u32(&mut report.input_data[2..6], layout_option);
                        }
                        #[cfg(not(feature = "host_lock"))]
                        ViaKeyboardInfo::SwitchMatrixState => {
                            error!("It is not secure to use matrix tester without vial lock");
                        }
                        #[cfg(feature = "host_lock")]
                        ViaKeyboardInfo::SwitchMatrixState if self.locker.is_unlocked() => {
                            let bitmap = &mut report.input_data[2..];
                            self.ctx.read_matrix_state(bitmap);
                            // Vial wants each row's bytes big-endian (QMK matrix_row_t order).
                            let (rows, cols, _) = self.ctx.keymap_dimensions();
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
                            self.ctx.set_layout_options(layout_option).await;
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
                let action = self.ctx.get_action(layer, row, col);
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
                self.ctx.set_action(layer, row, col, action).await;
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
                self.ctx.reset_storage().await;
                // TODO: Reboot after a eeprom reset?
            }
            ViaCommand::BootloaderJump => {
                warn!("Bootloader jumping");
                boot::jump_to_bootloader();
            }
            ViaCommand::DynamicKeymapMacroGetCount => {
                report.input_data[1] = 32;
                warn!("Macro get count -- to be implemented")
            }
            ViaCommand::DynamicKeymapMacroGetBufferSize => {
                report.input_data[1] = (MACRO_SPACE_SIZE as u16 >> 8) as u8;
                report.input_data[2] = (MACRO_SPACE_SIZE & 0xFF) as u8;
            }
            ViaCommand::DynamicKeymapMacroGetBuffer => {
                let offset = BigEndian::read_u16(&report.output_data[1..3]) as usize;
                let size = report.output_data[3] as usize;
                if size <= 28 {
                    self.ctx.read_macro_buffer(offset, &mut report.input_data[4..4 + size]);
                    debug!("Get macro buffer: offset: {}, data: {:?}", offset, report.input_data);
                } else {
                    report.input_data[0] = 0xFF;
                }
            }
            ViaCommand::DynamicKeymapMacroSetBuffer => {
                // Every write writes all buffer space of the macro(if it's not empty)
                let offset = BigEndian::read_u16(&report.output_data[1..3]);
                // Current sequence size, <= 28
                let size = report.output_data[3];
                // `output_data` is 32 bytes, so the payload slice output_data[4..4 + size]
                // is only valid for size <= 28. Reject oversized writes instead of
                // panicking, mirroring the DynamicKeymapMacroGetBuffer handler above.
                if size <= 28 {
                    // End of current sequence in the macro cache
                    // The first sequence, reset the macro cache
                    if offset == 0 {
                        self.ctx.reset_macro_buffer();
                    }

                    // Update macro cache + flush full buffer to storage
                    info!("Setting macro buffer, offset: {}, size: {}", offset, size);
                    self.ctx
                        .write_macro_buffer(offset as usize, &report.output_data[4..4 + size as usize])
                        .await;
                } else {
                    report.input_data[0] = 0xFF;
                }
            }
            ViaCommand::DynamicKeymapMacroReset => {
                warn!("Macro reset -- to be implemented")
            }
            ViaCommand::DynamicKeymapGetLayerCount => {
                report.input_data[1] = self.ctx.keymap_dimensions().2 as u8;
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
                    let a = self.ctx.get_action_flat(start + i);
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
                let (rows, cols, _) = self.ctx.keymap_dimensions();
                for i in 0..(size as usize) {
                    let via_keycode = LittleEndian::read_u16(&report.output_data[idx..idx + 2]);
                    let action = from_via_keycode(via_keycode);
                    let flat_index = offset as usize + i;
                    self.ctx.try_set_action_flat(flat_index, action, rows, cols);
                    idx += 2;
                }
            }
            ViaCommand::DynamicKeymapGetEncoder => {
                warn!("Keymap get encoder -- not supported");
            }
            ViaCommand::DynamicKeymapSetEncoder => {
                warn!("Keymap set encoder -- not supported");
            }
            ViaCommand::Vial => {
                process_vial(
                    report,
                    &self.vial_config,
                    #[cfg(feature = "host_lock")]
                    &self.locker,
                    &self.ctx,
                )
                .await
            }
            ViaCommand::Unhandled => {
                info!("Unknown cmd: {:?}", report.output_data);
                report.input_data[0] = ViaCommand::Unhandled as u8
            }
        }
    }
}

impl VialService<'_> {
    /// Drive one Vial session against `rx`/`tx` (32-byte request → 32-byte
    /// response, processed in place). Returns on any read/write error;
    /// transport-specific reconnect lives in the caller.
    pub async fn run_session<R: Read, T: Write>(&self, rx: &mut R, tx: &mut T) {
        let mut buf = [0u8; 32];
        loop {
            if rx.read_exact(&mut buf).await.is_err() {
                return;
            }
            let mut report = ViaReport {
                input_data: buf,
                output_data: buf,
            };
            self.process_via_packet(&mut report).await;
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

    // `output_data` is [u8; 32], so the handler slices `output_data[4..4 + size]`.
    // size == 28 is the largest payload that fits (writes output_data[4..32]).
    #[test]
    fn macro_set_buffer_max_size_ok() {
        with_service(|service| {
            let mut report = macro_set_buffer_report(28);
            block_on(service.process_via_packet(&mut report));
        });
    }

    // size == 29 slices output_data[4..33], which is out of bounds. The sibling
    // DynamicKeymapMacroGetBuffer handler already rejects size > 28 with 0xFF;
    // SetBuffer must do the same instead of panicking.
    #[test]
    fn macro_set_buffer_oversize_rejected() {
        with_service(|service| {
            let mut report = macro_set_buffer_report(29);
            block_on(service.process_via_packet(&mut report));
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
            block_on(service.process_via_packet(&mut set));

            let mut get = layout_options_report(0x02, 0);
            block_on(service.process_via_packet(&mut get));
            assert_eq!(BigEndian::read_u32(&get.input_data[2..6]), 0x00C0_FFEE);
        });
    }
}
