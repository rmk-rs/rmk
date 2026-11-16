//! Every advertisement RMK sends, in one place.

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::{Duration, with_timeout};
use trouble_host::prelude::appearance::human_interface_device::KEYBOARD;
use trouble_host::prelude::service::{BATTERY, HUMAN_INTERFACE_DEVICE};
use trouble_host::prelude::*;

use crate::ble::ExtAdvCmds;

/// Company identifier marking an advertisement as RMK's own.
const RMK_ADV_COMPANY_ID: u16 = 0x5253;

// First payload byte of an RMK advertisement, naming the kind. This is wire
// format between two RMK devices: append kinds, never renumber.
const SPLIT_PERIPHERAL: u8 = 0;
const DONGLE_SEEKING: u8 = 1;

/// An advertisement, named for the peer meant to answer it.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub(crate) enum Adv<'a> {
    /// A peer that already knows us: ADV_DIRECT_IND carries no data, only this address.
    Directed(Address),
    /// Any BLE host, which finds us as a standard HID keyboard.
    Host { name: &'a str },
    /// A bonded host reconnecting without HID discovery data.
    HostReconnect,
    /// The split central that owns peripheral `id`.
    SplitPeripheral { id: u8 },
    /// An RMK dongle whose pairing window is open.
    DongleSeeking,
}

impl Adv<'_> {
    /// Encode into `buf`, which the returned advertisement borrows.
    fn build<'b>(&self, buf: &'b mut [u8; 31]) -> Result<Advertisement<'b>, Error> {
        let adv_data: &[AdStructure] = match *self {
            Self::Directed(peer) => return Ok(Advertisement::ConnectableNonscannableDirected { peer }),
            Self::Host { name } => &[
                AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
                AdStructure::CompleteServiceUuids16(&[BATTERY.to_le_bytes(), HUMAN_INTERFACE_DEVICE.to_le_bytes()]),
                AdStructure::CompleteLocalName(name.as_bytes()),
                AdStructure::Unknown {
                    ty: 0x19, // Appearance, which trouble-host has no variant for
                    data: &KEYBOARD.to_le_bytes(),
                },
            ],
            Self::HostReconnect => &[AdStructure::Flags(BR_EDR_NOT_SUPPORTED)],
            // The two kinds below name themselves to another RMK device and nothing
            // else, undiscoverable: only RMK should act on these, and no host should
            // list them.
            Self::SplitPeripheral { id } => &[
                AdStructure::Flags(BR_EDR_NOT_SUPPORTED),
                AdStructure::ManufacturerSpecificData {
                    company_identifier: RMK_ADV_COMPANY_ID,
                    payload: &[SPLIT_PERIPHERAL, id],
                },
            ],
            Self::DongleSeeking => &[
                AdStructure::Flags(BR_EDR_NOT_SUPPORTED),
                AdStructure::ManufacturerSpecificData {
                    company_identifier: RMK_ADV_COMPANY_ID,
                    payload: &[DONGLE_SEEKING],
                },
            ],
        };
        let len = AdStructure::encode_slice(adv_data, &mut buf[..])?;
        Ok(Advertisement::ConnectableScannableUndirected {
            adv_data: &buf[..len],
            scan_data: &[],
        })
    }

    /// Read an RMK advertisement out of a scan report, or `None` if the report
    /// is not one of ours.
    pub(crate) fn decode(adv_data: &[u8]) -> Option<Adv<'static>> {
        let mut rest = adv_data;
        loop {
            // Every AD structure is a length byte covering the type byte and the data.
            let (&len, tail) = rest.split_first()?;
            let (structure, tail) = tail.split_at_checked(len as usize)?;
            rest = tail;
            // 0xFF is manufacturer-specific data: company id (little-endian), then us.
            let [0xFF, lo, hi, payload @ ..] = structure else {
                continue;
            };
            if u16::from_le_bytes([*lo, *hi]) != RMK_ADV_COMPANY_ID {
                continue;
            }
            return match payload {
                [SPLIT_PERIPHERAL, id] => Some(Adv::SplitPeripheral { id: *id }),
                [DONGLE_SEEKING] => Some(Adv::DongleSeeking),
                // A kind only a newer firmware knows.
                _ => None,
            };
        }
    }

    /// All variants use legacy PDUs, which require the 1M primary PHY.
    fn params(&self) -> AdvertisementParameters {
        let interval = match self {
            Self::Host { .. } => Duration::from_millis(200),
            _ => Duration::from_millis(50),
        };
        AdvertisementParameters {
            primary_phy: PhyKind::Le1M,
            secondary_phy: PhyKind::Le1M,
            tx_power: TxPower::Plus8dBm,
            interval_min: interval,
            interval_max: interval,
            ..Default::default()
        }
    }
}

