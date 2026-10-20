use embassy_usb::class::hid::device::HidReader;
use embassy_usb::driver::Driver;
use rmk_types::led_indicator::LedIndicator;

use crate::hid::{HidError, HidReaderTrait};

pub(crate) struct UsbLedReader<'a, 'd, D: Driver<'d>> {
    hid_reader: &'a mut HidReader<'d, D, 1>,
}

impl<'a, 'd, D: Driver<'d>> UsbLedReader<'a, 'd, D> {
    pub(crate) fn new(hid_reader: &'a mut HidReader<'d, D, 1>) -> Self {
        Self { hid_reader }
    }
}

impl<'d, D: Driver<'d>> HidReaderTrait for UsbLedReader<'_, 'd, D> {
    type ReportType = LedIndicator;

    async fn read_report(&mut self) -> Result<Self::ReportType, HidError> {
        let mut buf = [0u8; 1];
        self.hid_reader.read(&mut buf).await.map_err(HidError::UsbReadError)?;

        Ok(LedIndicator::from_bits(buf[0]))
    }
}
