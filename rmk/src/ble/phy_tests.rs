use core::cell::{Cell, RefCell};
use core::convert::Infallible;
use core::pin::pin;

use bt_hci::cmd::{self, AsyncCmd, Cmd, CmdReturnBuf, SyncCmd};
use bt_hci::controller::{Controller, ControllerCmdAsync, ControllerCmdSync};
use bt_hci::{ControllerToHostPacket, FromHciBytes, WriteHci};
use futures::poll;
use trouble_host::prelude::*;

struct PhyController<'a> {
    connected: Cell<bool>,
    supports_2m: bool,
    busy: Cell<bool>,
    attempts: &'a Cell<usize>,
    default: &'a RefCell<Option<[u8; 3]>>,
    connection: &'a RefCell<Option<[u8; 7]>>,
}

impl embedded_io_async::ErrorType for PhyController<'_> {
    type Error = Infallible;
}

impl Controller for PhyController<'_> {
    type Buffer<'a> = [u8; 64];
    fn alloc_buf(&self) -> Result<Self::Buffer<'_>, Self::Error> {
        Ok([0; 64])
    }
    async fn write_acl_data(&self, _: &bt_hci::data::AclPacket<'_>) -> Result<(), Self::Error> {
        panic!("unexpected ACL")
    }
    async fn write_sync_data(&self, _: &bt_hci::data::SyncPacket<'_>) -> Result<(), Self::Error> {
        panic!("unexpected sync")
    }
    async fn write_iso_data(&self, _: &bt_hci::data::IsoPacket<'_>) -> Result<(), Self::Error> {
        panic!("unexpected ISO")
    }
    async fn read<'a>(&self, buf: &'a mut Self::Buffer<'_>) -> Result<ControllerToHostPacket<'a>, Self::Error> {
        if self.connected.replace(true) {
            core::future::pending::<()>().await;
        }
        let event = [4, 0x3e, 19, 1, 0, 1, 0, 1, 0, 1, 2, 3, 4, 5, 6, 24, 0, 0, 0, 0xf4, 1, 0];
        buf[..event.len()].copy_from_slice(&event);
        Ok(ControllerToHostPacket::from_hci_bytes_complete(&buf[..event.len()]).unwrap())
    }
}

impl<C: SyncCmd> ControllerCmdSync<C> for PhyController<'_> {
    async fn exec(&self, command: &C) -> Result<C::Return, cmd::Error<Self::Error>> {
        let mut buffer = C::ReturnBuf::new();
        if C::OPCODE == cmd::le::LeReadBufferSize::OPCODE {
            buffer.as_mut().copy_from_slice(&[64, 0, 32]);
        } else if C::OPCODE == cmd::le::LeReadLocalSupportedFeatures::OPCODE {
            buffer.as_mut()[1] = u8::from(self.supports_2m);
        } else if C::OPCODE == cmd::le::LeSetDefaultPhy::OPCODE {
            let mut bytes = [0; 3];
            command.params().write_hci(&mut bytes[..]).unwrap();
            *self.default.borrow_mut() = Some(bytes);
        }
        Ok(C::Return::from_hci_bytes_complete(buffer.as_ref()).unwrap())
    }
}

impl<C: AsyncCmd> ControllerCmdAsync<C> for PhyController<'_> {
    async fn exec(&self, command: &C) -> Result<(), cmd::Error<Self::Error>> {
        assert_eq!(C::OPCODE, cmd::le::LeSetPhy::OPCODE);
        self.attempts.set(self.attempts.get() + 1);
        if self.busy.replace(false) {
            return Err(cmd::Error::Hci(bt_hci::param::Error::CONTROLLER_BUSY));
        }
        let mut bytes = [0; 7];
        command.params().write_hci(&mut bytes[..]).unwrap();
        *self.connection.borrow_mut() = Some(bytes);
        Ok(())
    }
}

#[test]
fn generic_hci_phy_commands_respect_capability_and_retry_busy() {
    crate::test_support::test_block_on(async {
        for supports_2m in [false, true] {
            let attempts = Cell::new(0);
            let default = RefCell::new(None);
            let connection = RefCell::new(None);
            let mut resources = HostResources::<DefaultPacketPool, 1, 1>::new();
            let stack = trouble_host::new(
                PhyController {
                    connected: Cell::new(false),
                    supports_2m,
                    busy: Cell::new(true),
                    attempts: &attempts,
                    default: &default,
                    connection: &connection,
                },
                &mut resources,
            )
            .build();
            let (mut rx, mut control, _) = stack.runner().split();
            let mut control = pin!(control.run());
            assert!(poll!(control.as_mut()).is_pending());
            let mut rx = pin!(rx.run());
            assert!(poll!(rx.as_mut()).is_pending());
            let conn = stack.peripheral().try_accept().unwrap();

            super::configure_default_phy(&stack).await;
            let mask = 1 | if crate::BLE_USE_2M_PHY { 2 } else { 0 };
            assert_eq!(*default.borrow(), supports_2m.then_some([0, mask, mask]));
            super::update_ble_phy(&stack, &conn, PhyKind::Le2M).await;
            let mask = if supports_2m { 2 } else { 1 };
            assert_eq!(*connection.borrow(), Some([1, 0, 0, mask, mask, 0, 0]));
            assert_eq!(attempts.get(), 2);
            super::update_ble_phy(&stack, &conn, PhyKind::Le1M).await;
            assert_eq!(*connection.borrow(), Some([1, 0, 0, 1, 1, 0, 0]));
        }
    });
}
