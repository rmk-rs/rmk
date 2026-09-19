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
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use rynk::{RynkDevice, RynkHostError};

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
/// Mirrors the firmware-side [`rmk::crc32::Crc32`] so the host can
/// independently track the transfer CRC for sync and rewind.
struct Crc32 {
    state: u32,
}

impl Crc32 {
    const fn new() -> Self {
        Self { state: !0u32 }
    }

    /// Precomputed once at compile time; shared by all `update` calls.
    const TABLE: [u32; 256] = build_crc32_table();

    fn update(&mut self, data: &[u8]) {
        for &byte in data {
            let idx = ((self.state ^ byte as u32) & 0xFF) as usize;
            self.state = (self.state >> 8) ^ Self::TABLE[idx];
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
/// Supported: `.bin` (raw binary), `.uf2` (UF2 container), `.elf` (ELF binary).
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

/// A single parsed UF2 data block: target flash address and payload bytes.
struct Uf2Block {
    target_addr: u32,
    payload: Vec<u8>,
}

/// Parse a UF2 file into data blocks. Validates magic numbers, filters
/// out metadata blocks (familyID), and returns only payload-bearing blocks.
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

/// Merge parsed UF2 blocks into a flat, contiguous firmware image.
/// Fills gaps between blocks with `0xFF` (erased flash).
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
        max_addr = max_addr.max(
            block
                .target_addr
                .checked_add(block.payload.len() as u32)
                .context("UF2 block address overflows u32")?,
        );
    }

    let size = (max_addr - min_addr) as usize;
    let mut firmware = vec![0xFFu8; size];
    // Track written ranges to detect overlapping blocks (usually corrupt input).
    let mut written: Vec<(usize, usize)> = Vec::with_capacity(blocks.len());

    for block in &blocks {
        let offset = (block.target_addr - min_addr) as usize;
        let end = offset + block.payload.len();
        if written.iter().any(|&(s, e)| offset < e && s < end) {
            log::warn!(
                "UF2 block at {:#010x} overlaps a previous block, overwriting",
                block.target_addr
            );
        }
        written.push((offset, end));
        firmware[offset..end].copy_from_slice(&block.payload);
    }

    log::info!(
        "Loaded UF2: {} bytes at address {:#010x}..{:#010x}",
        firmware.len(),
        min_addr,
        max_addr
    );
    Ok(firmware)
}

/// Extract PT_LOAD segments from an ELF file into a contiguous firmware
/// image. Fills BSS regions with zeros, gaps between segments with `0xFF`.
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
        let vaddr = u32::try_from(ph.p_vaddr).context("ELF segment address exceeds u32")?;
        let end64 = ph
            .p_vaddr
            .checked_add(ph.p_memsz)
            .context("ELF segment end overflows")?;
        let end = u32::try_from(end64).context("ELF segment end exceeds u32")?;
        min_addr = min_addr.min(vaddr);
        max_addr = max_addr.max(end);
    }

    let size = (max_addr - min_addr) as usize;
    let mut firmware = vec![0xFFu8; size];

    for ph in &load_segments {
        let vaddr = u32::try_from(ph.p_vaddr).context("ELF segment address exceeds u32")?;
        let offset = (vaddr - min_addr) as usize;
        let mem_end = (ph.p_memsz) as usize;

        if ph.p_filesz > 0 {
            let file_end = ph
                .p_offset
                .checked_add(ph.p_filesz)
                .context("ELF segment file range overflows")?;
            let file_end = usize::try_from(file_end).context("ELF segment file end exceeds usize")?;
            if file_end > data.len() {
                anyhow::bail!(
                    "ELF segment file range {:#x}..{:#x} exceeds file size",
                    ph.p_offset,
                    file_end
                );
            }
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

/// Print a progress bar to stderr showing transfer completion.
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
    if firmware.is_empty() {
        anyhow::bail!("firmware image is empty (0 bytes), refusing to flash");
    }
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
    let (client, mut driver) = tokio::time::timeout(Duration::from_secs(15), device.connect())
        .await
        .context("connection timed out after 15s — BLE link or GATT handshake did not complete")?
        .context("connection failed")?;
    let client = Arc::new(client);

    let result = tokio::select! {
        err = driver.run(Arc::as_ref(&client)) => Err(anyhow::anyhow!("driver error: {}", err)),
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

            if caps.max_payload_size <= 12 {
                anyhow::bail!(
                    "firmware reports max_payload_size {} which is too small for DFU",
                    caps.max_payload_size
                );
            }
            let mut chunk_size = caps.max_payload_size as usize - 8;
            chunk_size -= chunk_size % 4; // NOR flash writes must be word-aligned
            let total = firmware.len();
            let mut crc = Crc32::new();
            let mut last_checkpoint_offset = 0u32;
            let mut last_checkpoint_crc: u32 = Crc32::new().finalize();
            let mut packets_since_check: u32 = 0;

            eprintln!("Starting DFU transfer... {} bytes ({} byte chunks)", total, chunk_size);
            client.dfu_start().await?;

            // Pipelined DFU writes with batch flush: send up to PIPELINE_DEPTH
            // chunks fire-and-forget (no per-chunk ACK wait), then CRC-sync as
            // a flush barrier. The sync round-trip gives the firmware's BLE
            // stack time to drain — without it the host outruns the ~5 ACL
            // buffers of the CYW43 and the link OOMs. Note: the effective sync
            // cadence is capped at PIPELINE_DEPTH regardless of --crc-interval,
            // because the batch size bounds how many chunks can be in flight.
            // --crc-interval 0 skips intermediate syncs (no flow control —
            // benchmark use only).
            const PIPELINE_DEPTH: usize = 4;
            let sync_every = if cli.crc_interval == 0 {
                None
            } else {
                Some((cli.crc_interval as usize).min(PIPELINE_DEPTH).max(1))
            };
            let num_chunks = total.div_ceil(chunk_size);
            let mut chunk_idx = 0usize;
            while chunk_idx < num_chunks {
                let batch_end = (chunk_idx + PIPELINE_DEPTH).min(num_chunks);
                // 1) Send the whole batch fire-and-forget.
                for idx in chunk_idx..batch_end {
                    let start = idx * chunk_size;
                    let end = (start + chunk_size).min(total);
                    let abs_offset = (idx * chunk_size) as u32;
                    client.dfu_write(abs_offset, firmware[start..end].to_vec()).await?;
                    crc.update(&firmware[start..end]);
                    print_progress(end, total);
                }
                packets_since_check += (batch_end - chunk_idx) as u32;
                chunk_idx = batch_end;
                // 2) Flush barrier: CRC-sync forces the firmware to drain all
                //    queued writes before responding. Skipped only with
                //    --crc-interval 0 (no flow control).
                if sync_every.map_or(false, |n| packets_since_check >= n as u32) {
                    match client.dfu_crc_sync(crc.finalize()).await {
                        Ok(()) => {
                            last_checkpoint_offset = (chunk_idx * chunk_size).min(total) as u32;
                            last_checkpoint_crc = crc.finalize();
                            packets_since_check = 0;
                        }
                        Err(RynkHostError::Rejected(_)) => {
                            eprintln!("\nCRC mismatch, rolling back to checkpoint at offset {}...", last_checkpoint_offset);
                            client.dfu_crc_rewind(last_checkpoint_offset, last_checkpoint_crc).await?;
                            crc = Crc32::from_state(last_checkpoint_crc);
                            // last_checkpoint_offset is always chunk-aligned.
                            chunk_idx = last_checkpoint_offset as usize / chunk_size;
                            packets_since_check = 0;
                        }
                        Err(other) => return Err(other.into()),
                    }
                }
            }
            eprintln!();

            // Mandatory drain sync: a partial final batch may not have hit
            // the sync cadence. Without this, dfu_verify could read back
            // flash before the queued writes land.
            if packets_since_check > 0 {
                client.dfu_crc_sync(crc.finalize()).await?;
            }

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
