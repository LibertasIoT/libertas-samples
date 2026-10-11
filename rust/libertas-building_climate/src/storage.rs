use super::{
    BuildingHvacMachineLearningModelSetV1 as ModelSet,
    BuildingHvacMachineLearningModelSlotV1 as ModelSlot,
    BuildingHvacMachineLearningModelV1 as Model, BuildingHvacPersistentData as Value,
};
use core::ops::Bound::{Excluded, Included, Unbounded};
use libertas::data::{
    Data, Direction, EncodedKey, KeyRange, Mode, ScanOptions, Target, Transaction,
};
use libertas::{AvroDecode, LibertasEndpoint};
use libertas_macros::{
    LibertasAvroDecode, LibertasAvroEncode, LibertasExport, LibertasKeyCodec, LibertasKeyDecode,
    LibertasKeyFieldCodec, LibertasKeyFieldDecode,
};
use sha2::{Digest, Sha256};

/// Climate persistence keys
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasExport, LibertasKeyCodec, LibertasKeyDecode,
)]
pub enum ClimateKey {
    Building {
        record: BuildingRecord,
    },
    Room {
        endpoint: LibertasEndpoint,
        record: RoomRecord,
    },
    Models {
        endpoint: LibertasEndpoint,
        record: ModelRecord,
    },
}
/// Building-wide cached inputs
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasExport, LibertasKeyFieldCodec, LibertasKeyFieldDecode,
)]
pub enum BuildingRecord {
    OutdoorTemperature,
    OutdoorHumidity,
    OutdoorAirQuality,
    WeatherHistory,
    WeatherCurrent,
    WeatherForecast,
    ProviderAirQuality,
    ExternalFeatures,
}
/// Room state and training history
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasExport, LibertasKeyFieldCodec, LibertasKeyFieldDecode,
)]
pub enum RoomRecord {
    Control,
    Statistics,
    Learning,
    Sensors,
    Urgent,
    Sample { index: i64 },
}
/// Model manifest and immutable artifact parts
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasExport, LibertasKeyFieldCodec, LibertasKeyFieldDecode,
)]
pub enum ModelRecord {
    Manifest,
    Part { digest: [u8; 32], part: u16 },
}

/// Reference to a complete, checksummed model artifact
#[derive(Clone, Debug, PartialEq, LibertasExport, LibertasAvroEncode, LibertasAvroDecode)]
pub struct ModelReference {
    #[libertas_size(min = 32, max = 32)]
    pub digest: Vec<u8>,
    pub byte_length: u32,
}
/// Independently promoted model with its immediate rollback artifact
#[derive(Clone, Debug, PartialEq, LibertasExport, LibertasAvroEncode, LibertasAvroDecode)]
pub struct StoredModelSlot {
    pub active: ModelReference,
    pub previous: Option<ModelReference>,
}

pub(super) type Tx = Transaction<ClimateKey, Value>;
pub(super) fn room(endpoint: LibertasEndpoint, record: RoomRecord) -> ClimateKey {
    ClimateKey::Room { endpoint, record }
}
pub(super) fn building(record: BuildingRecord) -> ClimateKey {
    ClimateKey::Building { record }
}
pub(super) fn get(key: ClimateKey) -> Option<Value> {
    Data::current().get(&Target::keys(&[(&key).into()]).unwrap(), (&key).into())
}
pub(super) fn put(key: ClimateKey, value: &Value) {
    Data::current().put(
        &Target::keys(&[(&key).into()]).unwrap(),
        (&key).into(),
        value,
    );
}
pub(super) fn remove(key: ClimateKey) {
    Data::<ClimateKey, Value>::current()
        .delete(&Target::keys(&[(&key).into()]).unwrap(), (&key).into());
}
pub(super) fn begin_room(endpoint: LibertasEndpoint) -> Tx {
    let encoded = EncodedKey::new(&room(endpoint, RoomRecord::Control)).unwrap();
    Data::current().begin(
        &Target::prefix(&encoded.as_bytes()[..10]).unwrap(),
        Mode::ReadWrite,
    )
}
pub(super) fn begin_building() -> Tx {
    let encoded = EncodedKey::new(&building(BuildingRecord::WeatherHistory)).unwrap();
    Data::current().begin(
        &Target::prefix(&encoded.as_bytes()[..2]).unwrap(),
        Mode::ReadWrite,
    )
}
pub(super) fn sample_range(
    endpoint: LibertasEndpoint,
    first: i64,
    last: i64,
) -> KeyRange<ClimateKey> {
    KeyRange::new(
        Included(EncodedKey::new(&room(endpoint, RoomRecord::Sample { index: first })).unwrap()),
        Included(EncodedKey::new(&room(endpoint, RoomRecord::Sample { index: last })).unwrap()),
    )
}
pub(super) fn prune_samples(endpoint: LibertasEndpoint, before: i64) {
    let Some(last) = before.checked_sub(1) else {
        return;
    };
    loop {
        let mut tx = begin_room(endpoint);
        let page = tx.scan_keys(
            &sample_range(endpoint, i64::MIN, last),
            &ScanOptions::new(64),
        );
        let complete = page.next.is_none();
        for key in page.items {
            tx.delete((&key).into());
        }
        tx.commit();
        if complete {
            break;
        }
    }
}
pub(super) fn samples(
    endpoint: LibertasEndpoint,
    through: i64,
    maximum: usize,
) -> Vec<(i64, Value)> {
    let sample_key = EncodedKey::new(&room(endpoint, RoomRecord::Sample { index: 0 })).unwrap();
    let mut tx = Data::current().begin(
        &Target::prefix(&sample_key.as_bytes()[..12]).unwrap(),
        Mode::ReadOnly,
    );
    let range = sample_range(endpoint, i64::MIN, through);
    let mut options = ScanOptions::new(64);
    options.direction = Direction::Descending;
    let mut rows = Vec::new();
    while rows.len() < maximum {
        options.limit = (maximum - rows.len()).min(64) as u32;
        let page = tx.scan(&range, &options);
        for entry in page.items {
            let ClimateKey::Room {
                endpoint: owner,
                record: RoomRecord::Sample { index },
            } = entry.key.decode().expect("sample key")
            else {
                panic!("unexpected sample key")
            };
            assert_eq!(owner, endpoint);
            rows.push((index, entry.value));
        }
        options.continuation = page.next;
        if options.continuation.is_none() {
            break;
        }
    }
    tx.commit();
    rows
}

