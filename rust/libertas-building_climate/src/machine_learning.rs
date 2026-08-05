//! Bounded XGBoost thermal prediction for the Hub runtime.
//!
//! The public V1 values in this module describe training evidence, persisted
//! model artifacts, and user-visible prediction state. XGBoost itself stays
//! behind the safe `xgb` wrapper and is owned by one worker thread.

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    thread,
};

use libertas::{
    IndexDirection, LibertasDateTime, LibertasEndpoint, NotificationArgument,
    libertas_data_open_indexed, libertas_data_read_indexed, libertas_data_read_indexed_range,
    libertas_data_remove_indexed_records, libertas_data_write_indexed,
};
use libertas_macros::{LibertasAvroDecode, LibertasAvroEncode, LibertasExport};
use sha2::{Digest, Sha256};
#[cfg(target_os = "linux")]
use xgb::{
    Booster, DMatrix,
    parameters::{
        self, BoosterType,
        learning::{LearningTaskParametersBuilder, Objective},
        tree::{TreeBoosterParametersBuilder, TreeMethod},
    },
};

/// Machine-learning feature schema version
/// Identifies the exact ordered V1 thermal feature vector. A persisted model is
/// rejected rather than interpreted with a different feature order.
pub const BUILDING_HVAC_ML_FEATURE_SCHEMA_VERSION: u32 = 1;

/// Maximum machine-learning feature count
/// Bounds one building-specific dense vector while leaving room for every
/// configured thermostat, room, sensor class, weather horizon, utility input,
/// equipment measurement, and controller-history signal. Missing values still
/// occupy their named column and are passed to XGBoost as `NaN`.
pub const BUILDING_HVAC_ML_MAXIMUM_FEATURE_COUNT: usize = 8_192;

/// Maximum machine-learning feature-name bytes
/// Bounds each stable human- and AI-readable column identifier.
pub const BUILDING_HVAC_ML_MAXIMUM_FEATURE_NAME_BYTES: usize = 192;

/// Minimum training samples
/// The minimum number of ordered, valid, labeled room periods required before
/// attempting a candidate model.
pub const BUILDING_HVAC_ML_MINIMUM_TRAINING_SAMPLES: usize = 14 * 24 * 4;

/// Machine-learning history retention
/// Keeps a complete 400-day rolling archive so every annual weather season is
/// represented with more than one month of overlap after a full year.
pub const BUILDING_HVAC_ML_HISTORY_RETENTION_SECONDS: u64 = 400 * 24 * 60 * 60;

/// Maximum retained samples per room
/// Bounds an inclusive 400-day indexed history of 15-minute observations.
pub const BUILDING_HVAC_ML_MAXIMUM_RETAINED_SAMPLES_PER_ROOM: usize = 400 * 24 * 4 + 1;

/// Recent adaptation window
/// Samples in the latest 91 days represent current equipment, envelope, and
/// occupancy behavior.
pub const BUILDING_HVAC_ML_RECENT_WINDOW_SECONDS: u64 = 91 * 24 * 60 * 60;

/// Maximum selected training samples per room
/// Bounds each worker command and XGBoost fit. When both periods have enough
/// evidence, half comes from the recent window and half from the older archive.
pub const BUILDING_HVAC_ML_MAXIMUM_TRAINING_SAMPLES_PER_ROOM: usize = 91 * 24 * 4;

/// Recent training share
/// Reserves half of a full training set for recent adaptation. The other half
/// is distributed across older seasonal, weather, and equipment-demand strata.
pub const BUILDING_HVAC_ML_RECENT_TRAINING_PERCENT: usize = 50;

/// Validation holdout percentage
/// The newest fifth of an ordered dataset is withheld from candidate training
/// and used only for time-forward validation.
pub const BUILDING_HVAC_ML_VALIDATION_PERCENT: usize = 20;

/// Minimum validation samples
/// At least one day of 15-minute periods is used to validate a candidate.
pub const BUILDING_HVAC_ML_MINIMUM_VALIDATION_SAMPLES: usize = 24 * 4;

/// Candidate promotion improvement
/// A candidate must reduce validation RMSE by at least this fraction relative
/// to the deterministic no-temperature-change prediction.
pub const BUILDING_HVAC_ML_MINIMUM_PROMOTION_IMPROVEMENT_NORMALIZED: f32 = 0.05;

/// XGBoost boost rounds
/// Bounds training work while retaining enough shallow trees for nonlinear
/// weather and equipment interactions.
pub const BUILDING_HVAC_ML_BOOST_ROUNDS: u32 = 128;

/// XGBoost maximum tree depth
/// Bounds model complexity and inference cost.
pub const BUILDING_HVAC_ML_MAXIMUM_TREE_DEPTH: u32 = 4;

/// XGBoost worker threads
/// Training and inference use exactly one native XGBoost thread so learning
/// cannot occupy every Hub processor.
pub const BUILDING_HVAC_ML_XGBOOST_THREADS: u32 = 1;

/// Machine-learning worker nice increment
/// The Linux worker lowers its CPU scheduling priority by this amount before
/// accepting work. A positive nice increment yields CPU time to the Libertas
/// application and other Hub services under contention.
pub const BUILDING_HVAC_ML_WORKER_NICE_INCREMENT: i32 = 10;

/// Maximum persisted UBJSON model bytes
/// Rejects unexpectedly large or corrupt model artifacts before loading them.
pub const BUILDING_HVAC_ML_MAXIMUM_MODEL_BYTES: usize = 16 * 1024 * 1024;

/// Maximum absolute thermal prediction
/// A model output outside this many degrees Celsius is rejected and replaced
/// with the deterministic fallback before it can influence control.
pub const BUILDING_HVAC_ML_MAXIMUM_PREDICTED_CHANGE_CELSIUS: f32 = 10.0;

/// Bounded machine-learning command queue
/// Prevents sensor or timer activity from creating unbounded training work.
pub const BUILDING_HVAC_ML_COMMAND_CAPACITY: usize = 8;

/// Bounded machine-learning result queue
/// Prevents an unavailable Libertas thread from creating unbounded results.
pub const BUILDING_HVAC_ML_RESULT_CAPACITY: usize = 16;

/// Bundled XGBoost version
/// The CPU-only XGBoost source revision compiled into the application.
pub const BUILDING_HVAC_XGBOOST_VERSION: &str = "3.0.0";

pub(crate) const BUILDING_HVAC_ML_SAMPLE_RESOURCE: &str = "HVAC_ML_SAMPLE";

/// Thermal prediction horizon V1
/// Selects one independently trained room-temperature-change model. Separate
/// models avoid silently combining targets with different error distributions.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    LibertasAvroDecode,
    LibertasAvroEncode,
    LibertasExport,
)]
pub enum BuildingHvacThermalPredictionHorizonV1 {
    /// Fifteen minutes
    /// Predicts room temperature movement during the next control period.
    FifteenMinutes,
    /// Thirty minutes
    /// Predicts near-term movement across two control periods.
    ThirtyMinutes,
    /// Sixty minutes
    /// Predicts the one-hour response used for preconditioning decisions.
    SixtyMinutes,
}

impl BuildingHvacThermalPredictionHorizonV1 {
    /// Horizon seconds
    /// Returns the prediction interval represented by this model.
    pub const fn seconds(self) -> u32 {
        match self {
            Self::FifteenMinutes => 15 * 60,
            Self::ThirtyMinutes => 30 * 60,
            Self::SixtyMinutes => 60 * 60,
        }
    }
}

/// Named machine-learning feature V1
/// Defines one stable column in the building-specific XGBoost matrix. The name
/// includes its family, stable device or room identity when applicable,
/// measurement, unit, and lookback or forecast horizon. `None` is encoded as
/// XGBoost's `NaN` missing value; a meaningful zero remains `Some(0.0)`.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacMachineLearningFeatureV1 {
    /// Feature name
    /// Stable lowercase identifier in
    /// `family.identity.measurement_unit.horizon` form. Names are sorted
    /// lexicographically and become the persisted model manifest.
    #[libertas_size(min = 1, max = 192)]
    pub name: String,
    /// Feature value
    /// Finite value in the unit stated by `name`. Absence means genuinely
    /// unavailable or inapplicable input and is passed to XGBoost as `NaN`.
    pub value: Option<f32>,
}

impl BuildingHvacMachineLearningFeatureV1 {
    fn is_well_formed(&self) -> bool {
        !self.name.is_empty()
            && self.name.len() <= BUILDING_HVAC_ML_MAXIMUM_FEATURE_NAME_BYTES
            && self.name.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            })
            && optional_finite(self.value)
    }
}

/// XGBoost thermal features V1
/// Contains the exact ordered inputs for one target-room observation. Every
/// configured room and thermostat receives its own stable columns so XGBoost
/// can learn shared-source and cross-zone correlations without a configured
/// equipment topology. Weather, utility, equipment, occupancy, air-quality,
/// time, controller-history, and lagged values remain separate rather than
/// being collapsed into lossy Boolean or aggregate demand features.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacMachineLearningFeaturesV1 {
    /// Target room
    /// Stable runtime endpoint of the room whose future temperature change is
    /// being predicted.
    pub target_room: LibertasEndpoint,
    /// Feature values
    /// Complete building-specific column set sorted by feature name. A source
    /// that is absent or stale retains its column with an absent value so every
    /// sample for one configuration has the same manifest.
    /// ----
    /// Named feature
    /// One finite value or an explicit XGBoost missing value.
    #[libertas_size(min = 1, max = 8192)]
    pub values: Vec<BuildingHvacMachineLearningFeatureV1>,
}

impl BuildingHvacMachineLearningFeaturesV1 {
    /// Well-formed features
    /// Rejects an empty or oversized vector, invalid names, nonfinite present
    /// values, and duplicate or unordered columns.
    pub fn is_well_formed(&self) -> bool {
        !self.values.is_empty()
            && self.values.len() <= BUILDING_HVAC_ML_MAXIMUM_FEATURE_COUNT
            && self.values.iter().all(|feature| feature.is_well_formed())
            && self
                .values
                .windows(2)
                .all(|pair| pair[0].name < pair[1].name)
    }

    /// Named feature value
    /// Returns one present value from the sorted vector. `None` represents
    /// either a missing column or a column whose value is missing.
    pub fn value(&self, name: &str) -> Option<f32> {
        self.values
            .binary_search_by(|feature| feature.name.as_str().cmp(name))
            .ok()
            .and_then(|index| self.values[index].value)
    }

    /// Ordered feature names
    /// Builds the exact self-describing XGBoost column manifest.
    pub fn feature_names(&self) -> Vec<String> {
        self.values
            .iter()
            .map(|feature| feature.name.clone())
            .collect()
    }

    #[cfg(any(target_os = "linux", test))]
    const fn dense_feature_count(&self) -> usize {
        self.values.len()
    }

    #[cfg(target_os = "linux")]
    fn append_dense(&self, values: &mut Vec<f32>) {
        values.extend(self.values.iter().map(|feature| missing(feature.value)));
    }

