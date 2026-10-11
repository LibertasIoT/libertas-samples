use super::SprinklerData;
use alloc::vec::Vec;
use core::ops::Bound::Included;
use libertas::data::{
    Data, Direction, EncodedKey, KeyRange, Mode, ScanOptions, Target, Transaction,
};
use libertas::{LibertasDevice, LibertasEndpoint};
use libertas_macros::{
    LibertasExport, LibertasKeyCodec, LibertasKeyDecode, LibertasKeyFieldCodec,
    LibertasKeyFieldDecode,
};

/// Saved sprinkler identities
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasExport, LibertasKeyCodec, LibertasKeyDecode,
)]
pub enum SprinklerKey {
    /// System settings and weather archives
    System {
        endpoint: LibertasEndpoint,
        record: SystemRecord,
    },
    /// One valve's settings, accounting and reports
    Zone {
        valve: LibertasDevice,
        record: ZoneRecord,
    },
}

/// System record identity
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasExport, LibertasKeyFieldCodec, LibertasKeyFieldDecode,
)]
pub enum SystemRecord {
    ArchiveState,
    Location,
    WateringMode,
    WinterizationReminder,
    WeatherHistory { generation: u64, index: i64 },
    WeatherObservation { generation: u64, index: i64 },
}

/// Valve record identity
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasExport, LibertasKeyFieldCodec, LibertasKeyFieldDecode,
)]
pub enum ZoneRecord {
    Memory,
    ActivityState,
    Restart,
    WaterEvent { index: i64 },
    Activity { index: i64 },
    DailyReport { index: i64 },
    ModeledWeatherGap { index: i64 },
}

