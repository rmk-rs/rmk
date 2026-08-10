//! Every advertisement RMK sends, in one place.

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::{Duration, with_timeout};
use trouble_host::prelude::appearance::human_interface_device::KEYBOARD;
use trouble_host::prelude::service::{BATTERY, HUMAN_INTERFACE_DEVICE};
use trouble_host::prelude::*;

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
        AdStructure::encode_slice(adv_data, &mut buf[..])?;
        Ok(Advertisement::ConnectableScannableUndirected {
            adv_data: &buf[..],
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

    /// A host link can afford a slow interval and gains from 2M; every other
    /// peer is RMK's own hardware, where reaching it fast matters more.
    fn params(&self) -> AdvertisementParameters {
        let (phy, interval) = match self {
            Self::Host { .. } => (PhyKind::Le2M, Duration::from_millis(200)),
            _ => (PhyKind::Le1M, Duration::from_millis(50)),
        };
        AdvertisementParameters {
            primary_phy: phy,
            secondary_phy: phy,
            tx_power: TxPower::Plus8dBm,
            interval_min: interval,
            interval_max: interval,
            ..Default::default()
        }
    }
}

/// Broadcast `adv` and hand back the connection a central makes on it, or
/// [`Error::Timeout`] if none does within `timeout`.
pub(crate) async fn advertise<'a, 'b, C: Controller, const ATT: usize, const CONN: usize>(
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    server: &'b AttributeServer<'_, NoopRawMutex, DefaultPacketPool, ATT, CONN>,
    adv: Adv<'_>,
    timeout: Duration,
) -> Result<GattConnection<'a, 'b, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut buf = [0; 31];
    let advertiser = peripheral.advertise(&adv.params(), adv.build(&mut buf)?).await?;
    let conn = with_timeout(timeout, advertiser.accept())
        .await
        .map_err(|_| Error::Timeout)??;
    Ok(conn.with_attribute_server(server)?)
}

#[cfg(test)]
mod tests {
    use super::Adv;

    /// Overrunning the 31-byte legacy advertisement only fails at runtime.
    fn fits(adv: Adv<'_>) -> bool {
        adv.build(&mut [0; 31]).is_ok()
    }

    #[test]
    fn every_advertisement_fits_the_legacy_budget() {
        assert!(fits(Adv::SplitPeripheral { id: 0xFF }));
        assert!(fits(Adv::DongleSeeking));
        // Flags, UUIDs and appearance leave 16 bytes for the name.
        assert!(fits(Adv::Host {
            name: "0123456789abcdef"
        }));
        assert!(!fits(Adv::Host {
            name: "0123456789abcdefg"
        }));
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