const MODEL_PART_BYTES: usize = 1024 * 1024;
const MODEL_PARTS_PER_TRANSACTION: usize = 8;
// A model's existing 16 MiB UBJSON limit plus its bounded feature manifest.
const MAX_ENCODED_MODEL_BYTES: usize = 20 * 1024 * 1024;
fn model_key(endpoint: LibertasEndpoint, record: ModelRecord) -> ClimateKey {
    ClimateKey::Models { endpoint, record }
}
fn model_target(endpoint: LibertasEndpoint) -> Target<ClimateKey> {
    let key = EncodedKey::new(&model_key(endpoint, ModelRecord::Manifest)).unwrap();
    Target::prefix(&key.as_bytes()[..10]).unwrap()
}
fn stage_model(endpoint: LibertasEndpoint, model: &Model) -> ModelReference {
    assert_eq!(model.room_endpoint, endpoint);
    assert!(model.is_well_formed(), "invalid model artifact");
    let bytes = model.to_avro();
    assert!(bytes.len() <= MAX_ENCODED_MODEL_BYTES);
    let reference = ModelReference {
        digest: Sha256::digest(&bytes).to_vec(),
        byte_length: bytes.len() as u32,
    };
    let first = EncodedKey::new(&model_key(
        endpoint,
        ModelRecord::Part {
            digest: reference
                .digest
                .as_slice()
                .try_into()
                .expect("model digest width"),
            part: 0,
        },
    ))
    .unwrap();
    let target = Target::prefix(&first.as_bytes()[..44]).unwrap();
    for (batch, bytes) in bytes
        .chunks(MODEL_PART_BYTES * MODEL_PARTS_PER_TRANSACTION)
        .enumerate()
    {
        let mut tx = Data::current().begin(&target, Mode::ReadWrite);
        for (offset, bytes) in bytes.chunks(MODEL_PART_BYTES).enumerate() {
            let key = model_key(
                endpoint,
                ModelRecord::Part {
                    digest: reference
                        .digest
                        .as_slice()
                        .try_into()
                        .expect("model digest width"),
                    part: (batch * MODEL_PARTS_PER_TRANSACTION + offset) as u16,
                },
            );
            tx.put(
                (&key).into(),
                &Value::ModelPartV2 {
                    bytes: bytes.to_vec(),
                },
            );
        }
        tx.commit();
    }
    reference
}
fn read_model(tx: &mut Tx, endpoint: LibertasEndpoint, reference: &ModelReference) -> Model {
    let length = reference.byte_length as usize;
    assert!(
        (1..=MAX_ENCODED_MODEL_BYTES).contains(&length),
        "invalid model length"
    );
    let mut bytes = Vec::with_capacity(length);
    for part in 0..length.div_ceil(MODEL_PART_BYTES) {
        let key = model_key(
            endpoint,
            ModelRecord::Part {
                digest: reference
                    .digest
                    .as_slice()
                    .try_into()
                    .expect("model digest width"),
                part: part as u16,
            },
        );
        let Some(Value::ModelPartV2 { bytes: fragment }) = tx.get((&key).into()) else {
            panic!("missing model part")
        };
        assert_eq!(
            fragment.len(),
            (length - bytes.len()).min(MODEL_PART_BYTES),
            "model part length"
        );
        bytes.extend_from_slice(&fragment);
    }
    assert_eq!(
        <[u8; 32]>::from(Sha256::digest(&bytes)),
        reference.digest.as_slice(),
        "model artifact checksum"
    );
    let mut offset = 0;
    let model = Model::avro_decode(&bytes, &mut offset).expect("model artifact encoding");
    assert_eq!(offset, bytes.len());
    assert_eq!(model.room_endpoint, endpoint);
    model
}
pub(super) fn load_models(endpoint: LibertasEndpoint) -> Option<ModelSet> {
    let mut tx = Data::current().begin(&model_target(endpoint), Mode::ReadOnly);
    let result = match tx.get((&model_key(endpoint, ModelRecord::Manifest)).into()) {
        None => None,
        Some(Value::ModelManifestV2 { slots }) => {
            assert!(slots.len() <= 3, "too many model slots");
            let models = slots
                .into_iter()
                .map(|slot| {
                    let active_model = read_model(&mut tx, endpoint, &slot.active);
                    let previous_model = slot
                        .previous
                        .as_ref()
                        .map(|reference| read_model(&mut tx, endpoint, reference));
                    ModelSlot {
                        horizon: active_model.horizon,
                        active_model,
                        previous_model,
                    }
                })
                .collect();
            Some(ModelSet {
                room_endpoint: endpoint,
                models,
            })
        }
        Some(_) => panic!("unexpected model manifest value"),
    };
    tx.commit();
    result
}
pub(super) fn save_models(models: &ModelSet) {
    assert!(models.is_well_formed(), "invalid model set");
    let endpoint = models.room_endpoint;
    // Stage complete immutable artifacts before publishing the small manifest.
    // A stopped staging operation leaves the previous manifest readable.
    let slots: Vec<_> = models
        .models
        .iter()
        .map(|slot| StoredModelSlot {
            active: stage_model(endpoint, &slot.active_model),
            previous: slot
                .previous_model
                .as_ref()
                .map(|model| stage_model(endpoint, model)),
        })
        .collect();
    let live: Vec<_> = slots
        .iter()
        .flat_map(|slot| {
            core::iter::once(slot.active.digest.clone())
                .chain(slot.previous.as_ref().map(|r| r.digest.clone()))
        })
        .collect();
    let mut tx: Tx = Data::current().begin(&model_target(endpoint), Mode::ReadWrite);
    tx.put(
        (&model_key(endpoint, ModelRecord::Manifest)).into(),
        &Value::ModelManifestV2 { slots },
    );
    tx.commit();
    // After publication, retire unreferenced artifacts in bounded transactions.
    // An interrupted cleanup leaves only unused parts, never a partial manifest.
    let mut start = Unbounded;
    loop {
        let mut tx: Tx = Data::current().begin(&model_target(endpoint), Mode::ReadWrite);
        let page = tx.scan_keys(
            &KeyRange::new(start.clone(), Unbounded),
            &ScanOptions::new(64),
        );
        let next = page.items.last().cloned();
        let complete = page.next.is_none();
        for key in page.items {
            if let ClimateKey::Models {
                record: ModelRecord::Part { digest, .. },
                ..
            } = key.decode().expect("model key")
                && !live.iter().any(|saved| saved.as_slice() == digest)
            {
                tx.delete((&key).into());
            }
        }
        tx.commit();
        if complete {
            break;
        }
        start = Excluded(next.expect("nonempty model page"));
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;

    #[test]
    fn production_key_layout_and_artifact_prefix_are_stable() {
        let owner = 0x0102_0304_0506_0708;
        let sample = room(owner, RoomRecord::Sample { index: -1 });
        let encoded = EncodedKey::new(&sample).unwrap();
        assert_eq!(
            encoded.as_bytes(),
            &[
                0, 1, 0x81, 2, 3, 4, 5, 6, 7, 8, 0, 5, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                0xff,
            ]
        );
        assert_eq!(encoded.decode().unwrap(), sample);
        assert_eq!(
            EncodedKey::new(&building(BuildingRecord::ExternalFeatures))
                .unwrap()
                .as_bytes(),
            &[0, 0, 0, 7]
        );
        let part = model_key(
            owner,
            ModelRecord::Part {
                digest: [0xaa; 32],
                part: 0x0102,
            },
        );
        let encoded = EncodedKey::new(&part).unwrap();
        assert_eq!(encoded.as_bytes().len(), 46);
        assert_eq!(
            &encoded.as_bytes()[..12],
            &[0, 2, 0x81, 2, 3, 4, 5, 6, 7, 8, 0, 1]
        );
        assert_eq!(&encoded.as_bytes()[12..44], &[0xaa; 32]);
        assert_eq!(&encoded.as_bytes()[44..], &[1, 2]);
        assert_eq!(encoded.decode().unwrap(), part);
        assert_eq!(
            EncodedKey::new(&model_key(owner, ModelRecord::Manifest))
                .unwrap()
                .as_bytes()
                .len(),
            46
        );
    }
}
