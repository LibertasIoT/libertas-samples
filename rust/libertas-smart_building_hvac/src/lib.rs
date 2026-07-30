//! Libertas Smart Building HVAC
//! Configures rooms, their Matter thermostats and environmental sensors, and a
//! building-HVAC weather client. Every room exposes a Libertas endpoint that
//! separates writable comfort intent from read-only observed state, statistics,
//! and the controller's calculated schedule. The Hub build uses `std` and
//! statically links a bounded CPU-only XGBoost thermal-prediction worker.
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::{boxed::Box, string::String, vec::Vec};
use core::any::Any;
use std::sync::mpsc::Receiver;

use libertas::{
    LibertasDateTime, LibertasDevice, LibertasEndpoint, LibertasUser, LogLevel,
    NotificationArgument, NotificationImportance, libertas_data_read, libertas_data_write,
    libertas_log, libertas_notification_send, libertas_register_shutdown_handler,
    libertas_register_wakeup_callback, libertas_shutdown_complete, libertas_wake_up,
};
use libertas_macros::{
    LibertasAvroDecode, LibertasAvroEncode, LibertasExport, libertas_data_schema,
    libertas_string_resources,
};

mod machine_learning;
pub use machine_learning::*;

pub use libertas_weather::{
    BuildingHvacCurrentWeatherV1, BuildingHvacOutdoorAirQualityPeriodV1,
    BuildingHvacOutdoorAirQualityV1, BuildingHvacOutdoorConditionsV1,
    BuildingHvacPrecipitationKindV1, BuildingHvacWeatherChangeV1, BuildingHvacWeatherCursorV1,
    BuildingHvacWeatherForecastPeriodV1, BuildingHvacWeatherForecastV1,
    BuildingHvacWeatherHistoryPeriodV1, BuildingHvacWeatherHistoryV1,
    BuildingHvacWeatherIncrementalReportV1, BuildingHvacWeatherProtocolV1,
    BuildingHvacWeatherRecoveryErrorV1, BuildingHvacWeatherRecoveryV1,
    BuildingHvacWeatherResetReasonV1, BuildingHvacWeatherSectionV1, BuildingHvacWeatherSnapshotV1,
    BuildingHvacWeatherTimeRangeV1,
};

/// Maximum configured rooms
/// The largest room list accepted by the V1 controller and its bounded runtime
/// data structures.
pub const BUILDING_HVAC_MAX_ROOMS: usize = 64;

/// Maximum configured thermostats
/// The largest Matter thermostat list accepted by the V1 controller and its
/// bounded Matter subscription.
pub const BUILDING_HVAC_MAX_THERMOSTATS: usize = 16;

/// Maximum sensor stations per room
/// The largest number of indoor Matter environmental stations one room may use.
/// Every station includes temperature and may add humidity and air quality.
pub const BUILDING_HVAC_MAX_SENSORS_PER_ROOM: usize = 8;

/// Maximum air measurements
/// The number of standard Matter concentration-measurement cluster kinds the
/// V1 controller can expose from one indoor or outdoor Air Quality Sensor.
pub const BUILDING_HVAC_MAX_AIR_MEASUREMENTS: usize = 10;

/// Room runtime maximum wait interval
/// The maximum number of seconds a subscribed room client waits after a
/// snapshot or change report before retrying `GetRoomV1`.
pub const BUILDING_HVAC_ROOM_MAXIMUM_WAIT_INTERVAL_SECONDS: u32 = 5 * 60;

/// Maximum room plan periods
/// The largest calculated per-room schedule exposed in one runtime snapshot.
/// Ninety-six 15-minute periods cover 24 hours.
pub const BUILDING_HVAC_MAX_ROOM_PLAN_PERIODS: usize = 96;

/// Maximum persisted room condition periods
/// The largest recent-condition history retained for one room. Ninety-six
/// 15-minute periods preserve one day across a controller restart.
pub const BUILDING_HVAC_MAX_PERSISTED_ROOM_CONDITION_PERIODS: usize = 96;

/// Cross-zone learning half-life
/// The number of seconds after which completed old observation weight is
/// halved, allowing continuous learning to adapt to duct, damper, seasonal, and
/// equipment changes without forgetting useful evidence on every restart.
pub const BUILDING_HVAC_CROSS_ZONE_LEARNING_HALF_LIFE_SECONDS: u64 = 30 * 24 * 60 * 60;

/// Minimum cross-zone learning weight
/// The effective number of high-quality observations required before exposing a
/// learned coefficient for control planning.
pub const BUILDING_HVAC_CROSS_ZONE_MINIMUM_EFFECTIVE_SAMPLE_WEIGHT: f64 = 4.0;

/// Maximum urgent-notification recipients
/// The largest user list that receives time-sensitive HVAC warnings.
pub const BUILDING_HVAC_MAX_URGENT_NOTIFICATION_RECIPIENTS: usize = 16;

/// Maximum simultaneous urgent room conditions
/// The complete set of distinct V1 urgent HVAC conditions that one room can
/// expose and persist without duplicate entries.
pub const BUILDING_HVAC_MAX_URGENT_ROOM_CONDITIONS: usize = 5;

/// Urgent temperature confirmation interval
/// The number of continuous seconds a fresh room temperature must remain in the
/// freeze-risk or excessive-heat range before the controller sends a warning.
pub const BUILDING_HVAC_URGENT_TEMPERATURE_CONFIRMATION_SECONDS: u32 = 5 * 60;

/// Urgent recovery confirmation interval
/// The number of continuous seconds a condition must remain outside its
/// hysteresis boundary before the controller sends a recovery notification.
pub const BUILDING_HVAC_URGENT_RECOVERY_CONFIRMATION_SECONDS: u32 = 10 * 60;

/// Maximum urgent-evidence gap
/// A pending activation or recovery interval restarts when fresh evidence is
/// separated by more than this many seconds. Active warnings remain active
/// across such a gap because missing data is never proof of recovery.
pub const BUILDING_HVAC_URGENT_EVIDENCE_MAX_GAP_SECONDS: u32 = 90;

/// Temperature-control unavailable confirmation interval
/// The number of continuous seconds without trustworthy room temperature or
/// thermostat state before the controller sends an urgent warning.
pub const BUILDING_HVAC_CONTROL_UNAVAILABLE_CONFIRMATION_SECONDS: u32 = 10 * 60;

/// Ineffective heating or cooling observation interval
/// The minimum continuous equipment runtime used to decide that heating or
/// cooling is not restoring the room temperature.
pub const BUILDING_HVAC_NOT_RECOVERING_OBSERVATION_SECONDS: u32 = 60 * 60;

/// Cold-room ineffective-heating temperature
/// Heating-not-recovering evaluation is limited to rooms at or below this
/// temperature so ordinary slow comfort adjustments do not create warnings.
pub const BUILDING_HVAC_HEATING_NOT_RECOVERING_TEMPERATURE_CELSIUS: f32 = 15.0;

/// Hot-room ineffective-cooling temperature
/// Cooling-not-recovering evaluation is limited to rooms at or above this
/// temperature so ordinary slow comfort adjustments do not create warnings.
pub const BUILDING_HVAC_COOLING_NOT_RECOVERING_TEMPERATURE_CELSIUS: f32 = 30.0;

/// Minimum one-hour recovery temperature change
/// The room must move at least this many degrees Celsius toward recovery during
/// the one-hour observation interval to avoid a not-recovering warning.
pub const BUILDING_HVAC_MINIMUM_RECOVERY_CHANGE_CELSIUS: f32 = 0.5;

/// Minimum ineffective-control data coverage
/// The fraction of the one-hour observation interval that must contain fresh
/// room temperature and equipment-running evidence before judging recovery.
pub const BUILDING_HVAC_MINIMUM_RECOVERY_DATA_COVERAGE_NORMALIZED: f32 = 0.8;

/// Temperature fusion outlier distance
/// A fresh room sensor more than this many degrees Celsius from the median is
/// excluded from the fused control temperature while remaining visible in
/// station-level runtime data.
pub const BUILDING_HVAC_TEMPERATURE_FUSION_OUTLIER_CELSIUS: f32 = 2.0;

/// Humidity fusion outlier distance
/// A fresh humidity sensor more than this many percentage points from the
/// median is excluded from the fused room humidity.
pub const BUILDING_HVAC_HUMIDITY_FUSION_OUTLIER_PERCENT: f32 = 15.0;

/// Weather humidity consistency tolerance
/// Current weather is rejected for psychrometric use when relative humidity
/// differs from the dry-bulb/dew-point result by more than this many percentage
/// points.
pub const BUILDING_HVAC_WEATHER_HUMIDITY_CONSISTENCY_PERCENT: f32 = 15.0;

/// Maximum comfort preference setpoint adjustment
/// The normalized comfort-or-savings input shifts each heating and cooling
/// target by at most this many degrees Celsius before shared-zone arbitration.
pub const BUILDING_HVAC_MAX_COMFORT_SETPOINT_ADJUSTMENT_CELSIUS: f32 = 1.0;

/// Setpoint command comparison tolerance
/// A calculated Matter thermostat target within this many degrees Celsius of
/// the effective target is treated as already applied.
pub const BUILDING_HVAC_SETPOINT_COMMAND_TOLERANCE_CELSIUS: f32 = 0.05;

/// Urgent notification reminder interval
/// The minimum interval between repeated notifications for one unchanged active
/// room condition. A severity increase bypasses this interval.
pub const BUILDING_HVAC_URGENT_NOTIFICATION_REMINDER_SECONDS: u32 = 30 * 60;

/// Freeze-risk activation temperature
/// A fresh room temperature at or below this value for the confirmation
/// interval activates the freeze-risk warning.
pub const BUILDING_HVAC_FREEZE_RISK_TEMPERATURE_CELSIUS: f32 = 5.0;

/// Freeze-risk recovery temperature
/// A freeze-risk warning begins recovery only after fresh room temperature
/// reaches at least this value, providing hysteresis against repeated warnings.
pub const BUILDING_HVAC_FREEZE_RECOVERY_TEMPERATURE_CELSIUS: f32 = 7.0;

/// Excessive-heat activation temperature
/// A fresh room temperature at or above this value for the confirmation
/// interval activates the excessive-heat warning.
pub const BUILDING_HVAC_EXCESSIVE_HEAT_TEMPERATURE_CELSIUS: f32 = 35.0;

/// Excessive-heat recovery temperature
/// An excessive-heat warning begins recovery only after fresh room temperature
/// falls to at most this value.
pub const BUILDING_HVAC_EXCESSIVE_HEAT_RECOVERY_TEMPERATURE_CELSIUS: f32 = 32.0;

/// Smart building HVAC localized strings
/// Templates used by FormattedText runtime values. The encoded byte arrays
/// carry these resource identifiers and Notification-compatible typed
/// arguments; clients select the localized template before printf-style
/// rendering. Urgent temperature resources receive room `LiteralText` and
/// `UnitFloat("temperature-celsius")`; unavailable control receives room and
/// `UnitUnsigned("duration-seconds")`; not-recovering resources receive room,
/// duration, and temperature in that order. Recovery receives room,
/// condition-name `ResourceText`, and current temperature.
pub static APP_STRINGS: [(&str, &str); 17] = [
    (
        "HVAC_ROOM_STATUS",
        "Room status: %1$s. HVAC: %2$s. Air quality: %3$s.",
    ),
    ("HVAC_ROOM_SCHEDULE", "Calculated schedule: %1$s."),
    (
        "HVAC_CONTROL_REVISION_CONFLICT",
        "The room changed on another client. Review the current settings and retry.",
    ),
    (
        "HVAC_URGENT_FREEZE_RISK",
        "Urgent HVAC warning for %1$s: room temperature is %2$s. Check heating and protect plumbing. This is not a life-safety alarm.",
    ),
    (
        "HVAC_URGENT_EXCESSIVE_HEAT",
        "Urgent HVAC warning for %1$s: room temperature is %2$s. Check cooling and the room promptly. This is not a life-safety alarm.",
    ),
    (
        "HVAC_URGENT_CONTROL_UNAVAILABLE",
        "Urgent HVAC warning for %1$s: trustworthy temperature or thermostat data has been unavailable for %2$s. Check the sensors and thermostat.",
    ),
    (
        "HVAC_URGENT_HEATING_NOT_RECOVERING",
        "Urgent HVAC warning for %1$s: heating has not restored the room after %2$s; current temperature is %3$s. Check the heating system.",
    ),
    (
        "HVAC_URGENT_COOLING_NOT_RECOVERING",
        "Urgent HVAC warning for %1$s: cooling has not restored the room after %2$s; current temperature is %3$s. Check the cooling system.",
    ),
    (
        "HVAC_URGENT_CONDITION_RECOVERED",
        "HVAC warning cleared for %1$s: %2$s. Current temperature is %3$s.",
    ),
    ("HVAC_CONDITION_FREEZE_RISK", "freeze risk"),
    ("HVAC_CONDITION_EXCESSIVE_HEAT", "excessive heat"),
    (
        "HVAC_CONDITION_CONTROL_UNAVAILABLE",
        "temperature control unavailable",
    ),
    (
        "HVAC_CONDITION_HEATING_NOT_RECOVERING",
        "heating not recovering",
    ),
    (
        "HVAC_CONDITION_COOLING_NOT_RECOVERING",
        "cooling not recovering",
    ),
    (
        "HVAC_ROOM_URGENT_NOTIFICATION_STATE",
        "Urgent HVAC notification state for %1$s.",
    ),
    (
        "HVAC_ML_MODELS",
        "Accepted smart building HVAC thermal prediction models for %1$s.",
    ),
    (
        "HVAC_ML_SAMPLE",
        "Thermal learning sample history for %1$s.",
    ),
];

/// Matter thermostat device descriptor
/// Device Type Editor output selecting a standard Matter Thermostat logical
/// device.
#[cfg(test)]
const MATTER_THERMOSTAT_DEVICE_DESCRIPTOR: &str = "BQEBAYEGAA==";

/// Matter temperature sensor device descriptor
/// Device Type Editor output selecting a standard Matter Temperature Sensor
/// logical device.
#[cfg(test)]
const MATTER_TEMPERATURE_SENSOR_DEVICE_DESCRIPTOR: &str = "BQEBAYIGAA==";

/// Matter humidity sensor device descriptor
/// Device Type Editor output selecting a standard Matter Humidity Sensor
/// logical device.
#[cfg(test)]
const MATTER_HUMIDITY_SENSOR_DEVICE_DESCRIPTOR: &str = "BQEBAYcGAA==";

/// Matter air quality sensor device descriptor
/// Device Type Editor output selecting a standard Matter Air Quality Sensor
/// logical device. Its optional concentration clusters are discovered at
/// runtime.
#[cfg(test)]
const MATTER_AIR_QUALITY_SENSOR_DEVICE_DESCRIPTOR: &str = "BQEBASwA";

/// Room operating preference V1
/// Describes the heating and cooling demand a user permits for one room. The
/// physical thermostat remains responsible for equipment protection and for
/// enforcing capabilities it actually supports.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacRoomOperatingPreferenceV1 {
    /// Automatic heating and cooling
    /// The room may request heating or cooling to remain inside its preferred
    /// temperature band.
    Auto,
    /// Heating only
    /// The room may request heating but does not request cooling.
    Heat,
    /// Cooling only
    /// The room may request cooling but does not request heating.
    Cool,
    /// No room demand
    /// The room contributes no heating or cooling demand. This does not command
    /// shared physical HVAC equipment off when another room still needs it.
    Off,
}

/// Room control V1
/// Contains the writable user intent for one room. The controller validates it
/// against the associated thermostat's reported limits and shared-system
/// constraints before changing a physical setpoint.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacRoomControlV1 {
    /// Operating preference
    /// The heating and cooling demand modes this room may contribute.
    pub operating_preference: BuildingHvacRoomOperatingPreferenceV1,
    /// Preferred heating temperature
    /// The lowest preferred room temperature in degrees Celsius. It must be
    /// lower than `preferred_cooling_temperature_celsius`.
    #[libertas_number(min = -50, max = 100, step = 0.1)]
    pub preferred_heating_temperature_celsius: f32,
    /// Preferred cooling temperature
    /// The highest preferred room temperature in degrees Celsius. It must be
    /// higher than `preferred_heating_temperature_celsius`.
    #[libertas_number(min = -50, max = 100, step = 0.1)]
    pub preferred_cooling_temperature_celsius: f32,
    /// Comfort or savings
    /// A normalized room preference from -1.0 for stronger energy-cost
    /// optimization, through 0.0 for balanced operation, to 1.0 for stronger
    /// comfort optimization. Hard safety and temperature limits are unchanged.
    #[libertas_number(min = -1, max = 1, step = 0.05)]
    pub comfort_or_savings_normalized: f32,
}

impl Default for BuildingHvacRoomControlV1 {
    fn default() -> Self {
        Self {
            operating_preference: BuildingHvacRoomOperatingPreferenceV1::Auto,
            preferred_heating_temperature_celsius: 20.0,
            preferred_cooling_temperature_celsius: 24.0,
            comfort_or_savings_normalized: 0.0,
        }
    }
}

impl BuildingHvacRoomControlV1 {
    /// Valid room control
    /// Returns `true` when every numeric value is finite, the normalized
    /// preference and schema temperatures are in range, and the heating target
    /// is below the cooling target. Narrower physical thermostat limits require
    /// separate runtime validation.
    pub fn is_well_formed(&self) -> bool {
        self.preferred_heating_temperature_celsius.is_finite()
            && self.preferred_cooling_temperature_celsius.is_finite()
            && self.comfort_or_savings_normalized.is_finite()
            && (-50.0..=100.0).contains(&self.preferred_heating_temperature_celsius)
            && (-50.0..=100.0).contains(&self.preferred_cooling_temperature_celsius)
            && self.preferred_heating_temperature_celsius
                < self.preferred_cooling_temperature_celsius
            && (-1.0..=1.0).contains(&self.comfort_or_savings_normalized)
    }
}

/// Room data quality V1
/// Summarizes whether the controller has enough recent Matter reports to
/// calculate and safely apply supervisory room setpoints.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacRoomDataQualityV1 {
    /// Ready
    /// Required thermostat and temperature data are fresh.
    Ready,
    /// Degraded
    /// The room remains observable through a fallback source, but one or more
    /// configured devices are stale or unavailable.
    Degraded,
    /// Unavailable
    /// No trustworthy current room temperature or associated thermostat state
    /// is available, so the controller must not apply optimized setpoints.
    Unavailable,
}

/// Urgent HVAC condition V1
/// Identifies time-sensitive supervisory conditions that warrant direct user
/// attention. These warnings help protect comfort, equipment, and property;
/// they are not certified smoke, fire, carbon-monoxide, medical, or other
/// life-safety alarms.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacUrgentConditionV1 {
    /// Freeze risk
    /// Fresh room temperature remained at or below 5 degrees Celsius for at
    /// least five minutes.
    FreezeRisk,
    /// Excessive heat
    /// Fresh room temperature remained at or above 35 degrees Celsius for at
    /// least five minutes.
    ExcessiveHeat,
    /// Temperature control unavailable
    /// Trustworthy room temperature or associated thermostat state remained
    /// unavailable for at least ten minutes.
    TemperatureControlUnavailable,
    /// Heating not recovering
    /// A room at or below 15 degrees Celsius had at least 80 percent fresh
    /// temperature and heating-runtime coverage for one hour but warmed by less
    /// than 0.5 degrees Celsius.
    HeatingNotRecovering,
    /// Cooling not recovering
    /// A room at or above 30 degrees Celsius had at least 80 percent fresh
    /// temperature and cooling-runtime coverage for one hour but cooled by less
    /// than 0.5 degrees Celsius.
    CoolingNotRecovering,
}

impl BuildingHvacUrgentConditionV1 {
    /// Notification resource
    /// Returns the localized resource used when this condition becomes active
    /// or produces a bounded reminder.
    pub const fn notification_resource(self) -> &'static str {
        match self {
            Self::FreezeRisk => "HVAC_URGENT_FREEZE_RISK",
            Self::ExcessiveHeat => "HVAC_URGENT_EXCESSIVE_HEAT",
            Self::TemperatureControlUnavailable => "HVAC_URGENT_CONTROL_UNAVAILABLE",
            Self::HeatingNotRecovering => "HVAC_URGENT_HEATING_NOT_RECOVERING",
            Self::CoolingNotRecovering => "HVAC_URGENT_COOLING_NOT_RECOVERING",
        }
    }

    /// Condition-name resource
    /// Returns the localized short label supplied as a typed resource-text
    /// argument when a recovery notification is sent.
    pub const fn condition_name_resource(self) -> &'static str {
        match self {
            Self::FreezeRisk => "HVAC_CONDITION_FREEZE_RISK",
            Self::ExcessiveHeat => "HVAC_CONDITION_EXCESSIVE_HEAT",
            Self::TemperatureControlUnavailable => "HVAC_CONDITION_CONTROL_UNAVAILABLE",
            Self::HeatingNotRecovering => "HVAC_CONDITION_HEATING_NOT_RECOVERING",
            Self::CoolingNotRecovering => "HVAC_CONDITION_COOLING_NOT_RECOVERING",
        }
    }

    /// Default notification severity
    /// Uses severe delivery for immediately hazardous room temperatures and
    /// high delivery for loss or apparent failure of HVAC supervision.
    pub const fn severity(self) -> BuildingHvacUrgentNotificationSeverityV1 {
        match self {
            Self::FreezeRisk | Self::ExcessiveHeat => {
                BuildingHvacUrgentNotificationSeverityV1::Severe
            }
            Self::TemperatureControlUnavailable
            | Self::HeatingNotRecovering
            | Self::CoolingNotRecovering => BuildingHvacUrgentNotificationSeverityV1::High,
        }
    }
}

/// Urgent HVAC notification severity V1
/// Maps an application-specific urgency decision to the Libertas notification
/// framework. Severity describes delivery priority and does not turn a
/// supervisory HVAC warning into a certified life-safety alarm.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacUrgentNotificationSeverityV1 {
    /// High
    /// Requires prompt attention because temperature supervision or HVAC
    /// recovery appears unavailable.
    High,
    /// Severe
    /// Requires immediate attention because a confirmed room temperature can
    /// damage property or threaten occupant comfort.
    Severe,
}

impl BuildingHvacUrgentNotificationSeverityV1 {
    /// Libertas notification importance
    /// Converts the application severity to its notification-framework delivery
    /// importance.
    pub const fn notification_importance(self) -> NotificationImportance {
        match self {
            Self::High => NotificationImportance::AlertHigh,
            Self::Severe => NotificationImportance::AlertSevere,
        }
    }
}

/// Active urgent HVAC condition V1
/// Exposes one confirmed condition in a room runtime snapshot. A condition
/// remains active during its recovery-confirmation interval so stale or
/// oscillating data cannot falsely clear an urgent warning.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacActiveUrgentConditionV1 {
    /// Condition
    /// The confirmed time-sensitive HVAC condition.
    #[libertas_read_only]
    pub condition: BuildingHvacUrgentConditionV1,
    /// Severity
    /// The current Libertas delivery priority for this condition.
    #[libertas_read_only]
    pub severity: BuildingHvacUrgentNotificationSeverityV1,
    /// Active since
    /// The UTC time at which the continuously qualifying condition began, not
    /// the later time at which its confirmation interval expired.
    #[libertas_read_only]
    pub active_since: LibertasDateTime,
    /// Updated at
    /// The UTC time represented by the latest fresh evidence for the condition.
    #[libertas_read_only]
    pub updated_at: LibertasDateTime,
    /// Room temperature
    /// The latest fresh room temperature in degrees Celsius when the condition
    /// has a trustworthy measurement. It is absent for unavailable control.
    #[libertas_read_only]
    pub temperature_celsius: Option<f32>,
    /// Last notification time
    /// The UTC time when the controller most recently submitted this condition
    /// to the Libertas notification framework. It is absent until the first
    /// notification attempt has been persisted and submitted.
    #[libertas_read_only]
    pub last_notification_at: Option<LibertasDateTime>,
}

impl BuildingHvacActiveUrgentConditionV1 {
    /// Valid active urgent condition
    /// Returns `true` when severity agrees with the condition, timestamps are
    /// ordered, and any supplied temperature is finite.
    pub fn is_well_formed(&self) -> bool {
        self.severity == self.condition.severity()
            && self.active_since <= self.updated_at
            && self
                .last_notification_at
                .is_none_or(|sent_at| (self.active_since..=self.updated_at).contains(&sent_at))
            && self.temperature_celsius.is_none_or(f32::is_finite)
            && (self.condition == BuildingHvacUrgentConditionV1::TemperatureControlUnavailable
                || self.temperature_celsius.is_some())
    }
}

/// Urgent-condition tracking phase V1
/// Persists activation and recovery debounce across application restarts so a
/// restart neither immediately clears a warning nor creates a burst of
/// duplicate notifications.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacUrgentConditionPhaseV1 {
    /// Activation pending
    /// The condition currently qualifies but has not yet remained continuous
    /// for its activation-confirmation interval.
    ActivationPending,
    /// Active
    /// The condition is confirmed and its urgent warning remains active.
    Active,
    /// Recovery pending
    /// Fresh evidence crossed the recovery hysteresis boundary, but the
    /// recovery-confirmation interval has not yet completed.
    RecoveryPending,
}

/// Persisted urgent-condition tracker V1
/// Stores one room condition's debounce, reminder, recovery, and last qualifying
/// temperature evidence. Current sensor truth remains in the independently
/// persisted sensor record.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacPersistedUrgentConditionV1 {
    /// Condition
    /// The unique condition tracked by this room record.
    pub condition: BuildingHvacUrgentConditionV1,
    /// Tracking phase
    /// Whether the condition is awaiting activation, active, or awaiting
    /// confirmed recovery.
    pub phase: BuildingHvacUrgentConditionPhaseV1,
    /// Condition started at
    /// The UTC time at which evidence first continuously qualified for this
    /// occurrence.
    pub condition_started_at: LibertasDateTime,
    /// Phase started at
    /// The UTC time at which the current tracking phase began.
    pub phase_started_at: LibertasDateTime,
    /// Updated at
    /// The UTC time represented by the latest fresh evidence applied to this
    /// tracker.
    pub updated_at: LibertasDateTime,
    /// Last qualifying temperature
    /// The latest fresh room temperature associated with this condition. It is
    /// absent only while temperature control is unavailable. Retaining it lets
    /// a restart preserve the last confirmed evidence without treating it as a
    /// new current sensor reading.
    pub last_temperature_celsius: Option<f32>,
    /// Last notification time
    /// The UTC time when this occurrence was most recently submitted to the
    /// notification framework. It is absent before activation.
    pub last_notification_at: Option<LibertasDateTime>,
}

impl BuildingHvacPersistedUrgentConditionV1 {
    /// Valid persisted urgent tracker
    /// Returns `true` when its phase timestamps and optional last-notification
    /// time form one ordered occurrence. Activation-pending conditions have not
    /// yet sent a notification.
    pub fn is_well_formed(&self) -> bool {
        self.condition_started_at <= self.phase_started_at
            && self.phase_started_at <= self.updated_at
            && self.last_notification_at.is_none_or(|sent_at| {
                (self.condition_started_at..=self.updated_at).contains(&sent_at)
            })
            && self.last_temperature_celsius.is_none_or(f32::is_finite)
            && (self.condition == BuildingHvacUrgentConditionV1::TemperatureControlUnavailable
                || self.last_temperature_celsius.is_some())
            && (self.phase != BuildingHvacUrgentConditionPhaseV1::ActivationPending
                || self.last_notification_at.is_none())
    }
}

/// Sensor air quality V1
/// Maps the standard Matter Air Quality cluster's overall classification. This
/// classification is descriptive sensor output; stale or missing data is never
/// evidence that outdoor air is safe for ventilation.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacAirQualityV1 {
    /// Unknown
    /// The sensor cannot classify current air quality.
    Unknown,
    /// Good
    /// The sensor classifies current air quality as good.
    Good,
    /// Fair
    /// The sensor classifies current air quality as fair.
    Fair,
    /// Moderate
    /// The sensor classifies current air quality as moderate.
    Moderate,
    /// Poor
    /// The sensor classifies current air quality as poor.
    Poor,
    /// Very poor
    /// The sensor classifies current air quality as very poor.
    VeryPoor,
    /// Extremely poor
    /// The sensor classifies current air quality as extremely poor.
    ExtremelyPoor,
}

/// Sensor concentration level V1
/// Maps the optional qualitative level reported by one standard Matter
/// concentration-measurement cluster.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacConcentrationLevelV1 {
    /// Unknown
    /// The sensor cannot classify this concentration.
    Unknown,
    /// Low
    /// The sensor classifies this concentration as low.
    Low,
    /// Medium
    /// The sensor classifies this concentration as medium.
    Medium,
    /// High
    /// The sensor classifies this concentration as high.
    High,
    /// Critical
    /// The sensor classifies this concentration as critical.
    Critical,
}

