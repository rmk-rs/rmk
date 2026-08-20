#![no_std]
#![no_main]

#[macro_use]
mod macros;
mod keymap;

use defmt::{info, unwrap};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_nrf::config::{ClockSpeed, Config as NrfConfig, HfclkSource, LfclkSource};
use embassy_nrf::mode::Blocking;
use embassy_nrf::peripherals::USBHS;
use embassy_nrf::usb::vbus_detect::HardwareVbusDetect;
use embassy_nrf::usb::{self, Driver};
use embassy_nrf::{bind_interrupts, cracen, pac};
use keymap::{COL, ROW, SIZE};
use nrf_mpsl::Flash;
use nrf_sdc::mpsl::MultiprotocolServiceLayer;
use nrf_sdc::{self as sdc, mpsl};
use panic_probe as _;
use rmk::ble::BleTransport;
use rmk::config::{BehaviorConfig, DeviceConfig, LockConfig, PositionalConfig, RmkConfig, StorageConfig};
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::host::HostService;
use rmk::keyboard::Keyboard;
use rmk::matrix::direct_pin::DirectPinMatrix;
use rmk::processor::builtin::wpm::WpmProcessor;
use rmk::usb::UsbTransport;
use rmk::{DefaultPacketPool, KeymapData, PacketPool, initialize_keymap_and_storage, run_all};
use static_cell::StaticCell;

type RandomSource = cracen::Cracen<'static, Blocking>;

bind_interrupts!(struct Irqs {
    USBHS => usb::InterruptHandler<USBHS>;
    VREGUSB => usb::vbus_detect::InterruptHandler;
    SWI00 => nrf_sdc::mpsl::LowPrioInterruptHandler;
    CLOCK_POWER => nrf_sdc::mpsl::ClockInterruptHandler;
    RADIO_0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
    TIMER10 => nrf_sdc::mpsl::HighPrioInterruptHandler;
    GRTC_3 => nrf_sdc::mpsl::HighPrioInterruptHandler;
});

#[embassy_executor::task]
async fn mpsl_task(mpsl: &'static MultiprotocolServiceLayer<'static>) -> ! {
    mpsl.run().await
}

const SDC_MEM_SIZE: usize = 5788;
const FLASH_START_ADDR: usize = 0x120000;
const FLASH_SECTORS: u8 = 6;
const RYNK_UNLOCK_KEYS: &[(u8, u8)] = &[(0, 0), (0, 1)];

const L2CAP_TXQ: u8 = 4;
const L2CAP_RXQ: u8 = 4;

fn build_sdc<'d, const N: usize>(
    p: nrf_sdc::Peripherals<'d>,
    rng: &'d mut RandomSource,
    mpsl: &'d MultiprotocolServiceLayer,
    mem: &'d mut sdc::Mem<N>,
) -> Result<nrf_sdc::SoftdeviceController<'d>, nrf_sdc::Error> {
    sdc::Builder::new()?
        .support_adv()
        .support_peripheral()
        .support_dle_peripheral()
        .support_phy_update_peripheral()
        .support_le_2m_phy()
        .peripheral_count(1)?
        .buffer_cfg(
            DefaultPacketPool::MTU as u16,
            DefaultPacketPool::MTU as u16,
            L2CAP_TXQ,
            L2CAP_RXQ,
        )?
        .build(p, rng, mpsl, mem)
}

