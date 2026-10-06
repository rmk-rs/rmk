//! PMW3610 Low-Power Mouse Sensor Driver
//!
//! Ported from the Zephyr driver implementation:
//! https://github.com/zephyrproject-rtos/zephyr/blob/d31c6e95033fd6b3763389edba6a655245ae1328/drivers/input/input_pmw3610.c

use embassy_time::{Duration, Instant, Timer};
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal_async::digital::Wait;
use embedded_hal_async::spi::SpiBus;

pub use crate::driver::bitbang_spi::{BitBangError, BitBangSpiBus};
use crate::input_device::pointing::{InitState, MotionData, PointingDevice, PointingDriver, PointingDriverError};

// ============================================================================
// Page 0 registers
// ============================================================================
const PMW3610_PROD_ID: u8 = 0x00;
const PMW3610_MOTION: u8 = 0x02;
const PMW3610_DELTA_XY_H: u8 = 0x05;
const PMW3610_PERFORMANCE: u8 = 0x11;
const PMW3610_BURST_READ: u8 = 0x12;
const PMW3610_RUN_DOWNSHIFT: u8 = 0x1b;
const PMW3610_REST1_RATE: u8 = 0x1c;
const PMW3610_REST1_DOWNSHIFT: u8 = 0x1d;
const PMW3610_OBSERVATION1: u8 = 0x2d;
const PMW3610_SMART_MODE: u8 = 0x32;
const PMW3610_POWER_UP_RESET: u8 = 0x3a;
const PMW3610_SPI_CLK_ON_REQ: u8 = 0x41;
const PMW3610_SPI_PAGE0: u8 = 0x7f;

// ============================================================================
// Page 1 registers
// ============================================================================
const PMW3610_RES_STEP: u8 = 0x05;
const PMW3610_SPI_PAGE1: u8 = 0x7f;

// ============================================================================
// Burst register offsets
// ============================================================================
const BURST_MOTION: usize = 0;
const BURST_DELTA_X_L: usize = 1;
const BURST_DELTA_Y_L: usize = 2;
const BURST_DELTA_XY_H: usize = 3;
const BURST_DELTA_X_H: usize = 4;
const BURST_SHUTTER_HI: usize = 5;
const BURST_SHUTTER_LO: usize = 6;

const BURST_DATA_LEN_NORMAL: usize = BURST_DELTA_XY_H + 1;
const BURST_DATA_LEN_SMART: usize = BURST_SHUTTER_LO + 1;

// ============================================================================
// Init sequence values
// ============================================================================
const OBSERVATION1_INIT_MASK: u8 = 0x0f;
const PERFORMANCE_INIT: u8 = 0x0d;
const RUN_DOWNSHIFT_INIT: u8 = 0x04;
const REST1_RATE_INIT: u8 = 0x04;
const REST1_DOWNSHIFT_INIT: u8 = 0x0f;

// ============================================================================
// Constants
// ============================================================================
const PRODUCT_ID_PMW3610: u8 = 0x3e;
const SPI_WRITE: u8 = 0x80;
const MOTION_STATUS_MOTION: u8 = 0x80;
const SPI_CLOCK_ON_REQ_ON: u8 = 0xba;
const SPI_CLOCK_ON_REQ_OFF: u8 = 0xb5;
const RES_STEP_SWAP_XY_BIT: u8 = 7;
const RES_STEP_INV_X_BIT: u8 = 6;
const RES_STEP_INV_Y_BIT: u8 = 5;
const RES_STEP_RES_MASK: u8 = 0x1f;
const PERFORMANCE_FMODE_MASK: u8 = 0x0f << 4;
const PERFORMANCE_FMODE_NORMAL: u8 = 0x00 << 4;
const PERFORMANCE_FMODE_FORCE_AWAKE: u8 = 0x0f << 4;
const POWER_UP_RESET_VAL: u8 = 0x5a;
const SPI_PAGE0_1: u8 = 0xff;
const SPI_PAGE1_0: u8 = 0x00;
const SHUTTER_SMART_THRESHOLD: u16 = 45;
const SMART_MODE_ENABLE: u8 = 0x00;
const SMART_MODE_DISABLE: u8 = 0x80;

const PMW3610_DATA_SIZE_BITS: usize = 12;

// Timing constants
const RESET_DELAY_MS: u64 = 10;
const INIT_OBSERVATION_DELAY_MS: u64 = 10;
const CLOCK_ON_DELAY_US: u64 = 300;

