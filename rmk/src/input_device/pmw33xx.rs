// PMW3360/ PMW3389 Mouse Sensor Driver
//
// Ported from kot149s PMW3610 driver:
// https://github.com/kot149/pmw3610-rs
// Which is ported from the Zephyr driver implementation:
// https://github.com/zephyrproject-rtos/zephyr/blob/d31c6e95033fd6b3763389edba6a655245ae1328/drivers/input/input_pmw3610.c

use embassy_time::{Duration, Instant, Timer};
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal_async::digital::Wait;
use embedded_hal_async::spi::SpiBus;

use crate::input_device::pointing::{
    InitState, MotionData, PointingDevice, PointingDriver, PointingDriverError, duration_from_report_hz,
};

// Burst report offsets
const BURST_MOTION_FLAGS: usize = 0;
#[allow(dead_code)]
const BURST_OBSERVATION: usize = 1;
const BURST_DELTA_X_L: usize = 2;
const BURST_DELTA_X_H: usize = 3;
const BURST_DELTA_Y_L: usize = 4;
const BURST_DELTA_Y_H: usize = 5;
const BURST_DATA_LEN: usize = 6;

// Constants
const SPI_WRITE: u8 = 0x80; // BIT(7)
const MOTION_STATUS_MOTION: u8 = 0x80; // BIT(7)
const MOTION_STATUS_LIFTED: u8 = 0x08; // BIT(4)
const POWER_UP_RESET_VAL: u8 = 0x5a;

// Timing constants
const RESET_DELAY_MS: u64 = 50;

// SPI timing constants (from PMW3360 datasheet)
/// NCS to SCLK active;
/// Delay from last NCS falling edge to 1st SCK rising edge
const T_NCS_SCLK_NS: u64 = 120;
/// SPI read address-data delay;
/// from rising SCLK for last bit of the address byte, to falling SCLK for the 1st bit of data being read.
const T_SRAD_US: u64 = 160;
/// SPI read motion burst delay;
/// From  rising  SCLK  for  last  bit  of  the  address byte, to falling SCLK for first bit
/// of data being read. Applicable for Burst Mode Motion Read only.
const T_SRAD_MOTBR_US: u64 = 35;
/// SPI time between read and subsequent commands;
/// from rising SCLK for last bit of the 1st data byte, to falling SCLK for the 1st bit of data being read.
const T_SRX_US: u64 = 20;
/// SPI time between write command;
/// From rising SCLK for last bit of the first data byte, to rising SCLK for last bit of the second data byte.
const T_SWX_US: u64 = 180;
/// SCLK to NCS inactive for SDIO write;
/// From last SCLK falling edge to NCS rising edge, for valid SDIO data transfer
const T_SCLK_NCS_W_US: u64 = 35;
/// SCLK to NCS inactive for SDIO read;
/// From last SCLK falling edge to NCS rising edge, for valid SDIO data transfer
const T_SCLK_NCS_R_NS: u64 = 120;
/// NCS inactive after motion burst;
/// Minimum NCS inactive time after motion burst before next SPI usage
const T_BEXIT_NS: u64 = 500;
/// Timeout between SROM data bursts
const T_BRSEP_US: u64 = 15;

// Rotational transform angle limits
const ROT_MIN: i8 = -127;
const ROT_MAX: i8 = 127;

// PMW3360/ 3389 registers are almost alike. This Enum contains a few extra registers that are not available for the 3360
#[allow(dead_code)]
#[derive(Eq, PartialEq, Debug)]
enum Register {
    ProductId,
    RevisionId,
    Motion,
    DeltaXL,
    DeltaXH,
    DeltaYL,
    DeltaYH,
    Squal,
    RawDataSum,
    MaximumRawData,
    MinimumRawData,
    ShutterLower,
    ShutterUpper,
    Control,
    Config1L,
    Config1H,
    Config2,
    AngleTune,
    FrameCapture,
    SromEnable,
    RunDownshift,
    Rest1RateLower,
    Rest1RateUpper,
    Rest1Downshift,
    Rest2RateLower,
    Rest2RateUpper,
    Rest2Downshift,
    Rest3RateLower,
    Rest3RateUpper,
    Observation,
    DataOutLower,
    DataOutUpper,
    RawDataDump,
    SromId,
    MinSqRun,
    RawDataThreshold,
    Control2,
    Config5L,
    Config5H,
    PowerUpReset,
    Shutdown,
    InverseProductId,
    LiftCutoffTune3,
    AngleSnap,
    LiftCutoffTune1,
    MotionBurst,
    LiftCutoffTuneTimeout,
    LiftCutoffTuneMinLength,
    SromLoadBurst,
    LiftConfig,
    RawDataBurst,
    LiftCutoffTune2,
    LiftCutoffCalTimeout,
    LiftCutoffCalMinLen,
    PWMPeriodCnt,
    PwmWidthCnt,
}

impl Register {
    fn value(&self) -> u8 {
        match self {
            Register::ProductId => 0x00,
            Register::RevisionId => 0x01,
            Register::Motion => 0x02,
            Register::DeltaXL => 0x03,
            Register::DeltaXH => 0x04,
            Register::DeltaYL => 0x05,
            Register::DeltaYH => 0x06,
            Register::Squal => 0x07,
            Register::RawDataSum => 0x08,
            Register::MaximumRawData => 0x09,
            Register::MinimumRawData => 0x0a,
            Register::ShutterLower => 0x0b,
            Register::ShutterUpper => 0x0c,
            Register::Control => 0x0d,
            Register::Config1L => 0x0e,
            Register::Config1H => 0x0f,
            Register::Config2 => 0x10,
            Register::AngleTune => 0x11,
            Register::FrameCapture => 0x12,
            Register::SromEnable => 0x13,
            Register::RunDownshift => 0x14,
            Register::Rest1RateLower => 0x15,
            Register::Rest1RateUpper => 0x16,
            Register::Rest1Downshift => 0x17,
            Register::Rest2RateLower => 0x18,
            Register::Rest2RateUpper => 0x19,
            Register::Rest2Downshift => 0x1a,
            Register::Rest3RateLower => 0x1b,
            Register::Rest3RateUpper => 0x1c,
            Register::Observation => 0x24,
            Register::DataOutLower => 0x25,
            Register::DataOutUpper => 0x26,
            Register::RawDataDump => 0x29,
            Register::SromId => 0x2a,
            Register::MinSqRun => 0x2b,
            Register::RawDataThreshold => 0x2c,
            Register::Control2 => 0x2d,
            Register::Config5L => 0x2f,
            Register::Config5H => 0x2f,
            Register::PowerUpReset => 0x3a,
            Register::Shutdown => 0x3b,
            Register::InverseProductId => 0x3f,
            Register::LiftCutoffTune3 => 0x41,
            Register::AngleSnap => 0x42,
            Register::LiftCutoffTune1 => 0x4a,
            Register::MotionBurst => 0x50,
            Register::LiftCutoffTuneTimeout => 0x58,
            Register::LiftCutoffTuneMinLength => 0x5a,
            Register::SromLoadBurst => 0x62,
            Register::LiftConfig => 0x63,
            Register::RawDataBurst => 0x64,
            Register::LiftCutoffTune2 => 0x65,
            Register::LiftCutoffCalTimeout => 0x71,
            Register::LiftCutoffCalMinLen => 0x72,
            Register::PWMPeriodCnt => 0x73,
            Register::PwmWidthCnt => 0x74,
        }
    }
}