    /// Compact persisted vector
    /// Replaces repeated feature names with their deterministic manifest hash
    /// before a 15-minute observation is written to indexed history.
    pub fn compact(&self) -> BuildingHvacMachineLearningFeatureVectorV1 {
        let names = self.feature_names();
        BuildingHvacMachineLearningFeatureVectorV1 {
            manifest_sha256: feature_manifest_sha256(&names).to_vec(),
            feature_count: u16::try_from(self.values.len()).unwrap_or(u16::MAX),
            values: self
                .values
                .iter()
                .enumerate()
                .filter_map(|(index, feature)| {
                    Some(BuildingHvacMachineLearningIndexedFeatureV1 {
                        index: u16::try_from(index).ok()?,
                        value: feature.value?,
                    })
                })
                .collect(),
        }
    }
}

/// Indexed machine-learning feature V1
/// Stores one present value from a compact sparse observation. Missing columns
/// are omitted; semantic zeros remain present entries.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacMachineLearningIndexedFeatureV1 {
    /// Column index
    /// Zero-based position in the manifest named by the containing vector.
    pub index: u16,
    /// Value
    /// Finite present value. Missing values have no indexed entry.
    pub value: f32,
}

/// Compact machine-learning feature vector V1
/// Stores one ordered observation without repeating thousands of feature-name
/// strings in every indexed record. The corresponding full ordered names are
/// regenerated from configuration and checked against `manifest_sha256` before
/// a sample can be selected or trained.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacMachineLearningFeatureVectorV1 {
    /// Manifest SHA-256
    /// Thirty-two deterministic checksum bytes over the feature-schema version
    /// and ordered names, including each name length so concatenation cannot
    /// create ambiguity.
    #[libertas_size(min = 32, max = 32)]
    pub manifest_sha256: Vec<u8>,
    /// Feature count
    /// Total number of columns, including omitted missing columns.
    #[libertas_number(min = 1, max = 8192)]
    pub feature_count: u16,
    /// Present values
    /// Sorted sparse values. A missing column has no entry; a meaningful zero
    /// has an entry whose value is zero.
    /// ----
    /// Indexed feature
    /// One present finite value at its manifest column.
    #[libertas_size(max = 8192)]
    pub values: Vec<BuildingHvacMachineLearningIndexedFeatureV1>,
}

impl BuildingHvacMachineLearningFeatureVectorV1 {
    /// Well-formed compact vector
    /// Requires a SHA-256-sized manifest identity and a bounded finite vector.
    pub fn is_well_formed(&self) -> bool {
        self.manifest_sha256.len() == 32
            && self.feature_count != 0
            && usize::from(self.feature_count) <= BUILDING_HVAC_ML_MAXIMUM_FEATURE_COUNT
            && self.values.len() <= usize::from(self.feature_count)
            && self
                .values
                .iter()
                .all(|value| value.index < self.feature_count && value.value.is_finite())
            && self
                .values
                .windows(2)
                .all(|pair| pair[0].index < pair[1].index)
    }

    fn matches_feature_names(&self, names: &[String]) -> bool {
        let expected_manifest = feature_manifest_sha256(names);
        usize::from(self.feature_count) == names.len()
            && self.manifest_sha256.as_slice() == expected_manifest.as_slice()
    }

    fn value(&self, names: &[String], name: &str) -> Option<f32> {
        let index = names
            .binary_search_by(|candidate| candidate.as_str().cmp(name))
            .ok()?;
        let index = u16::try_from(index).ok()?;
        self.values
            .binary_search_by_key(&index, |feature| feature.index)
            .ok()
            .map(|position| self.values[position].value)
    }

    #[cfg(target_os = "linux")]
    fn dense_feature_count(&self) -> usize {
        usize::from(self.feature_count)
    }
}

fn feature_manifest_sha256(names: &[String]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(BUILDING_HVAC_ML_FEATURE_SCHEMA_VERSION.to_le_bytes());
    for name in names {
        digest.update(u64::try_from(name.len()).unwrap_or(u64::MAX).to_le_bytes());
        digest.update(name.as_bytes());
    }
    digest.finalize().into()
}

fn feature_manifest_is_well_formed(names: &[String]) -> bool {
    !names.is_empty()
        && names.len() <= BUILDING_HVAC_ML_MAXIMUM_FEATURE_COUNT
        && names.iter().all(|name| {
            !name.is_empty()
                && name.len() <= BUILDING_HVAC_ML_MAXIMUM_FEATURE_NAME_BYTES
                && name.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'_' | b'-')
                })
        })
        && names.windows(2).all(|pair| pair[0] < pair[1])
}

/// Machine-learning training sample V1
/// One indexed room observation and the temperature changes later measured at
/// each supported horizon. It is written only after at least one target becomes
/// known; missing targets remain available for later completion.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacMachineLearningSampleV1 {
    /// Observed at
    /// UTC timestamp of the feature observation and indexed database key.
    pub observed_at: LibertasDateTime,
    /// Room endpoint
    /// Stable room identity used to reject a sample read from the wrong indexed
    /// room history.
    pub room_endpoint: LibertasEndpoint,
    /// Features
    /// Compact sparse V1 thermal feature values at `observed_at`. The manifest
    /// hash must match the full ordered names regenerated for this building.
    pub features: BuildingHvacMachineLearningFeatureVectorV1,
    /// Fifteen-minute temperature change
    /// Fused room temperature after 15 minutes minus the observed temperature.
    pub temperature_change_15_minutes_celsius: Option<f32>,
    /// Thirty-minute temperature change
    /// Fused room temperature after 30 minutes minus the observed temperature.
    pub temperature_change_30_minutes_celsius: Option<f32>,
    /// Sixty-minute temperature change
    /// Fused room temperature after 60 minutes minus the observed temperature.
    pub temperature_change_60_minutes_celsius: Option<f32>,
}

impl BuildingHvacMachineLearningSampleV1 {
    /// Target value
    /// Returns the labeled temperature change for one model horizon.
    pub const fn target(&self, horizon: BuildingHvacThermalPredictionHorizonV1) -> Option<f32> {
        match horizon {
            BuildingHvacThermalPredictionHorizonV1::FifteenMinutes => {
                self.temperature_change_15_minutes_celsius
            }
            BuildingHvacThermalPredictionHorizonV1::ThirtyMinutes => {
                self.temperature_change_30_minutes_celsius
            }
            BuildingHvacThermalPredictionHorizonV1::SixtyMinutes => {
                self.temperature_change_60_minutes_celsius
            }
        }
    }

    /// Well-formed sample
    /// Requires valid features and at least one bounded finite target.
    pub fn is_well_formed(&self) -> bool {
        self.features.is_well_formed()
            && [
                self.temperature_change_15_minutes_celsius,
                self.temperature_change_30_minutes_celsius,
                self.temperature_change_60_minutes_celsius,
            ]
            .into_iter()
            .flatten()
            .all(|value| {
                in_range(
                    value,
                    -BUILDING_HVAC_ML_MAXIMUM_PREDICTED_CHANGE_CELSIUS,
                    BUILDING_HVAC_ML_MAXIMUM_PREDICTED_CHANGE_CELSIUS,
                )
            })
            && (self.temperature_change_15_minutes_celsius.is_some()
                || self.temperature_change_30_minutes_celsius.is_some()
                || self.temperature_change_60_minutes_celsius.is_some())
    }
}

/// Machine-learning validation metrics V1
/// Time-forward validation evidence used to decide whether a model may replace
/// the deterministic fallback or an older accepted model.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacMachineLearningValidationV1 {
    /// Training samples
    /// Number of older ordered samples used to fit the candidate.
    pub training_sample_count: u32,
    /// Validation samples
    /// Number of newest ordered samples held out from fitting.
    pub validation_sample_count: u32,
    /// Candidate RMSE
    /// Root mean square temperature-change error in degrees Celsius on the
    /// time-forward holdout.
    pub candidate_rmse_celsius: f32,
    /// Baseline RMSE
    /// Root mean square error of predicting no temperature change on the same
    /// holdout.
    pub deterministic_baseline_rmse_celsius: f32,
    /// Improvement
    /// Fractional RMSE reduction relative to the deterministic baseline.
    #[libertas_number(min = 0, max = 1)]
    pub improvement_normalized: f32,
}

impl BuildingHvacMachineLearningValidationV1 {
    /// Promotion quality
    /// Returns true only for finite metrics that meet the V1 sample and
    /// improvement requirements.
    pub fn permits_promotion(&self) -> bool {
        self.training_sample_count as usize + self.validation_sample_count as usize
            >= BUILDING_HVAC_ML_MINIMUM_TRAINING_SAMPLES
            && self.validation_sample_count as usize >= BUILDING_HVAC_ML_MINIMUM_VALIDATION_SAMPLES
            && self.candidate_rmse_celsius.is_finite()
            && self.candidate_rmse_celsius >= 0.0
            && self.deterministic_baseline_rmse_celsius.is_finite()
            && self.deterministic_baseline_rmse_celsius > 0.0
            && in_range(self.improvement_normalized, 0.0, 1.0)
            && self.improvement_normalized
                >= BUILDING_HVAC_ML_MINIMUM_PROMOTION_IMPROVEMENT_NORMALIZED
    }
}

/// Persisted XGBoost thermal model V1
/// Self-describing accepted model artifact. The UBJSON bytes are loaded only
/// after the manifest, validation evidence, feature order, and checksum pass.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacMachineLearningModelV1 {
    /// Room endpoint
    /// Stable room identity whose thermal behavior this model learned.
    pub room_endpoint: LibertasEndpoint,
    /// Prediction horizon
    /// Thermal target represented by this model.
    pub horizon: BuildingHvacThermalPredictionHorizonV1,
    /// Feature schema version
    /// Exact feature-vector contract used for training.
    pub feature_schema_version: u32,
    /// Ordered feature names
    /// Human- and AI-readable manifest matching both the XGBoost model metadata
    /// and numeric column order. Trees split on indexes after training.
    /// ----
    /// Feature name
    /// One stable feature identifier in model-column order.
    #[libertas_size(min = 1, max = 8192)]
    pub feature_names: Vec<String>,
    /// XGBoost version
    /// Bundled native XGBoost version that produced the UBJSON artifact.
    #[libertas_size(min = 1, max = 32)]
    pub xgboost_version: String,
    /// Trained at
    /// UTC timestamp when candidate fitting and validation completed.
    pub trained_at: LibertasDateTime,
    /// Training range start
    /// Earliest feature timestamp included in candidate fitting.
    pub training_range_starts_at: LibertasDateTime,
    /// Training range end
    /// Latest feature timestamp included in candidate fitting.
    pub training_range_ends_at: LibertasDateTime,
    /// Boost rounds
    /// Number of shallow trees fitted by the bounded V1 training policy.
    pub boost_rounds: u32,
    /// Maximum tree depth
    /// Maximum depth allowed by the bounded V1 training policy.
    pub maximum_tree_depth: u32,
    /// Learning rate
    /// XGBoost shrinkage used during fitting.
    #[libertas_number(min = 0, max = 1)]
    pub learning_rate: f32,
    /// Validation
    /// Time-forward promotion evidence for this artifact.
    pub validation: BuildingHvacMachineLearningValidationV1,
    /// Model SHA-256
    /// Thirty-two checksum bytes over `model_ubjson`, checked before loading.
    #[libertas_size(min = 32, max = 32)]
    pub model_sha256: Vec<u8>,
    /// Model UBJSON
    /// Bounded XGBoost model bytes. They are application data, not executable
    /// code, and are never interpreted until the manifest and checksum pass.
    #[libertas_size(min = 1, max = 16777216)]
    pub model_ubjson: Vec<u8>,
}