/// Advertise with an optional local address override and accept a connection within `timeout`.
pub(crate) async fn advertise<'a, 'b, C: Controller + ExtAdvCmds, const ATT: usize, const CONN: usize>(
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    server: &'b AttributeServer<'_, NoopRawMutex, DefaultPacketPool, ATT, CONN>,
    adv: Adv<'_>,
    address: Option<BdAddr>,
    timeout: Duration,
) -> Result<GattConnection<'a, 'b, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut buf = [0; 31];
    #[cfg(feature = "ext_adv")]
    let advertiser = {
        let sets = [AdvertisementSet {
            params: adv.params(),
            data: adv.build(&mut buf)?,
            address,
        }];
        let mut handles = AdvertisementSet::handles(&sets);
        peripheral.advertise_ext(&sets, &mut handles).await?
    };
    #[cfg(not(feature = "ext_adv"))]
    let advertiser = {
        let _ = address;
        peripheral.advertise(&adv.params(), adv.build(&mut buf)?).await?
    };
    let conn = with_timeout(timeout, advertiser.accept())
        .await
        .map_err(|_| Error::Timeout)??;
    Ok(conn.with_attribute_server(server)?)
}

#[cfg(test)]
mod tests {
    use rmk_types::ble::BLE_ADV_NAME_MAX_LEN;
    use trouble_host::prelude::{Address, Advertisement, PhyKind};

    use super::Adv;

    #[test]
    fn legacy_advertisements_use_1m_and_fit_the_payload_budget() {
        // Where BLE_ADV_NAME_MAX_LEN comes from: this name fills the budget exactly.
        const LONGEST_NAME: &str = "0123456789abcdef";
        assert_eq!(LONGEST_NAME.len(), BLE_ADV_NAME_MAX_LEN);
        for adv in [
            Adv::Host { name: LONGEST_NAME },
            Adv::HostReconnect,
            Adv::Directed(Address::random([1, 2, 3, 4, 5, 0xC6])),
            Adv::SplitPeripheral { id: 0xFF },
            Adv::DongleSeeking,
        ] {
            assert!(adv.build(&mut [0; 31]).is_ok());
            assert_eq!(adv.params().primary_phy, PhyKind::Le1M);
        }
        assert!(
            Adv::Host {
                name: "0123456789abcdefg"
            }
            .build(&mut [0; 31])
            .is_err()
        );
    }

    #[test]
    fn host_reconnect_has_only_flags() {
        let mut buf = [0; 31];
        let Advertisement::ConnectableScannableUndirected { adv_data, scan_data } =
            Adv::HostReconnect.build(&mut buf).unwrap()
        else {
            panic!("reconnection must use undirected advertising");
        };
        assert_eq!(adv_data, [0x02, 0x01, 0x04]);
        assert!(scan_data.is_empty());
    }

    #[test]
    fn every_rmk_kind_round_trips_through_an_advertisement() {
        for adv in [Adv::SplitPeripheral { id: 2 }, Adv::DongleSeeking] {
            let mut buf = [0; 31];
            adv.build(&mut buf).unwrap();
            assert_eq!(Adv::decode(&buf), Some(adv));
        }
    }

    #[test]
    fn decode_skips_preceding_structures() {
        // An 18-byte 128-bit service UUID list sits between the flags and the MSD.
        let mut data = [0u8; 27];
        data[..5].copy_from_slice(&[0x02, 0x01, 0x06, 0x11, 0x07]);
        data[21..].copy_from_slice(&[0x05, 0xFF, 0x53, 0x52, 0x00, 0x02]);
        assert_eq!(Adv::decode(&data), Some(Adv::SplitPeripheral { id: 2 }));
    }

    #[test]
    fn decode_rejects_foreign_unknown_and_malformed_reports() {
        // Another vendor's company id.
        assert_eq!(
            Adv::decode(&[0x02, 0x01, 0x04, 0x05, 0xFF, 0x4C, 0x00, 0x00, 0x02]),
            None
        );
        // A kind a newer firmware knows and this one does not.
        assert_eq!(Adv::decode(&[0x04, 0xFF, 0x53, 0x52, 0x7F]), None);
        // A length running past the end of the report.
        assert_eq!(Adv::decode(&[0x02, 0x01, 0x04, 0x09, 0xFF, 0x53]), None);
    }
}