pub trait Pmw33xxSpec {
    // sensor type (60 or 89)
    const TYPENAME: &str;
    const PRODUCT_ID: u8;
    // firmware signature
    const FW_SIG_PID: u8;
    const FW_SIG_INV_PID: u8;
    // resolution constants
    const RES_MIN: u16;
    const RES_MAX: u16;
    const RES_STEP: u16;

    /// Write resolution register
    async fn write_resolution<SPI, CS, MOTION, SPEC>(
        driver: &mut Pmw33xx<'_, SPI, CS, MOTION, SPEC>,
        cpi: u16,
    ) -> Result<(), Pmw33xxError>
    where
        SPI: SpiBus,
        CS: OutputPin,
        MOTION: InputPin + Wait,
        SPEC: Pmw33xxSpec;
}

pub struct Pmw3360Spec;

impl Pmw33xxSpec for Pmw3360Spec {
    const TYPENAME: &str = "60";
    const PRODUCT_ID: u8 = 0x42;
    const FW_SIG_PID: u8 = Self::PRODUCT_ID;
    const FW_SIG_INV_PID: u8 = 0xBD;
    const RES_MIN: u16 = 100;
    const RES_MAX: u16 = 12000;
    const RES_STEP: u16 = 100;

    async fn write_resolution<SPI, CS, MOTION, SPEC>(
        driver: &mut Pmw33xx<'_, SPI, CS, MOTION, SPEC>,
        cpi: u16,
    ) -> Result<(), Pmw33xxError>
    where
        SPI: SpiBus,
        CS: OutputPin,
        MOTION: InputPin + Wait,
        SPEC: Pmw33xxSpec,
    {
        driver
            .write_reg(Register::Config1H, ((cpi / Self::RES_STEP) - 1) as u8)
            .await?;
        Ok(())
    }
}

pub struct Pmw3389Spec;

impl Pmw33xxSpec for Pmw3389Spec {
    const TYPENAME: &str = "89";
    const PRODUCT_ID: u8 = 0x47;
    const FW_SIG_PID: u8 = Self::PRODUCT_ID;
    const FW_SIG_INV_PID: u8 = 0xB8;
    const RES_MIN: u16 = 50;
    const RES_MAX: u16 = 16000;
    const RES_STEP: u16 = 50;

    async fn write_resolution<SPI, CS, MOTION, SPEC>(
        driver: &mut Pmw33xx<'_, SPI, CS, MOTION, SPEC>,
        cpi: u16,
    ) -> Result<(), Pmw33xxError>
    where
        SPI: SpiBus,
        CS: OutputPin,
        MOTION: InputPin + Wait,
        SPEC: Pmw33xxSpec,
    {
        if !(Self::RES_MIN..=Self::RES_MAX).contains(&cpi) {
            return Err(Pmw33xxError::InvalidCpi);
        }

        let cpi = (cpi / Self::RES_STEP) - 1;
        driver.write_reg(Register::Config1H, (cpi >> 8) as u8).await?;
        driver.write_reg(Register::Config1L, (cpi & 0xFF) as u8).await?;

        Ok(())
    }
}

/// PMW33xx configuration
#[derive(Clone)]
pub struct Pmw33xxConfig {
    /// CPI resolution (100-12000, step 100)
    pub res_cpi: u16,
    /// rot_trans_angle (-127 to 127
    pub rot_trans_angle: i8,
    /// liftoff distance
    pub liftoff_dist: u8,
}

impl Default for Pmw33xxConfig {
    fn default() -> Self {
        Self {
            res_cpi: 1600,
            rot_trans_angle: 0,
            liftoff_dist: 0x02,
        }
    }
}

/// PMW33xx error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Pmw33xxError {
    /// SPI communication error
    Spi,
    /// Invalid product ID detected
    InvalidProductId(u8),
    /// Initialization failed
    InitFailed,
    /// Invalid CPI value
    InvalidCpi,
    /// Invalid firmware signature detected
    InvalidFwSignature((u8, u8)),
}

impl From<Pmw33xxError> for PointingDriverError {
    fn from(err: Pmw33xxError) -> Self {
        match err {
            Pmw33xxError::Spi => PointingDriverError::Spi,
            Pmw33xxError::InvalidProductId(id) => PointingDriverError::InvalidProductId(id),
            Pmw33xxError::InitFailed => PointingDriverError::InitFailed,
            Pmw33xxError::InvalidCpi => PointingDriverError::InvalidCpi,
            Pmw33xxError::InvalidFwSignature(sig) => PointingDriverError::InvalidFwSignature(sig),
        }
    }
}

/// PMW33xx driver using embedded-hal SPI traits
pub struct Pmw33xx<'a, SPI: SpiBus, CS: OutputPin, MOTION: InputPin + Wait, SPEC: Pmw33xxSpec> {
    id: u8,
    spi: SPI,
    cs: CS,
    motion_gpio: Option<MOTION>,
    config: Pmw33xxConfig,
    in_burst: bool,
    srom_firmware: Option<&'a [u8]>,
    _spec: core::marker::PhantomData<SPEC>,
}