impl BuildingHvacMachineLearningModelV1 {
    /// Valid persisted model
    /// Validates the complete self-description and model checksum without
    /// calling XGBoost.
    pub fn is_well_formed(&self) -> bool {
        self.feature_schema_version == BUILDING_HVAC_ML_FEATURE_SCHEMA_VERSION
            && feature_manifest_is_well_formed(&self.feature_names)
            && self.xgboost_version == BUILDING_HVAC_XGBOOST_VERSION
            && self.training_range_starts_at <= self.training_range_ends_at
            && self.training_range_ends_at <= self.trained_at
            && self.boost_rounds == BUILDING_HVAC_ML_BOOST_ROUNDS
            && self.maximum_tree_depth == BUILDING_HVAC_ML_MAXIMUM_TREE_DEPTH
            && self.learning_rate == 0.05
            && self.validation.permits_promotion()
            && !self.model_ubjson.is_empty()
            && self.model_ubjson.len() <= BUILDING_HVAC_ML_MAXIMUM_MODEL_BYTES
            && self.model_sha256.as_slice() == model_sha256(&self.model_ubjson)
    }
}

/// Persisted model slot V1
/// Keeps one active model and one immediate rollback artifact for a prediction
/// horizon. The previous model is never used unless explicitly restored.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacMachineLearningModelSlotV1 {
    /// Prediction horizon
    /// Unique model target represented by this slot.
    pub horizon: BuildingHvacThermalPredictionHorizonV1,
    /// Active model
    /// Model permitted to produce bounded supervisory predictions.
    pub active_model: BuildingHvacMachineLearningModelV1,
    /// Previous model
    /// Last superseded accepted model retained for rollback.
    pub previous_model: Option<BuildingHvacMachineLearningModelV1>,
}

/// Persisted model set V1
/// Complete bounded set of independently promoted thermal models.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacMachineLearningModelSetV1 {
    /// Room endpoint
    /// Stable room identity used as the persistent database key.
    pub room_endpoint: LibertasEndpoint,
    /// Model slots
    /// At most one active and one rollback model for each V1 horizon.
    /// ----
    /// Model slot
    /// One independently validated thermal horizon.
    #[libertas_size(max = 3)]
    pub models: Vec<BuildingHvacMachineLearningModelSlotV1>,
}

impl BuildingHvacMachineLearningModelSetV1 {
    /// Empty room model set
    /// Creates the deterministic startup state for a room without an accepted
    /// model.
    pub const fn empty(room_endpoint: LibertasEndpoint) -> Self {
        Self {
            room_endpoint,
            models: Vec::new(),
        }
    }

    /// Valid model set
    /// Rejects duplicate targets and any malformed active or rollback artifact.
    pub fn is_well_formed(&self) -> bool {
        self.models.len() <= 3
            && self.models.iter().enumerate().all(|(index, slot)| {
                slot.horizon == slot.active_model.horizon
                    && slot.active_model.room_endpoint == self.room_endpoint
                    && slot.active_model.is_well_formed()
                    && slot.previous_model.as_ref().is_none_or(|previous| {
                        previous.room_endpoint == self.room_endpoint
                            && previous.horizon == slot.horizon
                            && previous.is_well_formed()
                    })
                    && !self.models[..index]
                        .iter()
                        .any(|other| other.horizon == slot.horizon)
            })
    }

    /// Promote candidate
    /// Replaces one active model and retains the old active model for rollback.
    pub fn promote(&mut self, candidate: BuildingHvacMachineLearningModelV1) -> bool {
        if candidate.room_endpoint != self.room_endpoint || !candidate.is_well_formed() {
            return false;
        }
        if let Some(slot) = self
            .models
            .iter_mut()
            .find(|slot| slot.horizon == candidate.horizon)
        {
            slot.previous_model = Some(std::mem::replace(&mut slot.active_model, candidate));
            true
        } else if self.models.len() < 3 {
            self.models.push(BuildingHvacMachineLearningModelSlotV1 {
                horizon: candidate.horizon,
                active_model: candidate,
                previous_model: None,
            });
            true
        } else {
            false
        }
    }

    /// Active models
    /// Returns the accepted artifacts to restore on the worker thread.
    pub fn active_models(&self) -> impl Iterator<Item = &BuildingHvacMachineLearningModelV1> {
        self.models.iter().map(|slot| &slot.active_model)
    }
}

/// Thermal prediction source V1
/// Makes deterministic fallback explicit rather than presenting it as a
/// learned result.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacThermalPredictionSourceV1 {
    /// XGBoost
    /// Prediction came from the currently accepted bounded model.
    Xgboost,
    /// Deterministic fallback
    /// No validated model or usable output existed, so the controller assumes
    /// no additional near-term temperature movement.
    DeterministicFallback,
}

/// Room thermal prediction V1
/// One bounded read-only near-term prediction used as an input to deterministic
/// planning and shared-thermostat arbitration.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacThermalPredictionV1 {
    /// Prediction horizon
    /// Future interval represented by the temperature change.
    #[libertas_read_only]
    pub horizon: BuildingHvacThermalPredictionHorizonV1,
    /// Predicted temperature change
    /// Degrees Celsius relative to current room temperature. Positive values
    /// predict warming and negative values predict cooling.
    #[libertas_read_only]
    pub temperature_change_celsius: f32,
    /// Prediction source
    /// Whether the value came from XGBoost or deterministic fallback.
    #[libertas_read_only]
    pub source: BuildingHvacThermalPredictionSourceV1,
    /// Model trained at
    /// Training timestamp of the active model, absent for fallback.
    #[libertas_read_only]
    pub model_trained_at: Option<LibertasDateTime>,
}

/// Room machine-learning state V1
/// Read-only model state and latest predictions exposed inside `RoomDataV1`.
#[derive(
    Clone, Debug, Default, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub struct BuildingHvacRoomMachineLearningV1 {
    /// Latest predictions
    /// At most one current prediction for each supported horizon.
    /// ----
    /// Thermal prediction
    /// One bounded XGBoost result or explicit deterministic fallback.
    #[libertas_size(max = 3)]
    #[libertas_read_only]
    pub predictions: Vec<BuildingHvacThermalPredictionV1>,
}

/// Machine-learning training rejection
/// Explains why a candidate was not fitted or promoted. These implementation
/// results are logged; they are not urgent user notifications.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildingHvacMachineLearningRejection {
    /// Too few samples
    TooFewSamples,
    /// Invalid samples
    InvalidSamples,
    /// XGBoost failure
    Xgboost(String),
    /// Candidate did not improve enough
    InsufficientImprovement,
    /// Candidate artifact failed validation
    InvalidArtifact,
}

/// Machine-learning worker result
/// Owned result drained by the Libertas wake-up callback without blocking.
#[derive(Clone, Debug, PartialEq)]
pub enum BuildingHvacMachineLearningResult {
    /// Candidate model
    /// Must be persisted by the Libertas thread before it is sent back to the
    /// worker for activation.
    Candidate(BuildingHvacMachineLearningModelV1),
    /// Training rejected
    TrainingRejected {
        /// Prediction horizon
        horizon: BuildingHvacThermalPredictionHorizonV1,
        /// Rejection reason
        reason: BuildingHvacMachineLearningRejection,
    },
    /// Prediction
    /// Correlates a bounded result to the caller's transient request ID.
    Prediction {
        /// Request identifier
        request_id: u64,
        /// Room endpoint
        room_endpoint: LibertasEndpoint,
        /// Prediction
        prediction: BuildingHvacThermalPredictionV1,
    },
}

enum BuildingHvacMachineLearningCommand {
    TrainAll {
        trained_at: LibertasDateTime,
        feature_names: Vec<String>,
        samples: Vec<BuildingHvacMachineLearningSampleV1>,
    },
    Activate {
        model: BuildingHvacMachineLearningModelV1,
    },
    Predict {
        request_id: u64,
        room_endpoint: LibertasEndpoint,
        horizon: BuildingHvacThermalPredictionHorizonV1,
        features: BuildingHvacMachineLearningFeaturesV1,
    },
    Shutdown,
}

/// Machine-learning queue error
/// A full queue is recoverable backpressure; a disconnected queue means the
/// worker is no longer available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildingHvacMachineLearningQueueError {
    /// Queue full
    Full,
    /// Worker disconnected
    Disconnected,
}

/// Machine-learning history error
/// Explains why an indexed thermal sample could not safely replace or extend a
/// room observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildingHvacMachineLearningHistoryError {
    /// Invalid sample
    /// A feature, target, room identity, or timestamp violates the V1 contract.
    InvalidSample,
    /// Conflicting observation
    /// A record already exists at this room timestamp with different features
    /// or a different previously stored target.
    ConflictingObservation,
}

/// Machine-learning indexed history
/// Libertas-thread helpers for merging labeled samples and loading an ordered
/// bounded training window. Values are always persisted through
/// `BuildingHvacPersistentDataV1::MachineLearningSampleV1`.
pub struct BuildingHvacMachineLearningHistory;

impl BuildingHvacMachineLearningHistory {
    /// Persist sample
    /// Merges newly available horizon labels without erasing labels already
    /// stored at the same timestamp. The call submits the indexed database
    /// write but the current Libertas API does not confirm completed durability.
    pub fn persist_sample(
        now_utc: LibertasDateTime,
        sample: BuildingHvacMachineLearningSampleV1,
    ) -> Result<(), BuildingHvacMachineLearningHistoryError> {
        if !sample.is_well_formed()
            || sample.observed_at > now_utc
            || sample.observed_at
                < now_utc.saturating_sub(BUILDING_HVAC_ML_HISTORY_RETENTION_SECONDS)
        {
            return Err(BuildingHvacMachineLearningHistoryError::InvalidSample);
        }
        let index = i64::try_from(sample.observed_at)
            .map_err(|_| BuildingHvacMachineLearningHistoryError::InvalidSample)?;
        let key = [NotificationArgument::Object(sample.room_endpoint)];
        let database = libertas_data_open_indexed(BUILDING_HVAC_ML_SAMPLE_RESOURCE, &key);
        let merged = if let Some(existing) = libertas_data_read_indexed::<
            crate::BuildingHvacPersistentDataV1,
        >(database.handle, index)
            && existing.index == index
        {
            match existing.data {
                crate::BuildingHvacPersistentDataV1::MachineLearningSampleV1 {
                    sample: existing,
                } if existing.observed_at == sample.observed_at
                    && existing.room_endpoint == sample.room_endpoint
                    && existing.features == sample.features =>
                {
                    merge_sample_targets(existing, sample)?
                }
                _ => {
                    return Err(BuildingHvacMachineLearningHistoryError::ConflictingObservation);
                }
            }
        } else {
            sample
        };
        let value = crate::BuildingHvacPersistentDataV1::MachineLearningSampleV1 { sample: merged };
        libertas_data_write_indexed(database.handle, index, &value);

        let oldest_retained = now_utc.saturating_sub(BUILDING_HVAC_ML_HISTORY_RETENTION_SECONDS);
        if let Ok(oldest_retained) = i64::try_from(oldest_retained)
            && oldest_retained > i64::MIN
        {
            libertas_data_remove_indexed_records(
                database.handle,
                i64::MIN,
                oldest_retained.saturating_sub(1),
            );
        }
        Ok(())
    }

