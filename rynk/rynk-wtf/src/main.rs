//! rynk-wtf — Wireless Transmission of Firmware
//!
//! A BLE DFU tool for RMK keyboards. Connects to an RMK keyboard over BLE
//! using the rynk protocol and transfers a firmware image for OTA update.
//!
//! Usage:
//! ```text
//! rynk-wtf [OPTIONS] <firmware>
//!
//! Arguments:
//!   <firmware>  Path to firmware file (.bin, .uf2, or .elf)
//!
//! Options:
//!   --reset              Reset device after successful update
//!   --crc-interval N     CRC check every N packets, 0 = skip intermediate checks [default: 100]
//!   --device <NAME>      Connect to specific device (interactive picker if omitted)
//!   -h, --help           Print help
//! ```

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use rynk::RynkDevice;

#[derive(Parser)]
#[command(name = "rynk-wtf", about = "Wireless Transmission of Firmware for RMK keyboards")]
struct Cli {
    /// Path to firmware file (.bin, .uf2, or .elf)
    firmware: PathBuf,

    /// Reset device after successful update
    #[arg(long)]
    reset: bool,

    /// CRC check every N packets, 0 = skip intermediate checks
    #[arg(long, default_value = "100")]
    crc_interval: u32,

    /// Connect to specific device name (interactive picker if omitted)
    #[arg(long)]
    device: Option<String>,
}

/// Incremental CRC-32 (IEEE 802.3 / PKZip) calculator.
struct Crc32 {
    state: u32,
}

impl Crc32 {
    const fn new() -> Self {
        Self { state: !0u32 }
    }

    fn update(&mut self, data: &[u8]) {
        for &byte in data {
            let idx = ((self.state ^ byte as u32) & 0xFF) as usize;
            const TABLE: [u32; 256] = build_crc32_table();
            self.state = (self.state >> 8) ^ TABLE[idx];
        }
    }

    const fn finalize(&self) -> u32 {
        !self.state
    }

    const fn from_state(finalized: u32) -> Self {
        Self { state: !finalized }
    }
}

const fn build_crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0u32;
    while i < 256 {
        let mut crc = i;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
}

/// Load firmware from a file, detecting the format by extension.
fn load_firmware(path: &std::path::Path) -> Result<Vec<u8>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    match ext {
        "bin" => {
            let data = std::fs::read(path).context("failed to read firmware file")?;
            Ok(data)
        }
        "uf2" => load_uf2(path),
        #[cfg(feature = "elf")]
        "elf" | "" => load_elf(path),
        #[cfg(not(feature = "elf"))]
        "" => {
            anyhow::bail!("cannot determine firmware format from empty extension; use .bin, .uf2, or .elf");
        }
        _ => {
            anyhow::bail!(
                "unsupported firmware format: .{ext}\n\
                 Supported formats: .bin, .uf2, .elf"
            );
        }
    }
}

const UF2_MAGIC_START0: u32 = 0x0A324655;
const UF2_MAGIC_START1: u32 = 0x9E5D5157;
const UF2_MAGIC_END: u32 = 0x0AB16F30;
const UF2_BLOCK_SIZE: usize = 512;
const UF2_PAYLOAD_OFFSET: usize = 32;
const UF2_PAYLOAD_SIZE: usize = 256;

struct Uf2Block {
    target_addr: u32,
    payload: Vec<u8>,
}

fn parse_uf2_blocks(data: &[u8]) -> Result<Vec<Uf2Block>> {
    if data.len() % UF2_BLOCK_SIZE != 0 {
        anyhow::bail!("UF2 file size ({}) is not a multiple of 512 bytes", data.len());
    }
    let mut blocks = Vec::new();
    for (i, chunk) in data.chunks(UF2_BLOCK_SIZE).enumerate() {
        let magic0 = u32::from_le_bytes(chunk[0..4].try_into().unwrap());
        let magic1 = u32::from_le_bytes(chunk[4..8].try_into().unwrap());
        if magic0 != UF2_MAGIC_START0 || magic1 != UF2_MAGIC_START1 {
            anyhow::bail!("invalid UF2 magic in block {i}");
        }
        let flags = u32::from_le_bytes(chunk[8..12].try_into().unwrap());
        let target_addr = u32::from_le_bytes(chunk[12..16].try_into().unwrap());
        let payload_len = u32::from_le_bytes(chunk[16..20].try_into().unwrap()) as usize;
        let magic_end = u32::from_le_bytes(chunk[508..512].try_into().unwrap());
        if magic_end != UF2_MAGIC_END {
            anyhow::bail!("invalid UF2 end magic in block {i}");
        }

        // Bit 12 of flags: not a default family block (familyID valid)
        // Bit 13: familyID present — skip those blocks (they are metadata)
        if flags & (1 << 13) != 0 {
            continue;
        }

        let payload_len = payload_len.min(UF2_PAYLOAD_SIZE);
        let payload = chunk[UF2_PAYLOAD_OFFSET..UF2_PAYLOAD_OFFSET + payload_len].to_vec();
        blocks.push(Uf2Block { target_addr, payload });
    }
    Ok(blocks)
}

