//! Vial output reports delivered through the HID control endpoint.

use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_usb::class::hid::ReportId;
use embassy_usb::class::hid::device::RequestHandler;
use embassy_usb::control::OutResponse;
use embedded_io_async::{ErrorType, Read};
use rmk_types::connection::UsbState;
use rmk_types::protocol::vial::VIAL_EP_SIZE;

use crate::RawMutex;
use crate::host::transport::HostTransportError;

static REQUESTS: Channel<RawMutex, [u8; VIAL_EP_SIZE], 2> = Channel::new();
pub(super) static SESSION_ENDED: Signal<RawMutex, ()> = Signal::new();

pub(in crate::usb) fn end_session() {
    REQUESTS.clear();
    SESSION_ENDED.signal(());
}

#[derive(Default)]
pub(super) struct Reports;

impl RequestHandler for Reports {
    fn set_report(&mut self, id: ReportId, data: &[u8]) -> OutResponse {
        if id != ReportId::Out(0) || crate::state::current_usb_state() != UsbState::Configured {
            return OutResponse::Rejected;
        }
        let Ok(report) = <[u8; VIAL_EP_SIZE]>::try_from(data) else {
            return OutResponse::Rejected;
        };
        match REQUESTS.try_send(report) {
            Ok(()) => OutResponse::Accepted,
            Err(_) => OutResponse::Rejected,
        }
    }
}

impl ErrorType for Reports {
    type Error = HostTransportError;
}

impl Read for Reports {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let dst = buf.get_mut(..VIAL_EP_SIZE).ok_or(HostTransportError)?;
        dst.copy_from_slice(&REQUESTS.receive().await);
        Ok(VIAL_EP_SIZE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::set_usb_state;

    #[test]
    fn reconnect_reads_the_new_hosts_vial_request() {
        let mut reports = Reports;
        set_usb_state(UsbState::Configured);
        let mut request = [0; VIAL_EP_SIZE];
        request[0] = rmk_types::protocol::vial::ViaCommand::GetProtocolVersion as u8;
        assert_eq!(reports.set_report(ReportId::Out(0), &request), OutResponse::Accepted);
        end_session();

        request[0] = rmk_types::protocol::vial::ViaCommand::Vial as u8;
        assert_eq!(reports.set_report(ReportId::Out(0), &request), OutResponse::Accepted);
        let mut received = [0; VIAL_EP_SIZE];
        assert_eq!(
            embassy_futures::block_on(reports.read(&mut received)).unwrap(),
            VIAL_EP_SIZE
        );
        assert_eq!(received, request);
    }
}