    /// Load training samples
    /// Reads the retained 400-day room archive through `through_utc`, rejects
    /// mismatched or invalid records, and selects a bounded training set.
    /// Selection reserves recent evidence for adaptation and distributes older
    /// evidence across annual phase, outdoor weather, and signed HVAC demand
    /// strata. The result remains in ascending observation order so the newest
    /// portion can be used for time-forward validation.
    pub fn load_training_samples(
        room_endpoint: LibertasEndpoint,
        through_utc: LibertasDateTime,
        expected_feature_names: &[String],
    ) -> Vec<BuildingHvacMachineLearningSampleV1> {
        let Ok(index) = i64::try_from(through_utc) else {
            return Vec::new();
        };
        if !feature_manifest_is_well_formed(expected_feature_names) {
            return Vec::new();
        }
        let key = [NotificationArgument::Object(room_endpoint)];
        let database = libertas_data_open_indexed(BUILDING_HVAC_ML_SAMPLE_RESOURCE, &key);
        let mut records = Vec::new();
        libertas_data_read_indexed_range::<crate::BuildingHvacPersistentDataV1>(
            database.handle,
            index,
            IndexDirection::Below,
            BUILDING_HVAC_ML_MAXIMUM_RETAINED_SAMPLES_PER_ROOM,
            &mut records,
        );
        let mut samples: Vec<_> = records
            .into_iter()
            .filter_map(|record| match record.data {
                crate::BuildingHvacPersistentDataV1::MachineLearningSampleV1 { sample }
                    if i64::try_from(sample.observed_at) == Ok(record.index)
                        && sample.room_endpoint == room_endpoint
                        && sample.observed_at <= through_utc
                        && sample.is_well_formed()
                        && sample
                            .features
                            .matches_feature_names(expected_feature_names) =>
                {
                    Some(sample)
                }
                _ => None,
            })
            .collect();
        samples.sort_by_key(|sample| sample.observed_at);
        samples.dedup_by_key(|sample| sample.observed_at);
        select_stratified_training_samples(&samples, through_utc, expected_feature_names)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TrainingStratum {
    annual_phase: u8,
    outdoor_temperature: u8,
    outdoor_humidity: u8,
    outdoor_wind: u8,
    solar_irradiance: u8,
    heating_demand_thermostats: u8,
    cooling_demand_thermostats: u8,
}

fn select_stratified_training_samples(
    samples: &[BuildingHvacMachineLearningSampleV1],
    through_utc: LibertasDateTime,
    feature_names: &[String],
) -> Vec<BuildingHvacMachineLearningSampleV1> {
    let maximum = BUILDING_HVAC_ML_MAXIMUM_TRAINING_SAMPLES_PER_ROOM.min(samples.len());
    if samples.len() <= maximum {
        return samples.to_vec();
    }

    let recent_starts_at = through_utc.saturating_sub(BUILDING_HVAC_ML_RECENT_WINDOW_SECONDS);
    let recent_start = samples.partition_point(|sample| sample.observed_at < recent_starts_at);
    let recent_available = samples.len().saturating_sub(recent_start);
    let older_available = recent_start;

    let desired_recent = maximum.saturating_mul(BUILDING_HVAC_ML_RECENT_TRAINING_PERCENT) / 100;
    let mut recent_target = recent_available.min(desired_recent);
    let mut older_target = older_available.min(maximum.saturating_sub(recent_target));
    let mut unallocated = maximum.saturating_sub(recent_target + older_target);

    let additional_recent = unallocated.min(recent_available.saturating_sub(recent_target));
    recent_target += additional_recent;
    unallocated -= additional_recent;
    older_target += unallocated.min(older_available.saturating_sub(older_target));

    let mut selected = vec![false; samples.len()];
    for index in select_stratified_indices(&samples[..recent_start], older_target, feature_names) {
        selected[index] = true;
    }
    for index in select_stratified_indices(&samples[recent_start..], recent_target, feature_names) {
        selected[recent_start + index] = true;
    }

    if recent_target > 0 {
        let newest = samples.len() - 1;
        if !selected[newest] {
            if let Some(replaced) = (recent_start..newest).find(|index| selected[*index]) {
                selected[replaced] = false;
            }
            selected[newest] = true;
        }
    }

    let selected_count = selected.iter().filter(|selected| **selected).count();
    if selected_count < maximum {
        let available: Vec<_> = selected
            .iter()
            .enumerate()
            .filter_map(|(index, selected)| (!*selected).then_some(index))
            .collect();
        for index in evenly_spaced_values(&available, maximum - selected_count) {
            selected[index] = true;
        }
    }

    samples
        .iter()
        .zip(selected)
        .filter(|(_, selected)| *selected)
        .map(|(sample, _)| sample.clone())
        .collect()
}

fn select_stratified_indices(
    samples: &[BuildingHvacMachineLearningSampleV1],
    target: usize,
    feature_names: &[String],
) -> Vec<usize> {
    if target == 0 || samples.is_empty() {
        return Vec::new();
    }
    if target >= samples.len() {
        return (0..samples.len()).collect();
    }

    let mut grouped = BTreeMap::<TrainingStratum, Vec<usize>>::new();
    for (index, sample) in samples.iter().enumerate() {
        grouped
            .entry(training_stratum(&sample.features, feature_names))
            .or_default()
            .push(index);
    }
    let buckets: Vec<_> = grouped.into_values().collect();
    if target < buckets.len() {
        let bucket_positions: Vec<_> = (0..buckets.len()).collect();
        return evenly_spaced_values(&bucket_positions, target)
            .into_iter()
            .map(|bucket| buckets[bucket][buckets[bucket].len() / 2])
            .collect();
    }

    let mut quotas = vec![0_usize; buckets.len()];
    let mut remaining = target;
    while remaining > 0 {
        let mut allocated = false;
        for (quota, bucket) in quotas.iter_mut().zip(&buckets) {
            if *quota < bucket.len() {
                *quota += 1;
                remaining -= 1;
                allocated = true;
                if remaining == 0 {
                    break;
                }
            }
        }
        if !allocated {
            break;
        }
    }

    let mut selected = Vec::with_capacity(target);
    for (bucket, quota) in buckets.iter().zip(quotas) {
        selected.extend(evenly_spaced_values(bucket, quota));
    }
    selected.sort_unstable();
    selected
}

fn evenly_spaced_values(values: &[usize], target: usize) -> Vec<usize> {
    let target = target.min(values.len());
    match target {
        0 => Vec::new(),
        1 => vec![values[values.len() / 2]],
        target if target == values.len() => values.to_vec(),
        target => (0..target)
            .map(|position| {
                let source = position.saturating_mul(values.len() - 1) / (target - 1);
                values[source]
            })
            .collect(),
    }
}

fn training_stratum(
    features: &BuildingHvacMachineLearningFeatureVectorV1,
    feature_names: &[String],
) -> TrainingStratum {
    let annual_sine = features
        .value(feature_names, "time.day_of_year_sine")
        .unwrap_or(0.0);
    let annual_cosine = features
        .value(feature_names, "time.day_of_year_cosine")
        .unwrap_or(1.0);
    TrainingStratum {
        annual_phase: annual_phase_bin(annual_sine, annual_cosine),
        outdoor_temperature: match features.value(
            feature_names,
            "weather.current.dry_bulb_temperature_celsius",
        ) {
            None => 0,
            Some(value) if value < -10.0 => 1,
            Some(value) if value < 0.0 => 2,
            Some(value) if value < 10.0 => 3,
            Some(value) if value < 20.0 => 4,
            Some(value) if value < 30.0 => 5,
            Some(value) if value < 35.0 => 6,
            Some(_) => 7,
        },
        outdoor_humidity: match features
            .value(feature_names, "weather.current.humidity_ratio_kg_per_kg")
        {
            None => 0,
            Some(value) if value < 0.004 => 1,
            Some(value) if value < 0.008 => 2,
            Some(value) if value < 0.012 => 3,
            Some(value) if value < 0.018 => 4,
            Some(_) => 5,
        },
        outdoor_wind: match features.value(
            feature_names,
            "weather.current.wind_speed_meters_per_second",
        ) {
            None => 0,
            Some(value) if value < 2.0 => 1,
            Some(value) if value < 6.0 => 2,
            Some(value) if value < 12.0 => 3,
            Some(_) => 4,
        },
        solar_irradiance: match features.value(
            feature_names,
            "weather.current.global_horizontal_irradiance_watts_per_square_meter",
        ) {
            None => 0,
            Some(value) if value <= 1.0 => 1,
            Some(value) if value < 200.0 => 2,
            Some(value) if value < 600.0 => 3,
            Some(_) => 4,
        },
        heating_demand_thermostats: u8::try_from(
            feature_names
                .iter()
                .enumerate()
                .filter(|(index, name)| {
                    name.starts_with("thermostat.")
                        && name.ends_with(".active_setpoint_delta_celsius")
                        && features
                            .values
                            .binary_search_by_key(
                                &u16::try_from(*index).unwrap_or(u16::MAX),
                                |feature| feature.index,
                            )
                            .ok()
                            .is_some_and(|position| features.values[position].value > 0.05)
                })
                .count(),
        )
        .unwrap_or(u8::MAX),
        cooling_demand_thermostats: u8::try_from(
            feature_names
                .iter()
                .enumerate()
                .filter(|(index, name)| {
                    name.starts_with("thermostat.")
                        && name.ends_with(".active_setpoint_delta_celsius")
                        && features
                            .values
                            .binary_search_by_key(
                                &u16::try_from(*index).unwrap_or(u16::MAX),
                                |feature| feature.index,
                            )
                            .ok()
                            .is_some_and(|position| features.values[position].value < -0.05)
                })
                .count(),
        )
        .unwrap_or(u8::MAX),
    }
}

fn annual_phase_bin(sine: f32, cosine: f32) -> u8 {
    let phase = sine.atan2(cosine).rem_euclid(std::f32::consts::TAU);
    ((phase / std::f32::consts::TAU * 8.0) as u8).min(7)
}

/// Machine-learning worker client
/// Cloneable bounded sender used by Libertas callbacks. Every operation is
/// nonblocking and transfers owned data to the single XGBoost thread.
#[derive(Clone)]
pub struct BuildingHvacMachineLearningClient {
    commands: SyncSender<BuildingHvacMachineLearningCommand>,
    stop_requested: Arc<AtomicBool>,
    training_pending: Arc<AtomicBool>,
}

impl BuildingHvacMachineLearningClient {
    /// Try training all horizons
    /// Transfers one bounded sample set to the worker, which fits the three
    /// horizons sequentially. Reusing one sparse sample allocation prevents
    /// three queued copies of the selected 400-day evidence.
    pub fn try_train_all(
        &self,
        trained_at: LibertasDateTime,
        feature_names: Vec<String>,
        samples: Vec<BuildingHvacMachineLearningSampleV1>,
    ) -> Result<(), BuildingHvacMachineLearningQueueError> {
        if self
            .training_pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(BuildingHvacMachineLearningQueueError::Full);
        }
        let result = self.try_send(BuildingHvacMachineLearningCommand::TrainAll {
            trained_at,
            feature_names,
            samples,
        });
        if result.is_err() {
            self.training_pending.store(false, Ordering::Release);
        }
        result
    }

    /// Training pending
    /// Reports whether a room training cycle is queued or executing so the
    /// Libertas thread does not allocate another maximum-size sample set.
    pub fn training_pending(&self) -> bool {
        self.training_pending.load(Ordering::Acquire)
    }

    /// Try activating
    /// Enqueues a model only after the caller has persisted its accepted model
    /// set.
    pub fn try_activate(
        &self,
        model: BuildingHvacMachineLearningModelV1,
    ) -> Result<(), BuildingHvacMachineLearningQueueError> {
        self.try_send(BuildingHvacMachineLearningCommand::Activate { model })
    }

    /// Try predicting
    /// Enqueues one bounded inference request. Missing or invalid model output
    /// returns an explicit deterministic fallback result.
    pub fn try_predict(
        &self,
        request_id: u64,
        room_endpoint: LibertasEndpoint,
        horizon: BuildingHvacThermalPredictionHorizonV1,
        features: BuildingHvacMachineLearningFeaturesV1,
    ) -> Result<(), BuildingHvacMachineLearningQueueError> {
        self.try_send(BuildingHvacMachineLearningCommand::Predict {
            request_id,
            room_endpoint,
            horizon,
            features,
        })
    }

    /// Request shutdown
    /// Sets the stop flag first and then wakes a worker blocked on its command
    /// receiver. The Libertas shutdown callback never waits for the worker.
    pub fn request_shutdown(&self) -> Result<(), BuildingHvacMachineLearningQueueError> {
        self.stop_requested.store(true, Ordering::Release);
        self.try_send(BuildingHvacMachineLearningCommand::Shutdown)
    }

    fn try_send(
        &self,
        command: BuildingHvacMachineLearningCommand,
    ) -> Result<(), BuildingHvacMachineLearningQueueError> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                TrySendError::Full(_) => BuildingHvacMachineLearningQueueError::Full,
                TrySendError::Disconnected(_) => {
                    BuildingHvacMachineLearningQueueError::Disconnected
                }
            })
    }
}

