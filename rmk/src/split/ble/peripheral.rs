#[cfg(feature = "subrating")]
use bt_hci::{cmd::le::LeSetHostFeature, controller::ControllerCmdSync};
use embassy_futures::join::join;
use embassy_futures::select::{Either, select, select3};
use embassy_time::{Duration, Timer, with_timeout};
use rmk_types::connection::ConnectionStatus;
use trouble_host::prelude::*;

#[cfg(feature = "storage")]
use super::PeerAddress;
use super::{split_channel_config, split_coc};
use crate::ble::adv::{Adv, advertise_conn};
use crate::event::{CentralConnectedEvent, KeyboardEvent, SleepStateEvent, SubscribableEvent, publish_event};
use crate::split::peripheral::{peripheral_read_loop, peripheral_write_loop};
use crate::state::update_status;

/// How long to wait for the central to open the split channel once it has
/// connected. Bounded so a central that connects and then stalls can't hold
/// the peripheral off the air.
const CHANNEL_OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// Let the controller accept the central's subrate requests on the split link.
///
/// Must run concurrently with `ble_task()` (whose runner serves the HCI command)
/// and before any advertising, since the flag only applies to links opened after
/// it is set.
#[cfg(feature = "subrating")]
async fn init_subrating_host_feature<C: Controller + ControllerCmdSync<LeSetHostFeature>>(
    stack: &Stack<'_, C, impl PacketPool>,
) {
    const CONN_SUBRATING_HOST_BIT: u8 = 38;
    let cmd = LeSetHostFeature::new(CONN_SUBRATING_HOST_BIT, 1);
    if let Err(e) = stack.command(cmd).await {
        error!("[split_peri] error setting subrating host feature flag: {:?}", e);
    }
}

/// Initialize and run the nRF peripheral keyboard service via BLE.
///
/// # Arguments
///
/// * `id` - The id of the peripheral
/// * `stack` - The stack to use
pub async fn initialize_nrf_ble_split_peripheral_and_run<
    'b,
    's: 'b,
    #[cfg(feature = "subrating")] C: Controller + ControllerCmdSync<LeSetHostFeature>,
    #[cfg(not(feature = "subrating"))] C: Controller,
>(
    id: usize,
    stack: &'b Stack<'s, C, DefaultPacketPool>,
) {
    publish_event(CentralConnectedEvent { connected: false });

    let mut peripheral = stack.peripheral();
    let runner = stack.runner();

    // First, read central address from storage
    let mut central_addr = crate::storage::read_peer_address(0)
        .await
        .filter(|a| a.is_valid)
        .map(|a| a.address);

    let peri_task = async {
        // Set subrating host support before any advertising/connecting
        #[cfg(feature = "subrating")]
        init_subrating_host_feature(stack).await;

        loop {
            update_status(|c| *c = ConnectionStatus::new());
            publish_event(CentralConnectedEvent { connected: false });
            publish_event(SleepStateEvent::new(false));
            match split_peripheral_advertise(id, central_addr, &mut peripheral).await {
                Ok(conn) => {
                    info!("Connected to the central");
                    publish_event(CentralConnectedEvent { connected: true });
                    let new_addr = conn.peer_address().addr.into_inner();
                    if central_addr != Some(new_addr) {
                        info!("Saving central address to storage");
                        if crate::storage::write_peer_address(PeerAddress {
                            peer_id: 0,
                            is_valid: true,
                            address: new_addr,
                        })
                        .await
                        {
                            central_addr = Some(new_addr);
                        }
                    }
                    run_split_session(stack, &conn).await;
                    info!("Disconnected from the central");
                    // The session also ends on a dead channel over a link that
                    // is still up. Dropping the last handle files a disconnect
                    // the runner serves on its own; the pause lets that finish
                    // before we advertise again.
                    drop(conn);
                    Timer::after_millis(500).await;
                }
                Err(BleHostError::BleHost(Error::Timeout)) => {
                    // Timeout, wait new keys to continue
                    error!("Connect to central timeout");
                    publish_event(SleepStateEvent::new(true));
                    let mut sub = KeyboardEvent::subscriber();
                    sub.clear();
                    let _ = sub.next_message_pure().await;
                    continue;
                }
                Err(e) => {
                    #[cfg(feature = "defmt")]
                    let e = defmt::Debug2Format(&e);
                    error!("Advertise error: {:?}", e);
                    Timer::after_millis(500).await;
                    continue;
                }
            };
        }
    };

    join(crate::ble::ble_task(runner, &crate::ble::NoopHandler), peri_task).await;
}

