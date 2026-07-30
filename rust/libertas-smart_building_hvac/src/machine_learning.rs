//! Bounded XGBoost thermal prediction for the Hub runtime.
//!
//! The public V1 values in this module describe training evidence, persisted
//! model artifacts, and user-visible prediction state. XGBoost itself stays
//! behind the safe `xgb` wrapper and is owned by one worker thread.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
};
use std::thread;

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

/// Machine-learning feature count
/// The exact number of ordered features accepted by the V1 XGBoost models.
pub const BUILDING_HVAC_ML_FEATURE_COUNT: usize = 16;

/// Minimum training samples
/// The minimum number of ordered, valid, labeled room periods required before
/// attempting a candidate model.
pub const BUILDING_HVAC_ML_MINIMUM_TRAINING_SAMPLES: usize = 14 * 24 * 4;

/// Maximum retained training samples per room
/// Ninety days of 15-minute samples, including one extra day of capacity for
/// pruning without creating a gap.
pub const BUILDING_HVAC_ML_MAXIMUM_TRAINING_SAMPLES_PER_ROOM: usize = 91 * 24 * 4;

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

/// Ordered V1 feature names
/// These names are persisted with every model and must match byte-for-byte
/// before the model is loaded.
pub const BUILDING_HVAC_ML_FEATURE_NAMES: [&str; BUILDING_HVAC_ML_FEATURE_COUNT] = [
    "room_temperature_celsius",
    "room_relative_humidity_percent",
    "outdoor_temperature_celsius",
    "outdoor_humidity_ratio_kilograms_per_kilogram",
    "outdoor_wind_speed_meters_per_second",
    "global_horizontal_solar_irradiance_watts_per_square_meter",
    "hour_of_day_sine",
    "hour_of_day_cosine",
    "day_of_year_sine",
    "day_of_year_cosine",
    "own_heating_runtime_fraction",
    "own_cooling_runtime_fraction",
    "other_zone_heating_runtime_fraction",
    "other_zone_cooling_runtime_fraction",
    "heating_setpoint_offset_celsius",
    "cooling_setpoint_offset_celsius",
];

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

/// XGBoost thermal features V1
/// Contains the exact ordered inputs for one room observation. Optional
/// physical measurements become XGBoost missing values; runtime fractions and
/// cyclical time encodings are always required.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacMachineLearningFeaturesV1 {
    /// Room temperature
    /// Current fused room temperature in degrees Celsius.
    pub room_temperature_celsius: f32,
    /// Room relative humidity
    /// Current fused room relative humidity percentage when available.
    pub room_relative_humidity_percent: Option<f32>,
    /// Outdoor temperature
    /// Fresh local or weather outdoor dry-bulb temperature in degrees Celsius.
    pub outdoor_temperature_celsius: Option<f32>,
    /// Outdoor humidity ratio
    /// Derived kilograms of water per kilogram of dry air when current weather
    /// is internally consistent.
    pub outdoor_humidity_ratio_kilograms_per_kilogram: Option<f32>,
    /// Outdoor wind speed
    /// Current wind speed in meters per second when available.
    pub outdoor_wind_speed_meters_per_second: Option<f32>,
    /// Global-horizontal solar irradiance
    /// Current solar irradiance in watts per square meter when available.
    pub global_horizontal_solar_irradiance_watts_per_square_meter: Option<f32>,
    /// Hour-of-day sine
    /// Sine encoding of local solar time, bounded from -1 through 1.
    #[libertas_number(min = -1, max = 1)]
    pub hour_of_day_sine: f32,
    /// Hour-of-day cosine
    /// Cosine encoding paired with `hour_of_day_sine`.
    #[libertas_number(min = -1, max = 1)]
    pub hour_of_day_cosine: f32,
    /// Day-of-year sine
    /// Sine encoding of the annual position, bounded from -1 through 1.
    #[libertas_number(min = -1, max = 1)]
    pub day_of_year_sine: f32,
    /// Day-of-year cosine
    /// Cosine encoding paired with `day_of_year_sine`.
    #[libertas_number(min = -1, max = 1)]
    pub day_of_year_cosine: f32,
    /// Own-zone heating runtime
    /// Fraction of the observation period during which this room's thermostat
    /// zone reported heating.
    #[libertas_number(min = 0, max = 1)]
    pub own_heating_runtime_fraction: f32,
    /// Own-zone cooling runtime
    /// Fraction of the observation period during which this room's thermostat
    /// zone reported cooling.
    #[libertas_number(min = 0, max = 1)]
    pub own_cooling_runtime_fraction: f32,
    /// Other-zone heating runtime
    /// Aggregate fraction of the observation period during which other source
    /// thermostat zones reported heating, capped at one.
    #[libertas_number(min = 0, max = 1)]
    pub other_zone_heating_runtime_fraction: f32,
    /// Other-zone cooling runtime
    /// Aggregate fraction of the observation period during which other source
    /// thermostat zones reported cooling, capped at one.
    #[libertas_number(min = 0, max = 1)]
    pub other_zone_cooling_runtime_fraction: f32,
    /// Heating-setpoint offset
    /// Effective heating setpoint minus current room temperature in degrees
    /// Celsius when heating is enabled.
    pub heating_setpoint_offset_celsius: Option<f32>,
    /// Cooling-setpoint offset
    /// Effective cooling setpoint minus current room temperature in degrees
    /// Celsius when cooling is enabled.
    pub cooling_setpoint_offset_celsius: Option<f32>,
}