pub(super) type Tx = Transaction<SprinklerKey, SprinklerData>;
pub(super) fn zone(valve: LibertasDevice, record: ZoneRecord) -> SprinklerKey {
    SprinklerKey::Zone { valve, record }
}
pub(super) fn system(endpoint: LibertasEndpoint, record: SystemRecord) -> SprinklerKey {
    SprinklerKey::System { endpoint, record }
}
pub(super) fn get(key: SprinklerKey) -> Option<SprinklerData> {
    let target = Target::keys(&[(&key).into()]).unwrap();
    Data::current().get(&target, (&key).into())
}
pub(super) fn put(key: SprinklerKey, value: &SprinklerData) {
    let target = Target::keys(&[(&key).into()]).unwrap();
    Data::current().put(&target, (&key).into(), value);
}
pub(super) fn begin_zone(valve: LibertasDevice) -> Tx {
    let encoded = EncodedKey::new(&zone(valve, ZoneRecord::Memory)).unwrap();
    Data::current().begin(
        &Target::prefix(&encoded.as_bytes()[..10]).unwrap(),
        Mode::ReadWrite,
    )
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ArchiveRecord {
    pub index: i64,
    pub data: SprinklerData,
}

/// A typed history selection, not a host database handle.
#[derive(Clone, Copy)]
pub(super) enum Archive {
    WeatherHistory {
        endpoint: LibertasEndpoint,
        generation: u64,
    },
    WeatherObservation {
        endpoint: LibertasEndpoint,
        generation: u64,
    },
    WaterEvents {
        valve: LibertasDevice,
    },
    Activities {
        valve: LibertasDevice,
    },
    DailyReports {
        valve: LibertasDevice,
    },
    ModeledWeatherGaps {
        valve: LibertasDevice,
    },
}
impl Archive {
    pub fn key(self, index: i64) -> SprinklerKey {
        match self {
            Self::WeatherHistory {
                endpoint,
                generation,
            } => system(endpoint, SystemRecord::WeatherHistory { generation, index }),
            Self::WeatherObservation {
                endpoint,
                generation,
            } => system(
                endpoint,
                SystemRecord::WeatherObservation { generation, index },
            ),
            Self::WaterEvents { valve } => zone(valve, ZoneRecord::WaterEvent { index }),
            Self::Activities { valve } => zone(valve, ZoneRecord::Activity { index }),
            Self::DailyReports { valve } => zone(valve, ZoneRecord::DailyReport { index }),
            Self::ModeledWeatherGaps { valve } => {
                zone(valve, ZoneRecord::ModeledWeatherGap { index })
            }
        }
    }
    pub fn target(self) -> Target<SprinklerKey> {
        let encoded = EncodedKey::new(&self.key(0)).unwrap();
        // Indexed variants occupy the maximum field-union width; index is last.
        Target::prefix(&encoded.as_bytes()[..encoded.as_bytes().len() - 8]).unwrap()
    }
    pub fn range(self, first: i64, last: i64) -> KeyRange<SprinklerKey> {
        KeyRange::new(
            Included(EncodedKey::new(&self.key(first)).unwrap()),
            Included(EncodedKey::new(&self.key(last)).unwrap()),
        )
    }
    pub fn index(self, key: &EncodedKey<SprinklerKey>) -> i64 {
        let decoded = key.decode().expect("sprinkler archive key");
        let index = match decoded {
            SprinklerKey::System {
                record:
                    SystemRecord::WeatherHistory { index, .. }
                    | SystemRecord::WeatherObservation { index, .. },
                ..
            }
            | SprinklerKey::Zone {
                record:
                    ZoneRecord::WaterEvent { index }
                    | ZoneRecord::Activity { index }
                    | ZoneRecord::DailyReport { index }
                    | ZoneRecord::ModeledWeatherGap { index },
                ..
            } => index,
            _ => panic!("unexpected sprinkler archive key"),
        };
        assert_eq!(self.key(index), decoded, "archive scope mismatch");
        index
    }
    pub fn begin(self, mode: Mode) -> Tx {
        Data::current().begin(&self.target(), mode)
    }
    pub fn get(self, index: i64) -> Option<ArchiveRecord> {
        get(self.key(index)).map(|data| ArchiveRecord { index, data })
    }
    pub fn put(self, index: i64, value: &SprinklerData) {
        put(self.key(index), value);
    }
    pub fn remove(self, index: i64) {
        let key = self.key(index);
        Data::<SprinklerKey, SprinklerData>::current()
            .delete(&Target::keys(&[(&key).into()]).unwrap(), (&key).into());
    }
    pub fn bounds(self) -> Option<(i64, i64)> {
        let mut tx = self.begin(Mode::ReadOnly);
        let result = self.bounds_in(&mut tx);
        tx.commit();
        result
    }
    pub fn bounds_in(self, tx: &mut Tx) -> Option<(i64, i64)> {
        let range = self.range(i64::MIN, i64::MAX);
        let mut options = ScanOptions::new(1);
        let first = tx.scan_keys(&range, &options).items.into_iter().next()?;
        options.direction = Direction::Descending;
        let last = tx
            .scan_keys(&range, &options)
            .items
            .into_iter()
            .next()
            .expect("nonempty archive");
        Some((self.index(&first), self.index(&last)))
    }
    pub fn records(self, index: i64, direction: Direction, maximum: usize) -> Vec<ArchiveRecord> {
        let mut tx = self.begin(Mode::ReadOnly);
        let result = self.records_in(&mut tx, index, direction, maximum);
        tx.commit();
        result
    }
    pub fn records_in(
        self,
        tx: &mut Tx,
        index: i64,
        direction: Direction,
        maximum: usize,
    ) -> Vec<ArchiveRecord> {
        let range = if direction == Direction::Descending {
            self.range(i64::MIN, index)
        } else {
            self.range(index, i64::MAX)
        };
        let mut records = Vec::new();
        let mut options = ScanOptions::new(64);
        options.direction = direction;
        while records.len() < maximum {
            options.limit = (maximum - records.len()).min(64) as u32;
            let page = tx.scan(&range, &options);
            records.extend(page.items.into_iter().map(|entry| ArchiveRecord {
                index: self.index(&entry.key),
                data: entry.value,
            }));
            options.continuation = page.next;
            if options.continuation.is_none() {
                break;
            }
        }
        records
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;

    #[test]
    fn production_key_layout_and_order_are_stable() {
        let owner = 0x0102_0304_0506_0708;
        let key = zone(owner, ZoneRecord::DailyReport { index: -1 });
        let encoded = EncodedKey::new(&key).unwrap();
        assert_eq!(
            encoded.as_bytes(),
            &[
                0, 1, 0x81, 2, 3, 4, 5, 6, 7, 8, 0, 5, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                0xff,
            ]
        );
        assert_eq!(encoded.decode().unwrap(), key);
        let archive = Archive::WeatherHistory {
            endpoint: owner,
            generation: 1,
        };
        let first = EncodedKey::new(&archive.key(i64::MIN)).unwrap();
        assert_eq!(
            first.as_bytes(),
            &[
                0, 0, 0x81, 2, 3, 4, 5, 6, 7, 8, 0, 4, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0,
                0,
            ]
        );
        let last = EncodedKey::new(&archive.key(i64::MAX)).unwrap();
        let next = EncodedKey::new(
            &Archive::WeatherHistory {
                endpoint: owner,
                generation: 2,
            }
            .key(i64::MIN),
        )
        .unwrap();
        assert!(first.as_bytes() < last.as_bytes());
        assert!(last.as_bytes() < next.as_bytes());
        assert_ne!(&last.as_bytes()[..20], &next.as_bytes()[..20]);
        assert_eq!(
            EncodedKey::new(&zone(owner, ZoneRecord::Memory))
                .unwrap()
                .as_bytes()
                .len(),
            20
        );
        assert_eq!(
            EncodedKey::new(&system(owner, SystemRecord::Location))
                .unwrap()
                .as_bytes()
                .len(),
            28
        );
    }
}
