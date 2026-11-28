use embassy_time::{Duration, Instant};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::spi::SpiBus;
use embedded_storage_async::nor_flash::{
    ErrorType, MultiwriteNorFlash, NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash,
};

const CMD_READ: u8 = 0x03;
const CMD_PAGE_PROGRAM: u8 = 0x02;
const CMD_WRITE_ENABLE: u8 = 0x06;
const CMD_READ_STATUS: u8 = 0x05;
const CMD_WRITE_STATUS: u8 = 0x01;
const CMD_SECTOR_ERASE: u8 = 0x20;
const CMD_BLOCK_ERASE_64K: u8 = 0xD8;
const CMD_JEDEC_ID: u8 = 0x9F;
const CMD_READ_4B: u8 = 0x13;
const CMD_PAGE_PROGRAM_4B: u8 = 0x12;
const CMD_SECTOR_ERASE_4B: u8 = 0x21;
const CMD_BLOCK_ERASE_64K_4B: u8 = 0xDC;

const PAGE_SIZE: u32 = 256;
const SECTOR_SIZE: u32 = 4096;
const BLOCK64_SIZE: u32 = 65536;
/// Largest part reachable with 3-byte addresses.
const MAX_3B_ADDR_SIZE: u32 = 1 << 24;

/// [`NorFlash`] implementation for W25Q and compatible 25-series SPI NOR flash
/// chips. Uses standard JEDEC commands valid across Winbond, Macronix, ISSI,
/// and similar families.
pub struct W25qNorFlash<BUS: SpiBus, CS: OutputPin> {
    bus: BUS,
    cs: CS,
    flash_size: u32,
}

impl<BUS: SpiBus, CS: OutputPin> W25qNorFlash<BUS, CS> {
    pub fn new(bus: BUS, mut cs: CS, flash_size: u32) -> Self {
        cs.set_high().ok();
        Self { bus, cs, flash_size }
    }

    /// Build an opcode + address header, using the stateless 4-byte-address
    /// opcode on parts larger than 16 MiB. Returns the buffer and its length.
    fn address_header(&self, opcode: u8, opcode_4b: u8, addr: u32) -> ([u8; 5], usize) {
        let [a3, a2, a1, a0] = addr.to_be_bytes();
        if self.flash_size > MAX_3B_ADDR_SIZE {
            ([opcode_4b, a3, a2, a1, a0], 5)
        } else {
            ([opcode, a2, a1, a0, 0], 4)
        }
    }

    /// Run one chip-select-framed transaction: write `cmd`, then read into
    /// `buf`. `SpiBus` may return before bytes are on the wire, so the bus is
    /// flushed before CS is released.
    async fn transaction(&mut self, cmd: &[u8], buf: &mut [u8]) -> Result<(), W25qError<BUS::Error>> {
        self.cs.set_low().ok();
        let res = async {
            self.bus.write(cmd).await?;
            if !buf.is_empty() {
                self.bus.read(buf).await?;
            }
            self.bus.flush().await
        }
        .await;
        self.cs.set_high().ok();
        res.map_err(W25qError::Spi)
    }

    /// Wait until the write-in-progress bit clears. Datasheet worst case for
    /// a 64 KiB block erase is ~400 ms; the 10 s deadline leaves ample margin
    /// before reporting a stuck chip instead of spinning forever.
    async fn wait_wip(&mut self) -> Result<(), W25qError<BUS::Error>> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = self.read_status().await?;
            if status & 0x01 == 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(W25qError::Timeout);
            }
            embassy_futures::yield_now().await;
        }
    }

    /// Verify the flash is busy (WIP=1) after an erase/program command.
    /// If WIP is still 0, the command was not accepted by the chip.
    async fn verify_wip_set(&mut self) -> Result<(), W25qError<BUS::Error>> {
        let status = self.read_status().await?;
        if status & 0x01 == 0 {
            return Err(W25qError::WriteNotAccepted);
        }
        Ok(())
    }

    pub async fn read_jedec_id(&mut self) -> Result<[u8; 3], W25qError<BUS::Error>> {
        let mut id = [0u8; 3];
        self.transaction(&[CMD_JEDEC_ID], &mut id).await?;
        Ok(id)
    }

    pub async fn read_status_register(&mut self) -> Result<u8, W25qError<BUS::Error>> {
        self.read_status().await
    }

    pub async fn clear_block_protection(&mut self) -> Result<(), W25qError<BUS::Error>> {
        self.write_enable().await?;
        self.transaction(&[CMD_WRITE_STATUS, 0x00], &mut []).await?;
        self.wait_wip().await
    }

    async fn read_status(&mut self) -> Result<u8, W25qError<BUS::Error>> {
        let mut status = [0u8; 1];
        self.transaction(&[CMD_READ_STATUS], &mut status).await?;
        Ok(status[0])
    }

    async fn write_enable(&mut self) -> Result<(), W25qError<BUS::Error>> {
        self.transaction(&[CMD_WRITE_ENABLE], &mut []).await?;
        // Verify Write-Enable-Latch was set
        let status = self.read_status().await?;
        if status & 0x02 == 0 {
            return Err(W25qError::WriteProtect);
        }
        Ok(())
    }

    async fn read_data(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), W25qError<BUS::Error>> {
        self.wait_wip().await?;
        let (cmd, cmd_len) = self.address_header(CMD_READ, CMD_READ_4B, addr);
        self.transaction(&cmd[..cmd_len], buf).await
    }

    async fn page_program(&mut self, addr: u32, data: &[u8]) -> Result<(), W25qError<BUS::Error>> {
        self.write_enable().await?;
        let (header, header_len) = self.address_header(CMD_PAGE_PROGRAM, CMD_PAGE_PROGRAM_4B, addr);
        let total = header_len + data.len();
        let mut tx_buf = [0u8; 5 + PAGE_SIZE as usize];
        tx_buf[..header_len].copy_from_slice(&header[..header_len]);
        tx_buf[header_len..total].copy_from_slice(data);
        self.transaction(&tx_buf[..total], &mut []).await?;
        self.wait_wip().await
    }
}

