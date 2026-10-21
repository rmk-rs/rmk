//! Identity and authorization commands shared by central and peripheral USB services.

use byteorder::{BigEndian, ByteOrder, LittleEndian};
use rmk_types::protocol::vial::{VIA_PROTOCOL_VERSION, VIAL_EP_SIZE, VIAL_PROTOCOL_VERSION, ViaCommand, VialCommand};

use crate::config::VialConfig;
use crate::hid::ViaReport;

pub(crate) fn process_system_packet(
    report: &mut ViaReport,
    vial_config: &VialConfig<'_>,
    #[cfg(feature = "host_lock")] locker: &crate::host::lock::HostLock<'_>,
) -> bool {
    match ViaCommand::from(report.output_data[0]) {
        ViaCommand::GetProtocolVersion => {
            BigEndian::write_u16(&mut report.input_data[1..3], VIA_PROTOCOL_VERSION);
        }
        ViaCommand::BootloaderJump => {
            #[cfg(feature = "host_lock")]
            let unlocked = locker.is_unlocked();
            #[cfg(not(feature = "host_lock"))]
            let unlocked = true;
            if unlocked {
                crate::boot::jump_to_bootloader();
            } else {
                report.input_data[0] = ViaCommand::Unhandled as u8;
            }
        }
        ViaCommand::Vial => match VialCommand::from(report.output_data[1]) {
            VialCommand::GetKeyboardId => {
                // Returns vial protocol version + vial keyboard id
                LittleEndian::write_u32(&mut report.input_data[0..4], VIAL_PROTOCOL_VERSION);
                report.input_data[4..12].clone_from_slice(vial_config.vial_keyboard_id);
                debug!("Vial return: {:?}", report.input_data);
            }
            VialCommand::GetSize => {
                LittleEndian::write_u32(&mut report.input_data[0..4], vial_config.vial_keyboard_def.len() as u32);
            }
            VialCommand::GetKeyboardDef => {
                let page = LittleEndian::read_u16(&report.output_data[2..4]) as usize;
                let start = page * VIAL_EP_SIZE;
                let mut end = start + VIAL_EP_SIZE;
                let vial_keyboard_def = &vial_config.vial_keyboard_def;
                if end < start || start >= vial_keyboard_def.len() {
                    return true;
                }
                if end > vial_keyboard_def.len() {
                    end = vial_keyboard_def.len();
                }
                vial_keyboard_def[start..end].iter().enumerate().for_each(|(i, v)| {
                    report.input_data[i] = *v;
                });
                debug!(
                    "Vial return: page:{} start:{} end: {}, data: {:?}",
                    page, start, end, report.input_data
                );
            }
            VialCommand::GetUnlockStatus => {
                // Reset all data to 0xFF(it's required!)
                report.input_data.fill(0xFF);
                #[cfg(feature = "host_lock")]
                {
                    // Unlocked
                    report.input_data[0] = locker.is_unlocked() as u8;
                    // Unlock in progress
                    report.input_data[1] = locker.is_unlocking() as u8;
                    // Unlock keys
                    for (idx, (row, col)) in vial_config.unlock_keys.iter().enumerate() {
                        report.input_data[2 + idx * 2] = *row;
                        report.input_data[3 + idx * 2] = *col;
                    }
                }
                #[cfg(not(feature = "host_lock"))]
                {
                    // Unlocked
                    report.input_data[0] = 1;
                    // Unlock in progress
                    report.input_data[1] = 0;
                    warn!("Vial lock feature is not enabled");
                }
            }
            VialCommand::UnlockStart => {
                #[cfg(feature = "host_lock")]
                locker.unlocking();
                #[cfg(not(feature = "host_lock"))]
                error!("Vial lock feature is not enabled");
            }
            VialCommand::UnlockPoll => {
                #[cfg(feature = "host_lock")]
                {
                    locker.unlocking();
                    report.input_data[0] = locker.is_unlocked() as u8;
                    report.input_data[1] = locker.is_unlocking() as u8;
                    report.input_data[2] = locker.check_unlock();
                }
                #[cfg(not(feature = "host_lock"))]
                error!("Vial lock feature is not enabled");
            }
            VialCommand::Lock => {
                #[cfg(feature = "host_lock")]
                locker.lock();
                #[cfg(not(feature = "host_lock"))]
                error!("Vial lock feature is not enabled");
            }
            _ => return false,
        },
        _ => return false,
    }
    true
}