impl BuildingHvacMachineLearningFeaturesV1 {
    /// Well-formed features
    /// Rejects nonfinite measurements, impossible percentages, negative
    /// physical magnitudes, and inconsistent activity fractions.
    pub fn is_well_formed(&self) -> bool {
        self.room_temperature_celsius.is_finite()
            && optional_in_range(self.room_relative_humidity_percent, 0.0, 100.0)
            && optional_finite(self.outdoor_temperature_celsius)
            && optional_in_range(self.outdoor_humidity_ratio_kilograms_per_kilogram, 0.0, 1.0)
            && optional_in_range(self.outdoor_wind_speed_meters_per_second, 0.0, f32::MAX)
            && optional_in_range(
                self.global_horizontal_solar_irradiance_watts_per_square_meter,
                0.0,
                f32::MAX,
            )
            && in_range(self.hour_of_day_sine, -1.0, 1.0)
            && in_range(self.hour_of_day_cosine, -1.0, 1.0)
            && in_range(self.day_of_year_sine, -1.0, 1.0)
            && in_range(self.day_of_year_cosine, -1.0, 1.0)
            && in_range(self.own_heating_runtime_fraction, 0.0, 1.0)
            && in_range(self.own_cooling_runtime_fraction, 0.0, 1.0)
            && self.own_heating_runtime_fraction + self.own_cooling_runtime_fraction <= 1.000_001
            && in_range(self.other_zone_heating_runtime_fraction, 0.0, 1.0)
            && in_range(self.other_zone_cooling_runtime_fraction, 0.0, 1.0)
            && self.other_zone_heating_runtime_fraction + self.other_zone_cooling_runtime_fraction
                <= 1.000_001
            && optional_in_range(self.heating_setpoint_offset_celsius, -50.0, 50.0)
            && optional_in_range(self.cooling_setpoint_offset_celsius, -50.0, 50.0)
    }