/// Sensor air measurement kind V1
/// Identifies one optional standard Matter concentration-measurement cluster
/// discovered on the configured outdoor Air Quality Sensor. The controller
/// does not require every sensor to implement every kind.
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
pub enum BuildingHvacAirMeasurementKindV1 {
    /// Carbon dioxide
    /// Carbon-dioxide concentration, commonly used to assess ventilation.
    CarbonDioxide,
    /// Carbon monoxide
    /// Carbon-monoxide concentration. This supervisory value never replaces a
    /// certified life-safety alarm.
    CarbonMonoxide,
    /// Nitrogen dioxide
    /// Nitrogen-dioxide concentration.
    NitrogenDioxide,
    /// Ozone
    /// Ozone concentration.
    Ozone,
    /// PM1
    /// Particulate matter with aerodynamic diameter at most 1 micrometer.
    ParticulateMatter1,
    /// PM2.5
    /// Particulate matter with aerodynamic diameter at most 2.5 micrometers.
    ParticulateMatter2_5,
    /// PM10
    /// Particulate matter with aerodynamic diameter at most 10 micrometers.
    ParticulateMatter10,
    /// Formaldehyde
    /// Formaldehyde concentration.
    Formaldehyde,
    /// Total volatile organic compounds
    /// Total volatile-organic-compound concentration.
    TotalVolatileOrganicCompounds,
    /// Radon
    /// Radon activity concentration.
    Radon,
}

/// Sensor air measurement unit V1
/// Preserves the standard Matter unit reported with a concentration. Consumers
/// must interpret `measured_value_in_reported_unit` together with this unit and
/// must not convert mass concentration to a mixing ratio without the required
/// gas and atmospheric context.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacAirMeasurementUnitV1 {
    /// Parts per million
    /// A mixing ratio in parts per million.
    PartsPerMillion,
    /// Parts per billion
    /// A mixing ratio in parts per billion.
    PartsPerBillion,
    /// Parts per trillion
    /// A mixing ratio in parts per trillion.
    PartsPerTrillion,
    /// Milligrams per cubic meter
    /// A mass concentration in milligrams per cubic meter.
    MilligramsPerCubicMeter,
    /// Micrograms per cubic meter
    /// A mass concentration in micrograms per cubic meter.
    MicrogramsPerCubicMeter,
    /// Nanograms per cubic meter
    /// A mass concentration in nanograms per cubic meter.
    NanogramsPerCubicMeter,
    /// Picograms per cubic meter
    /// A mass concentration in picograms per cubic meter.
    PicogramsPerCubicMeter,
    /// Becquerels per cubic meter
    /// A radioactive-activity concentration in becquerels per cubic meter.
    BecquerelsPerCubicMeter,
}

/// Sensor air measurement V1
/// Carries one current concentration discovered from an optional cluster on
/// the configured Matter Air Quality Sensor. The runtime accepts only finite,
/// nonnegative measurements whose Matter measurement medium is air.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacAirMeasurementV1 {
    /// Measurement kind
    /// The pollutant or particulate represented by this value.
    #[libertas_read_only]
    pub kind: BuildingHvacAirMeasurementKindV1,
    /// Measured value in reported unit
    /// The finite nonnegative numeric value in `reported_unit`.
    #[libertas_number(min = 0)]
    #[libertas_read_only]
    pub measured_value_in_reported_unit: f32,
    /// Reported unit
    /// The standard Matter measurement unit accompanying this value.
    #[libertas_read_only]
    pub reported_unit: BuildingHvacAirMeasurementUnitV1,
    /// Concentration level
    /// The optional standard Matter qualitative classification for this
    /// particular concentration.
    #[libertas_read_only]
    pub level: Option<BuildingHvacConcentrationLevelV1>,
}

impl BuildingHvacAirMeasurementV1 {
    /// Valid sensor air measurement
    /// Returns `true` when the value is finite and nonnegative. The Matter
    /// listener must separately reject measurements whose reported medium is
    /// not air before constructing this runtime value.
    pub fn is_well_formed(&self) -> bool {
        self.measured_value_in_reported_unit.is_finite()
            && self.measured_value_in_reported_unit >= 0.0
    }
}

/// Temperature reading V1
/// Caches one accepted reading from a standard Matter Temperature Sensor
/// logical device.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacTemperatureReadingV1 {
    /// Observed at
    /// The UTC time represented by the Matter temperature report.
    #[libertas_read_only]
    pub observed_at: LibertasDateTime,
    /// Valid until
    /// The exclusive UTC deadline after which the controller treats this
    /// reading as stale.
    #[libertas_read_only]
    pub valid_until: LibertasDateTime,
    /// Temperature
    /// Measured air temperature in degrees Celsius.
    #[libertas_number(min = -100, max = 100, step = 0.01)]
    #[libertas_read_only]
    pub temperature_celsius: f32,
}

impl BuildingHvacTemperatureReadingV1 {
    /// Valid temperature reading
    /// Returns `true` when the freshness interval is nonempty and the
    /// temperature is finite and inside the V1 outdoor range.
    pub fn is_well_formed(&self) -> bool {
        self.valid_until > self.observed_at
            && self.temperature_celsius.is_finite()
            && (-100.0..=100.0).contains(&self.temperature_celsius)
    }
}

/// Humidity reading V1
/// Caches one accepted reading from a standard Matter Humidity Sensor logical
/// device.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacHumidityReadingV1 {
    /// Observed at
    /// The UTC time represented by the Matter humidity report.
    #[libertas_read_only]
    pub observed_at: LibertasDateTime,
    /// Valid until
    /// The exclusive UTC deadline after which the controller treats this
    /// reading as stale.
    #[libertas_read_only]
    pub valid_until: LibertasDateTime,
    /// Relative humidity
    /// Measured relative humidity as a percentage.
    #[libertas_number(min = 0, max = 100, step = 0.01)]
    #[libertas_read_only]
    pub relative_humidity_percent: f32,
}

impl BuildingHvacHumidityReadingV1 {
    /// Valid humidity reading
    /// Returns `true` when the freshness interval is nonempty and relative
    /// humidity is finite and from 0 through 100 percent.
    pub fn is_well_formed(&self) -> bool {
        self.valid_until > self.observed_at
            && self.relative_humidity_percent.is_finite()
            && (0.0..=100.0).contains(&self.relative_humidity_percent)
    }
}

/// Sensor air quality reading V1
/// Caches the last accepted overall classification and every supported current
/// concentration from the optional Matter Air Quality Sensor. Optional cluster
/// discovery makes PM2.5, carbon dioxide, and other supported measurements
/// runtime capabilities rather than configuration flags.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacAirQualityReadingV1 {
    /// Observed at
    /// The UTC time represented by the accepted Air Quality Sensor data.
    #[libertas_read_only]
    pub observed_at: LibertasDateTime,
    /// Valid until
    /// The exclusive UTC deadline after which the controller treats all values
    /// in this reading as stale.
    #[libertas_read_only]
    pub valid_until: LibertasDateTime,
    /// Overall air quality
    /// The optional standard Matter overall classification.
    #[libertas_read_only]
    pub overall_air_quality: Option<BuildingHvacAirQualityV1>,
    /// Concentration measurements
    /// Current values from supported standard Matter concentration clusters,
    /// ordered by measurement kind with at most one value per kind. Missing
    /// kinds are unsupported, unavailable, or stale and are never inferred safe.
    /// ----
    /// Concentration measurement
    /// One typed value including its sensor-reported unit.
    #[libertas_size(max = 10)]
    #[libertas_read_only]
    pub measurements: Vec<BuildingHvacAirMeasurementV1>,
}

impl BuildingHvacAirQualityReadingV1 {
    /// Query sensor air measurement
    /// Returns the current accepted measurement for a supported kind. A missing
    /// result means the cluster is unsupported, unavailable, invalid, or stale;
    /// callers must not interpret absence as a safe concentration.
    pub fn measurement(
        &self,
        kind: BuildingHvacAirMeasurementKindV1,
    ) -> Option<BuildingHvacAirMeasurementV1> {
        self.measurements
            .iter()
            .copied()
            .find(|measurement| measurement.kind == kind)
    }

    /// Valid sensor air quality reading
    /// Returns `true` when the freshness interval is nonempty, the bounded
    /// concentration list contains only valid values, and measurement kinds are
    /// in strict enum order without duplicates.
    pub fn is_well_formed(&self) -> bool {
        self.valid_until > self.observed_at
            && self.measurements.len() <= BUILDING_HVAC_MAX_AIR_MEASUREMENTS
            && self
                .measurements
                .iter()
                .all(BuildingHvacAirMeasurementV1::is_well_formed)
            && self
                .measurements
                .windows(2)
                .all(|pair| pair[0].kind < pair[1].kind)
    }
}

/// Local outdoor sensor state V1
/// Exposes the independently available sections of the optional outdoor Matter
/// station. `None` for this whole value means no station is configured; a
/// present state with a missing section means that section is unavailable or
/// has no accepted reading.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacLocalOutdoorSensorStateV1 {
    /// Temperature
    /// Last accepted reading from the station's required temperature sensor.
    #[libertas_read_only]
    pub temperature: Option<BuildingHvacTemperatureReadingV1>,
    /// Relative humidity
    /// Last accepted reading from the station's optional humidity sensor.
    #[libertas_read_only]
    pub humidity: Option<BuildingHvacHumidityReadingV1>,
    /// Air quality
    /// Last accepted reading from the station's optional Air Quality Sensor and
    /// the concentration clusters it supports at runtime.
    #[libertas_read_only]
    pub air_quality: Option<BuildingHvacAirQualityReadingV1>,
}

/// Indoor sensor state V1
/// Exposes independently fresh readings from one configured indoor Matter
/// sensor station. Temperature identifies the station; optional humidity and
/// Air Quality Sensor logical devices may report on different schedules.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacIndoorSensorStateV1 {
    /// Temperature sensor
    /// The station's required Matter Temperature Sensor logical device.
    #[libertas_read_only]
    pub temperature_sensor: LibertasDevice,
    /// Temperature
    /// The last accepted reading from `temperature_sensor`.
    #[libertas_read_only]
    pub temperature: Option<BuildingHvacTemperatureReadingV1>,
    /// Humidity sensor
    /// The optional Matter Humidity Sensor logical device paired with this
    /// station.
    #[libertas_read_only]
    pub humidity_sensor: Option<LibertasDevice>,
    /// Relative humidity
    /// The last accepted reading from `humidity_sensor`.
    #[libertas_read_only]
    pub humidity: Option<BuildingHvacHumidityReadingV1>,
    /// Air quality sensor
    /// The optional Matter Air Quality Sensor logical device paired with this
    /// station.
    #[libertas_read_only]
    pub air_quality_sensor: Option<LibertasDevice>,
    /// Air quality
    /// The last accepted overall classification and runtime-discovered
    /// concentration measurements from `air_quality_sensor`.
    #[libertas_read_only]
    pub air_quality: Option<BuildingHvacAirQualityReadingV1>,
}

impl BuildingHvacIndoorSensorStateV1 {
    /// Query indoor air measurement
    /// Returns this station's accepted value for one air measurement kind.
    /// `None` means its Air Quality Sensor or cluster is absent, unavailable,
    /// invalid, or stale and must not be interpreted as a safe value.
    pub fn air_measurement(
        &self,
        kind: BuildingHvacAirMeasurementKindV1,
    ) -> Option<BuildingHvacAirMeasurementV1> {
        self.air_quality.as_ref()?.measurement(kind)
    }

    /// Valid indoor sensor state
    /// Returns `true` when each present reading is valid and no reading is
    /// attached to a capability absent from this configured station.
    pub fn is_well_formed(&self) -> bool {
        self.temperature
            .is_none_or(|reading| reading.is_well_formed())
            && self
                .humidity
                .as_ref()
                .is_none_or(BuildingHvacHumidityReadingV1::is_well_formed)
            && (self.humidity_sensor.is_some() || self.humidity.is_none())
            && self
                .air_quality
                .as_ref()
                .is_none_or(BuildingHvacAirQualityReadingV1::is_well_formed)
            && (self.air_quality_sensor.is_some() || self.air_quality.is_none())
    }
}

/// Room HVAC activity V1
/// Describes the observed or inferred current activity affecting one room.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacRoomActivityV1 {
    /// Unknown activity
    /// The associated thermostat has not reported a trustworthy running state.
    Unknown,
    /// Idle
    /// The associated thermostat reports no active heating or cooling.
    Idle,
    /// Heating
    /// The associated thermostat reports active heating.
    Heating,
    /// Cooling
    /// The associated thermostat reports active cooling.
    Cooling,
    /// Fan only
    /// The associated thermostat reports fan operation without active heating
    /// or cooling.
    FanOnly,
}

/// Room observed state V1
/// Contains read-only current room measurements, sensor availability, physical
/// thermostat association, and the effective setpoints chosen after arbitration
/// with other rooms sharing that thermostat.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacRoomObservedStateV1 {
    /// Data quality
    /// Whether current room state is ready, degraded, or unavailable.
    #[libertas_read_only]
    pub data_quality: BuildingHvacRoomDataQualityV1,
    /// Observed at
    /// The UTC time represented by the latest accepted room state. It is absent
    /// until wall-clock time and trustworthy device reports are both available.
    #[libertas_read_only]
    pub observed_at: Option<LibertasDateTime>,
    /// Room temperature
    /// The robust fused room temperature in degrees Celsius. It is absent when
    /// no configured temperature sensor has a fresh valid value.
    #[libertas_read_only]
    pub temperature_celsius: Option<f32>,
    /// Room relative humidity
    /// The robust fused room relative humidity as a percentage. It is absent
    /// when no configured humidity sensor has a fresh valid value.
    #[libertas_read_only]
    pub relative_humidity_percent: Option<f32>,
    /// Effective heating setpoint
    /// The heating setpoint in degrees Celsius currently requested from the
    /// shared physical thermostat after room arbitration. It is absent before
    /// the thermostat is ready or when the room requests no heating demand.
    #[libertas_read_only]
    pub effective_heating_setpoint_celsius: Option<f32>,
    /// Effective cooling setpoint
    /// The cooling setpoint in degrees Celsius currently requested from the
    /// shared physical thermostat after room arbitration. It is absent before
    /// the thermostat is ready or when the room requests no cooling demand.
    #[libertas_read_only]
    pub effective_cooling_setpoint_celsius: Option<f32>,
    /// HVAC activity
    /// The current heating, cooling, fan, idle, or unknown activity reported by
    /// the associated physical thermostat.
    #[libertas_read_only]
    pub activity: BuildingHvacRoomActivityV1,
    /// Physical thermostat
    /// The configured Matter Thermostat logical device that serves this room.
    #[libertas_read_only]
    pub physical_thermostat: LibertasDevice,
    /// Indoor sensor states
    /// Current independently available readings for every configured sensor
    /// station in this room. This is the runtime query surface for station-level
    /// PM2.5, carbon dioxide, and other supported air measurements.
    /// ----
    /// Indoor sensor state
    /// One required temperature sensor and its optional humidity and air-quality
    /// capabilities.
    #[libertas_size(min = 1, max = 8)]
    #[libertas_read_only]
    pub sensor_states: Vec<BuildingHvacIndoorSensorStateV1>,
    /// Fresh temperature sensors
    /// The number of configured room temperature sensors currently contributing
    /// trustworthy values.
    #[libertas_read_only]
    pub fresh_temperature_sensor_count: u16,
    /// Configured temperature sensors
    /// The total number of Matter Temperature Sensor logical devices configured
    /// for this room.
    #[libertas_read_only]
    pub configured_temperature_sensor_count: u16,
    /// Fresh humidity sensors
    /// The number of configured room humidity sensors currently contributing
    /// trustworthy values.
    #[libertas_read_only]
    pub fresh_humidity_sensor_count: u16,
    /// Configured humidity sensors
    /// The total number of Matter Humidity Sensor logical devices configured
    /// for this room.
    #[libertas_read_only]
    pub configured_humidity_sensor_count: u16,
}

/// Outdoor air analytics V1
/// Contains psychrometric values derived from one fresh, internally consistent
/// building-HVAC current-weather section. These values support sensible and
/// latent load calculations without persisting redundant weather fields.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacOutdoorAirAnalyticsV1 {
    /// Weather valid time
    /// The provider time represented by the source current conditions.
    #[libertas_read_only]
    pub weather_valid_at: LibertasDateTime,
    /// Humidity ratio
    /// Kilograms of water vapor per kilogram of dry air, derived from dew point
    /// and surface pressure.
    #[libertas_number(min = 0)]
    #[libertas_read_only]
    pub humidity_ratio_kilograms_water_per_kilogram_dry_air: f32,
    /// Moist-air enthalpy
    /// Approximate kilojoules per kilogram of dry air, derived from dry-bulb
    /// temperature and humidity ratio.
    #[libertas_read_only]
    pub moist_air_enthalpy_kilojoules_per_kilogram_dry_air: f32,
    /// Wet-bulb temperature
    /// Approximate thermodynamic wet-bulb temperature in degrees Celsius,
    /// solved from pressure and the derived humidity ratio.
    #[libertas_read_only]
    pub wet_bulb_temperature_celsius: f32,
}

impl BuildingHvacOutdoorAirAnalyticsV1 {
    /// Valid outdoor air analytics
    /// Returns `true` when all derived values are finite, humidity ratio is
    /// nonnegative, and wet bulb does not exceed dry bulb beyond numeric
    /// tolerance.
    pub fn is_well_formed_for(&self, dry_bulb_temperature_celsius: f32) -> bool {
        self.humidity_ratio_kilograms_water_per_kilogram_dry_air
            .is_finite()
            && self.humidity_ratio_kilograms_water_per_kilogram_dry_air >= 0.0
            && self
                .moist_air_enthalpy_kilojoules_per_kilogram_dry_air
                .is_finite()
            && self.wet_bulb_temperature_celsius.is_finite()
            && dry_bulb_temperature_celsius.is_finite()
            && self.wet_bulb_temperature_celsius <= dry_bulb_temperature_celsius + 0.05
    }
}

/// Room statistics V1
/// Contains read-only comfort and sensor-availability statistics for one room
/// over a documented half-open collection window. Shared equipment runtime and
/// energy are not attributed to individual rooms without an explicit metering
/// model.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacRoomStatisticsV1 {
    /// Window start
    /// The inclusive UTC start of the statistics collection window.
    #[libertas_read_only]
    pub starts_at: LibertasDateTime,
    /// Window end
    /// The exclusive UTC end of the statistics collection window.
    #[libertas_read_only]
    pub ends_before: LibertasDateTime,
    /// Temperature samples
    /// The number of accepted fused room-temperature samples in the window.
    #[libertas_read_only]
    pub temperature_sample_count: u64,
    /// Minimum temperature
    /// The lowest accepted fused room temperature in degrees Celsius.
    #[libertas_read_only]
    pub minimum_temperature_celsius: f32,
    /// Mean temperature
    /// The time-weighted mean accepted fused room temperature in degrees
    /// Celsius.
    #[libertas_read_only]
    pub mean_temperature_celsius: f32,
    /// Maximum temperature
    /// The highest accepted fused room temperature in degrees Celsius.
    #[libertas_read_only]
    pub maximum_temperature_celsius: f32,
    /// Temperature data availability
    /// The number of seconds in the window for which a trustworthy fused room
    /// temperature was available.
    #[libertas_time_interval]
    #[libertas_read_only]
    pub temperature_data_available_seconds: u64,
    /// Below-heating comfort
    /// Accumulated temperature deficit below the active preferred heating
    /// target, in degree-minutes Celsius.
    #[libertas_number(min = 0)]
    #[libertas_read_only]
    pub below_heating_comfort_degree_minutes_celsius: f32,
    /// Above-cooling comfort
    /// Accumulated temperature excess above the active preferred cooling target,
    /// in degree-minutes Celsius.
    #[libertas_number(min = 0)]
    #[libertas_read_only]
    pub above_cooling_comfort_degree_minutes_celsius: f32,
    /// Humidity samples
    /// The number of accepted fused room-humidity samples in the window.
    #[libertas_read_only]
    pub humidity_sample_count: u64,
    /// Mean relative humidity
    /// The time-weighted mean accepted fused room relative humidity percentage.
    /// It is absent when the room has no humidity samples in the window.
    #[libertas_read_only]
    pub mean_relative_humidity_percent: Option<f32>,
    /// Heating activity
    /// The number of seconds in the window during which the associated
    /// thermostat reported active heating. Shared equipment time is descriptive
    /// and is not an allocation of energy consumption to this room.
    #[libertas_time_interval]
    #[libertas_read_only]
    pub heating_active_seconds: u64,
    /// Cooling activity
    /// The number of seconds in the window during which the associated
    /// thermostat reported active cooling. Shared equipment time is descriptive
    /// and is not an allocation of energy consumption to this room.
    #[libertas_time_interval]
    #[libertas_read_only]
    pub cooling_active_seconds: u64,
    /// Fan-only activity
    /// The number of seconds in the window during which the associated
    /// thermostat reported fan operation without heating or cooling.
    #[libertas_time_interval]
    #[libertas_read_only]
    pub fan_only_active_seconds: u64,
}

/// Room plan reason V1
/// Explains the principal reason for one calculated room schedule period.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacRoomPlanReasonV1 {
    /// Room comfort
    /// The period follows the room's writable comfort intent.
    RoomComfort,
    /// Weather preconditioning
    /// The controller shifts heating or cooling ahead of forecast outdoor load.
    WeatherPreconditioning,
    /// Low-cost preconditioning
    /// The controller shifts conditioning into a lower-cost utility period.
    LowCostPreconditioning,
    /// High-cost reduction
    /// The controller reduces discretionary conditioning during a higher-cost
    /// utility period without crossing hard comfort limits.
    HighCostReduction,
    /// Shared thermostat arbitration
    /// Multiple rooms sharing one thermostat require one reconciled physical
    /// setpoint.
    SharedThermostatArbitration,
    /// Degraded fallback
    /// Missing or stale optimization inputs require conservative local
    /// operation.
    DegradedFallback,
}

/// Room plan period V1
/// Contains the read-only effective room targets calculated for one future
/// period.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacRoomPlanPeriodV1 {
    /// Start time
    /// The inclusive UTC time at which this calculated period begins.
    #[libertas_read_only]
    pub starts_at: LibertasDateTime,
    /// Duration
    /// The length of the calculated period in seconds.
    #[libertas_time_interval]
    #[libertas_read_only]
    pub duration_seconds: u32,
    /// Heating setpoint
    /// The effective heating setpoint in degrees Celsius for the period. It is
    /// absent when this room contributes no heating demand.
    #[libertas_read_only]
    pub heating_setpoint_celsius: Option<f32>,
    /// Cooling setpoint
    /// The effective cooling setpoint in degrees Celsius for the period. It is
    /// absent when this room contributes no cooling demand.
    #[libertas_read_only]
    pub cooling_setpoint_celsius: Option<f32>,
    /// Plan reason
    /// The principal reason these effective targets differ from or preserve the
    /// room's writable comfort intent.
    #[libertas_read_only]
    pub reason: BuildingHvacRoomPlanReasonV1,
}

/// Room plan V1
/// Contains the current read-only calculated schedule for one room. It exposes
/// user-visible decisions without claiming that rooms sharing one physical
/// thermostat can be actuated independently.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacRoomPlanV1 {
    /// Formatted schedule
    /// Notification-compatible Avro bytes containing a localized string
    /// resource and typed printf-style arguments summarizing the calculated
    /// periods, setpoints, and principal reasons. It is a derived presentation
    /// of `periods`; clients performing calculations must use the structured
    /// fields below. Do not Base64-wrap this byte array.
    #[libertas_formatted_text]
    #[libertas_size(max = 8192)]
    #[libertas_read_only]
    pub formatted_schedule: Vec<u8>,
    /// Calculated at
    /// The UTC time when the controller produced this complete plan.
    #[libertas_read_only]
    pub calculated_at: LibertasDateTime,
    /// Valid until
    /// The exclusive UTC deadline after which the plan must be recalculated.
    #[libertas_read_only]
    pub valid_until: LibertasDateTime,
    /// Plan periods
    /// Ordered, non-overlapping future periods covering no more than 24 hours.
    /// ----
    /// Plan period
    /// One calculated room target period and its reason.
    #[libertas_size(max = 96)]
    #[libertas_read_only]
    pub periods: Vec<BuildingHvacRoomPlanPeriodV1>,
}

/// Room control error V1
/// Explains why a room-control replacement was not accepted.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacRoomControlErrorV1 {
    /// Revision conflict
    /// Another client changed the room after the supplied expected revision.
    RevisionConflict,
    /// Invalid temperature band
    /// The heating target is not lower than the cooling target or a temperature
    /// value is not finite.
    InvalidTemperatureBand,
    /// Invalid normalized preference
    /// The comfort-or-savings value is not finite or lies outside -1.0 through
    /// 1.0.
    InvalidNormalizedPreference,
    /// Unsupported operating preference
    /// The associated physical thermostat cannot support the requested heating
    /// or cooling mode.
    UnsupportedOperatingPreference,
    /// Temporarily unavailable
    /// Required persistent or thermostat state is temporarily unavailable.
    TemporarilyUnavailable,
}

/// Cross-zone influence V1
/// Exposes the learned read-only effect of one other thermostat-zone operating
/// while this room's own thermostat-zone is inactive. The values are
/// supervisory predictions, never equipment or life-safety limits.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacCrossZoneInfluenceV1 {
    /// Source thermostat
    /// The other Matter thermostat whose heating or cooling calls create the
    /// learned passive effect in this room.
    #[libertas_read_only]
    pub source_thermostat: LibertasDevice,
    /// Heating effect
    /// Predicted room temperature rise in degrees Celsius per hour of source
    /// thermostat heating. It is absent until sufficient identifiable evidence
    /// exists.
    #[libertas_read_only]
    pub heating_temperature_rise_celsius_per_runtime_hour: Option<f32>,
    /// Heating confidence
    /// Normalized confidence from 0.0 through 1.0 in the heating-effect estimate.
    #[libertas_number(min = 0, max = 1)]
    #[libertas_read_only]
    pub heating_confidence_normalized: f32,
    /// Cooling effect
    /// Predicted room temperature drop in degrees Celsius per hour of source
    /// thermostat cooling. It is absent until sufficient identifiable evidence
    /// exists.
    #[libertas_read_only]
    pub cooling_temperature_drop_celsius_per_runtime_hour: Option<f32>,
    /// Cooling confidence
    /// Normalized confidence from 0.0 through 1.0 in the cooling-effect estimate.
    #[libertas_number(min = 0, max = 1)]
    #[libertas_read_only]
    pub cooling_confidence_normalized: f32,
    /// Learned at
    /// The UTC time of the latest accepted observation for either mode. It is
    /// absent before the first accepted observation.
    #[libertas_read_only]
    pub learned_at: Option<LibertasDateTime>,
}

/// Building HVAC room protocol V1
/// Defines every externally visible runtime request, response, and subscription
/// report for one room. The configured endpoint identifies the room, so messages
/// never carry a reorder-sensitive room-array index.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum BuildingHvacRoomProtocolV1 {
    /// Get room V1
    /// Reads the current room runtime or starts a subscription. The endpoint
    /// operation selects one-shot or subscription behavior.
    #[libertas_request]
    #[libertas_subscription_request]
    #[libertas_next_response(RoomDataV1)]
    GetRoomV1,
    /// Replace room control V1
    /// Atomically replaces all writable room intent when `expected_revision`
    /// still matches the server's current room revision.
    #[libertas_request]
    #[libertas_next_response("RoomDataV1,RoomControlRejectedV1")]
    ReplaceRoomControlV1 {
        /// Expected revision
        /// The revision from the last fully received room runtime. A mismatch
        /// rejects the write without changing persistent data.
        expected_revision: u64,
        /// Room control
        /// The complete replacement writable intent.
        control: BuildingHvacRoomControlV1,
    },
    /// Room data V1
    /// Carries the complete authoritative room runtime. It is returned by
    /// `GetRoomV1`, returned after an accepted control replacement, and reported
    /// to subscribers after any visible control, state, statistics, or plan
    /// change. An unchanged report is a valid subscription heartbeat.
    #[libertas_response]
    #[libertas_subscription_data]
    RoomDataV1 {
        /// Formatted room status
        /// Notification-compatible Avro bytes containing a localized string
        /// resource and typed printf-style arguments for current comfort, HVAC
        /// activity, noteworthy air measurements, and the next schedule change.
        /// It is a derived view only; clients and control logic must use the
        /// structured fields in this message. Do not Base64-wrap this byte
        /// array.
        #[libertas_formatted_text]
        #[libertas_size(max = 8192)]
        #[libertas_read_only]
        formatted_room_status: Vec<u8>,
        /// Maximum wait interval
        /// The maximum number of seconds a subscribed client waits after this
        /// response or report before retrying `GetRoomV1`. The server sends a
        /// changed or unchanged `RoomDataV1` report before this interval
        /// expires. A one-shot client ignores the value.
        #[libertas_time_interval]
        #[libertas_number(min = 1)]
        maximum_wait_interval_seconds: u32,
        /// Control revision
        /// The monotonically increasing revision of accepted writable room
        /// control. Sensor-only changes do not create avoidable write conflicts.
        #[libertas_read_only]
        control_revision: u64,
        /// Room control
        /// The complete current writable user intent for this room.
        control: BuildingHvacRoomControlV1,
        /// Observed state
        /// Current read-only sensor, thermostat, and effective-setpoint state.
        #[libertas_read_only]
        state: Box<BuildingHvacRoomObservedStateV1>,
        /// Active urgent conditions
        /// Confirmed time-sensitive HVAC warnings for this room. The structured
        /// list remains authoritative even though activation, reminders, and
        /// recovery are also delivered as localized notifications. These are
        /// supervisory HVAC warnings, not life-safety alarms.
        /// ----
        /// Active urgent condition
        /// One confirmed condition and its latest evidence and delivery time.
        #[libertas_size(max = 5)]
        #[libertas_read_only]
        active_urgent_conditions: Vec<BuildingHvacActiveUrgentConditionV1>,
        /// Local outdoor sensor state
        /// Independently available current readings from the optional local
        /// Matter outdoor station. The same read-only building-level state is
        /// included on every room endpoint so a runtime `GetRoomV1` query can
        /// discover PM2.5, carbon dioxide, and other supported measurements.
        #[libertas_read_only]
        local_outdoor_sensor: Option<Box<BuildingHvacLocalOutdoorSensorStateV1>>,
        /// Outdoor air analytics
        /// Psychrometric humidity ratio, moist-air enthalpy, and wet-bulb
        /// temperature derived from fresh current weather. It is absent when
        /// current weather is stale or internally inconsistent.
        #[libertas_read_only]
        outdoor_air_analytics: Option<BuildingHvacOutdoorAirAnalyticsV1>,
        /// Room statistics
        /// Read-only statistics when at least one valid collection window has
        /// been completed or is in progress.
        #[libertas_read_only]
        statistics: Option<Box<BuildingHvacRoomStatisticsV1>>,
        /// Passive outdoor coupling
        /// Learned fraction per hour by which the room naturally moves toward
        /// outdoor temperature while all thermostat-zones are inactive. It is
        /// absent until sufficient clean passive periods have been observed.
        #[libertas_read_only]
        passive_outdoor_temperature_coupling_per_hour: Option<f32>,
        /// Passive model confidence
        /// Normalized confidence from 0.0 through 1.0 in the passive outdoor
        /// coupling estimate.
        #[libertas_number(min = 0, max = 1)]
        #[libertas_read_only]
        passive_model_confidence_normalized: f32,
        /// Cross-zone influences
        /// Learned effects from other thermostat-zones on this room while its
        /// own zone is inactive. The controller uses these estimates to avoid
        /// over-conditioning an ostensibly off room.
        /// ----
        /// Cross-zone influence
        /// One directional source-thermostat to affected-room estimate.
        #[libertas_size(max = 16)]
        #[libertas_read_only]
        cross_zone_influences: Vec<BuildingHvacCrossZoneInfluenceV1>,
        /// Machine-learning predictions
        /// Read-only bounded near-term room temperature changes. Each result
        /// names XGBoost or deterministic fallback as its source. These values
        /// inform planning but never replace thermostat, deadband, explicit
        /// `Off`, urgent-condition, or life-safety constraints.
        #[libertas_read_only]
        machine_learning: BuildingHvacRoomMachineLearningV1,
        /// Calculated plan
        /// The read-only room schedule when the optimizer has sufficient data
        /// to calculate one.
        #[libertas_read_only]
        plan: Option<Box<BuildingHvacRoomPlanV1>>,
    },
    /// Room control rejected V1
    /// Rejects a control replacement without changing persistent or runtime
    /// state. The current revision and control let a client reconcile before
    /// retrying.
    #[libertas_response]
    RoomControlRejectedV1 {
        /// Formatted rejection
        /// Notification-compatible Avro bytes containing the localized resource
        /// and typed printf-style arguments that explain the rejection and safe
        /// next action. It is derived from `error` and the current structured
        /// control state and is not Base64-wrapped.
        #[libertas_formatted_text]
        #[libertas_size(max = 2048)]
        #[libertas_read_only]
        formatted_rejection: Vec<u8>,
        /// Error
        /// The reason the complete control replacement was rejected.
        error: BuildingHvacRoomControlErrorV1,
        /// Current control revision
        /// The unchanged current control revision.
        #[libertas_read_only]
        current_control_revision: u64,
        /// Current room control
        /// The unchanged current writable intent.
        #[libertas_read_only]
        current_control: BuildingHvacRoomControlV1,
    },
}