/// Machine-learning engine
/// Synchronous fitting and prediction primitives used only on the dedicated
/// worker thread and in tests.
pub struct BuildingHvacMachineLearningEngine;

impl BuildingHvacMachineLearningEngine {
    /// Train candidate
    /// Fits a shallow CPU histogram model on older samples and validates it
    /// against the newest time-ordered holdout.
    #[cfg(target_os = "linux")]
    pub fn train_candidate(
        horizon: BuildingHvacThermalPredictionHorizonV1,
        trained_at: LibertasDateTime,
        feature_names: &[String],
        samples: &[BuildingHvacMachineLearningSampleV1],
    ) -> Result<BuildingHvacMachineLearningModelV1, BuildingHvacMachineLearningRejection> {
        let labeled = validate_and_collect_samples(horizon, feature_names, samples)?;
        let validation_count = (labeled.len() * BUILDING_HVAC_ML_VALIDATION_PERCENT / 100)
            .max(BUILDING_HVAC_ML_MINIMUM_VALIDATION_SAMPLES)
            .min(labeled.len().saturating_sub(1));
        let training_count = labeled.len().saturating_sub(validation_count);
        if training_count == 0 || validation_count < BUILDING_HVAC_ML_MINIMUM_VALIDATION_SAMPLES {
            return Err(BuildingHvacMachineLearningRejection::TooFewSamples);
        }

        let (training, validation) = labeled.split_at(training_count);
        let booster = train_booster(training, feature_names)?;
        let validation_predictions = predict_feature_vectors(
            &booster,
            &validation
                .iter()
                .map(|(sample, _)| sample.features.clone())
                .collect::<Vec<_>>(),
        )?;
        let labels: Vec<f32> = validation.iter().map(|(_, label)| *label).collect();
        let candidate_rmse = rmse(&validation_predictions, &labels)
            .ok_or(BuildingHvacMachineLearningRejection::InvalidSamples)?;
        let baseline_predictions = vec![0.0; labels.len()];
        let baseline_rmse = rmse(&baseline_predictions, &labels)
            .ok_or(BuildingHvacMachineLearningRejection::InvalidSamples)?;
        if baseline_rmse <= f32::EPSILON {
            return Err(BuildingHvacMachineLearningRejection::InsufficientImprovement);
        }
        let improvement = ((baseline_rmse - candidate_rmse) / baseline_rmse).clamp(0.0, 1.0);
        let validation = BuildingHvacMachineLearningValidationV1 {
            training_sample_count: u32::try_from(training_count).unwrap_or(u32::MAX),
            validation_sample_count: u32::try_from(validation_count).unwrap_or(u32::MAX),
            candidate_rmse_celsius: candidate_rmse,
            deterministic_baseline_rmse_celsius: baseline_rmse,
            improvement_normalized: improvement,
        };
        if !validation.permits_promotion() {
            return Err(BuildingHvacMachineLearningRejection::InsufficientImprovement);
        }

        let model_ubjson = booster.save_buffer(true).map_err(xgboost_rejection)?;
        if model_ubjson.is_empty() || model_ubjson.len() > BUILDING_HVAC_ML_MAXIMUM_MODEL_BYTES {
            return Err(BuildingHvacMachineLearningRejection::InvalidArtifact);
        }
        let persisted_booster = Booster::load_buffer(&model_ubjson).map_err(xgboost_rejection)?;
        let persisted_feature_names = persisted_booster
            .get_feature_names()
            .map_err(xgboost_rejection)?;
        if persisted_feature_names.as_slice() != feature_names {
            return Err(BuildingHvacMachineLearningRejection::InvalidArtifact);
        }
        let candidate = BuildingHvacMachineLearningModelV1 {
            room_endpoint: training[0].0.room_endpoint,
            horizon,
            feature_schema_version: BUILDING_HVAC_ML_FEATURE_SCHEMA_VERSION,
            feature_names: feature_names.to_vec(),
            xgboost_version: String::from(BUILDING_HVAC_XGBOOST_VERSION),
            trained_at,
            training_range_starts_at: training[0].0.observed_at,
            training_range_ends_at: training[training.len() - 1].0.observed_at,
            boost_rounds: BUILDING_HVAC_ML_BOOST_ROUNDS,
            maximum_tree_depth: BUILDING_HVAC_ML_MAXIMUM_TREE_DEPTH,
            learning_rate: 0.05,
            validation,
            model_sha256: model_sha256(&model_ubjson).to_vec(),
            model_ubjson,
        };
        if candidate.is_well_formed() {
            Ok(candidate)
        } else {
            Err(BuildingHvacMachineLearningRejection::InvalidArtifact)
        }
    }

    /// Train candidate
    /// Non-Linux development builds preserve schemas and deterministic fallback
    /// but do not carry the Hub's statically linked native backend.
    #[cfg(not(target_os = "linux"))]
    pub fn train_candidate(
        _horizon: BuildingHvacThermalPredictionHorizonV1,
        _trained_at: LibertasDateTime,
        _feature_names: &[String],
        _samples: &[BuildingHvacMachineLearningSampleV1],
    ) -> Result<BuildingHvacMachineLearningModelV1, BuildingHvacMachineLearningRejection> {
        Err(BuildingHvacMachineLearningRejection::Xgboost(String::from(
            "the static XGBoost backend is available on the Linux Hub target",
        )))
    }

    /// Predict from persisted model
    /// Validates and loads a persisted artifact before producing one bounded
    /// temperature change.
    #[cfg(target_os = "linux")]
    pub fn predict(
        model: &BuildingHvacMachineLearningModelV1,
        features: BuildingHvacMachineLearningFeaturesV1,
    ) -> Result<f32, BuildingHvacMachineLearningRejection> {
        if !model.is_well_formed()
            || !features.is_well_formed()
            || features.feature_names() != model.feature_names
        {
            return Err(BuildingHvacMachineLearningRejection::InvalidArtifact);
        }
        let booster = Booster::load_buffer(&model.model_ubjson).map_err(xgboost_rejection)?;
        if booster.get_feature_names().map_err(xgboost_rejection)? != model.feature_names {
            return Err(BuildingHvacMachineLearningRejection::InvalidArtifact);
        }
        let prediction = predict_booster(&booster, &[features])?
            .into_iter()
            .next()
            .ok_or(BuildingHvacMachineLearningRejection::InvalidArtifact)?;
        bounded_prediction(prediction).ok_or(BuildingHvacMachineLearningRejection::InvalidArtifact)
    }

    /// Predict from persisted model
    /// Non-Linux development builds deliberately return backend-unavailable so
    /// callers select deterministic fallback.
    #[cfg(not(target_os = "linux"))]
    pub fn predict(
        _model: &BuildingHvacMachineLearningModelV1,
        _features: BuildingHvacMachineLearningFeaturesV1,
    ) -> Result<f32, BuildingHvacMachineLearningRejection> {
        Err(BuildingHvacMachineLearningRejection::Xgboost(String::from(
            "the static XGBoost backend is available on the Linux Hub target",
        )))
    }
}

#[cfg(target_os = "linux")]
struct ActiveBooster {
    model: BuildingHvacMachineLearningModelV1,
    booster: Booster,
}

#[cfg(not(target_os = "linux"))]
struct ActiveBooster {
    model: BuildingHvacMachineLearningModelV1,
}

/// Start machine-learning worker
/// Creates the bounded single-owner XGBoost thread. `wake_main` is called only
/// after a result is accepted by the result queue. `shutdown_complete` is the
/// worker's final action after a requested shutdown.
pub(crate) fn start_machine_learning_worker(
    wake_main: fn(),
    shutdown_complete: fn(),
) -> Result<
    (
        BuildingHvacMachineLearningClient,
        Receiver<BuildingHvacMachineLearningResult>,
    ),
    String,
> {
    let (command_sender, command_receiver) = sync_channel(BUILDING_HVAC_ML_COMMAND_CAPACITY);
    let (result_sender, result_receiver) = sync_channel(BUILDING_HVAC_ML_RESULT_CAPACITY);
    let (startup_sender, startup_receiver) = sync_channel(1);
    let stop_requested = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop_requested);
    let training_pending = Arc::new(AtomicBool::new(false));
    let worker_training_pending = Arc::clone(&training_pending);
    thread::Builder::new()
        .name(String::from("libertas-hvac-xgboost"))
        .spawn(move || {
            let priority = configure_machine_learning_worker_priority();
            let ready = priority.is_ok();
            if startup_sender.send(priority).is_err() || !ready {
                return;
            }
            machine_learning_worker(
                command_receiver,
                result_sender,
                worker_stop,
                worker_training_pending,
                wake_main,
                shutdown_complete,
            );
        })
        .map_err(|error| format!("failed to start XGBoost worker: {error}"))?;
    startup_receiver
        .recv()
        .map_err(|_| String::from("XGBoost worker stopped before reporting its CPU priority"))??;
    Ok((
        BuildingHvacMachineLearningClient {
            commands: command_sender,
            stop_requested,
            training_pending,
        },
        result_receiver,
    ))
}

#[cfg(target_os = "linux")]
fn configure_machine_learning_worker_priority() -> Result<i32, String> {
    rustix::process::nice(BUILDING_HVAC_ML_WORKER_NICE_INCREMENT)
        .map_err(|error| format!("failed to lower XGBoost worker CPU priority: {error}"))
}

#[cfg(not(target_os = "linux"))]
fn configure_machine_learning_worker_priority() -> Result<i32, String> {
    Ok(0)
}