impl<'a, SPI, CS, MOTION, SPEC> Pmw33xx<'a, SPI, CS, MOTION, SPEC>
where
    SPI: SpiBus,
    CS: OutputPin,
    MOTION: InputPin + Wait,
    SPEC: Pmw33xxSpec,
{
    /// Create a new PMW33xx driver instance
    pub fn new(id: u8, spi: SPI, cs: CS, motion_gpio: Option<MOTION>, config: Pmw33xxConfig) -> Self {
        Self {
            id,
            spi,
            cs,
            motion_gpio,
            config,
            in_burst: false,
            srom_firmware: None,
            _spec: core::marker::PhantomData,
        }
    }

    /// Create a new PMW33xx driver instance with firmware (SROM)
    pub fn new_with_firmware(
        id: u8,
        spi: SPI,
        cs: CS,
        motion_gpio: Option<MOTION>,
        config: Pmw33xxConfig,
        firmware: &'a [u8],
    ) -> Self {
        Self {
            id,
            spi,
            cs,
            motion_gpio,
            config,
            in_burst: false,
            srom_firmware: Some(firmware),
            _spec: core::marker::PhantomData,
        }
    }

    /// Set sensor rotational transform angle (-127 to 127)
    async fn set_rot_trans_angle(&mut self, angle: i8) -> Result<(), PointingDriverError> {
        if !(ROT_MIN..=ROT_MAX).contains(&angle) {
            return Err(PointingDriverError::InvalidRotTransAngle);
        }

        self.write_reg(Register::AngleTune, angle as u8).await?;

        debug!("PMW33{}: Rotational transform angle set to {}", SPEC::TYPENAME, angle);

        Ok(())
    }

    /// Set sensor liftoff distance
    async fn set_liftoff_dist(&mut self, dist: u8) -> Result<(), PointingDriverError> {
        self.write_reg(Register::LiftConfig, dist).await?;

        debug!("PMW33{}: Liftoff distance set to {}", SPEC::TYPENAME, dist);

        Ok(())
    }

    /// Self check firmware signature of the sensor
    pub async fn check_fw_signature(&mut self) -> Result<(), Pmw33xxError> {
        let product_id = self.read_reg(Register::ProductId).await?;
        let inverse_product_id = self.read_reg(Register::InverseProductId).await?;

        if product_id == SPEC::FW_SIG_PID && inverse_product_id == SPEC::FW_SIG_INV_PID {
            Ok(())
        } else {
            error!(
                "PMW33{}: Firmware signature check failed, expected: {}, {} got: {}, {}",
                SPEC::TYPENAME,
                SPEC::FW_SIG_PID,
                SPEC::FW_SIG_INV_PID,
                product_id,
                inverse_product_id
            );
            Err(Pmw33xxError::InvalidFwSignature((product_id, inverse_product_id)))
        }
    }

    async fn read_reg(&mut self, register: Register) -> Result<u8, Pmw33xxError> {
        let _ = self.cs.set_low();
        Timer::after(Duration::from_nanos(T_NCS_SCLK_NS)).await;

        // Send address with read bit (bit 7 = 0)
        self.spi
            .write(&[register.value() & 0x7f])
            .await
            .map_err(|_| Pmw33xxError::Spi)?;

        Timer::after(Duration::from_micros(T_SRAD_US)).await;

        let mut value = [0u8];
        self.spi.read(&mut value).await.map_err(|_| Pmw33xxError::Spi)?;

        Timer::after(Duration::from_nanos(T_SCLK_NCS_R_NS)).await;
        let _ = self.cs.set_high();

        Timer::after(Duration::from_micros(T_SRX_US)).await;

        Ok(value[0])
    }

    async fn read_burst(&mut self, register: Register, data: &mut [u8]) -> Result<(), Pmw33xxError> {
        let _ = self.cs.set_low();
        Timer::after(Duration::from_nanos(T_NCS_SCLK_NS)).await;

        // Send address with read bit (bit 7 = 0)
        self.spi
            .write(&[register.value() & 0x7f])
            .await
            .map_err(|_| Pmw33xxError::Spi)?;

        Timer::after(Duration::from_micros(T_SRAD_MOTBR_US)).await;

        self.spi.read(data).await.map_err(|_| Pmw33xxError::Spi)?;

        Timer::after(Duration::from_nanos(T_SCLK_NCS_R_NS)).await;
        let _ = self.cs.set_high();

        Timer::after(Duration::from_nanos(T_BEXIT_NS)).await;

        Ok(())
    }

    async fn write_reg(&mut self, register: Register, value: u8) -> Result<(), Pmw33xxError> {
        let _ = self.cs.set_low();
        Timer::after(Duration::from_nanos(T_NCS_SCLK_NS)).await;

        // Send address with write bit (bit 7 = 1)
        self.spi
            .write(&[register.value() | SPI_WRITE, value])
            .await
            .map_err(|_| Pmw33xxError::Spi)?;

        Timer::after(Duration::from_micros(T_SCLK_NCS_W_US)).await;

        let _ = self.cs.set_high();

        Timer::after(Duration::from_micros(T_SWX_US)).await;

        Ok(())
    }

    async fn configure(&mut self) -> Result<(), Pmw33xxError> {
        // Power-up reset
        self.write_reg(Register::PowerUpReset, POWER_UP_RESET_VAL).await?;
        Timer::after(Duration::from_millis(RESET_DELAY_MS)).await;

        // Verify product ID
        let val = self.read_reg(Register::ProductId).await?;
        if val != SPEC::PRODUCT_ID {
            error!("PWM33{}: Invalid product id: {:#02}", SPEC::TYPENAME, val);
            return Err(Pmw33xxError::InvalidProductId(val));
        }
        info!("PMW33{} detected, product ID: {:#02x}", SPEC::TYPENAME, val);

        // Power-up init sequence
        // Read motion registers to clear them
        self.read_reg(Register::Motion).await?;
        self.read_reg(Register::DeltaXL).await?;
        self.read_reg(Register::DeltaXH).await?;
        self.read_reg(Register::DeltaYL).await?;
        self.read_reg(Register::DeltaYH).await?;

        if let Some(firmware) = self.srom_firmware {
            self.upload_firmware(firmware).await?;
        }

        self.set_resolution(self.config.res_cpi)
            .await
            .map_err(|_| Pmw33xxError::Spi)?;
        self.write_reg(Register::Config2, 0x00)
            .await
            .map_err(|_| Pmw33xxError::Spi)?;
        self.set_rot_trans_angle(self.config.rot_trans_angle)
            .await
            .map_err(|_| Pmw33xxError::Spi)?;
        self.set_liftoff_dist(self.config.liftoff_dist)
            .await
            .map_err(|_| Pmw33xxError::Spi)?;

        self.check_fw_signature().await?;

        info!("PMW33{} initialized successfully", SPEC::TYPENAME);
        Ok(())
    }

    async fn upload_firmware(&mut self, firmware: &[u8]) -> Result<(), Pmw33xxError> {
        self.write_reg(Register::Config2, 0x00).await?; // disable REST mode

        let srom_id = firmware[1];
        info!(
            "PMW33{}: Uploading SROM firmware with SROM-Id 0x{:02x}",
            SPEC::TYPENAME,
            srom_id
        );

        self.write_reg(Register::SromEnable, 0x1d).await?;
        Timer::after(Duration::from_millis(10)).await;
        self.write_reg(Register::SromEnable, 0x18).await?;

        let _ = self.cs.set_low();
        Timer::after(Duration::from_nanos(T_NCS_SCLK_NS)).await;

        self.spi
            .write(&[Register::SromLoadBurst.value() | SPI_WRITE])
            .await
            .map_err(|_| Pmw33xxError::Spi)?;

        Timer::after(Duration::from_micros(T_SCLK_NCS_W_US)).await;

        for &byte in firmware {
            debug!("PMW33{}: Uploading srom byte: 0x{:02x}", SPEC::TYPENAME, byte);
            self.spi.write(&[byte]).await.map_err(|_| Pmw33xxError::Spi)?;
            Timer::after(Duration::from_micros(T_BRSEP_US)).await;
        }

        let _ = self.cs.set_high();

        Timer::after(Duration::from_nanos(T_BEXIT_NS)).await;

        let flashed_srom_id = self.read_reg(Register::SromId).await?;
        if srom_id != flashed_srom_id {
            error!(
                "PMW33{}: SROM Firmware upload failed, expected SROM-Id 0x{:02x}, but got 0x{:02x} from the sensor.",
                SPEC::TYPENAME,
                srom_id,
                flashed_srom_id
            );
        } else {
            info!(
                "PMW33{}: Upload successfull, new SROM-Id: 0x{:02x}",
                SPEC::TYPENAME,
                flashed_srom_id
            );
        }

        self.write_reg(Register::Config2, 0x00).await?;

        Ok(())
    }
}