/// Building HVAC room V1
/// Defines one user-visible room and its stable runtime-control endpoint. The
/// room name is the label shown by `EnumSource` when thermostats select rooms.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacRoomV1 {
    /// Room name
    /// A nonempty name unique within this building. It is the human-readable
    /// `EnumSource` label for thermostat room associations.
    #[libertas_size(min = 1, max = 64)]
    #[libertas_ui_header]
    #[libertas_unique]
    pub name: String,
    /// Room control endpoint
    /// A stable Libertas server endpoint exposing this room's writable comfort
    /// intent and read-only state, statistics, and calculated schedule. Runtime
    /// persistence uses this endpoint as the room key rather than an array
    /// index.
    #[libertas_endpoint_schema(BuildingHvacRoomProtocolV1)]
    #[libertas_endpoint_server]
    #[libertas_unique]
    pub control_endpoint: LibertasEndpoint,
}

/// Indoor sensor V1
/// Configures one indoor Matter environmental station. Temperature is required;
/// humidity and air quality are optional capabilities of the same physical
/// station represented by their standard Matter logical devices.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub struct BuildingHvacIndoorSensorV1 {
    /// Temperature sensor
    /// A standard Matter Temperature Sensor logical device. One logical device
    /// may be assigned to only one station and contributes to the room's robust
    /// fused temperature.
    #[libertas_device_type("BQEBAYIGAA==")]
    #[libertas_ui_header]
    #[libertas_unique(3)]
    pub temperature_sensor: LibertasDevice,
    /// Humidity sensor
    /// An optional standard Matter Humidity Sensor logical device from the same
    /// physical station. Its accepted reading contributes to the room's fused
    /// relative humidity.
    #[libertas_device_type("BQEBAYcGAA==")]
    #[libertas_unique(3)]
    pub humidity_sensor: Option<LibertasDevice>,
    /// Air quality sensor
    /// An optional standard Matter Air Quality Sensor logical device from the
    /// same physical station. The controller discovers and queries PM2.5,
    /// carbon dioxide, and its other standard concentration clusters at runtime.
    #[libertas_device_type("BQEBASwA")]
    #[libertas_unique(3)]
    pub air_quality_sensor: Option<LibertasDevice>,
}

/// Outdoor sensor V1
/// Configures an optional local outdoor sensing station. Temperature is
/// required whenever the station is present; relative humidity and air quality
/// are optional additional Matter logical devices.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub struct BuildingHvacOutdoorSensorV1 {
    /// Outdoor temperature sensor
    /// A standard Matter Temperature Sensor logical device installed where it
    /// measures representative outdoor air rather than sun-heated surfaces,
    /// exhaust, or equipment discharge.
    #[libertas_device_type("BQEBAYIGAA==")]
    pub temperature_sensor: LibertasDevice,
    /// Outdoor humidity sensor
    /// An optional standard Matter Humidity Sensor logical device installed at
    /// the same representative outdoor location.
    #[libertas_device_type("BQEBAYcGAA==")]
    pub humidity_sensor: Option<LibertasDevice>,
    /// Outdoor air quality sensor
    /// An optional standard Matter Air Quality Sensor logical device. The
    /// controller discovers and queries its optional standard concentration
    /// clusters at runtime, including PM2.5 and carbon dioxide when supported.
    #[libertas_device_type("BQEBASwA")]
    pub air_quality_sensor: Option<LibertasDevice>,
}

/// Thermostat room association V1
/// Associates one room with its serving physical thermostat and with the
/// room-specific Matter sensors used to evaluate that room's comfort demand.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacThermostatRoomV1 {
    /// Room
    /// The zero-based index of a room in the building's `rooms` list. The
    /// `EnumSource` UI displays the selected room by its room-name header.
    #[libertas_enum_source("$.rooms")]
    #[libertas_unique(2)]
    pub room_index: u16,
    /// Indoor sensors
    /// One or more Matter environmental stations located in this room. Every
    /// station supplies temperature and may additionally supply humidity and
    /// runtime-discovered air-quality measurements. At least one station is
    /// required even when the thermostat reports its own local temperature.
    /// ----
    /// Indoor sensor
    /// One room-specific environmental station.
    #[libertas_size(min = 1, max = 8)]
    pub sensors: Vec<BuildingHvacIndoorSensorV1>,
}

/// Building thermostat V1
/// Configures one standard Matter Thermostat logical device and every room it
/// serves. Multiple rooms may share one thermostat, but their runtime room
/// controls are comfort demands reconciled into common physical setpoints.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacThermostatV1 {
    /// Matter thermostat
    /// The standard Matter Thermostat logical device controlled through typed
    /// Matter reads, subscriptions, and writes.
    #[libertas_device_type("BQEBAYEGAA==")]
    #[libertas_ui_header]
    #[libertas_unique]
    pub thermostat: LibertasDevice,
    /// Served rooms
    /// One or more unique rooms served by this physical thermostat. Every
    /// building room must appear exactly once across all thermostat entries.
    /// ----
    /// Served room
    /// One room reference and its required temperature and optional humidity
    /// sensor assignments.
    #[libertas_size(min = 1, max = 64)]
    pub rooms: Vec<BuildingHvacThermostatRoomV1>,
}

/// Smart building HVAC configuration V1
/// Contains the complete physical room, thermostat, and sensor topology. Rooms
/// and thermostats share one schema-data tree so nested room references can use
/// `EnumSource("$.rooms")`.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacBuildingV1 {
    /// Rooms
    /// Define all user-visible rooms before associating them with thermostats.
    /// Every room has a stable runtime endpoint and must be selected exactly
    /// once by a thermostat room association.
    /// ----
    /// Room
    /// One named room and its runtime-control endpoint.
    #[libertas_size(min = 1, max = 64)]
    pub rooms: Vec<BuildingHvacRoomV1>,
    /// Thermostats
    /// Standard Matter thermostats and the rooms and sensors they serve.
    /// ----
    /// Thermostat
    /// One physical thermostat with one or more room associations.
    #[libertas_size(min = 1, max = 16)]
    pub thermostats: Vec<BuildingHvacThermostatV1>,
    /// Outdoor sensor
    /// An optional local Matter outdoor station. When present it must include a
    /// temperature sensor and may include humidity and a standard Matter Air
    /// Quality Sensor. Fresh local temperature takes precedence over internet
    /// current weather for passive-drift learning. Runtime cluster discovery
    /// exposes any supported air measurements; forecasts and other weather
    /// inputs still come from the weather client.
    pub outdoor_sensor: Option<BuildingHvacOutdoorSensorV1>,
    /// Urgent notification recipients
    /// One or more Libertas users who receive confirmed time-sensitive HVAC
    /// warnings, bounded reminders, and recovery notifications. These
    /// notifications cover temperature, control availability, and apparent
    /// HVAC recovery failures; they are not life-safety alarms.
    /// ----
    /// Notification recipient
    /// One unique Libertas user authorized to receive building HVAC warnings.
    #[libertas_size(min = 1, max = 16)]
    #[libertas_unordered]
    #[libertas_unique]
    pub urgent_notification_recipients: Vec<LibertasUser>,
}

/// Building HVAC weather client V1
/// Selects the typed building-HVAC endpoint expected from
/// `libertas-weather_server`. The controller is only a client and never
/// performs provider HTTP requests itself.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub struct BuildingHvacWeatherClientV1 {
    /// Weather server endpoint
    /// A client endpoint implementing `BuildingHvacWeatherProtocolV1` for
    /// current conditions, recent history, forecast, outdoor air quality, and
    /// incremental recovery.
    #[libertas_endpoint_schema(BuildingHvacWeatherProtocolV1)]
    pub endpoint: LibertasEndpoint,
}

/// Persisted room condition period V1
/// Retains one bounded, time-aggregated room condition period so statistics and
/// thermal-response calculations can continue after a restart without treating
/// a last-known instantaneous sensor value as current.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacPersistedRoomConditionPeriodV1 {
    /// Start time
    /// The inclusive UTC start of this aggregated condition period.
    pub starts_at: LibertasDateTime,
    /// Duration
    /// The nonzero length of the period in seconds.
    #[libertas_time_interval]
    #[libertas_number(min = 1)]
    pub duration_seconds: u32,
    /// Room temperature
    /// The time-weighted room temperature in degrees Celsius. It is absent for
    /// a period with no trustworthy room-temperature coverage.
    pub temperature_celsius: Option<f32>,
    /// Room relative humidity
    /// The time-weighted room relative humidity percentage. It is absent for a
    /// period with no trustworthy humidity coverage.
    pub relative_humidity_percent: Option<f32>,
    /// HVAC activity
    /// The dominant thermostat activity observed during this period.
    pub activity: BuildingHvacRoomActivityV1,
    /// Effective heating setpoint
    /// The time-weighted effective heating setpoint in degrees Celsius. It is
    /// absent when the room contributed no heating target.
    pub effective_heating_setpoint_celsius: Option<f32>,
    /// Effective cooling setpoint
    /// The time-weighted effective cooling setpoint in degrees Celsius. It is
    /// absent when the room contributed no cooling target.
    pub effective_cooling_setpoint_celsius: Option<f32>,
    /// Outdoor dry-bulb temperature
    /// The time-aligned cached outdoor dry-bulb temperature in degrees Celsius.
    /// It is absent when no weather period covered this room period.
    pub outdoor_dry_bulb_temperature_celsius: Option<f32>,
}

/// Online regression state V1
/// Stores sufficient statistics for one continuously learned scalar
/// relationship. The controller decays old weight in completed half-life steps,
/// then adds each accepted observation without retaining an unbounded raw
/// history.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacOnlineRegressionStateV1 {
    /// Updated at
    /// The UTC time of the latest accepted observation. It is absent for an
    /// empty learner.
    pub updated_at: Option<LibertasDateTime>,
    /// Accepted observations
    /// The lifetime number of accepted observations before forgetting weights.
    pub accepted_observation_count: u64,
    /// Effective sample weight
    /// The quality-weighted observation count after age-based forgetting.
    #[libertas_number(min = 0)]
    pub effective_sample_weight: f64,
    /// Weighted input squared sum
    /// The decayed sum of quality weight multiplied by squared model input.
    #[libertas_number(min = 0)]
    pub weighted_input_squared_sum: f64,
    /// Weighted input-output sum
    /// The decayed sum of quality weight multiplied by model input and observed
    /// output.
    pub weighted_input_output_sum: f64,
    /// Weighted output squared sum
    /// The decayed sum of quality weight multiplied by squared observed output.
    #[libertas_number(min = 0)]
    pub weighted_output_squared_sum: f64,
}

impl BuildingHvacOnlineRegressionStateV1 {
    /// Empty online regression
    /// Creates a learner with no observations or wall-clock dependency.
    pub const fn empty() -> Self {
        Self {
            updated_at: None,
            accepted_observation_count: 0,
            effective_sample_weight: 0.0,
            weighted_input_squared_sum: 0.0,
            weighted_input_output_sum: 0.0,
            weighted_output_squared_sum: 0.0,
        }
    }

    /// Add observation
    /// Adds one finite, ordered, nonzero-input observation with a normalized
    /// quality weight. Old sufficient statistics are halved for every completed
    /// 30-day learning half-life before the new evidence is included.
    pub fn observe(
        &mut self,
        observed_at: LibertasDateTime,
        input: f64,
        output: f64,
        quality_weight_normalized: f64,
    ) -> bool {
        if !input.is_finite()
            || !output.is_finite()
            || !quality_weight_normalized.is_finite()
            || input.abs() <= f64::EPSILON
            || !(0.0..=1.0).contains(&quality_weight_normalized)
            || quality_weight_normalized <= 0.0
            || self
                .updated_at
                .is_some_and(|updated_at| observed_at < updated_at)
        {
            return false;
        }

        if let Some(updated_at) = self.updated_at {
            let completed_half_lives = observed_at.saturating_sub(updated_at)
                / BUILDING_HVAC_CROSS_ZONE_LEARNING_HALF_LIFE_SECONDS;
            if completed_half_lives >= 64 {
                self.effective_sample_weight = 0.0;
                self.weighted_input_squared_sum = 0.0;
                self.weighted_input_output_sum = 0.0;
                self.weighted_output_squared_sum = 0.0;
            } else {
                let mut remaining = completed_half_lives;
                while remaining > 0 {
                    self.effective_sample_weight *= 0.5;
                    self.weighted_input_squared_sum *= 0.5;
                    self.weighted_input_output_sum *= 0.5;
                    self.weighted_output_squared_sum *= 0.5;
                    remaining -= 1;
                }
            }
        }

        let next_effective_weight = self.effective_sample_weight + quality_weight_normalized;
        let next_input_squared =
            self.weighted_input_squared_sum + quality_weight_normalized * input * input;
        let next_input_output =
            self.weighted_input_output_sum + quality_weight_normalized * input * output;
        let next_output_squared =
            self.weighted_output_squared_sum + quality_weight_normalized * output * output;
        if !next_effective_weight.is_finite()
            || !next_input_squared.is_finite()
            || !next_input_output.is_finite()
            || !next_output_squared.is_finite()
        {
            return false;
        }

        self.updated_at = Some(observed_at);
        self.accepted_observation_count = self.accepted_observation_count.saturating_add(1);
        self.effective_sample_weight = next_effective_weight;
        self.weighted_input_squared_sum = next_input_squared;
        self.weighted_input_output_sum = next_input_output;
        self.weighted_output_squared_sum = next_output_squared;
        true
    }

    /// Estimated coefficient
    /// Returns the learned output per unit input after enough effective evidence
    /// exists. A result may be negative; callers apply the physical sign rule of
    /// their specific model.
    pub fn estimated_coefficient(&self) -> Option<f64> {
        if self.effective_sample_weight < BUILDING_HVAC_CROSS_ZONE_MINIMUM_EFFECTIVE_SAMPLE_WEIGHT
            || self.weighted_input_squared_sum <= f64::EPSILON
        {
            return None;
        }
        let coefficient = self.weighted_input_output_sum / self.weighted_input_squared_sum;
        coefficient.is_finite().then_some(coefficient)
    }

    /// Estimate confidence
    /// Returns a bounded confidence combining effective sample weight, total
    /// exposure, and signal relative to residual noise.
    pub fn confidence_normalized(&self) -> f64 {
        let Some(coefficient) = self.estimated_coefficient() else {
            return 0.0;
        };
        let sample_confidence =
            self.effective_sample_weight / (self.effective_sample_weight + 16.0);
        let exposure_confidence =
            self.weighted_input_squared_sum / (self.weighted_input_squared_sum + 0.25);
        let residual_sum = (self.weighted_output_squared_sum
            - 2.0 * coefficient * self.weighted_input_output_sum
            + coefficient * coefficient * self.weighted_input_squared_sum)
            .max(0.0);
        let signal_sum = coefficient * coefficient * self.weighted_input_squared_sum;
        let signal_confidence = signal_sum / (signal_sum + residual_sum + f64::EPSILON);
        (sample_confidence * exposure_confidence * signal_confidence).clamp(0.0, 1.0)
    }
}

impl Default for BuildingHvacOnlineRegressionStateV1 {
    fn default() -> Self {
        Self::empty()
    }
}

/// Persisted cross-zone learner V1
/// Stores separate continuously learned heating and cooling sufficient
/// statistics for one directional source-thermostat to affected-room edge.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacPersistedCrossZoneLearnerV1 {
    /// Source thermostat
    /// The other Matter thermostat whose calls may influence this room.
    pub source_thermostat: LibertasDevice,
    /// Heating learner
    /// Learns positive residual room-temperature rise per source heating runtime
    /// hour.
    pub heating: BuildingHvacOnlineRegressionStateV1,
    /// Cooling learner
    /// Learns positive residual room-temperature drop per source cooling runtime
    /// hour.
    pub cooling: BuildingHvacOnlineRegressionStateV1,
}

/// Room learning state V1
/// Persists bounded continuous-learning state for one affected room. The record
/// is keyed by that room's stable endpoint; each list item is one directional
/// influence from another thermostat-zone.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacRoomLearningStateV1 {
    /// Passive outdoor coupling learner
    /// Learns natural room-temperature movement toward outdoor temperature from
    /// periods in which every thermostat-zone is inactive.
    pub passive_outdoor_coupling: BuildingHvacOnlineRegressionStateV1,
    /// Cross-zone learners
    /// One learner for each other configured thermostat-zone. At most one edge
    /// exists for a source thermostat.
    /// ----
    /// Cross-zone learner
    /// Separate heating and cooling online regression state for one source.
    #[libertas_size(max = 16)]
    pub cross_zone_learners: Vec<BuildingHvacPersistedCrossZoneLearnerV1>,
}

impl BuildingHvacRoomLearningStateV1 {
    /// Observe identifiable passive period
    /// Learns the room's natural outdoor coupling only when every
    /// thermostat-zone is inactive. Model input is the indoor-to-outdoor
    /// temperature difference integrated over the period; output is observed
    /// room-temperature change.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_identifiable_passive_period(
        &mut self,
        observed_at: LibertasDateTime,
        every_thermostat_zone_inactive: bool,
        period_seconds: u32,
        starting_room_temperature_celsius: f32,
        outdoor_temperature_celsius: f32,
        observed_temperature_change_celsius: f32,
        quality_weight_normalized: f32,
    ) -> bool {
        if !every_thermostat_zone_inactive
            || period_seconds == 0
            || !starting_room_temperature_celsius.is_finite()
            || !outdoor_temperature_celsius.is_finite()
            || !observed_temperature_change_celsius.is_finite()
            || !quality_weight_normalized.is_finite()
            || !(0.0..=1.0).contains(&quality_weight_normalized)
            || quality_weight_normalized <= 0.0
        {
            return false;
        }
        let period_hours = f64::from(period_seconds) / 3_600.0;
        let outdoor_difference_degree_hours =
            f64::from(outdoor_temperature_celsius - starting_room_temperature_celsius)
                * period_hours;
        self.passive_outdoor_coupling.observe(
            observed_at,
            outdoor_difference_degree_hours,
            f64::from(observed_temperature_change_celsius),
            f64::from(quality_weight_normalized),
        )
    }

    /// Predict passive temperature change
    /// Predicts natural room-temperature change for an integrated
    /// indoor-to-outdoor temperature difference. It returns `None` until the
    /// passive learner has enough effective evidence.
    pub fn predict_passive_temperature_change_celsius(
        &self,
        outdoor_difference_degree_hours: f64,
    ) -> Option<f64> {
        if !outdoor_difference_degree_hours.is_finite() {
            return None;
        }
        self.passive_outdoor_coupling
            .estimated_coefficient()
            .map(|coefficient| coefficient * outdoor_difference_degree_hours)
            .filter(|prediction| prediction.is_finite())
    }

    /// Observe identifiable cross-zone period
    /// Learns only when the affected room's own thermostat is idle, exactly one
    /// other source thermostat is active, the source mode is heating or cooling,
    /// and all numeric inputs are trustworthy. `passive_temperature_change`
    /// removes the room's expected outdoor and natural drift; the remaining
    /// change is attributed directionally to the active source zone.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_identifiable_cross_zone_period(
        &mut self,
        observed_at: LibertasDateTime,
        affected_room_thermostat: LibertasDevice,
        affected_room_activity: BuildingHvacRoomActivityV1,
        source_thermostat: LibertasDevice,
        source_activity: BuildingHvacRoomActivityV1,
        active_source_count: u16,
        period_seconds: u32,
        source_runtime_fraction: f32,
        observed_temperature_change_celsius: f32,
        passive_temperature_change_celsius: f32,
        quality_weight_normalized: f32,
    ) -> bool {
        if affected_room_activity != BuildingHvacRoomActivityV1::Idle
            || affected_room_thermostat == source_thermostat
            || active_source_count != 1
            || period_seconds == 0
            || !source_runtime_fraction.is_finite()
            || !(0.0..=1.0).contains(&source_runtime_fraction)
            || source_runtime_fraction <= 0.0
            || !observed_temperature_change_celsius.is_finite()
            || !passive_temperature_change_celsius.is_finite()
            || !quality_weight_normalized.is_finite()
            || !(0.0..=1.0).contains(&quality_weight_normalized)
            || quality_weight_normalized <= 0.0
        {
            return false;
        }

        let learner_index = if let Some(index) = self
            .cross_zone_learners
            .iter()
            .position(|learner| learner.source_thermostat == source_thermostat)
        {
            index
        } else {
            if self.cross_zone_learners.len() >= BUILDING_HVAC_MAX_THERMOSTATS {
                return false;
            }
            self.cross_zone_learners
                .push(BuildingHvacPersistedCrossZoneLearnerV1 {
                    source_thermostat,
                    heating: BuildingHvacOnlineRegressionStateV1::empty(),
                    cooling: BuildingHvacOnlineRegressionStateV1::empty(),
                });
            self.cross_zone_learners.len() - 1
        };

        let source_runtime_hours =
            f64::from(source_runtime_fraction) * f64::from(period_seconds) / 3_600.0;
        let observed_change = f64::from(observed_temperature_change_celsius);
        let passive_change = f64::from(passive_temperature_change_celsius);
        let learner = &mut self.cross_zone_learners[learner_index];
        match source_activity {
            BuildingHvacRoomActivityV1::Heating => learner.heating.observe(
                observed_at,
                source_runtime_hours,
                observed_change - passive_change,
                f64::from(quality_weight_normalized),
            ),
            BuildingHvacRoomActivityV1::Cooling => learner.cooling.observe(
                observed_at,
                source_runtime_hours,
                passive_change - observed_change,
                f64::from(quality_weight_normalized),
            ),
            _ => false,
        }
    }

    /// Exposed cross-zone influences
    /// Derives bounded read-only runtime estimates from persisted sufficient
    /// statistics. Nonpositive or insufficient estimates remain absent.
    pub fn runtime_influences(&self) -> Vec<BuildingHvacCrossZoneInfluenceV1> {
        self.cross_zone_learners
            .iter()
            .map(|learner| {
                let heating = learner
                    .heating
                    .estimated_coefficient()
                    .filter(|coefficient| *coefficient > 0.0)
                    .map(|coefficient| coefficient as f32);
                let cooling = learner
                    .cooling
                    .estimated_coefficient()
                    .filter(|coefficient| *coefficient > 0.0)
                    .map(|coefficient| coefficient as f32);
                let learned_at = match (learner.heating.updated_at, learner.cooling.updated_at) {
                    (Some(heating_at), Some(cooling_at)) => Some(heating_at.max(cooling_at)),
                    (Some(updated_at), None) | (None, Some(updated_at)) => Some(updated_at),
                    (None, None) => None,
                };
                BuildingHvacCrossZoneInfluenceV1 {
                    source_thermostat: learner.source_thermostat,
                    heating_temperature_rise_celsius_per_runtime_hour: heating,
                    heating_confidence_normalized: learner.heating.confidence_normalized() as f32,
                    cooling_temperature_drop_celsius_per_runtime_hour: cooling,
                    cooling_confidence_normalized: learner.cooling.confidence_normalized() as f32,
                    learned_at,
                }
            })
            .collect()
    }
}

/// Smart building HVAC persistent data V1
/// Defines every database record written by the controller. Room records use
/// the configured room endpoint as their stable key. Local outdoor sensor and
/// weather records are singleton sections. No record contains subscription
/// cursors, peer state, or transaction identifiers.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum BuildingHvacPersistentDataV1 {
    /// Room control V1
    /// Stores the last accepted writable room intent and its optimistic
    /// concurrency revision. It is written before publishing accepted runtime
    /// data.
    RoomControlV1 {
        /// Control revision
        /// The persisted room-control revision.
        control_revision: u64,
        /// Room control
        /// The complete last accepted writable room intent.
        control: BuildingHvacRoomControlV1,
    },
    /// Room statistics V1
    /// Stores the latest bounded room comfort and data-availability statistics
    /// and its recent condition periods so a restart does not silently reset
    /// the visible collection window or short-term thermal context.
    RoomStatisticsV1 {
        /// Room statistics
        /// The complete persisted statistics collection window.
        statistics: BuildingHvacRoomStatisticsV1,
        /// Recent room conditions
        /// Ordered, non-overlapping periods covering at most the most recent
        /// day. Missing sensor or weather coverage remains explicit inside each
        /// period.
        /// ----
        /// Room condition period
        /// One aggregated indoor, setpoint, activity, and outdoor condition
        /// period.
        #[libertas_size(max = 96)]
        recent_conditions: Vec<BuildingHvacPersistedRoomConditionPeriodV1>,
    },
    /// Room learning V1
    /// Stores continuous cross-zone learning sufficient statistics for one
    /// affected room. Persist this record after each accepted learning update
    /// before exposing the newly derived influence.
    RoomLearningV1 {
        /// Room learning state
        /// The complete bounded source-zone influence learner for this room.
        learning: BuildingHvacRoomLearningStateV1,
    },
    /// Room sensor state V1
    /// Stores the independently last accepted readings for each configured
    /// indoor sensor station. The record is keyed by room endpoint; stale data
    /// remains historical context and is not treated as current after restart.
    RoomSensorStateV1 {
        /// Indoor sensor states
        /// The bounded sensor-station list in configuration order. A missing
        /// optional reading does not erase another valid station capability.
        /// ----
        /// Indoor sensor state
        /// Last accepted temperature, humidity, and air-quality sections for
        /// one station.
        #[libertas_size(min = 1, max = 8)]
        sensors: Vec<BuildingHvacIndoorSensorStateV1>,
    },
    /// Local outdoor temperature V1
    /// Stores the last valid reading from the configured local outdoor
    /// temperature sensor independently from weather and other station
    /// sections. A failed or invalid Matter report leaves this record unchanged.
    LocalOutdoorTemperatureV1 {
        /// Local outdoor temperature
        /// The accepted temperature value and its freshness deadline.
        temperature: BuildingHvacTemperatureReadingV1,
    },
    /// Local outdoor humidity V1
    /// Stores the last valid reading from the optional local outdoor humidity
    /// sensor independently. Absence of this record means no reading is
    /// available and does not clear temperature or air quality.
    LocalOutdoorHumidityV1 {
        /// Local outdoor humidity
        /// The accepted relative-humidity value and its freshness deadline.
        humidity: BuildingHvacHumidityReadingV1,
    },
    /// Local outdoor air quality V1
    /// Stores the last valid overall air-quality classification and supported
    /// concentration readings from the optional Matter Air Quality Sensor.
    /// Missing or stale values remain unknown, not safe.
    LocalOutdoorAirQualityV1 {
        /// Local outdoor air quality
        /// The accepted bounded air-quality reading and its freshness deadline.
        air_quality: BuildingHvacAirQualityReadingV1,
    },
    /// Weather history V1
    /// Stores the last accepted building-HVAC physical weather history section.
    /// A failed refresh leaves this record unchanged.
    WeatherHistoryV1 {
        /// Weather history
        /// Recent outdoor physical conditions used for load-model context.
        history: BuildingHvacWeatherHistoryV1,
    },
    /// Current weather V1
    /// Stores the last accepted building-HVAC current physical weather section.
    /// Stale data remains available but is not evidence that outdoor-air or
    /// economizer operation is safe.
    WeatherCurrentV1 {
        /// Current weather
        /// The last accepted current outdoor physical conditions.
        current: BuildingHvacCurrentWeatherV1,
    },
    /// Weather forecast V1
    /// Stores the last accepted building-HVAC physical forecast section for
    /// restart-time preheating and precooling calculations.
    WeatherForecastV1 {
        /// Weather forecast
        /// The last accepted outdoor physical forecast.
        forecast: BuildingHvacWeatherForecastV1,
    },
    /// Outdoor air quality V1
    /// Stores the last accepted modeled outdoor-air-quality section separately
    /// from physical weather. Missing or stale model data remains unknown, not
    /// safe.
    OutdoorAirQualityV1 {
        /// Outdoor air quality
        /// The last accepted modeled outdoor-air-quality periods.
        outdoor_air_quality: BuildingHvacOutdoorAirQualityV1,
    },
    /// Room urgent notification state V1
    /// Stores bounded activation, reminder, and recovery state for one room.
    /// Persist each state transition before sending its notification so a
    /// restart does not reset confirmation intervals or create an avoidable
    /// duplicate-warning burst.
    RoomUrgentNotificationStateV1 {
        /// Urgent-condition trackers
        /// At most one tracker exists for each V1 condition. An absent tracker
        /// means that condition is inactive with no pending transition.
        /// ----
        /// Urgent-condition tracker
        /// One condition's persisted debounce and last-notification state.
        #[libertas_size(max = 5)]
        conditions: Vec<BuildingHvacPersistedUrgentConditionV1>,
    },
    /// Machine-learning models V1
    /// Stores every accepted thermal model together with one rollback artifact
    /// per horizon. A candidate is persisted here before it is activated on the
    /// XGBoost worker.
    MachineLearningModelsV1 {
        /// Model set
        /// Complete bounded active and rollback model state.
        models: BuildingHvacMachineLearningModelSetV1,
    },
    /// Machine-learning sample V1
    /// One record in a room-keyed indexed history. Its database index is
    /// `sample.observed_at`; the value repeats the timestamp and stable room
    /// endpoint so a mismatched or corrupt record can be rejected.
    MachineLearningSampleV1 {
        /// Training sample
        /// Validated features and any temperature-change labels that have
        /// become available for this observation.
        sample: BuildingHvacMachineLearningSampleV1,
    },
}

