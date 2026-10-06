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

const PAGE_SIZE: u32 = 256;
const SECTOR_SIZE: u32 = 4096;
const BLOCK64_SIZE: u32 = 65536;

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
        self.cs.set_low().ok();
        if let Err(e) = self.bus.write(&[CMD_JEDEC_ID]).await {
            self.cs.set_high().ok();
            return Err(W25qError::Spi(e));
        }
        let mut id = [0u8; 3];
        let res = self.bus.read(&mut id).await;
        self.cs.set_high().ok();
        res.map_err(W25qError::Spi)?;
        Ok(id)
    }

    pub async fn read_status_register(&mut self) -> Result<u8, W25qError<BUS::Error>> {
        self.read_status().await
    }

    pub async fn clear_block_protection(&mut self) -> Result<(), W25qError<BUS::Error>> {
        self.write_enable().await?;
        self.cs.set_low().ok();
        let res = self.bus.write(&[CMD_WRITE_STATUS, 0x00]).await.map_err(W25qError::Spi);
        self.cs.set_high().ok();
        res?;
        self.wait_wip().await
    }

    async fn read_status(&mut self) -> Result<u8, W25qError<BUS::Error>> {
        self.cs.set_low().ok();
        if let Err(e) = self.bus.write(&[CMD_READ_STATUS]).await {
            self.cs.set_high().ok();
            return Err(W25qError::Spi(e));
        }
        let mut status = [0u8; 1];
        let res = self.bus.read(&mut status).await;
        self.cs.set_high().ok();
        res.map_err(W25qError::Spi)?;
        Ok(status[0])
    }

    async fn write_enable(&mut self) -> Result<(), W25qError<BUS::Error>> {
        self.cs.set_low().ok();
        let res = self.bus.write(&[CMD_WRITE_ENABLE]).await.map_err(W25qError::Spi);
        self.cs.set_high().ok();
        res?;
        // Verify Write-Enable-Latch was set
        let status = self.read_status().await?;
        if status & 0x02 == 0 {
            return Err(W25qError::WriteProtect);
        }
        Ok(())
    }

    async fn read_data(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), W25qError<BUS::Error>> {
        self.wait_wip().await?;
        let cmd = [CMD_READ, (addr >> 16) as u8, (addr >> 8) as u8, addr as u8];
        self.cs.set_low().ok();
        if let Err(e) = self.bus.write(&cmd).await {
            self.cs.set_high().ok();
            return Err(W25qError::Spi(e));
        }
        let res = self.bus.read(buf).await.map_err(W25qError::Spi);
        self.cs.set_high().ok();
        res
    }

    async fn page_program(&mut self, addr: u32, data: &[u8]) -> Result<(), W25qError<BUS::Error>> {
        self.write_enable().await?;
        let header_len = 4;
        let total = header_len + data.len();
        let mut tx_buf = [0u8; 4 + PAGE_SIZE as usize];
        tx_buf[0] = CMD_PAGE_PROGRAM;
        tx_buf[1] = (addr >> 16) as u8;
        tx_buf[2] = (addr >> 8) as u8;
        tx_buf[3] = addr as u8;
        tx_buf[header_len..total].copy_from_slice(data);
        self.cs.set_low().ok();
        let res = self.bus.write(&tx_buf[..total]).await.map_err(W25qError::Spi);
        self.cs.set_high().ok();
        res?;
        self.wait_wip().await
    }

    async fn sector_erase(&mut self, addr: u32) -> Result<(), W25qError<BUS::Error>> {
        self.write_enable().await?;
        let cmd = [CMD_SECTOR_ERASE, (addr >> 16) as u8, (addr >> 8) as u8, addr as u8];
        self.cs.set_low().ok();
        let res = self.bus.write(&cmd).await.map_err(W25qError::Spi);
        self.cs.set_high().ok();
        res?;
        self.verify_wip_set().await?;
        self.wait_wip().await
    }

    async fn block_erase_64k(&mut self, addr: u32) -> Result<(), W25qError<BUS::Error>> {
        self.write_enable().await?;
        let cmd = [CMD_BLOCK_ERASE_64K, (addr >> 16) as u8, (addr >> 8) as u8, addr as u8];
        self.cs.set_low().ok();
        let res = self.bus.write(&cmd).await.map_err(W25qError::Spi);
        self.cs.set_high().ok();
        res?;
        self.verify_wip_set().await?;
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
            if remaining >= BLOCK64_SIZE && from.is_multiple_of(BLOCK64_SIZE) {
                self.block_erase_64k(from).await?;
                from += BLOCK64_SIZE;
            } else {
                self.sector_erase(from).await?;
                from += SECTOR_SIZE;
            }
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