// SPI timing constants (from PMW3610 datasheet)
/// NCS to SCLK active;
/// Delay from last NCS falling edge to 1st SCK rising edge
const T_NCS_SCLK_NS: u64 = 120;
/// SPI read address-data delay;
/// from rising SCLK for last bit of the address byte, to falling SCLK for the 1st bit of data being read.
const T_SRAD_US: u64 = 4;
/// SPI time between read and subsequent commands;
/// from rising SCLK for last bit of the 1st data byte, to falling SCLK for the 1st bit of data being read.
const T_SRX_NS: u64 = 250;
/// SPI time between write command;
/// From rising SCLK for last bit of the first data byte, to rising SCLK for last bit of the second data byte.
/// It's actually 20 us before read and 30 us before write, but we don't distinguish between write and read commands here, and use the larger of the two.
const T_SWX_US: u64 = 30;
/// SCLK to NCS inactive for SDIO write;
/// From last SCLK falling edge to NCS rising edge, for valid SDIO data transfer
const T_SCLK_NCS_W_US: u64 = 10;
/// SCLK to NCS inactive for SDIO read;
/// From last SCLK falling edge to NCS rising edge, for valid SDIO data transfer
const T_SCLK_NCS_R_NS: u64 = 120;
/// NCS inactive after motion burst;
/// Minimum NCS inactive time after motion burst before next SPI usage
const T_BEXIT_NS: u64 = 250;

// Resolution constants
const RES_STEP: u16 = 200;
const RES_MIN: u16 = 200;
const RES_MAX: u16 = 3200;

/// PMW3610 configuration
#[derive(Clone)]
pub struct Pmw3610Config {
    /// CPI resolution (200-3200, step 200). Set to -1 to use default.
    pub res_cpi: i16,
    /// Invert X axis
    pub invert_x: bool,
    /// Invert Y axis
    pub invert_y: bool,
    /// Swap X and Y axes
    pub swap_xy: bool,
    /// Force awake mode (disable power saving)
    pub force_awake: bool,
    /// Enable smart mode for better tracking on shiny surfaces
    pub smart_mode: bool,
}

impl Default for Pmw3610Config {
    fn default() -> Self {
        Self {
            res_cpi: -1,
            invert_x: false,
            invert_y: false,
            swap_xy: false,
            force_awake: false,
            smart_mode: false,
        }
    }
}

/// PMW3610 error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Pmw3610Error {
    /// SPI communication error
    Spi,
    /// Invalid product ID detected
    InvalidProductId(u8),
    /// Initialization failed
    InitFailed,
    /// Invalid CPI value
    InvalidCpi,
}

impl From<Pmw3610Error> for PointingDriverError {
    fn from(err: Pmw3610Error) -> Self {
        match err {
            Pmw3610Error::Spi => PointingDriverError::Spi,
            Pmw3610Error::InvalidProductId(id) => PointingDriverError::InvalidProductId(id),
            Pmw3610Error::InitFailed => PointingDriverError::InitFailed,
            Pmw3610Error::InvalidCpi => PointingDriverError::InvalidCpi,
        }
    }
}

/// PMW3610 driver using embedded-hal SPI traits
pub struct Pmw3610<SPI: SpiBus, CS: OutputPin, MOTION: InputPin + Wait> {
    id: u8,
    spi: SPI,
    cs: CS,
    motion_gpio: Option<MOTION>,
    config: Pmw3610Config,
    smart_flag: bool,
}

impl<SPI: SpiBus, CS: OutputPin, MOTION: InputPin + Wait> Pmw3610<SPI, CS, MOTION> {
    /// Create a new PMW3610 driver instance
    pub fn new(id: u8, spi: SPI, cs: CS, motion_gpio: Option<MOTION>, config: Pmw3610Config) -> Self {
        Self {
            id,
            spi,
            cs,
            motion_gpio,
            config,
            smart_flag: false,
        }
    }

    /// Write the FMODE bits of the performance register.
    async fn write_force_awake(&mut self, enable: bool) -> Result<(), PointingDriverError> {
        let mut val = self.read_reg(PMW3610_PERFORMANCE).await?;
        val &= !PERFORMANCE_FMODE_MASK;
        if enable {
            val |= PERFORMANCE_FMODE_FORCE_AWAKE;
        } else {
            val |= PERFORMANCE_FMODE_NORMAL;
        }

        self.spi_clk_on().await?;
        self.write_reg(PMW3610_PERFORMANCE, val).await?;
        self.spi_clk_off().await?;

        Ok(())
    }