fn machine_learning_worker(
    commands: Receiver<BuildingHvacMachineLearningCommand>,
    results: SyncSender<BuildingHvacMachineLearningResult>,
    stop_requested: Arc<AtomicBool>,
    training_pending: Arc<AtomicBool>,
    wake_main: fn(),
    shutdown_complete: fn(),
) {
    let mut active_models: Vec<ActiveBooster> = Vec::new();
    while let Ok(command) = commands.recv() {
        if stop_requested.load(Ordering::Acquire)
            || matches!(command, BuildingHvacMachineLearningCommand::Shutdown)
        {
            training_pending.store(false, Ordering::Release);
            shutdown_complete();
            return;
        }
        match command {
            BuildingHvacMachineLearningCommand::TrainAll {
                trained_at,
                feature_names,
                samples,
            } => {
                for horizon in [
                    BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
                    BuildingHvacThermalPredictionHorizonV1::ThirtyMinutes,
                    BuildingHvacThermalPredictionHorizonV1::SixtyMinutes,
                ] {
                    if stop_requested.load(Ordering::Acquire) {
                        training_pending.store(false, Ordering::Release);
                        shutdown_complete();
                        return;
                    }
                    let result = match BuildingHvacMachineLearningEngine::train_candidate(
                        horizon,
                        trained_at,
                        &feature_names,
                        &samples,
                    ) {
                        Ok(candidate) => BuildingHvacMachineLearningResult::Candidate(candidate),
                        Err(reason) => {
                            BuildingHvacMachineLearningResult::TrainingRejected { horizon, reason }
                        }
                    };
                    send_worker_result(&results, result, wake_main);
                }
                training_pending.store(false, Ordering::Release);
            }
            BuildingHvacMachineLearningCommand::Activate { model } => {
                let horizon = model.horizon;
                match load_active_booster(model) {
                    Ok(booster) => {
                        if let Some(existing) = active_models.iter_mut().find(|existing| {
                            existing.model.room_endpoint == booster.model.room_endpoint
                                && existing.model.horizon == booster.model.horizon
                        }) {
                            *existing = booster;
                        } else if active_models.len() < 192 {
                            active_models.push(booster);
                        }
                    }
                    Err(reason) => send_worker_result(
                        &results,
                        BuildingHvacMachineLearningResult::TrainingRejected { horizon, reason },
                        wake_main,
                    ),
                }
            }
            BuildingHvacMachineLearningCommand::Predict {
                request_id,
                room_endpoint,
                horizon,
                features,
            } => {
                let prediction = active_models
                    .iter()
                    .find(|active| {
                        active.model.room_endpoint == room_endpoint
                            && active.model.horizon == horizon
                    })
                    .and_then(|active| predict_active_booster(active, features))
                    .unwrap_or(BuildingHvacThermalPredictionV1 {
                        horizon,
                        temperature_change_celsius: 0.0,
                        source: BuildingHvacThermalPredictionSourceV1::DeterministicFallback,
                        model_trained_at: None,
                    });
                send_worker_result(
                    &results,
                    BuildingHvacMachineLearningResult::Prediction {
                        request_id,
                        room_endpoint,
                        prediction,
                    },
                    wake_main,
                );
            }
            BuildingHvacMachineLearningCommand::Shutdown => unreachable!(),
        }
    }
    training_pending.store(false, Ordering::Release);
    if stop_requested.load(Ordering::Acquire) {
        shutdown_complete();
    }
}

fn send_worker_result(
    results: &SyncSender<BuildingHvacMachineLearningResult>,
    result: BuildingHvacMachineLearningResult,
    wake_main: fn(),
) {
    if results.try_send(result).is_ok() {
        wake_main();
    }
}

#[cfg(target_os = "linux")]
fn load_active_booster(
    model: BuildingHvacMachineLearningModelV1,
) -> Result<ActiveBooster, BuildingHvacMachineLearningRejection> {
    if !model.is_well_formed() {
        return Err(BuildingHvacMachineLearningRejection::InvalidArtifact);
    }
    let booster = Booster::load_buffer(&model.model_ubjson).map_err(xgboost_rejection)?;
    if booster.get_feature_names().map_err(xgboost_rejection)? != model.feature_names {
        return Err(BuildingHvacMachineLearningRejection::InvalidArtifact);
    }
    Ok(ActiveBooster { model, booster })
}

#[cfg(not(target_os = "linux"))]
fn load_active_booster(
    _model: BuildingHvacMachineLearningModelV1,
) -> Result<ActiveBooster, BuildingHvacMachineLearningRejection> {
    Err(BuildingHvacMachineLearningRejection::Xgboost(String::from(
        "the static XGBoost backend is available on the Linux Hub target",
    )))
}

#[cfg(target_os = "linux")]
fn predict_active_booster(
    active: &ActiveBooster,
    features: BuildingHvacMachineLearningFeaturesV1,
) -> Option<BuildingHvacThermalPredictionV1> {
    if !features.is_well_formed() || features.feature_names() != active.model.feature_names {
        return None;
    }
    predict_booster(&active.booster, &[features])
        .ok()
        .and_then(|values| values.into_iter().next())
        .and_then(bounded_prediction)
        .map(
            |temperature_change_celsius| BuildingHvacThermalPredictionV1 {
                horizon: active.model.horizon,
                temperature_change_celsius,
                source: BuildingHvacThermalPredictionSourceV1::Xgboost,
                model_trained_at: Some(active.model.trained_at),
            },
        )
}

#[cfg(not(target_os = "linux"))]
fn predict_active_booster(
    _active: &ActiveBooster,
    _features: BuildingHvacMachineLearningFeaturesV1,
) -> Option<BuildingHvacThermalPredictionV1> {
    None
}

#[cfg(target_os = "linux")]
fn validate_and_collect_samples(
    horizon: BuildingHvacThermalPredictionHorizonV1,
    feature_names: &[String],
    samples: &[BuildingHvacMachineLearningSampleV1],
) -> Result<Vec<(BuildingHvacMachineLearningSampleV1, f32)>, BuildingHvacMachineLearningRejection> {
    if samples.len() < BUILDING_HVAC_ML_MINIMUM_TRAINING_SAMPLES
        || samples.len() > BUILDING_HVAC_ML_MAXIMUM_TRAINING_SAMPLES_PER_ROOM
    {
        return Err(BuildingHvacMachineLearningRejection::TooFewSamples);
    }
    let room_endpoint = samples[0].room_endpoint;
    if !feature_manifest_is_well_formed(feature_names) {
        return Err(BuildingHvacMachineLearningRejection::InvalidSamples);
    }
    let mut previous_time = None;
    let mut labeled = Vec::with_capacity(samples.len());
    for sample in samples {
        if !sample.is_well_formed()
            || sample.room_endpoint != room_endpoint
            || !sample.features.matches_feature_names(feature_names)
            || previous_time.is_some_and(|previous| previous >= sample.observed_at)
        {
            return Err(BuildingHvacMachineLearningRejection::InvalidSamples);
        }
        previous_time = Some(sample.observed_at);
        if let Some(label) = sample.target(horizon) {
            if !in_range(
                label,
                -BUILDING_HVAC_ML_MAXIMUM_PREDICTED_CHANGE_CELSIUS,
                BUILDING_HVAC_ML_MAXIMUM_PREDICTED_CHANGE_CELSIUS,
            ) {
                return Err(BuildingHvacMachineLearningRejection::InvalidSamples);
            }
            labeled.push((sample.clone(), label));
        }
    }
    if labeled.len() < BUILDING_HVAC_ML_MINIMUM_TRAINING_SAMPLES {
        Err(BuildingHvacMachineLearningRejection::TooFewSamples)
    } else {
        Ok(labeled)
    }
}

#[cfg(target_os = "linux")]
fn train_booster(
    samples: &[(BuildingHvacMachineLearningSampleV1, f32)],
    feature_names: &[String],
) -> Result<Booster, BuildingHvacMachineLearningRejection> {
    let feature_count = samples[0].0.features.dense_feature_count();
    let mut indptr = Vec::with_capacity(samples.len() + 1);
    let mut indices = Vec::new();
    let mut values = Vec::new();
    let mut labels = Vec::with_capacity(samples.len());
    indptr.push(0);
    for (sample, label) in samples {
        if sample.features.dense_feature_count() != feature_count {
            return Err(BuildingHvacMachineLearningRejection::InvalidSamples);
        }
        indices.extend(
            sample
                .features
                .values
                .iter()
                .map(|feature| usize::from(feature.index)),
        );
        values.extend(sample.features.values.iter().map(|feature| feature.value));
        indptr.push(indices.len());
        labels.push(*label);
    }
    let mut matrix = DMatrix::from_csr(&indptr, &indices, &values, Some(feature_count))
        .map_err(xgboost_rejection)?;
    matrix.set_labels(&labels).map_err(xgboost_rejection)?;

    let learning = LearningTaskParametersBuilder::default()
        .objective(Objective::RegLinear)
        .seed(0)
        .build()
        .map_err(|error| BuildingHvacMachineLearningRejection::Xgboost(error.to_string()))?;
    let tree = TreeBoosterParametersBuilder::default()
        .tree_method(TreeMethod::Hist)
        .max_depth(BUILDING_HVAC_ML_MAXIMUM_TREE_DEPTH)
        .eta(0.05)
        .subsample(0.8)
        .colsample_bytree(0.8)
        .min_child_weight(4.0)
        .build()
        .map_err(|error| BuildingHvacMachineLearningRejection::Xgboost(error.to_string()))?;
    let booster_parameters = parameters::BoosterParametersBuilder::default()
        .booster_type(BoosterType::Tree(tree))
        .learning_params(learning)
        .threads(Some(BUILDING_HVAC_ML_XGBOOST_THREADS))
        .verbose(false)
        .build()
        .map_err(|error| BuildingHvacMachineLearningRejection::Xgboost(error.to_string()))?;
    let mut booster = Booster::new_with_cached_dmats(&booster_parameters, &[&matrix])
        .map_err(xgboost_rejection)?;
    let feature_name_references: Vec<_> = feature_names.iter().map(String::as_str).collect();
    booster
        .set_feature_names(&feature_name_references)
        .map_err(xgboost_rejection)?;
    // Drive each round explicitly. The xgb 3.0.5 convenience `train` function
    // creates a booster but does not call `update`, leaving an untrained model.
    for iteration in 0..BUILDING_HVAC_ML_BOOST_ROUNDS {
        booster
            .update(&matrix, i32::try_from(iteration).unwrap_or(i32::MAX))
            .map_err(xgboost_rejection)?;
    }
    Ok(booster)
}

#[cfg(target_os = "linux")]
fn predict_booster(
    booster: &Booster,
    features: &[BuildingHvacMachineLearningFeaturesV1],
) -> Result<Vec<f32>, BuildingHvacMachineLearningRejection> {
    if features.is_empty() || features.iter().any(|features| !features.is_well_formed()) {
        return Err(BuildingHvacMachineLearningRejection::InvalidSamples);
    }
    let feature_count = features[0].dense_feature_count();
    if features
        .iter()
        .any(|features| features.dense_feature_count() != feature_count)
    {
        return Err(BuildingHvacMachineLearningRejection::InvalidSamples);
    }
    let mut dense = Vec::with_capacity(features.len() * feature_count);
    for features in features {
        features.append_dense(&mut dense);
    }
    let matrix = DMatrix::from_dense(&dense, features.len()).map_err(xgboost_rejection)?;
    let predictions = booster.predict(&matrix).map_err(xgboost_rejection)?;
    if predictions.len() != features.len()
        || predictions
            .iter()
            .any(|prediction| bounded_prediction(*prediction).is_none())
    {
        Err(BuildingHvacMachineLearningRejection::InvalidArtifact)
    } else {
        Ok(predictions)
    }
}