const BUILDING_HVAC_URGENT_CONDITIONS: [BuildingHvacUrgentConditionV1;
    BUILDING_HVAC_MAX_URGENT_ROOM_CONDITIONS] = [
    BuildingHvacUrgentConditionV1::FreezeRisk,
    BuildingHvacUrgentConditionV1::ExcessiveHeat,
    BuildingHvacUrgentConditionV1::TemperatureControlUnavailable,
    BuildingHvacUrgentConditionV1::HeatingNotRecovering,
    BuildingHvacUrgentConditionV1::CoolingNotRecovering,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuildingHvacUrgentEvidence {
    Qualifying,
    Recovering,
    Neutral,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuildingHvacUrgentNotificationActionKind {
    ActivatedOrReminder,
    Recovered,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct BuildingHvacUrgentNotificationAction {
    kind: BuildingHvacUrgentNotificationActionKind,
    condition: BuildingHvacUrgentConditionV1,
    severity: BuildingHvacUrgentNotificationSeverityV1,
    active_since: LibertasDateTime,
    occurred_at: LibertasDateTime,
    temperature_celsius: Option<f32>,
}

impl BuildingHvacUrgentNotificationAction {
    fn submit(self, recipients: &[LibertasUser], room_name: &str) -> bool {
        if recipients.is_empty() {
            return false;
        }

        if self.kind == BuildingHvacUrgentNotificationActionKind::Recovered {
            let Some(temperature_celsius) = self.temperature_celsius else {
                return false;
            };
            let arguments = [
                NotificationArgument::LiteralText(room_name),
                NotificationArgument::ResourceText(self.condition.condition_name_resource()),
                NotificationArgument::UnitFloat {
                    unit_type: "temperature-celsius",
                    value: temperature_celsius,
                },
            ];
            libertas_notification_send(
                recipients,
                NotificationImportance::Info,
                None,
                "HVAC_URGENT_CONDITION_RECOVERED",
                &arguments,
            );
            return true;
        }

        let elapsed_seconds = self.occurred_at.saturating_sub(self.active_since);
        match self.condition {
            BuildingHvacUrgentConditionV1::FreezeRisk
            | BuildingHvacUrgentConditionV1::ExcessiveHeat => {
                let Some(temperature_celsius) = self.temperature_celsius else {
                    return false;
                };
                let arguments = [
                    NotificationArgument::LiteralText(room_name),
                    NotificationArgument::UnitFloat {
                        unit_type: "temperature-celsius",
                        value: temperature_celsius,
                    },
                ];
                libertas_notification_send(
                    recipients,
                    self.severity.notification_importance(),
                    None,
                    self.condition.notification_resource(),
                    &arguments,
                );
            }
            BuildingHvacUrgentConditionV1::TemperatureControlUnavailable => {
                let arguments = [
                    NotificationArgument::LiteralText(room_name),
                    NotificationArgument::UnitUnsigned {
                        unit_type: "duration-seconds",
                        value: elapsed_seconds,
                    },
                ];
                libertas_notification_send(
                    recipients,
                    self.severity.notification_importance(),
                    None,
                    self.condition.notification_resource(),
                    &arguments,
                );
            }
            BuildingHvacUrgentConditionV1::HeatingNotRecovering
            | BuildingHvacUrgentConditionV1::CoolingNotRecovering => {
                let Some(temperature_celsius) = self.temperature_celsius else {
                    return false;
                };
                let arguments = [
                    NotificationArgument::LiteralText(room_name),
                    NotificationArgument::UnitUnsigned {
                        unit_type: "duration-seconds",
                        value: elapsed_seconds,
                    },
                    NotificationArgument::UnitFloat {
                        unit_type: "temperature-celsius",
                        value: temperature_celsius,
                    },
                ];
                libertas_notification_send(
                    recipients,
                    self.severity.notification_importance(),
                    None,
                    self.condition.notification_resource(),
                    &arguments,
                );
            }
        }
        true
    }
}

/// Urgent notification evaluation
/// Reports whether persisted/runtime condition state changed and how many
/// localized notification submissions are pending. The action payload remains
/// implementation-only and is submitted by the engine after persistence.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildingHvacUrgentNotificationEvaluation {
    state_changed: bool,
    actions: Vec<BuildingHvacUrgentNotificationAction>,
}

impl BuildingHvacUrgentNotificationEvaluation {
    /// State changed
    /// Returns whether the caller must persist and report the engine's new
    /// condition state.
    pub const fn state_changed(&self) -> bool {
        self.state_changed
    }

    /// Pending notification count
    /// Returns the number of activation, reminder, or recovery messages that
    /// will be submitted after persistence.
    pub fn pending_notification_count(&self) -> usize {
        self.actions.len()
    }
}

/// Urgent HVAC notification engine
/// Evaluates trusted room state and bounded recent conditions using persisted
/// activation, hysteresis, reminder, and recovery state. Calculation is pure;
/// `persist_and_submit` performs the ordered Libertas side effects afterward.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildingHvacUrgentNotificationEngine {
    conditions: Vec<BuildingHvacPersistedUrgentConditionV1>,
}

impl BuildingHvacUrgentNotificationEngine {
    /// Empty urgent notification engine
    /// Starts with no pending or active conditions.
    pub const fn new() -> Self {
        Self {
            conditions: Vec::new(),
        }
    }

    /// Restore urgent notification engine
    /// Keeps only well-formed unique V1 trackers and orders them by condition.
    /// Invalid or duplicate decoded records are ignored rather than exposed.
    pub fn restore(
        conditions: Vec<BuildingHvacPersistedUrgentConditionV1>,
    ) -> BuildingHvacUrgentNotificationEngine {
        let mut restored = Vec::new();
        for condition in conditions {
            if condition.is_well_formed()
                && !restored
                    .iter()
                    .any(|existing: &BuildingHvacPersistedUrgentConditionV1| {
                        existing.condition == condition.condition
                    })
                && restored.len() < BUILDING_HVAC_MAX_URGENT_ROOM_CONDITIONS
            {
                restored.push(condition);
            }
        }
        restored.sort_by_key(|condition| urgent_condition_index(condition.condition));
        Self {
            conditions: restored,
        }
    }

    /// Persisted urgent conditions
    /// Returns the complete validated tracker list to store in
    /// `RoomUrgentNotificationStateV1`.
    pub fn persisted_conditions(&self) -> &[BuildingHvacPersistedUrgentConditionV1] {
        &self.conditions
    }

    /// Active urgent conditions
    /// Derives the authoritative runtime list. Recovery-pending warnings remain
    /// active until the complete hysteresis interval succeeds.
    pub fn active_conditions(&self) -> Vec<BuildingHvacActiveUrgentConditionV1> {
        self.conditions
            .iter()
            .filter(|condition| {
                condition.phase != BuildingHvacUrgentConditionPhaseV1::ActivationPending
            })
            .map(|condition| BuildingHvacActiveUrgentConditionV1 {
                condition: condition.condition,
                severity: condition.condition.severity(),
                active_since: condition.condition_started_at,
                updated_at: condition.updated_at,
                temperature_celsius: condition.last_temperature_celsius,
                last_notification_at: condition.last_notification_at,
            })
            .collect()
    }

    /// Evaluate urgent room conditions
    /// Applies activation confirmation, restart-safe evidence gaps, recovery
    /// hysteresis, and reminder limits using already validated room state and
    /// ordered recent condition periods. Backward evaluation time is ignored.
    pub fn evaluate(
        &mut self,
        evaluated_at: LibertasDateTime,
        room_state: &BuildingHvacRoomObservedStateV1,
        recent_conditions: &[BuildingHvacPersistedRoomConditionPeriodV1],
    ) -> BuildingHvacUrgentNotificationEvaluation {
        let mut evaluation = BuildingHvacUrgentNotificationEvaluation {
            state_changed: false,
            actions: Vec::new(),
        };
        if self
            .conditions
            .iter()
            .any(|condition| condition.updated_at > evaluated_at)
        {
            return evaluation;
        }

        for condition in BUILDING_HVAC_URGENT_CONDITIONS {
            let evidence =
                urgent_condition_evidence(condition, evaluated_at, room_state, recent_conditions);
            self.apply_evidence(
                condition,
                evidence,
                evaluated_at,
                room_state,
                &mut evaluation,
            );
        }
        self.conditions
            .sort_by_key(|condition| urgent_condition_index(condition.condition));
        evaluation
    }

    /// Persist and submit urgent notifications
    /// Writes the complete room tracker union before submitting any activation,
    /// reminder, or recovery notification. It returns the number submitted.
    /// Libertas currently provides no persistence-completion or notification
    /// delivery acknowledgement.
    pub fn persist_and_submit(
        &self,
        room_endpoint: LibertasEndpoint,
        room_name: &str,
        recipients: &[LibertasUser],
        evaluation: &BuildingHvacUrgentNotificationEvaluation,
    ) -> usize {
        if !evaluation.state_changed {
            return 0;
        }

        let key = [NotificationArgument::Object(room_endpoint)];
        let value = BuildingHvacPersistentDataV1::RoomUrgentNotificationStateV1 {
            conditions: self.conditions.clone(),
        };
        libertas_data_write("HVAC_ROOM_URGENT_NOTIFICATION_STATE", &key, &value);
        evaluation
            .actions
            .iter()
            .copied()
            .filter(|action| action.submit(recipients, room_name))
            .count()
    }

    fn apply_evidence(
        &mut self,
        condition: BuildingHvacUrgentConditionV1,
        evidence: BuildingHvacUrgentEvidence,
        evaluated_at: LibertasDateTime,
        room_state: &BuildingHvacRoomObservedStateV1,
        evaluation: &mut BuildingHvacUrgentNotificationEvaluation,
    ) {
        let temperature_celsius = room_state
            .temperature_celsius
            .filter(|temperature| temperature.is_finite());
        let Some(index) = self
            .conditions
            .iter()
            .position(|tracker| tracker.condition == condition)
        else {
            if evidence != BuildingHvacUrgentEvidence::Qualifying {
                return;
            }
            let immediately_active = condition
                == BuildingHvacUrgentConditionV1::HeatingNotRecovering
                || condition == BuildingHvacUrgentConditionV1::CoolingNotRecovering;
            let condition_started_at = if immediately_active {
                evaluated_at
                    .saturating_sub(u64::from(BUILDING_HVAC_NOT_RECOVERING_OBSERVATION_SECONDS))
            } else {
                evaluated_at
            };
            let tracker = BuildingHvacPersistedUrgentConditionV1 {
                condition,
                phase: if immediately_active {
                    BuildingHvacUrgentConditionPhaseV1::Active
                } else {
                    BuildingHvacUrgentConditionPhaseV1::ActivationPending
                },
                condition_started_at,
                phase_started_at: evaluated_at,
                updated_at: evaluated_at,
                last_temperature_celsius: if condition
                    == BuildingHvacUrgentConditionV1::TemperatureControlUnavailable
                {
                    None
                } else {
                    temperature_celsius
                },
                last_notification_at: immediately_active.then_some(evaluated_at),
            };
            self.conditions.push(tracker);
            evaluation.state_changed = true;
            if immediately_active {
                evaluation.actions.push(notification_action(
                    BuildingHvacUrgentNotificationActionKind::ActivatedOrReminder,
                    tracker,
                    evaluated_at,
                ));
            }
            return;
        };

        let gap_too_large = evaluated_at.saturating_sub(self.conditions[index].updated_at)
            > u64::from(BUILDING_HVAC_URGENT_EVIDENCE_MAX_GAP_SECONDS);
        match self.conditions[index].phase {
            BuildingHvacUrgentConditionPhaseV1::ActivationPending => {
                if evidence != BuildingHvacUrgentEvidence::Qualifying {
                    self.conditions.remove(index);
                    evaluation.state_changed = true;
                    return;
                }
                if gap_too_large {
                    let tracker = &mut self.conditions[index];
                    tracker.condition_started_at = evaluated_at;
                    tracker.phase_started_at = evaluated_at;
                    tracker.updated_at = evaluated_at;
                    tracker.last_temperature_celsius = temperature_celsius;
                    evaluation.state_changed = true;
                    return;
                }

                let activation_seconds = urgent_activation_seconds(condition);
                let tracker = &mut self.conditions[index];
                tracker.updated_at = evaluated_at;
                tracker.last_temperature_celsius = temperature_celsius;
                if evaluated_at.saturating_sub(tracker.condition_started_at)
                    >= u64::from(activation_seconds)
                {
                    tracker.phase = BuildingHvacUrgentConditionPhaseV1::Active;
                    tracker.phase_started_at = evaluated_at;
                    tracker.last_notification_at = Some(evaluated_at);
                    evaluation.state_changed = true;
                    evaluation.actions.push(notification_action(
                        BuildingHvacUrgentNotificationActionKind::ActivatedOrReminder,
                        *tracker,
                        evaluated_at,
                    ));
                }
            }
            BuildingHvacUrgentConditionPhaseV1::Active => match evidence {
                BuildingHvacUrgentEvidence::Qualifying => {
                    let tracker = &mut self.conditions[index];
                    tracker.updated_at = evaluated_at;
                    tracker.last_temperature_celsius = temperature_celsius;
                    let reminder_due = tracker.last_notification_at.is_none_or(|sent_at| {
                        evaluated_at.saturating_sub(sent_at)
                            >= u64::from(BUILDING_HVAC_URGENT_NOTIFICATION_REMINDER_SECONDS)
                    });
                    if reminder_due {
                        tracker.last_notification_at = Some(evaluated_at);
                        evaluation.state_changed = true;
                        evaluation.actions.push(notification_action(
                            BuildingHvacUrgentNotificationActionKind::ActivatedOrReminder,
                            *tracker,
                            evaluated_at,
                        ));
                    }
                }
                BuildingHvacUrgentEvidence::Recovering => {
                    let tracker = &mut self.conditions[index];
                    tracker.phase = BuildingHvacUrgentConditionPhaseV1::RecoveryPending;
                    tracker.phase_started_at = evaluated_at;
                    tracker.updated_at = evaluated_at;
                    tracker.last_temperature_celsius = temperature_celsius;
                    evaluation.state_changed = true;
                }
                BuildingHvacUrgentEvidence::Neutral => {
                    let tracker = &mut self.conditions[index];
                    tracker.updated_at = evaluated_at;
                    tracker.last_temperature_celsius = temperature_celsius;
                }
                BuildingHvacUrgentEvidence::Unknown => {}
            },
            BuildingHvacUrgentConditionPhaseV1::RecoveryPending => match evidence {
                BuildingHvacUrgentEvidence::Recovering => {
                    if gap_too_large {
                        self.conditions[index].phase_started_at = evaluated_at;
                        evaluation.state_changed = true;
                    }
                    let tracker = &mut self.conditions[index];
                    tracker.updated_at = evaluated_at;
                    tracker.last_temperature_celsius = temperature_celsius;
                    if evaluated_at.saturating_sub(tracker.phase_started_at)
                        >= u64::from(BUILDING_HVAC_URGENT_RECOVERY_CONFIRMATION_SECONDS)
                    {
                        let tracker = self.conditions.remove(index);
                        evaluation.state_changed = true;
                        evaluation.actions.push(notification_action(
                            BuildingHvacUrgentNotificationActionKind::Recovered,
                            tracker,
                            evaluated_at,
                        ));
                    }
                }
                BuildingHvacUrgentEvidence::Qualifying => {
                    let tracker = &mut self.conditions[index];
                    tracker.phase = BuildingHvacUrgentConditionPhaseV1::Active;
                    tracker.phase_started_at = evaluated_at;
                    tracker.updated_at = evaluated_at;
                    tracker.last_temperature_celsius = temperature_celsius;
                    evaluation.state_changed = true;
                    let reminder_due = tracker.last_notification_at.is_none_or(|sent_at| {
                        evaluated_at.saturating_sub(sent_at)
                            >= u64::from(BUILDING_HVAC_URGENT_NOTIFICATION_REMINDER_SECONDS)
                    });
                    if reminder_due {
                        tracker.last_notification_at = Some(evaluated_at);
                        evaluation.actions.push(notification_action(
                            BuildingHvacUrgentNotificationActionKind::ActivatedOrReminder,
                            *tracker,
                            evaluated_at,
                        ));
                    }
                }
                BuildingHvacUrgentEvidence::Neutral | BuildingHvacUrgentEvidence::Unknown => {
                    let tracker = &mut self.conditions[index];
                    tracker.phase = BuildingHvacUrgentConditionPhaseV1::Active;
                    tracker.phase_started_at = evaluated_at;
                    if evidence == BuildingHvacUrgentEvidence::Neutral {
                        tracker.updated_at = evaluated_at;
                        tracker.last_temperature_celsius = temperature_celsius;
                    }
                    evaluation.state_changed = true;
                }
            },
        }
    }
}

impl Default for BuildingHvacUrgentNotificationEngine {
    fn default() -> Self {
        Self::new()
    }
}

const fn urgent_condition_index(condition: BuildingHvacUrgentConditionV1) -> usize {
    match condition {
        BuildingHvacUrgentConditionV1::FreezeRisk => 0,
        BuildingHvacUrgentConditionV1::ExcessiveHeat => 1,
        BuildingHvacUrgentConditionV1::TemperatureControlUnavailable => 2,
        BuildingHvacUrgentConditionV1::HeatingNotRecovering => 3,
        BuildingHvacUrgentConditionV1::CoolingNotRecovering => 4,
    }
}

const fn urgent_activation_seconds(condition: BuildingHvacUrgentConditionV1) -> u32 {
    match condition {
        BuildingHvacUrgentConditionV1::FreezeRisk
        | BuildingHvacUrgentConditionV1::ExcessiveHeat => {
            BUILDING_HVAC_URGENT_TEMPERATURE_CONFIRMATION_SECONDS
        }
        BuildingHvacUrgentConditionV1::TemperatureControlUnavailable => {
            BUILDING_HVAC_CONTROL_UNAVAILABLE_CONFIRMATION_SECONDS
        }
        BuildingHvacUrgentConditionV1::HeatingNotRecovering
        | BuildingHvacUrgentConditionV1::CoolingNotRecovering => {
            BUILDING_HVAC_NOT_RECOVERING_OBSERVATION_SECONDS
        }
    }
}

fn notification_action(
    kind: BuildingHvacUrgentNotificationActionKind,
    tracker: BuildingHvacPersistedUrgentConditionV1,
    occurred_at: LibertasDateTime,
) -> BuildingHvacUrgentNotificationAction {
    BuildingHvacUrgentNotificationAction {
        kind,
        condition: tracker.condition,
        severity: tracker.condition.severity(),
        active_since: tracker.condition_started_at,
        occurred_at,
        temperature_celsius: tracker.last_temperature_celsius,
    }
}

fn urgent_condition_evidence(
    condition: BuildingHvacUrgentConditionV1,
    evaluated_at: LibertasDateTime,
    room_state: &BuildingHvacRoomObservedStateV1,
    recent_conditions: &[BuildingHvacPersistedRoomConditionPeriodV1],
) -> BuildingHvacUrgentEvidence {
    let temperature_celsius = room_state
        .temperature_celsius
        .filter(|temperature| temperature.is_finite());
    let trustworthy_temperature = (room_state.data_quality
        != BuildingHvacRoomDataQualityV1::Unavailable)
        .then_some(temperature_celsius)
        .flatten();

    match condition {
        BuildingHvacUrgentConditionV1::FreezeRisk => {
            let Some(temperature) = trustworthy_temperature else {
                return BuildingHvacUrgentEvidence::Unknown;
            };
            if temperature <= BUILDING_HVAC_FREEZE_RISK_TEMPERATURE_CELSIUS {
                BuildingHvacUrgentEvidence::Qualifying
            } else if temperature >= BUILDING_HVAC_FREEZE_RECOVERY_TEMPERATURE_CELSIUS {
                BuildingHvacUrgentEvidence::Recovering
            } else {
                BuildingHvacUrgentEvidence::Neutral
            }
        }
        BuildingHvacUrgentConditionV1::ExcessiveHeat => {
            let Some(temperature) = trustworthy_temperature else {
                return BuildingHvacUrgentEvidence::Unknown;
            };
            if temperature >= BUILDING_HVAC_EXCESSIVE_HEAT_TEMPERATURE_CELSIUS {
                BuildingHvacUrgentEvidence::Qualifying
            } else if temperature <= BUILDING_HVAC_EXCESSIVE_HEAT_RECOVERY_TEMPERATURE_CELSIUS {
                BuildingHvacUrgentEvidence::Recovering
            } else {
                BuildingHvacUrgentEvidence::Neutral
            }
        }
        BuildingHvacUrgentConditionV1::TemperatureControlUnavailable => {
            if trustworthy_temperature.is_some() {
                BuildingHvacUrgentEvidence::Recovering
            } else {
                BuildingHvacUrgentEvidence::Qualifying
            }
        }
        BuildingHvacUrgentConditionV1::HeatingNotRecovering => {
            let Some(temperature) = trustworthy_temperature else {
                return BuildingHvacUrgentEvidence::Unknown;
            };
            if temperature > BUILDING_HVAC_HEATING_NOT_RECOVERING_TEMPERATURE_CELSIUS {
                return BuildingHvacUrgentEvidence::Recovering;
            }
            if room_state.activity != BuildingHvacRoomActivityV1::Heating {
                return BuildingHvacUrgentEvidence::Neutral;
            }
            match recovery_temperature_change(
                evaluated_at,
                BuildingHvacRoomActivityV1::Heating,
                temperature,
                recent_conditions,
            ) {
                Some(change) if change < BUILDING_HVAC_MINIMUM_RECOVERY_CHANGE_CELSIUS => {
                    BuildingHvacUrgentEvidence::Qualifying
                }
                Some(_) => BuildingHvacUrgentEvidence::Recovering,
                None => BuildingHvacUrgentEvidence::Unknown,
            }
        }
        BuildingHvacUrgentConditionV1::CoolingNotRecovering => {
            let Some(temperature) = trustworthy_temperature else {
                return BuildingHvacUrgentEvidence::Unknown;
            };
            if temperature < BUILDING_HVAC_COOLING_NOT_RECOVERING_TEMPERATURE_CELSIUS {
                return BuildingHvacUrgentEvidence::Recovering;
            }
            if room_state.activity != BuildingHvacRoomActivityV1::Cooling {
                return BuildingHvacUrgentEvidence::Neutral;
            }
            match recovery_temperature_change(
                evaluated_at,
                BuildingHvacRoomActivityV1::Cooling,
                temperature,
                recent_conditions,
            ) {
                Some(change) if change < BUILDING_HVAC_MINIMUM_RECOVERY_CHANGE_CELSIUS => {
                    BuildingHvacUrgentEvidence::Qualifying
                }
                Some(_) => BuildingHvacUrgentEvidence::Recovering,
                None => BuildingHvacUrgentEvidence::Unknown,
            }
        }
    }
}

fn recovery_temperature_change(
    evaluated_at: LibertasDateTime,
    activity: BuildingHvacRoomActivityV1,
    current_temperature_celsius: f32,
    recent_conditions: &[BuildingHvacPersistedRoomConditionPeriodV1],
) -> Option<f32> {
    let window_start =
        evaluated_at.saturating_sub(u64::from(BUILDING_HVAC_NOT_RECOVERING_OBSERVATION_SECONDS));
    let mut covered_seconds = 0_u64;
    let mut starting_temperature_celsius = None;
    let mut previous_end = None;
    for period in recent_conditions {
        let period_end = period
            .starts_at
            .checked_add(u64::from(period.duration_seconds))?;
        if period.duration_seconds == 0
            || previous_end.is_some_and(|end| period.starts_at < end)
            || period
                .temperature_celsius
                .is_some_and(|temperature| !temperature.is_finite())
        {
            return None;
        }
        previous_end = Some(period_end);
        let overlap_start = period.starts_at.max(window_start);
        let overlap_end = period_end.min(evaluated_at);
        if overlap_end <= overlap_start
            || period.activity != activity
            || period.temperature_celsius.is_none()
        {
            continue;
        }
        covered_seconds = covered_seconds.saturating_add(overlap_end.saturating_sub(overlap_start));
        if starting_temperature_celsius.is_none() {
            starting_temperature_celsius = period.temperature_celsius;
        }
    }

    let required_coverage = f64::from(BUILDING_HVAC_NOT_RECOVERING_OBSERVATION_SECONDS)
        * f64::from(BUILDING_HVAC_MINIMUM_RECOVERY_DATA_COVERAGE_NORMALIZED);
    if covered_seconds as f64 + f64::EPSILON < required_coverage {
        return None;
    }
    let starting_temperature_celsius = starting_temperature_celsius?;
    match activity {
        BuildingHvacRoomActivityV1::Heating => {
            Some(current_temperature_celsius - starting_temperature_celsius)
        }
        BuildingHvacRoomActivityV1::Cooling => {
            Some(starting_temperature_celsius - current_temperature_celsius)
        }
        _ => None,
    }
    .filter(|change| change.is_finite())
}

/// Building HVAC analytics engine
/// Produces trustworthy room state and bounded statistics from already decoded
/// Matter sensor data. It excludes stale and invalid values, rejects ambiguous
/// two-sensor disagreement, and preserves station-level data for diagnosis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildingHvacAnalyticsEngine;

impl BuildingHvacAnalyticsEngine {
    /// New analytics engine
    /// Creates the stateless V1 analytics implementation.
    pub const fn new() -> Self {
        Self
    }

    /// Analyze room
    /// Fuses fresh temperature and humidity values around their medians,
    /// evaluates thermostat freshness, and returns the complete protocol room
    /// state. Optional air-quality sections remain per station and never affect
    /// temperature-control readiness.
    #[allow(clippy::too_many_arguments)]
    pub fn analyze_room(
        &self,
        evaluated_at: LibertasDateTime,
        physical_thermostat: LibertasDevice,
        thermostat_observed_at: Option<LibertasDateTime>,
        thermostat_valid_until: Option<LibertasDateTime>,
        thermostat_activity: BuildingHvacRoomActivityV1,
        effective_heating_setpoint_celsius: Option<f32>,
        effective_cooling_setpoint_celsius: Option<f32>,
        sensor_states: &[BuildingHvacIndoorSensorStateV1],
    ) -> BuildingHvacRoomObservedStateV1 {
        let thermostat_is_fresh = match (thermostat_observed_at, thermostat_valid_until) {
            (Some(observed_at), Some(valid_until)) => {
                observed_at <= evaluated_at
                    && valid_until > evaluated_at
                    && valid_until > observed_at
            }
            _ => false,
        };

        let mut current_sensor_states = Vec::new();
        for sensor in sensor_states
            .iter()
            .take(BUILDING_HVAC_MAX_SENSORS_PER_ROOM)
        {
            let mut sensor = sensor.clone();
            if !sensor
                .temperature
                .is_some_and(|reading| temperature_reading_is_current(reading, evaluated_at))
            {
                sensor.temperature = None;
            }
            if !sensor
                .humidity
                .is_some_and(|reading| humidity_reading_is_current(reading, evaluated_at))
            {
                sensor.humidity = None;
            }
            if !sensor
                .air_quality
                .as_ref()
                .is_some_and(|reading| air_quality_reading_is_current(reading, evaluated_at))
            {
                sensor.air_quality = None;
            }
            current_sensor_states.push(sensor);
        }

        let temperature_values: Vec<(f32, LibertasDateTime)> = current_sensor_states
            .iter()
            .filter_map(|sensor| {
                sensor
                    .temperature
                    .map(|reading| (reading.temperature_celsius, reading.observed_at))
            })
            .collect();
        let humidity_values: Vec<(f32, LibertasDateTime)> = current_sensor_states
            .iter()
            .filter_map(|sensor| {
                sensor
                    .humidity
                    .map(|reading| (reading.relative_humidity_percent, reading.observed_at))
            })
            .collect();
        let (temperature_celsius, fresh_temperature_sensor_count, temperature_observed_at) =
            robust_fused_measurement(
                &temperature_values,
                BUILDING_HVAC_TEMPERATURE_FUSION_OUTLIER_CELSIUS,
            );
        let (relative_humidity_percent, fresh_humidity_sensor_count, humidity_observed_at) =
            robust_fused_measurement(
                &humidity_values,
                BUILDING_HVAC_HUMIDITY_FUSION_OUTLIER_PERCENT,
            );

        let configured_temperature_sensor_count = current_sensor_states.len() as u16;
        let configured_humidity_sensor_count = current_sensor_states
            .iter()
            .filter(|sensor| sensor.humidity_sensor.is_some())
            .count() as u16;
        let data_quality = if !thermostat_is_fresh || temperature_celsius.is_none() {
            BuildingHvacRoomDataQualityV1::Unavailable
        } else if fresh_temperature_sensor_count < configured_temperature_sensor_count
            || fresh_humidity_sensor_count < configured_humidity_sensor_count
        {
            BuildingHvacRoomDataQualityV1::Degraded
        } else {
            BuildingHvacRoomDataQualityV1::Ready
        };

        let sensor_observed_at = current_sensor_states
            .iter()
            .filter_map(|sensor| {
                sensor
                    .air_quality
                    .as_ref()
                    .map(|reading| reading.observed_at)
            })
            .chain(temperature_observed_at)
            .chain(humidity_observed_at)
            .max();
        let observed_at = sensor_observed_at
            .into_iter()
            .chain(
                thermostat_is_fresh
                    .then_some(thermostat_observed_at)
                    .flatten(),
            )
            .max();

        BuildingHvacRoomObservedStateV1 {
            data_quality,
            observed_at,
            temperature_celsius,
            relative_humidity_percent,
            effective_heating_setpoint_celsius: thermostat_is_fresh
                .then_some(effective_heating_setpoint_celsius)
                .flatten()
                .filter(|value| value.is_finite()),
            effective_cooling_setpoint_celsius: thermostat_is_fresh
                .then_some(effective_cooling_setpoint_celsius)
                .flatten()
                .filter(|value| value.is_finite()),
            activity: if thermostat_is_fresh {
                thermostat_activity
            } else {
                BuildingHvacRoomActivityV1::Unknown
            },
            physical_thermostat,
            sensor_states: current_sensor_states,
            fresh_temperature_sensor_count,
            configured_temperature_sensor_count,
            fresh_humidity_sensor_count,
            configured_humidity_sensor_count,
        }
    }

    /// Analyze outdoor air
    /// Derives humidity ratio, moist-air enthalpy, and wet-bulb temperature from
    /// fresh current weather. Dew point and pressure form the calculation input;
    /// provider relative humidity is used only as a consistency check.
    pub fn analyze_outdoor_air(
        &self,
        evaluated_at: LibertasDateTime,
        current: &BuildingHvacCurrentWeatherV1,
    ) -> Option<BuildingHvacOutdoorAirAnalyticsV1> {
        if !current.is_fresh_at(evaluated_at)
            || current.retrieved_at > evaluated_at
            || current.valid_at > evaluated_at
            || current.interval_seconds == 0
        {
            return None;
        }
        let conditions = current.conditions;
        let dry_bulb_temperature_celsius = conditions.dry_bulb_temperature_celsius;
        let dew_point_temperature_celsius = conditions.dew_point_temperature_celsius;
        let pressure_pascals = conditions.surface_pressure_hectopascals * 100.0;
        if !dry_bulb_temperature_celsius.is_finite()
            || !dew_point_temperature_celsius.is_finite()
            || dew_point_temperature_celsius > dry_bulb_temperature_celsius + 0.05
            || !pressure_pascals.is_finite()
            || pressure_pascals <= 0.0
        {
            return None;
        }

        let vapor_pressure_pascals =
            saturation_vapor_pressure_pascals(dew_point_temperature_celsius)?;
        let dry_bulb_saturation_pressure_pascals =
            saturation_vapor_pressure_pascals(dry_bulb_temperature_celsius)?;
        if vapor_pressure_pascals >= pressure_pascals || dry_bulb_saturation_pressure_pascals <= 0.0
        {
            return None;
        }
        let calculated_relative_humidity_percent =
            100.0 * vapor_pressure_pascals / dry_bulb_saturation_pressure_pascals;
        if !calculated_relative_humidity_percent.is_finite()
            || !(0.0..=100.5).contains(&calculated_relative_humidity_percent)
            || (calculated_relative_humidity_percent
                - f32::from(conditions.relative_humidity_percent))
            .abs()
                > BUILDING_HVAC_WEATHER_HUMIDITY_CONSISTENCY_PERCENT
        {
            return None;
        }

        let humidity_ratio =
            0.621_945 * vapor_pressure_pascals / (pressure_pascals - vapor_pressure_pascals);
        let enthalpy = moist_air_enthalpy_kilojoules_per_kilogram_dry_air(
            dry_bulb_temperature_celsius,
            humidity_ratio,
        );
        let wet_bulb_temperature_celsius = solve_wet_bulb_temperature_celsius(
            dry_bulb_temperature_celsius,
            dew_point_temperature_celsius,
            pressure_pascals,
            enthalpy,
        )?;
        let analytics = BuildingHvacOutdoorAirAnalyticsV1 {
            weather_valid_at: current.valid_at,
            humidity_ratio_kilograms_water_per_kilogram_dry_air: humidity_ratio,
            moist_air_enthalpy_kilojoules_per_kilogram_dry_air: enthalpy,
            wet_bulb_temperature_celsius,
        };
        analytics
            .is_well_formed_for(dry_bulb_temperature_celsius)
            .then_some(analytics)
    }

    /// Summarize recent room conditions
    /// Produces time-weighted comfort, availability, humidity, and equipment
    /// statistics from ordered non-overlapping periods. Invalid, overlapping,
    /// or temperature-empty input returns `None`.
    pub fn summarize_conditions(
        &self,
        periods: &[BuildingHvacPersistedRoomConditionPeriodV1],
    ) -> Option<BuildingHvacRoomStatisticsV1> {
        let first = periods.first()?;
        let mut previous_end = None;
        let mut ends_before = first.starts_at;
        let mut temperature_sample_count = 0_u64;
        let mut temperature_available_seconds = 0_u64;
        let mut minimum_temperature_celsius = f32::INFINITY;
        let mut maximum_temperature_celsius = f32::NEG_INFINITY;
        let mut weighted_temperature_sum = 0_f64;
        let mut below_degree_minutes = 0_f64;
        let mut above_degree_minutes = 0_f64;
        let mut humidity_sample_count = 0_u64;
        let mut humidity_available_seconds = 0_u64;
        let mut weighted_humidity_sum = 0_f64;
        let mut heating_active_seconds = 0_u64;
        let mut cooling_active_seconds = 0_u64;
        let mut fan_only_active_seconds = 0_u64;

        for period in periods {
            let period_end = period
                .starts_at
                .checked_add(u64::from(period.duration_seconds))?;
            if period.duration_seconds == 0
                || previous_end.is_some_and(|end| period.starts_at < end)
                || period
                    .temperature_celsius
                    .is_some_and(|value| !value.is_finite())
                || period
                    .relative_humidity_percent
                    .is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value))
                || period
                    .effective_heating_setpoint_celsius
                    .is_some_and(|value| !value.is_finite())
                || period
                    .effective_cooling_setpoint_celsius
                    .is_some_and(|value| !value.is_finite())
            {
                return None;
            }
            previous_end = Some(period_end);
            ends_before = period_end;
            let duration_seconds = u64::from(period.duration_seconds);

            if let Some(temperature) = period.temperature_celsius {
                temperature_sample_count = temperature_sample_count.saturating_add(1);
                temperature_available_seconds =
                    temperature_available_seconds.saturating_add(duration_seconds);
                minimum_temperature_celsius = minimum_temperature_celsius.min(temperature);
                maximum_temperature_celsius = maximum_temperature_celsius.max(temperature);
                weighted_temperature_sum += f64::from(temperature) * duration_seconds as f64;
                if let Some(heating_setpoint) = period.effective_heating_setpoint_celsius {
                    below_degree_minutes += f64::from((heating_setpoint - temperature).max(0.0))
                        * duration_seconds as f64
                        / 60.0;
                }
                if let Some(cooling_setpoint) = period.effective_cooling_setpoint_celsius {
                    above_degree_minutes += f64::from((temperature - cooling_setpoint).max(0.0))
                        * duration_seconds as f64
                        / 60.0;
                }
            }
            if let Some(humidity) = period.relative_humidity_percent {
                humidity_sample_count = humidity_sample_count.saturating_add(1);
                humidity_available_seconds =
                    humidity_available_seconds.saturating_add(duration_seconds);
                weighted_humidity_sum += f64::from(humidity) * duration_seconds as f64;
            }
            match period.activity {
                BuildingHvacRoomActivityV1::Heating => {
                    heating_active_seconds =
                        heating_active_seconds.saturating_add(duration_seconds);
                }
                BuildingHvacRoomActivityV1::Cooling => {
                    cooling_active_seconds =
                        cooling_active_seconds.saturating_add(duration_seconds);
                }
                BuildingHvacRoomActivityV1::FanOnly => {
                    fan_only_active_seconds =
                        fan_only_active_seconds.saturating_add(duration_seconds);
                }
                BuildingHvacRoomActivityV1::Unknown | BuildingHvacRoomActivityV1::Idle => {}
            }
        }

        if temperature_available_seconds == 0 {
            return None;
        }
        let mean_temperature_celsius =
            (weighted_temperature_sum / temperature_available_seconds as f64) as f32;
        let mean_relative_humidity_percent = (humidity_available_seconds != 0)
            .then(|| (weighted_humidity_sum / humidity_available_seconds as f64) as f32);
        let below_heating_comfort_degree_minutes_celsius = below_degree_minutes as f32;
        let above_cooling_comfort_degree_minutes_celsius = above_degree_minutes as f32;
        if !mean_temperature_celsius.is_finite()
            || mean_relative_humidity_percent.is_some_and(|value| !value.is_finite())
            || !below_heating_comfort_degree_minutes_celsius.is_finite()
            || !above_cooling_comfort_degree_minutes_celsius.is_finite()
        {
            return None;
        }

        Some(BuildingHvacRoomStatisticsV1 {
            starts_at: first.starts_at,
            ends_before,
            temperature_sample_count,
            minimum_temperature_celsius,
            mean_temperature_celsius,
            maximum_temperature_celsius,
            temperature_data_available_seconds: temperature_available_seconds,
            below_heating_comfort_degree_minutes_celsius,
            above_cooling_comfort_degree_minutes_celsius,
            humidity_sample_count,
            mean_relative_humidity_percent,
            heating_active_seconds,
            cooling_active_seconds,
            fan_only_active_seconds,
        })
    }
}

