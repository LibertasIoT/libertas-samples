use core::ops::Bound::Included;
use libertas::LibertasEndpoint;
use libertas::data::{Data, EncodedKey, KeyRange, Mode, ScanOptions, Target, Transaction};
pub(super) use libertas_weather::{
    SprinklerWeatherKey as Key, SprinklerWeatherRecordKey as RecordKey,
};
use libertas_weather::{SprinklerWeatherLocationV1, SprinklerWeatherPersistentData as Value};

pub(super) type Tx = Transaction<Key, Value>;

pub(super) fn key(endpoint: LibertasEndpoint, record: RecordKey) -> Key {
    Key::Endpoint { endpoint, record }
}

pub(super) fn begin(endpoint: LibertasEndpoint) -> Tx {
    let encoded = EncodedKey::new(&key(endpoint, RecordKey::Location)).unwrap();
    // Root u16 tag and signed eight-byte endpoint; the nested record follows.
    let target = Target::prefix(&encoded.as_bytes()[..10]).unwrap();
    Data::current().begin(&target, Mode::ReadWrite)
}

pub(super) fn get(endpoint: LibertasEndpoint, record: RecordKey) -> Option<Value> {
    let key = key(endpoint, record);
    let target = Target::keys(&[(&key).into()]).unwrap();
    Data::<Key, Value>::current().get(&target, (&key).into())
}

pub(super) fn put(endpoint: LibertasEndpoint, record: RecordKey, value: &Value) {
    let key = key(endpoint, record);
    let target = Target::keys(&[(&key).into()]).unwrap();
    Data::<Key, Value>::current().put(&target, (&key).into(), value);
}

pub(super) fn remove(endpoint: LibertasEndpoint, record: RecordKey) {
    let key = key(endpoint, record);
    let target = Target::keys(&[(&key).into()]).unwrap();
    Data::<Key, Value>::current().delete(&target, (&key).into());
}

fn history_range(endpoint: LibertasEndpoint) -> KeyRange<Key> {
    KeyRange::new(
        Included(
            EncodedKey::new(&key(endpoint, RecordKey::HistoryPeriod { starts_at: 0 })).unwrap(),
        ),
        Included(
            EncodedKey::new(&key(
                endpoint,
                RecordKey::HistoryPeriod {
                    starts_at: u64::MAX,
                },
            ))
            .unwrap(),
        ),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct HistoryRecord {
    pub index: i64,
    pub data: Value,
}

pub(super) fn history(
    tx: &mut Tx,
    endpoint: LibertasEndpoint,
    maximum: usize,
) -> Vec<HistoryRecord> {
    let range = history_range(endpoint);
    let mut rows = Vec::new();
    let mut options = ScanOptions::new(64);
    loop {
        options.limit = (maximum - rows.len()).min(64) as u32;
        if options.limit == 0 {
            break;
        }
        let page = tx.scan(&range, &options);
        for entry in page.items {
            let Key::Endpoint {
                endpoint: owner,
                record: RecordKey::HistoryPeriod { starts_at },
            } = entry.key.decode().expect("weather key")
            else {
                panic!("unexpected weather key")
            };
            assert_eq!(owner, endpoint);
            rows.push(HistoryRecord {
                index: i64::try_from(starts_at).expect("weather timestamp range"),
                data: entry.value,
            });
        }
        options.continuation = page.next;
        if options.continuation.is_none() {
            break;
        }
    }
    rows
}

pub(super) fn clear_history(tx: &mut Tx, endpoint: LibertasEndpoint) {
    tx.delete((&key(endpoint, RecordKey::HistoryMetadata)).into());
    loop {
        let page = tx.scan_keys(&history_range(endpoint), &ScanOptions::new(64));
        if page.items.is_empty() {
            break;
        }
        for key in page.items {
            tx.delete((&key).into());
        }
    }
}

pub(super) fn clear_sections(
    endpoint: LibertasEndpoint,
    location: Option<SprinklerWeatherLocationV1>,
) {
    let mut tx = begin(endpoint);
    clear_history(&mut tx, endpoint);
    tx.delete((&key(endpoint, RecordKey::Current)).into());
    tx.delete((&key(endpoint, RecordKey::Forecast)).into());
    if let Some(location) = location {
        tx.put(
            (&key(endpoint, RecordKey::Location)).into(),
            &Value::LocationV1 { location },
        );
    }
    tx.commit();
}