    #[cfg(target_os = "linux")]
    fn append_dense(self, values: &mut Vec<f32>) {
        values.extend_from_slice(&[
            self.room_temperature_celsius,
            missing(self.room_relative_humidity_percent),
            missing(self.outdoor_temperature_celsius),
            missing(self.outdoor_humidity_ratio_kilograms_per_kilogram),
            missing(self.outdoor_wind_speed_meters_per_second),
            missing(self.global_horizontal_solar_irradiance_watts_per_square_meter),
            self.hour_of_day_sine,
            self.hour_of_day_cosine,
            self.day_of_year_sine,
            self.day_of_year_cosine,
            self.own_heating_runtime_fraction,
            self.own_cooling_runtime_fraction,
            self.other_zone_heating_runtime_fraction,
            self.other_zone_cooling_runtime_fraction,
            missing(self.heating_setpoint_offset_celsius),
            missing(self.cooling_setpoint_offset_celsius),
        ]);
    }
}

/// Machine-learning training sample V1
/// One indexed room observation and the temperature changes later measured at
/// each supported horizon. It is written only after at least one target becomes
/// known; missing targets remain available for later completion.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacMachineLearningSampleV1 {
    /// Observed at
    /// UTC timestamp of the feature observation and indexed database key.
    pub observed_at: LibertasDateTime,
    /// Room endpoint
    /// Stable room identity used to reject a sample read from the wrong indexed
    /// room history.
    pub room_endpoint: LibertasEndpoint,
    /// Features
    /// Exact V1 thermal feature values at `observed_at`.
    pub features: BuildingHvacMachineLearningFeaturesV1,
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
    pub const fn target(self, horizon: BuildingHvacThermalPredictionHorizonV1) -> Option<f32> {
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
    /// Human- and AI-readable manifest matching the XGBoost column order.
    /// ----
    /// Feature name
    /// One stable feature identifier in model-column order.
    #[libertas_size(min = 16, max = 16)]
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
            && self.feature_names.len() == BUILDING_HVAC_ML_FEATURE_COUNT
            && self
                .feature_names
                .iter()
                .zip(BUILDING_HVAC_ML_FEATURE_NAMES)
                .all(|(actual, expected)| actual == expected)
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
    Train {
        horizon: BuildingHvacThermalPredictionHorizonV1,
        trained_at: LibertasDateTime,
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
        let retention_seconds =
            (BUILDING_HVAC_ML_MAXIMUM_TRAINING_SAMPLES_PER_ROOM as u64).saturating_mul(15 * 60);
        if !sample.is_well_formed()
            || sample.observed_at > now_utc
            || sample.observed_at < now_utc.saturating_sub(retention_seconds)
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

        let oldest_retained = now_utc.saturating_sub(retention_seconds);
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

    /// Load recent samples
    /// Reads newest records through `through_utc`, rejects mismatched or invalid
    /// values, removes duplicate timestamps, and returns ascending observation
    /// order for time-forward training.
    pub fn load_recent_samples(
        room_endpoint: LibertasEndpoint,
        through_utc: LibertasDateTime,
        maximum_samples: usize,
    ) -> Vec<BuildingHvacMachineLearningSampleV1> {
        let maximum_samples =
            maximum_samples.min(BUILDING_HVAC_ML_MAXIMUM_TRAINING_SAMPLES_PER_ROOM);
        if maximum_samples == 0 {
            return Vec::new();
        }
        let Ok(index) = i64::try_from(through_utc) else {
            return Vec::new();
        };
        let key = [NotificationArgument::Object(room_endpoint)];
        let database = libertas_data_open_indexed(BUILDING_HVAC_ML_SAMPLE_RESOURCE, &key);
        let mut records = Vec::new();
        libertas_data_read_indexed_range::<crate::BuildingHvacPersistentDataV1>(
            database.handle,
            index,
            IndexDirection::Below,
            maximum_samples,
            &mut records,
        );
        let mut samples: Vec<_> = records
            .into_iter()
            .filter_map(|record| match record.data {
                crate::BuildingHvacPersistentDataV1::MachineLearningSampleV1 { sample }
                    if i64::try_from(sample.observed_at) == Ok(record.index)
                        && sample.room_endpoint == room_endpoint
                        && sample.observed_at <= through_utc
                        && sample.is_well_formed() =>
                {
                    Some(sample)
                }
                _ => None,
            })
            .collect();
        samples.sort_by_key(|sample| sample.observed_at);
        samples.dedup_by_key(|sample| sample.observed_at);
        samples
    }
}

/// Machine-learning worker client
/// Cloneable bounded sender used by Libertas callbacks. Every operation is
/// nonblocking and transfers owned data to the single XGBoost thread.
#[derive(Clone)]
pub struct BuildingHvacMachineLearningClient {
    commands: SyncSender<BuildingHvacMachineLearningCommand>,
    stop_requested: Arc<AtomicBool>,
}

impl BuildingHvacMachineLearningClient {
    /// Try training
    /// Enqueues one independently bounded candidate-training job.
    pub fn try_train(
        &self,
        horizon: BuildingHvacThermalPredictionHorizonV1,
        trained_at: LibertasDateTime,
        samples: Vec<BuildingHvacMachineLearningSampleV1>,
    ) -> Result<(), BuildingHvacMachineLearningQueueError> {
        self.try_send(BuildingHvacMachineLearningCommand::Train {
            horizon,
            trained_at,
            samples,
        })
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
        samples: &[BuildingHvacMachineLearningSampleV1],
    ) -> Result<BuildingHvacMachineLearningModelV1, BuildingHvacMachineLearningRejection> {
        let labeled = validate_and_collect_samples(horizon, samples)?;
        let validation_count = (labeled.len() * BUILDING_HVAC_ML_VALIDATION_PERCENT / 100)
            .max(BUILDING_HVAC_ML_MINIMUM_VALIDATION_SAMPLES)
            .min(labeled.len().saturating_sub(1));
        let training_count = labeled.len().saturating_sub(validation_count);
        if training_count == 0 || validation_count < BUILDING_HVAC_ML_MINIMUM_VALIDATION_SAMPLES {
            return Err(BuildingHvacMachineLearningRejection::TooFewSamples);
        }

        let (training, validation) = labeled.split_at(training_count);
        let booster = train_booster(training)?;
        let validation_predictions = predict_booster(
            &booster,
            &validation
                .iter()
                .map(|(sample, _)| sample.features)
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
        let candidate = BuildingHvacMachineLearningModelV1 {
            room_endpoint: training[0].0.room_endpoint,
            horizon,
            feature_schema_version: BUILDING_HVAC_ML_FEATURE_SCHEMA_VERSION,
            feature_names: BUILDING_HVAC_ML_FEATURE_NAMES
                .iter()
                .map(|name| String::from(*name))
                .collect(),
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
        if !model.is_well_formed() || !features.is_well_formed() {
            return Err(BuildingHvacMachineLearningRejection::InvalidArtifact);
        }
        let booster = Booster::load_buffer(&model.model_ubjson).map_err(xgboost_rejection)?;
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
    let stop_requested = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop_requested);
    thread::Builder::new()
        .name(String::from("libertas-hvac-xgboost"))
        .spawn(move || {
            machine_learning_worker(
                command_receiver,
                result_sender,
                worker_stop,
                wake_main,
                shutdown_complete,
            );
        })
        .map_err(|error| format!("failed to start XGBoost worker: {error}"))?;
    Ok((
        BuildingHvacMachineLearningClient {
            commands: command_sender,
            stop_requested,
        },
        result_receiver,
    ))
}

fn machine_learning_worker(
    commands: Receiver<BuildingHvacMachineLearningCommand>,
    results: SyncSender<BuildingHvacMachineLearningResult>,
    stop_requested: Arc<AtomicBool>,
    wake_main: fn(),
    shutdown_complete: fn(),
) {
    let mut active_models: Vec<ActiveBooster> = Vec::new();
    while let Ok(command) = commands.recv() {
        if stop_requested.load(Ordering::Acquire)
            || matches!(command, BuildingHvacMachineLearningCommand::Shutdown)
        {
            shutdown_complete();
            return;
        }
        match command {
            BuildingHvacMachineLearningCommand::Train {
                horizon,
                trained_at,
                samples,
            } => {
                let result = match BuildingHvacMachineLearningEngine::train_candidate(
                    horizon, trained_at, &samples,
                ) {
                    Ok(candidate) => BuildingHvacMachineLearningResult::Candidate(candidate),
                    Err(reason) => {
                        BuildingHvacMachineLearningResult::TrainingRejected { horizon, reason }
                    }
                };
                send_worker_result(&results, result, wake_main);
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
    if !features.is_well_formed() {
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
    samples: &[BuildingHvacMachineLearningSampleV1],
) -> Result<Vec<(BuildingHvacMachineLearningSampleV1, f32)>, BuildingHvacMachineLearningRejection> {
    if samples.len() < BUILDING_HVAC_ML_MINIMUM_TRAINING_SAMPLES
        || samples.len() > BUILDING_HVAC_ML_MAXIMUM_TRAINING_SAMPLES_PER_ROOM
    {
        return Err(BuildingHvacMachineLearningRejection::TooFewSamples);
    }
    let room_endpoint = samples[0].room_endpoint;
    let mut previous_time = None;
    let mut labeled = Vec::with_capacity(samples.len());
    for sample in samples {
        if !sample.is_well_formed()
            || sample.room_endpoint != room_endpoint
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
            labeled.push((*sample, label));
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
) -> Result<Booster, BuildingHvacMachineLearningRejection> {
    let mut dense = Vec::with_capacity(samples.len() * BUILDING_HVAC_ML_FEATURE_COUNT);
    let mut labels = Vec::with_capacity(samples.len());
    for (sample, label) in samples {
        sample.features.append_dense(&mut dense);
        labels.push(*label);
    }
    let mut matrix = DMatrix::from_dense(&dense, samples.len()).map_err(xgboost_rejection)?;
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
        .threads(Some(1))
        .verbose(false)
        .build()
        .map_err(|error| BuildingHvacMachineLearningRejection::Xgboost(error.to_string()))?;
    let mut booster = Booster::new_with_cached_dmats(&booster_parameters, &[&matrix])
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
    let mut dense = Vec::with_capacity(features.len() * BUILDING_HVAC_ML_FEATURE_COUNT);
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

    fn features() -> BuildingHvacMachineLearningFeaturesV1 {
        BuildingHvacMachineLearningFeaturesV1 {
            room_temperature_celsius: 20.0,
            room_relative_humidity_percent: Some(45.0),
            outdoor_temperature_celsius: Some(5.0),
            outdoor_humidity_ratio_kilograms_per_kilogram: Some(0.004),
            outdoor_wind_speed_meters_per_second: Some(3.0),
            global_horizontal_solar_irradiance_watts_per_square_meter: Some(150.0),
            hour_of_day_sine: 0.0,
            hour_of_day_cosine: 1.0,
            day_of_year_sine: 0.0,
            day_of_year_cosine: 1.0,
            own_heating_runtime_fraction: 0.5,
            own_cooling_runtime_fraction: 0.0,
            other_zone_heating_runtime_fraction: 0.0,
            other_zone_cooling_runtime_fraction: 0.0,
            heating_setpoint_offset_celsius: Some(2.0),
            cooling_setpoint_offset_celsius: Some(6.0),
        }
    }

    fn sample() -> BuildingHvacMachineLearningSampleV1 {
        BuildingHvacMachineLearningSampleV1 {
            observed_at: 1_785_059_200,
            room_endpoint: 100,
            features: features(),
            temperature_change_15_minutes_celsius: Some(0.2),
            temperature_change_30_minutes_celsius: None,
            temperature_change_60_minutes_celsius: None,
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
}