fn saturation_vapor_pressure_pascals(temperature_celsius: f32) -> Option<f32> {
    if !temperature_celsius.is_finite() || !(-100.0..=100.0).contains(&temperature_celsius) {
        return None;
    }
    let exponent = if temperature_celsius >= 0.0 {
        17.625 * temperature_celsius / (temperature_celsius + 243.04)
    } else {
        22.587 * temperature_celsius / (temperature_celsius + 273.86)
    };
    let pressure = 610.94 * exponent.exp();
    (pressure.is_finite() && pressure > 0.0).then_some(pressure)
}

fn saturation_humidity_ratio(temperature_celsius: f32, pressure_pascals: f32) -> Option<f32> {
    let saturation_pressure = saturation_vapor_pressure_pascals(temperature_celsius)?;
    if saturation_pressure >= pressure_pascals {
        return None;
    }
    let ratio = 0.621_945 * saturation_pressure / (pressure_pascals - saturation_pressure);
    (ratio.is_finite() && ratio >= 0.0).then_some(ratio)
}

fn moist_air_enthalpy_kilojoules_per_kilogram_dry_air(
    dry_bulb_temperature_celsius: f32,
    humidity_ratio: f32,
) -> f32 {
    1.006 * dry_bulb_temperature_celsius
        + humidity_ratio * (2_501.0 + 1.86 * dry_bulb_temperature_celsius)
}

fn solve_wet_bulb_temperature_celsius(
    dry_bulb_temperature_celsius: f32,
    dew_point_temperature_celsius: f32,
    pressure_pascals: f32,
    target_enthalpy: f32,
) -> Option<f32> {
    if !target_enthalpy.is_finite() {
        return None;
    }
    let mut lower = dew_point_temperature_celsius;
    let mut upper = dry_bulb_temperature_celsius;
    for _ in 0..32 {
        let candidate = (lower + upper) / 2.0;
        let saturation_humidity_ratio = saturation_humidity_ratio(candidate, pressure_pascals)?;
        let saturated_enthalpy = moist_air_enthalpy_kilojoules_per_kilogram_dry_air(
            candidate,
            saturation_humidity_ratio,
        );
        if saturated_enthalpy > target_enthalpy {
            upper = candidate;
        } else {
            lower = candidate;
        }
    }
    let wet_bulb = (lower + upper) / 2.0;
    wet_bulb.is_finite().then_some(wet_bulb)
}

fn temperature_reading_is_current(
    reading: BuildingHvacTemperatureReadingV1,
    evaluated_at: LibertasDateTime,
) -> bool {
    reading.is_well_formed()
        && reading.observed_at <= evaluated_at
        && reading.valid_until > evaluated_at
}

fn humidity_reading_is_current(
    reading: BuildingHvacHumidityReadingV1,
    evaluated_at: LibertasDateTime,
) -> bool {
    reading.is_well_formed()
        && reading.observed_at <= evaluated_at
        && reading.valid_until > evaluated_at
}

fn air_quality_reading_is_current(
    reading: &BuildingHvacAirQualityReadingV1,
    evaluated_at: LibertasDateTime,
) -> bool {
    reading.is_well_formed()
        && reading.observed_at <= evaluated_at
        && reading.valid_until > evaluated_at
}

fn robust_fused_measurement(
    values: &[(f32, LibertasDateTime)],
    maximum_distance: f32,
) -> (Option<f32>, u16, Option<LibertasDateTime>) {
    if values.is_empty() || !maximum_distance.is_finite() || maximum_distance < 0.0 {
        return (None, 0, None);
    }
    let mut sorted_values: Vec<f32> = values.iter().map(|(value, _)| *value).collect();
    sorted_values.sort_by(f32::total_cmp);
    let middle = sorted_values.len() / 2;
    let median = if sorted_values.len().is_multiple_of(2) {
        (sorted_values[middle - 1] + sorted_values[middle]) / 2.0
    } else {
        sorted_values[middle]
    };
    let contributors: Vec<(f32, LibertasDateTime)> = values
        .iter()
        .copied()
        .filter(|(value, _)| (*value - median).abs() <= maximum_distance)
        .collect();
    if contributors.is_empty() {
        return (None, 0, None);
    }
    let fused = contributors
        .iter()
        .map(|(value, _)| f64::from(*value))
        .sum::<f64>()
        / contributors.len() as f64;
    let observed_at = contributors
        .iter()
        .map(|(_, observed_at)| *observed_at)
        .max();
    (Some(fused as f32), contributors.len() as u16, observed_at)
}

/// One room considered by shared-thermostat arbitration
/// References the authoritative room protocol state and writable control, plus
/// the learned temperature change expected from other active thermostat-zones.
#[derive(Clone, Copy, Debug)]
pub struct BuildingHvacRoomControlCandidate<'a> {
    /// Room endpoint
    /// The stable room identity returned with a dominant demand.
    pub room_endpoint: LibertasEndpoint,
    /// Room control
    /// The current validated writable comfort intent.
    pub control: &'a BuildingHvacRoomControlV1,
    /// Room state
    /// The current analytics result for this room.
    pub state: &'a BuildingHvacRoomObservedStateV1,
    /// Predicted cross-zone temperature change
    /// Learned near-term temperature change in degrees Celsius expected from
    /// other zones. Positive values predict warming; negative values predict
    /// cooling.
    pub predicted_cross_zone_temperature_change_celsius: f32,
    /// Predicted machine-learning temperature change
    /// Optional bounded XGBoost prediction for the control horizon. `None`
    /// selects deterministic fallback while retaining the independently
    /// learned cross-zone contribution.
    pub predicted_machine_learning_temperature_change_celsius: Option<f32>,
}

/// Matter thermostat control limits
/// Supplies the physical thermostat's current heating/cooling bounds and
/// minimum deadband to shared-room arbitration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildingHvacThermostatControlLimits {
    /// Minimum heating setpoint
    /// Lowest supported heating target in degrees Celsius.
    pub minimum_heating_setpoint_celsius: f32,
    /// Maximum heating setpoint
    /// Highest supported heating target in degrees Celsius.
    pub maximum_heating_setpoint_celsius: f32,
    /// Minimum cooling setpoint
    /// Lowest supported cooling target in degrees Celsius.
    pub minimum_cooling_setpoint_celsius: f32,
    /// Maximum cooling setpoint
    /// Highest supported cooling target in degrees Celsius.
    pub maximum_cooling_setpoint_celsius: f32,
    /// Minimum deadband
    /// Required separation between heating and cooling setpoints in degrees
    /// Celsius.
    pub minimum_deadband_celsius: f32,
}

impl BuildingHvacThermostatControlLimits {
    /// Valid thermostat limits
    /// Returns `true` for finite ordered bounds with a nonnegative deadband and
    /// at least one feasible heating/cooling pair.
    pub fn is_well_formed(&self) -> bool {
        self.minimum_heating_setpoint_celsius.is_finite()
            && self.maximum_heating_setpoint_celsius.is_finite()
            && self.minimum_cooling_setpoint_celsius.is_finite()
            && self.maximum_cooling_setpoint_celsius.is_finite()
            && self.minimum_deadband_celsius.is_finite()
            && self.minimum_heating_setpoint_celsius <= self.maximum_heating_setpoint_celsius
            && self.minimum_cooling_setpoint_celsius <= self.maximum_cooling_setpoint_celsius
            && self.minimum_deadband_celsius >= 0.0
            && self.minimum_heating_setpoint_celsius + self.minimum_deadband_celsius
                <= self.maximum_cooling_setpoint_celsius
    }
}

/// Thermostat control hold reason
/// Explains why shared-room arbitration deliberately produces no Matter
/// setpoint write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildingHvacThermostatControlHoldReason {
    /// Invalid limits
    /// The thermostat's reported bounds or deadband are not usable.
    InvalidLimits,
    /// No trustworthy room
    /// No room has current analyzable temperature and thermostat state.
    NoTrustworthyRoom,
    /// No enabled demand
    /// Every trustworthy room explicitly disables heating and cooling demand.
    NoEnabledDemand,
    /// Already applied
    /// Current effective setpoints are within command tolerance.
    AlreadyApplied,
}

/// Shared Matter thermostat control decision
/// Contains either a deliberate hold or one bounded heating/cooling target
/// pair. A caller converts `ApplySetpoints` into generated typed Matter writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BuildingHvacThermostatControlDecision {
    /// Hold
    /// Does not write the physical thermostat.
    Hold {
        /// Hold reason
        /// The reason no command is appropriate.
        reason: BuildingHvacThermostatControlHoldReason,
    },
    /// Apply setpoints
    /// Writes one or both bounded targets to the shared thermostat.
    ApplySetpoints {
        /// Heating setpoint
        /// The shared heating target when at least one room permits heating.
        heating_setpoint_celsius: Option<f32>,
        /// Cooling setpoint
        /// The shared cooling target when at least one room permits cooling.
        cooling_setpoint_celsius: Option<f32>,
        /// Dominant heating room
        /// The room whose adjusted heating target is most demanding.
        dominant_heating_room: Option<LibertasEndpoint>,
        /// Dominant cooling room
        /// The room whose adjusted cooling target is most demanding.
        dominant_cooling_room: Option<LibertasEndpoint>,
        /// Trustworthy rooms
        /// Number of rooms that contributed valid current state.
        trustworthy_room_count: u16,
    },
}

/// Shared-thermostat control engine
/// Reconciles virtual room comfort demands into at most one bounded command for
/// each physical Matter thermostat. It never commands from unavailable room
/// state and never overrides an explicit room `Off` preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildingHvacControlEngine;

impl BuildingHvacControlEngine {
    /// New control engine
    /// Creates the stateless V1 shared-zone arbitration implementation.
    pub const fn new() -> Self {
        Self
    }