    async fn read_reg(&mut self, addr: u8) -> Result<u8, Pmw3610Error> {
        let _ = self.cs.set_low();
        Timer::after(Duration::from_nanos(T_NCS_SCLK_NS)).await;

        self.spi.write(&[addr & 0x7f]).await.map_err(|_| Pmw3610Error::Spi)?;

        Timer::after(Duration::from_micros(T_SRAD_US)).await;

        let mut value = [0u8];
        self.spi.read(&mut value).await.map_err(|_| Pmw3610Error::Spi)?;

        Timer::after(Duration::from_nanos(T_SCLK_NCS_R_NS)).await;
        let _ = self.cs.set_high();

        Timer::after(Duration::from_nanos(T_SRX_NS)).await;

        Ok(value[0])
    }

    async fn read_burst(&mut self, addr: u8, data: &mut [u8]) -> Result<(), Pmw3610Error> {
        let _ = self.cs.set_low();
        Timer::after(Duration::from_nanos(T_NCS_SCLK_NS)).await;

        self.spi.write(&[addr & 0x7f]).await.map_err(|_| Pmw3610Error::Spi)?;

        Timer::after(Duration::from_micros(T_SRAD_US)).await;

        self.spi.read(data).await.map_err(|_| Pmw3610Error::Spi)?;

        Timer::after(Duration::from_nanos(T_SCLK_NCS_R_NS)).await;
        let _ = self.cs.set_high();

        Timer::after(Duration::from_nanos(T_BEXIT_NS)).await;

        Ok(())
    }

    async fn write_reg(&mut self, addr: u8, value: u8) -> Result<(), Pmw3610Error> {
        let _ = self.cs.set_low();
        Timer::after(Duration::from_nanos(T_NCS_SCLK_NS)).await;

        self.spi
            .write(&[addr | SPI_WRITE, value])
            .await
            .map_err(|_| Pmw3610Error::Spi)?;

        Timer::after(Duration::from_micros(T_SCLK_NCS_W_US)).await;
        let _ = self.cs.set_high();

        Timer::after(Duration::from_micros(T_SWX_US)).await;

        Ok(())
    }

    async fn spi_clk_on(&mut self) -> Result<(), Pmw3610Error> {
        self.write_reg(PMW3610_SPI_CLK_ON_REQ, SPI_CLOCK_ON_REQ_ON).await?;
        Timer::after(Duration::from_micros(CLOCK_ON_DELAY_US)).await;
        Ok(())
    }

    async fn spi_clk_off(&mut self) -> Result<(), Pmw3610Error> {
        self.write_reg(PMW3610_SPI_CLK_ON_REQ, SPI_CLOCK_ON_REQ_OFF).await
    }

    async fn configure(&mut self) -> Result<(), Pmw3610Error> {
        self.write_reg(PMW3610_POWER_UP_RESET, POWER_UP_RESET_VAL).await?;
        Timer::after(Duration::from_millis(RESET_DELAY_MS)).await;

        let val = self.read_reg(PMW3610_PROD_ID).await?;
        if val != PRODUCT_ID_PMW3610 {
            error!("Invalid product id: {:#02x}", val);
            return Err(Pmw3610Error::InvalidProductId(val));
        }
        info!("PMW3610 detected, product ID: {:#02x}", val);

        self.spi_clk_on().await?;

        self.write_reg(PMW3610_OBSERVATION1, 0).await?;
        Timer::after(Duration::from_millis(INIT_OBSERVATION_DELAY_MS)).await;

        let val = self.read_reg(PMW3610_OBSERVATION1).await?;
        if (val & OBSERVATION1_INIT_MASK) != OBSERVATION1_INIT_MASK {
            error!("Unexpected OBSERVATION1 value: {:#02x}", val);
            return Err(Pmw3610Error::InitFailed);
        }

        for reg in PMW3610_MOTION..=PMW3610_DELTA_XY_H {
            self.read_reg(reg).await?;
        }

        self.write_reg(PMW3610_PERFORMANCE, PERFORMANCE_INIT).await?;
        self.write_reg(PMW3610_RUN_DOWNSHIFT, RUN_DOWNSHIFT_INIT).await?;
        self.write_reg(PMW3610_REST1_RATE, REST1_RATE_INIT).await?;
        self.write_reg(PMW3610_REST1_DOWNSHIFT, REST1_DOWNSHIFT_INIT).await?;

        self.write_reg(PMW3610_SPI_PAGE0, SPI_PAGE0_1).await?;

        let mut res_step_val = self.read_reg(PMW3610_RES_STEP).await?;

        if self.config.swap_xy {
            res_step_val |= 1 << RES_STEP_SWAP_XY_BIT;
        } else {
            res_step_val &= !(1 << RES_STEP_SWAP_XY_BIT);
        }

        if self.config.invert_x {
            res_step_val |= 1 << RES_STEP_INV_X_BIT;
        } else {
            res_step_val &= !(1 << RES_STEP_INV_X_BIT);
        }

        if self.config.invert_y {
            res_step_val |= 1 << RES_STEP_INV_Y_BIT;
        } else {
            res_step_val &= !(1 << RES_STEP_INV_Y_BIT);
        }

        self.write_reg(PMW3610_RES_STEP, res_step_val).await?;
        self.write_reg(PMW3610_SPI_PAGE1, SPI_PAGE1_0).await?;

        self.spi_clk_off().await?;

        if self.config.res_cpi > 0 {
            self.set_resolution(self.config.res_cpi as u16)
                .await
                .map_err(|_| Pmw3610Error::Spi)?;
        }

        self.write_force_awake(self.config.force_awake)
            .await
            .map_err(|_| Pmw3610Error::Spi)?;

        info!("PMW3610 initialized successfully");
        Ok(())
    }