impl<'a, SPI, CS, MOTION, SPEC> PointingDriver for Pmw33xx<'a, SPI, CS, MOTION, SPEC>
where
    SPI: SpiBus,
    CS: OutputPin,
    MOTION: InputPin + Wait,
    SPEC: Pmw33xxSpec,
{
    type MOTION = MOTION;

    /// Initialize the sensor (public API)
    async fn init(&mut self) -> Result<(), PointingDriverError> {
        // Set initial pin states
        let _ = self.cs.set_high();
        Timer::after(Duration::from_millis(1)).await;

        self.configure().await?;
        Ok(())
    }

    /// Read motion data from the sensor (motion work handler)
    async fn read_motion(&mut self) -> Result<MotionData, PointingDriverError> {
        if !self.in_burst {
            self.write_reg(Register::MotionBurst, 0x00).await?;
            self.in_burst = true;
        }

        let mut burst_data = [0u8; BURST_DATA_LEN];
        self.read_burst(Register::MotionBurst, &mut burst_data[..BURST_DATA_LEN])
            .await?;

        debug!("PMW33{}: Burst raw data {:?}", SPEC::TYPENAME, burst_data);

        // panic recovery, sometimes burst mode works weird.
        if (burst_data[BURST_MOTION_FLAGS] & 0b111) != 0x00 {
            debug!("PMW33{}: Burst panic recovery", SPEC::TYPENAME);
            self.in_burst = false;
        }

        if (burst_data[BURST_MOTION_FLAGS] & MOTION_STATUS_MOTION) == 0x00 {
            return Ok(MotionData::default());
        }
        if (burst_data[BURST_MOTION_FLAGS] & MOTION_STATUS_LIFTED) != 0x00 {
            return Ok(MotionData::default());
        }

        let dx: i16 = i16::from_le_bytes([burst_data[BURST_DELTA_X_L], burst_data[BURST_DELTA_X_H]]);
        let dy: i16 = i16::from_le_bytes([burst_data[BURST_DELTA_Y_L], burst_data[BURST_DELTA_Y_H]]);

        debug!("PMW33{} motion: x: {}, y: {}", SPEC::TYPENAME, dx, dy);

        Ok(MotionData { dx, dy })
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

    /// Set sensor resolution in CPI (100-12000, step 100)
    async fn set_resolution(&mut self, cpi: u16) -> Result<(), PointingDriverError> {
        if !(SPEC::RES_MIN..=SPEC::RES_MAX).contains(&cpi) {
            return Err(PointingDriverError::InvalidCpi);
        }

        SPEC::write_resolution(self, cpi).await?;

        debug!("PMW33{}: Resolution set to {} CPI", SPEC::TYPENAME, cpi);

        Ok(())
    }
}

/// PMW33xx as an InputDevice for RMK
impl<'a, SPI, CS, MOTION, SPEC> PointingDevice<Pmw33xx<'a, SPI, CS, MOTION, SPEC>>
where
    SPI: SpiBus,
    CS: OutputPin,
    MOTION: InputPin + Wait,
    SPEC: Pmw33xxSpec,
{
    const DEFAULT_POLL_INTERVAL_US: u64 = 500;
    const DEFAULT_REPORT_HZ: u16 = 125;

    /// Create a new PMW33xx device
    pub fn new(id: u8, spi: SPI, cs: CS, motion_gpio: Option<MOTION>, sensor_config: Pmw33xxConfig) -> Self {
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

    /// Create a new PMW33xx device with a custom non-zero report rate (Hz)
    pub fn with_report_hz(
        id: u8,
        spi: SPI,
        cs: CS,
        motion_gpio: Option<MOTION>,
        sensor_config: Pmw33xxConfig,
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

    /// Create a new PMW3360 device with custom poll interval
    pub fn with_poll_interval(
        id: u8,
        spi: SPI,
        cs: CS,
        motion_gpio: Option<MOTION>,
        sensor_config: Pmw33xxConfig,
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

    /// Create a new PMW3360 device with a custom poll interval and non-zero report rate
    pub fn with_poll_interval_and_report_hz(
        id: u8,
        spi: SPI,
        cs: CS,
        motion_gpio: Option<MOTION>,
        sensor_config: Pmw33xxConfig,
        poll_interval_us: u64,
        report_hz: u16,
    ) -> Self {
        let report_interval = duration_from_report_hz(report_hz);

        // Polling should be more frequent than reporting
        let poll_interval = Duration::from_micros(poll_interval_us).min(report_interval);

        Self {
            id,
            sensor: Pmw33xx::new(id, spi, cs, motion_gpio, sensor_config),
            init_state: InitState::Pending,
            poll_interval,
            report_interval,
            last_poll: Instant::MIN,
            last_report: Instant::MIN,
            accumulated_x: 0,
            accumulated_y: 0,
        }
    }

    /// Create a new PMW33xx device with SROM firmware, custom poll interval and non-zero report rate
    ///
    /// Firmware is downloaded to the sensor on every startup
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_firmware_poll_interval_report_hertz(
        id: u8,
        spi: SPI,
        cs: CS,
        motion_gpio: Option<MOTION>,
        sensor_config: Pmw33xxConfig,
        poll_interval_us: u64,
        report_hz: u16,
        firmware: &'a [u8],
    ) -> Self {
        let report_interval = duration_from_report_hz(report_hz);

        // Polling should be more frequent than reporting
        let poll_interval = Duration::from_micros(poll_interval_us).min(report_interval);

        Self {
            id,
            sensor: Pmw33xx::new_with_firmware(id, spi, cs, motion_gpio, sensor_config, firmware),
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

#[cfg(test)]
mod tests {
    use embedded_hal_mock::eh1::digital::{Mock as PinMock, State as PinState, Transaction as PinTrans};
    use embedded_hal_mock::eh1::spi::{Mock as SpiMock, Transaction as SpiTrans};

    use super::*;
    use crate::test_support::test_block_on as block_on;

    #[test]
    fn test_device_creation() {
        // Empty transaction list for SPI and GPIO mocks
        let expectations: Vec<SpiTrans<u8>> = vec![];

        let spi = SpiMock::new(&expectations);
        let cs = PinMock::new(&[]);

        let config = Pmw33xxConfig::default();

        // Create PMW33xx device
        let mut device = Pmw33xx::<_, _, _, Pmw3360Spec>::new(0, spi, cs, None::<PinMock>, config);

        // Validate default configuration values
        assert_eq!(device.id, 0);
        assert_eq!(device.config.res_cpi, 1600);
        assert_eq!(device.config.rot_trans_angle, 0);
        assert_eq!(device.config.liftoff_dist, 0x02);
        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_set_valid_resolution() {
        let spi_expectations: Vec<SpiTrans<u8>> = vec![SpiTrans::write_vec(vec![
            (Register::Config1H.value() | SPI_WRITE),
            (2000 / 100 - 1) as u8,
        ])];

        let spi = SpiMock::new(&spi_expectations);
        let cs = PinMock::new(&[PinTrans::set(PinState::Low), PinTrans::set(PinState::High)]);

        let config = Pmw33xxConfig::default();

        let mut device = Pmw33xx::<_, _, _, Pmw3360Spec>::new(0, spi, cs, None::<PinMock>, config);

        let _ = block_on(device.set_resolution(2000));
        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_set_min_resolution_pmw3360() {
        let spi_expectations: Vec<SpiTrans<u8>> = vec![SpiTrans::write_vec(vec![
            Register::Config1H.value() | SPI_WRITE,
            (100 / 100 - 1) as u8, // = 0
        ])];

        let spi = SpiMock::new(&spi_expectations);
        let cs = PinMock::new(&[PinTrans::set(PinState::Low), PinTrans::set(PinState::High)]);

        let mut device = Pmw33xx::<_, _, _, Pmw3360Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());

        block_on(device.set_resolution(100)).unwrap();

        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_set_max_resolution_pmw3360() {
        let spi_expectations: Vec<SpiTrans<u8>> = vec![SpiTrans::write_vec(vec![
            Register::Config1H.value() | SPI_WRITE,
            (12000 / 100 - 1) as u8, // = 0
        ])];

        let spi = SpiMock::new(&spi_expectations);
        let cs = PinMock::new(&[PinTrans::set(PinState::Low), PinTrans::set(PinState::High)]);

        let mut device = Pmw33xx::<_, _, _, Pmw3360Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());

        block_on(device.set_resolution(12000)).unwrap();

        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_set_invalid_low_resolution_pmw3360() {
        let expectations: Vec<SpiTrans<u8>> = vec![];

        let spi = SpiMock::new(&expectations);
        let cs = PinMock::new(&[]);

        let mut device = Pmw33xx::<_, _, _, Pmw3360Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());

        // 99 is illegal (below RES_MIN = 100 for PMW3360)
        let result = block_on(device.set_resolution(99));
        assert!(matches!(result, Err(PointingDriverError::InvalidCpi)));
        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_set_invalid_high_resolution_pmw3360() {
        let expectations: Vec<SpiTrans<u8>> = vec![];

        let spi = SpiMock::new(&expectations);
        let cs = PinMock::new(&[]);

        let mut device = Pmw33xx::<_, _, _, Pmw3360Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());

        // 12001 is illegal (below RES_MAX = 12000 for PMW3360)
        let result = block_on(device.set_resolution(12001));
        assert!(matches!(result, Err(PointingDriverError::InvalidCpi)));
        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_set_valid_resolution_pmw3389() {
        // (13000 / 50) - 1 = 319 = 0x013F
        let spi_expectations: Vec<SpiTrans<u8>> = vec![
            SpiTrans::write_vec(vec![Register::Config1H.value() | SPI_WRITE, 0x01]),
            SpiTrans::write_vec(vec![Register::Config1L.value() | SPI_WRITE, 0x03]),
        ];

        let spi = SpiMock::new(&spi_expectations);
        let cs = PinMock::new(&[
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
        ]);

        let mut device = Pmw33xx::<_, _, _, Pmw3389Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());

        block_on(device.set_resolution(13000)).unwrap();

        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_set_valid_min_resolution_pmw3389() {
        // (50 / 50) - 1 = 0 = 0x013F
        let spi_expectations: Vec<SpiTrans<u8>> = vec![
            SpiTrans::write_vec(vec![Register::Config1H.value() | SPI_WRITE, 0x00]),
            SpiTrans::write_vec(vec![Register::Config1L.value() | SPI_WRITE, 0x00]),
        ];

        let spi = SpiMock::new(&spi_expectations);
        let cs = PinMock::new(&[
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
        ]);

        let mut device = Pmw33xx::<_, _, _, Pmw3389Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());

        block_on(device.set_resolution(50)).unwrap();

        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_set_valid_max_resolution_pmw3389() {
        // (16000 / 50) - 1 = 319 = 0x013F
        let spi_expectations: Vec<SpiTrans<u8>> = vec![
            SpiTrans::write_vec(vec![Register::Config1H.value() | SPI_WRITE, 0x01]),
            SpiTrans::write_vec(vec![Register::Config1L.value() | SPI_WRITE, 0x3F]),
        ];

        let spi = SpiMock::new(&spi_expectations);
        let cs = PinMock::new(&[
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
        ]);

        let mut device = Pmw33xx::<_, _, _, Pmw3389Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());

        block_on(device.set_resolution(16000)).unwrap();

        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_set_invalid_low_resolution_pmw3389() {
        let expectations: Vec<SpiTrans<u8>> = vec![];

        let spi = SpiMock::new(&expectations);
        let cs = PinMock::new(&[]);

        let mut device = Pmw33xx::<_, _, _, Pmw3389Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());

        // 49 is illegal (below RES_MIN = 50 for PMW3389)
        let result = block_on(device.set_resolution(49));
        assert!(matches!(result, Err(PointingDriverError::InvalidCpi)));
        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_set_invalid_high_resolution_pmw3389() {
        let expectations: Vec<SpiTrans<u8>> = vec![];

        let spi = SpiMock::new(&expectations);
        let cs = PinMock::new(&[]);

        let mut device = Pmw33xx::<_, _, _, Pmw3389Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());

        // 16001 is illegal (below RES_MAX = 16000 for PMW3389)
        let result = block_on(device.set_resolution(16001));
        assert!(matches!(result, Err(PointingDriverError::InvalidCpi)));
        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_set_rot_trans_angle_valid() {
        let spi_expectations = vec![SpiTrans::write_vec(vec![Register::AngleTune.value() | SPI_WRITE, 15u8])];

        let spi = SpiMock::new(&spi_expectations);
        let cs = PinMock::new(&[PinTrans::set(PinState::Low), PinTrans::set(PinState::High)]);

        let mut device = Pmw33xx::<_, _, _, Pmw3360Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());

        block_on(device.set_rot_trans_angle(15)).unwrap();

        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_set_rot_trans_angle_too_small() {
        let spi = SpiMock::new(&[]);
        let cs = PinMock::new(&[]);

        let mut device = Pmw33xx::<_, _, _, Pmw3360Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());

        let err = block_on(device.set_rot_trans_angle(-128)).unwrap_err();

        assert_eq!(err, PointingDriverError::InvalidRotTransAngle);

        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_set_rot_trans_angle_too_large() {
        let spi = SpiMock::new(&[]);
        let cs = PinMock::new(&[]);

        let mut device = Pmw33xx::<_, _, _, Pmw3360Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());

        let err = block_on(device.set_rot_trans_angle(128i16 as i8)).unwrap_err();

        assert_eq!(err, PointingDriverError::InvalidRotTransAngle);

        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_set_rot_trans_angle_values() {
        let angles = [127, -127, 15];

        for &angle in &angles {
            let spi = SpiMock::new(&[SpiTrans::write_vec(vec![
                Register::AngleTune.value() | SPI_WRITE,
                angle as u8,
            ])]);
            let cs = PinMock::new(&[PinTrans::set(PinState::Low), PinTrans::set(PinState::High)]);

            let mut device =
                Pmw33xx::<_, _, _, Pmw3360Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());
            block_on(device.set_rot_trans_angle(angle)).unwrap();

            device.spi.done();
            device.cs.done();
        }
    }

    #[test]
    fn test_motion_pending_without_gpio() {
        let spi = SpiMock::new(&[]);
        let cs = PinMock::new(&[]);

        let mut device = Pmw33xx::<_, _, _, Pmw3360Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());

        assert!(device.motion_pending());

        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_init_with_firmware() {
        const TEST_FW: &[u8] = &[0x00, 0x2A, 0xAA, 0xBB, 0xCC, 0xDD]; // example firmware

        let spi_expectations = vec![
            // Power-up reset
            SpiTrans::write_vec(vec![Register::PowerUpReset.value() | SPI_WRITE, POWER_UP_RESET_VAL]),
            // Verify Product ID
            SpiTrans::write_vec(vec![Register::ProductId.value() & 0x7F]),
            SpiTrans::read(Pmw3360Spec::PRODUCT_ID), // single-byte read
            // Clear motion registers
            SpiTrans::write_vec(vec![Register::Motion.value() & 0x7F]),
            SpiTrans::read(0), // return 0 for Motion
            SpiTrans::write_vec(vec![Register::DeltaXL.value() & 0x7F]),
            SpiTrans::read(0),
            SpiTrans::write_vec(vec![Register::DeltaXH.value() & 0x7F]),
            SpiTrans::read(0),
            SpiTrans::write_vec(vec![Register::DeltaYL.value() & 0x7F]),
            SpiTrans::read(0),
            SpiTrans::write_vec(vec![Register::DeltaYH.value() & 0x7F]),
            SpiTrans::read(0),
            // Start SROM upload
            SpiTrans::write_vec(vec![Register::Config2.value() | SPI_WRITE, 0x00]),
            SpiTrans::write_vec(vec![Register::SromEnable.value() | SPI_WRITE, 0x1D]),
            SpiTrans::write_vec(vec![Register::SromEnable.value() | SPI_WRITE, 0x18]),
            // SROM Load Burst command
            SpiTrans::write_vec(vec![Register::SromLoadBurst.value() | SPI_WRITE]),
            // Upload firmware bytes
            SpiTrans::write_vec(vec![TEST_FW[0]]),
            SpiTrans::write_vec(vec![TEST_FW[1]]),
            SpiTrans::write_vec(vec![TEST_FW[2]]),
            SpiTrans::write_vec(vec![TEST_FW[3]]),
            SpiTrans::write_vec(vec![TEST_FW[4]]),
            SpiTrans::write_vec(vec![TEST_FW[5]]),
            // Verify SROM ID
            SpiTrans::write_vec(vec![Register::SromId.value() & 0x7F]),
            SpiTrans::read(TEST_FW[1]),
            // Config2 write after SROM upload
            SpiTrans::write_vec(vec![Register::Config2.value() | SPI_WRITE, 0x00]),
            // Example resolution write
            SpiTrans::write_vec(vec![Register::Config1H.value() | SPI_WRITE, 15]),
            // Config2 after set resolution
            SpiTrans::write_vec(vec![Register::Config2.value() | SPI_WRITE, 0x00]),
            // Example angle tune write
            SpiTrans::write_vec(vec![Register::AngleTune.value() | SPI_WRITE, 0x00]),
            // Example liftoff distance write
            SpiTrans::write_vec(vec![Register::LiftConfig.value() | SPI_WRITE, 0x02]),
            // Firmware signature check
            SpiTrans::write_vec(vec![Register::ProductId.value() & 0x7F]),
            SpiTrans::read(Pmw3360Spec::FW_SIG_PID),
            SpiTrans::write_vec(vec![Register::InverseProductId.value() & 0x7F]),
            SpiTrans::read(Pmw3360Spec::FW_SIG_INV_PID),
        ];

        let spi = SpiMock::new(&spi_expectations);

        // Chip Select (CS) Mock
        let cs_expectations = vec![
            // power up & reset
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // product id
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // motion
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // delta xl
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // delta xh
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High), // after fourth byte
            // delta yl
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // delta yh
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // Config2 write
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // srom enable 0x1D
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // srom enable 0x18
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // srom load burst & firmware upload
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // read flashed
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // config2
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // Set resolution
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // Config 2
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // AngleTune write
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // LiftConfig write
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // Check firmware sig
            // Read FW Signature
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // Read Inverse FW
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
        ];

        let cs = PinMock::new(&cs_expectations);

        let mut device = Pmw33xx::<_, _, _, Pmw3360Spec>::new_with_firmware(
            0,
            spi,
            cs,
            None::<PinMock>,
            Pmw33xxConfig::default(),
            TEST_FW,
        );

        block_on(device.configure()).unwrap();

        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_read_motion() {
        const TEST_FW: &[u8] = &[0x00, 0x2A, 0xAA, 0xBB, 0xCC, 0xDD]; // example firmware

        let spi_expectations = vec![
            // Power-up reset
            SpiTrans::write_vec(vec![Register::PowerUpReset.value() | SPI_WRITE, POWER_UP_RESET_VAL]),
            // Verify Product ID
            SpiTrans::write_vec(vec![Register::ProductId.value() & 0x7F]),
            SpiTrans::read(Pmw3360Spec::PRODUCT_ID), // single-byte read
            // Clear motion registers
            SpiTrans::write_vec(vec![Register::Motion.value() & 0x7F]),
            SpiTrans::read(0), // return 0 for Motion
            SpiTrans::write_vec(vec![Register::DeltaXL.value() & 0x7F]),
            SpiTrans::read(0),
            SpiTrans::write_vec(vec![Register::DeltaXH.value() & 0x7F]),
            SpiTrans::read(0),
            SpiTrans::write_vec(vec![Register::DeltaYL.value() & 0x7F]),
            SpiTrans::read(0),
            SpiTrans::write_vec(vec![Register::DeltaYH.value() & 0x7F]),
            SpiTrans::read(0),
            // Start SROM upload
            SpiTrans::write_vec(vec![Register::Config2.value() | SPI_WRITE, 0x00]),
            SpiTrans::write_vec(vec![Register::SromEnable.value() | SPI_WRITE, 0x1D]),
            SpiTrans::write_vec(vec![Register::SromEnable.value() | SPI_WRITE, 0x18]),
            // SROM Load Burst command
            SpiTrans::write_vec(vec![Register::SromLoadBurst.value() | SPI_WRITE]),
            // Upload firmware bytes
            SpiTrans::write_vec(vec![TEST_FW[0]]),
            SpiTrans::write_vec(vec![TEST_FW[1]]),
            SpiTrans::write_vec(vec![TEST_FW[2]]),
            SpiTrans::write_vec(vec![TEST_FW[3]]),
            SpiTrans::write_vec(vec![TEST_FW[4]]),
            SpiTrans::write_vec(vec![TEST_FW[5]]),
            // Verify SROM ID
            SpiTrans::write_vec(vec![Register::SromId.value() & 0x7F]),
            SpiTrans::read(TEST_FW[1]),
            // Config2 write after SROM upload
            SpiTrans::write_vec(vec![Register::Config2.value() | SPI_WRITE, 0x00]),
            // Example resolution write
            SpiTrans::write_vec(vec![Register::Config1H.value() | SPI_WRITE, 15]),
            // Config2 after set resolution
            SpiTrans::write_vec(vec![Register::Config2.value() | SPI_WRITE, 0x00]),
            // Example angle tune write
            SpiTrans::write_vec(vec![Register::AngleTune.value() | SPI_WRITE, 0x00]),
            // Example liftoff distance write
            SpiTrans::write_vec(vec![Register::LiftConfig.value() | SPI_WRITE, 0x02]),
            // Firmware signature check
            SpiTrans::write_vec(vec![Register::ProductId.value() & 0x7F]),
            SpiTrans::read(Pmw3360Spec::FW_SIG_PID),
            SpiTrans::write_vec(vec![Register::InverseProductId.value() & 0x7F]),
            SpiTrans::read(Pmw3360Spec::FW_SIG_INV_PID),
            // request read motion
            // at start were !in_burst, so write MotionBurst first
            SpiTrans::write_vec(vec![Register::MotionBurst.value() | SPI_WRITE, 0x00]),
            SpiTrans::write_vec(vec![Register::MotionBurst.value() & 0x7F]),
            SpiTrans::read_vec(vec![
                MOTION_STATUS_MOTION, // motion flags
                0x00,                 // observation
                0x00,                 // DELTA_X_L
                0x10,                 // DELTA_X_H = 4096
                0x00,                 // DELTA_Y_L
                0x20,                 // DELTA_Y_H = 8192
            ]),
            // request read motion
            SpiTrans::write_vec(vec![Register::MotionBurst.value() & 0x7F]),
            SpiTrans::read_vec(vec![
                MOTION_STATUS_MOTION | MOTION_STATUS_LIFTED, // motion flags
                0x00,                                        // observation
                0x00,                                        // DELTA_X_L
                0x10,                                        // DELTA_X_H = 4096
                0x00,                                        // DELTA_Y_L
                0x20,                                        // DELTA_Y_H = 8192
            ]),
            // request read motion
            SpiTrans::write_vec(vec![Register::MotionBurst.value() & 0x7F]),
            SpiTrans::read_vec(vec![
                0x00, // motion flags
                0x00, // observation
                0x00, // DELTA_X_L
                0x10, // DELTA_X_H = 4096
                0x00, // DELTA_Y_L
                0x20, // DELTA_Y_H = 8192
            ]),
            // request read motion
            SpiTrans::write_vec(vec![Register::MotionBurst.value() & 0x7F]),
            SpiTrans::read_vec(vec![
                MOTION_STATUS_MOTION | 0b111, // motion flags => sensor panic
                0x00,                         // observation
                0x00,                         // DELTA_X_L
                0x10,                         // DELTA_X_H = 4096
                0x00,                         // DELTA_Y_L
                0x20,                         // DELTA_Y_H = 8192
            ]),
            // request read motion
            // at start were !in_burst, so write MotionBurst first
            SpiTrans::write_vec(vec![Register::MotionBurst.value() | SPI_WRITE, 0x00]),
            SpiTrans::write_vec(vec![Register::MotionBurst.value() & 0x7F]),
            SpiTrans::read_vec(vec![
                MOTION_STATUS_MOTION, // motion flags
                0x00,                 // observation
                0x00,                 // DELTA_X_L
                0x10,                 // DELTA_X_H = 4096
                0x00,                 // DELTA_Y_L
                0x20,                 // DELTA_Y_H = 8192
            ]),
            // send normal motion data again without need to write motionburst first
            SpiTrans::write_vec(vec![Register::MotionBurst.value() & 0x7F]),
            SpiTrans::read_vec(vec![
                MOTION_STATUS_MOTION, // motion flags
                0x00,                 // observation
                0x00,                 // DELTA_X_L
                0x10,                 // DELTA_X_H = 4096
                0x00,                 // DELTA_Y_L
                0x20,                 // DELTA_Y_H = 8192
            ]),
        ];

        let spi = SpiMock::new(&spi_expectations);

        // Chip Select (CS) Mock
        let cs_expectations = vec![
            // power up & reset
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // product id
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // motion
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // delta xl
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // delta xh
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High), // after fourth byte
            // delta yl
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // delta yh
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // Config2 write
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // srom enable 0x1D
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // srom enable 0x18
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // srom load burst & firmware upload
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // read flashed
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // config2
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // Set resolution
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // Config 2
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // AngleTune write
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // LiftConfig write
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // Check firmware sig
            // Read FW Signature
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // Read Inverse FW
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // read burst, normal motion
            // !inburst so 1x write motionburst
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // write register motionburst to read it
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // read burst, lifted
            // write register motionburst to read it
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // read burst, ignore when no motion
            // write register motionburst to read it
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // read burst, trigger panic recovery
            // write register motionburst to read it
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // read burst, normal motion
            // after panic !in_burst, so driver should write motionburst first
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // write register motionburst to read it
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
            // next read_burst without panic
            // write register motionburst to read it
            PinTrans::set(PinState::Low),
            PinTrans::set(PinState::High),
        ];

        let cs = PinMock::new(&cs_expectations);

        let mut device = Pmw33xx::<_, _, _, Pmw3360Spec>::new_with_firmware(
            0,
            spi,
            cs,
            None::<PinMock>,
            Pmw33xxConfig::default(),
            TEST_FW,
        );

        block_on(device.configure()).unwrap();

        let motion = block_on(device.read_motion()).unwrap();
        assert_eq!(motion.dx, 4096);
        assert_eq!(motion.dy, 8192);

        // second test is for ignore motion when sensor is lifted
        let motion = block_on(device.read_motion()).unwrap();
        assert_eq!(motion.dx, 0);
        assert_eq!(motion.dy, 0);

        // third test is for no motion reported, should report 0 even if there is movement data
        let motion = block_on(device.read_motion()).unwrap();
        assert_eq!(motion.dx, 0);
        assert_eq!(motion.dy, 0);

        // test panic recovery. This test is mostly about the order of
        // expectations, see above.
        let motion = block_on(device.read_motion()).unwrap();
        // first read triggers panic recovery, motion data could still be valid
        assert_eq!(motion.dx, 4096);
        assert_eq!(motion.dy, 8192);
        let motion = block_on(device.read_motion()).unwrap();
        // second call normal motion data
        assert_eq!(motion.dx, 4096);
        assert_eq!(motion.dy, 8192);
        let motion = block_on(device.read_motion()).unwrap();
        // second call normal motion data
        assert_eq!(motion.dx, 4096);
        assert_eq!(motion.dy, 8192);

        device.spi.done();
        device.cs.done();
    }

    #[test]
    fn test_read_reg_timing() {
        let spi_expectations = vec![
            SpiTrans::write_vec(vec![Register::AngleTune.value() & 0x7f]),
            SpiTrans::read(10),
        ];

        let spi = SpiMock::new(&spi_expectations);
        let cs = PinMock::new(&[PinTrans::set(PinState::Low), PinTrans::set(PinState::High)]);

        let mut device = Pmw33xx::<_, _, _, Pmw3360Spec>::new(0, spi, cs, None::<PinMock>, Pmw33xxConfig::default());

        let read = block_on(device.read_reg(Register::AngleTune)).unwrap();
        assert_eq!(read, 10);

        device.spi.done();
        device.cs.done();
    }
}