#[cfg(target_os = "linux")]
fn predict_feature_vectors(
    booster: &Booster,
    features: &[BuildingHvacMachineLearningFeatureVectorV1],
) -> Result<Vec<f32>, BuildingHvacMachineLearningRejection> {
    if features.is_empty() || features.iter().any(|features| !features.is_well_formed()) {
        return Err(BuildingHvacMachineLearningRejection::InvalidSamples);
    }
    let feature_count = features[0].dense_feature_count();
    if features
        .iter()
        .any(|features| features.dense_feature_count() != feature_count)
    {
        return Err(BuildingHvacMachineLearningRejection::InvalidSamples);
    }
    let mut indptr = Vec::with_capacity(features.len() + 1);
    let mut indices = Vec::new();
    let mut values = Vec::new();
    indptr.push(0);
    for features in features {
        indices.extend(
            features
                .values
                .iter()
                .map(|feature| usize::from(feature.index)),
        );
        values.extend(features.values.iter().map(|feature| feature.value));
        indptr.push(indices.len());
    }
    let matrix = DMatrix::from_csr(&indptr, &indices, &values, Some(feature_count))
        .map_err(xgboost_rejection)?;
    let predictions = booster.predict(&matrix).map_err(xgboost_rejection)?;
    if predictions.len() != features.len()
        || predictions
            .iter()
            .any(|prediction| bounded_prediction(*prediction).is_none())
    {
        Err(BuildingHvacMachineLearningRejection::InvalidArtifact)
    } else {
        Ok(predictions)
    }
}

#[cfg(target_os = "linux")]
fn rmse(predictions: &[f32], labels: &[f32]) -> Option<f32> {
    if predictions.is_empty() || predictions.len() != labels.len() {
        return None;
    }
    let sum_squared_error =
        predictions
            .iter()
            .zip(labels)
            .try_fold(0.0_f64, |sum, (prediction, label)| {
                if !prediction.is_finite() || !label.is_finite() {
                    return None;
                }
                let error = f64::from(*prediction) - f64::from(*label);
                Some(sum + error * error)
            })?;
    let result = (sum_squared_error / predictions.len() as f64).sqrt() as f32;
    result.is_finite().then_some(result)
}

fn model_sha256(model_ubjson: &[u8]) -> [u8; 32] {
    Sha256::digest(model_ubjson).into()
}

fn merge_sample_targets(
    existing: BuildingHvacMachineLearningSampleV1,
    incoming: BuildingHvacMachineLearningSampleV1,
) -> Result<BuildingHvacMachineLearningSampleV1, BuildingHvacMachineLearningHistoryError> {
    Ok(BuildingHvacMachineLearningSampleV1 {
        observed_at: existing.observed_at,
        room_endpoint: existing.room_endpoint,
        features: existing.features,
        temperature_change_15_minutes_celsius: merge_target(
            existing.temperature_change_15_minutes_celsius,
            incoming.temperature_change_15_minutes_celsius,
        )?,
        temperature_change_30_minutes_celsius: merge_target(
            existing.temperature_change_30_minutes_celsius,
            incoming.temperature_change_30_minutes_celsius,
        )?,
        temperature_change_60_minutes_celsius: merge_target(
            existing.temperature_change_60_minutes_celsius,
            incoming.temperature_change_60_minutes_celsius,
        )?,
    })
}

fn merge_target(
    existing: Option<f32>,
    incoming: Option<f32>,
) -> Result<Option<f32>, BuildingHvacMachineLearningHistoryError> {
    match (existing, incoming) {
        (Some(existing), Some(incoming)) if (existing - incoming).abs() > f32::EPSILON => {
            Err(BuildingHvacMachineLearningHistoryError::ConflictingObservation)
        }
        (Some(existing), _) => Ok(Some(existing)),
        (None, incoming) => Ok(incoming),
    }
}

#[cfg(target_os = "linux")]
fn bounded_prediction(value: f32) -> Option<f32> {
    in_range(
        value,
        -BUILDING_HVAC_ML_MAXIMUM_PREDICTED_CHANGE_CELSIUS,
        BUILDING_HVAC_ML_MAXIMUM_PREDICTED_CHANGE_CELSIUS,
    )
    .then_some(value)
}

#[cfg(target_os = "linux")]
fn xgboost_rejection(error: xgb::XGBError) -> BuildingHvacMachineLearningRejection {
    BuildingHvacMachineLearningRejection::Xgboost(error.to_string())
}

#[cfg(target_os = "linux")]
fn missing(value: Option<f32>) -> f32 {
    value.unwrap_or(f32::NAN)
}

fn optional_finite(value: Option<f32>) -> bool {
    value.is_none_or(f32::is_finite)
}

fn optional_in_range(value: Option<f32>, minimum: f32, maximum: f32) -> bool {
    value.is_none_or(|value| in_range(value, minimum, maximum))
}