    fn sign_extend(value: u16, bits: usize) -> i16 {
        let sign_bit = 1 << bits;
        if value & sign_bit != 0 {
            (value | !((1 << (bits + 1)) - 1)) as i16
        } else {
            value as i16
        }
    }
}

impl<SPI, CS, MOTION> PointingDriver for Pmw3610<SPI, CS, MOTION>
where
    SPI: SpiBus,
    CS: OutputPin,
    MOTION: InputPin + Wait,
{
    type MOTION = MOTION;

    /// Initialize the sensor (public API)
    async fn init(&mut self) -> Result<(), PointingDriverError> {
        let _ = self.cs.set_high();
        Timer::after(Duration::from_millis(1)).await;

        self.configure().await?;
        Ok(())
    }

    /// Read motion data from the sensor
    async fn read_motion(&mut self) -> Result<MotionData, PointingDriverError> {
        let burst_data_len = if self.config.smart_mode {
            BURST_DATA_LEN_SMART
        } else {
            BURST_DATA_LEN_NORMAL
        };

        let mut burst_data = [0u8; BURST_DATA_LEN_SMART];
        self.read_burst(PMW3610_BURST_READ, &mut burst_data[..burst_data_len])
            .await?;

        if (burst_data[BURST_MOTION] & MOTION_STATUS_MOTION) == 0x00 {
            return Ok(MotionData::default());
        }

        let x = ((burst_data[BURST_DELTA_XY_H] as u16) << 4) & 0xf00 | (burst_data[BURST_DELTA_X_L] as u16);
        let y = ((burst_data[BURST_DELTA_XY_H] as u16) << 8) & 0xf00 | (burst_data[BURST_DELTA_Y_L] as u16);

        let dx = Self::sign_extend(x, PMW3610_DATA_SIZE_BITS - 1);
        let dy = Self::sign_extend(y, PMW3610_DATA_SIZE_BITS - 1);

        if self.config.smart_mode {
            let shutter_val = ((burst_data[BURST_SHUTTER_HI] as u16) << 8) | (burst_data[BURST_SHUTTER_LO] as u16);

            if self.smart_flag && shutter_val < SHUTTER_SMART_THRESHOLD {
                self.spi_clk_on().await?;
                self.write_reg(PMW3610_SMART_MODE, SMART_MODE_ENABLE)
                    .await
                    .map_err(|_| PointingDriverError::Spi)?;
                self.spi_clk_off().await?;
                self.smart_flag = false;
            } else if !self.smart_flag && shutter_val > SHUTTER_SMART_THRESHOLD {
                self.spi_clk_on().await?;
                self.write_reg(PMW3610_SMART_MODE, SMART_MODE_DISABLE)
                    .await
                    .map_err(|_| PointingDriverError::Spi)?;
                self.spi_clk_off().await?;
                self.smart_flag = true;
            }
        }

        Ok(MotionData { dx, dy })
    }

    /// Release the awake hold while the keyboard sleeps, restore it on wake.
    /// Without `force_awake` the sensor manages its own power and this is a no-op.
    async fn set_low_power(&mut self, enabled: bool) -> Result<(), PointingDriverError> {
        if !self.config.force_awake {
            return Ok(());
        }
        debug!(
            "PMW3610: {} force awake",
            if enabled { "releasing" } else { "restoring" }
        );
        self.write_force_awake(!enabled).await
    }

    /// Check if motion is pending (motion GPIO is active low)
    fn motion_pending(&mut self) -> bool {
        match &mut self.motion_gpio {
            Some(gpio) => gpio.is_low().unwrap_or(true),
            None => true,
        }
    }

    fn motion_gpio(&mut self) -> Option<&mut MOTION> {
        self.motion_gpio.as_mut()
    }

    /// Set sensor resolution in CPI (200-3200, step 200)
    async fn set_resolution(&mut self, cpi: u16) -> Result<(), PointingDriverError> {
        if !(RES_MIN..=RES_MAX).contains(&cpi) {
            return Err(PointingDriverError::InvalidCpi);
        }

        self.spi_clk_on().await?;

        self.write_reg(PMW3610_SPI_PAGE0, SPI_PAGE0_1).await?;

        let mut val = self.read_reg(PMW3610_RES_STEP).await?;
        val &= !RES_STEP_RES_MASK;
        val |= (cpi / RES_STEP) as u8;

        self.write_reg(PMW3610_RES_STEP, val).await?;
        self.write_reg(PMW3610_SPI_PAGE1, SPI_PAGE1_0).await?;

        self.spi_clk_off().await?;

        debug!("PMW3610: Resolution set to {} CPI", cpi);
        Ok(())
    }
}