#[derive(Debug)]
pub enum W25qError<SPI: embedded_hal::spi::Error> {
    Spi(SPI),
    Timeout,
    WriteProtect,
    WriteNotAccepted,
}

impl<SPI: embedded_hal::spi::Error + core::fmt::Debug> core::fmt::Display for W25qError<SPI> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            W25qError::Spi(e) => write!(f, "SPI error: {:?}", e),
            W25qError::Timeout => write!(f, "timeout waiting for write-in-progress to clear"),
            W25qError::WriteProtect => write!(f, "write enable latch not set (write protected?)"),
            W25qError::WriteNotAccepted => write!(f, "command not accepted by flash"),
        }
    }
}

#[cfg(feature = "defmt")]
impl<SPI: embedded_hal::spi::Error + defmt::Format> defmt::Format for W25qError<SPI> {
    fn format(&self, f: defmt::Formatter) {
        match self {
            W25qError::Spi(e) => defmt::write!(f, "Spi({})", e),
            W25qError::Timeout => defmt::write!(f, "Timeout"),
            W25qError::WriteProtect => defmt::write!(f, "WriteProtect"),
            W25qError::WriteNotAccepted => defmt::write!(f, "WriteNotAccepted"),
        }
    }
}

impl<SPI: embedded_hal::spi::Error> NorFlashError for W25qError<SPI> {
    fn kind(&self) -> NorFlashErrorKind {
        NorFlashErrorKind::Other
    }
}

impl<BUS: SpiBus, CS: OutputPin> ErrorType for W25qNorFlash<BUS, CS> {
    type Error = W25qError<BUS::Error>;
}

impl<BUS: SpiBus, CS: OutputPin> ReadNorFlash for W25qNorFlash<BUS, CS> {
    const READ_SIZE: usize = 1;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        self.read_data(offset, bytes).await
    }

    fn capacity(&self) -> usize {
        self.flash_size as usize
    }
}

impl<BUS: SpiBus, CS: OutputPin> NorFlash for W25qNorFlash<BUS, CS> {
    const WRITE_SIZE: usize = 1;
    const ERASE_SIZE: usize = SECTOR_SIZE as usize;

    async fn erase(&mut self, mut from: u32, to: u32) -> Result<(), Self::Error> {
        while from < to {
            let remaining = to - from;
            let (opcode, opcode_4b, size) = if remaining >= BLOCK64_SIZE && from.is_multiple_of(BLOCK64_SIZE) {
                (CMD_BLOCK_ERASE_64K, CMD_BLOCK_ERASE_64K_4B, BLOCK64_SIZE)
            } else {
                (CMD_SECTOR_ERASE, CMD_SECTOR_ERASE_4B, SECTOR_SIZE)
            };
            self.write_enable().await?;
            let (cmd, cmd_len) = self.address_header(opcode, opcode_4b, from);
            self.transaction(&cmd[..cmd_len], &mut []).await?;
            self.verify_wip_set().await?;
            self.wait_wip().await?;
            from += size;
        }
        Ok(())
    }

    async fn write(&mut self, mut offset: u32, mut bytes: &[u8]) -> Result<(), Self::Error> {
        while !bytes.is_empty() {
            let page_offset = offset & (PAGE_SIZE - 1);
            let chunk = bytes.len().min((PAGE_SIZE - page_offset) as usize);
            self.page_program(offset, &bytes[..chunk]).await?;
            offset += chunk as u32;
            bytes = &bytes[chunk..];
        }
        Ok(())
    }
}