fn in_range(value: f32, minimum: f32, maximum: f32) -> bool {
    value.is_finite() && value >= minimum && value <= maximum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(name: &str, value: Option<f32>) -> BuildingHvacMachineLearningFeatureV1 {
        BuildingHvacMachineLearningFeatureV1 {
            name: String::from(name),
            value,
        }
    }

    fn set_feature(
        features: &mut BuildingHvacMachineLearningFeaturesV1,
        name: &str,
        value: Option<f32>,
    ) {
        let index = features
            .values
            .binary_search_by(|feature| feature.name.as_str().cmp(name))
            .unwrap();
        features.values[index].value = value;
    }

    fn set_compact_feature(
        features: &mut BuildingHvacMachineLearningFeatureVectorV1,
        feature_names: &[String],
        name: &str,
        value: Option<f32>,
    ) {
        let index = feature_names
            .binary_search_by(|feature| feature.as_str().cmp(name))
            .unwrap();
        let index = u16::try_from(index).unwrap();
        match (
            features
                .values
                .binary_search_by_key(&index, |feature| feature.index),
            value,
        ) {
            (Ok(position), Some(value)) => features.values[position].value = value,
            (Ok(position), None) => {
                features.values.remove(position);
            }
            (Err(position), Some(value)) => {
                features.values.insert(
                    position,
                    BuildingHvacMachineLearningIndexedFeatureV1 { index, value },
                );
            }
            (Err(_), None) => {}
        }
    }

    fn features() -> BuildingHvacMachineLearningFeaturesV1 {
        let mut values = vec![
            feature("target.temperature_celsius", Some(20.0)),
            feature("thermostat.200.active_setpoint_delta_celsius", Some(2.0)),
            feature("thermostat.201.active_setpoint_delta_celsius", Some(0.0)),
            feature("time.day_of_year_cosine", Some(1.0)),
            feature("time.day_of_year_sine", Some(0.0)),
            feature("weather.current.dry_bulb_temperature_celsius", Some(5.0)),
            feature(
                "weather.current.global_horizontal_irradiance_watts_per_square_meter",
                Some(150.0),
            ),
            feature("weather.current.humidity_ratio_kg_per_kg", Some(0.004)),
            feature("weather.current.wind_speed_meters_per_second", Some(3.0)),
        ];
        values.sort_by(|left, right| left.name.cmp(&right.name));
        BuildingHvacMachineLearningFeaturesV1 {
            target_room: 100,
            values,
        }
    }

    fn sample() -> BuildingHvacMachineLearningSampleV1 {
        BuildingHvacMachineLearningSampleV1 {
            observed_at: 1_785_059_200,
            room_endpoint: 100,
            features: features().compact(),
            temperature_change_15_minutes_celsius: Some(0.2),
            temperature_change_30_minutes_celsius: None,
            temperature_change_60_minutes_celsius: None,
        }
    }

    fn archived_sample(
        observed_at: LibertasDateTime,
        annual_position: f32,
        outdoor_temperature_celsius: f32,
        demand_pattern: u8,
    ) -> BuildingHvacMachineLearningSampleV1 {
        let mut features = features();
        let phase = annual_position * std::f32::consts::TAU;
        set_feature(&mut features, "time.day_of_year_sine", Some(phase.sin()));
        set_feature(&mut features, "time.day_of_year_cosine", Some(phase.cos()));
        set_feature(
            &mut features,
            "weather.current.dry_bulb_temperature_celsius",
            Some(outdoor_temperature_celsius),
        );
        set_feature(
            &mut features,
            "weather.current.humidity_ratio_kg_per_kg",
            Some(if outdoor_temperature_celsius > 25.0 {
                0.016
            } else {
                0.005
            }),
        );
        set_feature(
            &mut features,
            "weather.current.global_horizontal_irradiance_watts_per_square_meter",
            Some(if (observed_at / (12 * 60 * 60)).is_multiple_of(2) {
                0.0
            } else {
                700.0
            }),
        );
        set_feature(
            &mut features,
            "thermostat.200.active_setpoint_delta_celsius",
            Some(0.0),
        );
        set_feature(
            &mut features,
            "thermostat.201.active_setpoint_delta_celsius",
            Some(0.0),
        );
        match demand_pattern {
            1 => set_feature(
                &mut features,
                "thermostat.200.active_setpoint_delta_celsius",
                Some(2.0),
            ),
            2 => set_feature(
                &mut features,
                "thermostat.200.active_setpoint_delta_celsius",
                Some(-2.0),
            ),
            3 => set_feature(
                &mut features,
                "thermostat.201.active_setpoint_delta_celsius",
                Some(2.0),
            ),
            4 => set_feature(
                &mut features,
                "thermostat.201.active_setpoint_delta_celsius",
                Some(-2.0),
            ),
            5 => {
                set_feature(
                    &mut features,
                    "thermostat.200.active_setpoint_delta_celsius",
                    Some(2.0),
                );
                set_feature(
                    &mut features,
                    "thermostat.201.active_setpoint_delta_celsius",
                    Some(2.0),
                );
            }
            6 => {
                set_feature(
                    &mut features,
                    "thermostat.200.active_setpoint_delta_celsius",
                    Some(-2.0),
                );
                set_feature(
                    &mut features,
                    "thermostat.201.active_setpoint_delta_celsius",
                    Some(-2.0),
                );
            }
            _ => {}
        }
        BuildingHvacMachineLearningSampleV1 {
            observed_at,
            room_endpoint: 100,
            features: features.compact(),
            temperature_change_15_minutes_celsius: Some(0.2),
            temperature_change_30_minutes_celsius: Some(0.35),
            temperature_change_60_minutes_celsius: Some(0.5),
        }
    }

    #[test]
    fn sample_merge_adds_labels_without_erasing_existing_targets() {
        let existing = sample();
        let mut incoming = sample();
        incoming.temperature_change_15_minutes_celsius = None;
        incoming.temperature_change_30_minutes_celsius = Some(0.35);
        let merged = merge_sample_targets(existing, incoming).unwrap();
        assert_eq!(merged.temperature_change_15_minutes_celsius, Some(0.2));
        assert_eq!(merged.temperature_change_30_minutes_celsius, Some(0.35));
    }

    #[test]
    fn sample_merge_rejects_changed_targets() {
        let existing = sample();
        let mut conflicting_target = sample();
        conflicting_target.temperature_change_15_minutes_celsius = Some(0.3);
        assert_eq!(
            merge_sample_targets(existing, conflicting_target),
            Err(BuildingHvacMachineLearningHistoryError::ConflictingObservation)
        );
    }

    #[test]
    fn feature_manifest_preserves_sorted_named_columns_and_missing_values() {
        let features = features();
        assert_eq!(features.dense_feature_count(), 9);
        assert_eq!(
            features.feature_names(),
            [
                "target.temperature_celsius",
                "thermostat.200.active_setpoint_delta_celsius",
                "thermostat.201.active_setpoint_delta_celsius",
                "time.day_of_year_cosine",
                "time.day_of_year_sine",
                "weather.current.dry_bulb_temperature_celsius",
                "weather.current.global_horizontal_irradiance_watts_per_square_meter",
                "weather.current.humidity_ratio_kg_per_kg",
                "weather.current.wind_speed_meters_per_second",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        );
        assert!(feature_manifest_is_well_formed(&features.feature_names()));

        let mut reordered = features.clone();
        reordered.values.swap(0, 1);
        assert!(!reordered.is_well_formed());

        let mut duplicate = features;
        duplicate.values[1].name = duplicate.values[0].name.clone();
        assert!(!duplicate.is_well_formed());
    }

    #[test]
    fn compact_features_keep_numeric_zero_and_omit_only_missing_values() {
        let mut named = features();
        set_feature(
            &mut named,
            "weather.current.wind_speed_meters_per_second",
            None,
        );
        let names = named.feature_names();
        let compact = named.compact();

        assert!(compact.is_well_formed());
        assert!(compact.matches_feature_names(&names));
        assert_eq!(
            compact.value(&names, "thermostat.201.active_setpoint_delta_celsius"),
            Some(0.0)
        );
        assert_eq!(
            compact.value(&names, "weather.current.wind_speed_meters_per_second"),
            None
        );

        let mut different_names = names;
        different_names[0].push_str("_different");
        different_names.sort();
        assert!(!compact.matches_feature_names(&different_names));
    }

    #[test]
    fn training_selection_keeps_recent_and_stratified_seasonal_evidence() {
        const SAMPLE_INTERVAL_SECONDS: u64 = 15 * 60;
        let starts_at = 1_700_000_000;
        let mut samples = Vec::with_capacity(400 * 24 * 4);
        for index in 0..400 * 24 * 4 {
            let day = index / (24 * 4);
            let annual_position = (day % 365) as f32 / 365.0;
            let seasonal_temperature =
                15.0 - 20.0 * (annual_position * std::f32::consts::TAU).cos();
            samples.push(archived_sample(
                starts_at + index as u64 * SAMPLE_INTERVAL_SECONDS,
                annual_position,
                seasonal_temperature,
                (index % 7) as u8,
            ));
        }
        let rare_extreme_at = samples[100].observed_at;
        let feature_names = features().feature_names();
        set_compact_feature(
            &mut samples[100].features,
            &feature_names,
            "weather.current.dry_bulb_temperature_celsius",
            Some(-30.0),
        );
        set_compact_feature(
            &mut samples[100].features,
            &feature_names,
            "weather.current.humidity_ratio_kg_per_kg",
            Some(0.002),
        );
        set_compact_feature(
            &mut samples[100].features,
            &feature_names,
            "weather.current.wind_speed_meters_per_second",
            Some(20.0),
        );

        let through_utc = samples.last().unwrap().observed_at;
        let selected = select_stratified_training_samples(&samples, through_utc, &feature_names);

        assert_eq!(
            selected.len(),
            BUILDING_HVAC_ML_MAXIMUM_TRAINING_SAMPLES_PER_ROOM
        );
        assert!(
            selected
                .windows(2)
                .all(|pair| pair[0].observed_at < pair[1].observed_at)
        );
        assert_eq!(selected.last().unwrap().observed_at, through_utc);
        assert!(
            selected
                .iter()
                .any(|sample| sample.observed_at == rare_extreme_at)
        );

        let recent_starts_at = through_utc.saturating_sub(BUILDING_HVAC_ML_RECENT_WINDOW_SECONDS);
        assert_eq!(
            selected
                .iter()
                .filter(|sample| sample.observed_at >= recent_starts_at)
                .count(),
            BUILDING_HVAC_ML_MAXIMUM_TRAINING_SAMPLES_PER_ROOM
                * BUILDING_HVAC_ML_RECENT_TRAINING_PERCENT
                / 100
        );

        let annual_phases: std::collections::BTreeSet<_> = selected
            .iter()
            .map(|sample| {
                annual_phase_bin(
                    sample
                        .features
                        .value(&feature_names, "time.day_of_year_sine")
                        .unwrap(),
                    sample
                        .features
                        .value(&feature_names, "time.day_of_year_cosine")
                        .unwrap(),
                )
            })
            .collect();
        let demand_modes: std::collections::BTreeSet<_> = selected
            .iter()
            .map(|sample| {
                let stratum = training_stratum(&sample.features, &feature_names);
                (
                    stratum.heating_demand_thermostats,
                    stratum.cooling_demand_thermostats,
                )
            })
            .collect();
        assert_eq!(annual_phases.len(), 8);
        assert_eq!(
            demand_modes,
            [(0, 0), (0, 1), (0, 2), (1, 0), (2, 0)]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn training_selection_does_not_drop_a_bounded_history() {
        let samples: Vec<_> = (0..BUILDING_HVAC_ML_MINIMUM_TRAINING_SAMPLES)
            .map(|index| archived_sample(1_700_000_000 + index as u64 * 900, 0.25, 5.0, 1))
            .collect();
        let feature_names = features().feature_names();
        assert_eq!(
            select_stratified_training_samples(
                &samples,
                samples.last().unwrap().observed_at,
                &feature_names,
            ),
            samples
        );
    }

    #[test]
    fn retained_archive_covers_400_days_at_fifteen_minute_resolution() {
        assert_eq!(
            BUILDING_HVAC_ML_HISTORY_RETENTION_SECONDS,
            400 * 24 * 60 * 60
        );
        assert_eq!(
            BUILDING_HVAC_ML_MAXIMUM_RETAINED_SAMPLES_PER_ROOM,
            400 * 24 * 4 + 1
        );
    }

    #[test]
    fn client_allows_only_one_queued_or_running_training_cycle() {
        let (commands, _receiver) = sync_channel(BUILDING_HVAC_ML_COMMAND_CAPACITY);
        let client = BuildingHvacMachineLearningClient {
            commands,
            stop_requested: Arc::new(AtomicBool::new(false)),
            training_pending: Arc::new(AtomicBool::new(false)),
        };
        assert!(!client.training_pending());
        assert_eq!(client.try_train_all(1, Vec::new(), Vec::new()), Ok(()));
        assert!(client.training_pending());
        assert_eq!(
            client.try_train_all(1, Vec::new(), Vec::new()),
            Err(BuildingHvacMachineLearningQueueError::Full)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn machine_learning_worker_priority_is_lowered_on_linux() {
        let inherited = rustix::process::getpriority_process(None).unwrap();
        let (reported, observed) = thread::spawn(|| {
            let reported = configure_machine_learning_worker_priority().unwrap();
            let observed = rustix::process::getpriority_process(None).unwrap();
            (reported, observed)
        })
        .join()
        .unwrap();
        assert_eq!(reported, observed);
        assert_eq!(
            observed,
            inherited
                .saturating_add(BUILDING_HVAC_ML_WORKER_NICE_INCREMENT)
                .min(19)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "manual Hub-target resource measurement"]
    fn benchmark_three_horizon_training_cycle() {
        let starts_at = 1_700_000_000;
        let feature_names = features().feature_names();
        let samples: Vec<_> = (0..BUILDING_HVAC_ML_MAXIMUM_TRAINING_SAMPLES_PER_ROOM)
            .map(|index| {
                let annual_position = (index / (24 * 4) % 365) as f32 / 365.0;
                let mut sample = archived_sample(
                    starts_at + index as u64 * 15 * 60,
                    annual_position,
                    15.0 - 20.0 * (annual_position * std::f32::consts::TAU).cos(),
                    (index % 5) as u8,
                );
                let first_heating = index.is_multiple_of(11);
                let second_heating = (index + 1).is_multiple_of(13);
                set_compact_feature(
                    &mut sample.features,
                    &feature_names,
                    "thermostat.200.active_setpoint_delta_celsius",
                    Some(if first_heating { 2.0 } else { 0.0 }),
                );
                set_compact_feature(
                    &mut sample.features,
                    &feature_names,
                    "thermostat.201.active_setpoint_delta_celsius",
                    Some(if second_heating { 2.0 } else { 0.0 }),
                );
                let first_delta = sample
                    .features
                    .value(
                        &feature_names,
                        "thermostat.200.active_setpoint_delta_celsius",
                    )
                    .unwrap();
                let second_delta = sample
                    .features
                    .value(
                        &feature_names,
                        "thermostat.201.active_setpoint_delta_celsius",
                    )
                    .unwrap();
                sample.temperature_change_15_minutes_celsius = Some(
                    0.3 * first_delta
                        + 0.5
                            * if first_delta > 0.0 && second_delta > 0.0 {
                                1.0
                            } else {
                                0.0
                            }
                        + (20.0
                            - sample
                                .features
                                .value(&feature_names, "target.temperature_celsius")
                                .unwrap())
                            * 0.03,
                );
                sample
            })
            .collect();
        let trained_at = samples.last().unwrap().observed_at + 15 * 60;
        let started = std::time::Instant::now();
        let models: Vec<_> = [
            BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
            BuildingHvacThermalPredictionHorizonV1::ThirtyMinutes,
            BuildingHvacThermalPredictionHorizonV1::SixtyMinutes,
        ]
        .into_iter()
        .map(|horizon| {
            BuildingHvacMachineLearningEngine::train_candidate(
                horizon,
                trained_at,
                &feature_names,
                &samples,
            )
            .unwrap()
        })
        .collect();
        let feature_vector_bytes: usize = samples
            .iter()
            .map(|sample| {
                sample.features.values.capacity()
                    * std::mem::size_of::<BuildingHvacMachineLearningIndexedFeatureV1>()
            })
            .sum();
        eprintln!(
            "samples={} features={} horizons={} sample_size_bytes={} sample_vector_bytes={} feature_vector_bytes={} owned_sample_bytes={} model_bytes={} elapsed_ms={}",
            samples.len(),
            samples[0].features.dense_feature_count(),
            models.len(),
            std::mem::size_of::<BuildingHvacMachineLearningSampleV1>(),
            samples.capacity() * std::mem::size_of::<BuildingHvacMachineLearningSampleV1>(),
            feature_vector_bytes,
            samples.capacity() * std::mem::size_of::<BuildingHvacMachineLearningSampleV1>()
                + feature_vector_bytes,
            models
                .iter()
                .map(|model| model.model_ubjson.len())
                .sum::<usize>(),
            started.elapsed().as_millis(),
        );
    }
}