    /// Arbitrate thermostat
    /// Applies comfort-or-savings adjustment, learned cross-zone and bounded
    /// machine-learning predictions, physical setpoint limits, and deadband.
    /// Ready rooms receive full demand weight; degraded-but-trustworthy rooms
    /// receive half weight when resolving a heating/cooling conflict.
    pub fn arbitrate_thermostat(
        &self,
        thermostat: LibertasDevice,
        limits: BuildingHvacThermostatControlLimits,
        rooms: &[BuildingHvacRoomControlCandidate<'_>],
    ) -> BuildingHvacThermostatControlDecision {
        if !limits.is_well_formed() {
            return BuildingHvacThermostatControlDecision::Hold {
                reason: BuildingHvacThermostatControlHoldReason::InvalidLimits,
            };
        }

        let mut trustworthy_room_count = 0_u16;
        let mut heating: Option<(f32, f32, LibertasEndpoint)> = None;
        let mut cooling: Option<(f32, f32, LibertasEndpoint)> = None;
        let mut current_heating_setpoint = None;
        let mut current_cooling_setpoint = None;
        for room in rooms {
            if room.state.physical_thermostat != thermostat
                || room.state.data_quality == BuildingHvacRoomDataQualityV1::Unavailable
                || !room.control.is_well_formed()
                || !room
                    .predicted_cross_zone_temperature_change_celsius
                    .is_finite()
                || room
                    .predicted_machine_learning_temperature_change_celsius
                    .is_some_and(|prediction| {
                        !prediction.is_finite()
                            || prediction.abs() > BUILDING_HVAC_ML_MAXIMUM_PREDICTED_CHANGE_CELSIUS
                    })
            {
                continue;
            }
            let Some(current_temperature_celsius) = room.state.temperature_celsius else {
                continue;
            };
            if !current_temperature_celsius.is_finite() {
                continue;
            }
            let predicted_temperature_celsius = current_temperature_celsius
                + room.predicted_cross_zone_temperature_change_celsius
                + room
                    .predicted_machine_learning_temperature_change_celsius
                    .unwrap_or(0.0);
            if !predicted_temperature_celsius.is_finite() {
                continue;
            }
            trustworthy_room_count = trustworthy_room_count.saturating_add(1);
            current_heating_setpoint =
                current_heating_setpoint.or(room.state.effective_heating_setpoint_celsius);
            current_cooling_setpoint =
                current_cooling_setpoint.or(room.state.effective_cooling_setpoint_celsius);

            let adjustment = room.control.comfort_or_savings_normalized
                * BUILDING_HVAC_MAX_COMFORT_SETPOINT_ADJUSTMENT_CELSIUS;
            let quality_weight = if room.state.data_quality == BuildingHvacRoomDataQualityV1::Ready
            {
                1.0
            } else {
                0.5
            };
            if matches!(
                room.control.operating_preference,
                BuildingHvacRoomOperatingPreferenceV1::Auto
                    | BuildingHvacRoomOperatingPreferenceV1::Heat
            ) {
                let target = (room.control.preferred_heating_temperature_celsius + adjustment)
                    .clamp(
                        limits.minimum_heating_setpoint_celsius,
                        limits.maximum_heating_setpoint_celsius,
                    );
                let demand = (target - predicted_temperature_celsius).max(0.0) * quality_weight;
                if heating.is_none_or(|(current_target, current_demand, current_room)| {
                    target > current_target
                        || (target == current_target && demand > current_demand)
                        || (target == current_target
                            && demand == current_demand
                            && room.room_endpoint < current_room)
                }) {
                    heating = Some((target, demand, room.room_endpoint));
                }
            }
            if matches!(
                room.control.operating_preference,
                BuildingHvacRoomOperatingPreferenceV1::Auto
                    | BuildingHvacRoomOperatingPreferenceV1::Cool
            ) {
                let target = (room.control.preferred_cooling_temperature_celsius - adjustment)
                    .clamp(
                        limits.minimum_cooling_setpoint_celsius,
                        limits.maximum_cooling_setpoint_celsius,
                    );
                let demand = (predicted_temperature_celsius - target).max(0.0) * quality_weight;
                if cooling.is_none_or(|(current_target, current_demand, current_room)| {
                    target < current_target
                        || (target == current_target && demand > current_demand)
                        || (target == current_target
                            && demand == current_demand
                            && room.room_endpoint < current_room)
                }) {
                    cooling = Some((target, demand, room.room_endpoint));
                }
            }
        }

        if trustworthy_room_count == 0 {
            return BuildingHvacThermostatControlDecision::Hold {
                reason: BuildingHvacThermostatControlHoldReason::NoTrustworthyRoom,
            };
        }
        if heating.is_none() && cooling.is_none() {
            return BuildingHvacThermostatControlDecision::Hold {
                reason: BuildingHvacThermostatControlHoldReason::NoEnabledDemand,
            };
        }

        let mut heating_target = heating.map(|(target, _, _)| target);
        let mut cooling_target = cooling.map(|(target, _, _)| target);
        if let (Some(heat), Some(cool)) = (heating_target, cooling_target)
            && heat + limits.minimum_deadband_celsius > cool
        {
            let heating_demand = heating.map_or(0.0, |(_, demand, _)| demand);
            let cooling_demand = cooling.map_or(0.0, |(_, demand, _)| demand);
            if heating_demand >= cooling_demand {
                let adjusted_cooling = heat + limits.minimum_deadband_celsius;
                if adjusted_cooling <= limits.maximum_cooling_setpoint_celsius {
                    cooling_target = Some(adjusted_cooling);
                } else {
                    heating_target = Some(
                        (limits.maximum_cooling_setpoint_celsius - limits.minimum_deadband_celsius)
                            .clamp(
                                limits.minimum_heating_setpoint_celsius,
                                limits.maximum_heating_setpoint_celsius,
                            ),
                    );
                    cooling_target = Some(limits.maximum_cooling_setpoint_celsius);
                }
            } else {
                let adjusted_heating = cool - limits.minimum_deadband_celsius;
                if adjusted_heating >= limits.minimum_heating_setpoint_celsius {
                    heating_target = Some(adjusted_heating);
                } else {
                    heating_target = Some(limits.minimum_heating_setpoint_celsius);
                    cooling_target = Some(
                        (limits.minimum_heating_setpoint_celsius + limits.minimum_deadband_celsius)
                            .clamp(
                                limits.minimum_cooling_setpoint_celsius,
                                limits.maximum_cooling_setpoint_celsius,
                            ),
                    );
                }
            }
        }

        let heating_is_applied = setpoint_is_applied(heating_target, current_heating_setpoint);
        let cooling_is_applied = setpoint_is_applied(cooling_target, current_cooling_setpoint);
        if heating_is_applied && cooling_is_applied {
            return BuildingHvacThermostatControlDecision::Hold {
                reason: BuildingHvacThermostatControlHoldReason::AlreadyApplied,
            };
        }

        BuildingHvacThermostatControlDecision::ApplySetpoints {
            heating_setpoint_celsius: heating_target,
            cooling_setpoint_celsius: cooling_target,
            dominant_heating_room: heating.map(|(_, _, room)| room),
            dominant_cooling_room: cooling.map(|(_, _, room)| room),
            trustworthy_room_count,
        }
    }
}

fn setpoint_is_applied(calculated: Option<f32>, current: Option<f32>) -> bool {
    calculated.is_none_or(|calculated| {
        current.is_some_and(|current| {
            current.is_finite()
                && (calculated - current).abs() <= BUILDING_HVAC_SETPOINT_COMMAND_TOLERANCE_CELSIUS
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuildingHvacConfigurationError {
    NoRooms,
    TooManyRooms,
    NoThermostats,
    TooManyThermostats,
    NoUrgentNotificationRecipients,
    TooManyUrgentNotificationRecipients,
    DuplicateUrgentNotificationRecipient,
    EmptyRoomName,
    DuplicateRoomName,
    DuplicateRoomEndpoint,
    DuplicateThermostat,
    ThermostatWithoutRoom,
    TooManyRoomsForThermostat,
    RoomIndexOutOfRange,
    RoomAssignedMoreThanOnce,
    RoomNotAssigned,
    NoIndoorSensor,
    TooManyIndoorSensors,
    DuplicateIndoorSensorDevice,
    DuplicateOutdoorSensor,
}

fn validate_building_configuration(
    building: &BuildingHvacBuildingV1,
) -> Result<(), BuildingHvacConfigurationError> {
    if building.rooms.is_empty() {
        return Err(BuildingHvacConfigurationError::NoRooms);
    }
    if building.rooms.len() > BUILDING_HVAC_MAX_ROOMS {
        return Err(BuildingHvacConfigurationError::TooManyRooms);
    }
    if building.thermostats.is_empty() {
        return Err(BuildingHvacConfigurationError::NoThermostats);
    }
    if building.thermostats.len() > BUILDING_HVAC_MAX_THERMOSTATS {
        return Err(BuildingHvacConfigurationError::TooManyThermostats);
    }
    if building.urgent_notification_recipients.is_empty() {
        return Err(BuildingHvacConfigurationError::NoUrgentNotificationRecipients);
    }
    if building.urgent_notification_recipients.len()
        > BUILDING_HVAC_MAX_URGENT_NOTIFICATION_RECIPIENTS
    {
        return Err(BuildingHvacConfigurationError::TooManyUrgentNotificationRecipients);
    }
    for (index, recipient) in building.urgent_notification_recipients.iter().enumerate() {
        if building.urgent_notification_recipients[..index].contains(recipient) {
            return Err(BuildingHvacConfigurationError::DuplicateUrgentNotificationRecipient);
        }
    }

    for (index, room) in building.rooms.iter().enumerate() {
        if room.name.is_empty() {
            return Err(BuildingHvacConfigurationError::EmptyRoomName);
        }
        if building.rooms[..index]
            .iter()
            .any(|other| other.name == room.name)
        {
            return Err(BuildingHvacConfigurationError::DuplicateRoomName);
        }
        if building.rooms[..index]
            .iter()
            .any(|other| other.control_endpoint == room.control_endpoint)
        {
            return Err(BuildingHvacConfigurationError::DuplicateRoomEndpoint);
        }
    }

    let mut assigned_rooms = [false; BUILDING_HVAC_MAX_ROOMS];
    let mut sensor_devices: Vec<LibertasDevice> = Vec::new();

    for (thermostat_index, thermostat) in building.thermostats.iter().enumerate() {
        if building.thermostats[..thermostat_index]
            .iter()
            .any(|other| other.thermostat == thermostat.thermostat)
        {
            return Err(BuildingHvacConfigurationError::DuplicateThermostat);
        }
        if thermostat.rooms.is_empty() {
            return Err(BuildingHvacConfigurationError::ThermostatWithoutRoom);
        }
        if thermostat.rooms.len() > BUILDING_HVAC_MAX_ROOMS {
            return Err(BuildingHvacConfigurationError::TooManyRoomsForThermostat);
        }

        for association in &thermostat.rooms {
            let room_index = usize::from(association.room_index);
            if room_index >= building.rooms.len() {
                return Err(BuildingHvacConfigurationError::RoomIndexOutOfRange);
            }
            if assigned_rooms[room_index] {
                return Err(BuildingHvacConfigurationError::RoomAssignedMoreThanOnce);
            }
            assigned_rooms[room_index] = true;

            if association.sensors.is_empty() {
                return Err(BuildingHvacConfigurationError::NoIndoorSensor);
            }
            if association.sensors.len() > BUILDING_HVAC_MAX_SENSORS_PER_ROOM {
                return Err(BuildingHvacConfigurationError::TooManyIndoorSensors);
            }

            for sensor in &association.sensors {
                for device in [
                    Some(sensor.temperature_sensor),
                    sensor.humidity_sensor,
                    sensor.air_quality_sensor,
                ]
                .into_iter()
                .flatten()
                {
                    if sensor_devices.contains(&device)
                        || building
                            .thermostats
                            .iter()
                            .any(|thermostat| thermostat.thermostat == device)
                    {
                        return Err(BuildingHvacConfigurationError::DuplicateIndoorSensorDevice);
                    }
                    sensor_devices.push(device);
                }
            }
        }
    }

    if assigned_rooms[..building.rooms.len()]
        .iter()
        .any(|assigned| !assigned)
    {
        return Err(BuildingHvacConfigurationError::RoomNotAssigned);
    }

    if let Some(outdoor_sensor) = building.outdoor_sensor {
        for device in [
            Some(outdoor_sensor.temperature_sensor),
            outdoor_sensor.humidity_sensor,
            outdoor_sensor.air_quality_sensor,
        ]
        .into_iter()
        .flatten()
        {
            if sensor_devices.contains(&device)
                || building
                    .thermostats
                    .iter()
                    .any(|thermostat| thermostat.thermostat == device)
            {
                return Err(BuildingHvacConfigurationError::DuplicateOutdoorSensor);
            }
            sensor_devices.push(device);
        }
    }

    Ok(())
}

const BUILDING_HVAC_ML_MODELS_RESOURCE: &str = "HVAC_ML_MODELS";

struct BuildingHvacMachineLearningWakeupContext {
    client: BuildingHvacMachineLearningClient,
    results: Receiver<BuildingHvacMachineLearningResult>,
    model_sets: Vec<BuildingHvacMachineLearningModelSetV1>,
}

struct BuildingHvacMachineLearningShutdownContext {
    client: BuildingHvacMachineLearningClient,
}

fn restore_machine_learning_models(
    building: &BuildingHvacBuildingV1,
) -> Vec<BuildingHvacMachineLearningModelSetV1> {
    building
        .rooms
        .iter()
        .map(|room| {
            let key = [NotificationArgument::Object(room.control_endpoint)];
            if let Some(BuildingHvacPersistentDataV1::MachineLearningModelsV1 { models }) =
                libertas_data_read(BUILDING_HVAC_ML_MODELS_RESOURCE, &key)
                && models.room_endpoint == room.control_endpoint
                && models.is_well_formed()
            {
                return models;
            }

            let models = BuildingHvacMachineLearningModelSetV1::empty(room.control_endpoint);
            let value = BuildingHvacPersistentDataV1::MachineLearningModelsV1 {
                models: models.clone(),
            };
            libertas_data_write(BUILDING_HVAC_ML_MODELS_RESOURCE, &key, &value);
            models
        })
        .collect()
}

fn handle_machine_learning_wakeup(context: &mut Box<dyn Any>) {
    let context = context
        .downcast_mut::<BuildingHvacMachineLearningWakeupContext>()
        .expect("invalid smart building HVAC machine-learning wake-up context");
    while let Ok(result) = context.results.try_recv() {
        match result {
            BuildingHvacMachineLearningResult::Candidate(candidate) => {
                let Some(current) = context
                    .model_sets
                    .iter()
                    .find(|models| models.room_endpoint == candidate.room_endpoint)
                else {
                    libertas_log(
                        LogLevel::Warn,
                        "Smart building HVAC rejected an XGBoost candidate for an unknown room",
                    );
                    continue;
                };
                let mut updated = current.clone();
                if !updated.promote(candidate.clone()) {
                    libertas_log(
                        LogLevel::Warn,
                        "Smart building HVAC rejected an invalid XGBoost candidate",
                    );
                    continue;
                }

                let value = BuildingHvacPersistentDataV1::MachineLearningModelsV1 {
                    models: updated.clone(),
                };
                let key = [NotificationArgument::Object(updated.room_endpoint)];
                libertas_data_write(BUILDING_HVAC_ML_MODELS_RESOURCE, &key, &value);
                if let Some(current) = context
                    .model_sets
                    .iter_mut()
                    .find(|models| models.room_endpoint == updated.room_endpoint)
                {
                    *current = updated;
                }
                if context.client.try_activate(candidate).is_err() {
                    libertas_log(
                        LogLevel::Warn,
                        "Smart building HVAC persisted an XGBoost model but could not activate it; it will be restored after restart",
                    );
                }
            }
            BuildingHvacMachineLearningResult::TrainingRejected { horizon, reason } => {
                let message = format!(
                    "Smart building HVAC XGBoost training rejected for {horizon:?}: {reason:?}"
                );
                libertas_log(LogLevel::Warn, &message);
            }
            BuildingHvacMachineLearningResult::Prediction { .. } => {
                // The live Matter/weather runtime will correlate predictions
                // when it submits work. The worker remains independently
                // usable and deterministic fallback is explicit in the result.
            }
        }
    }
}

fn handle_machine_learning_shutdown(context: &mut Box<dyn Any>) {
    let context = context
        .downcast_mut::<BuildingHvacMachineLearningShutdownContext>()
        .expect("invalid smart building HVAC machine-learning shutdown context");
    if matches!(
        context.client.request_shutdown(),
        Err(BuildingHvacMachineLearningQueueError::Disconnected)
    ) {
        libertas_shutdown_complete();
    }
}

/// Libertas smart building HVAC
/// Configures a room-first building topology and its dedicated building-HVAC
/// weather client. Room endpoints expose writable comfort intent and read-only
/// indoor and outdoor sensor state, statistics, learned cross-zone influence,
/// calculated schedules, and active urgent HVAC warnings. Selected Libertas
/// users receive time-sensitive supervisory notifications; this application
/// does not generate certified life-safety alarms. The runtime protocol and
/// persistent union are design contracts. Pure analytics, shared-thermostat
/// control, learning, and urgent-notification algorithms are implemented; this
/// entry function validates configuration and starts the bounded statically
/// linked XGBoost worker with persisted accepted models. Matter and weather
/// listeners remain separate runtime integration work.
#[libertas_data_schema(BuildingHvacPersistentDataV1)]
#[libertas_string_resources(APP_STRINGS)]
pub fn libertas_smart_building_hvac(
    /*
     * Building
     * Define rooms first, then select those rooms from each Matter thermostat
     * using the room-name EnumSource. Every room must be assigned exactly once
     * and must have at least one indoor station with a Matter Temperature
     * Sensor. Each station may add Matter Humidity and Air Quality Sensor
     * logical devices whose optional measurements are discovered at runtime.
     * Select at least one user to receive urgent HVAC warnings and recovery
     * notifications.
     */
    building: BuildingHvacBuildingV1,
    /*
     * Building HVAC weather
     * The special BuildingHvacWeatherProtocolV1 client endpoint expected from
     * libertas-weather_server.
     */
    weather: BuildingHvacWeatherClientV1,
) {
    if validate_building_configuration(&building).is_err() {
        libertas_log(
            LogLevel::Error,
            "Smart building HVAC configuration is invalid",
        );
        return;
    }

    let model_sets = restore_machine_learning_models(&building);
    let active_models: Vec<_> = model_sets
        .iter()
        .flat_map(BuildingHvacMachineLearningModelSetV1::active_models)
        .cloned()
        .collect();
    let (machine_learning, machine_learning_results) =
        match start_machine_learning_worker(libertas_wake_up, libertas_shutdown_complete) {
            Ok(runtime) => runtime,
            Err(error) => {
                libertas_log(LogLevel::Error, &error);
                let _ = weather;
                return;
            }
        };
    libertas_register_wakeup_callback(
        handle_machine_learning_wakeup,
        Box::new(BuildingHvacMachineLearningWakeupContext {
            client: machine_learning.clone(),
            results: machine_learning_results,
            model_sets,
        }),
    );
    libertas_register_shutdown_handler(
        handle_machine_learning_shutdown,
        Box::new(BuildingHvacMachineLearningShutdownContext {
            client: machine_learning.clone(),
        }),
    );
    for model in active_models {
        if machine_learning.try_activate(model).is_err() {
            libertas_log(
                LogLevel::Warn,
                "Smart building HVAC could not queue an accepted XGBoost model for activation",
            );
        }
    }

    let _ = weather;
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{string::ToString, vec};
    use libertas::{
        AvroDecode, NotificationArgument, libertas_formatted_text, libertas_formatted_text_decode,
    };

    macro_rules! assert_round_trip {
        ($type:ty, $value:expr) => {{
            let value: $type = $value;
            let encoded = value.to_avro();
            let mut offset = 0;
            let decoded = <$type>::avro_decode(&encoded, &mut offset).unwrap();
            assert_eq!(decoded, value);
            assert_eq!(offset, encoded.len());
        }};
    }

    fn room(name: &str, control_endpoint: LibertasEndpoint) -> BuildingHvacRoomV1 {
        BuildingHvacRoomV1 {
            name: name.to_string(),
            control_endpoint,
        }
    }

    fn association(
        room_index: u16,
        temperature_sensor_id: LibertasDevice,
        humidity_sensor_id: LibertasDevice,
        air_quality_sensor_id: LibertasDevice,
    ) -> BuildingHvacThermostatRoomV1 {
        BuildingHvacThermostatRoomV1 {
            room_index,
            sensors: vec![BuildingHvacIndoorSensorV1 {
                temperature_sensor: temperature_sensor_id,
                humidity_sensor: Some(humidity_sensor_id),
                air_quality_sensor: Some(air_quality_sensor_id),
            }],
        }
    }

    fn building() -> BuildingHvacBuildingV1 {
        BuildingHvacBuildingV1 {
            rooms: vec![room("Living room", 100), room("Bedroom", 101)],
            thermostats: vec![BuildingHvacThermostatV1 {
                thermostat: 200,
                rooms: vec![association(0, 300, 400, 450), association(1, 301, 401, 451)],
            }],
            outdoor_sensor: None,
            urgent_notification_recipients: vec![700, 701],
        }
    }

    fn local_outdoor_temperature() -> BuildingHvacTemperatureReadingV1 {
        BuildingHvacTemperatureReadingV1 {
            observed_at: 1_785_059_200,
            valid_until: 1_785_059_290,
            temperature_celsius: 28.75,
        }
    }

    fn local_outdoor_humidity() -> BuildingHvacHumidityReadingV1 {
        BuildingHvacHumidityReadingV1 {
            observed_at: 1_785_059_200,
            valid_until: 1_785_059_290,
            relative_humidity_percent: 48.5,
        }
    }

    fn local_outdoor_air_quality() -> BuildingHvacAirQualityReadingV1 {
        BuildingHvacAirQualityReadingV1 {
            observed_at: 1_785_059_200,
            valid_until: 1_785_059_290,
            overall_air_quality: Some(BuildingHvacAirQualityV1::Good),
            measurements: vec![
                BuildingHvacAirMeasurementV1 {
                    kind: BuildingHvacAirMeasurementKindV1::CarbonDioxide,
                    measured_value_in_reported_unit: 421.0,
                    reported_unit: BuildingHvacAirMeasurementUnitV1::PartsPerMillion,
                    level: Some(BuildingHvacConcentrationLevelV1::Low),
                },
                BuildingHvacAirMeasurementV1 {
                    kind: BuildingHvacAirMeasurementKindV1::ParticulateMatter2_5,
                    measured_value_in_reported_unit: 7.0,
                    reported_unit: BuildingHvacAirMeasurementUnitV1::MicrogramsPerCubicMeter,
                    level: Some(BuildingHvacConcentrationLevelV1::Low),
                },
            ],
        }
    }

    fn local_outdoor_sensor_state() -> BuildingHvacLocalOutdoorSensorStateV1 {
        BuildingHvacLocalOutdoorSensorStateV1 {
            temperature: Some(local_outdoor_temperature()),
            humidity: Some(local_outdoor_humidity()),
            air_quality: Some(local_outdoor_air_quality()),
        }
    }

    fn indoor_sensor_state() -> BuildingHvacIndoorSensorStateV1 {
        BuildingHvacIndoorSensorStateV1 {
            temperature_sensor: 300,
            temperature: Some(BuildingHvacTemperatureReadingV1 {
                observed_at: 1_785_059_200,
                valid_until: 1_785_059_290,
                temperature_celsius: 22.4,
            }),
            humidity_sensor: Some(400),
            humidity: Some(BuildingHvacHumidityReadingV1 {
                observed_at: 1_785_059_200,
                valid_until: 1_785_059_290,
                relative_humidity_percent: 46.0,
            }),
            air_quality_sensor: Some(450),
            air_quality: Some(local_outdoor_air_quality()),
        }
    }

    fn observed_state() -> BuildingHvacRoomObservedStateV1 {
        BuildingHvacRoomObservedStateV1 {
            data_quality: BuildingHvacRoomDataQualityV1::Ready,
            observed_at: Some(1_785_059_200),
            temperature_celsius: Some(22.4),
            relative_humidity_percent: Some(46.0),
            effective_heating_setpoint_celsius: Some(20.0),
            effective_cooling_setpoint_celsius: Some(24.0),
            activity: BuildingHvacRoomActivityV1::Idle,
            physical_thermostat: 200,
            sensor_states: vec![indoor_sensor_state()],
            fresh_temperature_sensor_count: 1,
            configured_temperature_sensor_count: 1,
            fresh_humidity_sensor_count: 1,
            configured_humidity_sensor_count: 1,
        }
    }

    fn statistics() -> BuildingHvacRoomStatisticsV1 {
        BuildingHvacRoomStatisticsV1 {
            starts_at: 1_784_972_800,
            ends_before: 1_785_059_200,
            temperature_sample_count: 288,
            minimum_temperature_celsius: 19.7,
            mean_temperature_celsius: 21.8,
            maximum_temperature_celsius: 24.3,
            temperature_data_available_seconds: 84_900,
            below_heating_comfort_degree_minutes_celsius: 7.5,
            above_cooling_comfort_degree_minutes_celsius: 2.0,
            humidity_sample_count: 288,
            mean_relative_humidity_percent: Some(47.2),
            heating_active_seconds: 18_000,
            cooling_active_seconds: 7_200,
            fan_only_active_seconds: 1_800,
        }
    }

    fn formatted_schedule() -> Vec<u8> {
        libertas_formatted_text(
            "HVAC_ROOM_SCHEDULE",
            &[NotificationArgument::LiteralText(
                "Next 15 minutes: maintain 20.0–24.0 °C for room comfort",
            )],
        )
    }

    fn formatted_room_status() -> Vec<u8> {
        libertas_formatted_text(
            "HVAC_ROOM_STATUS",
            &[
                NotificationArgument::LiteralText("22.4 °C and 46% RH"),
                NotificationArgument::LiteralText("idle"),
                NotificationArgument::LiteralText("good"),
            ],
        )
    }

    fn formatted_revision_conflict() -> Vec<u8> {
        libertas_formatted_text("HVAC_CONTROL_REVISION_CONFLICT", &[])
    }

    fn plan() -> BuildingHvacRoomPlanV1 {
        BuildingHvacRoomPlanV1 {
            formatted_schedule: formatted_schedule(),
            calculated_at: 1_785_059_200,
            valid_until: 1_785_062_800,
            periods: vec![BuildingHvacRoomPlanPeriodV1 {
                starts_at: 1_785_059_200,
                duration_seconds: 900,
                heating_setpoint_celsius: Some(20.0),
                cooling_setpoint_celsius: Some(24.0),
                reason: BuildingHvacRoomPlanReasonV1::RoomComfort,
            }],
        }
    }

    fn active_urgent_condition() -> BuildingHvacActiveUrgentConditionV1 {
        BuildingHvacActiveUrgentConditionV1 {
            condition: BuildingHvacUrgentConditionV1::FreezeRisk,
            severity: BuildingHvacUrgentNotificationSeverityV1::Severe,
            active_since: 1_785_059_200,
            updated_at: 1_785_059_500,
            temperature_celsius: Some(4.5),
            last_notification_at: Some(1_785_059_500),
        }
    }

    fn persisted_urgent_condition() -> BuildingHvacPersistedUrgentConditionV1 {
        BuildingHvacPersistedUrgentConditionV1 {
            condition: BuildingHvacUrgentConditionV1::FreezeRisk,
            phase: BuildingHvacUrgentConditionPhaseV1::Active,
            condition_started_at: 1_785_059_200,
            phase_started_at: 1_785_059_500,
            updated_at: 1_785_059_500,
            last_temperature_celsius: Some(4.5),
            last_notification_at: Some(1_785_059_500),
        }
    }

    fn urgent_room_state(
        observed_at: LibertasDateTime,
        temperature_celsius: Option<f32>,
        data_quality: BuildingHvacRoomDataQualityV1,
        activity: BuildingHvacRoomActivityV1,
    ) -> BuildingHvacRoomObservedStateV1 {
        let mut state = observed_state();
        state.observed_at = Some(observed_at);
        state.temperature_celsius = temperature_celsius;
        state.data_quality = data_quality;
        state.activity = activity;
        state
    }

    fn recovery_periods(
        ends_before: LibertasDateTime,
        activity: BuildingHvacRoomActivityV1,
        temperature_celsius: f32,
    ) -> Vec<BuildingHvacPersistedRoomConditionPeriodV1> {
        let starts_at = ends_before - u64::from(BUILDING_HVAC_NOT_RECOVERING_OBSERVATION_SECONDS);
        (0..4)
            .map(|index| BuildingHvacPersistedRoomConditionPeriodV1 {
                starts_at: starts_at + index * 900,
                duration_seconds: 900,
                temperature_celsius: Some(temperature_celsius),
                relative_humidity_percent: Some(45.0),
                activity,
                effective_heating_setpoint_celsius: Some(20.0),
                effective_cooling_setpoint_celsius: Some(24.0),
                outdoor_dry_bulb_temperature_celsius: Some(0.0),
            })
            .collect()
    }

    fn room_data() -> BuildingHvacRoomProtocolV1 {
        BuildingHvacRoomProtocolV1::RoomDataV1 {
            formatted_room_status: formatted_room_status(),
            maximum_wait_interval_seconds: BUILDING_HVAC_ROOM_MAXIMUM_WAIT_INTERVAL_SECONDS,
            control_revision: 3,
            control: BuildingHvacRoomControlV1::default(),
            state: Box::new(observed_state()),
            active_urgent_conditions: vec![active_urgent_condition()],
            local_outdoor_sensor: Some(Box::new(local_outdoor_sensor_state())),
            outdoor_air_analytics: BuildingHvacAnalyticsEngine::new()
                .analyze_outdoor_air(1_785_059_200, &current_weather()),
            statistics: Some(Box::new(statistics())),
            passive_outdoor_temperature_coupling_per_hour: Some(0.1),
            passive_model_confidence_normalized: 0.6,
            cross_zone_influences: vec![BuildingHvacCrossZoneInfluenceV1 {
                source_thermostat: 201,
                heating_temperature_rise_celsius_per_runtime_hour: Some(0.8),
                heating_confidence_normalized: 0.7,
                cooling_temperature_drop_celsius_per_runtime_hour: Some(0.6),
                cooling_confidence_normalized: 0.5,
                learned_at: Some(1_785_059_200),
            }],
            machine_learning: BuildingHvacRoomMachineLearningV1::default(),
            plan: Some(Box::new(plan())),
        }
    }

    fn persisted_condition() -> BuildingHvacPersistedRoomConditionPeriodV1 {
        BuildingHvacPersistedRoomConditionPeriodV1 {
            starts_at: 1_785_058_300,
            duration_seconds: 900,
            temperature_celsius: Some(22.3),
            relative_humidity_percent: Some(46.0),
            activity: BuildingHvacRoomActivityV1::Idle,
            effective_heating_setpoint_celsius: Some(20.0),
            effective_cooling_setpoint_celsius: Some(24.0),
            outdoor_dry_bulb_temperature_celsius: Some(29.0),
        }
    }

    fn outdoor_conditions() -> BuildingHvacOutdoorConditionsV1 {
        BuildingHvacOutdoorConditionsV1 {
            dry_bulb_temperature_celsius: 29.0,
            dew_point_temperature_celsius: 17.0,
            relative_humidity_percent: 48,
            surface_pressure_hectopascals: 1_010.0,
            wind_speed_meters_per_second: 3.0,
            wind_gust_meters_per_second: 5.0,
            wind_direction_degrees: 180,
            precipitation_millimeters: 0.0,
            precipitation_kind: BuildingHvacPrecipitationKindV1::None,
            global_horizontal_irradiance_watts_per_square_meter: 600.0,
            direct_normal_irradiance_watts_per_square_meter: 700.0,
            diffuse_horizontal_irradiance_watts_per_square_meter: 120.0,
        }
    }

    fn weather_history() -> BuildingHvacWeatherHistoryV1 {
        BuildingHvacWeatherHistoryV1 {
            retrieved_at: 1_785_059_200,
            valid_until: 1_785_066_400,
            periods: vec![BuildingHvacWeatherHistoryPeriodV1 {
                starts_at: 1_785_055_600,
                duration_seconds: 3_600,
                conditions: outdoor_conditions(),
            }],
        }
    }

    fn current_weather() -> BuildingHvacCurrentWeatherV1 {
        BuildingHvacCurrentWeatherV1 {
            retrieved_at: 1_785_059_200,
            valid_until: 1_785_061_000,
            valid_at: 1_785_059_200,
            interval_seconds: 900,
            conditions: outdoor_conditions(),
        }
    }

    fn weather_forecast() -> BuildingHvacWeatherForecastV1 {
        BuildingHvacWeatherForecastV1 {
            retrieved_at: 1_785_059_200,
            valid_until: 1_785_070_000,
            periods: vec![BuildingHvacWeatherForecastPeriodV1 {
                starts_at: 1_785_059_200,
                duration_seconds: 900,
                precipitation_probability_percent: 10,
                conditions: outdoor_conditions(),
            }],
        }
    }

    fn outdoor_air_quality() -> BuildingHvacOutdoorAirQualityV1 {
        BuildingHvacOutdoorAirQualityV1 {
            retrieved_at: 1_785_059_200,
            valid_until: 1_785_066_400,
            periods: vec![BuildingHvacOutdoorAirQualityPeriodV1 {
                starts_at: 1_785_059_200,
                duration_seconds: 3_600,
                particulate_matter_2_5_micrograms_per_cubic_meter: 7.0,
                particulate_matter_10_micrograms_per_cubic_meter: 12.0,
                ozone_micrograms_per_cubic_meter: 55.0,
                nitrogen_dioxide_micrograms_per_cubic_meter: 18.0,
            }],
        }
    }

    fn room_learning() -> BuildingHvacRoomLearningStateV1 {
        let mut learning = BuildingHvacRoomLearningStateV1 {
            passive_outdoor_coupling: BuildingHvacOnlineRegressionStateV1::empty(),
            cross_zone_learners: Vec::new(),
        };
        for step in 0..4 {
            assert!(learning.observe_identifiable_cross_zone_period(
                1_785_059_200 + step * 900,
                200,
                BuildingHvacRoomActivityV1::Idle,
                201,
                BuildingHvacRoomActivityV1::Heating,
                1,
                900,
                1.0,
                0.2,
                0.0,
                1.0,
            ));
        }
        learning
    }

    fn machine_learning_features(
        room_temperature_celsius: f32,
    ) -> BuildingHvacMachineLearningFeaturesV1 {
        BuildingHvacMachineLearningFeaturesV1 {
            room_temperature_celsius,
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

    fn machine_learning_sample() -> BuildingHvacMachineLearningSampleV1 {
        BuildingHvacMachineLearningSampleV1 {
            observed_at: 1_785_059_200,
            room_endpoint: 100,
            features: machine_learning_features(20.0),
            temperature_change_15_minutes_celsius: Some(0.2),
            temperature_change_30_minutes_celsius: Some(0.35),
            temperature_change_60_minutes_celsius: Some(0.6),
        }
    }

    fn persistent_values() -> [BuildingHvacPersistentDataV1; 14] {
        [
            BuildingHvacPersistentDataV1::RoomControlV1 {
                control_revision: 3,
                control: BuildingHvacRoomControlV1::default(),
            },
            BuildingHvacPersistentDataV1::RoomStatisticsV1 {
                statistics: statistics(),
                recent_conditions: vec![persisted_condition()],
            },
            BuildingHvacPersistentDataV1::RoomLearningV1 {
                learning: room_learning(),
            },
            BuildingHvacPersistentDataV1::RoomSensorStateV1 {
                sensors: vec![indoor_sensor_state()],
            },
            BuildingHvacPersistentDataV1::LocalOutdoorTemperatureV1 {
                temperature: local_outdoor_temperature(),
            },
            BuildingHvacPersistentDataV1::LocalOutdoorHumidityV1 {
                humidity: local_outdoor_humidity(),
            },
            BuildingHvacPersistentDataV1::LocalOutdoorAirQualityV1 {
                air_quality: local_outdoor_air_quality(),
            },
            BuildingHvacPersistentDataV1::WeatherHistoryV1 {
                history: weather_history(),
            },
            BuildingHvacPersistentDataV1::WeatherCurrentV1 {
                current: current_weather(),
            },
            BuildingHvacPersistentDataV1::WeatherForecastV1 {
                forecast: weather_forecast(),
            },
            BuildingHvacPersistentDataV1::OutdoorAirQualityV1 {
                outdoor_air_quality: outdoor_air_quality(),
            },
            BuildingHvacPersistentDataV1::RoomUrgentNotificationStateV1 {
                conditions: vec![persisted_urgent_condition()],
            },
            BuildingHvacPersistentDataV1::MachineLearningModelsV1 {
                models: BuildingHvacMachineLearningModelSetV1::empty(100),
            },
            BuildingHvacPersistentDataV1::MachineLearningSampleV1 {
                sample: machine_learning_sample(),
            },
        ]
    }

    #[test]
    fn configuration_accepts_multiple_rooms_per_thermostat() {
        assert_eq!(validate_building_configuration(&building()), Ok(()));

        let mut temperature_only_station = building();
        temperature_only_station.thermostats[0].rooms[0].sensors[0].humidity_sensor = None;
        temperature_only_station.thermostats[0].rooms[0].sensors[0].air_quality_sensor = None;
        assert_eq!(
            validate_building_configuration(&temperature_only_station),
            Ok(())
        );
    }

    #[test]
    fn configuration_requires_unique_urgent_notification_recipients() {
        let mut missing = building();
        missing.urgent_notification_recipients.clear();
        assert_eq!(
            validate_building_configuration(&missing),
            Err(BuildingHvacConfigurationError::NoUrgentNotificationRecipients)
        );

        let mut duplicate = building();
        duplicate.urgent_notification_recipients = vec![700, 700];
        assert_eq!(
            validate_building_configuration(&duplicate),
            Err(BuildingHvacConfigurationError::DuplicateUrgentNotificationRecipient)
        );

        let mut too_many = building();
        too_many.urgent_notification_recipients =
            (0..=BUILDING_HVAC_MAX_URGENT_NOTIFICATION_RECIPIENTS as u32).collect();
        assert_eq!(
            validate_building_configuration(&too_many),
            Err(BuildingHvacConfigurationError::TooManyUrgentNotificationRecipients)
        );
    }

    #[test]
    fn configuration_accepts_required_outdoor_temperature_and_optional_capabilities() {
        let mut temperature_only = building();
        temperature_only.outdoor_sensor = Some(BuildingHvacOutdoorSensorV1 {
            temperature_sensor: 500,
            humidity_sensor: None,
            air_quality_sensor: None,
        });
        assert_eq!(validate_building_configuration(&temperature_only), Ok(()));

        let mut complete_station = building();
        complete_station.outdoor_sensor = Some(BuildingHvacOutdoorSensorV1 {
            temperature_sensor: 500,
            humidity_sensor: Some(501),
            air_quality_sensor: Some(502),
        });
        assert_eq!(validate_building_configuration(&complete_station), Ok(()));
    }

    #[test]
    fn configuration_rejects_reused_outdoor_station_devices() {
        let mut reused_indoor_sensor = building();
        reused_indoor_sensor.outdoor_sensor = Some(BuildingHvacOutdoorSensorV1 {
            temperature_sensor: 300,
            humidity_sensor: None,
            air_quality_sensor: None,
        });
        assert_eq!(
            validate_building_configuration(&reused_indoor_sensor),
            Err(BuildingHvacConfigurationError::DuplicateOutdoorSensor)
        );

        let mut reused_station_sensor = building();
        reused_station_sensor.outdoor_sensor = Some(BuildingHvacOutdoorSensorV1 {
            temperature_sensor: 500,
            humidity_sensor: Some(501),
            air_quality_sensor: Some(501),
        });
        assert_eq!(
            validate_building_configuration(&reused_station_sensor),
            Err(BuildingHvacConfigurationError::DuplicateOutdoorSensor)
        );
    }

    #[test]
    fn configuration_requires_every_room_exactly_once() {
        let mut missing = building();
        missing.thermostats[0].rooms.pop();
        assert_eq!(
            validate_building_configuration(&missing),
            Err(BuildingHvacConfigurationError::RoomNotAssigned)
        );

        let mut duplicate = building();
        duplicate.thermostats.push(BuildingHvacThermostatV1 {
            thermostat: 201,
            rooms: vec![association(0, 302, 402, 452)],
        });
        assert_eq!(
            validate_building_configuration(&duplicate),
            Err(BuildingHvacConfigurationError::RoomAssignedMoreThanOnce)
        );
    }

    #[test]
    fn configuration_requires_an_indoor_sensor_for_every_room() {
        let mut value = building();
        value.thermostats[0].rooms[1].sensors.clear();

        assert_eq!(
            validate_building_configuration(&value),
            Err(BuildingHvacConfigurationError::NoIndoorSensor)
        );
    }

    #[test]
    fn configuration_rejects_out_of_range_room_references() {
        let mut value = building();
        value.thermostats[0].rooms[1].room_index = 2;

        assert_eq!(
            validate_building_configuration(&value),
            Err(BuildingHvacConfigurationError::RoomIndexOutOfRange)
        );
    }

    #[test]
    fn configuration_rejects_reused_devices_and_endpoints() {
        let mut duplicate_thermostat = building();
        duplicate_thermostat
            .thermostats
            .push(BuildingHvacThermostatV1 {
                thermostat: 200,
                rooms: vec![association(0, 302, 402, 452)],
            });
        assert_eq!(
            validate_building_configuration(&duplicate_thermostat),
            Err(BuildingHvacConfigurationError::DuplicateThermostat)
        );

        let mut duplicate_sensor = building();
        duplicate_sensor.thermostats[0].rooms[1].sensors[0].temperature_sensor = 300;
        assert_eq!(
            validate_building_configuration(&duplicate_sensor),
            Err(BuildingHvacConfigurationError::DuplicateIndoorSensorDevice)
        );

        let mut duplicate_station_capability = building();
        duplicate_station_capability.thermostats[0].rooms[0].sensors[0].air_quality_sensor =
            Some(400);
        assert_eq!(
            validate_building_configuration(&duplicate_station_capability),
            Err(BuildingHvacConfigurationError::DuplicateIndoorSensorDevice)
        );

        let mut duplicate_endpoint = building();
        duplicate_endpoint.rooms[1].control_endpoint = 100;
        assert_eq!(
            validate_building_configuration(&duplicate_endpoint),
            Err(BuildingHvacConfigurationError::DuplicateRoomEndpoint)
        );
    }

    #[test]
    fn room_control_validation_rejects_invalid_numeric_data() {
        assert!(BuildingHvacRoomControlV1::default().is_well_formed());

        assert!(
            !BuildingHvacRoomControlV1 {
                preferred_heating_temperature_celsius: 24.0,
                preferred_cooling_temperature_celsius: 20.0,
                ..BuildingHvacRoomControlV1::default()
            }
            .is_well_formed()
        );
        assert!(
            !BuildingHvacRoomControlV1 {
                comfort_or_savings_normalized: f32::NAN,
                ..BuildingHvacRoomControlV1::default()
            }
            .is_well_formed()
        );
        assert!(
            !BuildingHvacRoomControlV1 {
                comfort_or_savings_normalized: 1.1,
                ..BuildingHvacRoomControlV1::default()
            }
            .is_well_formed()
        );
        assert!(
            !BuildingHvacRoomControlV1 {
                preferred_cooling_temperature_celsius: 100.1,
                ..BuildingHvacRoomControlV1::default()
            }
            .is_well_formed()
        );
    }

    #[test]
    fn indoor_and_outdoor_air_measurements_are_queryable_and_validated() {
        let reading = local_outdoor_air_quality();
        assert!(reading.is_well_formed());
        assert_eq!(
            reading
                .measurement(BuildingHvacAirMeasurementKindV1::ParticulateMatter2_5)
                .unwrap()
                .measured_value_in_reported_unit,
            7.0
        );
        assert_eq!(
            reading.measurement(BuildingHvacAirMeasurementKindV1::Ozone),
            None
        );

        let mut duplicate = reading.clone();
        duplicate.measurements.push(duplicate.measurements[1]);
        assert!(!duplicate.is_well_formed());

        let mut nonfinite = reading;
        nonfinite.measurements[0].measured_value_in_reported_unit = f32::NAN;
        assert!(!nonfinite.is_well_formed());

        assert!(local_outdoor_temperature().is_well_formed());
        assert!(local_outdoor_humidity().is_well_formed());

        let indoor = indoor_sensor_state();
        assert!(indoor.is_well_formed());
        assert_eq!(
            indoor
                .air_measurement(BuildingHvacAirMeasurementKindV1::CarbonDioxide)
                .unwrap()
                .measured_value_in_reported_unit,
            421.0
        );
    }

    #[test]
    fn public_configuration_and_runtime_shapes_round_trip_through_avro() {
        assert_round_trip!(BuildingHvacBuildingV1, building());
        assert_round_trip!(
            BuildingHvacIndoorSensorV1,
            BuildingHvacIndoorSensorV1 {
                temperature_sensor: 300,
                humidity_sensor: Some(400),
                air_quality_sensor: Some(450),
            }
        );
        assert_round_trip!(
            BuildingHvacOutdoorSensorV1,
            BuildingHvacOutdoorSensorV1 {
                temperature_sensor: 500,
                humidity_sensor: Some(501),
                air_quality_sensor: Some(502),
            }
        );
        assert_round_trip!(
            BuildingHvacWeatherClientV1,
            BuildingHvacWeatherClientV1 { endpoint: 500 }
        );
        assert_round_trip!(
            BuildingHvacRoomOperatingPreferenceV1,
            BuildingHvacRoomOperatingPreferenceV1::Auto
        );
        assert_round_trip!(
            BuildingHvacRoomControlV1,
            BuildingHvacRoomControlV1::default()
        );
        assert_round_trip!(
            BuildingHvacRoomDataQualityV1,
            BuildingHvacRoomDataQualityV1::Degraded
        );
        assert_round_trip!(BuildingHvacAirQualityV1, BuildingHvacAirQualityV1::Moderate);
        assert_round_trip!(
            BuildingHvacConcentrationLevelV1,
            BuildingHvacConcentrationLevelV1::High
        );
        assert_round_trip!(
            BuildingHvacAirMeasurementKindV1,
            BuildingHvacAirMeasurementKindV1::ParticulateMatter2_5
        );
        assert_round_trip!(
            BuildingHvacAirMeasurementUnitV1,
            BuildingHvacAirMeasurementUnitV1::MicrogramsPerCubicMeter
        );
        assert_round_trip!(
            BuildingHvacAirMeasurementV1,
            local_outdoor_air_quality().measurements[0]
        );
        assert_round_trip!(
            BuildingHvacTemperatureReadingV1,
            local_outdoor_temperature()
        );
        assert_round_trip!(BuildingHvacHumidityReadingV1, local_outdoor_humidity());
        assert_round_trip!(BuildingHvacAirQualityReadingV1, local_outdoor_air_quality());
        assert_round_trip!(
            BuildingHvacLocalOutdoorSensorStateV1,
            local_outdoor_sensor_state()
        );
        assert_round_trip!(BuildingHvacIndoorSensorStateV1, indoor_sensor_state());
        assert_round_trip!(
            BuildingHvacRoomActivityV1,
            BuildingHvacRoomActivityV1::Heating
        );
        assert_round_trip!(BuildingHvacRoomObservedStateV1, observed_state());
        assert_round_trip!(BuildingHvacRoomStatisticsV1, statistics());
        assert_round_trip!(
            BuildingHvacRoomPlanReasonV1,
            BuildingHvacRoomPlanReasonV1::WeatherPreconditioning
        );
        assert_round_trip!(
            BuildingHvacRoomPlanPeriodV1,
            plan().periods.into_iter().next().unwrap()
        );
        assert_round_trip!(BuildingHvacRoomPlanV1, plan());
        assert_round_trip!(
            BuildingHvacRoomControlErrorV1,
            BuildingHvacRoomControlErrorV1::RevisionConflict
        );
        assert_round_trip!(
            BuildingHvacPersistedRoomConditionPeriodV1,
            persisted_condition()
        );
    }

    #[test]
    fn room_protocol_transactions_round_trip_through_avro() {
        let values = [
            BuildingHvacRoomProtocolV1::GetRoomV1,
            BuildingHvacRoomProtocolV1::ReplaceRoomControlV1 {
                expected_revision: 3,
                control: BuildingHvacRoomControlV1::default(),
            },
            room_data(),
            BuildingHvacRoomProtocolV1::RoomControlRejectedV1 {
                formatted_rejection: formatted_revision_conflict(),
                error: BuildingHvacRoomControlErrorV1::RevisionConflict,
                current_control_revision: 3,
                current_control: BuildingHvacRoomControlV1::default(),
            },
        ];

        for value in values {
            assert_round_trip!(BuildingHvacRoomProtocolV1, value);
        }
    }

    #[test]
    fn formatted_runtime_byte_arrays_use_notification_resource_arguments() {
        let status = libertas_formatted_text_decode(&formatted_room_status()).unwrap();
        assert_eq!(status.resource_name, "HVAC_ROOM_STATUS");
        assert_eq!(status.arguments.len(), 3);

        let schedule = libertas_formatted_text_decode(&formatted_schedule()).unwrap();
        assert_eq!(schedule.resource_name, "HVAC_ROOM_SCHEDULE");
        assert_eq!(schedule.arguments.len(), 1);

        let rejection = libertas_formatted_text_decode(&formatted_revision_conflict()).unwrap();
        assert_eq!(rejection.resource_name, "HVAC_CONTROL_REVISION_CONFLICT");
        assert!(rejection.arguments.is_empty());
    }

    #[test]
    fn urgent_hvac_notifications_have_localized_resources_and_framework_priorities() {
        let conditions = [
            BuildingHvacUrgentConditionV1::FreezeRisk,
            BuildingHvacUrgentConditionV1::ExcessiveHeat,
            BuildingHvacUrgentConditionV1::TemperatureControlUnavailable,
            BuildingHvacUrgentConditionV1::HeatingNotRecovering,
            BuildingHvacUrgentConditionV1::CoolingNotRecovering,
        ];

        for condition in conditions {
            assert!(
                APP_STRINGS
                    .iter()
                    .any(|(resource, _)| *resource == condition.notification_resource())
            );
            assert!(
                APP_STRINGS
                    .iter()
                    .any(|(resource, _)| *resource == condition.condition_name_resource())
            );
        }
        assert!(
            APP_STRINGS
                .iter()
                .any(|(resource, _)| *resource == "HVAC_URGENT_CONDITION_RECOVERED")
        );
        assert_eq!(
            BuildingHvacUrgentConditionV1::FreezeRisk
                .severity()
                .notification_importance(),
            NotificationImportance::AlertSevere
        );
        assert_eq!(
            BuildingHvacUrgentConditionV1::TemperatureControlUnavailable
                .severity()
                .notification_importance(),
            NotificationImportance::AlertHigh
        );
        assert!(active_urgent_condition().is_well_formed());
        assert!(persisted_urgent_condition().is_well_formed());
    }

    #[test]
    fn urgent_notification_engine_confirms_reminds_and_recovers_freeze_risk() {
        let started_at = 1_785_100_000;
        let mut engine = BuildingHvacUrgentNotificationEngine::new();
        let mut state = urgent_room_state(
            started_at,
            Some(4.5),
            BuildingHvacRoomDataQualityV1::Ready,
            BuildingHvacRoomActivityV1::Heating,
        );

        let first = engine.evaluate(started_at, &state, &[]);
        assert!(first.state_changed());
        assert_eq!(first.pending_notification_count(), 0);
        for elapsed in (60..BUILDING_HVAC_URGENT_TEMPERATURE_CONFIRMATION_SECONDS).step_by(60) {
            state.observed_at = Some(started_at + u64::from(elapsed));
            let pending = engine.evaluate(started_at + u64::from(elapsed), &state, &[]);
            assert!(!pending.state_changed());
        }
        let activated_at =
            started_at + u64::from(BUILDING_HVAC_URGENT_TEMPERATURE_CONFIRMATION_SECONDS);
        state.observed_at = Some(activated_at);
        let activated = engine.evaluate(activated_at, &state, &[]);
        assert!(activated.state_changed());
        assert_eq!(activated.pending_notification_count(), 1);
        assert_eq!(engine.active_conditions().len(), 1);

        for elapsed in (60..BUILDING_HVAC_URGENT_NOTIFICATION_REMINDER_SECONDS).step_by(60) {
            let evaluated_at = activated_at + u64::from(elapsed);
            state.observed_at = Some(evaluated_at);
            assert_eq!(
                engine
                    .evaluate(evaluated_at, &state, &[])
                    .pending_notification_count(),
                0
            );
        }
        let reminder_at =
            activated_at + u64::from(BUILDING_HVAC_URGENT_NOTIFICATION_REMINDER_SECONDS);
        state.observed_at = Some(reminder_at);
        let reminder = engine.evaluate(reminder_at, &state, &[]);
        assert_eq!(reminder.pending_notification_count(), 1);

        state.temperature_celsius = Some(7.5);
        let recovery_started_at = reminder_at + 60;
        state.observed_at = Some(recovery_started_at);
        let recovery_started = engine.evaluate(recovery_started_at, &state, &[]);
        assert!(recovery_started.state_changed());
        assert_eq!(engine.active_conditions().len(), 1);
        for elapsed in (60..BUILDING_HVAC_URGENT_RECOVERY_CONFIRMATION_SECONDS).step_by(60) {
            let evaluated_at = recovery_started_at + u64::from(elapsed);
            state.observed_at = Some(evaluated_at);
            engine.evaluate(evaluated_at, &state, &[]);
        }
        let recovered_at =
            recovery_started_at + u64::from(BUILDING_HVAC_URGENT_RECOVERY_CONFIRMATION_SECONDS);
        state.observed_at = Some(recovered_at);
        let recovered = engine.evaluate(recovered_at, &state, &[]);
        assert!(recovered.state_changed());
        assert_eq!(recovered.pending_notification_count(), 1);
        assert!(engine.active_conditions().is_empty());
    }

    #[test]
    fn urgent_notification_engine_breaks_pending_continuity_and_never_clears_from_missing_data() {
        let started_at = 1_785_200_000;
        let mut state = urgent_room_state(
            started_at,
            Some(4.0),
            BuildingHvacRoomDataQualityV1::Ready,
            BuildingHvacRoomActivityV1::Heating,
        );
        let mut engine = BuildingHvacUrgentNotificationEngine::new();
        engine.evaluate(started_at, &state, &[]);

        let gap_at = started_at + u64::from(BUILDING_HVAC_URGENT_EVIDENCE_MAX_GAP_SECONDS) + 1;
        state.observed_at = Some(gap_at);
        let reset = engine.evaluate(gap_at, &state, &[]);
        assert!(reset.state_changed());
        assert_eq!(
            engine.persisted_conditions()[0].condition_started_at,
            gap_at
        );

        for elapsed in (60..=BUILDING_HVAC_URGENT_TEMPERATURE_CONFIRMATION_SECONDS).step_by(60) {
            let evaluated_at = gap_at + u64::from(elapsed);
            state.observed_at = Some(evaluated_at);
            engine.evaluate(evaluated_at, &state, &[]);
        }
        assert_eq!(engine.active_conditions().len(), 1);

        state.temperature_celsius = None;
        state.data_quality = BuildingHvacRoomDataQualityV1::Unavailable;
        let missing_at =
            gap_at + u64::from(BUILDING_HVAC_URGENT_TEMPERATURE_CONFIRMATION_SECONDS) + 60;
        state.observed_at = Some(missing_at);
        engine.evaluate(missing_at, &state, &[]);
        assert!(
            engine
                .active_conditions()
                .iter()
                .any(|condition| condition.condition == BuildingHvacUrgentConditionV1::FreezeRisk)
        );
    }

    #[test]
    fn urgent_notification_engine_repairs_restored_state_and_rejects_backward_time() {
        let valid = persisted_urgent_condition();
        let mut invalid = valid;
        invalid.condition = BuildingHvacUrgentConditionV1::ExcessiveHeat;
        invalid.last_temperature_celsius = None;
        let mut duplicate = valid;
        duplicate.updated_at += 1;
        let mut engine =
            BuildingHvacUrgentNotificationEngine::restore(vec![valid, invalid, duplicate]);
        assert_eq!(engine.persisted_conditions(), &[valid]);

        let earlier_state = urgent_room_state(
            valid.updated_at - 1,
            Some(4.0),
            BuildingHvacRoomDataQualityV1::Ready,
            BuildingHvacRoomActivityV1::Heating,
        );
        let ignored = engine.evaluate(valid.updated_at - 1, &earlier_state, &[]);
        assert!(!ignored.state_changed());
        assert_eq!(ignored.pending_notification_count(), 0);
        assert_eq!(engine.persisted_conditions(), &[valid]);
    }

    #[test]
    fn urgent_notification_engine_detects_ineffective_heating_from_bounded_history() {
        let evaluated_at = 1_785_300_000;
        let periods = recovery_periods(evaluated_at, BuildingHvacRoomActivityV1::Heating, 14.0);
        let state = urgent_room_state(
            evaluated_at,
            Some(14.2),
            BuildingHvacRoomDataQualityV1::Ready,
            BuildingHvacRoomActivityV1::Heating,
        );
        let mut engine = BuildingHvacUrgentNotificationEngine::new();
        let evaluation = engine.evaluate(evaluated_at, &state, &periods);
        assert!(evaluation.state_changed());
        assert_eq!(evaluation.pending_notification_count(), 1);
        assert!(engine.active_conditions().iter().any(|condition| {
            condition.condition == BuildingHvacUrgentConditionV1::HeatingNotRecovering
        }));

        let improving = recovery_periods(evaluated_at, BuildingHvacRoomActivityV1::Heating, 13.0);
        let mut improving_engine = BuildingHvacUrgentNotificationEngine::new();
        let no_warning = improving_engine.evaluate(evaluated_at, &state, &improving);
        assert_eq!(no_warning.pending_notification_count(), 0);
        assert!(improving_engine.active_conditions().is_empty());
    }

    #[test]
    fn analytics_engine_fuses_current_sensors_and_rejects_ambiguous_disagreement() {
        let now = 1_785_400_000;
        let station = |device, temperature_celsius: f32, humidity_percent: f32| {
            BuildingHvacIndoorSensorStateV1 {
                temperature_sensor: device,
                temperature: Some(BuildingHvacTemperatureReadingV1 {
                    observed_at: now,
                    valid_until: now + 90,
                    temperature_celsius,
                }),
                humidity_sensor: Some(device + 100),
                humidity: Some(BuildingHvacHumidityReadingV1 {
                    observed_at: now,
                    valid_until: now + 90,
                    relative_humidity_percent: humidity_percent,
                }),
                air_quality_sensor: None,
                air_quality: None,
            }
        };
        let sensors = vec![
            station(300, 20.0, 40.0),
            station(301, 20.5, 42.0),
            station(302, 30.0, 41.0),
        ];
        let analytics = BuildingHvacAnalyticsEngine::new();
        let state = analytics.analyze_room(
            now,
            200,
            Some(now),
            Some(now + 90),
            BuildingHvacRoomActivityV1::Idle,
            Some(20.0),
            Some(24.0),
            &sensors,
        );
        assert_eq!(state.data_quality, BuildingHvacRoomDataQualityV1::Degraded);
        assert_eq!(state.fresh_temperature_sensor_count, 2);
        assert!((state.temperature_celsius.unwrap() - 20.25).abs() < 0.001);
        assert_eq!(state.fresh_humidity_sensor_count, 3);
        assert!((state.relative_humidity_percent.unwrap() - 41.0).abs() < 0.001);

        let ambiguous = vec![station(303, 20.0, 40.0), station(304, 25.0, 40.0)];
        let unavailable = analytics.analyze_room(
            now,
            200,
            Some(now),
            Some(now + 90),
            BuildingHvacRoomActivityV1::Idle,
            Some(20.0),
            Some(24.0),
            &ambiguous,
        );
        assert_eq!(
            unavailable.data_quality,
            BuildingHvacRoomDataQualityV1::Unavailable
        );
        assert_eq!(unavailable.temperature_celsius, None);
        assert_eq!(unavailable.fresh_temperature_sensor_count, 0);

        let stale_thermostat = analytics.analyze_room(
            now,
            200,
            Some(now - 120),
            Some(now),
            BuildingHvacRoomActivityV1::Heating,
            Some(20.0),
            Some(24.0),
            &sensors,
        );
        assert_eq!(
            stale_thermostat.data_quality,
            BuildingHvacRoomDataQualityV1::Unavailable
        );
        assert_eq!(
            stale_thermostat.activity,
            BuildingHvacRoomActivityV1::Unknown
        );
        assert_eq!(stale_thermostat.effective_heating_setpoint_celsius, None);
        assert!(stale_thermostat.temperature_celsius.is_some());
    }

    #[test]
    fn analytics_engine_summarizes_time_weighted_conditions() {
        let starts_at = 1_785_500_000;
        let periods = vec![
            BuildingHvacPersistedRoomConditionPeriodV1 {
                starts_at,
                duration_seconds: 900,
                temperature_celsius: Some(18.0),
                relative_humidity_percent: Some(40.0),
                activity: BuildingHvacRoomActivityV1::Heating,
                effective_heating_setpoint_celsius: Some(20.0),
                effective_cooling_setpoint_celsius: Some(24.0),
                outdoor_dry_bulb_temperature_celsius: Some(0.0),
            },
            BuildingHvacPersistedRoomConditionPeriodV1 {
                starts_at: starts_at + 900,
                duration_seconds: 900,
                temperature_celsius: Some(22.0),
                relative_humidity_percent: Some(50.0),
                activity: BuildingHvacRoomActivityV1::Cooling,
                effective_heating_setpoint_celsius: Some(20.0),
                effective_cooling_setpoint_celsius: Some(24.0),
                outdoor_dry_bulb_temperature_celsius: Some(1.0),
            },
        ];
        let statistics = BuildingHvacAnalyticsEngine::new()
            .summarize_conditions(&periods)
            .unwrap();
        assert_eq!(statistics.starts_at, starts_at);
        assert_eq!(statistics.ends_before, starts_at + 1_800);
        assert_eq!(statistics.temperature_sample_count, 2);
        assert_eq!(statistics.minimum_temperature_celsius, 18.0);
        assert_eq!(statistics.mean_temperature_celsius, 20.0);
        assert_eq!(statistics.maximum_temperature_celsius, 22.0);
        assert_eq!(statistics.temperature_data_available_seconds, 1_800);
        assert_eq!(
            statistics.below_heating_comfort_degree_minutes_celsius,
            30.0
        );
        assert_eq!(statistics.above_cooling_comfort_degree_minutes_celsius, 0.0);
        assert_eq!(statistics.mean_relative_humidity_percent, Some(45.0));
        assert_eq!(statistics.heating_active_seconds, 900);
        assert_eq!(statistics.cooling_active_seconds, 900);
    }

    #[test]
    fn analytics_engine_derives_psychrometrics_only_from_fresh_consistent_weather() {
        let analytics = BuildingHvacAnalyticsEngine::new();
        let weather = current_weather();
        let result = analytics
            .analyze_outdoor_air(weather.valid_at, &weather)
            .unwrap();
        assert!(
            (0.011..=0.013).contains(&result.humidity_ratio_kilograms_water_per_kilogram_dry_air)
        );
        assert!((58.0..=62.0).contains(&result.moist_air_enthalpy_kilojoules_per_kilogram_dry_air));
        assert!((20.0..=23.0).contains(&result.wet_bulb_temperature_celsius));
        assert!(result.is_well_formed_for(weather.conditions.dry_bulb_temperature_celsius));

        assert_eq!(
            analytics.analyze_outdoor_air(weather.valid_until, &weather),
            None
        );
        let mut inconsistent = weather;
        inconsistent.conditions.relative_humidity_percent = 90;
        assert_eq!(
            analytics.analyze_outdoor_air(inconsistent.valid_at, &inconsistent),
            None
        );
    }

    #[test]
    fn control_engine_arbitrates_shared_rooms_and_enforces_deadband() {
        let limits = BuildingHvacThermostatControlLimits {
            minimum_heating_setpoint_celsius: 10.0,
            maximum_heating_setpoint_celsius: 25.0,
            minimum_cooling_setpoint_celsius: 18.0,
            maximum_cooling_setpoint_celsius: 35.0,
            minimum_deadband_celsius: 2.0,
        };
        let mut cold_state = observed_state();
        cold_state.temperature_celsius = Some(18.0);
        cold_state.effective_heating_setpoint_celsius = Some(19.0);
        cold_state.effective_cooling_setpoint_celsius = Some(25.0);
        let mut hot_state = observed_state();
        hot_state.temperature_celsius = Some(26.0);
        hot_state.effective_heating_setpoint_celsius = Some(19.0);
        hot_state.effective_cooling_setpoint_celsius = Some(25.0);
        let cold_control = BuildingHvacRoomControlV1 {
            preferred_heating_temperature_celsius: 20.0,
            preferred_cooling_temperature_celsius: 24.0,
            ..BuildingHvacRoomControlV1::default()
        };
        let hot_control = BuildingHvacRoomControlV1 {
            preferred_heating_temperature_celsius: 19.0,
            preferred_cooling_temperature_celsius: 23.0,
            ..BuildingHvacRoomControlV1::default()
        };
        let candidates = [
            BuildingHvacRoomControlCandidate {
                room_endpoint: 100,
                control: &cold_control,
                state: &cold_state,
                predicted_cross_zone_temperature_change_celsius: 0.0,
                predicted_machine_learning_temperature_change_celsius: None,
            },
            BuildingHvacRoomControlCandidate {
                room_endpoint: 101,
                control: &hot_control,
                state: &hot_state,
                predicted_cross_zone_temperature_change_celsius: 0.0,
                predicted_machine_learning_temperature_change_celsius: None,
            },
        ];
        assert_eq!(
            BuildingHvacControlEngine::new().arbitrate_thermostat(200, limits, &candidates),
            BuildingHvacThermostatControlDecision::ApplySetpoints {
                heating_setpoint_celsius: Some(20.0),
                cooling_setpoint_celsius: Some(23.0),
                dominant_heating_room: Some(100),
                dominant_cooling_room: Some(101),
                trustworthy_room_count: 2,
            }
        );

        let comfort_control = BuildingHvacRoomControlV1 {
            comfort_or_savings_normalized: 1.0,
            ..BuildingHvacRoomControlV1::default()
        };
        let comfort_candidate = [BuildingHvacRoomControlCandidate {
            room_endpoint: 100,
            control: &comfort_control,
            state: &cold_state,
            predicted_cross_zone_temperature_change_celsius: 1.0,
            predicted_machine_learning_temperature_change_celsius: None,
        }];
        assert_eq!(
            BuildingHvacControlEngine::new().arbitrate_thermostat(200, limits, &comfort_candidate),
            BuildingHvacThermostatControlDecision::ApplySetpoints {
                heating_setpoint_celsius: Some(21.0),
                cooling_setpoint_celsius: Some(23.0),
                dominant_heating_room: Some(100),
                dominant_cooling_room: Some(100),
                trustworthy_room_count: 1,
            }
        );

        let conflict_heat = BuildingHvacRoomControlV1 {
            preferred_heating_temperature_celsius: 23.0,
            preferred_cooling_temperature_celsius: 24.0,
            operating_preference: BuildingHvacRoomOperatingPreferenceV1::Heat,
            comfort_or_savings_normalized: 0.0,
        };
        let conflict_cool = BuildingHvacRoomControlV1 {
            preferred_heating_temperature_celsius: 20.0,
            preferred_cooling_temperature_celsius: 22.0,
            operating_preference: BuildingHvacRoomOperatingPreferenceV1::Cool,
            comfort_or_savings_normalized: 0.0,
        };
        cold_state.temperature_celsius = Some(10.0);
        hot_state.temperature_celsius = Some(23.0);
        let conflict_candidates = [
            BuildingHvacRoomControlCandidate {
                room_endpoint: 100,
                control: &conflict_heat,
                state: &cold_state,
                predicted_cross_zone_temperature_change_celsius: 0.0,
                predicted_machine_learning_temperature_change_celsius: None,
            },
            BuildingHvacRoomControlCandidate {
                room_endpoint: 101,
                control: &conflict_cool,
                state: &hot_state,
                predicted_cross_zone_temperature_change_celsius: 0.0,
                predicted_machine_learning_temperature_change_celsius: None,
            },
        ];
        let decision = BuildingHvacControlEngine::new().arbitrate_thermostat(
            200,
            limits,
            &conflict_candidates,
        );
        assert!(matches!(
            decision,
            BuildingHvacThermostatControlDecision::ApplySetpoints {
                heating_setpoint_celsius: Some(23.0),
                cooling_setpoint_celsius: Some(25.0),
                ..
            }
        ));

        let predicted_conflict_candidates = [
            BuildingHvacRoomControlCandidate {
                room_endpoint: 100,
                control: &conflict_heat,
                state: &cold_state,
                predicted_cross_zone_temperature_change_celsius: 10.0,
                predicted_machine_learning_temperature_change_celsius: Some(10.0),
            },
            BuildingHvacRoomControlCandidate {
                room_endpoint: 101,
                control: &conflict_cool,
                state: &hot_state,
                predicted_cross_zone_temperature_change_celsius: 0.0,
                predicted_machine_learning_temperature_change_celsius: None,
            },
        ];
        assert!(matches!(
            BuildingHvacControlEngine::new().arbitrate_thermostat(
                200,
                limits,
                &predicted_conflict_candidates
            ),
            BuildingHvacThermostatControlDecision::ApplySetpoints {
                heating_setpoint_celsius: Some(20.0),
                cooling_setpoint_celsius: Some(22.0),
                ..
            }
        ));
    }

    #[test]
    fn control_engine_holds_for_off_unavailable_and_already_applied_rooms() {
        let limits = BuildingHvacThermostatControlLimits {
            minimum_heating_setpoint_celsius: 10.0,
            maximum_heating_setpoint_celsius: 25.0,
            minimum_cooling_setpoint_celsius: 18.0,
            maximum_cooling_setpoint_celsius: 35.0,
            minimum_deadband_celsius: 2.0,
        };
        let mut state = observed_state();
        state.effective_heating_setpoint_celsius = Some(20.0);
        state.effective_cooling_setpoint_celsius = Some(24.0);
        let control = BuildingHvacRoomControlV1::default();
        let candidate = [BuildingHvacRoomControlCandidate {
            room_endpoint: 100,
            control: &control,
            state: &state,
            predicted_cross_zone_temperature_change_celsius: 0.0,
            predicted_machine_learning_temperature_change_celsius: None,
        }];
        assert_eq!(
            BuildingHvacControlEngine::new().arbitrate_thermostat(200, limits, &candidate),
            BuildingHvacThermostatControlDecision::Hold {
                reason: BuildingHvacThermostatControlHoldReason::AlreadyApplied,
            }
        );

        let off = BuildingHvacRoomControlV1 {
            operating_preference: BuildingHvacRoomOperatingPreferenceV1::Off,
            ..BuildingHvacRoomControlV1::default()
        };
        let off_candidate = [BuildingHvacRoomControlCandidate {
            room_endpoint: 100,
            control: &off,
            state: &state,
            predicted_cross_zone_temperature_change_celsius: 0.0,
            predicted_machine_learning_temperature_change_celsius: None,
        }];
        assert_eq!(
            BuildingHvacControlEngine::new().arbitrate_thermostat(200, limits, &off_candidate),
            BuildingHvacThermostatControlDecision::Hold {
                reason: BuildingHvacThermostatControlHoldReason::NoEnabledDemand,
            }
        );

        state.data_quality = BuildingHvacRoomDataQualityV1::Unavailable;
        let unavailable_candidate = [BuildingHvacRoomControlCandidate {
            room_endpoint: 100,
            control: &control,
            state: &state,
            predicted_cross_zone_temperature_change_celsius: 0.0,
            predicted_machine_learning_temperature_change_celsius: None,
        }];
        assert_eq!(
            BuildingHvacControlEngine::new().arbitrate_thermostat(
                200,
                limits,
                &unavailable_candidate
            ),
            BuildingHvacThermostatControlDecision::Hold {
                reason: BuildingHvacThermostatControlHoldReason::NoTrustworthyRoom,
            }
        );
    }

    #[test]
    fn persistent_variants_round_trip_independently() {
        for value in persistent_values() {
            assert_round_trip!(BuildingHvacPersistentDataV1, value);
        }
    }

    #[test]
    fn cross_zone_learning_estimates_directional_heating_effect() {
        let learning = room_learning();
        let influences = learning.runtime_influences();

        assert_eq!(influences.len(), 1);
        assert_eq!(influences[0].source_thermostat, 201);
        let effect = influences[0]
            .heating_temperature_rise_celsius_per_runtime_hour
            .unwrap();
        assert!((effect - 0.8).abs() < 0.000_01);
        assert!(influences[0].heating_confidence_normalized > 0.0);
        assert_eq!(
            influences[0].cooling_temperature_drop_celsius_per_runtime_hour,
            None
        );
    }

    #[test]
    fn cross_zone_learning_rejects_confounded_periods() {
        let mut learning = BuildingHvacRoomLearningStateV1 {
            passive_outdoor_coupling: BuildingHvacOnlineRegressionStateV1::empty(),
            cross_zone_learners: Vec::new(),
        };

        assert!(!learning.observe_identifiable_cross_zone_period(
            1_785_059_200,
            200,
            BuildingHvacRoomActivityV1::Heating,
            201,
            BuildingHvacRoomActivityV1::Heating,
            1,
            900,
            1.0,
            0.2,
            0.0,
            1.0,
        ));
        assert!(!learning.observe_identifiable_cross_zone_period(
            1_785_059_200,
            200,
            BuildingHvacRoomActivityV1::Idle,
            201,
            BuildingHvacRoomActivityV1::Heating,
            2,
            900,
            1.0,
            0.2,
            0.0,
            1.0,
        ));
        assert!(!learning.observe_identifiable_cross_zone_period(
            1_785_059_200,
            200,
            BuildingHvacRoomActivityV1::Idle,
            200,
            BuildingHvacRoomActivityV1::Heating,
            1,
            900,
            1.0,
            0.2,
            0.0,
            1.0,
        ));
        assert!(learning.cross_zone_learners.is_empty());
    }

    #[test]
    fn passive_learning_separates_outdoor_drift_from_zone_spillover() {
        let mut learning = BuildingHvacRoomLearningStateV1 {
            passive_outdoor_coupling: BuildingHvacOnlineRegressionStateV1::empty(),
            cross_zone_learners: Vec::new(),
        };
        for step in 0..4 {
            assert!(learning.observe_identifiable_passive_period(
                1_785_059_200 + step * 900,
                true,
                900,
                20.0,
                30.0,
                0.25,
                1.0,
            ));
        }

        let prediction = learning
            .predict_passive_temperature_change_celsius(2.5)
            .unwrap();
        assert!((prediction - 0.25).abs() < 0.000_001);
        assert!(!learning.observe_identifiable_passive_period(
            1_785_063_700,
            false,
            900,
            20.0,
            30.0,
            0.25,
            1.0,
        ));
    }

    #[test]
    fn online_learning_forgets_old_weight_but_keeps_lifetime_count() {
        let mut learner = BuildingHvacOnlineRegressionStateV1::empty();
        let started_at = 1_785_059_200;
        for step in 0..4 {
            assert!(learner.observe(started_at + step, 0.25, 0.2, 1.0));
        }
        assert_eq!(learner.effective_sample_weight, 4.0);
        assert_eq!(learner.accepted_observation_count, 4);

        assert!(learner.observe(
            started_at + 3 + BUILDING_HVAC_CROSS_ZONE_LEARNING_HALF_LIFE_SECONDS,
            0.25,
            0.2,
            1.0,
        ));
        assert_eq!(learner.effective_sample_weight, 3.0);
        assert_eq!(learner.accepted_observation_count, 5);
        assert_eq!(learner.estimated_coefficient(), None);
        assert!(!learner.observe(started_at, 0.25, 0.2, 1.0));
    }

    fn structurally_valid_machine_learning_model(
        room_endpoint: LibertasEndpoint,
        horizon: BuildingHvacThermalPredictionHorizonV1,
        trained_at: LibertasDateTime,
        model_ubjson: Vec<u8>,
    ) -> BuildingHvacMachineLearningModelV1 {
        use sha2::{Digest, Sha256};

        BuildingHvacMachineLearningModelV1 {
            room_endpoint,
            horizon,
            feature_schema_version: BUILDING_HVAC_ML_FEATURE_SCHEMA_VERSION,
            feature_names: BUILDING_HVAC_ML_FEATURE_NAMES
                .iter()
                .map(|name| String::from(*name))
                .collect(),
            xgboost_version: String::from(BUILDING_HVAC_XGBOOST_VERSION),
            trained_at,
            training_range_starts_at: trained_at - 14 * 24 * 60 * 60,
            training_range_ends_at: trained_at - 24 * 60 * 60,
            boost_rounds: BUILDING_HVAC_ML_BOOST_ROUNDS,
            maximum_tree_depth: BUILDING_HVAC_ML_MAXIMUM_TREE_DEPTH,
            learning_rate: 0.05,
            validation: BuildingHvacMachineLearningValidationV1 {
                training_sample_count: 1_076,
                validation_sample_count: 268,
                candidate_rmse_celsius: 0.1,
                deterministic_baseline_rmse_celsius: 0.2,
                improvement_normalized: 0.5,
            },
            model_sha256: Sha256::digest(&model_ubjson).to_vec(),
            model_ubjson,
        }
    }

    #[test]
    fn machine_learning_features_and_samples_reject_invalid_values() {
        let features = machine_learning_features(20.0);
        assert!(features.is_well_formed());
        assert!(machine_learning_sample().is_well_formed());

        let mut invalid = features;
        invalid.own_heating_runtime_fraction = 0.75;
        invalid.own_cooling_runtime_fraction = 0.5;
        assert!(!invalid.is_well_formed());

        let mut missing_targets = machine_learning_sample();
        missing_targets.temperature_change_15_minutes_celsius = None;
        missing_targets.temperature_change_30_minutes_celsius = None;
        missing_targets.temperature_change_60_minutes_celsius = None;
        assert!(!missing_targets.is_well_formed());
    }

    #[test]
    fn machine_learning_model_manifest_checksum_and_rollback_are_validated() {
        let first = structurally_valid_machine_learning_model(
            100,
            BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
            1_786_000_000,
            vec![1, 2, 3],
        );
        assert!(first.is_well_formed());

        let mut corrupt = first.clone();
        corrupt.model_ubjson.push(4);
        assert!(!corrupt.is_well_formed());

        let mut models = BuildingHvacMachineLearningModelSetV1::empty(100);
        assert!(models.promote(first.clone()));
        let second = structurally_valid_machine_learning_model(
            100,
            BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
            1_786_086_400,
            vec![4, 5, 6],
        );
        assert!(models.promote(second.clone()));
        assert!(models.is_well_formed());
        assert_eq!(models.models[0].active_model, second);
        assert_eq!(models.models[0].previous_model, Some(first));

        let wrong_room = structurally_valid_machine_learning_model(
            101,
            BuildingHvacThermalPredictionHorizonV1::ThirtyMinutes,
            1_786_086_400,
            vec![7, 8, 9],
        );
        assert!(!models.promote(wrong_room));
    }

    #[test]
    fn machine_learning_public_values_round_trip() {
        let model = structurally_valid_machine_learning_model(
            100,
            BuildingHvacThermalPredictionHorizonV1::SixtyMinutes,
            1_786_000_000,
            vec![1, 2, 3],
        );
        let mut models = BuildingHvacMachineLearningModelSetV1::empty(100);
        assert!(models.promote(model.clone()));
        let runtime = BuildingHvacRoomMachineLearningV1 {
            predictions: vec![BuildingHvacThermalPredictionV1 {
                horizon: BuildingHvacThermalPredictionHorizonV1::SixtyMinutes,
                temperature_change_celsius: -0.5,
                source: BuildingHvacThermalPredictionSourceV1::Xgboost,
                model_trained_at: Some(model.trained_at),
            }],
        };

        assert_round_trip!(
            BuildingHvacMachineLearningFeaturesV1,
            machine_learning_features(20.0)
        );
        assert_round_trip!(
            BuildingHvacMachineLearningSampleV1,
            machine_learning_sample()
        );
        assert_round_trip!(BuildingHvacMachineLearningModelV1, model);
        assert_round_trip!(BuildingHvacMachineLearningModelSetV1, models);
        assert_round_trip!(BuildingHvacRoomMachineLearningV1, runtime);
    }

    fn no_op_worker_callback() {}

    #[test]
    fn machine_learning_worker_returns_explicit_fallback_without_a_model() {
        let (client, results) =
            start_machine_learning_worker(no_op_worker_callback, no_op_worker_callback).unwrap();
        client
            .try_predict(
                7,
                100,
                BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
                machine_learning_features(20.0),
            )
            .unwrap();
        let result = results
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert_eq!(
            result,
            BuildingHvacMachineLearningResult::Prediction {
                request_id: 7,
                room_endpoint: 100,
                prediction: BuildingHvacThermalPredictionV1 {
                    horizon: BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
                    temperature_change_celsius: 0.0,
                    source: BuildingHvacThermalPredictionSourceV1::DeterministicFallback,
                    model_trained_at: None,
                },
            }
        );
        client.request_shutdown().unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn statically_linked_xgboost_trains_serializes_and_predicts() {
        let starts_at = 1_785_000_000;
        let samples: Vec<_> = (0..BUILDING_HVAC_ML_MINIMUM_TRAINING_SAMPLES)
            .map(|index| {
                let heating = if index % 4 < 2 { 0.75 } else { 0.0 };
                let cooling = if index % 4 == 3 { 0.5 } else { 0.0 };
                let room_temperature = 18.0 + (index % 40) as f32 * 0.1;
                let mut features = machine_learning_features(room_temperature);
                features.own_heating_runtime_fraction = heating;
                features.own_cooling_runtime_fraction = cooling;
                features.outdoor_temperature_celsius = Some(2.0 + (index % 24) as f32 * 0.5);
                let target = 0.7 * heating - 0.8 * cooling + (20.0 - room_temperature) * 0.03;
                BuildingHvacMachineLearningSampleV1 {
                    observed_at: starts_at + index as u64 * 900,
                    room_endpoint: 100,
                    features,
                    temperature_change_15_minutes_celsius: Some(target),
                    temperature_change_30_minutes_celsius: None,
                    temperature_change_60_minutes_celsius: None,
                }
            })
            .collect();
        let trained_at = samples.last().unwrap().observed_at + 900;
        let model = BuildingHvacMachineLearningEngine::train_candidate(
            BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
            trained_at,
            &samples,
        )
        .unwrap();
        assert!(model.is_well_formed());
        assert!(
            model.validation.candidate_rmse_celsius
                < model.validation.deterministic_baseline_rmse_celsius
        );
        let prediction =
            BuildingHvacMachineLearningEngine::predict(&model, machine_learning_features(19.0))
                .unwrap();
        assert!(prediction.is_finite());
        assert!(prediction.abs() <= BUILDING_HVAC_ML_MAXIMUM_PREDICTED_CHANGE_CELSIUS);
    }

    #[test]
    fn enum_and_union_discriminants_are_stable() {
        let preferences = [
            BuildingHvacRoomOperatingPreferenceV1::Auto,
            BuildingHvacRoomOperatingPreferenceV1::Heat,
            BuildingHvacRoomOperatingPreferenceV1::Cool,
            BuildingHvacRoomOperatingPreferenceV1::Off,
        ];
        let qualities = [
            BuildingHvacRoomDataQualityV1::Ready,
            BuildingHvacRoomDataQualityV1::Degraded,
            BuildingHvacRoomDataQualityV1::Unavailable,
        ];
        let activities = [
            BuildingHvacRoomActivityV1::Unknown,
            BuildingHvacRoomActivityV1::Idle,
            BuildingHvacRoomActivityV1::Heating,
            BuildingHvacRoomActivityV1::Cooling,
            BuildingHvacRoomActivityV1::FanOnly,
        ];
        let urgent_conditions = [
            BuildingHvacUrgentConditionV1::FreezeRisk,
            BuildingHvacUrgentConditionV1::ExcessiveHeat,
            BuildingHvacUrgentConditionV1::TemperatureControlUnavailable,
            BuildingHvacUrgentConditionV1::HeatingNotRecovering,
            BuildingHvacUrgentConditionV1::CoolingNotRecovering,
        ];
        let urgent_severities = [
            BuildingHvacUrgentNotificationSeverityV1::High,
            BuildingHvacUrgentNotificationSeverityV1::Severe,
        ];
        let urgent_phases = [
            BuildingHvacUrgentConditionPhaseV1::ActivationPending,
            BuildingHvacUrgentConditionPhaseV1::Active,
            BuildingHvacUrgentConditionPhaseV1::RecoveryPending,
        ];
        let local_air_qualities = [
            BuildingHvacAirQualityV1::Unknown,
            BuildingHvacAirQualityV1::Good,
            BuildingHvacAirQualityV1::Fair,
            BuildingHvacAirQualityV1::Moderate,
            BuildingHvacAirQualityV1::Poor,
            BuildingHvacAirQualityV1::VeryPoor,
            BuildingHvacAirQualityV1::ExtremelyPoor,
        ];
        let concentration_levels = [
            BuildingHvacConcentrationLevelV1::Unknown,
            BuildingHvacConcentrationLevelV1::Low,
            BuildingHvacConcentrationLevelV1::Medium,
            BuildingHvacConcentrationLevelV1::High,
            BuildingHvacConcentrationLevelV1::Critical,
        ];
        let measurement_kinds = [
            BuildingHvacAirMeasurementKindV1::CarbonDioxide,
            BuildingHvacAirMeasurementKindV1::CarbonMonoxide,
            BuildingHvacAirMeasurementKindV1::NitrogenDioxide,
            BuildingHvacAirMeasurementKindV1::Ozone,
            BuildingHvacAirMeasurementKindV1::ParticulateMatter1,
            BuildingHvacAirMeasurementKindV1::ParticulateMatter2_5,
            BuildingHvacAirMeasurementKindV1::ParticulateMatter10,
            BuildingHvacAirMeasurementKindV1::Formaldehyde,
            BuildingHvacAirMeasurementKindV1::TotalVolatileOrganicCompounds,
            BuildingHvacAirMeasurementKindV1::Radon,
        ];
        let measurement_units = [
            BuildingHvacAirMeasurementUnitV1::PartsPerMillion,
            BuildingHvacAirMeasurementUnitV1::PartsPerBillion,
            BuildingHvacAirMeasurementUnitV1::PartsPerTrillion,
            BuildingHvacAirMeasurementUnitV1::MilligramsPerCubicMeter,
            BuildingHvacAirMeasurementUnitV1::MicrogramsPerCubicMeter,
            BuildingHvacAirMeasurementUnitV1::NanogramsPerCubicMeter,
            BuildingHvacAirMeasurementUnitV1::PicogramsPerCubicMeter,
            BuildingHvacAirMeasurementUnitV1::BecquerelsPerCubicMeter,
        ];
        let reasons = [
            BuildingHvacRoomPlanReasonV1::RoomComfort,
            BuildingHvacRoomPlanReasonV1::WeatherPreconditioning,
            BuildingHvacRoomPlanReasonV1::LowCostPreconditioning,
            BuildingHvacRoomPlanReasonV1::HighCostReduction,
            BuildingHvacRoomPlanReasonV1::SharedThermostatArbitration,
            BuildingHvacRoomPlanReasonV1::DegradedFallback,
        ];
        let errors = [
            BuildingHvacRoomControlErrorV1::RevisionConflict,
            BuildingHvacRoomControlErrorV1::InvalidTemperatureBand,
            BuildingHvacRoomControlErrorV1::InvalidNormalizedPreference,
            BuildingHvacRoomControlErrorV1::UnsupportedOperatingPreference,
            BuildingHvacRoomControlErrorV1::TemporarilyUnavailable,
        ];
        let machine_learning_horizons = [
            BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
            BuildingHvacThermalPredictionHorizonV1::ThirtyMinutes,
            BuildingHvacThermalPredictionHorizonV1::SixtyMinutes,
        ];
        let machine_learning_sources = [
            BuildingHvacThermalPredictionSourceV1::Xgboost,
            BuildingHvacThermalPredictionSourceV1::DeterministicFallback,
        ];

        for values in [
            preferences
                .iter()
                .map(|value| value.to_avro())
                .collect::<Vec<_>>(),
            qualities
                .iter()
                .map(|value| value.to_avro())
                .collect::<Vec<_>>(),
            activities
                .iter()
                .map(|value| value.to_avro())
                .collect::<Vec<_>>(),
            urgent_conditions
                .iter()
                .map(|value| value.to_avro())
                .collect::<Vec<_>>(),
            urgent_severities
                .iter()
                .map(|value| value.to_avro())
                .collect::<Vec<_>>(),
            urgent_phases
                .iter()
                .map(|value| value.to_avro())
                .collect::<Vec<_>>(),
            local_air_qualities
                .iter()
                .map(|value| value.to_avro())
                .collect::<Vec<_>>(),
            concentration_levels
                .iter()
                .map(|value| value.to_avro())
                .collect::<Vec<_>>(),
            measurement_kinds
                .iter()
                .map(|value| value.to_avro())
                .collect::<Vec<_>>(),
            measurement_units
                .iter()
                .map(|value| value.to_avro())
                .collect::<Vec<_>>(),
            reasons
                .iter()
                .map(|value| value.to_avro())
                .collect::<Vec<_>>(),
            errors
                .iter()
                .map(|value| value.to_avro())
                .collect::<Vec<_>>(),
            machine_learning_horizons
                .iter()
                .map(|value| value.to_avro())
                .collect::<Vec<_>>(),
            machine_learning_sources
                .iter()
                .map(|value| value.to_avro())
                .collect::<Vec<_>>(),
        ] {
            for (index, encoded) in values.iter().enumerate() {
                assert_eq!(encoded.first(), Some(&((index as u8) * 2)));
            }
        }

        let protocols = [
            BuildingHvacRoomProtocolV1::GetRoomV1,
            BuildingHvacRoomProtocolV1::ReplaceRoomControlV1 {
                expected_revision: 3,
                control: BuildingHvacRoomControlV1::default(),
            },
            room_data(),
            BuildingHvacRoomProtocolV1::RoomControlRejectedV1 {
                formatted_rejection: formatted_revision_conflict(),
                error: BuildingHvacRoomControlErrorV1::RevisionConflict,
                current_control_revision: 3,
                current_control: BuildingHvacRoomControlV1::default(),
            },
        ];
        for (index, value) in protocols.iter().enumerate() {
            assert_eq!(value.to_avro().first(), Some(&((index as u8) * 2)));
        }

        for (index, value) in persistent_values().iter().enumerate() {
            assert_eq!(value.to_avro().first(), Some(&((index as u8) * 2)));
        }
    }

    #[test]
    fn fixed_capacity_policy_matches_schema_limits() {
        assert_eq!(BUILDING_HVAC_MAX_ROOMS, 64);
        assert_eq!(BUILDING_HVAC_MAX_THERMOSTATS, 16);
        assert_eq!(BUILDING_HVAC_MAX_SENSORS_PER_ROOM, 8);
        assert_eq!(BUILDING_HVAC_MAX_AIR_MEASUREMENTS, 10);
        assert_eq!(BUILDING_HVAC_MAX_ROOM_PLAN_PERIODS, 96);
        assert_eq!(BUILDING_HVAC_MAX_PERSISTED_ROOM_CONDITION_PERIODS, 96);
        assert_eq!(BUILDING_HVAC_MAX_URGENT_NOTIFICATION_RECIPIENTS, 16);
        assert_eq!(BUILDING_HVAC_MAX_URGENT_ROOM_CONDITIONS, 5);
        assert_eq!(BUILDING_HVAC_ROOM_MAXIMUM_WAIT_INTERVAL_SECONDS, 300);
        assert_eq!(
            BUILDING_HVAC_URGENT_TEMPERATURE_CONFIRMATION_SECONDS,
            5 * 60
        );
        assert_eq!(BUILDING_HVAC_URGENT_RECOVERY_CONFIRMATION_SECONDS, 10 * 60);
        assert_eq!(BUILDING_HVAC_URGENT_EVIDENCE_MAX_GAP_SECONDS, 90);
        assert_eq!(
            BUILDING_HVAC_CONTROL_UNAVAILABLE_CONFIRMATION_SECONDS,
            10 * 60
        );
        assert_eq!(BUILDING_HVAC_NOT_RECOVERING_OBSERVATION_SECONDS, 60 * 60);
        assert_eq!(
            BUILDING_HVAC_HEATING_NOT_RECOVERING_TEMPERATURE_CELSIUS,
            15.0
        );
        assert_eq!(
            BUILDING_HVAC_COOLING_NOT_RECOVERING_TEMPERATURE_CELSIUS,
            30.0
        );
        assert_eq!(BUILDING_HVAC_MINIMUM_RECOVERY_CHANGE_CELSIUS, 0.5);
        assert_eq!(BUILDING_HVAC_MINIMUM_RECOVERY_DATA_COVERAGE_NORMALIZED, 0.8);
        assert_eq!(BUILDING_HVAC_TEMPERATURE_FUSION_OUTLIER_CELSIUS, 2.0);
        assert_eq!(BUILDING_HVAC_HUMIDITY_FUSION_OUTLIER_PERCENT, 15.0);
        assert_eq!(BUILDING_HVAC_WEATHER_HUMIDITY_CONSISTENCY_PERCENT, 15.0);
        assert_eq!(BUILDING_HVAC_MAX_COMFORT_SETPOINT_ADJUSTMENT_CELSIUS, 1.0);
        assert_eq!(BUILDING_HVAC_SETPOINT_COMMAND_TOLERANCE_CELSIUS, 0.05);
        assert_eq!(BUILDING_HVAC_URGENT_NOTIFICATION_REMINDER_SECONDS, 30 * 60);
        assert_eq!(BUILDING_HVAC_FREEZE_RISK_TEMPERATURE_CELSIUS, 5.0);
        assert_eq!(BUILDING_HVAC_FREEZE_RECOVERY_TEMPERATURE_CELSIUS, 7.0);
        assert_eq!(BUILDING_HVAC_EXCESSIVE_HEAT_TEMPERATURE_CELSIUS, 35.0);
        assert_eq!(
            BUILDING_HVAC_EXCESSIVE_HEAT_RECOVERY_TEMPERATURE_CELSIUS,
            32.0
        );
        assert_eq!(
            BUILDING_HVAC_CROSS_ZONE_LEARNING_HALF_LIFE_SECONDS,
            30 * 24 * 60 * 60
        );
        assert_eq!(BUILDING_HVAC_ML_FEATURE_COUNT, 16);
        assert_eq!(BUILDING_HVAC_ML_MINIMUM_TRAINING_SAMPLES, 14 * 24 * 4);
        assert_eq!(
            BUILDING_HVAC_ML_MAXIMUM_TRAINING_SAMPLES_PER_ROOM,
            91 * 24 * 4
        );
        assert_eq!(BUILDING_HVAC_ML_COMMAND_CAPACITY, 8);
        assert_eq!(BUILDING_HVAC_ML_RESULT_CAPACITY, 16);
    }

    #[test]
    fn device_descriptors_are_the_device_type_editor_outputs() {
        assert_eq!(MATTER_THERMOSTAT_DEVICE_DESCRIPTOR, "BQEBAYEGAA==");
        assert_eq!(MATTER_TEMPERATURE_SENSOR_DEVICE_DESCRIPTOR, "BQEBAYIGAA==");
        assert_eq!(MATTER_HUMIDITY_SENSOR_DEVICE_DESCRIPTOR, "BQEBAYcGAA==");
        assert_eq!(MATTER_AIR_QUALITY_SENSOR_DEVICE_DESCRIPTOR, "BQEBASwA");
    }
}