impl<BUS: SpiBus, CS: OutputPin> MultiwriteNorFlash for W25qNorFlash<BUS, CS> {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::vec::Vec;

    use embedded_storage_async::nor_flash::{NorFlash, ReadNorFlash};

    use super::*;
    use crate::test_support::test_block_on;

    /// Fake 25-series chip behind a buffered `SpiBus`: `write` only queues
    /// bytes, so they are guaranteed on the wire only after `flush` or a
    /// subsequent `read`.
    #[derive(Default)]
    struct Chip {
        frame: Vec<u8>,
        frames: Vec<Vec<u8>>,
        status: u8,
        unflushed: bool,
        cs_released_unflushed: usize,
    }

    struct Bus(Rc<RefCell<Chip>>);
    struct Cs(Rc<RefCell<Chip>>);

    impl embedded_hal::spi::ErrorType for Bus {
        type Error = core::convert::Infallible;
    }

    impl SpiBus for Bus {
        async fn read(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
            let mut chip = self.0.borrow_mut();
            chip.unflushed = false;
            if chip.frame == [CMD_READ_STATUS] {
                buf.fill(chip.status);
                chip.status &= !0x01;
            } else {
                buf.fill(0xFF);
            }
            Ok(())
        }

        async fn write(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
            let mut chip = self.0.borrow_mut();
            chip.frame.extend_from_slice(buf);
            chip.unflushed = true;
            Ok(())
        }

        async fn transfer(&mut self, _: &mut [u8], _: &[u8]) -> Result<(), Self::Error> {
            unimplemented!()
        }

        async fn transfer_in_place(&mut self, _: &mut [u8]) -> Result<(), Self::Error> {
            unimplemented!()
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            self.0.borrow_mut().unflushed = false;
            Ok(())
        }
    }

    impl embedded_hal::digital::ErrorType for Cs {
        type Error = core::convert::Infallible;
    }

    impl OutputPin for Cs {
        fn set_low(&mut self) -> Result<(), Self::Error> {
            self.0.borrow_mut().frame.clear();
            Ok(())
        }

        fn set_high(&mut self) -> Result<(), Self::Error> {
            let mut chip = self.0.borrow_mut();
            if chip.unflushed {
                chip.cs_released_unflushed += 1;
            }
            let frame = core::mem::take(&mut chip.frame);
            match frame.first() {
                Some(&CMD_WRITE_ENABLE) => chip.status |= 0x02,
                Some(&(CMD_PAGE_PROGRAM | CMD_SECTOR_ERASE | CMD_BLOCK_ERASE_64K | CMD_WRITE_STATUS)) => {
                    chip.status = 0x01
                }
                _ => {}
            }
            if !frame.is_empty() {
                chip.frames.push(frame);
            }
            Ok(())
        }
    }

    fn flash(capacity: u32) -> (W25qNorFlash<Bus, Cs>, Rc<RefCell<Chip>>) {
        let chip = Rc::new(RefCell::new(Chip::default()));
        let flash = W25qNorFlash::new(Bus(chip.clone()), Cs(chip.clone()), capacity);
        (flash, chip)
    }

    #[test]
    fn w25q_32mib_part_addresses_offsets_beyond_16mib_distinctly() {
        test_block_on(async {
            let (mut flash, chip) = flash(32 << 20);
            assert_eq!(flash.capacity(), 32 << 20);

            let mut buf = [0u8; 1];
            flash.read(0, &mut buf).await.unwrap();
            let low = core::mem::take(&mut chip.borrow_mut().frames);
            flash.read(16 << 20, &mut buf).await.unwrap();
            let high = core::mem::take(&mut chip.borrow_mut().frames);

            assert_ne!(
                low, high,
                "reads from two distinct advertised offsets put identical frames on the bus"
            );
        });
    }

    #[test]
    fn w25q_completes_spi_transfer_before_releasing_chip_select() {
        test_block_on(async {
            let (mut flash, chip) = flash(1 << 20);

            flash.write(0x100, &[0xA5; 4]).await.unwrap();
            flash.erase(0, SECTOR_SIZE).await.unwrap();
            flash.clear_block_protection().await.unwrap();

            let chip = chip.borrow();
            assert!(
                chip.frames
                    .contains(&std::vec![CMD_PAGE_PROGRAM, 0, 1, 0, 0xA5, 0xA5, 0xA5, 0xA5])
            );
            assert!(chip.frames.contains(&std::vec![CMD_SECTOR_ERASE, 0, 0, 0]));
            assert!(chip.frames.contains(&std::vec![CMD_WRITE_STATUS, 0x00]));
            assert_eq!(
                chip.cs_released_unflushed, 0,
                "chip select deasserted while buffered SPI writes were still pending"
            );
        });
    }
}