fn ble_addr() -> [u8; 6] {
    let ficr = pac::FICR;
    let high = u64::from(ficr.deviceaddr(1).read());
    let addr = high << 32 | u64::from(ficr.deviceaddr(0).read());
    let addr = addr | 0x0000_c000_0000_0000;
    unwrap!(addr.to_le_bytes()[..6].try_into())
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Hello RMK BLE on nRF54LM20A!");

    let mut nrf_config = NrfConfig::default();
    nrf_config.clock_speed = ClockSpeed::CK128;
    nrf_config.hfclk_source = HfclkSource::ExternalXtal;
    nrf_config.lfclk_source = LfclkSource::ExternalXtal;
    let p = embassy_nrf::init(nrf_config);
    // Off at reset; enabling it makes compute-bound code such as LESC pairing ~3x faster.
    pac::ICACHE.enable().write(|w| w.set_enable(true));
    info!("nRF initialized");

    let mpsl_p = mpsl::Peripherals::new(
        p.GRTC_CH7,
        p.GRTC_CH8,
        p.GRTC_CH9,
        p.GRTC_CH10,
        p.GRTC_CH11,
        p.TIMER10,
        p.TIMER20,
        p.TEMP,
        p.PPI10_CH0,
        p.PPI20_CH1,
        p.PPIB11_CH0,
        p.PPIB21_CH0,
    );
    let lfclk_cfg = mpsl::raw::mpsl_clock_lfclk_cfg_t {
        source: mpsl::raw::MPSL_CLOCK_LF_SRC_XTAL as u8,
        rc_ctiv: 0,
        rc_temp_ctiv: 0,
        accuracy_ppm: 50,
        skip_wait_lfclk_started: false,
    };
    static MPSL: StaticCell<MultiprotocolServiceLayer> = StaticCell::new();
    static SESSION_MEM: StaticCell<mpsl::SessionMem<1>> = StaticCell::new();
    let mpsl = MPSL.init(unwrap!(mpsl::MultiprotocolServiceLayer::with_timeslots(
        mpsl_p,
        Irqs,
        lfclk_cfg,
        SESSION_MEM.init(mpsl::SessionMem::new())
    )));
    spawner.spawn(unwrap!(mpsl_task(&*mpsl)));
    info!("MPSL started");

    let sdc_p = sdc::Peripherals::new(
        p.PPI00_CH1,
        p.PPI00_CH3,
        p.PPI10_CH1,
        p.PPI10_CH2,
        p.PPI10_CH3,
        p.PPI10_CH4,
        p.PPI10_CH5,
        p.PPI10_CH6,
        p.PPI10_CH7,
        p.PPI10_CH8,
        p.PPI10_CH9,
        p.PPI10_CH10,
        p.PPI10_CH11,
        p.PPIB00_CH1,
        p.PPIB00_CH2,
        p.PPIB00_CH3,
        p.PPIB10_CH1,
        p.PPIB10_CH2,
        p.PPIB10_CH3,
    );

    let mut rng = cracen::Cracen::new_blocking(p.CRACEN);
    let mut sdc_mem = sdc::Mem::<SDC_MEM_SIZE>::new();
    let sdc = unwrap!(build_sdc(sdc_p, &mut rng, mpsl, &mut sdc_mem));
    info!("SDC built");
    info!("BLE stack ready");

    static EP_OUT_BUFFER: StaticCell<[u8; 2048]> = StaticCell::new();
    let driver = Driver::new(
        p.USBHS,
        Irqs,
        HardwareVbusDetect::new(Irqs),
        EP_OUT_BUFFER.init([0; 2048]),
        usb::Config::default(),
    );
    info!("USB driver ready");

    let direct_pins = config_matrix_pins_nrf! {
        peripherals: p,
        direct_pins: [
            [P1_26, P1_09],
            [P1_08, P0_05],
        ]
    };

    let keyboard_device_config = DeviceConfig {
        vid: 0x4c4b,
        pid: 0x4643,
        manufacturer: "Haobo",
        product_name: "RMK nRF54LM20A",
        ..DeviceConfig::default()
    };
    let storage_config = StorageConfig {
        start_addr: FLASH_START_ADDR,
        num_sectors: FLASH_SECTORS,
        ..Default::default()
    };
    let rmk_config = RmkConfig {
        device_config: keyboard_device_config,
        lock_config: LockConfig {
            unlock_keys: RYNK_UNLOCK_KEYS,
            insecure: false,
            write_requires_unlock: false,
            bootloader_requires_unlock: true,
        },
        storage_config,
        ..Default::default()
    };

    let mut keymap_data = KeymapData::new(keymap::get_default_keymap());
    let mut behavior_config = BehaviorConfig::default();
    let per_key_config = PositionalConfig::default();

    let flash = Flash::take(mpsl, p.RRAMC);

    let (keymap, mut storage) = initialize_keymap_and_storage(
        &mut keymap_data,
        flash,
        &storage_config,
        &mut behavior_config,
        &per_key_config,
    )
    .await;
    info!("Storage initialized");

    let debouncer = DefaultDebouncer::new();
    let mut matrix = DirectPinMatrix::<_, _, ROW, COL, SIZE>::new(direct_pins, debouncer, true);
    let mut keyboard = Keyboard::new(&keymap);
    let host_service = HostService::new(&keymap, &rmk_config);

    let mut usb_transport = UsbTransport::new(driver, rmk_config.device_config).with_host_service(&host_service);
    let mut ble_transport = BleTransport::new(sdc, ble_addr(), rmk_config)
        .with_host_service(&host_service);
    let mut wpm_processor = WpmProcessor::new();

    run_all!(matrix, storage, usb_transport, ble_transport, wpm_processor, keyboard).await;
}