/// Accept the split channel the central opens, then run the peripheral over it
/// until either the channel or the link goes away.
async fn run_split_session<'d, C: Controller>(
    stack: &'d Stack<'_, C, DefaultPacketPool>,
    conn: &Connection<'d, DefaultPacketPool>,
) {
    // Scoped so the listener stops listening once the channel is up: the
    // central only ever opens this one.
    let channel = {
        let listener = L2capChannel::listen(stack, conn);
        // The central sets the PHY and connection parameters right about now,
        // which is already the whole depth of the connection event queue, so it
        // has to be drained alongside the accept or the disconnect is dropped.
        let config = split_channel_config();
        let accept = with_timeout(CHANNEL_OPEN_TIMEOUT, listener.accept(&config));
        match select(accept, drain_until_disconnect(conn)).await {
            Either::First(Ok(Ok(channel))) => channel,
            Either::First(Ok(Err(e))) => {
                #[cfg(feature = "defmt")]
                let e = defmt::Debug2Format(&e);
                error!("Accepting the split channel failed: {:?}", e);
                return;
            }
            Either::First(Err(_)) => {
                error!("Central connected but never opened the split channel");
                return;
            }
            Either::Second(()) => return,
        }
    };
    info!("Split channel open, mtu {}", channel.mtu());

    let (mut reader, mut writer) = split_coc(stack, channel);
    // Read and write concurrently: credits are granted from inside `receive`, so
    // a send waiting on the central must never be able to stop the reader.
    select3(
        peripheral_read_loop(&mut reader),
        peripheral_write_loop(&mut writer),
        drain_until_disconnect(conn),
    )
    .await;
    update_status(|c| *c = ConnectionStatus::new());
}

/// Consume this link's connection events until it drops. Nothing else reads
/// them and the queue holds only a couple, so leaving it unread loses the
/// disconnect and the session can only end by timing out.
async fn drain_until_disconnect(conn: &Connection<'_, DefaultPacketPool>) {
    loop {
        match conn.next().await {
            ConnectionEvent::Disconnected { reason } => {
                error!("Disconnected from central: {:?}", reason);
                return;
            }
            ConnectionEvent::ConnectionParamsUpdated {
                conn_interval,
                peripheral_latency,
                supervision_timeout,
            } => info!(
                "[split] params updated: interval {:?}us, latency {:?}, timeout {:?}ms",
                conn_interval.as_micros(),
                peripheral_latency,
                supervision_timeout.as_millis()
            ),
            ConnectionEvent::SubratingParamsUpdated {
                subrate_factor,
                peripheral_latency,
                continuation_number,
                supervision_timeout,
            } => info!(
                "[split] subrating updated: subrate {:?}, latency {:?}, continuation {:?}, timeout {:?}ms",
                subrate_factor,
                peripheral_latency,
                continuation_number,
                supervision_timeout.as_millis()
            ),
            ConnectionEvent::PhyUpdated { tx_phy, rx_phy } => {
                info!("[split] PHY updated: {:?}, {:?}", tx_phy, rx_phy)
            }
            _ => (),
        }
    }
}

/// Reconnect to the saved central, falling back to seeking any central when it
/// does not answer.
async fn split_peripheral_advertise<'a, C: Controller>(
    id: usize,
    central_addr: Option<[u8; 6]>,
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
) -> Result<Connection<'a, DefaultPacketPool>, BleHostError<C::Error>> {
    if let Some(addr) = central_addr {
        let directed = Adv::Directed(Address::random(addr));
        match advertise_conn(peripheral, directed, Duration::from_secs(10)).await {
            Err(BleHostError::BleHost(Error::Timeout)) => warn!("[adv] Try update central_addr"),
            result => return result,
        }
    }
    let seeking = Adv::SplitPeripheral { id: id as u8 };
    advertise_conn(peripheral, seeking, Duration::from_secs(300)).await
}