fn load_uf2(path: &std::path::Path) -> Result<Vec<u8>> {
    let data = std::fs::read(path).context("failed to read UF2 file")?;
    let blocks = parse_uf2_blocks(&data).context("failed to parse UF2 file")?;

    if blocks.is_empty() {
        anyhow::bail!("UF2 file contains no data blocks");
    }

    let mut min_addr = u32::MAX;
    let mut max_addr = u32::MIN;
    for block in &blocks {
        min_addr = min_addr.min(block.target_addr);
        max_addr = max_addr.max(block.target_addr + block.payload.len() as u32);
    }

    let size = (max_addr - min_addr) as usize;
    let mut firmware = vec![0xFFu8; size];

    for block in &blocks {
        let offset = (block.target_addr - min_addr) as usize;
        firmware[offset..offset + block.payload.len()].copy_from_slice(&block.payload);
    }

    log::info!(
        "Loaded UF2: {} bytes at address {:#010x}..{:#010x}",
        firmware.len(),
        min_addr,
        max_addr
    );
    Ok(firmware)
}

#[cfg(feature = "elf")]
fn load_elf(path: &std::path::Path) -> Result<Vec<u8>> {
    let data = std::fs::read(path).context("failed to read ELF file")?;
    let elf = goblin::elf::Elf::parse(&data).context("failed to parse ELF file")?;

    let load_segments: Vec<_> = elf
        .program_headers
        .iter()
        .filter(|ph| ph.p_type == goblin::elf::program_header::PT_LOAD)
        .collect();

    if load_segments.is_empty() {
        anyhow::bail!("ELF file contains no PT_LOAD segments");
    }

    let mut min_addr = u32::MAX;
    let mut max_addr = u32::MIN;
    for ph in &load_segments {
        min_addr = min_addr.min(ph.p_vaddr as u32);
        max_addr = max_addr.max((ph.p_vaddr + ph.p_memsz) as u32);
    }

    let size = (max_addr - min_addr) as usize;
    let mut firmware = vec![0xFFu8; size];

    for ph in &load_segments {
        let offset = (ph.p_vaddr as u32 - min_addr) as usize;
        let mem_end = (ph.p_vaddr + ph.p_memsz) as usize - ph.p_vaddr as usize;

        if ph.p_filesz > 0 {
            let file_end = (ph.p_offset + ph.p_filesz) as usize;
            firmware[offset..offset + ph.p_filesz as usize].copy_from_slice(&data[ph.p_offset as usize..file_end]);
        }
        if ph.p_memsz > ph.p_filesz {
            let bss_start = offset + ph.p_filesz as usize;
            let bss_end = offset + mem_end;
            firmware[bss_start..bss_end].fill(0x00);
        }
    }

    log::info!(
        "Loaded ELF: {} bytes at address {:#010x}..{:#010x}",
        firmware.len(),
        min_addr,
        max_addr
    );
    Ok(firmware)
}