impl<SPI, CS, MOTION> PointingDevice<Pmw3610<SPI, CS, MOTION>>
where
    SPI: SpiBus,
    CS: OutputPin,
    MOTION: InputPin + Wait,
{
    const DEFAULT_POLL_INTERVAL_US: u64 = 500;
    const DEFAULT_REPORT_HZ: u16 = 125;

    /// Create a new PMW3610 device
    pub fn new(id: u8, spi: SPI, cs: CS, motion_gpio: Option<MOTION>, sensor_config: Pmw3610Config) -> Self {
        Self::with_poll_interval_and_report_hz(
            id,
            spi,
            cs,
            motion_gpio,
            sensor_config,
            Self::DEFAULT_POLL_INTERVAL_US,
            Self::DEFAULT_REPORT_HZ,
        )
    }

    /// Create a new PMW3610 device with custom report rate (Hz)
    pub fn with_report_hz(
        id: u8,
        spi: SPI,
        cs: CS,
        motion_gpio: Option<MOTION>,
        sensor_config: Pmw3610Config,
        report_hz: u16,
    ) -> Self {
        Self::with_poll_interval_and_report_hz(
            id,
            spi,
            cs,
            motion_gpio,
            sensor_config,
            Self::DEFAULT_POLL_INTERVAL_US,
            report_hz,
        )
    }

    /// Create a new PMW3610 device with custom poll interval
    pub fn with_poll_interval(
        id: u8,
        spi: SPI,
        cs: CS,
        motion_gpio: Option<MOTION>,
        sensor_config: Pmw3610Config,
        poll_interval_us: u64,
    ) -> Self {
        Self::with_poll_interval_and_report_hz(
            id,
            spi,
            cs,
            motion_gpio,
            sensor_config,
            poll_interval_us,
            Self::DEFAULT_REPORT_HZ,
        )
    }

    /// Create a new PMW3610 device with custom poll interval and report rate
    pub fn with_poll_interval_and_report_hz(
        id: u8,
        spi: SPI,
        cs: CS,
        motion_gpio: Option<MOTION>,
        sensor_config: Pmw3610Config,
        poll_interval_us: u64,
        report_hz: u16,
    ) -> Self {
        let report_interval = Duration::from_hz(report_hz as u64);

        // Polling should be more frequent than reporting
        let poll_interval = Duration::from_micros(poll_interval_us).min(report_interval);

        Self {
            id,
            sensor: Pmw3610::new(id, spi, cs, motion_gpio, sensor_config),
            init_state: InitState::Pending,
            poll_interval,
            report_interval,
            last_poll: Instant::MIN,
            last_report: Instant::MIN,
            accumulated_x: 0,
            accumulated_y: 0,
        }
    }
}