fn print_progress(current: usize, total: usize) {
    let percent = if total > 0 { (current * 100) / total } else { 100 };
    let width = 30;
    let filled = (percent * width) / 100;
    let bar: String = "#".repeat(filled) + &"-".repeat(width - filled);
    eprint!("\r[{}] {}% {}/{} bytes", bar, percent, current, total);
    let _ = std::io::stderr().flush();
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    eprintln!("Loading firmware: {}", cli.firmware.display());
    let firmware = load_firmware(&cli.firmware)?;
    eprintln!("Firmware size: {} bytes", firmware.len());

    eprintln!("Scanning for BLE keyboards...");
    let devices = rynk_ble::BleDevice::discover_all()
        .await
        .context("BLE discovery failed")?;

    if devices.is_empty() {
        anyhow::bail!("No BLE keyboards found. Make sure your keyboard is in pairing mode.");
    }

    let device = if let Some(ref name) = cli.device {
        devices
            .into_iter()
            .find(|d| d.label().contains(name.as_str()))
            .with_context(|| format!("device '{}' not found", name))?
    } else if devices.len() == 1 {
        let d = devices.into_iter().next().unwrap();
        eprintln!("Found: {}", d.label());
        d
    } else {
        eprintln!("Found {} devices:", devices.len());
        for (i, d) in devices.iter().enumerate() {
            eprintln!("  [{}] {}", i + 1, d.label());
        }
        eprint!("Select device [1]: ");
        let _ = std::io::stderr().flush();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let idx: usize = input.trim().parse::<usize>().unwrap_or(1).saturating_sub(1);
        devices
            .into_iter()
            .nth(idx)
            .with_context(|| format!("invalid selection: {}", idx + 1))?
    };

    eprintln!("Connecting to {}...", device.label());
    let (client, mut driver) = device.connect().await.context("connection failed")?;

    let result = tokio::select! {
        err = driver.run(&client) => Err(anyhow::anyhow!("driver error: {}", err)),
        result = async {
            let caps = client.get_capabilities().await?;
            if !caps.dfu_enabled {
                anyhow::bail!("Device does not support DFU");
            }

            let lock = client.get_lock_status().await?;
            if lock.locked {
                if lock.key_positions.is_empty() {
                    anyhow::bail!("Device is permanently locked (no unlock keys configured)");
                }
                eprintln!("Unlock required. Hold keys: {:?}", lock.key_positions);
                loop {
                    let status = client.unlock_poll().await?;
                    if !status.locked {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
                eprintln!("Unlock successful.");
            }

            let chunk_size = caps.max_payload_size as usize;
            let total = firmware.len();
            let mut crc = Crc32::new();
            let mut checkpoint_offset = 0u32;
            let mut checkpoint_crc: u32 = Crc32::new().finalize();
            let mut packets_since_check: u32 = 0;

            eprintln!("Starting DFU transfer... {} bytes ({} byte chunks)", total, chunk_size);
            client.dfu_start().await?;

            for (idx, chunk) in firmware.chunks(chunk_size).enumerate() {
                let abs_offset = (idx * chunk_size) as u32;

                client.dfu_write(abs_offset, chunk.to_vec()).await?;
                crc.update(chunk);
                packets_since_check += 1;

                let progress = (abs_offset as usize + chunk.len()).min(total);
                print_progress(progress, total);

                if cli.crc_interval > 0 && packets_since_check >= cli.crc_interval {
                    match client.dfu_crc_sync(crc.finalize()).await {
                        Ok(()) => {
                            checkpoint_offset = abs_offset + chunk.len() as u32;
                            checkpoint_crc = crc.finalize();
                            packets_since_check = 0;
                        }
                        Err(_) => {
                            eprintln!("\nCRC mismatch at offset {}, rolling back...", abs_offset);
                            client
                                .dfu_crc_rewind(checkpoint_offset, checkpoint_crc)
                                .await?;
                            crc = Crc32::from_state(checkpoint_crc);

                            let start_idx = (checkpoint_offset / chunk_size as u32) as usize;
                            for (retry_idx, retry_chunk) in firmware[start_idx..].chunks(chunk_size).enumerate() {
                                let retry_offset = ((start_idx + retry_idx) * chunk_size) as u32;
                                client.dfu_write(retry_offset, retry_chunk.to_vec()).await?;
                                crc.update(retry_chunk);
                                let progress = (retry_offset as usize + retry_chunk.len()).min(total);
                                print_progress(progress, total);
                            }
                            packets_since_check = 0;
                        }
                    }
                }
            }
            eprintln!();

            eprintln!("Verifying...");
            client.dfu_verify(crc.finalize()).await?;

            client.dfu_finish().await?;
            eprintln!("Update complete!");

            if cli.reset {
                eprintln!("Resetting device...");
                client.dfu_reset().await?;
            }

            Ok::<_, anyhow::Error>(())
        } => result,
    };

    result
}
