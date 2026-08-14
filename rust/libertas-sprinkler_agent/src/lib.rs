//! Libertas Sprinkler
//! Calculates and executes weather-aware watering schedules for sprinkler zones
//! controlled by Matter Valve Configuration and Control devices.
//!
//! Configuration identifies the shared sprinkler weather endpoint, one
//! chart-ready Sprinkler Report endpoint, reminder recipients, and, for each
//! zone, its valve, plant type, sprinkler-head type, and state endpoint. The
//! controller adapts each watering
//! run from weather, observed valve time, and one user-facing water amount
//! adjuster. Hold-off periods remain runtime schedule constraints.
//! When a fresh forecast and site location are available, the controller moves
//! a run into the best nearby morning window. It derives solar position from
//! UTC and the site coordinates, prefers low evapotranspiration and wind, and
//! avoids prolonged foliage wetness for overhead watering. A critically dry
//! zone still uses the first weather-safe opportunity.
//! Hold-offs remain hard constraints. The controller waters before one only
//! when a fresh continuous forecast shows a safe, rain-free opportunity and
//! delaying would produce a critical deficit; otherwise it recalculates the
//! make-up amount and duration at the first legal post-hold-off start.
//!
//! Each zone persists compact settings and a folded water-balance baseline.
//! Recent precipitation, evapotranspiration, and actual valve-open irrigation
//! are incremental indexed records reconstructed into a bounded ledger during
//! startup. If internet weather stops, persisted local demand falls back to an
//! offline location-and-season estimate and finally a conservative built-in
//! rate. Valve subscriptions count both automatic and manual watering, so a
//! restart or manual run does not cause the controller to water the same
//! deficit twice.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::{boxed::Box, rc::Rc, string::String, vec::Vec};
use core::{any::Any, cell::RefCell};
use libm::{asin, cos, floor, sin};

use libertas::{
    IndexDirection, IndexedData, LIBERTAS_HUB_ENDPOINT, LibertasDateTime, LibertasDevice,
    LibertasEndpoint, LibertasEndpointHandlerResult, LibertasEndpointMessage,
    LibertasEndpointStandardStatus, LibertasUser, LogLevel, NotificationArgument,
    NotificationImportance, OP_ENDPOINT_DATA, OP_ENDPOINT_PEER_ALIVE, OP_ENDPOINT_PEER_DOWN,
    OP_ENDPOINT_PEER_UP, OP_ENDPOINT_REQ, OP_ENDPOINT_RSP, OP_ENDPOINT_SUB_REQ,
    libertas_data_open_indexed, libertas_data_read_indexed, libertas_data_read_indexed_range,
    libertas_data_read_single, libertas_data_remove_indexed_records, libertas_data_write_indexed,
    libertas_data_write_single, libertas_endpoint_report, libertas_endpoint_response,
    libertas_endpoint_subscribe_request, libertas_get_sys_ticks, libertas_get_utc_time,
    libertas_log, libertas_notification_send, libertas_register_device_listener,
    libertas_register_endpoint_status_listener, libertas_timer_cancel, libertas_timer_new_interval,
    libertas_timer_update_interval,
};
use libertas_hub::HubProtocol;
use libertas_macros::{
    LibertasAvroDecode, LibertasAvroEncode, LibertasExport, libertas_chart, libertas_data_schema,
    libertas_export, libertas_permissions, libertas_string_resources,
};
use libertas_matter::{
    MatterDevice, MatterDeviceSubscription, MatterResponse, MatterSubscriptionBatch,
    MatterSubscriptionCluster, decode_attribute_report, decode_command_response,
    decode_event_report,
    definitions::ValveConfigurationandControl::{
        attributes::{CurrentState, ValveFault},
        commands::{Close, Open},
        events::ValveStateChanged,
        types::ValveStateEnum,
    },
    frame::Operation,
    tlv::Nullable,
};
use libertas_weather::{
    SPRINKLER_FORECAST_HORIZON_SECONDS, SPRINKLER_HISTORY_WINDOW_SECONDS,
    SprinklerCurrentWeatherV1, SprinklerWeatherChangeV1, SprinklerWeatherCursorV1,
    SprinklerWeatherForecastPeriodV1, SprinklerWeatherForecastV1, SprinklerWeatherHistoryPeriodV1,
    SprinklerWeatherHistoryPeriodV2, SprinklerWeatherHistoryV2,
    SprinklerWeatherIncrementalReportV1, SprinklerWeatherLocationV1, SprinklerWeatherProtocolV1,
    SprinklerWeatherRecoveryErrorV1, SprinklerWeatherRecoveryV1, SprinklerWeatherSectionV1,
    SprinklerWeatherSnapshotV2, SprinklerWeatherTimeRangeV1,
};

const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
#[allow(dead_code)]
const SPRINKLER_PERMISSIONS: &[&str] = &["libertas.permission.ACCESS_FINE_LOCATION"];
const RECENT_WATER_WINDOW_SECONDS: u64 = 7 * 24 * 60 * 60;
const WEATHER_RETRY_SECONDS: u32 = 60;
const VALVE_COMMAND_TIMEOUT_SECONDS: u32 = 60;
const VALVE_DECISION_DELAY_SECONDS: u32 = 10;
const VALVE_ACCOUNTING_INTERVAL_SECONDS: u32 = 60;
const SCHEDULE_EVALUATION_INTERVAL_SECONDS: u32 = 60;
const SCHEDULE_CANDIDATE_INTERVAL_SECONDS: u64 = 15 * 60;
const VALVE_SUBSCRIPTION_MAX_INTERVAL_SECONDS: u16 = 30;
const VALVE_SUBSCRIPTION_STALE_SECONDS: u32 = (VALVE_SUBSCRIPTION_MAX_INTERVAL_SECONDS as u32) * 3;
const MAX_HOLD_OFFS: usize = 64;
const MAX_WATER_EVENTS: usize = 512;
const MAX_WATER_EVENT_RECORDS_SCANNED: usize = MAX_WATER_EVENTS * 2;
const MAX_REMINDER_RECIPIENTS: usize = 16;
const MAX_SPRINKLER_ZONES: usize = 32;
const WATER_EVENT_INDEX_KIND_COUNT: i64 = 2;
const REPORT_ACTIVITY_INDEXES_PER_SECOND: i64 = 1_024;
const REPORT_ACTIVITY_INDEXES_PER_ORIGIN: u16 = 256;
const MAX_REPORT_RANGE_SECONDS: u64 = 31 * 24 * 60 * 60;
const DEFAULT_REPORT_RANGE_SECONDS: u64 = 7 * 24 * 60 * 60;
const DEFAULT_WEATHER_HISTORY_SECONDS: u64 = 2 * 24 * 60 * 60;
// Water-usage rectangles share the report's real UTC axis, but their horizontal
// length represents water depth rather than elapsed time. The App therefore
// finds the shortest represented bucket in the complete snapshot and uses three
// quarters of that duration as one common mark span. Combining that span with
// the chart-wide maximum amount creates one seconds-per-millimeter scale: equal
// amounts must have equal DateTime lengths even when edge buckets are clipped.
// Keeping this projection entirely here is essential: clients render the
// supplied x/x2 coordinates literally and must never infer or restack data.
const WATER_USAGE_BUCKET_MARK_SPAN_NUMERATOR: u64 = 3;
const WATER_USAGE_BUCKET_MARK_SPAN_DENOMINATOR: u64 = 4;
const MAX_REPORT_WEATHER_PERIODS: usize = 1_024;
const MAX_REPORT_WEATHER_REPLACEMENT_RECORDS_SCANNED: usize = MAX_REPORT_WEATHER_PERIODS + 1;
const MAX_REPORT_WEATHER_OBSERVATIONS: usize = 4_096;
const MAX_REPORT_ACTIVITIES: usize = 4_096;
// One activity may occur in the primary archive and in every UTC-day overlap
// bucket touched by the longest accepted report query. Keep that worst-case
// read bounded across the entire multi-zone response as well as bounding the
// number of unique activities returned.
const MAX_REPORT_ACTIVITY_RECORDS_SCANNED: usize = MAX_REPORT_ACTIVITIES * 33;
const MAX_REPORT_DAILY_RECORDS_PER_ZONE: usize = 32;
const MAX_REPORT_MODELED_GAPS: usize = 4_096;
const MAX_REPORT_CHART_ROWS: usize = 100_000;
const MAX_REPORT_POINTS_PER_PATH: usize = 20_000;
// Provider periods repeat per zone, while modeled gaps and activities share
// response-wide caps. The actual distinct output points receive a separate
// path-limit check after the rate-change sweep.
const MAX_REPORT_BALANCE_INTERVALS_PER_ZONE: usize =
    MAX_REPORT_WEATHER_PERIODS * 2 + MAX_REPORT_MODELED_GAPS + MAX_REPORT_ACTIVITIES;
const LOCATION_EQUALITY_TOLERANCE_DEGREES: f64 = 0.000_001;
const MIN_RECENT_WEATHER_COVERAGE_SECONDS: u64 = 24 * 60 * 60;
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
const CONSERVATIVE_REFERENCE_ET_MILLIMETERS_PER_DAY: f32 = 5.0;
const MIN_REFERENCE_ET_MILLIMETERS_PER_DAY: f32 = 0.5;
const MAX_REFERENCE_ET_MILLIMETERS_PER_DAY: f32 = 10.0;
const UNSAFE_WEATHER_RETRY_SECONDS: u64 = 6 * 60 * 60;
const HUB_LOCATION_MAX_REPORT_INTERVAL_SECONDS: u32 = 60 * 60;
const MIN_WATERING_DURATION_SECONDS: u32 = 60;
const MAX_WATERING_DURATION_SECONDS: u32 = 2 * 60 * 60;
const SAFE_MINIMUM_TEMPERATURE_CELSIUS: f32 = 3.0;
const SAFE_MAXIMUM_WIND_METERS_PER_SECOND: f32 = 10.0;
const SAFE_MAXIMUM_GUST_METERS_PER_SECOND: f32 = 15.0;
const HIGH_RAIN_PROBABILITY_PERCENT: u8 = 50;
const FORECAST_LOOKAHEAD_SECONDS: u64 = 12 * 60 * 60;
const PREFERRED_DEFICIT_RATIO: f32 = 0.40;
const TARGET_DEFICIT_RATIO: f32 = 0.50;
const CRITICAL_DEFICIT_RATIO: f32 = 0.65;
const REPLENISHED_DEFICIT_RATIO: f32 = 0.20;
const OVERHEAD_MINIMUM_SOLAR_ELEVATION_DEGREES: f64 = -6.0;
const OVERHEAD_MAXIMUM_SOLAR_ELEVATION_DEGREES: f64 = 10.0;
const TARGET_SOLAR_ELEVATION_DEGREES: f64 = -1.0;
const HIGH_HUMIDITY_TARGET_SOLAR_ELEVATION_DEGREES: f64 = 3.0;
const BRIGHT_FINISH_SOLAR_ELEVATION_DEGREES: f64 = 25.0;
const HIGH_HUMIDITY_PERCENT: f32 = 85.0;
const DEFICIT_PENALTY_WEIGHT: f64 = 120.0;
const OVERHEAD_SOLAR_PENALTY_WEIGHT: f64 = 1.25;
const NON_OVERHEAD_SOLAR_PENALTY_WEIGHT: f64 = 0.35;
const EVAPOTRANSPIRATION_PENALTY_WEIGHT: f64 = 45.0;
const RAIN_PROBABILITY_PENALTY_WEIGHT: f64 = 0.15;
const RAIN_AMOUNT_PENALTY_WEIGHT: f64 = 100.0;
const HEAT_PENALTY_START_CELSIUS: f32 = 20.0;
const HEAT_PENALTY_WEIGHT: f64 = 0.4;
const FOLIAGE_WETNESS_PENALTY_WEIGHT: f64 = 0.8;
const BRIGHT_FINISH_PENALTY_WEIGHT: f64 = 1.5;
const DEGREES_TO_RADIANS: f64 = core::f64::consts::PI / 180.0;
const RADIANS_TO_DEGREES: f64 = 180.0 / core::f64::consts::PI;
const WINTERIZATION_REMINDER_LATITUDE_CUTOFF_DEGREES: f64 = 35.0;
const WINTERIZATION_REMINDER_INTERVAL_SECONDS: u64 = 30 * SECONDS_PER_DAY;
const NORTHERN_WINTERIZATION_SEASON_END_DAY: u16 = 90;
const SOUTHERN_WINTERIZATION_SEASON_END_DAY: u16 = 273;

/// Sprinkler database names
/// Stable resource identifiers and their user-facing descriptions.
pub const APP_STRINGS: [(&str, &str); 17] = [
    (
        "SPRINKLER_ZONE_MEMORY_V1",
        "Sprinkler water balance and settings for %1$s.",
    ),
    (
        "SPRINKLER_WATER_EVENTS_V1",
        "Sprinkler water history for %1$s.",
    ),
    (
        "SPRINKLER_SITE_LOCATION_V1",
        "Sprinkler site location for %1$s.",
    ),
    (
        "SPRINKLER_WATERING_MODE_V1",
        "Sprinkler watering mode for %1$s.",
    ),
    (
        "SPRINKLER_WINTERIZATION_REMINDER_V1",
        "Sprinkler winterization reminder state for %1$s.",
    ),
    (
        "SPRINKLER_REPORT_WEATHER_HISTORY_V1",
        "Indefinite sprinkler report weather history for %1$s.",
    ),
    (
        "SPRINKLER_WATERING_ACTIVITIES_V1",
        "Indefinite sprinkler watering activity for %1$s.",
    ),
    (
        "SPRINKLER_DAILY_REPORT_V1",
        "Indefinite daily sprinkler water accounting for %1$s.",
    ),
    (
        "SPRINKLER_WATERING_ACTIVITY_STATE_V1",
        "Current sprinkler watering activity for %1$s.",
    ),
    (
        "SPRINKLER_REPORT_WEATHER_OBSERVATIONS_V1",
        "Indefinite sprinkler current-weather observations for %1$s.",
    ),
    (
        "SPRINKLER_REPORT_WEATHER_ARCHIVE_STATE_V1",
        "Current sprinkler weather-archive generation for %1$s.",
    ),
    (
        "SPRINKLER_WINTERIZATION_WEATHER_REMINDER",
        "Freezing weather (%1$s) may damage your sprinkler system. Winterize it, then set Watering mode to Winterization.",
    ),
    (
        "SPRINKLER_WINTERIZATION_SEASON_REMINDER",
        "Cold season is approaching at this location. Winterize your sprinkler system, then set Watering mode to Winterization.",
    ),
    (
        "libertas.permission.ACCESS_FINE_LOCATION",
        "Allow the sprinkler task to receive location-specific conditions and forecasts from the weather agent.",
    ),
    (
        "SPRINKLER_WATERING_ACTIVITY_DAYS_V1",
        "Indefinite sprinkler watering activity overlap index for %1$s on UTC day %2$u.",
    ),
    (
        "SPRINKLER_MODELED_WEATHER_GAPS_V1",
        "Indefinite modeled sprinkler weather gaps for %1$s.",
    ),
    (
        "SPRINKLER_REPORT_WEATHER_HISTORY_V2",
        "Indefinite full-observation sprinkler report weather history for %1$s.",
    ),
];
const ZONE_DATA_RESOURCE: &str = APP_STRINGS[0].0;
const WATER_EVENTS_RESOURCE: &str = APP_STRINGS[1].0;
const SITE_LOCATION_RESOURCE: &str = APP_STRINGS[2].0;
const WATERING_MODE_RESOURCE: &str = APP_STRINGS[3].0;
const WINTERIZATION_REMINDER_RESOURCE: &str = APP_STRINGS[4].0;
const REPORT_WEATHER_HISTORY_RESOURCE: &str = APP_STRINGS[5].0;
const WATERING_ACTIVITIES_RESOURCE: &str = APP_STRINGS[6].0;
const DAILY_REPORT_RESOURCE: &str = APP_STRINGS[7].0;
const WATERING_ACTIVITY_STATE_RESOURCE: &str = APP_STRINGS[8].0;
const REPORT_WEATHER_OBSERVATIONS_RESOURCE: &str = APP_STRINGS[9].0;
const REPORT_WEATHER_ARCHIVE_STATE_RESOURCE: &str = APP_STRINGS[10].0;
const WINTERIZATION_WEATHER_NOTIFICATION_RESOURCE: &str = APP_STRINGS[11].0;
const WINTERIZATION_SEASON_NOTIFICATION_RESOURCE: &str = APP_STRINGS[12].0;
const WATERING_ACTIVITY_DAYS_RESOURCE: &str = APP_STRINGS[14].0;
const MODELED_WEATHER_GAPS_RESOURCE: &str = APP_STRINGS[15].0;
const REPORT_WEATHER_HISTORY_V2_RESOURCE: &str = APP_STRINGS[16].0;

/// Sprinkler time slot
/// Defines one half-open schedule or hold-off interval.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerTimeSlotV1 {
    /// Start time
    /// The inclusive start date and time in seconds since the Unix epoch.
    #[libertas_ui_header]
    pub starts_at: LibertasDateTime,
    /// Duration
    /// The interval length in seconds. A valid slot always has a nonzero
    /// duration and an end time representable by `LibertasDateTime`.
    #[libertas_time_interval]
    #[libertas_default(14400)]
    pub duration_seconds: u32,
}

impl SprinklerTimeSlotV1 {
    fn ends_at(self) -> Option<LibertasDateTime> {
        self.starts_at.checked_add(u64::from(self.duration_seconds))
    }

    fn overlaps(self, other: Self) -> bool {
        let (Some(self_end), Some(other_end)) = (self.ends_at(), other.ends_at()) else {
            return true;
        };
        self.starts_at < other_end && other.starts_at < self_end
    }
}

/// Sprinkler head type
/// Selects a nominal delivery profile used to translate the adaptive water need
/// into valve-open time. Weather history and observed valve time drive later
/// calculations, while the water amount adjuster provides the only user tuning.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerHeadTypeV1 {
    /// Surface drip
    /// Slow, targeted delivery from surface drip emitters.
    SurfaceDrip,
    /// Bubblers
    /// Concentrated delivery around trees, shrubs, or planting basins.
    Bubblers,
    /// Pop-up spray
    /// Broad, relatively fast delivery from fixed pop-up spray heads.
    PopupSpray,
    /// Low-rate rotors
    /// Slow broad-area delivery from high-efficiency or multi-stream rotors.
    RotorsLowRate,
    /// High-rate rotors
    /// Faster broad-area delivery from conventional rotor heads.
    RotorsHighRate,
}

/// Plant type
/// Selects the plant water-storage and weather-demand profile used by the
/// zone's adaptive water-balance calculation.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerPlantTypeV1 {
    /// Lawn
    /// Closely planted turf with shallow roots and regular water demand.
    Lawn,
    /// Flowers
    /// Ornamental flowering plants with moderate roots and water demand.
    Flowers,
    /// Vegetables
    /// Seasonal food crops with moderate roots and relatively high demand.
    Vegetables,
    /// Fruit trees
    /// Established fruit trees with deeper roots and moderate demand.
    FruitTrees,
    /// Citrus
    /// Citrus trees with deep roots and moderate-to-high demand.
    Citrus,
    /// Trees and bushes
    /// Established woody landscape plants with deep roots.
    TreesAndBushes,
    /// Xeriscape
    /// Drought-adapted planting with low weather-driven water demand.
    Xeriscape,
}

/// Sprinkler schedule condition
/// Explains the current calculated schedule and why watering may be deferred.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerScheduleConditionV1 {
    /// Initializing
    /// The controller is restoring persisted state and has not completed its
    /// first schedule calculation.
    Initializing,
    /// Water not needed
    /// No positive watering amount can be calculated from the current zone
    /// configuration.
    WaterNotNeeded,
    /// Forecast rain
    /// Significant high-probability rain is expected before watering is needed.
    ForecastRain,
    /// Waiting for safe weather
    /// Rain, freezing temperature, or excessive wind currently prevents
    /// watering; the displayed future slot is forecast-derived.
    WaitingForSafeWeather,
    /// Preempting a hold-off
    /// Watering is scheduled before a user hold-off because waiting until the
    /// first legal slot afterward would reach the critical plant-deficit
    /// threshold. Preemption requires a fresh, safe forecast whose expected
    /// rain cannot replace enough of the planned water.
    PreemptiveHoldOff,
    /// Held off
    /// A user hold-off moved watering to the first legal slot afterward. The
    /// displayed amount and duration are recalculated for that delayed start.
    HeldOff,
    /// Scheduled
    /// A watering slot has been calculated and is waiting to begin. With a
    /// fresh forecast and known location, this is the best nearby rising-sun
    /// period after considering plant demand, humidity, evapotranspiration,
    /// precipitation, temperature, sprinkler-head drift, and hold-offs.
    Scheduled,
    /// Valve command pending
    /// A Matter Valve command was sent and is awaiting confirmation.
    ValveCommandPending,
    /// Valve state unavailable
    /// The controller has not yet observed the Matter Valve's current state and
    /// will not start automatic watering.
    ValveStateUnavailable,
    /// Valve open
    /// The valve is observed open. Its actual open time is being added to the
    /// recent-water ledger whether the opening was automatic or manual.
    ValveOpen,
    /// Valve fault
    /// The Matter Valve reports a fault and automatic watering is inhibited.
    ValveFault,
    /// Offline weather estimate
    /// Live weather is unavailable, so the schedule uses recent local demand,
    /// a location-and-season estimate, or the conservative built-in fallback.
    OfflineWeatherEstimate,
}

/// Water demand source
/// Explains the reference evapotranspiration rate used to project the next
/// adaptive watering requirement.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerWaterDemandSourceV1 {
    /// Recent local weather
    /// Uses the average rate reconstructed from persisted local weather events.
    RecentLocalWeather,
    /// Location and season
    /// Uses an offline latitude, hemisphere, and time-of-year estimate.
    LocationAndSeason,
    /// Conservative default
    /// Uses the built-in reference rate because neither sufficient recent
    /// weather nor a valid cached location is available.
    ConservativeDefault,
}

/// Sprinkler zone configuration
/// Groups the two end-user scheduling settings in one configuration view while
/// allowing each setting to be changed independently.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerZoneConfigurationV1 {
    /// Water amount adjuster
    /// Percentage of the adaptive watering amount to apply. Use 100% for the
    /// adaptive amount, less than 100% for less water, and more than 100% for
    /// more water.
    #[libertas_number(min = 20, max = 200, step = 10)]
    pub watering_percent: u16,
    /// Hold-off periods
    /// Active sorted, non-overlapping intervals that watering must avoid.
    /// ----
    /// Hold-off period
    /// A half-open interval during which this zone cannot water.
    #[libertas_size(max = 64)]
    pub hold_off_periods: Vec<SprinklerTimeSlotV1>,
}

/// Active sprinkler state
/// Exposes calculation, water-balance, and valve diagnostics for a zone that is
/// actively calculating automatic watering. End-user settings are exposed by
/// `SprinklerZoneConfigurationV1` instead.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerZoneActiveStateV1 {
    /// Water demand source
    /// The best available source used to estimate when the root zone will next
    /// need water.
    pub water_demand_source: SprinklerWaterDemandSourceV1,
    /// Estimated reference evapotranspiration
    /// The reference water-loss rate used for projection, in millimeters per
    /// day. The plant-specific crop coefficient is applied separately.
    #[libertas_number(min = 0)]
    pub estimated_reference_evapotranspiration_millimeters_per_day: f32,
    /// Calculated at
    /// The date and time represented by this schedule calculation.
    pub calculated_at: LibertasDateTime,
    /// Condition
    /// The current watering decision or constraint.
    pub condition: SprinklerScheduleConditionV1,
    /// Next watering
    /// The best calculated valve-open slot. Weather or valve availability may
    /// change whether it can be executed, but never removes the estimate.
    pub next_watering: SprinklerTimeSlotV1,
    /// Planned water
    /// The water depth that the next automatic run intends to apply, in
    /// millimeters.
    #[libertas_number(min = 0)]
    pub planned_water_millimeters: f32,
    /// Estimated water deficit
    /// The estimated root-zone water deficit in millimeters after applying the
    /// persisted recent-water ledger.
    #[libertas_number(min = 0)]
    pub estimated_deficit_millimeters: f32,
    /// Recent precipitation
    /// Total precipitation represented by the retained seven-day ledger, in
    /// millimeters.
    #[libertas_number(min = 0)]
    pub recent_precipitation_millimeters: f32,
    /// Recent irrigation
    /// Total observed valve-open irrigation represented by the retained
    /// seven-day ledger, in millimeters.
    #[libertas_number(min = 0)]
    pub recent_irrigation_millimeters: f32,
    /// Valve open
    /// Whether the Matter Valve is currently observed open.
    pub valve_is_open: bool,
    /// Valve state known
    /// Whether the controller has received a non-null current state for this
    /// Matter Valve. Automatic watering is inhibited while this is false.
    pub valve_state_known: bool,
    /// Valve fault bitmap
    /// The current Matter Valve Configuration and Control fault bitmap.
    pub valve_fault_bitmap: u16,
}

/// Watering mode
/// Selects whether the sprinkler system is actively watering or winterized.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerWateringModeV1 {
    /// Active
    /// Automatic watering is enabled.
    Active,
    /// Winterization
    /// Automatic watering is shut down for the cold season.
    Winterization,
}

/// Winterization reminder reason
/// Records whether the latest reminder came from seasonal location guidance or
/// fresh freezing-weather evidence.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerWinterizationReminderReasonV1 {
    /// Location and season
    /// The site's latitude and current season indicate that cold weather is
    /// approaching.
    LocationAndSeason,
    /// Freezing weather
    /// Fresh current conditions or forecast data show a temperature at or
    /// below the sprinkler's cold-weather safety threshold.
    FreezingWeather,
}

/// Winterization reminder memory
/// Persists the most recent system-wide reminder so restarts do not create a
/// notification burst.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub struct SprinklerWinterizationReminderMemoryV1 {
    /// Last reminded at
    /// The date and time when the reminder state was persisted immediately
    /// before its notification was submitted.
    pub last_reminded_at: LibertasDateTime,
    /// Reminder reason
    /// The evidence used for the latest reminder.
    pub reason: SprinklerWinterizationReminderReasonV1,
}

/// Sprinkler state
/// Presents the essential current condition and next watering schedule for a
/// regular user, without configuration or diagnostic details.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum SprinklerZoneStateV1 {
    /// Active
    /// Automatic watering is enabled and a next watering slot is available.
    #[libertas_ui_header]
    ActiveV1 {
        /// Condition
        /// The current watering decision or constraint.
        condition: SprinklerScheduleConditionV1,
        /// Next watering
        /// The best calculated valve-open slot. Weather or valve availability
        /// may change whether it can be executed, but never removes the
        /// estimate.
        next_watering: SprinklerTimeSlotV1,
    },
    /// Winterization
    /// Automatic watering is disabled for the entire sprinkler system and no
    /// watering slot is scheduled.
    WinterizationV1,
}

/// Advanced sprinkler state
/// Distinguishes complete active zone data from system winterization for users
/// diagnosing a zone or controlling the system watering mode.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum SprinklerZoneAdvancedStateV1 {
    /// Active
    /// Automatic watering is enabled and all current zone data is available.
    ActiveV1 {
        /// Current state
        /// The complete current calculation, water balance, and valve status
        /// for the active zone.
        current: SprinklerZoneActiveStateV1,
    },
    /// Winterization
    /// Automatic watering is disabled for the entire sprinkler system and no
    /// watering slot is scheduled.
    WinterizationV1,
}

/// Sprinkler zone protocol
/// Reads or subscribes to regular-user state, retrieves advanced diagnostics,
/// presents both end-user settings in one configuration view, and updates the
/// water amount adjuster or hold-off constraints independently.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum SprinklerZoneProtocolV1 {
    /// Get state
    /// Requests the essential regular-user state. This first protocol variant
    /// is the default GUI action and may establish a subscription because the
    /// endpoint operation is outside this value.
    #[libertas_request]
    #[libertas_subscription_request]
    #[libertas_next_response(StateV1)]
    GetStateV1,
    /// State
    /// Returns or reports the essential current condition and next watering
    /// schedule. Advanced diagnostics and configuration are available on
    /// demand.
    #[libertas_response]
    #[libertas_subscription_data]
    #[libertas_next_request("GetAdvancedStateV1,GetConfigurationV1")]
    StateV1 {
        /// Sprinkler state
        /// The essential active-zone state or the Winterization state.
        state: SprinklerZoneStateV1,
    },
    /// Get advanced state
    /// Requests complete calculation, water-balance, and valve diagnostics for
    /// this zone.
    #[libertas_request]
    #[libertas_next_response(AdvancedStateV1)]
    GetAdvancedStateV1,
    /// Advanced state
    /// Returns complete calculation, water-balance, and valve diagnostics after
    /// an advanced-state request or watering-mode update.
    #[libertas_response]
    #[libertas_next_request("GetConfigurationV1,SetWateringModeV1")]
    AdvancedStateV1 {
        /// Watering mode
        /// The current system-wide mode, exposed explicitly so its control can
        /// initialize from this response.
        mode: SprinklerWateringModeV1,
        /// Sprinkler state
        /// The active zone diagnostics or the Winterization state.
        state: SprinklerZoneAdvancedStateV1,
    },
    /// Get configuration
    /// Opens the zone's single end-user configuration view containing the water
    /// amount adjuster and hold-off periods.
    #[libertas_request]
    #[libertas_next_response(ConfigurationV1)]
    GetConfigurationV1,
    /// Configuration
    /// Returns both end-user settings together and offers a separate action for
    /// changing either one.
    #[libertas_response]
    #[libertas_next_request("SetWaterAmountAdjusterV1,ReplaceHoldOffPeriodsV1")]
    ConfigurationV1 {
        /// Configuration
        /// The current water amount adjuster and hold-off periods.
        #[libertas_ui_header]
        configuration: SprinklerZoneConfigurationV1,
    },
    /// Set water amount adjuster
    /// Independently updates the user tuning parameter used by the adaptive
    /// calculation.
    #[libertas_request]
    #[libertas_next_response(ConfigurationV1)]
    SetWaterAmountAdjusterV1 {
        /// Water amount adjuster
        /// Percentage of the adaptive watering amount to apply. Use 100% for
        /// the adaptive amount, less than 100% for less water, and more than
        /// 100% for more water.
        #[libertas_number(min = 20, max = 200, step = 10)]
        #[libertas_copy_from("$.configuration.watering_percent")]
        watering_percent: u16,
    },
    /// Replace hold-off periods
    /// Independently replaces all scheduling constraints for this zone.
    /// Overlapping or touching periods are normalized into sorted merged
    /// intervals.
    #[libertas_request]
    #[libertas_next_response(ConfigurationV1)]
    ReplaceHoldOffPeriodsV1 {
        /// Hold-off periods
        /// The complete replacement list, limited to 64 valid intervals.
        /// ----
        /// Hold-off period
        /// A half-open interval during which the schedule cannot water.
        #[libertas_size(max = 64)]
        #[libertas_copy_from("$.configuration.hold_off_periods")]
        hold_off_periods: Vec<SprinklerTimeSlotV1>,
    },
    /// Set watering mode
    /// Selects Active or Winterization for the entire sprinkler system. The
    /// selected mode persists across restarts and internet outages.
    #[libertas_request]
    #[libertas_next_response(AdvancedStateV1)]
    SetWateringModeV1 {
        /// Watering mode
        /// Active enables automatic watering. Winterization shuts it down.
        #[libertas_copy_from("$.mode")]
        mode: SprinklerWateringModeV1,
    },
}

/// Sprinkler report time range
/// Selects one bounded half-open UTC interval from the indefinitely retained
/// report archive. A single response is limited to 31 days; older ranges remain
/// queryable with another request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SprinklerReportTimeRangeV1 {
    /// Start time
    /// The inclusive beginning of the requested report window.
    starts_at: LibertasDateTime,
    /// End time
    /// The exclusive end of the requested report window.
    ends_before: LibertasDateTime,
}

/// Water usage bucket
/// Selects the UTC calendar interval used to aggregate indefinitely retained
/// daily water accounting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SprinklerReportBucketV1 {
    /// Day
    /// Groups rain and irrigation into UTC calendar days.
    Day,
    /// Week
    /// Groups rain and irrigation into Monday-through-Sunday UTC weeks.
    Week,
}

/// Water input type
/// Distinguishes observed and planned sources contributing water to a zone.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerWaterInputTypeV1 {
    /// Rain
    /// Provider-recorded precipitation.
    Rain,
    /// Irrigation
    /// Water estimated from observed valve-open time and the sprinkler-head
    /// profile; this is not a flow-meter measurement.
    Irrigation,
    /// Forecast rain
    /// Provider forecast precipitation for a future interval.
    ForecastRain,
    /// Scheduled water
    /// Planned automatic irrigation that has not begun yet.
    ScheduledWater,
}

/// Water-balance series
/// Identifies the calculated available-water line or one agronomic reference
/// line in each zone facet.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerWaterBalanceSeriesV1 {
    /// Available water
    /// Calculated root-zone available water.
    AvailableWater,
    /// Field capacity
    /// The modeled root zone is full.
    FieldCapacity,
    /// Watering threshold
    /// The normal target-deficit boundary at which watering becomes due.
    WateringThreshold,
    /// Critical threshold
    /// The dry boundary used to prioritize urgent watering.
    CriticalThreshold,
}

/// Empty report state
/// Supplies an honest localized annotation when a configured zone has no rows
/// for one chart in the requested time window.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerReportEmptyStateV1 {
    /// No recorded watering activity
    /// This zone has no scheduled, skipped, manual, or completed watering
    /// activity in the requested window.
    NoRecordedWateringActivity,
    /// No recorded water input
    /// This zone has no positive rain or observed-irrigation input in the
    /// requested window.
    NoRecordedWaterInput,
    /// No recorded modeled ET gap
    /// Provider history covers the represented zone interval, or no modeled
    /// fallback interval is retained for this window.
    NoRecordedModeledEtGap,
}

/// Watering origin
/// Records whether a valve opening was initiated by the sprinkler controller,
/// observed as an externally owned manual run, or predates origin tracking.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerWateringOriginV1 {
    /// Automatic
    /// The controller initiated the timed valve-open command.
    Automatic,
    /// Manual
    /// The controller observed but did not initiate or own the valve opening.
    Manual,
    /// Legacy unknown
    /// The irrigation predates durable origin tracking and cannot be classified
    /// honestly as automatic or manual.
    LegacyUnknown,
}

/// Watering outcome
/// Durable lifecycle state shown on the activity timeline and decision markers.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerWateringOutcomeV1 {
    /// Scheduled
    /// A calculated future automatic run is still planned.
    Scheduled,
    /// Command pending
    /// The durable automatic plan was persisted and the timed valve-open
    /// command was submitted, but an open has not yet been observed.
    CommandPending,
    /// Running
    /// The valve is observed open and actual delivered water is being counted.
    Running,
    /// Completed
    /// The valve closed after a nonzero observed open interval.
    Completed,
    /// Skipped
    /// A due plan did not run because a durable safety or scheduling reason
    /// prevented it.
    Skipped,
    /// Failed
    /// A valve command or confirmation failed before useful watering completed.
    Failed,
    /// Superseded
    /// A newer calculation replaced a future plan before it became due.
    Superseded,
}

/// Watering reason
/// Gives the durable controller fact explaining a schedule, skip, failure, or
/// interruption. It never claims a synthetic heat-adjustment percentage.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerWateringReasonV1 {
    /// Smart schedule
    /// The adaptive water-balance schedule selected the run.
    SmartSchedule,
    /// Manual operation
    /// The valve opening was externally initiated.
    ManualOperation,
    /// Forecast rain
    /// Forecast precipitation deferred or replaced the due run.
    ForecastRain,
    /// Observed rain
    /// Fresh observed precipitation stopped or prevented watering.
    ObservedRain,
    /// Freezing weather
    /// Fresh observed or forecast temperature was unsafe for irrigation.
    FreezingWeather,
    /// Excessive wind
    /// Fresh observed or forecast wind was unsafe for the sprinkler-head type.
    ExcessiveWind,
    /// Other unsafe weather
    /// Fresh weather was unsafe but did not match a more specific retained
    /// reason.
    OtherUnsafeWeather,
    /// Hold-off
    /// A configured hold-off period prevented the due run.
    HoldOff,
    /// Winterization
    /// System-wide winterization disabled automatic watering.
    Winterization,
    /// Valve unavailable
    /// No trustworthy current valve state was available.
    ValveUnavailable,
    /// Valve fault
    /// The Matter Valve reported a fault.
    ValveFault,
    /// Command failed
    /// The Matter command could not be encoded or returned a failure status.
    CommandFailed,
    /// Command timeout
    /// The Matter command did not produce a timely confirmation; the observed
    /// valve state still determines whether watering ran.
    CommandTimeout,
    /// No open observed
    /// A timed open was requested but no open interval was observed.
    NoOpenObserved,
    /// Recalculated
    /// A newer water-balance calculation replaced a future plan.
    Recalculated,
    /// Legacy unknown
    /// The retained record does not contain enough evidence for a more precise
    /// reason.
    LegacyUnknown,
}

/// Persisted watering activity
/// Keeps scheduled and actual timing separate so the report can explain what
/// was planned, what happened, and why a run was skipped or failed.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWateringActivityV1 {
    /// Activity index
    /// Stable signed database index reused for every lifecycle update.
    pub activity_index: i64,
    /// Activity ordinal
    /// Collision-safe sequence within the same anchor second and origin.
    pub activity_ordinal: u16,
    /// Origin
    /// Automatic, manual, or unclassifiable legacy activity.
    pub origin: SprinklerWateringOriginV1,
    /// Outcome
    /// Current or terminal durable lifecycle state.
    pub outcome: SprinklerWateringOutcomeV1,
    /// Reason
    /// Durable explanation for the current or terminal outcome.
    pub reason: SprinklerWateringReasonV1,
    /// Scheduled start
    /// Planned automatic start. Manual and legacy activities omit it.
    pub scheduled_starts_at: Option<LibertasDateTime>,
    /// Scheduled duration
    /// Planned automatic valve-open duration in seconds. It is never
    /// overwritten with the actual observed duration.
    #[libertas_time_interval]
    pub scheduled_duration_seconds: Option<u32>,
    /// Planned water
    /// Planned automatic water depth in millimeters.
    #[libertas_number(min = 0)]
    pub planned_water_millimeters: Option<f32>,
    /// Actual start
    /// First observed valve-open time, absent until an open is observed.
    pub actual_starts_at: Option<LibertasDateTime>,
    /// Actual duration
    /// Accounted observed valve-open duration in seconds.
    #[libertas_time_interval]
    pub actual_duration_seconds: Option<u32>,
    /// Applied water
    /// Water depth estimated from actual open time and the sprinkler-head
    /// profile, not from a flow meter.
    #[libertas_number(min = 0)]
    pub applied_water_millimeters: Option<f32>,
    /// Water amount adjuster
    /// Zone adjuster in effect when this activity was created.
    #[libertas_number(min = 20, max = 200, step = 10)]
    pub watering_percent: u16,
    /// Updated at
    /// UTC time represented by the latest persisted lifecycle update.
    pub updated_at: LibertasDateTime,
}

/// Legacy current watering activity state
/// Decodes the original bounded nonterminal snapshot. New writes use
/// `SprinklerWateringActivityStateV2`, which can also repair a terminal audit
/// row after a crash.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWateringActivityStateV1 {
    /// Current activity
    /// The complete authoritative nonterminal snapshot to reconcile, or no
    /// value when every archived activity is terminal. Keeping the snapshot in
    /// this bounded record makes nonterminal restart reconciliation independent
    /// of an indefinite archive scan.
    pub current_activity: Option<SprinklerWateringActivityV1>,
}

/// Authoritative watering activity state
/// Stores the latest complete lifecycle snapshot before its matching audit-row
/// write. The explicit current flag distinguishes a restart-reconcilable
/// nonterminal activity from a terminal snapshot that only needs audit repair.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWateringActivityStateV2 {
    /// Latest activity
    /// Complete latest lifecycle snapshot, or no value before the first
    /// activity. Terminal snapshots remain here until a newer activity begins.
    pub latest_activity: Option<SprinklerWateringActivityV1>,
    /// Activity is current
    /// True only when the latest activity is scheduled, command-pending, or
    /// running and must be reconciled at startup.
    pub activity_is_current: bool,
}

/// Legacy report weather archive state
/// Decodes the original site-generation record whose history-clear handshake
/// could remain pending indefinitely. New writes use
/// `SprinklerReportWeatherArchiveStateV2`.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerReportWeatherArchiveStateV1 {
    /// Generation
    /// Monotonically increases when the Hub location or provider history site
    /// is cleared.
    pub generation: u64,
    /// Site location
    /// The Hub location associated with this generation, when known. The first
    /// valid location binds an unassociated generation without advancing it.
    pub location: Option<SprinklerWeatherLocationV1>,
    /// Awaiting history clear
    /// A direct Hub location change already opened this generation, so the
    /// weather agent's matching history-clear event must not open another one.
    pub awaiting_history_clear: bool,
}

/// Report weather archive state
/// Selects the active physical-site generation while older generations remain
/// retained but are not mixed into the current sprinkler report. An explicit
/// weather-stream site replacement or site-bound reset binds a generation;
/// section clears never act as site-boundary acknowledgements.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerReportWeatherArchiveStateV2 {
    /// Generation
    /// Monotonically increases for every accepted transition between distinct
    /// provider sites.
    pub generation: u64,
    /// Site location
    /// Provider location explicitly bound to this generation, when known.
    pub location: Option<SprinklerWeatherLocationV1>,
}

/// Daily sprinkler report
/// An indefinitely retained UTC-day checkpoint and water-accounting rollup.
/// It makes an old water-balance window queryable without replaying every event
/// since installation.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerDailyReportV1 {
    /// Day start
    /// Inclusive UTC calendar-day boundary used as the indexed record key.
    pub starts_at: LibertasDateTime,
    /// Day end
    /// Exclusive UTC calendar-day boundary.
    pub ends_before: LibertasDateTime,
    /// Coverage start
    /// First instant represented by this checkpoint. It can be later than the
    /// UTC day boundary on the installation or site-transition day.
    pub coverage_starts_at: LibertasDateTime,
    /// Coverage end
    /// Exclusive last instant represented by this checkpoint. It can be before
    /// the day boundary for the current partial day.
    pub coverage_ends_before: LibertasDateTime,
    /// Root-zone capacity
    /// Modeled plant-profile capacity in millimeters.
    #[libertas_number(min = 0)]
    pub capacity_millimeters: f32,
    /// Opening deficit
    /// Calculated root-zone deficit at the beginning of the represented day.
    #[libertas_number(min = 0)]
    pub opening_deficit_millimeters: f32,
    /// Closing deficit
    /// Calculated root-zone deficit at the represented end or latest known time.
    #[libertas_number(min = 0)]
    pub closing_deficit_millimeters: f32,
    /// Rain
    /// Provider-recorded precipitation assigned to this UTC day.
    #[libertas_number(min = 0)]
    pub precipitation_millimeters: f32,
    /// Reference evapotranspiration
    /// Provider-recorded reference ET assigned to this UTC day before the
    /// plant-specific crop coefficient is applied.
    #[libertas_number(min = 0)]
    pub reference_evapotranspiration_millimeters: f32,
    /// Modeled reference evapotranspiration
    /// Reference ET used to fill intervals not covered by provider history.
    #[libertas_number(min = 0)]
    pub modeled_reference_evapotranspiration_millimeters: f32,
    /// Modeled demand source
    /// Source of the modeled ET amount, omitted when provider history covers
    /// the complete represented interval.
    pub modeled_demand_source: Option<SprinklerWaterDemandSourceV1>,
    /// Provider weather coverage
    /// Number of seconds in the represented interval covered by accepted
    /// provider weather periods.
    #[libertas_time_interval]
    pub provider_weather_coverage_seconds: u32,
    /// Irrigation
    /// Water depth estimated from observed valve-open intervals assigned to
    /// this UTC day.
    #[libertas_number(min = 0)]
    pub irrigation_millimeters: f32,
    /// Complete day
    /// Whether the checkpoint represents the complete UTC day rather than the
    /// current partial day.
    pub complete: bool,
    /// Calculated at
    /// UTC time when this checkpoint was last rebuilt.
    pub calculated_at: LibertasDateTime,
}

/// Modeled weather gap
/// One exact provider-uncovered interval whose fallback reference-ET source and
/// rate are frozen when the interval is first recorded. A later accepted
/// provider period may clip or supersede this interval; later fallback estimate
/// changes do not rewrite its provenance.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerModeledWeatherGapV1 {
    /// Start time
    /// Inclusive beginning of the provider-uncovered interval.
    pub starts_at: LibertasDateTime,
    /// End time
    /// Exclusive end of the provider-uncovered interval.
    pub ends_before: LibertasDateTime,
    /// Reference evapotranspiration rate
    /// Frozen fallback reference-ET rate in millimeters per UTC day.
    #[libertas_number(min = 0)]
    pub reference_evapotranspiration_millimeters_per_day: f32,
    /// Demand source
    /// Frozen recent-weather, location/season, or conservative provenance.
    pub demand_source: SprinklerWaterDemandSourceV1,
    /// Recorded at
    /// UTC time at which this interval first acquired its frozen provenance.
    pub recorded_at: LibertasDateTime,
}

/// Water-balance point
/// One calculated or reference point in an all-zone root-zone balance chart.
/// Available water is modeled from rain, ET, and observed valve time; it is not
/// a soil-moisture sensor measurement.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWaterBalancePointV1 {
    /// Time
    /// UTC time represented by this balance point.
    #[libertas_chart_channel(x, tooltip, key)]
    #[libertas_chart_scale(id = report_time, kind = utc)]
    pub at: LibertasDateTime,
    /// Available water
    /// Modeled available root-zone water from 0 through 100 percent.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(
        id = available_water_percent,
        kind = linear,
        min = 0,
        max = 100,
        zero = true
    )]
    pub available_water_percent: f32,
    /// Series
    /// Calculated available water or one agronomic reference line.
    #[libertas_chart_channel(color, detail, tooltip)]
    pub series: SprinklerWaterBalanceSeriesV1,
    /// Zone
    /// Configured zone represented by this chart facet.
    #[libertas_chart_channel(row, tooltip)]
    #[libertas_chart_scale(id = report_zone, kind = band)]
    #[libertas_device_type("BQEBAUABgQED")]
    pub zone: LibertasDevice,
}

/// Water balance
/// Calculated available water and agronomic reference lines for every zone.
#[libertas_chart(line)]
pub type SprinklerWaterBalanceChartV1 = Vec<SprinklerWaterBalancePointV1>;

/// Watering timeline row
/// One actual or planned interval for one configured zone.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWateringTimelineRowV1 {
    /// Start time
    /// Actual start when observed, otherwise the scheduled start.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = report_time, kind = utc)]
    pub starts_at: LibertasDateTime,
    /// End time
    /// Actual end when observed, otherwise the scheduled end.
    #[libertas_chart_channel(x2, tooltip)]
    pub ends_at: LibertasDateTime,
    /// Zone
    /// Configured zone valve. The client resolves the device's normal display
    /// name, so configuring a duplicate report-only name is unnecessary.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = report_zone, kind = band)]
    #[libertas_device_type("BQEBAUABgQED")]
    pub zone: LibertasDevice,
    /// Outcome
    /// Durable activity lifecycle state.
    #[libertas_chart_channel(color, detail, tooltip)]
    pub outcome: SprinklerWateringOutcomeV1,
    /// Origin
    /// Automatic, manual, or legacy unknown.
    #[libertas_chart_channel(tooltip)]
    pub origin: SprinklerWateringOriginV1,
    /// Reason
    /// Durable activity explanation.
    #[libertas_chart_channel(tooltip)]
    pub reason: SprinklerWateringReasonV1,
    /// Scheduled duration
    /// Planned automatic duration in seconds, or zero when unavailable.
    #[libertas_chart_channel(tooltip)]
    #[libertas_time_interval]
    pub scheduled_duration_seconds: u32,
    /// Actual duration
    /// Accounted observed duration in seconds, or zero until unavailable.
    #[libertas_chart_channel(tooltip)]
    #[libertas_time_interval]
    pub actual_duration_seconds: u32,
    /// Activity key
    /// Stable focus key unique across every configured zone.
    #[libertas_chart_channel(key)]
    pub activity_key: String,
}

/// Watering-event timeline
/// Shows what actually happened across zones alongside scheduled and skipped
/// activities.
#[libertas_chart(rect)]
pub type SprinklerWateringTimelineMarksV1 = Vec<SprinklerWateringTimelineRowV1>;

/// Empty timeline annotation
/// Keeps an otherwise-idle configured zone visible without fabricating a
/// watering event or duration.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerTimelineEmptyZoneRowV1 {
    /// Horizontal center
    /// A singleton discrete position centers the annotation without inventing
    /// a report timestamp.
    #[libertas_chart_channel(x)]
    #[libertas_chart_scale(kind = point, guide = none)]
    pub horizontal_center: bool,
    /// Zone
    /// Configured valve device whose empty timeline lane is annotated.
    #[libertas_chart_channel(y, tooltip, key)]
    #[libertas_chart_scale(id = report_zone, kind = band, guide = none)]
    #[libertas_device_type("BQEBAUABgQED")]
    pub zone: LibertasDevice,
    /// Empty state
    /// Localized explanation for the absence of activity marks.
    #[libertas_chart_channel(text, tooltip)]
    pub empty_state: SprinklerReportEmptyStateV1,
}

/// Empty watering timeline zones
/// Text annotations for configured zones with no watering activity.
#[libertas_chart(text)]
pub type SprinklerTimelineEmptyZonesV1 = Vec<SprinklerTimelineEmptyZoneRowV1>;

/// Watering-event timeline
/// Layers real activity intervals with honest annotations for idle zones.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(layer)]
pub struct SprinklerWateringTimelineChartV1 {
    /// Watering activity
    /// Scheduled and observed activity intervals.
    pub activities: SprinklerWateringTimelineMarksV1,
    /// Empty zones
    /// Configured zones with no activity in the requested window.
    pub empty_zones: SprinklerTimelineEmptyZonesV1,
}

/// Water-usage row
/// One server-positioned water-amount segment on a configured zone's shared
/// timeline lane.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWaterUsageRowV1 {
    /// Bucket
    /// Real sparse UTC day or week bucket represented by this segment. This is
    /// tooltip data; the server-computed display coordinates own the time axis.
    #[libertas_chart_channel(tooltip)]
    pub at: LibertasDateTime,
    /// Time
    /// Server-computed horizontal start. The first colored segment begins at
    /// the real bucket time and later segments continue its amount stack.
    #[libertas_chart_channel(x)]
    #[libertas_chart_scale(id = report_time, kind = utc)]
    pub segment_starts_at: LibertasDateTime,
    /// Segment end
    /// Server-computed horizontal end. It is a display coordinate whose distance
    /// from the start encodes water depth; it is not an observed event end.
    #[libertas_chart_channel(x2)]
    pub segment_ends_at: LibertasDateTime,
    /// Water amount
    /// Exact rain or irrigation depth represented by this colored segment.
    #[libertas_chart_channel(tooltip)]
    #[libertas_number(min = 0)]
    pub amount_millimeters: f32,
    /// Input type
    /// Rain, observed irrigation, forecast rain, or scheduled irrigation.
    #[libertas_chart_channel(color, detail, tooltip)]
    pub input_type: SprinklerWaterInputTypeV1,
    /// Zone
    /// Configured zone valve used as one categorical timeline lane. The client
    /// resolves its normal device display name.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = report_zone, kind = band)]
    #[libertas_device_type("BQEBAUABgQED")]
    pub zone: LibertasDevice,
}

/// Water usage
/// Places every zone on one shared time axis. Each sparse bucket starts one
/// horizontal colored stack whose segment lengths encode exact water amounts.
#[libertas_chart(rect)]
pub type SprinklerWaterUsageMarksV1 = Vec<SprinklerWaterUsageRowV1>;

/// Empty water-usage lane annotation
/// Keeps an otherwise-dry configured zone visible without fabricating a time or
/// water amount.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWaterUsageEmptyZoneRowV1 {
    /// Horizontal center
    /// A singleton discrete position centers the annotation without inventing a
    /// report timestamp.
    #[libertas_chart_channel(x)]
    #[libertas_chart_scale(kind = point, guide = none)]
    pub horizontal_center: bool,
    /// Zone
    /// Configured valve device whose empty timeline lane is annotated.
    #[libertas_chart_channel(y, tooltip, key)]
    #[libertas_chart_scale(id = report_zone, kind = band, guide = none)]
    #[libertas_device_type("BQEBAUABgQED")]
    pub zone: LibertasDevice,
    /// Empty state
    /// Localized explanation for the absence of water-usage marks.
    #[libertas_chart_channel(text, tooltip)]
    pub empty_state: SprinklerReportEmptyStateV1,
}

/// Empty water-usage lanes
/// Text annotations for configured zones with no positive water input.
#[libertas_chart(text)]
pub type SprinklerWaterUsageEmptyZonesV1 = Vec<SprinklerWaterUsageEmptyZoneRowV1>;

/// Empty faceted-zone annotation
/// Centers localized text inside a Device facet without inventing a time or
/// quantitative value.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerFacetedEmptyZoneRowV1 {
    /// Horizontal center
    /// Singleton discrete x position for the annotation.
    #[libertas_chart_channel(x)]
    #[libertas_chart_scale(kind = point, guide = none)]
    pub horizontal_center: bool,
    /// Vertical center
    /// Singleton discrete y position for the annotation.
    #[libertas_chart_channel(y)]
    #[libertas_chart_scale(kind = point, guide = none)]
    pub vertical_center: bool,
    /// Zone
    /// Configured valve device represented by this empty facet.
    #[libertas_chart_channel(row, tooltip, key)]
    #[libertas_chart_scale(id = report_zone, kind = band, guide = none)]
    #[libertas_device_type("BQEBAUABgQED")]
    pub zone: LibertasDevice,
    /// Empty state
    /// Localized explanation for the absence of quantitative marks.
    #[libertas_chart_channel(text, tooltip)]
    pub empty_state: SprinklerReportEmptyStateV1,
}

/// Empty faceted zones
/// Text annotations for configured Device facets with no quantitative rows.
#[libertas_chart(text)]
pub type SprinklerFacetedEmptyZonesV1 = Vec<SprinklerFacetedEmptyZoneRowV1>;

/// Water usage
/// Layers positive rain/irrigation bars with annotations for dry, idle zones.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(layer)]
pub struct SprinklerWaterUsageChartV1 {
    /// Water inputs
    /// Positive observed and planned water-input segments.
    pub inputs: SprinklerWaterUsageMarksV1,
    /// Empty zones
    /// Configured zones with no positive water input in the requested window.
    pub empty_zones: SprinklerWaterUsageEmptyZonesV1,
}

/// Weather data source
/// Distinguishes completed provider observations from the latest forecast.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerWeatherChartSourceV1 {
    /// Historical observation
    /// A completed provider historical period.
    HistoricalObservation,
    /// Current observation
    /// A retained higher-frequency current-condition sample.
    CurrentObservation,
    /// Forecast
    /// A future value from the latest available forecast snapshot.
    Forecast,
    /// Recent-weather estimate
    /// A retained recent local ET rate fills a provider-history gap.
    RecentWeatherEstimate,
    /// Location-and-season estimate
    /// The offline latitude, hemisphere, and season model fills a history gap.
    LocationAndSeasonEstimate,
    /// Conservative estimate
    /// The built-in 5 mm/day reference rate fills a history gap.
    ConservativeEstimate,
}

/// Wind series
/// Separates sustained wind and gusts for observed and forecast periods.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerWindSeriesV1 {
    /// Historical sustained wind
    /// Sustained wind from a completed historical period.
    HistoricalWind,
    /// Historical gust
    /// Peak gust from a completed historical period.
    HistoricalGust,
    /// Current sustained wind
    /// Sustained wind from a retained current observation.
    CurrentWind,
    /// Current gust
    /// Peak gust from a retained current observation.
    CurrentGust,
    /// Forecast sustained wind
    /// Predicted sustained wind.
    ForecastWind,
    /// Forecast gust
    /// Predicted peak gust.
    ForecastGust,
}

/// Weather interval value
/// One observed or forecast ET amount over an explicit interval.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerEtRowV1 {
    /// Start time
    /// Inclusive provider period start.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = report_time, kind = utc)]
    pub starts_at: LibertasDateTime,
    /// End time
    /// Exclusive provider period end.
    #[libertas_chart_channel(x2, tooltip)]
    pub ends_at: LibertasDateTime,
    /// Reference evapotranspiration
    /// Provider FAO-56 reference ET in millimeters.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(kind = linear, min = 0, zero = true)]
    pub reference_evapotranspiration_millimeters: f32,
    /// Source
    /// Observed or forecast.
    #[libertas_chart_channel(color, detail, tooltip)]
    pub source: SprinklerWeatherChartSourceV1,
    /// Stable key
    /// Server-generated identity for this interval and source.
    #[libertas_chart_channel(key)]
    pub sample_key: String,
}

/// Reference evapotranspiration
/// Observed and forecast ET on the shared report time axis.
#[libertas_chart(bar)]
pub type SprinklerEtChartV1 = Vec<SprinklerEtRowV1>;

/// Zone modeled-ET row
/// One exact provider-uncovered interval and the fallback reference-ET amount
/// used by one configured zone.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerModeledEtRowV1 {
    /// Start time
    /// Inclusive modeled interval start.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = report_time, kind = utc)]
    pub starts_at: LibertasDateTime,
    /// End time
    /// Exclusive modeled interval end.
    #[libertas_chart_channel(x2, tooltip)]
    pub ends_at: LibertasDateTime,
    /// Reference evapotranspiration
    /// Fallback reference ET applied during this provider-history gap.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(kind = linear, min = 0, zero = true)]
    pub reference_evapotranspiration_millimeters: f32,
    /// Source
    /// Recent-weather, location-and-season, or conservative estimate.
    #[libertas_chart_channel(color, detail, tooltip)]
    pub source: SprinklerWeatherChartSourceV1,
    /// Zone
    /// Configured zone whose retained calculation used this fallback.
    #[libertas_chart_channel(row, tooltip)]
    #[libertas_chart_scale(id = report_zone, kind = band)]
    #[libertas_device_type("BQEBAUABgQED")]
    pub zone: LibertasDevice,
    /// Stable key
    /// Server-generated identity for this zone, interval, and source.
    #[libertas_chart_channel(key)]
    pub sample_key: String,
}

/// Zone modeled evapotranspiration
/// Provider-history gaps and their frozen fallback source for every zone.
#[libertas_chart(bar)]
pub type SprinklerModeledEtMarksV1 = Vec<SprinklerModeledEtRowV1>;

/// Zone modeled evapotranspiration
/// Layers actual modeled-gap bars with annotations for zones whose represented
/// interval needs no retained fallback estimate.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(layer)]
pub struct SprinklerModeledEtChartV1 {
    /// Modeled ET gaps
    /// Exact retained fallback intervals and their source.
    pub gaps: SprinklerModeledEtMarksV1,
    /// Empty zones
    /// Configured zones with no modeled gap in the requested window.
    pub empty_zones: SprinklerFacetedEmptyZonesV1,
}

/// Temperature row
/// One observed or forecast air-temperature sample.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerTemperatureRowV1 {
    /// Time
    /// Provider period start.
    #[libertas_chart_channel(x, tooltip, key)]
    #[libertas_chart_scale(id = report_time, kind = utc)]
    pub at: LibertasDateTime,
    /// Temperature
    /// Air temperature in degrees Celsius.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(kind = linear, zero = false)]
    pub temperature_celsius: f32,
    /// Source
    /// Observed or forecast.
    #[libertas_chart_channel(color, detail, tooltip)]
    pub source: SprinklerWeatherChartSourceV1,
}

/// Temperature
/// Observed and forecast temperature without mixing its scale with ET or wind.
#[libertas_chart(line)]
pub type SprinklerTemperatureChartV1 = Vec<SprinklerTemperatureRowV1>;

/// Humidity row
/// One observed or forecast relative-humidity sample.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerHumidityRowV1 {
    /// Time
    /// Provider period start.
    #[libertas_chart_channel(x, tooltip, key)]
    #[libertas_chart_scale(id = report_time, kind = utc)]
    pub at: LibertasDateTime,
    /// Relative humidity
    /// Relative humidity percentage.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(kind = linear, min = 0, max = 100, zero = true)]
    pub relative_humidity_percent: u8,
    /// Source
    /// Observed or forecast.
    #[libertas_chart_channel(color, detail, tooltip)]
    pub source: SprinklerWeatherChartSourceV1,
}

/// Relative humidity
/// Observed and forecast humidity on its own zero-to-100-percent scale.
#[libertas_chart(line)]
pub type SprinklerHumidityChartV1 = Vec<SprinklerHumidityRowV1>;

/// Wind row
/// One sustained-wind or gust sample.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWindRowV1 {
    /// Time
    /// Provider period start.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = report_time, kind = utc)]
    pub at: LibertasDateTime,
    /// Wind speed
    /// Sustained wind or gust speed in meters per second.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(kind = linear, min = 0, zero = true)]
    pub meters_per_second: f32,
    /// Series
    /// Observed or forecast sustained wind or gust.
    #[libertas_chart_channel(color, detail, tooltip)]
    pub series: SprinklerWindSeriesV1,
    /// Stable key
    /// Server-generated identity for this time and wind series.
    #[libertas_chart_channel(key)]
    pub sample_key: String,
}

/// Wind and gusts
/// Observed and forecast sustained wind and gusts on one comparable scale.
#[libertas_chart(line)]
pub type SprinklerWindChartV1 = Vec<SprinklerWindRowV1>;

/// Weather and ET chart
/// Vertically aligns ET, temperature, humidity, and wind without mixing their
/// incompatible units or scales.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(vconcat)]
pub struct SprinklerWeatherEtChartV1 {
    /// Reference evapotranspiration
    /// Observed and forecast ET.
    pub reference_evapotranspiration: SprinklerEtChartV1,
    /// Modeled reference evapotranspiration
    /// Exact provider-history gaps, faceted across every configured zone.
    pub modeled_reference_evapotranspiration: SprinklerModeledEtChartV1,
    /// Temperature
    /// Observed and forecast air temperature.
    pub temperature: SprinklerTemperatureChartV1,
    /// Relative humidity
    /// Observed and forecast relative humidity.
    pub relative_humidity: SprinklerHumidityChartV1,
    /// Wind
    /// Observed and forecast sustained wind and gusts.
    pub wind: SprinklerWindChartV1,
}

/// Sprinkler report protocol
/// Exposes four independently requested all-zone charts. Every request can be
/// sent immediately with both time bounds null; the server then selects a
/// useful fixed default window. A client may later resend that chart's request
/// with one or both bounds to customize only its time window.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
// This public Avro/schema union must expose the chart composition directly;
// boxing the response fields would change their generated chart shape.
#[allow(clippy::large_enum_variant)]
pub enum SprinklerReportProtocolV1 {
    /// Get water balance
    /// Requests calculated available water and agronomic reference lines for
    /// every configured zone.
    #[libertas_request]
    #[libertas_next_response(WaterBalanceV1)]
    GetWaterBalanceV1 {
        /// Start time
        /// Optional inclusive UTC bound. Leave null for the server default.
        starts_at: Option<LibertasDateTime>,
        /// End time
        /// Optional exclusive UTC bound. Leave null for the server default.
        ends_before: Option<LibertasDateTime>,
    },
    /// Water balance
    /// Facets calculated available water and reference lines across every
    /// configured zone. Water inputs and decisions are available in the
    /// all-zone usage and timeline charts.
    #[libertas_response]
    #[libertas_next_request(GetWaterBalanceV1)]
    #[libertas_chart(line)]
    WaterBalanceV1(SprinklerWaterBalanceChartV1),
    /// Get watering timeline
    /// Requests scheduled and actual watering activity across every zone.
    #[libertas_request]
    #[libertas_next_response(WateringTimelineV1)]
    GetWateringTimelineV1 {
        /// Start time
        /// Optional inclusive UTC bound. Leave null for the server default.
        starts_at: Option<LibertasDateTime>,
        /// End time
        /// Optional exclusive UTC bound. Leave null for the server default.
        ends_before: Option<LibertasDateTime>,
    },
    /// Watering timeline
    /// Scheduled and actual controller activity across configured zones.
    #[libertas_response]
    #[libertas_next_request(GetWateringTimelineV1)]
    #[libertas_chart(layer)]
    WateringTimelineV1(SprinklerWateringTimelineChartV1),
    /// Get water usage
    /// Requests rain and observed irrigation accounting for every zone. The
    /// server selects day or week buckets from the represented duration.
    #[libertas_request]
    #[libertas_next_response(WaterUsageV1)]
    GetWaterUsageV1 {
        /// Start time
        /// Optional inclusive UTC bound. Leave null for the server default.
        starts_at: Option<LibertasDateTime>,
        /// End time
        /// Optional exclusive UTC bound. Leave null for the server default.
        ends_before: Option<LibertasDateTime>,
    },
    /// Water usage
    /// Rain and estimated irrigation by server-selected bucket and zone.
    #[libertas_response]
    #[libertas_next_request(GetWaterUsageV1)]
    #[libertas_chart(layer)]
    WaterUsageV1(SprinklerWaterUsageChartV1),
    /// Get weather and ET
    /// Requests shared site weather plus every zone's modeled ET gaps.
    #[libertas_request]
    #[libertas_next_response(WeatherEtV1)]
    GetWeatherEtV1 {
        /// Start time
        /// Optional inclusive UTC bound. Leave null for the server default.
        starts_at: Option<LibertasDateTime>,
        /// End time
        /// Optional exclusive UTC bound. Leave null for the server default.
        ends_before: Option<LibertasDateTime>,
    },
    /// Weather and ET
    /// Shared observed/forecast weather and per-zone modeled ET gaps.
    #[libertas_response]
    #[libertas_next_request(GetWeatherEtV1)]
    #[libertas_chart(vconcat)]
    WeatherEtV1(SprinklerWeatherEtChartV1),
}

/// Sprinkler water event
/// Stores one independently indexed weather period or observed irrigation
/// interval in the seven-day water history.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum SprinklerWaterEventV1 {
    /// Weather period
    /// Records provider precipitation and reference evapotranspiration for one
    /// completed historical period.
    WeatherV1 {
        /// Start time
        /// The inclusive start of the completed weather period.
        starts_at: LibertasDateTime,
        /// Duration
        /// The completed weather period length in seconds.
        #[libertas_time_interval]
        duration_seconds: u32,
        /// Precipitation
        /// Provider precipitation accumulated during the period, in
        /// millimeters.
        #[libertas_number(min = 0)]
        precipitation_millimeters: f32,
        /// Reference evapotranspiration
        /// Provider FAO-56 reference evapotranspiration accumulated during the
        /// period, in millimeters.
        #[libertas_number(min = 0)]
        reference_evapotranspiration_millimeters: f32,
    },
    /// Irrigation interval
    /// Records water inferred from actual Matter Valve open time, including
    /// manual openings, together with the zone's water amount adjuster during
    /// that observed interval.
    IrrigationV1 {
        /// Start time
        /// The inclusive start of the accounted valve-open interval.
        starts_at: LibertasDateTime,
        /// Duration
        /// The observed valve-open interval length in seconds.
        #[libertas_time_interval]
        duration_seconds: u32,
        /// Water amount adjuster
        /// The zone's configured watering percentage while this valve-open
        /// interval was observed. If the setting changes while the valve is
        /// open, the history uses separate adjacent intervals.
        #[libertas_number(min = 20, max = 200, step = 10)]
        watering_percent: u16,
        /// Applied water
        /// Estimated water depth calculated from observed open time and the
        /// configured sprinkler-head profile, in millimeters.
        #[libertas_number(min = 0)]
        applied_water_millimeters: f32,
    },
}

impl SprinklerWaterEventV1 {
    fn starts_at(&self) -> LibertasDateTime {
        match self {
            Self::WeatherV1 { starts_at, .. } | Self::IrrigationV1 { starts_at, .. } => *starts_at,
        }
    }

    fn duration_seconds(&self) -> u32 {
        match self {
            Self::WeatherV1 {
                duration_seconds, ..
            }
            | Self::IrrigationV1 {
                duration_seconds, ..
            } => *duration_seconds,
        }
    }

    fn ends_at(&self) -> Option<LibertasDateTime> {
        self.starts_at()
            .checked_add(u64::from(self.duration_seconds()))
    }
}

/// Sprinkler zone memory
/// Persists the compact restart-safe water amount adjuster, constraints, and
/// folded water-balance baseline for one configured valve. Water events are
/// stored separately as incremental indexed records.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerZoneMemoryV1 {
    /// Water amount adjuster
    /// Percentage of the adaptive watering amount to apply. The default is
    /// 100%.
    #[libertas_number(min = 20, max = 200, step = 10)]
    pub watering_percent: u16,
    /// Hold-off periods
    /// The normalized runtime scheduling constraints.
    /// ----
    /// Hold-off period
    /// A half-open interval during which the zone cannot water.
    #[libertas_size(max = 64)]
    pub hold_off_periods: Vec<SprinklerTimeSlotV1>,
    /// Balance baseline time
    /// The date and time through which older water inputs have been folded into
    /// `baseline_deficit_millimeters`.
    pub balance_baseline_at: LibertasDateTime,
    /// Baseline deficit
    /// Root-zone water deficit at `balance_baseline_at`, in millimeters.
    #[libertas_number(min = 0)]
    pub baseline_deficit_millimeters: f32,
}

/// Sprinkler persistent data
/// Defines every value written by the sprinkler application. Zone data is
/// stored under each Matter Valve; shared system data uses the weather endpoint.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum SprinklerDataV1 {
    /// Zone memory
    /// Stores one zone's runtime adjuster, constraints, and folded baseline.
    ZoneMemoryV1 {
        /// Zone memory
        /// The complete restart-safe state for the configured valve.
        memory: SprinklerZoneMemoryV1,
    },
    /// Water event
    /// Stores one independently indexed weather or irrigation event.
    WaterEventV1 {
        /// Water event
        /// One completed weather period or accounted valve-open interval.
        event: SprinklerWaterEventV1,
    },
    /// Site location
    /// Stores the last valid Hub location for offline seasonal estimation.
    SiteLocationV1 {
        /// Site location
        /// The cached WGS84 coordinates for the sprinkler site.
        location: SprinklerWeatherLocationV1,
    },
    /// Watering mode
    /// Stores the shared Active or Winterization mode for the sprinkler system.
    WateringModeV1 {
        /// Watering mode
        /// The restart-safe operating mode shared by every zone.
        mode: SprinklerWateringModeV1,
    },
    /// Winterization reminder
    /// Stores the latest system-wide reminder time and reason.
    WinterizationReminderV1 {
        /// Reminder memory
        /// Restart-safe throttling state for winterization notifications.
        memory: SprinklerWinterizationReminderMemoryV1,
    },
    /// Report weather period
    /// Archives one accepted completed provider period indefinitely for weather,
    /// ET, and decision-explanation charts.
    ReportWeatherPeriodV1 {
        /// Weather period
        /// Legacy rain and ET for one completed provider period. Existing rows
        /// remain usable for water balance, but do not invent unavailable wind.
        period: SprinklerWeatherHistoryPeriodV1,
    },
    /// Watering activity
    /// Archives one scheduled, running, completed, skipped, failed, superseded,
    /// manual, or legacy watering activity indefinitely.
    WateringActivityV1 {
        /// Activity
        /// Complete durable scheduled-versus-actual lifecycle data.
        activity: SprinklerWateringActivityV1,
    },
    /// Daily report
    /// Archives one UTC-day water-accounting and balance checkpoint
    /// indefinitely so old ranges remain queryable.
    DailyReportV1 {
        /// Daily report
        /// Rain, ET, estimated irrigation, and opening/closing modeled deficit.
        report: SprinklerDailyReportV1,
    },
    /// Watering activity state
    /// Stores one bounded restart pointer into the indefinite activity archive.
    WateringActivityStateV1 {
        /// Activity state
        /// Exact current-activity index, if a nonterminal activity exists.
        state: SprinklerWateringActivityStateV1,
    },
    /// Report weather observation
    /// Archives one accepted current-condition sample indefinitely. It keeps
    /// 15-minute decision evidence without double-counting rain or ET already
    /// represented by completed hourly history.
    ReportWeatherObservationV1 {
        /// Weather observation
        /// Temperature, humidity, wind, gusts, and immediate water inputs at
        /// the provider's represented time.
        observation: SprinklerCurrentWeatherV1,
    },
    /// Report weather archive state
    /// Stores the active site generation without deleting prior generations.
    ReportWeatherArchiveStateV1 {
        /// Archive state
        /// The active indefinitely retained weather generation.
        state: SprinklerReportWeatherArchiveStateV1,
    },
    /// Authoritative watering activity state
    /// Appended successor to `WateringActivityStateV1`. It retains terminal as
    /// well as nonterminal snapshots so either audit write can be repaired after
    /// a restart without changing any existing persistent-data discriminant.
    WateringActivityStateV2 {
        /// Activity state
        /// Latest complete activity snapshot and its explicit current status.
        state: SprinklerWateringActivityStateV2,
    },
    /// Modeled weather gap
    /// Archives one exact fallback-ET interval indefinitely. This variant is
    /// appended so every existing persistent-data discriminant remains stable.
    ModeledWeatherGapV1 {
        /// Modeled gap
        /// Exact interval with immutable source and rate provenance.
        gap: SprinklerModeledWeatherGapV1,
    },
    /// Report weather period V2
    /// Archives one full accepted provider observation indefinitely. This
    /// append-only variant leaves all existing persistent discriminants stable.
    ReportWeatherPeriodV2 {
        /// Weather period
        /// Temperature, humidity, rain, ET, wind, and gusts for one completed
        /// provider period.
        period: SprinklerWeatherHistoryPeriodV2,
    },
    /// Report weather archive state V2
    /// Appended successor whose generation is bound only by explicit provider
    /// site messages, without relying on an optional section-clear handshake.
    ReportWeatherArchiveStateV2 {
        /// Archive state
        /// Active indefinitely retained provider-site generation.
        state: SprinklerReportWeatherArchiveStateV2,
    },
}

/// Sprinkler zone
/// Configures the physical facts needed to calculate and execute watering for
/// one area. The water amount adjuster and hold-offs are runtime data.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerZoneV1 {
    /// Zone valve
    /// A Matter Irrigation System logical device exposing the Valve
    /// Configuration and Control server cluster for reads, subscriptions, and
    /// Open and Close commands.
    #[libertas_device_type("BQEBAUABgQED")]
    #[libertas_ui_header]
    #[libertas_unique]
    pub valve: LibertasDevice,
    /// Plant type
    /// The curated water-storage and weather-demand profile for this zone.
    #[libertas_default(Lawn)]
    pub plant_type: SprinklerPlantTypeV1,
    /// Sprinkler head type
    /// The delivery style used to estimate valve-open time. The water amount
    /// adjuster corrects the automatic amount without exposing flow-rate setup.
    #[libertas_default(RotorsLowRate)]
    pub sprinkler_head_type: SprinklerHeadTypeV1,
    /// State endpoint
    /// Exposes an essential regular-user state by default, offers complete
    /// advanced state on demand, groups end-user settings in one configuration
    /// view, and accepts independent adjuster, hold-off, and watering-mode
    /// actions.
    #[libertas_endpoint_schema(SprinklerZoneProtocolV1)]
    #[libertas_endpoint_server]
    #[libertas_endpoint_base_objects("^.valve")]
    #[libertas_unique]
    pub state_endpoint: LibertasEndpoint,
}

#[derive(Clone, Copy)]
struct PlantProfile {
    water_capacity_millimeters: f32,
    crop_coefficient: f32,
    foliage_wetness_sensitivity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValveCommandKind {
    Open,
    Close,
}

#[derive(Clone, Copy)]
struct PendingValveCommand {
    kind: ValveCommandKind,
    transaction_id: Option<u32>,
    sent_at_ticks: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExpectedIrrigation {
    starts_at: LibertasDateTime,
    activity_index: i64,
    activity_ordinal: u16,
}

fn restored_expected_irrigation(
    activity: &SprinklerWateringActivityV1,
) -> Option<ExpectedIrrigation> {
    if activity.origin != SprinklerWateringOriginV1::Automatic
        || !matches!(
            activity.outcome,
            SprinklerWateringOutcomeV1::CommandPending | SprinklerWateringOutcomeV1::Running
        )
    {
        return None;
    }
    Some(ExpectedIrrigation {
        starts_at: activity.scheduled_starts_at.or(activity.actual_starts_at)?,
        activity_index: activity.activity_index,
        activity_ordinal: activity.activity_ordinal,
    })
}

struct ZoneRuntime {
    configuration: SprinklerZoneV1,
    memory: SprinklerZoneMemoryV1,
    water_events: Vec<SprinklerWaterEventV1>,
    modeled_weather_gaps: Vec<SprinklerModeledWeatherGapV1>,
    active_state: SprinklerZoneActiveStateV1,
    valve_state_known: bool,
    valve_is_open: bool,
    valve_opened_automatically: bool,
    valve_fault_bitmap: u16,
    valve_last_report_ticks: Option<u64>,
    accounted_at_ticks: Option<u64>,
    accounted_at_utc: Option<LibertasDateTime>,
    pending_command: Option<PendingValveCommand>,
    expected_irrigation: Option<ExpectedIrrigation>,
    current_activity: Option<SprinklerWateringActivityV1>,
    finalized_daily_reports: Vec<SprinklerDailyReportV1>,
}

struct ControllerState {
    weather_endpoint: LibertasEndpoint,
    report_weather_archive_state: SprinklerReportWeatherArchiveStateV2,
    reminder_recipients: Vec<LibertasUser>,
    watering_mode: SprinklerWateringModeV1,
    winterization_reminder: Option<SprinklerWinterizationReminderMemoryV1>,
    site_location: Option<SprinklerWeatherLocationV1>,
    hub_location_server_up: bool,
    hub_location_subscription_ready: bool,
    site_location_retry_timer: u32,
    weather: SprinklerWeatherSnapshotV2,
    weather_cursor: Option<SprinklerWeatherCursorV1>,
    weather_stream_ready: bool,
    weather_server_up: bool,
    weather_maximum_wait_seconds: u32,
    weather_retry_timer: u32,
    valve_decision_not_before_ticks: u64,
    valve_decision_timer: u32,
    zones: Vec<ZoneRuntime>,
}

struct ZoneContext {
    shared: Rc<RefCell<ControllerState>>,
    zone_index: usize,
}

#[derive(Clone, Copy)]
enum ControllerAction {
    Open {
        zone_index: usize,
        duration_seconds: u32,
    },
    Close {
        zone_index: usize,
        reason: SprinklerWateringReasonV1,
    },
}

#[derive(Clone, Copy)]
enum ZoneResponseKind {
    State,
    AdvancedState,
    Configuration,
}

struct EvaluationOutcome {
    changed_zones: Vec<usize>,
    zone_memories_to_persist: Vec<(LibertasDevice, SprinklerZoneMemoryV1)>,
    activities_to_persist: Vec<(LibertasDevice, SprinklerWateringActivityV1)>,
    daily_reports_to_persist: Vec<(LibertasDevice, Vec<SprinklerDailyReportV1>)>,
    modeled_gap_changes: Vec<ModeledGapPersistenceChange>,
    action: Option<ControllerAction>,
}

struct ModeledGapPersistenceChange {
    valve: LibertasDevice,
    previous: Vec<SprinklerModeledWeatherGapV1>,
    current: Vec<SprinklerModeledWeatherGapV1>,
}

impl ModeledGapPersistenceChange {
    fn submit(self) {
        persist_modeled_gap_delta(self.valve, &self.previous, &self.current);
    }
}

struct ZonePersistenceChange {
    valve: LibertasDevice,
    previous_memory: SprinklerZoneMemoryV1,
    memory: SprinklerZoneMemoryV1,
    previous_events: Vec<SprinklerWaterEventV1>,
    water_events: Vec<SprinklerWaterEventV1>,
}

impl ZonePersistenceChange {
    fn submit(self) {
        persist_zone_runtime_change(
            self.valve,
            &self.previous_memory,
            &self.memory,
            &self.previous_events,
            &self.water_events,
        );
    }
}

fn zone_persistence_change(
    zone: &ZoneRuntime,
    previous_memory: SprinklerZoneMemoryV1,
    previous_events: Vec<SprinklerWaterEventV1>,
) -> ZonePersistenceChange {
    ZonePersistenceChange {
        valve: zone.configuration.valve,
        previous_memory,
        memory: zone.memory.clone(),
        previous_events,
        water_events: zone.water_events.clone(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum WinterizationReminderEvidence {
    LocationAndSeason,
    FreezingWeather { temperature_celsius: f32 },
}

impl WinterizationReminderEvidence {
    const fn reason(self) -> SprinklerWinterizationReminderReasonV1 {
        match self {
            Self::LocationAndSeason => SprinklerWinterizationReminderReasonV1::LocationAndSeason,
            Self::FreezingWeather { .. } => SprinklerWinterizationReminderReasonV1::FreezingWeather,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct WinterizationReminderAction {
    recipients: Vec<LibertasUser>,
    evidence: WinterizationReminderEvidence,
}

impl WinterizationReminderAction {
    fn submit(self) {
        let Self {
            recipients,
            evidence,
        } = self;
        match evidence {
            WinterizationReminderEvidence::LocationAndSeason => libertas_notification_send(
                &recipients,
                NotificationImportance::Info,
                None,
                WINTERIZATION_SEASON_NOTIFICATION_RESOURCE,
                &[],
            ),
            WinterizationReminderEvidence::FreezingWeather {
                temperature_celsius,
            } => {
                let arguments = [NotificationArgument::UnitFloat {
                    unit_type: "temperature-celsius",
                    value: temperature_celsius,
                }];
                libertas_notification_send(
                    &recipients,
                    NotificationImportance::AlertGuarded,
                    None,
                    WINTERIZATION_WEATHER_NOTIFICATION_RESOURCE,
                    &arguments,
                );
            }
        }
    }
}

fn utc_seconds() -> Option<LibertasDateTime> {
    libertas_get_utc_time()
        .map(|microseconds| microseconds / MICROSECONDS_PER_SECOND)
        .filter(|seconds| *seconds > 0)
}

fn absolute_interval_ticks(now_ticks: u64, interval_seconds: u32) -> u64 {
    now_ticks.saturating_add(u64::from(interval_seconds).saturating_mul(MICROSECONDS_PER_SECOND))
}

fn valve_decision_allowed(now_ticks: u64, not_before_ticks: u64) -> bool {
    now_ticks >= not_before_ticks
}

fn plant_profile(plant: SprinklerPlantTypeV1) -> PlantProfile {
    match plant {
        SprinklerPlantTypeV1::Lawn => PlantProfile {
            water_capacity_millimeters: 32.0,
            crop_coefficient: 0.80,
            foliage_wetness_sensitivity: 0.8,
        },
        SprinklerPlantTypeV1::Flowers => PlantProfile {
            water_capacity_millimeters: 48.0,
            crop_coefficient: 0.70,
            foliage_wetness_sensitivity: 1.0,
        },
        SprinklerPlantTypeV1::Vegetables => PlantProfile {
            water_capacity_millimeters: 72.0,
            crop_coefficient: 0.90,
            foliage_wetness_sensitivity: 1.0,
        },
        SprinklerPlantTypeV1::FruitTrees => PlantProfile {
            water_capacity_millimeters: 128.0,
            crop_coefficient: 0.75,
            foliage_wetness_sensitivity: 0.4,
        },
        SprinklerPlantTypeV1::Citrus => PlantProfile {
            water_capacity_millimeters: 120.0,
            crop_coefficient: 0.80,
            foliage_wetness_sensitivity: 0.4,
        },
        SprinklerPlantTypeV1::TreesAndBushes => PlantProfile {
            water_capacity_millimeters: 160.0,
            crop_coefficient: 0.60,
            foliage_wetness_sensitivity: 0.25,
        },
        SprinklerPlantTypeV1::Xeriscape => PlantProfile {
            water_capacity_millimeters: 80.0,
            crop_coefficient: 0.30,
            foliage_wetness_sensitivity: 0.1,
        },
    }
}

fn root_zone_capacity_millimeters(zone: &SprinklerZoneV1) -> f32 {
    plant_profile(zone.plant_type).water_capacity_millimeters
}

fn nominal_delivery_millimeters_per_hour(head: SprinklerHeadTypeV1) -> f32 {
    match head {
        SprinklerHeadTypeV1::SurfaceDrip => 8.0,
        SprinklerHeadTypeV1::Bubblers => 25.0,
        SprinklerHeadTypeV1::PopupSpray => 40.0,
        SprinklerHeadTypeV1::RotorsLowRate => 12.0,
        SprinklerHeadTypeV1::RotorsHighRate => 20.0,
    }
}

fn head_exposes_foliage(head: SprinklerHeadTypeV1) -> bool {
    matches!(
        head,
        SprinklerHeadTypeV1::PopupSpray
            | SprinklerHeadTypeV1::RotorsLowRate
            | SprinklerHeadTypeV1::RotorsHighRate
    )
}

fn head_wind_sensitivity(head: SprinklerHeadTypeV1) -> f32 {
    match head {
        SprinklerHeadTypeV1::SurfaceDrip => 0.05,
        SprinklerHeadTypeV1::Bubblers => 0.20,
        SprinklerHeadTypeV1::PopupSpray => 1.0,
        SprinklerHeadTypeV1::RotorsLowRate => 0.75,
        SprinklerHeadTypeV1::RotorsHighRate => 0.90,
    }
}

fn preferred_solar_elevation_range(head: SprinklerHeadTypeV1) -> (f64, f64) {
    match head {
        SprinklerHeadTypeV1::SurfaceDrip => (-12.0, 15.0),
        SprinklerHeadTypeV1::Bubblers => (-9.0, 12.0),
        SprinklerHeadTypeV1::PopupSpray
        | SprinklerHeadTypeV1::RotorsLowRate
        | SprinklerHeadTypeV1::RotorsHighRate => (
            OVERHEAD_MINIMUM_SOLAR_ELEVATION_DEGREES,
            OVERHEAD_MAXIMUM_SOLAR_ELEVATION_DEGREES,
        ),
    }
}

fn valid_watering_percent(value: u16) -> bool {
    (20..=200).contains(&value) && value.is_multiple_of(10)
}

fn valid_reminder_recipients(recipients: &[LibertasUser]) -> bool {
    !recipients.is_empty()
        && recipients.len() <= MAX_REMINDER_RECIPIENTS
        && recipients
            .iter()
            .enumerate()
            .all(|(index, recipient)| !recipients[..index].contains(recipient))
}

fn valid_zones(zones: &[SprinklerZoneV1]) -> bool {
    !zones.is_empty()
        && zones.len() <= MAX_SPRINKLER_ZONES
        && zones.iter().enumerate().all(|(index, zone)| {
            zones[..index].iter().all(|previous| {
                previous.valve != zone.valve && previous.state_endpoint != zone.state_endpoint
            })
        })
}

fn valid_report_endpoint(
    report_endpoint: LibertasEndpoint,
    weather_endpoint: LibertasEndpoint,
    zones: &[SprinklerZoneV1],
) -> bool {
    report_endpoint != weather_endpoint
        && zones
            .iter()
            .all(|zone| zone.state_endpoint != report_endpoint)
}

fn valid_nonnegative(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}

fn valid_slot(slot: SprinklerTimeSlotV1) -> bool {
    slot.duration_seconds > 0 && slot.ends_at().is_some()
}

fn normalize_hold_offs(
    mut hold_offs: Vec<SprinklerTimeSlotV1>,
) -> Result<Vec<SprinklerTimeSlotV1>, ()> {
    if hold_offs.len() > MAX_HOLD_OFFS || !hold_offs.iter().copied().all(valid_slot) {
        return Err(());
    }
    hold_offs.sort_by_key(|slot| slot.starts_at);
    let mut normalized: Vec<SprinklerTimeSlotV1> = Vec::with_capacity(hold_offs.len());
    for slot in hold_offs {
        let Some(last) = normalized.last_mut() else {
            normalized.push(slot);
            continue;
        };
        let last_end = last.ends_at().ok_or(())?;
        if slot.starts_at <= last_end {
            let merged_end = last_end.max(slot.ends_at().ok_or(())?);
            last.duration_seconds =
                u32::try_from(merged_end.saturating_sub(last.starts_at)).map_err(|_| ())?;
        } else {
            normalized.push(slot);
        }
    }
    Ok(normalized)
}

fn prune_expired_hold_offs(memory: &mut SprinklerZoneMemoryV1, now: LibertasDateTime) -> bool {
    let previous_len = memory.hold_off_periods.len();
    memory
        .hold_off_periods
        .retain(|hold_off| hold_off.ends_at().is_some_and(|ends_at| ends_at > now));
    memory.hold_off_periods.len() != previous_len
}

fn water_event_index(event: &SprinklerWaterEventV1) -> Option<i64> {
    let kind = match event {
        SprinklerWaterEventV1::WeatherV1 { .. } => 0,
        SprinklerWaterEventV1::IrrigationV1 { .. } => 1,
    };
    i64::try_from(event.starts_at())
        .ok()?
        .checked_mul(WATER_EVENT_INDEX_KIND_COUNT)?
        .checked_add(kind)
}

fn valid_water_event(event: &SprinklerWaterEventV1) -> bool {
    if event.duration_seconds() == 0 || event.ends_at().is_none() {
        return false;
    }
    let amounts_are_valid = match event {
        SprinklerWaterEventV1::WeatherV1 {
            precipitation_millimeters,
            reference_evapotranspiration_millimeters,
            ..
        } => {
            valid_nonnegative(*precipitation_millimeters)
                && valid_nonnegative(*reference_evapotranspiration_millimeters)
        }
        SprinklerWaterEventV1::IrrigationV1 {
            applied_water_millimeters,
            watering_percent,
            ..
        } => {
            valid_nonnegative(*applied_water_millimeters)
                && valid_watering_percent(*watering_percent)
        }
    };
    amounts_are_valid && water_event_index(event).is_some()
}

fn valid_memory(memory: &SprinklerZoneMemoryV1) -> bool {
    valid_watering_percent(memory.watering_percent)
        && valid_nonnegative(memory.baseline_deficit_millimeters)
        && memory.hold_off_periods.len() <= MAX_HOLD_OFFS
        && memory.hold_off_periods.iter().copied().all(valid_slot)
}

#[derive(Clone, Copy)]
struct WaterDemandEstimate {
    source: SprinklerWaterDemandSourceV1,
    reference_evapotranspiration_millimeters_per_day: f32,
}

fn recent_reference_evapotranspiration_millimeters_per_day(
    water_events: &[SprinklerWaterEventV1],
) -> Option<f32> {
    let mut duration_seconds = 0_u64;
    let mut reference_evapotranspiration_millimeters = 0.0_f32;
    for event in water_events {
        if let SprinklerWaterEventV1::WeatherV1 {
            duration_seconds: duration,
            reference_evapotranspiration_millimeters: amount,
            ..
        } = event
        {
            duration_seconds = duration_seconds.saturating_add(u64::from(*duration));
            reference_evapotranspiration_millimeters += amount;
        }
    }
    if duration_seconds < MIN_RECENT_WEATHER_COVERAGE_SECONDS
        || !reference_evapotranspiration_millimeters.is_finite()
    {
        return None;
    }
    let rate =
        reference_evapotranspiration_millimeters * SECONDS_PER_DAY as f32 / duration_seconds as f32;
    rate.is_finite().then_some(rate.clamp(
        MIN_REFERENCE_ET_MILLIMETERS_PER_DAY,
        MAX_REFERENCE_ET_MILLIMETERS_PER_DAY,
    ))
}

fn utc_day_of_year(now: LibertasDateTime) -> u16 {
    let days_since_epoch = i64::try_from(now / SECONDS_PER_DAY).unwrap_or(i64::MAX / 2);
    let shifted = days_since_epoch + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let march_day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * march_day_of_year + 2) / 153;
    let day = march_day_of_year - (153 * march_month + 2) / 5 + 1;
    let month = march_month + if march_month < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_offsets = [0_i64, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let month_index = usize::try_from(month.saturating_sub(1))
        .unwrap_or(0)
        .min(11);
    let leap_offset = i64::from(leap_year && month > 2);
    u16::try_from(month_offsets[month_index] + day + leap_offset).unwrap_or(1)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SolarPosition {
    elevation_degrees: f64,
    rising: bool,
}

fn solar_position(
    location: SprinklerWeatherLocationV1,
    at: LibertasDateTime,
) -> Option<SolarPosition> {
    if !valid_site_location(location) || at == 0 {
        return None;
    }

    // NOAA's fractional-year approximation is accurate enough for selecting a
    // 15-minute irrigation window. UTC and longitude produce apparent solar
    // time directly, so no civil timezone or daylight-saving rule is needed.
    let day = f64::from(utc_day_of_year(at));
    let utc_hour = (at % SECONDS_PER_DAY) as f64 / 3_600.0;
    let gamma = 2.0 * core::f64::consts::PI / 365.0 * (day - 1.0 + (utc_hour - 12.0) / 24.0);
    let equation_of_time_minutes = 229.18
        * (0.000_075 + 0.001_868 * cos(gamma)
            - 0.032_077 * sin(gamma)
            - 0.014_615 * cos(2.0 * gamma)
            - 0.040_849 * sin(2.0 * gamma));
    let declination = 0.006_918 - 0.399_912 * cos(gamma) + 0.070_257 * sin(gamma)
        - 0.006_758 * cos(2.0 * gamma)
        + 0.000_907 * sin(2.0 * gamma)
        - 0.002_697 * cos(3.0 * gamma)
        + 0.001_480 * sin(3.0 * gamma);
    let utc_minutes = (at % SECONDS_PER_DAY) as f64 / 60.0;
    let unwrapped_solar_minutes =
        utc_minutes + equation_of_time_minutes + 4.0 * location.longitude_degrees;
    let true_solar_minutes =
        unwrapped_solar_minutes - floor(unwrapped_solar_minutes / 1_440.0) * 1_440.0;
    let hour_angle_degrees = true_solar_minutes / 4.0 - 180.0;
    let hour_angle = hour_angle_degrees * DEGREES_TO_RADIANS;
    let latitude = location.latitude_degrees * DEGREES_TO_RADIANS;
    let sine_elevation = (sin(latitude) * sin(declination)
        + cos(latitude) * cos(declination) * cos(hour_angle))
    .clamp(-1.0, 1.0);
    Some(SolarPosition {
        elevation_degrees: asin(sine_elevation) * RADIANS_TO_DEGREES,
        rising: hour_angle_degrees <= 0.0,
    })
}

fn location_is_in_winterization_season(
    location: SprinklerWeatherLocationV1,
    now: LibertasDateTime,
) -> bool {
    if !valid_site_location(location)
        || location.latitude_degrees.abs() < WINTERIZATION_REMINDER_LATITUDE_CUTOFF_DEGREES
    {
        return false;
    }

    let day = utc_day_of_year(now);
    let absolute_latitude = location.latitude_degrees.abs();
    if location.latitude_degrees >= 0.0 {
        let season_start = if absolute_latitude >= 55.0 {
            244 // September 1
        } else if absolute_latitude >= 45.0 {
            274 // October 1
        } else {
            305 // November 1
        };
        day >= season_start || day <= NORTHERN_WINTERIZATION_SEASON_END_DAY
    } else {
        let season_start = if absolute_latitude >= 55.0 {
            60 // March 1
        } else if absolute_latitude >= 45.0 {
            91 // April 1
        } else {
            121 // May 1
        };
        day >= season_start && day <= SOUTHERN_WINTERIZATION_SEASON_END_DAY
    }
}

fn freezing_weather_temperature(
    weather: &SprinklerWeatherSnapshotV2,
    now: LibertasDateTime,
) -> Option<f32> {
    let mut temperature = weather
        .current
        .as_ref()
        .filter(|current| current.is_fresh_at(now))
        .map(|current| current.temperature_celsius)
        .filter(|value| value.is_finite() && *value <= SAFE_MINIMUM_TEMPERATURE_CELSIUS);

    let forecast_end = now.saturating_add(u64::from(SPRINKLER_FORECAST_HORIZON_SECONDS));
    if let Some(forecast) = weather
        .forecast
        .as_ref()
        .filter(|forecast| forecast.is_fresh_at(now))
    {
        for period in &forecast.periods {
            let period_ends_at = period
                .starts_at
                .checked_add(u64::from(period.duration_seconds));
            if period.duration_seconds == 0
                || period_ends_at.is_none_or(|ends_at| ends_at <= now)
                || period.starts_at >= forecast_end
                || !period.temperature_celsius.is_finite()
                || period.temperature_celsius > SAFE_MINIMUM_TEMPERATURE_CELSIUS
            {
                continue;
            }
            temperature = Some(temperature.map_or(period.temperature_celsius, |current| {
                current.min(period.temperature_celsius)
            }));
        }
    }
    temperature
}

fn winterization_reminder_evidence(
    watering_mode: SprinklerWateringModeV1,
    weather: &SprinklerWeatherSnapshotV2,
    location: Option<SprinklerWeatherLocationV1>,
    now: LibertasDateTime,
) -> Option<WinterizationReminderEvidence> {
    if watering_mode == SprinklerWateringModeV1::Winterization || now == 0 {
        return None;
    }
    if let Some(temperature_celsius) = freezing_weather_temperature(weather, now) {
        return Some(WinterizationReminderEvidence::FreezingWeather {
            temperature_celsius,
        });
    }
    location
        .filter(|location| location_is_in_winterization_season(*location, now))
        .map(|_| WinterizationReminderEvidence::LocationAndSeason)
}

fn winterization_reminder_is_due(
    previous: Option<SprinklerWinterizationReminderMemoryV1>,
    evidence: WinterizationReminderEvidence,
    now: LibertasDateTime,
) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    if evidence.reason() == SprinklerWinterizationReminderReasonV1::FreezingWeather
        && previous.reason == SprinklerWinterizationReminderReasonV1::LocationAndSeason
    {
        return true;
    }
    now < previous.last_reminded_at
        || now.saturating_sub(previous.last_reminded_at) >= WINTERIZATION_REMINDER_INTERVAL_SECONDS
}

fn location_reference_evapotranspiration_millimeters_per_day(
    location: SprinklerWeatherLocationV1,
    now: LibertasDateTime,
) -> Option<f32> {
    if !valid_site_location(location) {
        return None;
    }
    let latitude = location.latitude_degrees as f32;
    let absolute_latitude = latitude.abs();
    let peak_day = if latitude >= 0.0 { 172_i32 } else { 355_i32 };
    let day = i32::from(utc_day_of_year(now));
    let direct_distance = (day - peak_day).unsigned_abs().min(365);
    let seasonal_distance = direct_distance.min(365 - direct_distance) as f32;
    let seasonal_position = 1.0 - 2.0 * seasonal_distance / 182.5;
    let annual_mean = 4.5 - 1.5 * (absolute_latitude / 90.0);
    let seasonal_amplitude = 2.5 * (absolute_latitude / 60.0).min(1.0);
    Some(
        (annual_mean + seasonal_amplitude * seasonal_position).clamp(
            MIN_REFERENCE_ET_MILLIMETERS_PER_DAY,
            MAX_REFERENCE_ET_MILLIMETERS_PER_DAY,
        ),
    )
}

fn water_demand_estimate(
    water_events: &[SprinklerWaterEventV1],
    location: Option<SprinklerWeatherLocationV1>,
    now: LibertasDateTime,
) -> WaterDemandEstimate {
    if let Some(rate) = recent_reference_evapotranspiration_millimeters_per_day(water_events) {
        return WaterDemandEstimate {
            source: SprinklerWaterDemandSourceV1::RecentLocalWeather,
            reference_evapotranspiration_millimeters_per_day: rate,
        };
    }
    if let Some(rate) = location.and_then(|location| {
        location_reference_evapotranspiration_millimeters_per_day(location, now)
    }) {
        return WaterDemandEstimate {
            source: SprinklerWaterDemandSourceV1::LocationAndSeason,
            reference_evapotranspiration_millimeters_per_day: rate,
        };
    }
    WaterDemandEstimate {
        source: SprinklerWaterDemandSourceV1::ConservativeDefault,
        reference_evapotranspiration_millimeters_per_day:
            CONSERVATIVE_REFERENCE_ET_MILLIMETERS_PER_DAY,
    }
}

fn default_memory(now: LibertasDateTime) -> SprinklerZoneMemoryV1 {
    SprinklerZoneMemoryV1 {
        watering_percent: 100,
        hold_off_periods: Vec::new(),
        // Prior irrigation is unknown when a zone is first configured. Start
        // fully replenished now instead of manufacturing a dry seven-day gap.
        balance_baseline_at: now,
        baseline_deficit_millimeters: 0.0,
    }
}

fn zone_key(valve: LibertasDevice) -> [NotificationArgument<'static>; 1] {
    [NotificationArgument::Object(valve)]
}

fn activity_day_key(
    valve: LibertasDevice,
    day: LibertasDateTime,
) -> [NotificationArgument<'static>; 2] {
    [
        NotificationArgument::Object(valve),
        NotificationArgument::Unsigned(day),
    ]
}

fn system_key(weather_endpoint: LibertasEndpoint) -> [NotificationArgument<'static>; 1] {
    [NotificationArgument::Object(weather_endpoint)]
}

fn report_weather_archive_key(
    weather_endpoint: LibertasEndpoint,
    generation: u64,
) -> [NotificationArgument<'static>; 2] {
    [
        NotificationArgument::Object(weather_endpoint),
        NotificationArgument::Unsigned(generation),
    ]
}

fn load_report_weather_archive_state(
    weather_endpoint: LibertasEndpoint,
) -> SprinklerReportWeatherArchiveStateV2 {
    match libertas_data_read_single(
        REPORT_WEATHER_ARCHIVE_STATE_RESOURCE,
        &system_key(weather_endpoint),
    ) {
        Some(SprinklerDataV1::ReportWeatherArchiveStateV2 { state }) => state,
        Some(SprinklerDataV1::ReportWeatherArchiveStateV1 { state }) => {
            let migrated = migrate_report_weather_archive_state(state);
            persist_report_weather_archive_state(weather_endpoint, migrated);
            migrated
        }
        _ => {
            let state = SprinklerReportWeatherArchiveStateV2 {
                generation: 0,
                location: None,
            };
            libertas_data_write_single(
                REPORT_WEATHER_ARCHIVE_STATE_RESOURCE,
                &system_key(weather_endpoint),
                &SprinklerDataV1::ReportWeatherArchiveStateV2 { state },
            );
            state
        }
    }
}

fn migrate_report_weather_archive_state(
    legacy: SprinklerReportWeatherArchiveStateV1,
) -> SprinklerReportWeatherArchiveStateV2 {
    SprinklerReportWeatherArchiveStateV2 {
        generation: legacy.generation,
        // A pending V1 history-clear handshake means the Hub opened the
        // generation but no provider SiteReplace was observed. Preserve that
        // reserved generation while leaving it unbound until an explicit
        // provider-site message arrives.
        location: (!legacy.awaiting_history_clear)
            .then_some(legacy.location)
            .flatten()
            .filter(|location| valid_site_location(*location)),
    }
}

fn persist_report_weather_archive_state(
    weather_endpoint: LibertasEndpoint,
    state: SprinklerReportWeatherArchiveStateV2,
) {
    libertas_data_write_single(
        REPORT_WEATHER_ARCHIVE_STATE_RESOURCE,
        &system_key(weather_endpoint),
        &SprinklerDataV1::ReportWeatherArchiveStateV2 { state },
    );
}

fn valid_site_location(location: SprinklerWeatherLocationV1) -> bool {
    location.latitude_degrees.is_finite()
        && (-90.0..=90.0).contains(&location.latitude_degrees)
        && location.longitude_degrees.is_finite()
        && (-180.0..=180.0).contains(&location.longitude_degrees)
}

fn same_weather_location(
    left: SprinklerWeatherLocationV1,
    right: SprinklerWeatherLocationV1,
) -> bool {
    (left.latitude_degrees - right.latitude_degrees).abs() <= LOCATION_EQUALITY_TOLERANCE_DEGREES
        && (left.longitude_degrees - right.longitude_degrees).abs()
            <= LOCATION_EQUALITY_TOLERANCE_DEGREES
}

fn persist_site_location(weather_endpoint: LibertasEndpoint, location: SprinklerWeatherLocationV1) {
    libertas_data_write_single(
        SITE_LOCATION_RESOURCE,
        &system_key(weather_endpoint),
        &SprinklerDataV1::SiteLocationV1 { location },
    );
}

fn load_site_location(weather_endpoint: LibertasEndpoint) -> Option<SprinklerWeatherLocationV1> {
    match libertas_data_read_single(SITE_LOCATION_RESOURCE, &system_key(weather_endpoint)) {
        Some(SprinklerDataV1::SiteLocationV1 { location }) if valid_site_location(location) => {
            Some(location)
        }
        _ => None,
    }
}

fn persist_watering_mode(weather_endpoint: LibertasEndpoint, mode: SprinklerWateringModeV1) {
    libertas_data_write_single(
        WATERING_MODE_RESOURCE,
        &system_key(weather_endpoint),
        &SprinklerDataV1::WateringModeV1 { mode },
    );
}

fn load_watering_mode(weather_endpoint: LibertasEndpoint) -> SprinklerWateringModeV1 {
    match libertas_data_read_single(WATERING_MODE_RESOURCE, &system_key(weather_endpoint)) {
        Some(SprinklerDataV1::WateringModeV1 { mode }) => mode,
        _ => {
            let mode = SprinklerWateringModeV1::Active;
            persist_watering_mode(weather_endpoint, mode);
            mode
        }
    }
}

fn persist_winterization_reminder(
    weather_endpoint: LibertasEndpoint,
    memory: SprinklerWinterizationReminderMemoryV1,
) {
    libertas_data_write_single(
        WINTERIZATION_REMINDER_RESOURCE,
        &system_key(weather_endpoint),
        &SprinklerDataV1::WinterizationReminderV1 { memory },
    );
}

fn load_winterization_reminder(
    weather_endpoint: LibertasEndpoint,
) -> Option<SprinklerWinterizationReminderMemoryV1> {
    match libertas_data_read_single(
        WINTERIZATION_REMINDER_RESOURCE,
        &system_key(weather_endpoint),
    ) {
        Some(SprinklerDataV1::WinterizationReminderV1 { memory })
            if memory.last_reminded_at > 0 =>
        {
            Some(memory)
        }
        _ => None,
    }
}

fn persist_zone_memory(valve: LibertasDevice, memory: &SprinklerZoneMemoryV1) {
    libertas_data_write_single(
        ZONE_DATA_RESOURCE,
        &zone_key(valve),
        &SprinklerDataV1::ZoneMemoryV1 {
            memory: memory.clone(),
        },
    );
}

fn load_zone_memory(valve: LibertasDevice, now: LibertasDateTime) -> SprinklerZoneMemoryV1 {
    match libertas_data_read_single(ZONE_DATA_RESOURCE, &zone_key(valve)) {
        Some(SprinklerDataV1::ZoneMemoryV1 { memory }) if valid_memory(&memory) => {
            let Ok(hold_off_periods) = normalize_hold_offs(memory.hold_off_periods.clone()) else {
                let memory = default_memory(now);
                persist_zone_memory(valve, &memory);
                return memory;
            };
            SprinklerZoneMemoryV1 {
                hold_off_periods,
                ..memory
            }
        }
        _ => {
            let memory = default_memory(now);
            persist_zone_memory(valve, &memory);
            memory
        }
    }
}

fn indexed_water_event_is_current(
    record: &IndexedData<SprinklerDataV1>,
    balance_baseline_at: LibertasDateTime,
) -> bool {
    matches!(
        &record.data,
        SprinklerDataV1::WaterEventV1 { event }
            if water_event_index(event) == Some(record.index)
                && valid_water_event(event)
                && event
                    .ends_at()
                    .is_some_and(|ends_at| ends_at > balance_baseline_at)
    )
}

fn reconstruct_water_events(
    records: &[IndexedData<SprinklerDataV1>],
    balance_baseline_at: LibertasDateTime,
) -> Vec<SprinklerWaterEventV1> {
    let mut events: Vec<_> = records
        .iter()
        .filter_map(|record| {
            if !indexed_water_event_is_current(record, balance_baseline_at) {
                return None;
            }
            match &record.data {
                SprinklerDataV1::WaterEventV1 { event } => Some(event.clone()),
                _ => None,
            }
        })
        .collect();
    sort_water_events(&mut events);
    events.dedup_by(|left, right| water_event_index(left) == water_event_index(right));
    events
}

fn load_water_events(
    valve: LibertasDevice,
    memory: &SprinklerZoneMemoryV1,
) -> Vec<SprinklerWaterEventV1> {
    let database = libertas_data_open_indexed(WATER_EVENTS_RESOURCE, &zone_key(valve));
    if database.count == 0 {
        return Vec::new();
    }
    let mut records = Vec::new();
    libertas_data_read_indexed_range::<SprinklerDataV1>(
        database.handle,
        database.max_index,
        IndexDirection::Below,
        MAX_WATER_EVENT_RECORDS_SCANNED,
        &mut records,
    );
    let events = reconstruct_water_events(&records, memory.balance_baseline_at);
    for record in &records {
        if !indexed_water_event_is_current(record, memory.balance_baseline_at) {
            libertas_data_remove_indexed_records(database.handle, record.index, record.index);
        }
    }
    events
}

fn valid_report_range(range: SprinklerReportTimeRangeV1) -> bool {
    range.starts_at < range.ends_before
        && range.ends_before.saturating_sub(range.starts_at) <= MAX_REPORT_RANGE_SECONDS
        && i64::try_from(range.starts_at).is_ok()
        && i64::try_from(range.ends_before).is_ok()
}

fn report_weather_period_index(period: &SprinklerWeatherHistoryPeriodV2) -> Option<i64> {
    i64::try_from(period.starts_at).ok()
}

fn valid_report_weather_period(period: &SprinklerWeatherHistoryPeriodV2) -> bool {
    period.duration_seconds > 0
        && period
            .starts_at
            .checked_add(u64::from(period.duration_seconds))
            .is_some()
        && period.temperature_celsius.is_finite()
        && period.relative_humidity_percent <= 100
        && valid_nonnegative(period.precipitation_millimeters)
        && valid_nonnegative(period.reference_evapotranspiration_millimeters)
        && valid_nonnegative(period.wind_speed_meters_per_second)
        && valid_nonnegative(period.wind_gust_meters_per_second)
        && report_weather_period_index(period).is_some()
}

fn persist_report_weather_periods(
    weather_endpoint: LibertasEndpoint,
    generation: u64,
    periods: &[SprinklerWeatherHistoryPeriodV2],
) {
    if periods.is_empty() {
        return;
    }
    let database = libertas_data_open_indexed(
        REPORT_WEATHER_HISTORY_V2_RESOURCE,
        &report_weather_archive_key(weather_endpoint, generation),
    );
    for period in periods {
        let Some(index) = report_weather_period_index(period) else {
            continue;
        };
        if valid_report_weather_period(period) {
            libertas_data_write_indexed(
                database.handle,
                index,
                &SprinklerDataV1::ReportWeatherPeriodV2 { period: *period },
            );
        }
    }
}

fn valid_legacy_report_weather_period(period: &SprinklerWeatherHistoryPeriodV1) -> bool {
    period.duration_seconds > 0
        && period
            .starts_at
            .checked_add(u64::from(period.duration_seconds))
            .is_some()
        && valid_nonnegative(period.precipitation_millimeters)
        && valid_nonnegative(period.reference_evapotranspiration_millimeters)
        && i64::try_from(period.starts_at).is_ok()
}

fn persist_legacy_report_weather_periods(
    weather_endpoint: LibertasEndpoint,
    generation: u64,
    periods: &[SprinklerWeatherHistoryPeriodV1],
) {
    if periods.is_empty() {
        return;
    }
    let database = libertas_data_open_indexed(
        REPORT_WEATHER_HISTORY_RESOURCE,
        &report_weather_archive_key(weather_endpoint, generation),
    );
    for period in periods {
        let Ok(index) = i64::try_from(period.starts_at) else {
            continue;
        };
        if valid_legacy_report_weather_period(period) {
            libertas_data_write_indexed(
                database.handle,
                index,
                &SprinklerDataV1::ReportWeatherPeriodV1 { period: *period },
            );
        }
    }
}

fn report_weather_replacement_indexes(
    periods: &[SprinklerWeatherHistoryPeriodV2],
) -> Option<Vec<i64>> {
    if periods.is_empty() || periods.len() > MAX_REPORT_WEATHER_PERIODS {
        return None;
    }
    let indexes: Vec<_> = periods
        .iter()
        .map(|period| {
            if !valid_report_weather_period(period) {
                return None;
            }
            report_weather_period_index(period)
        })
        .collect::<Option<_>>()?;
    indexes
        .windows(2)
        .all(|pair| pair[0] < pair[1])
        .then_some(indexes)
}

fn legacy_report_weather_replacement_indexes(
    periods: &[SprinklerWeatherHistoryPeriodV1],
) -> Option<Vec<i64>> {
    if periods.is_empty() || periods.len() > MAX_REPORT_WEATHER_PERIODS {
        return None;
    }
    let indexes: Vec<_> = periods
        .iter()
        .map(|period| {
            if !valid_legacy_report_weather_period(period) {
                return None;
            }
            i64::try_from(period.starts_at).ok()
        })
        .collect::<Option<_>>()?;
    indexes
        .windows(2)
        .all(|pair| pair[0] < pair[1])
        .then_some(indexes)
}

fn stale_report_weather_period_indexes(
    existing_indexes: &[i64],
    replacement_indexes: &[i64],
    scan_complete: bool,
) -> Vec<i64> {
    if !scan_complete {
        return Vec::new();
    }
    let (Some(first_index), Some(last_index)) = (
        replacement_indexes.first().copied(),
        replacement_indexes.last().copied(),
    ) else {
        return Vec::new();
    };
    existing_indexes
        .iter()
        .copied()
        .filter(|index| {
            *index >= first_index
                && *index <= last_index
                && replacement_indexes.binary_search(index).is_err()
        })
        .collect()
}

fn replace_report_weather_period_span(
    weather_endpoint: LibertasEndpoint,
    generation: u64,
    periods: &[SprinklerWeatherHistoryPeriodV2],
) {
    let Some(replacement_indexes) = report_weather_replacement_indexes(periods) else {
        return;
    };
    let first_index = replacement_indexes[0];
    let last_index = replacement_indexes[replacement_indexes.len() - 1];
    let database = libertas_data_open_indexed(
        REPORT_WEATHER_HISTORY_V2_RESOURCE,
        &report_weather_archive_key(weather_endpoint, generation),
    );
    let mut existing_records = Vec::new();
    if database.count > 0 {
        libertas_data_read_indexed_range::<SprinklerDataV1>(
            database.handle,
            first_index,
            IndexDirection::Above,
            MAX_REPORT_WEATHER_REPLACEMENT_RECORDS_SCANNED,
            &mut existing_records,
        );
    }
    let scan_complete = existing_records.len() < MAX_REPORT_WEATHER_REPLACEMENT_RECORDS_SCANNED
        || existing_records
            .last()
            .is_some_and(|record| record.index > last_index);
    let existing_indexes: Vec<_> = existing_records
        .iter()
        .take_while(|record| record.index <= last_index)
        .map(|record| record.index)
        .collect();
    let stale_indexes =
        stale_report_weather_period_indexes(&existing_indexes, &replacement_indexes, scan_complete);

    // Submit every authoritative replacement first. Only after all replacement
    // rows are durable candidates do we delete old keys absent from that span,
    // so a stop between submissions can leave stale rows but never a hole.
    for (period, index) in periods.iter().zip(replacement_indexes) {
        libertas_data_write_indexed(
            database.handle,
            index,
            &SprinklerDataV1::ReportWeatherPeriodV2 { period: *period },
        );
    }
    for index in stale_indexes {
        libertas_data_remove_indexed_records(database.handle, index, index);
    }
}

fn replace_legacy_report_weather_period_span(
    weather_endpoint: LibertasEndpoint,
    generation: u64,
    periods: &[SprinklerWeatherHistoryPeriodV1],
) {
    let Some(replacement_indexes) = legacy_report_weather_replacement_indexes(periods) else {
        return;
    };
    let first_index = replacement_indexes[0];
    let last_index = replacement_indexes[replacement_indexes.len() - 1];
    let database = libertas_data_open_indexed(
        REPORT_WEATHER_HISTORY_RESOURCE,
        &report_weather_archive_key(weather_endpoint, generation),
    );
    let mut existing_records = Vec::new();
    if database.count > 0 {
        libertas_data_read_indexed_range::<SprinklerDataV1>(
            database.handle,
            first_index,
            IndexDirection::Above,
            MAX_REPORT_WEATHER_REPLACEMENT_RECORDS_SCANNED,
            &mut existing_records,
        );
    }
    let scan_complete = existing_records.len() < MAX_REPORT_WEATHER_REPLACEMENT_RECORDS_SCANNED
        || existing_records
            .last()
            .is_some_and(|record| record.index > last_index);
    let existing_indexes: Vec<_> = existing_records
        .iter()
        .take_while(|record| record.index <= last_index)
        .map(|record| record.index)
        .collect();
    let stale_indexes =
        stale_report_weather_period_indexes(&existing_indexes, &replacement_indexes, scan_complete);

    for (period, index) in periods.iter().zip(replacement_indexes) {
        libertas_data_write_indexed(
            database.handle,
            index,
            &SprinklerDataV1::ReportWeatherPeriodV1 { period: *period },
        );
    }
    for index in stale_indexes {
        libertas_data_remove_indexed_records(database.handle, index, index);
    }
}

fn valid_report_weather_observation(observation: &SprinklerCurrentWeatherV1) -> bool {
    observation.interval_seconds > 0
        && observation.valid_until > observation.retrieved_at
        && observation.valid_at > 0
        && observation.temperature_celsius.is_finite()
        && observation.relative_humidity_percent <= 100
        && valid_nonnegative(observation.precipitation_millimeters)
        && valid_nonnegative(observation.reference_evapotranspiration_millimeters)
        && valid_nonnegative(observation.wind_speed_meters_per_second)
        && valid_nonnegative(observation.wind_gust_meters_per_second)
        && i64::try_from(observation.valid_at).is_ok()
}

fn valid_weather_history_metadata(
    retrieved_at: LibertasDateTime,
    valid_until: LibertasDateTime,
) -> bool {
    retrieved_at > 0 && valid_until > retrieved_at
}

fn valid_legacy_weather_history_periods(
    retrieved_at: LibertasDateTime,
    periods: &[SprinklerWeatherHistoryPeriodV1],
) -> bool {
    periods.len() <= MAX_REPORT_WEATHER_PERIODS
        && periods.iter().all(|period| {
            let Some(ends_at) = period
                .starts_at
                .checked_add(u64::from(period.duration_seconds))
            else {
                return false;
            };
            valid_legacy_report_weather_period(period)
                && ends_at <= retrieved_at
                && ends_at
                    > retrieved_at.saturating_sub(u64::from(SPRINKLER_HISTORY_WINDOW_SECONDS))
        })
        && periods.windows(2).all(|pair| {
            pair[0]
                .starts_at
                .checked_add(u64::from(pair[0].duration_seconds))
                .is_some_and(|ends_at| ends_at <= pair[1].starts_at)
        })
}

fn valid_weather_history_periods(
    retrieved_at: LibertasDateTime,
    periods: &[SprinklerWeatherHistoryPeriodV2],
) -> bool {
    periods.len() <= MAX_REPORT_WEATHER_PERIODS
        && periods.iter().all(|period| {
            let Some(ends_at) = period
                .starts_at
                .checked_add(u64::from(period.duration_seconds))
            else {
                return false;
            };
            valid_report_weather_period(period)
                && ends_at <= retrieved_at
                && ends_at
                    > retrieved_at.saturating_sub(u64::from(SPRINKLER_HISTORY_WINDOW_SECONDS))
        })
        && periods.windows(2).all(|pair| {
            pair[0]
                .starts_at
                .checked_add(u64::from(pair[0].duration_seconds))
                .is_some_and(|ends_at| ends_at <= pair[1].starts_at)
        })
}

fn valid_legacy_weather_history(history: &libertas_weather::SprinklerWeatherHistoryV1) -> bool {
    valid_weather_history_metadata(history.retrieved_at, history.valid_until)
        && !history.periods.is_empty()
        && valid_legacy_weather_history_periods(history.retrieved_at, &history.periods)
}

fn valid_weather_history(history: &SprinklerWeatherHistoryV2) -> bool {
    valid_weather_history_metadata(history.retrieved_at, history.valid_until)
        && !history.periods.is_empty()
        && valid_weather_history_periods(history.retrieved_at, &history.periods)
}

fn valid_weather_forecast_period(period: &SprinklerWeatherForecastPeriodV1) -> bool {
    period.duration_seconds > 0
        && period
            .starts_at
            .checked_add(u64::from(period.duration_seconds))
            .is_some()
        && period.temperature_celsius.is_finite()
        && period.relative_humidity_percent <= 100
        && period.precipitation_probability_percent <= 100
        && valid_nonnegative(period.expected_precipitation_millimeters)
        && valid_nonnegative(period.reference_evapotranspiration_millimeters)
        && valid_nonnegative(period.wind_speed_meters_per_second)
        && valid_nonnegative(period.wind_gust_meters_per_second)
}

fn valid_weather_forecast_periods(periods: &[SprinklerWeatherForecastPeriodV1]) -> bool {
    periods.len() <= MAX_REPORT_WEATHER_PERIODS
        && periods.iter().all(valid_weather_forecast_period)
        && periods.windows(2).all(|pair| {
            pair[0]
                .starts_at
                .checked_add(u64::from(pair[0].duration_seconds))
                .is_some_and(|ends_at| ends_at <= pair[1].starts_at)
        })
}

fn valid_weather_forecast(forecast: &SprinklerWeatherForecastV1) -> bool {
    valid_weather_history_metadata(forecast.retrieved_at, forecast.valid_until)
        && valid_weather_forecast_periods(&forecast.periods)
}

fn valid_weather_time_range(range: SprinklerWeatherTimeRangeV1) -> bool {
    range.is_valid()
        && i64::try_from(range.starts_at).is_ok()
        && i64::try_from(range.ends_before).is_ok()
}

fn valid_weather_change(change: &SprinklerWeatherChangeV1) -> bool {
    match change {
        SprinklerWeatherChangeV1::HistoryPeriodsUpsertV1 {
            retrieved_at,
            valid_until,
            periods,
        } => {
            valid_weather_history_metadata(*retrieved_at, *valid_until)
                && valid_legacy_weather_history_periods(*retrieved_at, periods)
        }
        SprinklerWeatherChangeV1::HistoryPeriodsRemoveV1 { range }
        | SprinklerWeatherChangeV1::ForecastPeriodsRemoveV1 { range } => {
            valid_weather_time_range(*range)
        }
        SprinklerWeatherChangeV1::CurrentReplaceV1 { current } => {
            valid_report_weather_observation(current)
        }
        SprinklerWeatherChangeV1::ForecastPeriodsUpsertV1 {
            retrieved_at,
            valid_until,
            periods,
        } => {
            valid_weather_history_metadata(*retrieved_at, *valid_until)
                && valid_weather_forecast_periods(periods)
        }
        SprinklerWeatherChangeV1::SectionClearV1 { .. } => true,
        SprinklerWeatherChangeV1::HistoryReplaceV1 { history } => {
            valid_legacy_weather_history(history)
        }
        SprinklerWeatherChangeV1::ForecastReplaceV1 { forecast } => {
            valid_weather_forecast(forecast)
        }
        SprinklerWeatherChangeV1::SiteReplaceV1 { location } => valid_site_location(*location),
        SprinklerWeatherChangeV1::HistoryPeriodsUpsertV2 {
            retrieved_at,
            valid_until,
            periods,
        } => {
            valid_weather_history_metadata(*retrieved_at, *valid_until)
                && valid_weather_history_periods(*retrieved_at, periods)
        }
        SprinklerWeatherChangeV1::HistoryReplaceV2 { history } => valid_weather_history(history),
    }
}

fn valid_weather_snapshot_v1(snapshot: &libertas_weather::SprinklerWeatherSnapshotV1) -> bool {
    snapshot
        .history
        .as_ref()
        .is_none_or(valid_legacy_weather_history)
        && snapshot
            .current
            .as_ref()
            .is_none_or(valid_report_weather_observation)
        && snapshot
            .forecast
            .as_ref()
            .is_none_or(valid_weather_forecast)
}

fn valid_weather_snapshot_v2(snapshot: &SprinklerWeatherSnapshotV2) -> bool {
    snapshot.history.as_ref().is_none_or(valid_weather_history)
        && snapshot
            .current
            .as_ref()
            .is_none_or(valid_report_weather_observation)
        && snapshot
            .forecast
            .as_ref()
            .is_none_or(valid_weather_forecast)
}

fn persist_report_weather_observation(
    weather_endpoint: LibertasEndpoint,
    generation: u64,
    observation: SprinklerCurrentWeatherV1,
) {
    if !valid_report_weather_observation(&observation) {
        return;
    }
    let Ok(index) = i64::try_from(observation.valid_at) else {
        return;
    };
    let database = libertas_data_open_indexed(
        REPORT_WEATHER_OBSERVATIONS_RESOURCE,
        &report_weather_archive_key(weather_endpoint, generation),
    );
    libertas_data_write_indexed(
        database.handle,
        index,
        &SprinklerDataV1::ReportWeatherObservationV1 { observation },
    );
}

fn remove_report_weather_observation(
    weather_endpoint: LibertasEndpoint,
    generation: u64,
    valid_at: LibertasDateTime,
) {
    let Ok(index) = i64::try_from(valid_at) else {
        return;
    };
    let database = libertas_data_open_indexed(
        REPORT_WEATHER_OBSERVATIONS_RESOURCE,
        &report_weather_archive_key(weather_endpoint, generation),
    );
    if database.count > 0 {
        libertas_data_remove_indexed_records(database.handle, index, index);
    }
}

fn archive_weather_changes(
    weather_endpoint: LibertasEndpoint,
    generation: u64,
    changes: &[SprinklerWeatherChangeV1],
) {
    for change in changes {
        match change {
            SprinklerWeatherChangeV1::HistoryPeriodsUpsertV2 { periods, .. } => {
                persist_report_weather_periods(weather_endpoint, generation, periods);
            }
            SprinklerWeatherChangeV1::HistoryReplaceV2 {
                history: SprinklerWeatherHistoryV2 { periods, .. },
            } => replace_report_weather_period_span(weather_endpoint, generation, periods),
            SprinklerWeatherChangeV1::HistoryPeriodsUpsertV1 { periods, .. } => {
                persist_legacy_report_weather_periods(weather_endpoint, generation, periods);
            }
            SprinklerWeatherChangeV1::HistoryReplaceV1 { history } => {
                replace_legacy_report_weather_period_span(
                    weather_endpoint,
                    generation,
                    &history.periods,
                );
            }
            // This stream describes the provider's bounded working cache. A
            // removal therefore must not age-prune the separate indefinite
            // report archive. Same-start corrections arrive as upserts.
            SprinklerWeatherChangeV1::HistoryPeriodsRemoveV1 { .. } => {}
            SprinklerWeatherChangeV1::CurrentReplaceV1 { current } => {
                persist_report_weather_observation(weather_endpoint, generation, *current);
            }
            SprinklerWeatherChangeV1::ForecastPeriodsUpsertV1 { .. }
            | SprinklerWeatherChangeV1::ForecastPeriodsRemoveV1 { .. }
            | SprinklerWeatherChangeV1::SectionClearV1 { .. }
            | SprinklerWeatherChangeV1::ForecastReplaceV1 { .. }
            | SprinklerWeatherChangeV1::SiteReplaceV1 { .. } => {}
        }
    }
}

fn archive_weather_recovery(
    weather_endpoint: LibertasEndpoint,
    generation: u64,
    recovery: &SprinklerWeatherRecoveryV1,
) {
    match recovery {
        SprinklerWeatherRecoveryV1::ReplayedV1 { report } => {
            archive_weather_changes(weather_endpoint, generation, &report.changes);
        }
        SprinklerWeatherRecoveryV1::ResetV1 { snapshot, .. }
        | SprinklerWeatherRecoveryV1::ResetAtSiteV1 { snapshot, .. } => {
            if let Some(history) = &snapshot.history {
                replace_legacy_report_weather_period_span(
                    weather_endpoint,
                    generation,
                    &history.periods,
                );
            }
            if let Some(current) = snapshot.current {
                persist_report_weather_observation(weather_endpoint, generation, current);
            }
        }
        SprinklerWeatherRecoveryV1::ResetAtSiteV2 { snapshot, .. } => {
            if let Some(history) = &snapshot.history {
                replace_report_weather_period_span(weather_endpoint, generation, &history.periods);
            }
            if let Some(current) = snapshot.current {
                persist_report_weather_observation(weather_endpoint, generation, current);
            }
        }
        SprinklerWeatherRecoveryV1::ErrorV1 { .. } => {}
    }
}

fn weather_change_is_site_boundary(change: &SprinklerWeatherChangeV1) -> bool {
    matches!(change, SprinklerWeatherChangeV1::SiteReplaceV1 { .. })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ReportWeatherSiteTransition {
    archive_state: SprinklerReportWeatherArchiveStateV2,
    binding_changed: bool,
    generation_changed: bool,
}

fn transition_report_weather_site(
    current: SprinklerReportWeatherArchiveStateV2,
    location: SprinklerWeatherLocationV1,
) -> Option<ReportWeatherSiteTransition> {
    if !valid_site_location(location) {
        return None;
    }
    let mut archive_state = current;
    let generation_changed = current
        .location
        .is_some_and(|saved| !same_weather_location(saved, location));
    if generation_changed {
        archive_state.generation = archive_state.generation.checked_add(1)?;
    }
    archive_state.location = Some(location);
    Some(ReportWeatherSiteTransition {
        binding_changed: archive_state != current,
        archive_state,
        generation_changed,
    })
}

fn transition_report_weather_sites(
    initial: SprinklerReportWeatherArchiveStateV2,
    changes: &[SprinklerWeatherChangeV1],
) -> Option<ReportWeatherSiteTransition> {
    let mut transition = ReportWeatherSiteTransition {
        archive_state: initial,
        binding_changed: false,
        generation_changed: false,
    };
    for change in changes {
        if let SprinklerWeatherChangeV1::SiteReplaceV1 { location } = change {
            let next = transition_report_weather_site(transition.archive_state, *location)?;
            transition.binding_changed |= next.binding_changed;
            transition.generation_changed |= next.generation_changed;
            transition.archive_state = next.archive_state;
        }
    }
    Some(transition)
}

fn provider_site_matches_authoritative_hub(
    provider_site: Option<SprinklerWeatherLocationV1>,
    hub_site: Option<SprinklerWeatherLocationV1>,
    hub_location_subscription_ready: bool,
) -> bool {
    !hub_location_subscription_ready
        || matches!(
            (provider_site, hub_site),
            (Some(provider), Some(hub)) if same_weather_location(provider, hub)
        )
}

fn apply_weather_report_changes(
    mut snapshot: SprinklerWeatherSnapshotV2,
    changes: &[SprinklerWeatherChangeV1],
) -> SprinklerWeatherSnapshotV2 {
    for change in changes {
        if weather_change_is_site_boundary(change) {
            snapshot = SprinklerWeatherSnapshotV2 {
                history: None,
                current: None,
                forecast: None,
            };
        } else {
            apply_weather_change(&mut snapshot, change.clone());
        }
    }
    snapshot
}

fn archive_weather_changes_by_site(
    endpoint: LibertasEndpoint,
    initial_state: SprinklerReportWeatherArchiveStateV2,
    initial_current: Option<SprinklerCurrentWeatherV1>,
    changes: &[SprinklerWeatherChangeV1],
) {
    let mut archive_state = initial_state;
    let mut current = initial_current;
    for change in changes {
        if let SprinklerWeatherChangeV1::SiteReplaceV1 { location } = change {
            let Some(next) = transition_report_weather_site(archive_state, *location) else {
                return;
            };
            archive_state = next.archive_state;
            current = None;
            continue;
        }
        if archive_state.location.is_none() {
            continue;
        }
        if matches!(
            change,
            SprinklerWeatherChangeV1::SectionClearV1 {
                section: SprinklerWeatherSectionV1::Current
            }
        ) {
            if let Some(previous) = current.take() {
                remove_report_weather_observation(
                    endpoint,
                    archive_state.generation,
                    previous.valid_at,
                );
            }
        } else if let SprinklerWeatherChangeV1::CurrentReplaceV1 { current: next } = change {
            current = Some(*next);
        }
        archive_weather_changes(
            endpoint,
            archive_state.generation,
            core::slice::from_ref(change),
        );
    }
}

fn watering_activity_index(
    anchor: LibertasDateTime,
    origin: SprinklerWateringOriginV1,
    ordinal: u16,
) -> Option<i64> {
    if ordinal >= REPORT_ACTIVITY_INDEXES_PER_ORIGIN {
        return None;
    }
    let origin_offset = match origin {
        SprinklerWateringOriginV1::Automatic => 0_i64,
        SprinklerWateringOriginV1::Manual => i64::from(REPORT_ACTIVITY_INDEXES_PER_ORIGIN),
        SprinklerWateringOriginV1::LegacyUnknown => {
            i64::from(REPORT_ACTIVITY_INDEXES_PER_ORIGIN) * 2
        }
    };
    i64::try_from(anchor)
        .ok()?
        .checked_mul(REPORT_ACTIVITY_INDEXES_PER_SECOND)?
        .checked_add(origin_offset)?
        .checked_add(i64::from(ordinal))
}

fn allocate_watering_activity_index(
    valve: LibertasDevice,
    anchor: LibertasDateTime,
    origin: SprinklerWateringOriginV1,
) -> Option<(i64, u16)> {
    let database = libertas_data_open_indexed(WATERING_ACTIVITIES_RESOURCE, &zone_key(valve));
    for ordinal in 0..REPORT_ACTIVITY_INDEXES_PER_ORIGIN {
        let index = watering_activity_index(anchor, origin, ordinal)?;
        if libertas_data_read_indexed::<SprinklerDataV1>(database.handle, index).is_none() {
            return Some((index, ordinal));
        }
    }
    None
}

fn activity_anchor(activity: &SprinklerWateringActivityV1) -> Option<LibertasDateTime> {
    activity.scheduled_starts_at.or(activity.actual_starts_at)
}

fn valid_watering_activity(activity: &SprinklerWateringActivityV1) -> bool {
    let Some(anchor) = activity_anchor(activity) else {
        return false;
    };
    watering_activity_index(anchor, activity.origin, activity.activity_ordinal)
        == Some(activity.activity_index)
        && valid_watering_percent(activity.watering_percent)
        && activity
            .scheduled_duration_seconds
            .is_none_or(|duration| duration > 0)
        && activity
            .planned_water_millimeters
            .is_none_or(valid_nonnegative)
        && activity
            .actual_duration_seconds
            .is_none_or(|duration| duration > 0)
        && activity
            .applied_water_millimeters
            .is_none_or(valid_nonnegative)
}

fn watering_activity_is_current(activity: &SprinklerWateringActivityV1) -> bool {
    matches!(
        activity.outcome,
        SprinklerWateringOutcomeV1::Scheduled
            | SprinklerWateringOutcomeV1::CommandPending
            | SprinklerWateringOutcomeV1::Running
    )
}

#[derive(Debug, PartialEq)]
enum SavedWateringActivityState {
    Legacy(SprinklerWateringActivityStateV1),
    Authoritative(SprinklerWateringActivityStateV2),
}

fn load_watering_activity_state(valve: LibertasDevice) -> Option<SavedWateringActivityState> {
    match libertas_data_read_single(WATERING_ACTIVITY_STATE_RESOURCE, &zone_key(valve)) {
        Some(SprinklerDataV1::WateringActivityStateV1 { state }) => {
            Some(SavedWateringActivityState::Legacy(state))
        }
        Some(SprinklerDataV1::WateringActivityStateV2 { state }) => {
            Some(SavedWateringActivityState::Authoritative(state))
        }
        _ => None,
    }
}

fn empty_watering_activity_state() -> SprinklerWateringActivityStateV2 {
    SprinklerWateringActivityStateV2 {
        latest_activity: None,
        activity_is_current: false,
    }
}

fn watering_activity_state(
    activity: SprinklerWateringActivityV1,
) -> SprinklerWateringActivityStateV2 {
    SprinklerWateringActivityStateV2 {
        activity_is_current: watering_activity_is_current(&activity),
        latest_activity: Some(activity),
    }
}

fn persist_watering_activity_state(valve: LibertasDevice, state: SprinklerWateringActivityStateV2) {
    libertas_data_write_single(
        WATERING_ACTIVITY_STATE_RESOURCE,
        &zone_key(valve),
        &SprinklerDataV1::WateringActivityStateV2 { state },
    );
}

#[derive(Debug, PartialEq)]
enum WateringActivityLoadPlan {
    MigrateArchive,
    RepairArchive {
        activity: SprinklerWateringActivityV1,
        return_current: bool,
        migrate_state: bool,
    },
    ClearInvalidState,
    ReturnEmpty {
        migrate_state: bool,
    },
}

fn watering_activity_load_plan(
    saved_state: Option<SavedWateringActivityState>,
) -> WateringActivityLoadPlan {
    match saved_state {
        None => WateringActivityLoadPlan::MigrateArchive,
        Some(SavedWateringActivityState::Legacy(SprinklerWateringActivityStateV1 {
            current_activity: Some(activity),
        })) if valid_watering_activity(&activity) && watering_activity_is_current(&activity) => {
            WateringActivityLoadPlan::RepairArchive {
                activity,
                return_current: true,
                migrate_state: true,
            }
        }
        Some(SavedWateringActivityState::Legacy(SprinklerWateringActivityStateV1 {
            current_activity: Some(_),
        })) => WateringActivityLoadPlan::ClearInvalidState,
        Some(SavedWateringActivityState::Legacy(SprinklerWateringActivityStateV1 {
            current_activity: None,
        })) => WateringActivityLoadPlan::ReturnEmpty {
            migrate_state: true,
        },
        Some(SavedWateringActivityState::Authoritative(SprinklerWateringActivityStateV2 {
            latest_activity: Some(activity),
            activity_is_current,
        })) if valid_watering_activity(&activity)
            && activity_is_current == watering_activity_is_current(&activity) =>
        {
            WateringActivityLoadPlan::RepairArchive {
                activity,
                return_current: activity_is_current,
                migrate_state: false,
            }
        }
        Some(SavedWateringActivityState::Authoritative(SprinklerWateringActivityStateV2 {
            latest_activity: None,
            activity_is_current: false,
        })) => WateringActivityLoadPlan::ReturnEmpty {
            migrate_state: false,
        },
        Some(SavedWateringActivityState::Authoritative(_)) => {
            WateringActivityLoadPlan::ClearInvalidState
        }
    }
}

fn activity_report_days(activity: &SprinklerWateringActivityV1) -> Vec<LibertasDateTime> {
    let Some((starts_at, ends_at)) = activity_interval(activity) else {
        return Vec::new();
    };
    if starts_at >= ends_at {
        return Vec::new();
    }
    let last_day = utc_day_start(ends_at.saturating_sub(1));
    let mut days = Vec::new();
    let mut day = utc_day_start(starts_at);
    loop {
        days.push(day);
        if day >= last_day {
            break;
        }
        let next = day.saturating_add(SECONDS_PER_DAY);
        if next <= day {
            break;
        }
        day = next;
    }
    days
}

fn persist_watering_activity_days(valve: LibertasDevice, activity: &SprinklerWateringActivityV1) {
    for day in activity_report_days(activity) {
        let database = libertas_data_open_indexed(
            WATERING_ACTIVITY_DAYS_RESOURCE,
            &activity_day_key(valve, day),
        );
        libertas_data_write_indexed(
            database.handle,
            activity.activity_index,
            &SprinklerDataV1::WateringActivityV1 {
                activity: activity.clone(),
            },
        );
    }
}

fn persist_watering_activity(valve: LibertasDevice, activity: &SprinklerWateringActivityV1) {
    if !valid_watering_activity(activity) {
        return;
    }
    // The bounded state is authoritative across a restart. Write it first so
    // a crash cannot resurrect a stale nonterminal audit row after a terminal
    // transition. A missing audit write is repaired when this state is loaded.
    persist_watering_activity_state(valve, watering_activity_state(activity.clone()));
    // Materialize every overlapped UTC day before the primary audit write.
    // A report therefore reads only its at-most-32 day buckets even when a
    // manual or stuck-open interval is much longer than an automatic command.
    persist_watering_activity_days(valve, activity);
    let database = libertas_data_open_indexed(WATERING_ACTIVITIES_RESOURCE, &zone_key(valve));
    libertas_data_write_indexed(
        database.handle,
        activity.activity_index,
        &SprinklerDataV1::WateringActivityV1 {
            activity: activity.clone(),
        },
    );
}

fn load_current_watering_activity(valve: LibertasDevice) -> Option<SprinklerWateringActivityV1> {
    match watering_activity_load_plan(load_watering_activity_state(valve)) {
        WateringActivityLoadPlan::RepairArchive {
            activity,
            return_current,
            migrate_state,
        } => {
            if migrate_state {
                // Establish the crash-repairable V2 state before repairing the
                // legacy audit row for the same ordering guarantee as new writes.
                persist_watering_activity_state(valve, watering_activity_state(activity.clone()));
            }
            // Repair a missing or stale audit write from the authoritative
            // bounded snapshot before returning it.
            persist_watering_activity_days(valve, &activity);
            let database =
                libertas_data_open_indexed(WATERING_ACTIVITIES_RESOURCE, &zone_key(valve));
            libertas_data_write_indexed(
                database.handle,
                activity.activity_index,
                &SprinklerDataV1::WateringActivityV1 {
                    activity: activity.clone(),
                },
            );
            return return_current.then_some(activity);
        }
        WateringActivityLoadPlan::ClearInvalidState => {
            persist_watering_activity_state(valve, empty_watering_activity_state());
            return None;
        }
        // An explicit empty state is authoritative. Do not even open the audit
        // archive, so a stale nonterminal row cannot be resurrected after a
        // terminal-state write.
        WateringActivityLoadPlan::ReturnEmpty { migrate_state } => {
            if migrate_state {
                persist_watering_activity_state(valve, empty_watering_activity_state());
            }
            return None;
        }
        WateringActivityLoadPlan::MigrateArchive => {}
    }

    let database = libertas_data_open_indexed(WATERING_ACTIVITIES_RESOURCE, &zone_key(valve));
    if database.count == 0 {
        persist_watering_activity_state(valve, empty_watering_activity_state());
        return None;
    }

    // Migrate an archive created before the authoritative bounded state existed.
    // This is deliberately bounded; ordinary startup reads only the state record.
    let mut records = Vec::new();
    libertas_data_read_indexed_range::<SprinklerDataV1>(
        database.handle,
        database.max_index,
        IndexDirection::Below,
        32,
        &mut records,
    );
    let mut recovered = None;
    for record in records {
        if let SprinklerDataV1::WateringActivityV1 { activity } = record.data
            && activity.activity_index == record.index
            && valid_watering_activity(&activity)
            && watering_activity_is_current(&activity)
            && recovered
                .as_ref()
                .is_none_or(|saved: &SprinklerWateringActivityV1| {
                    (activity.updated_at, activity.activity_index)
                        > (saved.updated_at, saved.activity_index)
                })
        {
            recovered = Some(activity);
        }
    }
    persist_watering_activity_state(
        valve,
        recovered
            .clone()
            .map(watering_activity_state)
            .unwrap_or_else(empty_watering_activity_state),
    );
    recovered
}

fn valid_daily_report(report: &SprinklerDailyReportV1) -> bool {
    report.starts_at < report.ends_before
        && report.ends_before - report.starts_at == SECONDS_PER_DAY
        && report.starts_at.is_multiple_of(SECONDS_PER_DAY)
        && report.coverage_starts_at >= report.starts_at
        && report.coverage_starts_at < report.coverage_ends_before
        && report.coverage_ends_before <= report.ends_before
        && valid_nonnegative(report.capacity_millimeters)
        && report.opening_deficit_millimeters.is_finite()
        && (0.0..=report.capacity_millimeters).contains(&report.opening_deficit_millimeters)
        && report.closing_deficit_millimeters.is_finite()
        && (0.0..=report.capacity_millimeters).contains(&report.closing_deficit_millimeters)
        && valid_nonnegative(report.precipitation_millimeters)
        && valid_nonnegative(report.reference_evapotranspiration_millimeters)
        && valid_nonnegative(report.modeled_reference_evapotranspiration_millimeters)
        && (report.modeled_reference_evapotranspiration_millimeters > 0.0)
            == report.modeled_demand_source.is_some()
        && u64::from(report.provider_weather_coverage_seconds)
            <= report
                .coverage_ends_before
                .saturating_sub(report.coverage_starts_at)
        && valid_nonnegative(report.irrigation_millimeters)
        && report.complete
            == (report.coverage_starts_at == report.starts_at
                && report.coverage_ends_before == report.ends_before)
        && i64::try_from(report.starts_at).is_ok()
}

fn persist_daily_reports(valve: LibertasDevice, reports: &[SprinklerDailyReportV1]) {
    if reports.is_empty() {
        return;
    }
    let database = libertas_data_open_indexed(DAILY_REPORT_RESOURCE, &zone_key(valve));
    for report in reports {
        if !valid_daily_report(report) {
            continue;
        }
        let Ok(index) = i64::try_from(report.starts_at) else {
            continue;
        };
        libertas_data_write_indexed(
            database.handle,
            index,
            &SprinklerDataV1::DailyReportV1 { report: *report },
        );
    }
}

fn valid_modeled_weather_gap(gap: &SprinklerModeledWeatherGapV1) -> bool {
    gap.starts_at < gap.ends_before
        && gap.ends_before <= utc_day_start(gap.starts_at).saturating_add(SECONDS_PER_DAY)
        && valid_nonnegative(gap.reference_evapotranspiration_millimeters_per_day)
        && i64::try_from(gap.starts_at).is_ok()
        && i64::try_from(gap.ends_before).is_ok()
        && i64::try_from(gap.recorded_at).is_ok()
}

fn load_modeled_weather_gaps(
    valve: LibertasDevice,
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
    maximum: usize,
) -> Result<Vec<SprinklerModeledWeatherGapV1>, ()> {
    if starts_at >= ends_before {
        return Ok(Vec::new());
    }
    let database = libertas_data_open_indexed(MODELED_WEATHER_GAPS_RESOURCE, &zone_key(valve));
    if database.count == 0 {
        return Ok(Vec::new());
    }
    let start = i64::try_from(starts_at).map_err(|_| ())?;
    let end = i64::try_from(ends_before).map_err(|_| ())?;
    let mut records = Vec::new();
    if start > i64::MIN {
        libertas_data_read_indexed_range::<SprinklerDataV1>(
            database.handle,
            start - 1,
            IndexDirection::Below,
            1,
            &mut records,
        );
    }
    libertas_data_read_indexed_range::<SprinklerDataV1>(
        database.handle,
        start,
        IndexDirection::Above,
        maximum + 65,
        &mut records,
    );
    let mut gaps = Vec::new();
    for record in records {
        if record.index >= end {
            break;
        }
        if let SprinklerDataV1::ModeledWeatherGapV1 { gap } = record.data
            && i64::try_from(gap.starts_at) == Ok(record.index)
            && valid_modeled_weather_gap(&gap)
            && gap.starts_at < ends_before
            && gap.ends_before > starts_at
        {
            gaps.push(gap);
            if gaps.len() > maximum {
                return Err(());
            }
        }
    }
    gaps.sort_by_key(|gap| gap.starts_at);
    Ok(gaps)
}

fn persist_modeled_gap_delta(
    valve: LibertasDevice,
    previous: &[SprinklerModeledWeatherGapV1],
    current: &[SprinklerModeledWeatherGapV1],
) {
    let database = libertas_data_open_indexed(MODELED_WEATHER_GAPS_RESOURCE, &zone_key(valve));
    for gap in current {
        if !valid_modeled_weather_gap(gap)
            || previous
                .iter()
                .find(|saved| saved.starts_at == gap.starts_at)
                == Some(gap)
        {
            continue;
        }
        let Ok(index) = i64::try_from(gap.starts_at) else {
            continue;
        };
        libertas_data_write_indexed(
            database.handle,
            index,
            &SprinklerDataV1::ModeledWeatherGapV1 { gap: *gap },
        );
    }
    for gap in previous {
        if !current.iter().any(|saved| saved.starts_at == gap.starts_at)
            && let Ok(index) = i64::try_from(gap.starts_at)
        {
            // Only provider-history supersession or interval re-keying removes
            // one gap. No age-based path calls this for archived intervals.
            libertas_data_remove_indexed_records(database.handle, index, index);
        }
    }
}

fn merged_provider_intervals(
    water_events: &[SprinklerWaterEventV1],
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
) -> Vec<(LibertasDateTime, LibertasDateTime)> {
    let mut intervals: Vec<_> = water_events
        .iter()
        .filter_map(|event| {
            if !matches!(event, SprinklerWaterEventV1::WeatherV1 { .. }) {
                return None;
            }
            let start = event.starts_at().max(starts_at);
            let end = event.ends_at()?.min(ends_before);
            (start < end).then_some((start, end))
        })
        .collect();
    intervals.sort_by_key(|interval| interval.0);
    let mut merged: Vec<(LibertasDateTime, LibertasDateTime)> = Vec::new();
    for (start, end) in intervals {
        if let Some(last) = merged.last_mut()
            && start <= last.1
        {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    merged
}

fn split_modeled_gap_around_provider(
    gap: SprinklerModeledWeatherGapV1,
    provider: &[(LibertasDateTime, LibertasDateTime)],
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
) -> Vec<SprinklerModeledWeatherGapV1> {
    provider_uncovered_fragments(
        gap.starts_at.max(starts_at),
        gap.ends_before.min(ends_before),
        provider,
        usize::MAX,
    )
    .unwrap_or_default()
    .into_iter()
    .map(|(start, end)| SprinklerModeledWeatherGapV1 {
        starts_at: start,
        ends_before: end,
        ..gap
    })
    .collect()
}

fn normalized_modeled_weather_gaps(
    gaps: &[SprinklerModeledWeatherGapV1],
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
) -> Vec<SprinklerModeledWeatherGapV1> {
    let mut gaps: Vec<_> = gaps
        .iter()
        .copied()
        .filter(valid_modeled_weather_gap)
        .filter_map(|mut gap| {
            gap.starts_at = gap.starts_at.max(starts_at);
            gap.ends_before = gap.ends_before.min(ends_before);
            (gap.starts_at < gap.ends_before).then_some(gap)
        })
        .collect();
    gaps.sort_by_key(|gap| gap.starts_at);
    let mut normalized: Vec<SprinklerModeledWeatherGapV1> = Vec::new();
    for mut gap in gaps {
        if let Some(previous) = normalized.last() {
            // Earlier-start provenance already drove this interval and wins
            // over stale overlapping crash residue, matching reconciliation.
            gap.starts_at = gap.starts_at.max(previous.ends_before);
        }
        if gap.starts_at < gap.ends_before {
            normalized.push(gap);
        }
    }
    normalized
}

fn push_modeled_gap_parts(
    gaps: &mut Vec<SprinklerModeledWeatherGapV1>,
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
    provenance: Option<SprinklerModeledWeatherGapV1>,
    estimate: WaterDemandEstimate,
    recorded_at: LibertasDateTime,
) {
    let mut start = starts_at;
    while start < ends_before {
        let end = ends_before.min(utc_day_start(start).saturating_add(SECONDS_PER_DAY));
        if end <= start {
            break;
        }
        let inherited = provenance.filter(|gap| {
            gap.ends_before == starts_at && utc_day_start(gap.starts_at) == utc_day_start(start)
        });
        gaps.push(SprinklerModeledWeatherGapV1 {
            starts_at: start,
            ends_before: end,
            reference_evapotranspiration_millimeters_per_day: inherited
                .map(|gap| gap.reference_evapotranspiration_millimeters_per_day)
                .unwrap_or(estimate.reference_evapotranspiration_millimeters_per_day),
            demand_source: inherited
                .map(|gap| gap.demand_source)
                .unwrap_or(estimate.source),
            recorded_at: inherited.map(|gap| gap.recorded_at).unwrap_or(recorded_at),
        });
        start = end;
    }
}

fn reconcile_modeled_weather_gaps(
    existing: &[SprinklerModeledWeatherGapV1],
    water_events: &[SprinklerWaterEventV1],
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
    estimate: WaterDemandEstimate,
    recorded_at: LibertasDateTime,
) -> Vec<SprinklerModeledWeatherGapV1> {
    if starts_at >= ends_before {
        return Vec::new();
    }
    let provider = merged_provider_intervals(water_events, starts_at, ends_before);
    let mut preserved: Vec<_> = normalized_modeled_weather_gaps(existing, starts_at, ends_before)
        .into_iter()
        .flat_map(|gap| split_modeled_gap_around_provider(gap, &provider, starts_at, ends_before))
        .collect();
    preserved.sort_by_key(|gap| gap.starts_at);
    let mut normalized: Vec<SprinklerModeledWeatherGapV1> = Vec::new();
    for mut gap in preserved {
        if let Some(last) = normalized.last() {
            gap.starts_at = gap.starts_at.max(last.ends_before);
        }
        if gap.starts_at < gap.ends_before {
            normalized.push(gap);
        }
    }
    preserved = normalized;

    let mut occupied = provider;
    occupied.extend(preserved.iter().map(|gap| (gap.starts_at, gap.ends_before)));
    occupied.sort_by_key(|interval| interval.0);
    let mut merged: Vec<(LibertasDateTime, LibertasDateTime)> = Vec::new();
    for (start, end) in occupied {
        if let Some(last) = merged.last_mut()
            && start <= last.1
        {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    let mut uncovered = Vec::new();
    let mut cursor = starts_at;
    for (start, end) in merged {
        if cursor < start {
            uncovered.push((cursor, start));
        }
        cursor = cursor.max(end);
    }
    if cursor < ends_before {
        uncovered.push((cursor, ends_before));
    }

    let mut result = preserved;
    for (start, end) in uncovered {
        let provenance = result.iter().find(|gap| gap.ends_before == start).copied();
        push_modeled_gap_parts(&mut result, start, end, provenance, estimate, recorded_at);
    }
    result.sort_by_key(|gap| gap.starts_at);
    let mut combined: Vec<SprinklerModeledWeatherGapV1> = Vec::new();
    for gap in result {
        if let Some(last) = combined.last_mut()
            && last.ends_before == gap.starts_at
            && utc_day_start(last.starts_at) == utc_day_start(gap.starts_at)
            && last.reference_evapotranspiration_millimeters_per_day
                == gap.reference_evapotranspiration_millimeters_per_day
            && last.demand_source == gap.demand_source
            && last.recorded_at == gap.recorded_at
        {
            last.ends_before = gap.ends_before;
        } else {
            combined.push(gap);
        }
    }
    combined
}

fn reconcile_zone_modeled_weather_gaps(
    zone: &mut ZoneRuntime,
    site_location: Option<SprinklerWeatherLocationV1>,
    now: LibertasDateTime,
) -> Option<ModeledGapPersistenceChange> {
    let starts_at = zone.memory.balance_baseline_at;
    if starts_at >= now {
        return None;
    }
    let previous: Vec<_> = zone
        .modeled_weather_gaps
        .iter()
        .copied()
        .filter(|gap| gap.ends_before > starts_at && gap.starts_at < now)
        .collect();
    let estimate = water_demand_estimate(&zone.water_events, site_location, now);
    let current = reconcile_modeled_weather_gaps(
        &previous,
        &zone.water_events,
        starts_at,
        now,
        estimate,
        now,
    );
    zone.modeled_weather_gaps = current.clone();
    (current != previous).then_some(ModeledGapPersistenceChange {
        valve: zone.configuration.valve,
        previous,
        current,
    })
}

#[derive(Default)]
struct ReportWeatherPeriods {
    balance: Vec<SprinklerWeatherHistoryPeriodV1>,
    full: Vec<SprinklerWeatherHistoryPeriodV2>,
}

fn load_report_weather_periods(
    weather_endpoint: LibertasEndpoint,
    generation: u64,
    range: SprinklerReportTimeRangeV1,
) -> Result<ReportWeatherPeriods, ()> {
    let start = i64::try_from(range.starts_at).map_err(|_| ())?;
    let end = i64::try_from(range.ends_before).map_err(|_| ())?;
    let mut periods = ReportWeatherPeriods::default();
    for (resource, full_observations) in [
        (REPORT_WEATHER_HISTORY_RESOURCE, false),
        (REPORT_WEATHER_HISTORY_V2_RESOURCE, true),
    ] {
        let database = libertas_data_open_indexed(
            resource,
            &report_weather_archive_key(weather_endpoint, generation),
        );
        if database.count == 0 {
            continue;
        }
        let mut records = Vec::new();
        if start > i64::MIN {
            libertas_data_read_indexed_range::<SprinklerDataV1>(
                database.handle,
                start - 1,
                IndexDirection::Below,
                1,
                &mut records,
            );
        }
        libertas_data_read_indexed_range::<SprinklerDataV1>(
            database.handle,
            start,
            IndexDirection::Above,
            MAX_REPORT_WEATHER_PERIODS + 65,
            &mut records,
        );
        for record in records {
            if record.index >= end {
                break;
            }
            match record.data {
                SprinklerDataV1::ReportWeatherPeriodV1 { period }
                    if !full_observations
                        && i64::try_from(period.starts_at) == Ok(record.index)
                        && valid_legacy_report_weather_period(&period)
                        && period.starts_at < range.ends_before
                        && period
                            .starts_at
                            .saturating_add(u64::from(period.duration_seconds))
                            > range.starts_at =>
                {
                    periods.balance.push(period);
                }
                SprinklerDataV1::ReportWeatherPeriodV2 { period }
                    if full_observations
                        && report_weather_period_index(&period) == Some(record.index)
                        && valid_report_weather_period(&period)
                        && period.starts_at < range.ends_before
                        && period
                            .starts_at
                            .saturating_add(u64::from(period.duration_seconds))
                            > range.starts_at =>
                {
                    let balance_period = period.into();
                    if let Some(existing) = periods
                        .balance
                        .iter_mut()
                        .find(|existing| existing.starts_at == period.starts_at)
                    {
                        *existing = balance_period;
                    } else {
                        periods.balance.push(balance_period);
                    }
                    periods.full.push(period);
                }
                _ => {}
            }
            if periods.balance.len() > MAX_REPORT_WEATHER_PERIODS {
                return Err(());
            }
        }
    }
    periods.balance.sort_by_key(|period| period.starts_at);
    periods.full.sort_by_key(|period| period.starts_at);
    Ok(periods)
}

fn load_report_weather_observations(
    weather_endpoint: LibertasEndpoint,
    generation: u64,
    range: SprinklerReportTimeRangeV1,
) -> Result<Vec<SprinklerCurrentWeatherV1>, ()> {
    let database = libertas_data_open_indexed(
        REPORT_WEATHER_OBSERVATIONS_RESOURCE,
        &report_weather_archive_key(weather_endpoint, generation),
    );
    if database.count == 0 {
        return Ok(Vec::new());
    }
    let start = i64::try_from(range.starts_at).map_err(|_| ())?;
    let end = i64::try_from(range.ends_before).map_err(|_| ())?;
    let mut records = Vec::new();
    libertas_data_read_indexed_range::<SprinklerDataV1>(
        database.handle,
        start,
        IndexDirection::Above,
        MAX_REPORT_WEATHER_OBSERVATIONS + 65,
        &mut records,
    );
    let mut observations = Vec::new();
    for record in records {
        if record.index >= end {
            break;
        }
        if let SprinklerDataV1::ReportWeatherObservationV1 { observation } = record.data
            && i64::try_from(observation.valid_at) == Ok(record.index)
            && valid_report_weather_observation(&observation)
        {
            observations.push(observation);
            if observations.len() > MAX_REPORT_WEATHER_OBSERVATIONS {
                return Err(());
            }
        }
    }
    observations.sort_by_key(|observation| observation.valid_at);
    Ok(observations)
}

fn activity_interval(activity: &SprinklerWateringActivityV1) -> Option<(u64, u64)> {
    let starts_at = activity.actual_starts_at.or(activity.scheduled_starts_at)?;
    let duration = activity
        .actual_duration_seconds
        .or(activity.scheduled_duration_seconds)
        .unwrap_or(60)
        .max(1);
    Some((starts_at, starts_at.saturating_add(u64::from(duration))))
}

fn report_activity_overlaps(
    activity: &SprinklerWateringActivityV1,
    range: SprinklerReportTimeRangeV1,
) -> bool {
    valid_watering_activity(activity)
        && activity_interval(activity).is_some_and(|(starts_at, ends_at)| {
            starts_at < range.ends_before && ends_at > range.starts_at
        })
}

fn merge_report_activity(
    activities: &mut Vec<SprinklerWateringActivityV1>,
    record: IndexedData<SprinklerDataV1>,
    range: SprinklerReportTimeRangeV1,
    maximum: usize,
) -> Result<(), ()> {
    let SprinklerDataV1::WateringActivityV1 { activity } = record.data else {
        return Ok(());
    };
    if activity.activity_index != record.index || !report_activity_overlaps(&activity, range) {
        return Ok(());
    }
    if let Some(saved) = activities
        .iter_mut()
        .find(|saved| saved.activity_index == activity.activity_index)
    {
        if activity.updated_at >= saved.updated_at {
            *saved = activity;
        }
        return Ok(());
    }
    activities.push(activity);
    (activities.len() <= maximum).then_some(()).ok_or(())
}

fn load_report_activities(
    valve: LibertasDevice,
    range: SprinklerReportTimeRangeV1,
    maximum: usize,
    remaining_records_scanned: &mut usize,
) -> Result<Vec<SprinklerWateringActivityV1>, ()> {
    let start = i64::try_from(range.starts_at)
        .map_err(|_| ())?
        .checked_mul(REPORT_ACTIVITY_INDEXES_PER_SECOND)
        .ok_or(())?;
    let end = i64::try_from(range.ends_before)
        .map_err(|_| ())?
        .checked_mul(REPORT_ACTIVITY_INDEXES_PER_SECOND)
        .ok_or(())?;
    let mut activities = Vec::new();

    // The primary archive efficiently supplies activities whose identity is
    // anchored inside the query. Long predecessors are supplied by the day
    // overlap index below instead of an unsafe unbounded backward scan.
    let database = libertas_data_open_indexed(WATERING_ACTIVITIES_RESOURCE, &zone_key(valve));
    if database.count > 0 {
        let mut records = Vec::new();
        libertas_data_read_indexed_range::<SprinklerDataV1>(
            database.handle,
            start,
            IndexDirection::Above,
            remaining_records_scanned.saturating_add(1),
            &mut records,
        );
        // Records at or beyond `end` are not part of this query and therefore
        // do not consume its scan budget, even though the one-sided database
        // API may have returned them in the same bounded read.
        let records_in_range = records
            .iter()
            .position(|record| record.index >= end)
            .unwrap_or(records.len());
        if records_in_range > *remaining_records_scanned {
            return Err(());
        }
        *remaining_records_scanned -= records_in_range;
        for record in records.into_iter().take(records_in_range) {
            merge_report_activity(&mut activities, record, range, maximum)?;
        }
    }

    let last_day = utc_day_start(range.ends_before.saturating_sub(1));
    let mut day = utc_day_start(range.starts_at);
    loop {
        let database = libertas_data_open_indexed(
            WATERING_ACTIVITY_DAYS_RESOURCE,
            &activity_day_key(valve, day),
        );
        if database.count > 0 {
            let mut records = Vec::new();
            libertas_data_read_indexed_range::<SprinklerDataV1>(
                database.handle,
                database.min_index,
                IndexDirection::Above,
                remaining_records_scanned.saturating_add(1),
                &mut records,
            );
            if records.len() > *remaining_records_scanned {
                return Err(());
            }
            *remaining_records_scanned -= records.len();
            for record in records {
                merge_report_activity(&mut activities, record, range, maximum)?;
            }
        }
        if day >= last_day {
            break;
        }
        let next = day.saturating_add(SECONDS_PER_DAY);
        if next <= day {
            break;
        }
        day = next;
    }
    activities.sort_by_key(|activity| activity.activity_index);
    Ok(activities)
}

fn load_report_daily_records(
    valve: LibertasDevice,
    range: SprinklerReportTimeRangeV1,
) -> Result<Vec<SprinklerDailyReportV1>, ()> {
    let database = libertas_data_open_indexed(DAILY_REPORT_RESOURCE, &zone_key(valve));
    if database.count == 0 {
        return Ok(Vec::new());
    }
    let first_day = utc_day_start(range.starts_at);
    let start = i64::try_from(first_day).map_err(|_| ())?;
    let mut records = Vec::new();
    libertas_data_read_indexed_range::<SprinklerDataV1>(
        database.handle,
        start,
        IndexDirection::Above,
        MAX_REPORT_DAILY_RECORDS_PER_ZONE + 2,
        &mut records,
    );
    let mut reports = Vec::new();
    for record in records {
        if u64::try_from(record.index).is_ok_and(|index| index >= range.ends_before) {
            break;
        }
        if let SprinklerDataV1::DailyReportV1 { report } = record.data
            && i64::try_from(report.starts_at) == Ok(record.index)
            && valid_daily_report(&report)
            && report.starts_at < range.ends_before
            && report.ends_before > range.starts_at
        {
            reports.push(report);
            if reports.len() > MAX_REPORT_DAILY_RECORDS_PER_ZONE {
                return Err(());
            }
        }
    }
    reports.sort_by_key(|report| report.starts_at);
    Ok(reports)
}

#[derive(Default)]
struct WaterEventDelta {
    upserts: Vec<(i64, SprinklerWaterEventV1)>,
    removals: Vec<i64>,
}

fn water_event_delta(
    previous: &[SprinklerWaterEventV1],
    current: &[SprinklerWaterEventV1],
) -> WaterEventDelta {
    let mut delta = WaterEventDelta::default();
    for event in current {
        let Some(index) = water_event_index(event) else {
            continue;
        };
        if previous
            .iter()
            .find(|previous| water_event_index(previous) == Some(index))
            != Some(event)
        {
            delta.upserts.push((index, event.clone()));
        }
    }
    for event in previous {
        let Some(index) = water_event_index(event) else {
            continue;
        };
        if !current
            .iter()
            .any(|current| water_event_index(current) == Some(index))
        {
            delta.removals.push(index);
        }
    }
    delta
}

fn persist_water_event_delta(
    valve: LibertasDevice,
    previous: &[SprinklerWaterEventV1],
    current: &[SprinklerWaterEventV1],
) {
    let delta = water_event_delta(previous, current);
    if delta.upserts.is_empty() && delta.removals.is_empty() {
        return;
    }
    let database = libertas_data_open_indexed(WATER_EVENTS_RESOURCE, &zone_key(valve));
    for (index, event) in delta.upserts {
        libertas_data_write_indexed(
            database.handle,
            index,
            &SprinklerDataV1::WaterEventV1 { event },
        );
    }
    for index in delta.removals {
        libertas_data_remove_indexed_records(database.handle, index, index);
    }
}

fn persist_zone_runtime_change(
    valve: LibertasDevice,
    previous_memory: &SprinklerZoneMemoryV1,
    memory: &SprinklerZoneMemoryV1,
    previous_events: &[SprinklerWaterEventV1],
    water_events: &[SprinklerWaterEventV1],
) {
    // Persist a folded baseline before deleting the events it replaces. If the
    // process stops between these asynchronous submissions, startup filters the
    // still-present events through the persisted baseline instead of counting
    // them twice.
    if memory != previous_memory {
        persist_zone_memory(valve, memory);
    }
    persist_water_event_delta(valve, previous_events, water_events);
}

fn event_delta_millimeters(event: &SprinklerWaterEventV1, crop_coefficient: f32) -> f32 {
    match event {
        SprinklerWaterEventV1::WeatherV1 {
            precipitation_millimeters,
            reference_evapotranspiration_millimeters,
            ..
        } => {
            reference_evapotranspiration_millimeters * crop_coefficient - precipitation_millimeters
        }
        SprinklerWaterEventV1::IrrigationV1 {
            applied_water_millimeters,
            ..
        } => -*applied_water_millimeters,
    }
}

fn apply_deficit_delta(deficit: f32, delta: f32, capacity: f32) -> f32 {
    (deficit + delta).clamp(0.0, capacity)
}

fn sort_water_events(events: &mut [SprinklerWaterEventV1]) {
    events.sort_by_key(water_event_index);
}

fn utc_day_ceiling(at: LibertasDateTime) -> LibertasDateTime {
    let day = utc_day_start(at);
    if at == day {
        day
    } else {
        day.saturating_add(SECONDS_PER_DAY)
    }
}

fn prune_water_events(
    memory: &mut SprinklerZoneMemoryV1,
    water_events: &mut Vec<SprinklerWaterEventV1>,
    modeled_weather_gaps: &mut Vec<SprinklerModeledWeatherGapV1>,
    zone: &SprinklerZoneV1,
    _site_location: Option<SprinklerWeatherLocationV1>,
    now: LibertasDateTime,
) -> Vec<SprinklerDailyReportV1> {
    sort_water_events(water_events);
    // Fold only at a UTC-day boundary so an already finalized daily checkpoint
    // is never replaced by a reconstruction that starts in the middle of that
    // day. Projecting to the fold boundary also includes modeled demand in
    // weather gaps; merely summing retained event deltas would lose it.
    let last_safe_boundary = utc_day_start(now);
    let mut fold_through = utc_day_start(now.saturating_sub(RECENT_WATER_WINDOW_SECONDS));
    let retained_after_cutoff = water_events
        .iter()
        .filter(|event| event.ends_at().is_none_or(|ends_at| ends_at > fold_through))
        .count();
    if retained_after_cutoff > MAX_WATER_EVENTS {
        let excess = retained_after_cutoff - MAX_WATER_EVENTS;
        let mut ends_at = water_events
            .iter()
            .filter_map(SprinklerWaterEventV1::ends_at)
            .filter(|ends_at| *ends_at > fold_through)
            .collect::<Vec<_>>();
        ends_at.sort_unstable();
        if let Some(pressure_boundary) = ends_at
            .get(excess.saturating_sub(1))
            .copied()
            .map(utc_day_ceiling)
            .filter(|boundary| *boundary <= last_safe_boundary)
        {
            fold_through = fold_through.max(pressure_boundary);
        }
    }
    if fold_through <= memory.balance_baseline_at {
        return Vec::new();
    }
    // Build every checkpoint that would become unreconstructable before
    // advancing the baseline or removing its source events. The caller submits
    // these records before the memory/event delta for crash-safe ordering.
    let finalized_reports = build_daily_reports_from_ledger(
        zone,
        memory,
        water_events,
        modeled_weather_gaps,
        fold_through,
        usize::MAX,
    );
    let folded_deficit = projected_deficit_with_modeled_gaps(
        zone,
        memory,
        water_events,
        modeled_weather_gaps,
        fold_through,
    );
    memory.balance_baseline_at = fold_through;
    memory.baseline_deficit_millimeters = folded_deficit;
    water_events.retain(|event| event.ends_at().is_none_or(|ends_at| ends_at > fold_through));
    // Runtime state is bounded at the folded baseline; the separate indexed
    // report archive is deliberately not age-pruned.
    modeled_weather_gaps.retain(|gap| gap.ends_before > fold_through);
    finalized_reports
}

#[cfg(test)]
fn estimated_deficit_millimeters(
    zone: &SprinklerZoneV1,
    memory: &SprinklerZoneMemoryV1,
    water_events: &[SprinklerWaterEventV1],
) -> f32 {
    let capacity = root_zone_capacity_millimeters(zone);
    let crop_coefficient = plant_profile(zone.plant_type).crop_coefficient;
    water_events
        .iter()
        .fold(memory.baseline_deficit_millimeters, |deficit, event| {
            apply_deficit_delta(
                deficit,
                event_delta_millimeters(event, crop_coefficient),
                capacity,
            )
        })
}

fn fallback_demand_millimeters(
    duration_seconds: u64,
    crop_coefficient: f32,
    estimate: WaterDemandEstimate,
) -> f32 {
    estimate.reference_evapotranspiration_millimeters_per_day
        * crop_coefficient
        * duration_seconds as f32
        / SECONDS_PER_DAY as f32
}

#[derive(Clone, Copy)]
struct BalanceRateInterval {
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
    deficit_millimeters_per_second: f32,
}

fn water_event_balance_intervals(
    water_events: &[SprinklerWaterEventV1],
    crop_coefficient: f32,
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
) -> Vec<BalanceRateInterval> {
    water_events
        .iter()
        .filter_map(|event| {
            let event_end = event.ends_at()?;
            let start = event.starts_at().max(starts_at);
            let end = event_end.min(ends_before);
            if start >= end || event.duration_seconds() == 0 {
                return None;
            }
            Some(BalanceRateInterval {
                starts_at: start,
                ends_before: end,
                deficit_millimeters_per_second: event_delta_millimeters(event, crop_coefficient)
                    / event.duration_seconds() as f32,
            })
        })
        .collect()
}

fn modeled_gap_balance_intervals(
    gaps: &[SprinklerModeledWeatherGapV1],
    crop_coefficient: f32,
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
) -> Vec<BalanceRateInterval> {
    gaps.iter()
        .filter_map(|gap| {
            let start = gap.starts_at.max(starts_at);
            let end = gap.ends_before.min(ends_before);
            (start < end && valid_modeled_weather_gap(gap)).then_some(BalanceRateInterval {
                starts_at: start,
                ends_before: end,
                deficit_millimeters_per_second: gap
                    .reference_evapotranspiration_millimeters_per_day
                    * crop_coefficient
                    / SECONDS_PER_DAY as f32,
            })
        })
        .collect()
}

fn merged_report_provider_intervals(
    history: &[SprinklerWeatherHistoryPeriodV1],
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
) -> Vec<(LibertasDateTime, LibertasDateTime)> {
    let mut intervals: Vec<_> = history
        .iter()
        .filter_map(|period| {
            let end = period
                .starts_at
                .saturating_add(u64::from(period.duration_seconds))
                .min(ends_before);
            let start = period.starts_at.max(starts_at);
            (start < end).then_some((start, end))
        })
        .collect();
    intervals.sort_by_key(|interval| interval.0);
    let mut merged: Vec<(LibertasDateTime, LibertasDateTime)> = Vec::new();
    for (start, end) in intervals {
        if let Some(last) = merged.last_mut()
            && start <= last.1
        {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    merged
}

fn provider_uncovered_fragments(
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
    provider_intervals: &[(LibertasDateTime, LibertasDateTime)],
    maximum_fragments: usize,
) -> Result<Vec<(LibertasDateTime, LibertasDateTime)>, ()> {
    if starts_at >= ends_before {
        return Ok(Vec::new());
    }
    let first = provider_intervals.partition_point(|interval| interval.1 <= starts_at);
    let mut fragments = Vec::new();
    let mut cursor = starts_at;
    for &(provider_start, provider_end) in &provider_intervals[first..] {
        if provider_start >= ends_before {
            break;
        }
        if cursor < provider_start {
            if fragments.len() >= maximum_fragments {
                return Err(());
            }
            fragments.push((cursor, provider_start.min(ends_before)));
        }
        cursor = cursor.max(provider_end);
        if cursor >= ends_before {
            break;
        }
    }
    if cursor < ends_before {
        if fragments.len() >= maximum_fragments {
            return Err(());
        }
        fragments.push((cursor, ends_before));
    }
    Ok(fragments)
}

fn report_modeled_gap_balance_intervals(
    gaps: &[SprinklerModeledWeatherGapV1],
    provider_intervals: &[(LibertasDateTime, LibertasDateTime)],
    crop_coefficient: f32,
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
    maximum_intervals: usize,
) -> Result<Vec<BalanceRateInterval>, ()> {
    let mut intervals = Vec::new();
    for gap in normalized_modeled_weather_gaps(gaps, starts_at, ends_before) {
        let start = gap.starts_at;
        let end = gap.ends_before;
        let remaining = maximum_intervals.saturating_sub(intervals.len());
        let fragments = provider_uncovered_fragments(start, end, provider_intervals, remaining)?;
        for (starts_at, ends_before) in fragments {
            intervals.push(BalanceRateInterval {
                starts_at,
                ends_before,
                deficit_millimeters_per_second: gap
                    .reference_evapotranspiration_millimeters_per_day
                    * crop_coefficient
                    / SECONDS_PER_DAY as f32,
            });
        }
    }
    Ok(intervals)
}

fn replay_deficit_points(
    opening_deficit_millimeters: f32,
    capacity_millimeters: f32,
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
    intervals: &[BalanceRateInterval],
) -> Vec<(LibertasDateTime, f32)> {
    if starts_at > ends_before {
        return Vec::new();
    }
    let mut rate_changes = Vec::with_capacity(intervals.len().saturating_mul(2));
    for interval in intervals {
        let start = interval.starts_at.max(starts_at);
        let end = interval.ends_before.min(ends_before);
        if start < end {
            let rate = f64::from(interval.deficit_millimeters_per_second);
            rate_changes.push((start, rate, 1_i32));
            rate_changes.push((end, -rate, -1_i32));
        }
    }
    rate_changes.sort_by_key(|change| change.0);
    let mut deficit = opening_deficit_millimeters.clamp(0.0, capacity_millimeters);
    let mut points = Vec::with_capacity(rate_changes.len().saturating_add(2));
    points.push((starts_at, deficit));
    let mut represented_through = starts_at;
    let mut active_rate = 0.0_f64;
    let mut active_intervals = 0_i32;
    let mut change_index = 0;
    while change_index < rate_changes.len() {
        let at = rate_changes[change_index].0;
        if represented_through < at {
            deficit = apply_deficit_delta(
                deficit,
                (active_rate * at.saturating_sub(represented_through) as f64) as f32,
                capacity_millimeters,
            );
            points.push((at, deficit));
            represented_through = at;
        }
        while change_index < rate_changes.len() && rate_changes[change_index].0 == at {
            active_rate += rate_changes[change_index].1;
            active_intervals += rate_changes[change_index].2;
            change_index += 1;
        }
        if active_intervals == 0 {
            active_rate = 0.0;
        }
    }
    if represented_through < ends_before {
        deficit = apply_deficit_delta(
            deficit,
            (active_rate * ends_before.saturating_sub(represented_through) as f64) as f32,
            capacity_millimeters,
        );
        points.push((ends_before, deficit));
    }
    points
}

fn projected_deficit_with_modeled_gaps(
    zone: &SprinklerZoneV1,
    memory: &SprinklerZoneMemoryV1,
    water_events: &[SprinklerWaterEventV1],
    gaps: &[SprinklerModeledWeatherGapV1],
    through: LibertasDateTime,
) -> f32 {
    if through <= memory.balance_baseline_at {
        return memory.baseline_deficit_millimeters;
    }
    let capacity = root_zone_capacity_millimeters(zone);
    let crop_coefficient = plant_profile(zone.plant_type).crop_coefficient;
    let mut intervals = water_event_balance_intervals(
        water_events,
        crop_coefficient,
        memory.balance_baseline_at,
        through,
    );
    intervals.extend(modeled_gap_balance_intervals(
        gaps,
        crop_coefficient,
        memory.balance_baseline_at,
        through,
    ));
    replay_deficit_points(
        memory.baseline_deficit_millimeters,
        capacity,
        memory.balance_baseline_at,
        through,
        &intervals,
    )
    .last()
    .map(|(_, deficit)| *deficit)
    .unwrap_or(memory.baseline_deficit_millimeters)
}

fn projected_deficit_millimeters(
    zone: &SprinklerZoneV1,
    memory: &SprinklerZoneMemoryV1,
    water_events: &[SprinklerWaterEventV1],
    through: LibertasDateTime,
    estimate: WaterDemandEstimate,
) -> f32 {
    let capacity = root_zone_capacity_millimeters(zone);
    let crop_coefficient = plant_profile(zone.plant_type).crop_coefficient;
    let mut deficit = memory.baseline_deficit_millimeters;
    let mut demand_covered_through = memory.balance_baseline_at;

    for event in water_events {
        if event.starts_at() >= through {
            break;
        }
        let Some(event_ends_at) = event.ends_at() else {
            continue;
        };
        if event.starts_at() > demand_covered_through {
            deficit = apply_deficit_delta(
                deficit,
                fallback_demand_millimeters(
                    event.starts_at() - demand_covered_through,
                    crop_coefficient,
                    estimate,
                ),
                capacity,
            );
            demand_covered_through = event.starts_at();
        }
        let represented_start = event.starts_at().max(memory.balance_baseline_at);
        let represented_end = event_ends_at.min(through);
        if represented_start >= represented_end {
            continue;
        }
        let represented_fraction = represented_end.saturating_sub(represented_start) as f32
            / f32::max(event.duration_seconds() as f32, 1.0);
        deficit = apply_deficit_delta(
            deficit,
            event_delta_millimeters(event, crop_coefficient) * represented_fraction,
            capacity,
        );
        if matches!(event, SprinklerWaterEventV1::WeatherV1 { .. }) {
            demand_covered_through = demand_covered_through.max(represented_end);
        }
    }

    if through > demand_covered_through {
        deficit = apply_deficit_delta(
            deficit,
            fallback_demand_millimeters(
                through - demand_covered_through,
                crop_coefficient,
                estimate,
            ),
            capacity,
        );
    }
    deficit
}

fn seconds_until_deficit(
    current_deficit_millimeters: f32,
    target_deficit_millimeters: f32,
    crop_coefficient: f32,
    estimate: WaterDemandEstimate,
) -> u64 {
    let daily_demand = estimate.reference_evapotranspiration_millimeters_per_day * crop_coefficient;
    if !daily_demand.is_finite()
        || daily_demand <= 0.0
        || current_deficit_millimeters >= target_deficit_millimeters
    {
        return 0;
    }
    let seconds = (target_deficit_millimeters - current_deficit_millimeters) / daily_demand
        * SECONDS_PER_DAY as f32;
    if !seconds.is_finite() || seconds >= u64::MAX as f32 {
        return u64::MAX;
    }
    let whole_seconds = seconds as u64;
    whole_seconds.saturating_add(u64::from(whole_seconds as f32 != seconds))
}

fn recent_water_totals(water_events: &[SprinklerWaterEventV1]) -> (f32, f32) {
    water_events.iter().fold(
        (0.0, 0.0),
        |(precipitation, irrigation), event| match event {
            SprinklerWaterEventV1::WeatherV1 {
                precipitation_millimeters,
                ..
            } => (precipitation + precipitation_millimeters, irrigation),
            SprinklerWaterEventV1::IrrigationV1 {
                applied_water_millimeters,
                ..
            } => (precipitation, irrigation + applied_water_millimeters),
        },
    )
}

fn utc_day_start(at: LibertasDateTime) -> LibertasDateTime {
    at - at % SECONDS_PER_DAY
}

fn daily_report_totals(
    water_events: &[SprinklerWaterEventV1],
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
) -> (f32, f32, f32) {
    water_events.iter().fold(
        (0.0_f32, 0.0_f32, 0.0_f32),
        |(rain, reference_et, irrigation), event| {
            let Some(event_ends_at) = event.ends_at() else {
                return (rain, reference_et, irrigation);
            };
            let overlap_start = event.starts_at().max(starts_at);
            let overlap_end = event_ends_at.min(ends_before);
            if overlap_start >= overlap_end {
                return (rain, reference_et, irrigation);
            }
            let fraction = overlap_end.saturating_sub(overlap_start) as f32
                / f32::max(event.duration_seconds() as f32, 1.0);
            match event {
                SprinklerWaterEventV1::WeatherV1 {
                    precipitation_millimeters,
                    reference_evapotranspiration_millimeters,
                    ..
                } => (
                    rain + precipitation_millimeters * fraction,
                    reference_et + reference_evapotranspiration_millimeters * fraction,
                    irrigation,
                ),
                SprinklerWaterEventV1::IrrigationV1 {
                    applied_water_millimeters,
                    ..
                } => (
                    rain,
                    reference_et,
                    irrigation + applied_water_millimeters * fraction,
                ),
            }
        },
    )
}

fn provider_weather_coverage_seconds(
    water_events: &[SprinklerWaterEventV1],
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
) -> u64 {
    let mut intervals: Vec<(LibertasDateTime, LibertasDateTime)> = water_events
        .iter()
        .filter_map(|event| {
            if !matches!(event, SprinklerWaterEventV1::WeatherV1 { .. }) {
                return None;
            }
            let overlap_start = event.starts_at().max(starts_at);
            let overlap_end = event.ends_at()?.min(ends_before);
            (overlap_start < overlap_end).then_some((overlap_start, overlap_end))
        })
        .collect();
    intervals.sort_by_key(|interval| interval.0);
    let mut covered = 0_u64;
    let mut covered_through = starts_at;
    for (start, end) in intervals {
        if end <= covered_through {
            continue;
        }
        let uncovered_start = start.max(covered_through);
        covered = covered.saturating_add(end.saturating_sub(uncovered_start));
        covered_through = end;
    }
    covered
}

fn modeled_gap_summary(
    gaps: &[SprinklerModeledWeatherGapV1],
    starts_at: LibertasDateTime,
    ends_before: LibertasDateTime,
) -> (f32, Option<SprinklerWaterDemandSourceV1>) {
    gaps.iter().fold((0.0_f32, None), |(amount, source), gap| {
        let start = gap.starts_at.max(starts_at);
        let end = gap.ends_before.min(ends_before);
        if start >= end || !valid_modeled_weather_gap(gap) {
            return (amount, source);
        }
        (
            amount
                + gap.reference_evapotranspiration_millimeters_per_day
                    * end.saturating_sub(start) as f32
                    / SECONDS_PER_DAY as f32,
            source.or(Some(gap.demand_source)),
        )
    })
}

fn build_daily_reports_from_ledger(
    configuration: &SprinklerZoneV1,
    memory: &SprinklerZoneMemoryV1,
    water_events: &[SprinklerWaterEventV1],
    modeled_weather_gaps: &[SprinklerModeledWeatherGapV1],
    now: LibertasDateTime,
    maximum: usize,
) -> Vec<SprinklerDailyReportV1> {
    let first_event = water_events
        .first()
        .map(SprinklerWaterEventV1::starts_at)
        .unwrap_or(memory.balance_baseline_at);
    let first_day = utc_day_start(first_event.max(memory.balance_baseline_at));
    let last_day = utc_day_start(now);
    if first_day > last_day {
        return Vec::new();
    }
    let capacity = root_zone_capacity_millimeters(configuration);
    let mut reports = Vec::new();
    let earliest_bounded_day =
        last_day.saturating_sub((maximum.saturating_sub(1) as u64).saturating_mul(SECONDS_PER_DAY));
    let mut day = first_day.max(earliest_bounded_day);
    while day <= last_day && reports.len() < maximum {
        let ends_before = day.saturating_add(SECONDS_PER_DAY);
        let represented_start = day.max(memory.balance_baseline_at);
        let represented_end = ends_before.min(now);
        if represented_start >= represented_end {
            break;
        }
        let opening_deficit_millimeters = projected_deficit_with_modeled_gaps(
            configuration,
            memory,
            water_events,
            modeled_weather_gaps,
            represented_start,
        );
        let closing_deficit_millimeters = projected_deficit_with_modeled_gaps(
            configuration,
            memory,
            water_events,
            modeled_weather_gaps,
            represented_end,
        );
        let (
            precipitation_millimeters,
            reference_evapotranspiration_millimeters,
            irrigation_millimeters,
        ) = daily_report_totals(water_events, represented_start, represented_end);
        let represented_seconds = represented_end.saturating_sub(represented_start);
        let provider_coverage_seconds =
            provider_weather_coverage_seconds(water_events, represented_start, represented_end)
                .min(represented_seconds);
        let (modeled_reference_evapotranspiration_millimeters, modeled_demand_source) =
            modeled_gap_summary(modeled_weather_gaps, represented_start, represented_end);
        reports.push(SprinklerDailyReportV1 {
            starts_at: day,
            ends_before,
            coverage_starts_at: represented_start,
            coverage_ends_before: represented_end,
            capacity_millimeters: capacity,
            opening_deficit_millimeters,
            closing_deficit_millimeters,
            precipitation_millimeters,
            reference_evapotranspiration_millimeters,
            modeled_reference_evapotranspiration_millimeters,
            modeled_demand_source,
            provider_weather_coverage_seconds: u32::try_from(provider_coverage_seconds)
                .unwrap_or(u32::MAX),
            irrigation_millimeters,
            complete: represented_start == day && represented_end == ends_before,
            calculated_at: now,
        });
        let next = day.saturating_add(SECONDS_PER_DAY);
        if next <= day {
            break;
        }
        day = next;
    }
    reports
        .into_iter()
        // A day whose opening state predates the folded controller baseline
        // cannot be rebuilt honestly from the bounded ledger. Preserve its
        // previously finalized archive record instead of overwriting it with a
        // partial reconstruction.
        .filter(|report| report.starts_at >= memory.balance_baseline_at || !report.complete)
        .collect()
}

fn build_daily_reports(
    zone: &ZoneRuntime,
    _site_location: Option<SprinklerWeatherLocationV1>,
    now: LibertasDateTime,
) -> Vec<SprinklerDailyReportV1> {
    build_daily_reports_from_ledger(
        &zone.configuration,
        &zone.memory,
        &zone.water_events,
        &zone.modeled_weather_gaps,
        now,
        MAX_REPORT_DAILY_RECORDS_PER_ZONE,
    )
}

fn synchronize_history(
    memory: &SprinklerZoneMemoryV1,
    water_events: &mut Vec<SprinklerWaterEventV1>,
    history: Option<&SprinklerWeatherHistoryV2>,
) -> bool {
    let Some(history) = history else {
        return false;
    };
    let before = water_events.clone();
    water_events.retain(|event| matches!(event, SprinklerWaterEventV1::IrrigationV1 { .. }));
    water_events.extend(
        history
            .periods
            .iter()
            .filter(|period| {
                period
                    .starts_at
                    .saturating_add(u64::from(period.duration_seconds))
                    > memory.balance_baseline_at
            })
            .map(|period| SprinklerWaterEventV1::WeatherV1 {
                starts_at: period.starts_at,
                duration_seconds: period.duration_seconds,
                precipitation_millimeters: period.precipitation_millimeters,
                reference_evapotranspiration_millimeters: period
                    .reference_evapotranspiration_millimeters,
            }),
    );
    sort_water_events(water_events);
    *water_events != before
}

fn queue_finalized_daily_reports(zone: &mut ZoneRuntime, reports: Vec<SprinklerDailyReportV1>) {
    for report in reports {
        if let Some(saved) = zone
            .finalized_daily_reports
            .iter_mut()
            .find(|saved| saved.starts_at == report.starts_at)
        {
            *saved = report;
        } else {
            zone.finalized_daily_reports.push(report);
        }
    }
    zone.finalized_daily_reports
        .sort_by_key(|report| report.starts_at);
}

fn add_irrigation_event(
    zone: &mut ZoneRuntime,
    starts_at: LibertasDateTime,
    duration_seconds: u32,
    site_location: Option<SprinklerWeatherLocationV1>,
    now: LibertasDateTime,
) {
    if duration_seconds == 0 {
        return;
    }
    let applied_water_millimeters =
        nominal_delivery_millimeters_per_hour(zone.configuration.sprinkler_head_type)
            * duration_seconds as f32
            / 3_600.0;
    if !valid_nonnegative(applied_water_millimeters) {
        return;
    }
    let watering_percent = zone.memory.watering_percent;
    if let Some(SprinklerWaterEventV1::IrrigationV1 {
        starts_at: previous_starts_at,
        duration_seconds: previous_duration_seconds,
        watering_percent: previous_watering_percent,
        applied_water_millimeters: previous_applied_water_millimeters,
    }) = zone
        .water_events
        .iter_mut()
        .rev()
        .find(|event| matches!(event, SprinklerWaterEventV1::IrrigationV1 { .. }))
        && previous_starts_at.checked_add(u64::from(*previous_duration_seconds)) == Some(starts_at)
        && *previous_watering_percent == watering_percent
        && let Some(merged_duration_seconds) =
            previous_duration_seconds.checked_add(duration_seconds)
    {
        *previous_duration_seconds = merged_duration_seconds;
        *previous_applied_water_millimeters += applied_water_millimeters;
        let finalized_reports = prune_water_events(
            &mut zone.memory,
            &mut zone.water_events,
            &mut zone.modeled_weather_gaps,
            &zone.configuration,
            site_location,
            now,
        );
        queue_finalized_daily_reports(zone, finalized_reports);
        return;
    }
    zone.water_events.push(SprinklerWaterEventV1::IrrigationV1 {
        starts_at,
        duration_seconds,
        watering_percent,
        applied_water_millimeters,
    });
    let finalized_reports = prune_water_events(
        &mut zone.memory,
        &mut zone.water_events,
        &mut zone.modeled_weather_gaps,
        &zone.configuration,
        site_location,
        now,
    );
    queue_finalized_daily_reports(zone, finalized_reports);
}

fn begin_expected_irrigation(
    zone: &mut ZoneRuntime,
    starts_at: LibertasDateTime,
    duration_seconds: u32,
) -> bool {
    let activity_identity = zone
        .current_activity
        .as_ref()
        .filter(|activity| {
            activity.origin == SprinklerWateringOriginV1::Automatic
                && activity.outcome == SprinklerWateringOutcomeV1::Scheduled
        })
        .map(|activity| (activity.activity_index, activity.activity_ordinal))
        .or_else(|| {
            allocate_watering_activity_index(
                zone.configuration.valve,
                starts_at,
                SprinklerWateringOriginV1::Automatic,
            )
        });
    let Some((activity_index, activity_ordinal)) = activity_identity else {
        return false;
    };
    if duration_seconds == 0 || zone.expected_irrigation.is_some() {
        return false;
    }
    zone.expected_irrigation = Some(ExpectedIrrigation {
        starts_at,
        activity_index,
        activity_ordinal,
    });
    true
}

fn discard_expected_irrigation(zone: &mut ZoneRuntime) -> bool {
    zone.expected_irrigation.take().is_some()
}

fn reconcile_expected_irrigation(zone: &mut ZoneRuntime, _now_ticks: u64) -> bool {
    discard_expected_irrigation(zone)
}

fn safe_wind_limits(head: SprinklerHeadTypeV1) -> (f32, f32) {
    match head {
        SprinklerHeadTypeV1::SurfaceDrip => (15.0, 25.0),
        SprinklerHeadTypeV1::Bubblers => (12.0, 20.0),
        SprinklerHeadTypeV1::PopupSpray
        | SprinklerHeadTypeV1::RotorsLowRate
        | SprinklerHeadTypeV1::RotorsHighRate => (
            SAFE_MAXIMUM_WIND_METERS_PER_SECOND,
            SAFE_MAXIMUM_GUST_METERS_PER_SECOND,
        ),
    }
}

fn current_is_safe(
    current: &SprinklerCurrentWeatherV1,
    now: LibertasDateTime,
    head: SprinklerHeadTypeV1,
) -> bool {
    let (maximum_wind, maximum_gust) = safe_wind_limits(head);
    current.is_fresh_at(now)
        && current.temperature_celsius.is_finite()
        && current.temperature_celsius > SAFE_MINIMUM_TEMPERATURE_CELSIUS
        && current.relative_humidity_percent <= 100
        && current.precipitation_millimeters == 0.0
        && current.wind_speed_meters_per_second.is_finite()
        && current.wind_speed_meters_per_second <= maximum_wind
        && current.wind_gust_meters_per_second.is_finite()
        && current.wind_gust_meters_per_second <= maximum_gust
}

fn forecast_period_is_safe(
    period: &SprinklerWeatherForecastPeriodV1,
    head: SprinklerHeadTypeV1,
) -> bool {
    let (maximum_wind, maximum_gust) = safe_wind_limits(head);
    period.temperature_celsius.is_finite()
        && period.temperature_celsius > SAFE_MINIMUM_TEMPERATURE_CELSIUS
        && period.relative_humidity_percent <= 100
        && period.precipitation_probability_percent < HIGH_RAIN_PROBABILITY_PERCENT
        && period.expected_precipitation_millimeters <= 0.1
        && period.wind_speed_meters_per_second.is_finite()
        && period.wind_speed_meters_per_second <= maximum_wind
        && period.wind_gust_meters_per_second.is_finite()
        && period.wind_gust_meters_per_second <= maximum_gust
}

fn forecast_rain_delay(
    forecast: Option<&SprinklerWeatherForecastV1>,
    now: LibertasDateTime,
    required_water_millimeters: f32,
) -> Option<LibertasDateTime> {
    let forecast = forecast?;
    let lookahead_end = now.saturating_add(FORECAST_LOOKAHEAD_SECONDS);
    let mut weighted_rain = 0.0;
    let mut rainy_until = now;
    for period in forecast
        .periods
        .iter()
        .filter(|period| period.starts_at >= now && period.starts_at < lookahead_end)
    {
        if period.expected_precipitation_millimeters.is_finite() {
            weighted_rain += period.expected_precipitation_millimeters
                * f32::from(period.precipitation_probability_percent)
                / 100.0;
        }
        if period.precipitation_probability_percent >= HIGH_RAIN_PROBABILITY_PERCENT {
            rainy_until = rainy_until.max(
                period
                    .starts_at
                    .saturating_add(u64::from(period.duration_seconds)),
            );
        }
    }
    (weighted_rain >= required_water_millimeters * 0.5).then_some(rainy_until)
}

fn weighted_forecast_rain_between(
    forecast: &SprinklerWeatherForecastV1,
    starts_at: LibertasDateTime,
    ends_at: LibertasDateTime,
) -> Option<f32> {
    if ends_at <= starts_at {
        return Some(0.0);
    }
    let mut covered_through = starts_at;
    let mut weighted_rain = 0.0_f32;
    for period in &forecast.periods {
        let period_ends_at = period
            .starts_at
            .checked_add(u64::from(period.duration_seconds))?;
        if period_ends_at <= covered_through {
            continue;
        }
        if period.starts_at > covered_through {
            return None;
        }
        let overlap_ends_at = period_ends_at.min(ends_at);
        let overlap_seconds = overlap_ends_at.saturating_sub(covered_through);
        if overlap_seconds == 0 {
            continue;
        }
        let overlap_fraction = overlap_seconds as f32 / period.duration_seconds as f32;
        weighted_rain += period.expected_precipitation_millimeters
            * f32::from(period.precipitation_probability_percent)
            / 100.0
            * overlap_fraction;
        if !weighted_rain.is_finite() {
            return None;
        }
        covered_through = overlap_ends_at;
        if covered_through >= ends_at {
            return Some(weighted_rain);
        }
    }
    None
}

fn next_safe_forecast_start(
    forecast: Option<&SprinklerWeatherForecastV1>,
    not_before: LibertasDateTime,
    head: SprinklerHeadTypeV1,
) -> Option<LibertasDateTime> {
    forecast?
        .periods
        .iter()
        .find(|period| period.starts_at >= not_before && forecast_period_is_safe(period, head))
        .map(|period| period.starts_at)
}

fn shift_after_hold_offs(
    mut starts_at: LibertasDateTime,
    duration_seconds: u32,
    hold_offs: &[SprinklerTimeSlotV1],
) -> (LibertasDateTime, bool) {
    let mut shifted = false;
    for _ in 0..=hold_offs.len() {
        let candidate = SprinklerTimeSlotV1 {
            starts_at,
            duration_seconds,
        };
        if candidate.ends_at().is_none() {
            return (LibertasDateTime::MAX, shifted);
        }
        let Some(hold_off) = hold_offs
            .iter()
            .copied()
            .find(|hold_off| candidate.overlaps(*hold_off))
        else {
            return (starts_at, shifted);
        };
        starts_at = hold_off.ends_at().unwrap_or(LibertasDateTime::MAX);
        shifted = true;
    }
    (LibertasDateTime::MAX, shifted)
}

fn watering_duration_seconds(zone: &SprinklerZoneV1, water_millimeters: f32) -> u32 {
    let delivery_rate = nominal_delivery_millimeters_per_hour(zone.sprinkler_head_type);
    let seconds = water_millimeters / delivery_rate * 3_600.0;
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    if seconds >= MAX_WATERING_DURATION_SECONDS as f32 {
        return MAX_WATERING_DURATION_SECONDS;
    }
    let whole_seconds = seconds as u32;
    let rounded_up = whole_seconds + u32::from((whole_seconds as f32) < seconds);
    rounded_up.clamp(MIN_WATERING_DURATION_SECONDS, MAX_WATERING_DURATION_SECONDS)
}

fn planned_water_millimeters(
    capacity_millimeters: f32,
    planning_deficit_millimeters: f32,
    watering_percent: u16,
) -> f32 {
    let replenishment =
        (planning_deficit_millimeters - capacity_millimeters * REPLENISHED_DEFICIT_RATIO).max(0.0);
    let multiplier = f32::from(watering_percent) / 100.0;
    (replenishment * multiplier).clamp(0.0, capacity_millimeters)
}

#[derive(Clone, Copy)]
struct SlotForecast {
    temperature_celsius: f32,
    relative_humidity_percent: f32,
    precipitation_probability_percent: u8,
    expected_precipitation_millimeters: f32,
    reference_evapotranspiration_millimeters: f32,
    maximum_wind_meters_per_second: f32,
    maximum_gust_meters_per_second: f32,
}

fn forecast_for_slot(
    forecast: &SprinklerWeatherForecastV1,
    starts_at: LibertasDateTime,
    duration_seconds: u32,
    head: SprinklerHeadTypeV1,
) -> Option<SlotForecast> {
    let ends_at = starts_at.checked_add(u64::from(duration_seconds))?;
    if duration_seconds == 0 {
        return None;
    }

    let mut covered_through = starts_at;
    let mut covered_seconds = 0_u64;
    let mut weighted_temperature = 0.0_f32;
    let mut weighted_humidity = 0.0_f32;
    let mut probability = 0_u8;
    let mut expected_precipitation = 0.0_f32;
    let mut reference_evapotranspiration = 0.0_f32;
    let mut maximum_wind = 0.0_f32;
    let mut maximum_gust = 0.0_f32;

    for period in &forecast.periods {
        let period_ends_at = period
            .starts_at
            .checked_add(u64::from(period.duration_seconds))?;
        if period_ends_at <= covered_through {
            continue;
        }
        if period.starts_at >= ends_at {
            break;
        }
        if period.starts_at > covered_through || !forecast_period_is_safe(period, head) {
            return None;
        }

        let overlap_starts_at = covered_through.max(period.starts_at);
        let overlap_ends_at = ends_at.min(period_ends_at);
        let overlap_seconds = overlap_ends_at.saturating_sub(overlap_starts_at);
        if overlap_seconds == 0 {
            continue;
        }
        let overlap_fraction = overlap_seconds as f32 / period.duration_seconds as f32;
        weighted_temperature += period.temperature_celsius * overlap_seconds as f32;
        weighted_humidity += f32::from(period.relative_humidity_percent) * overlap_seconds as f32;
        probability = probability.max(period.precipitation_probability_percent);
        expected_precipitation += period.expected_precipitation_millimeters * overlap_fraction;
        reference_evapotranspiration +=
            period.reference_evapotranspiration_millimeters * overlap_fraction;
        maximum_wind = maximum_wind.max(period.wind_speed_meters_per_second);
        maximum_gust = maximum_gust.max(period.wind_gust_meters_per_second);
        covered_seconds = covered_seconds.saturating_add(overlap_seconds);
        covered_through = overlap_ends_at;
        if covered_through >= ends_at {
            break;
        }
    }

    if covered_through < ends_at || covered_seconds != u64::from(duration_seconds) {
        return None;
    }
    Some(SlotForecast {
        temperature_celsius: weighted_temperature / covered_seconds as f32,
        relative_humidity_percent: weighted_humidity / covered_seconds as f32,
        precipitation_probability_percent: probability,
        expected_precipitation_millimeters: expected_precipitation,
        reference_evapotranspiration_millimeters: reference_evapotranspiration,
        maximum_wind_meters_per_second: maximum_wind,
        maximum_gust_meters_per_second: maximum_gust,
    })
}

#[derive(Clone, Copy)]
struct MorningCandidate {
    starts_at: LibertasDateTime,
    penalty: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WateringPlan {
    starts_at: LibertasDateTime,
    planning_deficit_millimeters: f32,
    planned_water_millimeters: f32,
    duration_seconds: u32,
}

struct CandidateConditions {
    starts_at: LibertasDateTime,
    duration_seconds: u32,
    planning_deficit_millimeters: f32,
    solar: SolarPosition,
    slot_weather: SlotForecast,
}

struct MorningSearch<'a> {
    forecast: Option<&'a SprinklerWeatherForecastV1>,
    location: Option<SprinklerWeatherLocationV1>,
    now: LibertasDateTime,
    current_deficit_millimeters: f32,
    capacity_millimeters: f32,
    crop_coefficient: f32,
    demand_estimate: WaterDemandEstimate,
}

fn align_candidate_time(at: LibertasDateTime) -> LibertasDateTime {
    let remainder = at % SCHEDULE_CANDIDATE_INTERVAL_SECONDS;
    if remainder == 0 {
        at
    } else {
        at.saturating_add(SCHEDULE_CANDIDATE_INTERVAL_SECONDS - remainder)
    }
}

fn candidate_penalty(
    zone: &SprinklerZoneV1,
    capacity_millimeters: f32,
    location: SprinklerWeatherLocationV1,
    candidate: CandidateConditions,
) -> Option<f64> {
    let CandidateConditions {
        starts_at,
        duration_seconds,
        planning_deficit_millimeters,
        solar,
        slot_weather,
    } = candidate;
    let end_solar = solar_position(
        location,
        starts_at.checked_add(u64::from(duration_seconds))?,
    )?;
    let profile = plant_profile(zone.plant_type);
    let exposes_foliage = head_exposes_foliage(zone.sprinkler_head_type);
    let humidity = f64::from(slot_weather.relative_humidity_percent);
    let solar_target = if exposes_foliage && humidity >= f64::from(HIGH_HUMIDITY_PERCENT) {
        HIGH_HUMIDITY_TARGET_SOLAR_ELEVATION_DEGREES
    } else {
        TARGET_SOLAR_ELEVATION_DEGREES
    };
    let deficit_ratio = f64::from(planning_deficit_millimeters / capacity_millimeters);
    let deficit_penalty =
        (deficit_ratio - f64::from(TARGET_DEFICIT_RATIO)).abs() * DEFICIT_PENALTY_WEIGHT;
    let solar_penalty = (solar.elevation_degrees - solar_target).abs()
        * if exposes_foliage {
            OVERHEAD_SOLAR_PENALTY_WEIGHT
        } else {
            NON_OVERHEAD_SOLAR_PENALTY_WEIGHT
        };
    let evaporation_penalty = f64::from(slot_weather.reference_evapotranspiration_millimeters)
        * EVAPOTRANSPIRATION_PENALTY_WEIGHT;
    let wind_penalty = f64::from(head_wind_sensitivity(zone.sprinkler_head_type))
        * f64::from(
            slot_weather.maximum_wind_meters_per_second * 2.0
                + slot_weather.maximum_gust_meters_per_second,
        );
    let rain_penalty = f64::from(slot_weather.precipitation_probability_percent)
        * RAIN_PROBABILITY_PENALTY_WEIGHT
        + f64::from(slot_weather.expected_precipitation_millimeters) * RAIN_AMOUNT_PENALTY_WEIGHT;
    let heat_penalty =
        f64::from((slot_weather.temperature_celsius - HEAT_PENALTY_START_CELSIUS).max(0.0))
            * HEAT_PENALTY_WEIGHT;
    let humidity_excess = (humidity - f64::from(HIGH_HUMIDITY_PERCENT)).max(0.0);
    let predawn_darkness = (-solar.elevation_degrees / 6.0).clamp(0.0, 1.0);
    let foliage_wetness_penalty = if exposes_foliage {
        humidity_excess
            * predawn_darkness
            * f64::from(profile.foliage_wetness_sensitivity)
            * FOLIAGE_WETNESS_PENALTY_WEIGHT
    } else {
        0.0
    };
    let bright_finish_penalty = if exposes_foliage {
        (end_solar.elevation_degrees - BRIGHT_FINISH_SOLAR_ELEVATION_DEGREES).max(0.0)
            * BRIGHT_FINISH_PENALTY_WEIGHT
    } else {
        0.0
    };
    let penalty = deficit_penalty
        + solar_penalty
        + evaporation_penalty
        + wind_penalty
        + rain_penalty
        + heat_penalty
        + foliage_wetness_penalty
        + bright_finish_penalty;
    penalty.is_finite().then_some(penalty)
}

fn optimized_morning_candidate(
    zone: &ZoneRuntime,
    search: MorningSearch<'_>,
) -> Option<MorningCandidate> {
    let MorningSearch {
        forecast,
        location,
        now,
        current_deficit_millimeters,
        capacity_millimeters,
        crop_coefficient,
        demand_estimate,
    } = search;
    let location = location.filter(|location| valid_site_location(*location))?;
    let forecast = forecast.filter(|forecast| forecast.is_fresh_at(now))?;
    let critical_deficit = capacity_millimeters * CRITICAL_DEFICIT_RATIO;
    if current_deficit_millimeters >= critical_deficit {
        return None;
    }

    let preferred_deficit = capacity_millimeters * PREFERRED_DEFICIT_RATIO;
    let search_starts_at = now.saturating_add(seconds_until_deficit(
        current_deficit_millimeters,
        preferred_deficit,
        crop_coefficient,
        demand_estimate,
    ));
    let critical_at = now.saturating_add(seconds_until_deficit(
        current_deficit_millimeters,
        critical_deficit,
        crop_coefficient,
        demand_estimate,
    ));
    let forecast_ends_at = forecast
        .periods
        .iter()
        .filter_map(|period| {
            period
                .starts_at
                .checked_add(u64::from(period.duration_seconds))
        })
        .max()?;
    let search_ends_at = critical_at
        .min(forecast_ends_at)
        .min(now.saturating_add(u64::from(SPRINKLER_FORECAST_HORIZON_SECONDS)));
    let (minimum_solar, maximum_solar) =
        preferred_solar_elevation_range(zone.configuration.sprinkler_head_type);
    let mut candidate_at = align_candidate_time(search_starts_at.max(now));
    let mut best_preferred: Option<MorningCandidate> = None;
    let mut best_rising: Option<MorningCandidate> = None;

    while candidate_at <= search_ends_at {
        let planning_deficit = projected_deficit_millimeters(
            &zone.configuration,
            &zone.memory,
            &zone.water_events,
            candidate_at,
            demand_estimate,
        );
        let planned_water = planned_water_millimeters(
            capacity_millimeters,
            planning_deficit,
            zone.memory.watering_percent,
        );
        let duration = watering_duration_seconds(&zone.configuration, planned_water)
            .max(MIN_WATERING_DURATION_SECONDS);
        let solar = solar_position(location, candidate_at);
        let weather = forecast_for_slot(
            forecast,
            candidate_at,
            duration,
            zone.configuration.sprinkler_head_type,
        );
        if let (Some(solar), Some(weather)) = (solar, weather)
            && solar.rising
            && let Some(penalty) = candidate_penalty(
                &zone.configuration,
                capacity_millimeters,
                location,
                CandidateConditions {
                    starts_at: candidate_at,
                    duration_seconds: duration,
                    planning_deficit_millimeters: planning_deficit,
                    solar,
                    slot_weather: weather,
                },
            )
        {
            let candidate = MorningCandidate {
                starts_at: candidate_at,
                penalty,
            };
            if best_rising.is_none_or(|best| penalty < best.penalty) {
                best_rising = Some(candidate);
            }
            if (minimum_solar..=maximum_solar).contains(&solar.elevation_degrees)
                && best_preferred.is_none_or(|best| penalty < best.penalty)
            {
                best_preferred = Some(candidate);
            }
        }
        let next = candidate_at.saturating_add(SCHEDULE_CANDIDATE_INTERVAL_SECONDS);
        if next <= candidate_at {
            break;
        }
        candidate_at = next;
    }
    // At extreme latitudes the sun may never enter the normal dawn elevation
    // band. In that case use the lowest-penalty rising-sun slot instead of
    // reverting to an arbitrary wall-clock time.
    best_preferred.or(best_rising)
}

fn watering_plan_at(
    zone: &ZoneRuntime,
    starts_at: LibertasDateTime,
    capacity_millimeters: f32,
    demand_estimate: WaterDemandEstimate,
) -> WateringPlan {
    let planning_deficit_millimeters = projected_deficit_millimeters(
        &zone.configuration,
        &zone.memory,
        &zone.water_events,
        starts_at,
        demand_estimate,
    );
    let planned_water_millimeters = planned_water_millimeters(
        capacity_millimeters,
        planning_deficit_millimeters,
        zone.memory.watering_percent,
    );
    let duration_seconds =
        watering_duration_seconds(&zone.configuration, planned_water_millimeters)
            .max(MIN_WATERING_DURATION_SECONDS);
    WateringPlan {
        starts_at,
        planning_deficit_millimeters,
        planned_water_millimeters,
        duration_seconds,
    }
}

fn first_post_hold_off_plan(
    zone: &ZoneRuntime,
    starts_at: LibertasDateTime,
    capacity_millimeters: f32,
    demand_estimate: WaterDemandEstimate,
    hold_offs: &[SprinklerTimeSlotV1],
) -> (WateringPlan, Option<SprinklerTimeSlotV1>) {
    let mut candidate_at = starts_at;
    let mut first_blocking_hold_off = None;
    for _ in 0..=hold_offs.len() {
        let plan = watering_plan_at(zone, candidate_at, capacity_millimeters, demand_estimate);
        let slot = SprinklerTimeSlotV1 {
            starts_at: plan.starts_at,
            duration_seconds: plan.duration_seconds,
        };
        let Some(blocking_hold_off) = hold_offs
            .iter()
            .copied()
            .find(|hold_off| slot.overlaps(*hold_off))
        else {
            return (plan, first_blocking_hold_off);
        };
        first_blocking_hold_off.get_or_insert(blocking_hold_off);
        candidate_at = blocking_hold_off.ends_at().unwrap_or(LibertasDateTime::MAX);
    }
    (
        watering_plan_at(
            zone,
            LibertasDateTime::MAX,
            capacity_millimeters,
            demand_estimate,
        ),
        first_blocking_hold_off,
    )
}

struct PreemptiveHoldOffSearch<'a> {
    weather: &'a SprinklerWeatherSnapshotV2,
    location: Option<SprinklerWeatherLocationV1>,
    hold_offs: &'a [SprinklerTimeSlotV1],
    now: LibertasDateTime,
    current_deficit_millimeters: f32,
    capacity_millimeters: f32,
    crop_coefficient: f32,
    demand_estimate: WaterDemandEstimate,
}

fn best_preemptive_hold_off_plan(
    zone: &ZoneRuntime,
    blocking_hold_off: SprinklerTimeSlotV1,
    post_hold_off_plan: WateringPlan,
    search: PreemptiveHoldOffSearch<'_>,
) -> Option<WateringPlan> {
    let PreemptiveHoldOffSearch {
        weather,
        location,
        hold_offs,
        now,
        current_deficit_millimeters,
        capacity_millimeters,
        crop_coefficient,
        demand_estimate,
    } = search;
    let forecast = weather
        .forecast
        .as_ref()
        .filter(|forecast| forecast.is_fresh_at(now))?;
    let location = location.filter(|location| valid_site_location(*location))?;
    let critical_deficit = capacity_millimeters * CRITICAL_DEFICIT_RATIO;
    if post_hold_off_plan.planning_deficit_millimeters < critical_deficit {
        return None;
    }
    if post_hold_off_plan.starts_at
        > now.saturating_add(u64::from(SPRINKLER_FORECAST_HORIZON_SECONDS))
    {
        return None;
    }
    let weighted_rain =
        weighted_forecast_rain_between(forecast, now, post_hold_off_plan.starts_at)?;

    let preferred_deficit = capacity_millimeters * PREFERRED_DEFICIT_RATIO;
    let search_starts_at = now.saturating_add(seconds_until_deficit(
        current_deficit_millimeters,
        preferred_deficit,
        crop_coefficient,
        demand_estimate,
    ));
    let mut candidate_at = align_candidate_time(search_starts_at.max(now));
    let (minimum_solar, maximum_solar) =
        preferred_solar_elevation_range(zone.configuration.sprinkler_head_type);
    let mut best_preferred: Option<(WateringPlan, f64)> = None;
    let mut best_safe: Option<(WateringPlan, f64)> = None;

    while candidate_at < blocking_hold_off.starts_at {
        let plan = watering_plan_at(zone, candidate_at, capacity_millimeters, demand_estimate);
        let slot = SprinklerTimeSlotV1 {
            starts_at: plan.starts_at,
            duration_seconds: plan.duration_seconds,
        };
        let current_is_safe_for_slot = candidate_at > now
            || weather.current.as_ref().is_some_and(|current| {
                current_is_safe(current, now, zone.configuration.sprinkler_head_type)
            });
        if plan.planning_deficit_millimeters >= preferred_deficit
            && slot
                .ends_at()
                .is_some_and(|ends_at| ends_at <= blocking_hold_off.starts_at)
            && !hold_offs
                .iter()
                .copied()
                .any(|hold_off| slot.overlaps(hold_off))
            && current_is_safe_for_slot
            && let Some(solar) = solar_position(location, candidate_at)
            && let Some(slot_weather) = forecast_for_slot(
                forecast,
                candidate_at,
                plan.duration_seconds,
                zone.configuration.sprinkler_head_type,
            )
            && let Some(penalty) = candidate_penalty(
                &zone.configuration,
                capacity_millimeters,
                location,
                CandidateConditions {
                    starts_at: candidate_at,
                    duration_seconds: plan.duration_seconds,
                    planning_deficit_millimeters: plan.planning_deficit_millimeters,
                    solar,
                    slot_weather,
                },
            )
        {
            if best_safe.is_none_or(|(_, best_penalty)| penalty < best_penalty) {
                best_safe = Some((plan, penalty));
            }
            if solar.rising
                && (minimum_solar..=maximum_solar).contains(&solar.elevation_degrees)
                && best_preferred.is_none_or(|(_, best_penalty)| penalty < best_penalty)
            {
                best_preferred = Some((plan, penalty));
            }
        }
        let next = candidate_at.saturating_add(SCHEDULE_CANDIDATE_INTERVAL_SECONDS);
        if next <= candidate_at {
            break;
        }
        candidate_at = next;
    }

    let plan = best_preferred.or(best_safe)?.0;
    (weighted_rain < plan.planned_water_millimeters * 0.5).then_some(plan)
}

fn calculate_active_state(
    zone: &ZoneRuntime,
    weather: &SprinklerWeatherSnapshotV2,
    weather_stream_ready: bool,
    site_location: Option<SprinklerWeatherLocationV1>,
    now: LibertasDateTime,
) -> SprinklerZoneActiveStateV1 {
    let demand_estimate = water_demand_estimate(&zone.water_events, site_location, now);
    let deficit = projected_deficit_millimeters(
        &zone.configuration,
        &zone.memory,
        &zone.water_events,
        now,
        demand_estimate,
    );
    let capacity = root_zone_capacity_millimeters(&zone.configuration);
    let crop_coefficient = plant_profile(zone.configuration.plant_type).crop_coefficient;
    let (recent_precipitation, recent_irrigation) = recent_water_totals(&zone.water_events);
    let active_hold_offs: Vec<_> = zone
        .memory
        .hold_off_periods
        .iter()
        .copied()
        .filter(|hold_off| hold_off.ends_at().is_some_and(|ends_at| ends_at > now))
        .collect();
    let base = |condition, next_watering, planned_water_millimeters| SprinklerZoneActiveStateV1 {
        water_demand_source: demand_estimate.source,
        estimated_reference_evapotranspiration_millimeters_per_day: demand_estimate
            .reference_evapotranspiration_millimeters_per_day,
        calculated_at: now,
        condition,
        next_watering,
        planned_water_millimeters,
        estimated_deficit_millimeters: deficit,
        recent_precipitation_millimeters: recent_precipitation,
        recent_irrigation_millimeters: recent_irrigation,
        valve_is_open: zone.valve_is_open,
        valve_state_known: zone.valve_state_known,
        valve_fault_bitmap: zone.valve_fault_bitmap,
    };

    let trigger_deficit = capacity * TARGET_DEFICIT_RATIO;
    let mut candidate = if deficit < trigger_deficit {
        now.saturating_add(seconds_until_deficit(
            deficit,
            trigger_deficit,
            crop_coefficient,
            demand_estimate,
        ))
    } else {
        now
    };
    if let Some(optimized) = optimized_morning_candidate(
        zone,
        MorningSearch {
            forecast: weather.forecast.as_ref(),
            location: site_location,
            now,
            current_deficit_millimeters: deficit,
            capacity_millimeters: capacity,
            crop_coefficient,
            demand_estimate,
        },
    ) {
        candidate = optimized.starts_at;
    }
    let mut plan = watering_plan_at(zone, candidate, capacity, demand_estimate);

    let live_weather_ready = weather_stream_ready
        && weather
            .history
            .as_ref()
            .is_some_and(|history| history.is_fresh_at(now));
    let mut condition = if live_weather_ready {
        SprinklerScheduleConditionV1::Scheduled
    } else {
        SprinklerScheduleConditionV1::OfflineWeatherEstimate
    };

    if candidate <= now {
        let fresh_current = weather
            .current
            .as_ref()
            .filter(|current| current.is_fresh_at(now));
        if let Some(rainy_until) = weather
            .forecast
            .as_ref()
            .filter(|forecast| forecast.is_fresh_at(now))
            .and_then(|forecast| {
                forecast_rain_delay(Some(forecast), now, plan.planned_water_millimeters)
            })
        {
            candidate = next_safe_forecast_start(
                weather.forecast.as_ref(),
                rainy_until,
                zone.configuration.sprinkler_head_type,
            )
            .unwrap_or(rainy_until);
            condition = SprinklerScheduleConditionV1::ForecastRain;
        } else if fresh_current.is_some_and(|current| {
            !current_is_safe(current, now, zone.configuration.sprinkler_head_type)
        }) {
            candidate = next_safe_forecast_start(
                weather.forecast.as_ref(),
                now,
                zone.configuration.sprinkler_head_type,
            )
            .unwrap_or_else(|| now.saturating_add(UNSAFE_WEATHER_RETRY_SECONDS));
            condition = SprinklerScheduleConditionV1::WaitingForSafeWeather;
        }
    }

    let (post_hold_off_plan, blocking_hold_off) = first_post_hold_off_plan(
        zone,
        candidate,
        capacity,
        demand_estimate,
        &active_hold_offs,
    );
    plan = post_hold_off_plan;
    if let Some(blocking_hold_off) = blocking_hold_off {
        if let Some(preemptive_plan) = best_preemptive_hold_off_plan(
            zone,
            blocking_hold_off,
            post_hold_off_plan,
            PreemptiveHoldOffSearch {
                weather,
                location: site_location,
                hold_offs: &active_hold_offs,
                now,
                current_deficit_millimeters: deficit,
                capacity_millimeters: capacity,
                crop_coefficient,
                demand_estimate,
            },
        ) {
            plan = preemptive_plan;
            condition = SprinklerScheduleConditionV1::PreemptiveHoldOff;
        } else {
            condition = SprinklerScheduleConditionV1::HeldOff;
        }
    }
    condition = if zone.valve_fault_bitmap != 0 {
        SprinklerScheduleConditionV1::ValveFault
    } else if !zone.valve_state_known {
        SprinklerScheduleConditionV1::ValveStateUnavailable
    } else if zone.valve_is_open {
        SprinklerScheduleConditionV1::ValveOpen
    } else if zone.pending_command.is_some() {
        SprinklerScheduleConditionV1::ValveCommandPending
    } else {
        condition
    };
    base(
        condition,
        SprinklerTimeSlotV1 {
            starts_at: plan.starts_at,
            duration_seconds: plan.duration_seconds,
        },
        plan.planned_water_millimeters,
    )
}

fn weather_permits_immediate_watering(
    weather: &SprinklerWeatherSnapshotV2,
    head: SprinklerHeadTypeV1,
    now: LibertasDateTime,
) -> bool {
    weather
        .current
        .as_ref()
        .filter(|current| current.is_fresh_at(now))
        .is_none_or(|current| current_is_safe(current, now, head))
}

fn valve_permits_automatic_watering(
    zone: &ZoneRuntime,
    watering_mode: SprinklerWateringModeV1,
) -> bool {
    watering_mode == SprinklerWateringModeV1::Active
        && zone.valve_state_known
        && zone.valve_fault_bitmap == 0
        && !zone.valve_is_open
        && zone.pending_command.is_none()
        && zone.expected_irrigation.is_none()
}

fn public_zone_state(
    zone: &ZoneRuntime,
    watering_mode: SprinklerWateringModeV1,
) -> SprinklerZoneStateV1 {
    match watering_mode {
        SprinklerWateringModeV1::Active => SprinklerZoneStateV1::ActiveV1 {
            condition: zone.active_state.condition,
            next_watering: zone.active_state.next_watering,
        },
        SprinklerWateringModeV1::Winterization => SprinklerZoneStateV1::WinterizationV1,
    }
}

fn public_zone_advanced_state(
    zone: &ZoneRuntime,
    watering_mode: SprinklerWateringModeV1,
) -> SprinklerZoneAdvancedStateV1 {
    match watering_mode {
        SprinklerWateringModeV1::Active => SprinklerZoneAdvancedStateV1::ActiveV1 {
            current: zone.active_state.clone(),
        },
        SprinklerWateringModeV1::Winterization => SprinklerZoneAdvancedStateV1::WinterizationV1,
    }
}

fn public_zone_configuration(zone: &ZoneRuntime) -> SprinklerZoneConfigurationV1 {
    SprinklerZoneConfigurationV1 {
        watering_percent: zone.memory.watering_percent,
        hold_off_periods: zone.memory.hold_off_periods.clone(),
    }
}

fn unsafe_weather_reason(
    weather: &SprinklerWeatherSnapshotV2,
    head: SprinklerHeadTypeV1,
    now: LibertasDateTime,
) -> SprinklerWateringReasonV1 {
    let Some(current) = weather
        .current
        .as_ref()
        .filter(|current| current.is_fresh_at(now))
    else {
        return SprinklerWateringReasonV1::OtherUnsafeWeather;
    };
    if current.precipitation_millimeters > 0.0 {
        return SprinklerWateringReasonV1::ObservedRain;
    }
    if current.temperature_celsius <= SAFE_MINIMUM_TEMPERATURE_CELSIUS {
        return SprinklerWateringReasonV1::FreezingWeather;
    }
    let (maximum_wind, maximum_gust) = safe_wind_limits(head);
    if current.wind_speed_meters_per_second > maximum_wind
        || current.wind_gust_meters_per_second > maximum_gust
    {
        return SprinklerWateringReasonV1::ExcessiveWind;
    }
    SprinklerWateringReasonV1::OtherUnsafeWeather
}

fn schedule_condition_reason(
    condition: SprinklerScheduleConditionV1,
    weather: &SprinklerWeatherSnapshotV2,
    head: SprinklerHeadTypeV1,
    now: LibertasDateTime,
) -> SprinklerWateringReasonV1 {
    match condition {
        SprinklerScheduleConditionV1::ForecastRain => SprinklerWateringReasonV1::ForecastRain,
        SprinklerScheduleConditionV1::WaitingForSafeWeather => {
            unsafe_weather_reason(weather, head, now)
        }
        SprinklerScheduleConditionV1::HeldOff | SprinklerScheduleConditionV1::PreemptiveHoldOff => {
            SprinklerWateringReasonV1::HoldOff
        }
        SprinklerScheduleConditionV1::ValveStateUnavailable => {
            SprinklerWateringReasonV1::ValveUnavailable
        }
        SprinklerScheduleConditionV1::ValveFault => SprinklerWateringReasonV1::ValveFault,
        SprinklerScheduleConditionV1::Initializing
        | SprinklerScheduleConditionV1::WaterNotNeeded
        | SprinklerScheduleConditionV1::Scheduled
        | SprinklerScheduleConditionV1::ValveCommandPending
        | SprinklerScheduleConditionV1::ValveOpen
        | SprinklerScheduleConditionV1::OfflineWeatherEstimate => {
            SprinklerWateringReasonV1::SmartSchedule
        }
    }
}

fn condition_skips_due_plan(condition: SprinklerScheduleConditionV1) -> bool {
    matches!(
        condition,
        SprinklerScheduleConditionV1::ForecastRain
            | SprinklerScheduleConditionV1::WaitingForSafeWeather
            | SprinklerScheduleConditionV1::HeldOff
            | SprinklerScheduleConditionV1::ValveStateUnavailable
            | SprinklerScheduleConditionV1::ValveFault
    )
}

fn scheduled_activity(
    zone: &ZoneRuntime,
    weather: &SprinklerWeatherSnapshotV2,
    now: LibertasDateTime,
) -> Option<SprinklerWateringActivityV1> {
    let scheduled_starts_at = zone.active_state.next_watering.starts_at;
    let (activity_index, activity_ordinal) = zone
        .current_activity
        .as_ref()
        .filter(|activity| {
            activity.outcome == SprinklerWateringOutcomeV1::Scheduled
                && activity.origin == SprinklerWateringOriginV1::Automatic
                && activity.scheduled_starts_at == Some(scheduled_starts_at)
        })
        .map(|activity| (activity.activity_index, activity.activity_ordinal))
        .or_else(|| {
            allocate_watering_activity_index(
                zone.configuration.valve,
                scheduled_starts_at,
                SprinklerWateringOriginV1::Automatic,
            )
        })?;
    Some(SprinklerWateringActivityV1 {
        activity_index,
        activity_ordinal,
        origin: SprinklerWateringOriginV1::Automatic,
        outcome: SprinklerWateringOutcomeV1::Scheduled,
        reason: schedule_condition_reason(
            zone.active_state.condition,
            weather,
            zone.configuration.sprinkler_head_type,
            now,
        ),
        scheduled_starts_at: Some(scheduled_starts_at),
        scheduled_duration_seconds: Some(zone.active_state.next_watering.duration_seconds),
        planned_water_millimeters: Some(zone.active_state.planned_water_millimeters),
        actual_starts_at: None,
        actual_duration_seconds: None,
        applied_water_millimeters: None,
        watering_percent: zone.memory.watering_percent,
        updated_at: now,
    })
}

fn synchronize_scheduled_activity(
    zone: &mut ZoneRuntime,
    weather: &SprinklerWeatherSnapshotV2,
    now: LibertasDateTime,
) -> Vec<SprinklerWateringActivityV1> {
    if zone.current_activity.as_ref().is_some_and(|activity| {
        matches!(
            activity.outcome,
            SprinklerWateringOutcomeV1::CommandPending | SprinklerWateringOutcomeV1::Running
        )
    }) {
        return Vec::new();
    }
    let Some(mut next) = scheduled_activity(zone, weather, now) else {
        return Vec::new();
    };
    let mut changed = Vec::new();
    if let Some(mut current) = zone.current_activity.take() {
        if current.outcome == SprinklerWateringOutcomeV1::Scheduled
            && current.activity_index == next.activity_index
        {
            next.updated_at = current.updated_at;
            if current.scheduled_duration_seconds != next.scheduled_duration_seconds
                || current.planned_water_millimeters != next.planned_water_millimeters
                || current.watering_percent != next.watering_percent
                || current.reason != next.reason
            {
                next.updated_at = now;
                changed.push(next.clone());
            }
            zone.current_activity = Some(next);
            return changed;
        }
        if current.outcome == SprinklerWateringOutcomeV1::Scheduled {
            let due = current
                .scheduled_starts_at
                .is_some_and(|starts_at| starts_at <= now);
            if due && condition_skips_due_plan(zone.active_state.condition) {
                current.outcome = SprinklerWateringOutcomeV1::Skipped;
                current.reason = schedule_condition_reason(
                    zone.active_state.condition,
                    weather,
                    zone.configuration.sprinkler_head_type,
                    now,
                );
            } else {
                current.outcome = SprinklerWateringOutcomeV1::Superseded;
                current.reason = SprinklerWateringReasonV1::Recalculated;
            }
            current.updated_at = now;
            changed.push(current);
        }
    }
    changed.push(next.clone());
    zone.current_activity = Some(next);
    changed
}

fn mark_current_activity(
    zone: &mut ZoneRuntime,
    outcome: SprinklerWateringOutcomeV1,
    reason: SprinklerWateringReasonV1,
    now: LibertasDateTime,
) -> Option<SprinklerWateringActivityV1> {
    let activity = zone.current_activity.as_mut()?;
    activity.outcome = outcome;
    activity.reason = reason;
    activity.updated_at = now;
    Some(activity.clone())
}

fn mark_automatic_activity_open(
    zone: &mut ZoneRuntime,
    observed_at: LibertasDateTime,
) -> Option<SprinklerWateringActivityV1> {
    let activity = zone.current_activity.as_mut()?;
    if activity.origin != SprinklerWateringOriginV1::Automatic
        || !matches!(
            activity.outcome,
            SprinklerWateringOutcomeV1::CommandPending | SprinklerWateringOutcomeV1::Running
        )
    {
        return None;
    }
    let first_observation = activity.actual_starts_at.is_none();
    activity.outcome = SprinklerWateringOutcomeV1::Running;
    if first_observation {
        activity.reason = SprinklerWateringReasonV1::SmartSchedule;
        activity.actual_starts_at = Some(observed_at);
    }
    // A Running record restored after a restart already contains the observed
    // pre-restart duration and water. Preserve that checkpoint and resume
    // accounting from this first trustworthy open report.
    activity.updated_at = observed_at;
    Some(activity.clone())
}

fn start_manual_activity(
    zone: &mut ZoneRuntime,
    starts_at: LibertasDateTime,
) -> Vec<SprinklerWateringActivityV1> {
    let mut changed = Vec::new();
    if let Some(mut previous) = zone.current_activity.take()
        && previous.outcome == SprinklerWateringOutcomeV1::Scheduled
    {
        previous.outcome = SprinklerWateringOutcomeV1::Superseded;
        previous.reason = SprinklerWateringReasonV1::ManualOperation;
        previous.updated_at = starts_at;
        changed.push(previous);
    }
    let Some((activity_index, activity_ordinal)) = allocate_watering_activity_index(
        zone.configuration.valve,
        starts_at,
        SprinklerWateringOriginV1::Manual,
    ) else {
        return changed;
    };
    let activity = SprinklerWateringActivityV1 {
        activity_index,
        activity_ordinal,
        origin: SprinklerWateringOriginV1::Manual,
        outcome: SprinklerWateringOutcomeV1::Running,
        reason: SprinklerWateringReasonV1::ManualOperation,
        scheduled_starts_at: None,
        scheduled_duration_seconds: None,
        planned_water_millimeters: None,
        actual_starts_at: Some(starts_at),
        actual_duration_seconds: None,
        applied_water_millimeters: None,
        watering_percent: zone.memory.watering_percent,
        updated_at: starts_at,
    };
    changed.push(activity.clone());
    zone.current_activity = Some(activity);
    changed
}

fn automatic_valve_must_close(
    zone: &ZoneRuntime,
    weather_safe: bool,
    watering_mode: SprinklerWateringModeV1,
) -> bool {
    zone.valve_is_open
        && zone.valve_opened_automatically
        && zone.pending_command.is_none()
        && (!weather_safe || watering_mode == SprinklerWateringModeV1::Winterization)
}

fn evaluate_controller(shared: &Rc<RefCell<ControllerState>>) -> EvaluationOutcome {
    let now = utc_seconds().unwrap_or_default();
    let now_ticks = libertas_get_sys_ticks();
    let mut state = shared.borrow_mut();
    let mut changed_zones = Vec::new();
    let mut zone_memories_to_persist = Vec::new();
    let mut activities_to_persist = Vec::new();
    let mut daily_reports_to_persist = Vec::new();
    let mut modeled_gap_changes = Vec::new();
    for (zone_index, zone) in state.zones.iter_mut().enumerate() {
        if let Some(timed_out) = zone.pending_command.filter(|pending| {
            now_ticks.saturating_sub(pending.sent_at_ticks)
                >= u64::from(VALVE_COMMAND_TIMEOUT_SECONDS).saturating_mul(MICROSECONDS_PER_SECOND)
        }) {
            zone.pending_command = None;
            match timed_out.kind {
                ValveCommandKind::Open => {
                    // A missing command response does not prove the valve stayed
                    // closed. Keep the durable expectation until an observed
                    // CurrentState resolves it, so a late open is not mislabeled
                    // as a manual run and no unobserved water is counted.
                    if let Some(activity) = zone.current_activity.as_mut()
                        && activity.outcome == SprinklerWateringOutcomeV1::CommandPending
                    {
                        activity.reason = SprinklerWateringReasonV1::CommandTimeout;
                        activity.updated_at = now;
                        activities_to_persist.push((zone.configuration.valve, activity.clone()));
                    }
                }
                ValveCommandKind::Close => {
                    if let Some(activity) = zone.current_activity.as_mut()
                        && activity.outcome == SprinklerWateringOutcomeV1::Running
                    {
                        activity.reason = SprinklerWateringReasonV1::CommandTimeout;
                        activity.updated_at = now;
                        activities_to_persist.push((zone.configuration.valve, activity.clone()));
                    }
                }
            }
        }
        if prune_expired_hold_offs(&mut zone.memory, now) {
            zone_memories_to_persist.push((zone.configuration.valve, zone.memory.clone()));
            changed_zones.push(zone_index);
        }
    }
    let watering_mode = state.watering_mode;
    let weather = state.weather.clone();
    let weather_stream_ready = state.weather_stream_ready;
    let site_location = state.site_location;
    let any_open = state.zones.iter().any(|zone| zone.valve_is_open);
    let any_pending = state
        .zones
        .iter()
        .any(|zone| zone.pending_command.is_some() || zone.expected_irrigation.is_some());
    for (zone_index, zone) in state.zones.iter_mut().enumerate() {
        if let Some(change) = reconcile_zone_modeled_weather_gaps(zone, site_location, now) {
            modeled_gap_changes.push(change);
            if !changed_zones.contains(&zone_index) {
                changed_zones.push(zone_index);
            }
        }
        if watering_mode == SprinklerWateringModeV1::Winterization {
            if zone
                .current_activity
                .as_ref()
                .is_some_and(|activity| activity.outcome == SprinklerWateringOutcomeV1::Scheduled)
                && let Some(activity) = mark_current_activity(
                    zone,
                    SprinklerWateringOutcomeV1::Skipped,
                    SprinklerWateringReasonV1::Winterization,
                    now,
                )
            {
                activities_to_persist.push((zone.configuration.valve, activity));
            }
            continue;
        }
        let active_state =
            calculate_active_state(zone, &weather, weather_stream_ready, site_location, now);
        if active_state != zone.active_state {
            zone.active_state = active_state;
            if !changed_zones.contains(&zone_index) {
                changed_zones.push(zone_index);
            }
        }
        let activity_changes = synchronize_scheduled_activity(zone, &weather, now);
        if !activity_changes.is_empty() {
            if !changed_zones.contains(&zone_index) {
                changed_zones.push(zone_index);
            }
            activities_to_persist.extend(
                activity_changes
                    .into_iter()
                    .map(|activity| (zone.configuration.valve, activity)),
            );
        }
    }

    for zone_index in &changed_zones {
        let zone = &state.zones[*zone_index];
        daily_reports_to_persist.push((
            zone.configuration.valve,
            build_daily_reports(zone, site_location, now),
        ));
    }

    let action = if any_open {
        state
            .zones
            .iter()
            .enumerate()
            .find(|(_, zone)| {
                automatic_valve_must_close(
                    zone,
                    weather_permits_immediate_watering(
                        &weather,
                        zone.configuration.sprinkler_head_type,
                        now,
                    ),
                    watering_mode,
                )
            })
            .map(|(zone_index, zone)| ControllerAction::Close {
                zone_index,
                reason: if watering_mode == SprinklerWateringModeV1::Winterization {
                    SprinklerWateringReasonV1::Winterization
                } else {
                    unsafe_weather_reason(&weather, zone.configuration.sprinkler_head_type, now)
                },
            })
    } else if !any_open
        && !any_pending
        && valve_decision_allowed(now_ticks, state.valve_decision_not_before_ticks)
    {
        state
            .zones
            .iter()
            .enumerate()
            .find_map(|(zone_index, zone)| {
                (valve_permits_automatic_watering(zone, watering_mode)
                    && weather_permits_immediate_watering(
                        &weather,
                        zone.configuration.sprinkler_head_type,
                        now,
                    )
                    && zone.active_state.next_watering.starts_at <= now)
                    .then_some(ControllerAction::Open {
                        zone_index,
                        duration_seconds: zone.active_state.next_watering.duration_seconds,
                    })
            })
    } else {
        None
    };

    EvaluationOutcome {
        changed_zones,
        zone_memories_to_persist,
        activities_to_persist,
        daily_reports_to_persist,
        modeled_gap_changes,
        action,
    }
}

fn publish_zone_state(shared: &Rc<RefCell<ControllerState>>, zone_index: usize) {
    let (endpoint, message) = {
        let state = shared.borrow();
        let Some(zone) = state.zones.get(zone_index) else {
            return;
        };
        (
            zone.configuration.state_endpoint,
            SprinklerZoneProtocolV1::StateV1 {
                state: public_zone_state(zone, state.watering_mode),
            },
        )
    };
    libertas_endpoint_report(endpoint, &message, None);
}

fn rollback_expected_irrigation(shared: &Rc<RefCell<ControllerState>>, zone_index: usize) -> bool {
    let (valve, activity, changed) = {
        let mut state = shared.borrow_mut();
        let Some(zone) = state.zones.get_mut(zone_index) else {
            return false;
        };
        zone.pending_command = None;
        let changed = discard_expected_irrigation(zone);
        let activity = utc_seconds().and_then(|now| {
            mark_current_activity(
                zone,
                SprinklerWateringOutcomeV1::Failed,
                SprinklerWateringReasonV1::CommandFailed,
                now,
            )
        });
        (zone.configuration.valve, activity, changed)
    };
    if let Some(activity) = &activity {
        persist_watering_activity(valve, activity);
    }
    changed
}

fn execute_timed_open(
    shared: &Rc<RefCell<ControllerState>>,
    zone_index: usize,
    duration_seconds: u32,
) {
    let Some(starts_at) = utc_seconds() else {
        libertas_log(
            LogLevel::Warn,
            "Cannot persist expected irrigation without valid UTC time",
        );
        return;
    };
    let sent_at_ticks = libertas_get_sys_ticks();
    let prepared = {
        let mut state = shared.borrow_mut();
        let Some(zone) = state.zones.get_mut(zone_index) else {
            return;
        };
        if zone.pending_command.is_some() || zone.expected_irrigation.is_some() {
            return;
        }
        if !begin_expected_irrigation(zone, starts_at, duration_seconds) {
            libertas_log(
                LogLevel::Warn,
                "Cannot reserve an expected irrigation database record",
            );
            return;
        }
        zone.pending_command = Some(PendingValveCommand {
            kind: ValveCommandKind::Open,
            transaction_id: None,
            sent_at_ticks,
        });
        let expected_activity_index = zone.expected_irrigation.unwrap().activity_index;
        let expected_activity_ordinal = zone.expected_irrigation.unwrap().activity_ordinal;
        let can_reuse_scheduled_activity = zone.current_activity.as_ref().is_some_and(|activity| {
            activity.activity_index == expected_activity_index
                && activity.origin == SprinklerWateringOriginV1::Automatic
                && activity.outcome == SprinklerWateringOutcomeV1::Scheduled
        });
        if !can_reuse_scheduled_activity {
            zone.current_activity = Some(SprinklerWateringActivityV1 {
                activity_index: expected_activity_index,
                activity_ordinal: expected_activity_ordinal,
                origin: SprinklerWateringOriginV1::Automatic,
                outcome: SprinklerWateringOutcomeV1::Scheduled,
                reason: SprinklerWateringReasonV1::SmartSchedule,
                scheduled_starts_at: Some(starts_at),
                scheduled_duration_seconds: Some(duration_seconds),
                planned_water_millimeters: Some(zone.active_state.planned_water_millimeters),
                actual_starts_at: None,
                actual_duration_seconds: None,
                applied_water_millimeters: None,
                watering_percent: zone.memory.watering_percent,
                updated_at: starts_at,
            });
        }
        let activity = zone.current_activity.as_mut().unwrap();
        activity.outcome = SprinklerWateringOutcomeV1::CommandPending;
        activity.reason = SprinklerWateringReasonV1::SmartSchedule;
        activity.updated_at = starts_at;
        (zone.configuration.valve, activity.clone())
    };
    let (valve, activity) = prepared;
    // Persist the plan before issuing the timed command. Delivered water is
    // recorded only from observed valve-open time.
    persist_watering_activity(valve, &activity);
    match MatterDevice::new(valve).invoke(&Open {
        OpenDuration: Some(Nullable::some(duration_seconds)),
        TargetLevel: None,
    }) {
        Ok(transaction_id) => {
            if let Some(pending) = shared.borrow_mut().zones[zone_index]
                .pending_command
                .as_mut()
                && pending.kind == ValveCommandKind::Open
            {
                pending.transaction_id = Some(transaction_id);
            }
        }
        Err(error) => {
            rollback_expected_irrigation(shared, zone_index);
            libertas_log(
                LogLevel::Error,
                &alloc::format!("Matter Valve command could not be encoded: {error}"),
            );
        }
    }
}

fn execute_close(
    shared: &Rc<RefCell<ControllerState>>,
    zone_index: usize,
    reason: SprinklerWateringReasonV1,
) {
    let (valve, sent_at_ticks, activity) = {
        let mut state = shared.borrow_mut();
        let Some(zone) = state.zones.get_mut(zone_index) else {
            return;
        };
        if zone.pending_command.is_some() {
            return;
        }
        let sent_at_ticks = libertas_get_sys_ticks();
        zone.pending_command = Some(PendingValveCommand {
            kind: ValveCommandKind::Close,
            transaction_id: None,
            sent_at_ticks,
        });
        let activity = utc_seconds().and_then(|now| {
            let activity = zone.current_activity.as_mut()?;
            if activity.outcome != SprinklerWateringOutcomeV1::Running {
                return None;
            }
            activity.reason = reason;
            activity.updated_at = now;
            Some(activity.clone())
        });
        (zone.configuration.valve, sent_at_ticks, activity)
    };
    if let Some(activity) = &activity {
        persist_watering_activity(valve, activity);
    }
    match MatterDevice::new(valve).invoke(&Close {}) {
        Ok(transaction_id) => {
            if let Some(pending) = shared.borrow_mut().zones[zone_index]
                .pending_command
                .as_mut()
                && pending.kind == ValveCommandKind::Close
            {
                pending.transaction_id = Some(transaction_id);
                pending.sent_at_ticks = sent_at_ticks;
            }
        }
        Err(error) => {
            let failed_activity = {
                let mut state = shared.borrow_mut();
                let zone = &mut state.zones[zone_index];
                zone.pending_command = None;
                utc_seconds().and_then(|now| {
                    let activity = zone.current_activity.as_mut()?;
                    if activity.outcome != SprinklerWateringOutcomeV1::Running {
                        return None;
                    }
                    activity.reason = SprinklerWateringReasonV1::CommandFailed;
                    activity.updated_at = now;
                    Some(activity.clone())
                })
            };
            if let Some(activity) = &failed_activity {
                persist_watering_activity(valve, activity);
            }
            libertas_log(
                LogLevel::Error,
                &alloc::format!("Matter Valve command could not be encoded: {error}"),
            );
        }
    }
}

fn execute_controller_action(shared: &Rc<RefCell<ControllerState>>, action: ControllerAction) {
    match action {
        ControllerAction::Open {
            zone_index,
            duration_seconds,
        } => execute_timed_open(shared, zone_index, duration_seconds),
        ControllerAction::Close { zone_index, reason } => {
            execute_close(shared, zone_index, reason);
        }
    }
}

fn apply_evaluation_outcome(
    shared: &Rc<RefCell<ControllerState>>,
    outcome: EvaluationOutcome,
) -> Option<ControllerAction> {
    for (valve, memory) in outcome.zone_memories_to_persist {
        persist_zone_memory(valve, &memory);
    }
    for (valve, activity) in outcome.activities_to_persist {
        persist_watering_activity(valve, &activity);
    }
    for change in outcome.modeled_gap_changes {
        change.submit();
    }
    for (valve, reports) in outcome.daily_reports_to_persist {
        persist_daily_reports(valve, &reports);
    }
    for zone_index in outcome.changed_zones {
        publish_zone_state(shared, zone_index);
    }
    outcome.action
}

fn dispatch_evaluation(shared: &Rc<RefCell<ControllerState>>, outcome: EvaluationOutcome) {
    if let Some(action) = apply_evaluation_outcome(shared, outcome) {
        execute_controller_action(shared, action);
        let follow_up = evaluate_controller(shared);
        let _ = apply_evaluation_outcome(shared, follow_up);
    }
}

fn evaluate_winterization_reminder(
    shared: &Rc<RefCell<ControllerState>>,
) -> Option<(
    LibertasEndpoint,
    SprinklerWinterizationReminderMemoryV1,
    WinterizationReminderAction,
)> {
    let now = utc_seconds()?;
    let mut state = shared.borrow_mut();
    let evidence = winterization_reminder_evidence(
        state.watering_mode,
        &state.weather,
        state.site_location,
        now,
    )?;
    if !winterization_reminder_is_due(state.winterization_reminder, evidence, now) {
        return None;
    }
    let memory = SprinklerWinterizationReminderMemoryV1 {
        last_reminded_at: now,
        reason: evidence.reason(),
    };
    state.winterization_reminder = Some(memory);
    Some((
        state.weather_endpoint,
        memory,
        WinterizationReminderAction {
            recipients: state.reminder_recipients.clone(),
            evidence,
        },
    ))
}

fn send_winterization_reminder_if_due(shared: &Rc<RefCell<ControllerState>>) {
    let Some((weather_endpoint, memory, action)) = evaluate_winterization_reminder(shared) else {
        return;
    };
    // Submit persistence first so a restart cannot replay an avoidable reminder.
    persist_winterization_reminder(weather_endpoint, memory);
    action.submit();
}

fn evaluate_and_publish(shared: &Rc<RefCell<ControllerState>>) {
    let outcome = evaluate_controller(shared);
    dispatch_evaluation(shared, outcome);
    send_winterization_reminder_if_due(shared);
}

fn account_open_zone(
    zone: &mut ZoneRuntime,
    now_ticks: u64,
    now_utc: Option<LibertasDateTime>,
    site_location: Option<SprinklerWeatherLocationV1>,
) -> bool {
    if !zone.valve_is_open {
        return false;
    }
    let Some(previous_ticks) = zone.accounted_at_ticks else {
        zone.accounted_at_ticks = Some(now_ticks);
        zone.accounted_at_utc = now_utc;
        return false;
    };
    let elapsed_seconds = now_ticks.saturating_sub(previous_ticks) / MICROSECONDS_PER_SECOND;
    let Ok(duration_seconds) = u32::try_from(elapsed_seconds) else {
        return false;
    };
    if duration_seconds == 0 {
        return false;
    }
    let starts_at = match zone.accounted_at_utc {
        Some(starts_at) => starts_at,
        None => {
            let Some(now_utc) = now_utc else {
                return false;
            };
            now_utc.saturating_sub(u64::from(duration_seconds))
        }
    };
    let event_now = now_utc.unwrap_or_else(|| starts_at.saturating_add(elapsed_seconds));
    add_irrigation_event(zone, starts_at, duration_seconds, site_location, event_now);
    if let Some(activity) = zone.current_activity.as_mut()
        && matches!(
            activity.outcome,
            SprinklerWateringOutcomeV1::CommandPending | SprinklerWateringOutcomeV1::Running
        )
    {
        let applied_water_millimeters =
            nominal_delivery_millimeters_per_hour(zone.configuration.sprinkler_head_type)
                * duration_seconds as f32
                / 3_600.0;
        activity.actual_starts_at = Some(activity.actual_starts_at.unwrap_or(starts_at));
        activity.outcome = SprinklerWateringOutcomeV1::Running;
        activity.actual_duration_seconds = Some(
            activity
                .actual_duration_seconds
                .unwrap_or_default()
                .saturating_add(duration_seconds),
        );
        activity.applied_water_millimeters = Some(
            activity.applied_water_millimeters.unwrap_or_default() + applied_water_millimeters,
        );
        activity.updated_at = event_now;
    }
    zone.accounted_at_ticks = Some(
        previous_ticks.saturating_add(elapsed_seconds.saturating_mul(MICROSECONDS_PER_SECOND)),
    );
    zone.accounted_at_utc = Some(starts_at.saturating_add(elapsed_seconds));
    true
}

fn account_all_open_valves(shared: &Rc<RefCell<ControllerState>>) {
    let now_ticks = libertas_get_sys_ticks();
    let now_utc = utc_seconds();
    let changed = {
        let mut state = shared.borrow_mut();
        let site_location = state.site_location;
        let mut changed = Vec::new();
        for (zone_index, zone) in state.zones.iter_mut().enumerate() {
            let previous_memory = zone.memory.clone();
            let previous_events = zone.water_events.clone();
            if account_open_zone(zone, now_ticks, now_utc, site_location) {
                changed.push((
                    zone_index,
                    zone.configuration.valve,
                    previous_memory,
                    zone.memory.clone(),
                    previous_events,
                    zone.water_events.clone(),
                    zone.current_activity.clone(),
                    core::mem::take(&mut zone.finalized_daily_reports),
                ));
            }
        }
        changed
    };
    for (
        _,
        valve,
        previous_memory,
        memory,
        previous_events,
        water_events,
        activity,
        daily_reports,
    ) in &changed
    {
        persist_daily_reports(*valve, daily_reports);
        persist_zone_runtime_change(
            *valve,
            previous_memory,
            memory,
            previous_events,
            water_events,
        );
        if let Some(activity) = activity {
            persist_watering_activity(*valve, activity);
        }
    }
    if !changed.is_empty() {
        evaluate_and_publish(shared);
    }
}

fn arm_valve_decision_delay(shared: &Rc<RefCell<ControllerState>>, now_ticks: u64) {
    let requested_deadline = absolute_interval_ticks(now_ticks, VALVE_DECISION_DELAY_SECONDS);
    let (timer, deadline) = {
        let mut state = shared.borrow_mut();
        state.valve_decision_not_before_ticks = state
            .valve_decision_not_before_ticks
            .max(requested_deadline);
        (
            state.valve_decision_timer,
            state.valve_decision_not_before_ticks,
        )
    };
    if timer != 0 {
        libertas_timer_update_interval(timer, deadline);
    }
}

fn set_valve_open_state(shared: &Rc<RefCell<ControllerState>>, zone_index: usize, is_open: bool) {
    let now_ticks = libertas_get_sys_ticks();
    let now_utc = utc_seconds();
    let (change_to_persist, activities_to_persist, finalized_daily_reports, closed_transition) = {
        let mut state = shared.borrow_mut();
        let site_location = state.site_location;
        let Some(zone) = state.zones.get_mut(zone_index) else {
            return;
        };
        let was_known = zone.valve_state_known;
        let restored_manual_running = !was_known
            && zone.current_activity.as_ref().is_some_and(|activity| {
                activity.origin == SprinklerWateringOriginV1::Manual
                    && activity.outcome == SprinklerWateringOutcomeV1::Running
            });
        let was_open = zone.valve_is_open;
        let previous_memory = zone.memory.clone();
        let previous_events = zone.water_events.clone();
        zone.valve_state_known = true;
        zone.valve_last_report_ticks = Some(now_ticks);
        let mut irrigation_changed = false;
        let mut activities_to_persist = Vec::new();
        if was_open && !is_open {
            irrigation_changed = account_open_zone(zone, now_ticks, now_utc, site_location);
            reconcile_expected_irrigation(zone, now_ticks);
            if let Some(now) = now_utc {
                let completed = zone
                    .current_activity
                    .as_ref()
                    .and_then(|activity| activity.actual_duration_seconds)
                    .is_some_and(|duration| duration > 0);
                if let Some(activity) = mark_current_activity(
                    zone,
                    if completed {
                        SprinklerWateringOutcomeV1::Completed
                    } else {
                        SprinklerWateringOutcomeV1::Failed
                    },
                    if zone.valve_opened_automatically {
                        if completed {
                            zone.current_activity
                                .as_ref()
                                .map(|activity| activity.reason)
                                .unwrap_or(SprinklerWateringReasonV1::SmartSchedule)
                        } else {
                            SprinklerWateringReasonV1::NoOpenObserved
                        }
                    } else {
                        SprinklerWateringReasonV1::ManualOperation
                    },
                    now,
                ) {
                    activities_to_persist.push(activity);
                }
            }
        } else if !is_open && zone.expected_irrigation.is_some() {
            reconcile_expected_irrigation(zone, now_ticks);
            if let Some(now) = now_utc {
                let had_observed_water = zone
                    .current_activity
                    .as_ref()
                    .and_then(|activity| activity.actual_duration_seconds)
                    .is_some_and(|duration| duration > 0);
                let retained_reason = zone
                    .current_activity
                    .as_ref()
                    .map(|activity| activity.reason)
                    .unwrap_or(SprinklerWateringReasonV1::NoOpenObserved);
                if let Some(activity) = mark_current_activity(
                    zone,
                    if had_observed_water {
                        SprinklerWateringOutcomeV1::Completed
                    } else {
                        SprinklerWateringOutcomeV1::Failed
                    },
                    if had_observed_water {
                        retained_reason
                    } else {
                        SprinklerWateringReasonV1::NoOpenObserved
                    },
                    now,
                ) {
                    activities_to_persist.push(activity);
                }
            }
        } else if !is_open
            && restored_manual_running
            && let Some(now) = now_utc
            && let Some(activity) = mark_current_activity(
                zone,
                SprinklerWateringOutcomeV1::Completed,
                SprinklerWateringReasonV1::ManualOperation,
                now,
            )
        {
            activities_to_persist.push(activity);
        }
        if was_open != is_open {
            let opened_automatically = is_open && zone.expected_irrigation.is_some();
            zone.valve_is_open = is_open;
            zone.pending_command = None;
            if is_open {
                zone.valve_opened_automatically = opened_automatically;
                zone.accounted_at_ticks = Some(now_ticks);
                zone.accounted_at_utc = now_utc;
                if let Some(now) = now_utc {
                    if opened_automatically {
                        if let Some(activity) = mark_automatic_activity_open(zone, now) {
                            activities_to_persist.push(activity);
                        }
                    } else if restored_manual_running {
                        zone.valve_opened_automatically = false;
                        if let Some(activity) = zone.current_activity.as_mut() {
                            activity.updated_at = now;
                            activities_to_persist.push(activity.clone());
                        }
                    } else {
                        activities_to_persist.extend(start_manual_activity(zone, now));
                    }
                }
            } else {
                zone.valve_opened_automatically = false;
                zone.accounted_at_ticks = None;
                zone.accounted_at_utc = None;
            }
        } else if !is_open {
            zone.pending_command = None;
        }
        (
            irrigation_changed
                .then(|| zone_persistence_change(zone, previous_memory, previous_events)),
            activities_to_persist,
            core::mem::take(&mut zone.finalized_daily_reports),
            !is_open && (!was_known || was_open),
        )
    };
    let valve = shared.borrow().zones[zone_index].configuration.valve;
    persist_daily_reports(valve, &finalized_daily_reports);
    if let Some(change) = change_to_persist {
        change.submit();
    }
    for activity in &activities_to_persist {
        persist_watering_activity(valve, activity);
    }
    if closed_transition {
        arm_valve_decision_delay(shared, now_ticks);
    }
    evaluate_and_publish(shared);
}

fn set_valve_fault(shared: &Rc<RefCell<ControllerState>>, zone_index: usize, fault_bitmap: u16) {
    let now_ticks = libertas_get_sys_ticks();
    let changed = {
        let mut state = shared.borrow_mut();
        let Some(zone) = state.zones.get_mut(zone_index) else {
            return;
        };
        let changed = zone.valve_fault_bitmap != fault_bitmap;
        zone.valve_fault_bitmap = fault_bitmap;
        zone.valve_last_report_ticks = Some(now_ticks);
        changed
    };
    if changed {
        evaluate_and_publish(shared);
    }
}

fn handle_valve_command_response(
    shared: &Rc<RefCell<ControllerState>>,
    zone_index: usize,
    transaction_id: u32,
    data: &[u8],
) {
    let pending = shared
        .borrow()
        .zones
        .get(zone_index)
        .and_then(|zone| zone.pending_command);
    let Some(pending) = pending else {
        return;
    };
    if pending.transaction_id != Some(transaction_id) {
        return;
    }
    let successful = match pending.kind {
        ValveCommandKind::Open => matches!(
            decode_command_response::<Open>(data),
            Ok(MatterResponse::Status(status)) if status.status == 0
        ),
        ValveCommandKind::Close => matches!(
            decode_command_response::<Close>(data),
            Ok(MatterResponse::Status(status)) if status.status == 0
        ),
    };
    if !successful {
        if pending.kind == ValveCommandKind::Open {
            rollback_expected_irrigation(shared, zone_index);
        } else {
            shared.borrow_mut().zones[zone_index].pending_command = None;
        }
        libertas_log(LogLevel::Warn, "Matter Valve command failed");
        evaluate_and_publish(shared);
    }
}

fn handle_valve_event(
    _device: LibertasDevice,
    opcode: u8,
    data: &[u8],
    context: &mut Box<dyn Any>,
    transaction_id: u32,
    _peer: u32,
) {
    let context = context.downcast_mut::<ZoneContext>().unwrap();
    if opcode == Operation::InvokeResponse as u8 {
        handle_valve_command_response(&context.shared, context.zone_index, transaction_id, data);
        return;
    }
    if opcode != Operation::ReportData as u8 {
        return;
    }

    if let Ok(MatterResponse::Data(CurrentState(state))) =
        decode_attribute_report::<CurrentState>(data)
        && let Some(state) = state.into_option()
    {
        if state == ValveStateEnum::Open {
            set_valve_open_state(&context.shared, context.zone_index, true);
        } else if state == ValveStateEnum::Closed {
            set_valve_open_state(&context.shared, context.zone_index, false);
        }
        return;
    }
    if let Ok(MatterResponse::Data(ValveFault(fault))) = decode_attribute_report::<ValveFault>(data)
    {
        set_valve_fault(&context.shared, context.zone_index, fault.0);
        return;
    }
    if let Ok(MatterResponse::Data(event)) = decode_event_report::<ValveStateChanged>(data) {
        if event.ValveState == ValveStateEnum::Open {
            set_valve_open_state(&context.shared, context.zone_index, true);
        } else if event.ValveState == ValveStateEnum::Closed {
            set_valve_open_state(&context.shared, context.zone_index, false);
        }
    }
}

fn subscribe_to_valves(valves: &[LibertasDevice]) -> Result<(), libertas_matter::error::Error> {
    let mut builders = Vec::with_capacity(valves.len());
    for _ in valves {
        let mut cluster = MatterSubscriptionCluster::<2, 1>::for_attribute::<CurrentState>(
            0,
            VALVE_SUBSCRIPTION_MAX_INTERVAL_SECONDS,
        );
        cluster
            .add_attribute::<CurrentState>()?
            .add_attribute::<ValveFault>()?
            .add_event::<ValveStateChanged>(true)?;
        builders.push(cluster);
    }
    let cluster_requests: Result<Vec<_>, _> =
        builders.iter().map(|cluster| cluster.request()).collect();
    let cluster_sets: Vec<_> = cluster_requests?
        .into_iter()
        .map(|cluster| [cluster])
        .collect();
    let device_requests: Result<Vec<_>, _> = valves
        .iter()
        .zip(cluster_sets.iter())
        .map(|(valve, clusters)| MatterDeviceSubscription::new(MatterDevice::new(*valve), clusters))
        .collect();
    MatterSubscriptionBatch::new(&device_requests?)?.send();
    Ok(())
}

fn request_valve_subscriptions(shared: &Rc<RefCell<ControllerState>>) {
    let valves: Vec<_> = shared
        .borrow()
        .zones
        .iter()
        .map(|zone| zone.configuration.valve)
        .collect();
    if let Err(error) = subscribe_to_valves(&valves) {
        libertas_log(
            LogLevel::Error,
            &alloc::format!("Matter Valve subscription failed: {error}"),
        );
    }
}

fn refresh_valve_subscriptions(shared: &Rc<RefCell<ControllerState>>, now_ticks: u64) {
    let stale_interval =
        u64::from(VALVE_SUBSCRIPTION_STALE_SECONDS).saturating_mul(MICROSECONDS_PER_SECOND);
    let (subscription_needed, schedule_changed) = {
        let mut state = shared.borrow_mut();
        let mut subscription_needed = false;
        let mut schedule_changed = false;
        for zone in &mut state.zones {
            let stale = zone
                .valve_last_report_ticks
                .is_none_or(|last_report| now_ticks.saturating_sub(last_report) >= stale_interval);
            if !zone.valve_state_known || stale {
                subscription_needed = true;
            }
            if stale && zone.valve_state_known {
                zone.valve_state_known = false;
                schedule_changed = true;
            }
        }
        (subscription_needed, schedule_changed)
    };
    if subscription_needed {
        request_valve_subscriptions(shared);
    }
    if schedule_changed {
        evaluate_and_publish(shared);
    }
}

fn arm_site_location_retry(shared: &Rc<RefCell<ControllerState>>, delay_seconds: u32) {
    let (timer, server_up) = {
        let state = shared.borrow();
        (
            state.site_location_retry_timer,
            state.hub_location_server_up,
        )
    };
    if timer == 0 {
        return;
    }
    if !server_up {
        libertas_timer_cancel(timer);
        return;
    }
    libertas_timer_update_interval(
        timer,
        absolute_interval_ticks(libertas_get_sys_ticks(), delay_seconds.max(1)),
    );
}

fn request_site_location(shared: &Rc<RefCell<ControllerState>>) {
    if !shared.borrow().hub_location_server_up {
        return;
    }
    libertas_endpoint_subscribe_request(
        LIBERTAS_HUB_ENDPOINT,
        &HubProtocol::LocationReq {
            max_report_interval_seconds: HUB_LOCATION_MAX_REPORT_INTERVAL_SECONDS,
        },
    );
    arm_site_location_retry(shared, HUB_LOCATION_MAX_REPORT_INTERVAL_SECONDS);
}

fn site_location_retry_timer(timer: u32, now_ticks: u64, context: &mut Box<dyn Any>) {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .unwrap();
    if !shared.borrow().hub_location_server_up {
        libertas_timer_cancel(timer);
        return;
    }
    libertas_endpoint_subscribe_request(
        LIBERTAS_HUB_ENDPOINT,
        &HubProtocol::LocationReq {
            max_report_interval_seconds: HUB_LOCATION_MAX_REPORT_INTERVAL_SECONDS,
        },
    );
    libertas_timer_update_interval(
        timer,
        absolute_interval_ticks(now_ticks, HUB_LOCATION_MAX_REPORT_INTERVAL_SECONDS),
    );
}

fn handle_site_location_event(
    _endpoint: LibertasEndpoint,
    opcode: u8,
    message: LibertasEndpointMessage<HubProtocol>,
    context: &mut Box<dyn Any>,
    _transaction_id: u32,
    _peer: u32,
) -> LibertasEndpointHandlerResult {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .unwrap();
    if opcode == OP_ENDPOINT_PEER_ALIVE {
        // Signaling only: rearm an established watchdog before any data path.
        if !matches!(message, LibertasEndpointMessage::NoPayload) {
            return LibertasEndpointHandlerResult::InvalidMessage;
        }
        if shared.borrow().hub_location_subscription_ready {
            arm_site_location_retry(shared, HUB_LOCATION_MAX_REPORT_INTERVAL_SECONDS);
        }
        return LibertasEndpointHandlerResult::Handled;
    }
    if opcode == OP_ENDPOINT_PEER_DOWN {
        let timer = {
            let mut state = shared.borrow_mut();
            state.hub_location_server_up = false;
            state.hub_location_subscription_ready = false;
            state.site_location_retry_timer
        };
        if timer != 0 {
            libertas_timer_cancel(timer);
        }
        return LibertasEndpointHandlerResult::Handled;
    }
    if opcode == OP_ENDPOINT_PEER_UP {
        // Up can arrive without the preceding Down. Re-establish the
        // subscription for this newer Hub endpoint startup.
        {
            let mut state = shared.borrow_mut();
            state.hub_location_server_up = true;
            state.hub_location_subscription_ready = false;
        }
        request_site_location(shared);
        return LibertasEndpointHandlerResult::Handled;
    }
    if opcode == OP_ENDPOINT_RSP || opcode == OP_ENDPOINT_DATA {
        if let LibertasEndpointMessage::Data(HubProtocol::LocationRsp {
            longitude,
            latitude,
        }) = message
        {
            let location = SprinklerWeatherLocationV1 {
                longitude_degrees: longitude,
                latitude_degrees: latitude,
            };
            if !valid_site_location(location) {
                return LibertasEndpointHandlerResult::InvalidMessage;
            }
            shared.borrow_mut().hub_location_subscription_ready = true;
            let (weather_endpoint, location_changed) = {
                let state = shared.borrow();
                (
                    state.weather_endpoint,
                    state
                        .site_location
                        .is_none_or(|saved| !same_weather_location(saved, location)),
                )
            };
            if location_changed {
                persist_site_location(weather_endpoint, location);
                {
                    let mut state = shared.borrow_mut();
                    state.site_location = Some(location);
                    // A direct Hub location update is authoritative for the
                    // controller, but only the weather stream's explicit site
                    // message can bind an archive generation. Until then,
                    // reject mismatched increments and expose no old-site
                    // weather.
                    state.weather = SprinklerWeatherSnapshotV2 {
                        history: None,
                        current: None,
                        forecast: None,
                    };
                    state.weather_stream_ready = false;
                }
                clear_recent_weather_memories(shared);
                evaluate_and_publish(shared);
            }
            arm_site_location_retry(shared, HUB_LOCATION_MAX_REPORT_INTERVAL_SECONDS);
            return LibertasEndpointHandlerResult::Handled;
        }
        return LibertasEndpointHandlerResult::InvalidMessage;
    }
    LibertasEndpointHandlerResult::Handled
}

#[derive(Clone)]
struct ReportZoneData {
    valve: LibertasDevice,
    capacity_millimeters: f32,
    crop_coefficient: f32,
    active_state: SprinklerZoneActiveStateV1,
    water_events: Vec<SprinklerWaterEventV1>,
    modeled_weather_gaps: Vec<SprinklerModeledWeatherGapV1>,
    current_activity: Option<SprinklerWateringActivityV1>,
    activities: Vec<SprinklerWateringActivityV1>,
    daily_reports: Vec<SprinklerDailyReportV1>,
}

fn calculated_available_water_percent(capacity: f32, deficit: f32) -> f32 {
    if !capacity.is_finite() || capacity <= 0.0 || !deficit.is_finite() {
        return 0.0;
    }
    ((capacity - deficit.clamp(0.0, capacity)) / capacity * 100.0).clamp(0.0, 100.0)
}

fn merge_runtime_activity(zone: &mut ReportZoneData, range: SprinklerReportTimeRangeV1) {
    if let Some(activity) = &zone.current_activity
        && valid_watering_activity(activity)
        && activity_interval(activity).is_some_and(|(starts_at, ends_at)| {
            starts_at < range.ends_before && ends_at > range.starts_at
        })
        && !zone
            .activities
            .iter()
            .any(|saved| saved.activity_index == activity.activity_index)
    {
        zone.activities.push(activity.clone());
    }

    for event in &zone.water_events {
        let SprinklerWaterEventV1::IrrigationV1 {
            starts_at,
            duration_seconds,
            watering_percent,
            applied_water_millimeters,
        } = event
        else {
            continue;
        };
        let ends_at = starts_at.saturating_add(u64::from(*duration_seconds));
        if *starts_at >= range.ends_before
            || ends_at <= range.starts_at
            || zone.activities.iter().any(|activity| {
                activity.actual_starts_at == Some(*starts_at)
                    || activity.scheduled_starts_at == Some(*starts_at)
            })
        {
            continue;
        }
        let Some(activity_index) =
            watering_activity_index(*starts_at, SprinklerWateringOriginV1::LegacyUnknown, 0)
        else {
            continue;
        };
        zone.activities.push(SprinklerWateringActivityV1 {
            activity_index,
            activity_ordinal: 0,
            origin: SprinklerWateringOriginV1::LegacyUnknown,
            outcome: SprinklerWateringOutcomeV1::Completed,
            reason: SprinklerWateringReasonV1::LegacyUnknown,
            scheduled_starts_at: None,
            scheduled_duration_seconds: None,
            planned_water_millimeters: None,
            actual_starts_at: Some(*starts_at),
            actual_duration_seconds: Some(*duration_seconds),
            applied_water_millimeters: Some(*applied_water_millimeters),
            watering_percent: *watering_percent,
            updated_at: ends_at,
        });
    }
    zone.activities
        .sort_by_key(|activity| activity.activity_index);
}

fn water_balance_points(
    zone: &ReportZoneData,
    history: &[SprinklerWeatherHistoryPeriodV1],
    provider_intervals: &[(LibertasDateTime, LibertasDateTime)],
    range: SprinklerReportTimeRangeV1,
) -> Result<Vec<(LibertasDateTime, f32)>, ()> {
    let containing_anchor = zone
        .daily_reports
        .iter()
        .filter(|report| {
            valid_daily_report(report)
                && report.coverage_starts_at <= range.starts_at
                && report.coverage_ends_before > range.starts_at
        })
        .max_by_key(|report| report.coverage_starts_at);
    let anchor = containing_anchor.or_else(|| {
        zone.daily_reports
            .iter()
            .filter(|report| {
                valid_daily_report(report)
                    && report.coverage_starts_at >= range.starts_at
                    && report.coverage_starts_at < range.ends_before
            })
            .min_by_key(|report| report.coverage_starts_at)
    });
    let Some(anchor) = anchor else {
        return Ok((zone.active_state.calculated_at >= range.starts_at
            && zone.active_state.calculated_at < range.ends_before)
            .then_some((
                zone.active_state.calculated_at,
                calculated_available_water_percent(
                    zone.capacity_millimeters,
                    zone.active_state.estimated_deficit_millimeters,
                ),
            ))
            .into_iter()
            .collect());
    };
    let starts_at = range.starts_at.max(anchor.coverage_starts_at);
    let ends_before = zone
        .daily_reports
        .iter()
        .filter(|report| valid_daily_report(report))
        .map(|report| report.coverage_ends_before)
        .max()
        .unwrap_or(anchor.coverage_ends_before)
        .min(range.ends_before);
    if starts_at > ends_before {
        return Ok(Vec::new());
    }

    let replay_starts_at = anchor.coverage_starts_at;
    let mut intervals = Vec::new();
    for period in history {
        let period_ends_at = period
            .starts_at
            .saturating_add(u64::from(period.duration_seconds));
        let interval_start = period.starts_at.max(replay_starts_at);
        let interval_end = period_ends_at.min(ends_before);
        if interval_start >= interval_end || period.duration_seconds == 0 {
            continue;
        }
        intervals.push(BalanceRateInterval {
            starts_at: interval_start,
            ends_before: interval_end,
            deficit_millimeters_per_second: (period.reference_evapotranspiration_millimeters
                * zone.crop_coefficient
                - period.precipitation_millimeters)
                / period.duration_seconds as f32,
        });
    }
    if intervals.len() > MAX_REPORT_BALANCE_INTERVALS_PER_ZONE {
        return Err(());
    }
    intervals.extend(report_modeled_gap_balance_intervals(
        &zone.modeled_weather_gaps,
        provider_intervals,
        zone.crop_coefficient,
        replay_starts_at,
        ends_before,
        MAX_REPORT_BALANCE_INTERVALS_PER_ZONE - intervals.len(),
    )?);
    for activity in &zone.activities {
        let (Some(activity_starts_at), Some(duration_seconds), Some(applied_water_millimeters)) = (
            activity.actual_starts_at,
            activity.actual_duration_seconds,
            activity.applied_water_millimeters,
        ) else {
            continue;
        };
        let activity_ends_at = activity_starts_at.saturating_add(u64::from(duration_seconds));
        let interval_start = activity_starts_at.max(replay_starts_at);
        let interval_end = activity_ends_at.min(ends_before);
        if interval_start >= interval_end
            || duration_seconds == 0
            || !valid_nonnegative(applied_water_millimeters)
        {
            continue;
        }
        if intervals.len() >= MAX_REPORT_BALANCE_INTERVALS_PER_ZONE {
            return Err(());
        }
        intervals.push(BalanceRateInterval {
            starts_at: interval_start,
            ends_before: interval_end,
            deficit_millimeters_per_second: -applied_water_millimeters / duration_seconds as f32,
        });
    }
    let deficit_at_start = replay_deficit_points(
        anchor.opening_deficit_millimeters,
        anchor.capacity_millimeters,
        replay_starts_at,
        starts_at,
        &intervals,
    )
    .last()
    .map(|(_, deficit)| *deficit)
    .unwrap_or(anchor.opening_deficit_millimeters);
    let points = replay_deficit_points(
        deficit_at_start,
        anchor.capacity_millimeters,
        starts_at,
        ends_before,
        &intervals,
    );
    if points.len() > MAX_REPORT_POINTS_PER_PATH {
        return Err(());
    }
    Ok(points
        .into_iter()
        .map(|(at, deficit)| {
            (
                at,
                calculated_available_water_percent(anchor.capacity_millimeters, deficit),
            )
        })
        .collect())
}

fn activity_display_interval(
    activity: &SprinklerWateringActivityV1,
) -> Option<(LibertasDateTime, LibertasDateTime)> {
    let starts_at = activity.actual_starts_at.or(activity.scheduled_starts_at)?;
    let duration = activity
        .actual_duration_seconds
        .or(activity.scheduled_duration_seconds)
        .unwrap_or_else(|| {
            u32::try_from(activity.updated_at.saturating_sub(starts_at)).unwrap_or(u32::MAX)
        })
        .max(60);
    Some((starts_at, starts_at.saturating_add(u64::from(duration))))
}

fn build_water_balance_chart(
    zones: &[ReportZoneData],
    history: &[SprinklerWeatherHistoryPeriodV1],
    range: SprinklerReportTimeRangeV1,
) -> Result<SprinklerWaterBalanceChartV1, ()> {
    // The balance anchor can begin at the UTC-day boundary before the visible
    // left edge, so retain every loaded provider interval for that replay.
    let provider_intervals = merged_report_provider_intervals(history, 0, range.ends_before);
    let mut rows = Vec::new();
    for zone in zones {
        let points = water_balance_points(zone, history, &provider_intervals, range)?;
        let additional_rows = points.len().checked_add(6).ok_or(())?;
        if rows
            .len()
            .checked_add(additional_rows)
            .is_none_or(|total| total > MAX_REPORT_CHART_ROWS)
        {
            return Err(());
        }
        rows.extend(points.into_iter().map(|(at, available_water_percent)| {
            SprinklerWaterBalancePointV1 {
                at,
                available_water_percent,
                series: SprinklerWaterBalanceSeriesV1::AvailableWater,
                zone: zone.valve,
            }
        }));
        for (available_water_percent, series) in [
            (100.0, SprinklerWaterBalanceSeriesV1::FieldCapacity),
            (
                (1.0 - TARGET_DEFICIT_RATIO) * 100.0,
                SprinklerWaterBalanceSeriesV1::WateringThreshold,
            ),
            (
                (1.0 - CRITICAL_DEFICIT_RATIO) * 100.0,
                SprinklerWaterBalanceSeriesV1::CriticalThreshold,
            ),
        ] {
            for at in [range.starts_at, range.ends_before] {
                rows.push(SprinklerWaterBalancePointV1 {
                    at,
                    available_water_percent,
                    series,
                    zone: zone.valve,
                });
            }
        }
    }
    Ok(rows)
}

fn build_watering_timeline(
    zones: &[ReportZoneData],
    range: SprinklerReportTimeRangeV1,
) -> SprinklerWateringTimelineChartV1 {
    let mut activities = Vec::new();
    for zone in zones {
        for activity in &zone.activities {
            let Some((starts_at, ends_at)) = activity_display_interval(activity) else {
                continue;
            };
            let starts_at = starts_at.max(range.starts_at);
            let ends_at = ends_at.min(range.ends_before);
            if starts_at >= ends_at {
                continue;
            }
            activities.push(SprinklerWateringTimelineRowV1 {
                starts_at,
                ends_at,
                zone: zone.valve,
                outcome: activity.outcome,
                origin: activity.origin,
                reason: activity.reason,
                scheduled_duration_seconds: activity.scheduled_duration_seconds.unwrap_or_default(),
                actual_duration_seconds: activity.actual_duration_seconds.unwrap_or_default(),
                activity_key: alloc::format!("{}:{}", zone.valve, activity.activity_index),
            });
        }
    }
    activities.sort_by(|left, right| {
        left.starts_at
            .cmp(&right.starts_at)
            .then(left.zone.cmp(&right.zone))
            .then(left.activity_key.cmp(&right.activity_key))
    });
    let empty_zones = zones
        .iter()
        .filter(|zone| !activities.iter().any(|row| row.zone == zone.valve))
        .map(|zone| SprinklerTimelineEmptyZoneRowV1 {
            horizontal_center: true,
            zone: zone.valve,
            empty_state: SprinklerReportEmptyStateV1::NoRecordedWateringActivity,
        })
        .collect();
    SprinklerWateringTimelineChartV1 {
        activities,
        empty_zones,
    }
}

fn usage_bucket_bounds(
    at: LibertasDateTime,
    bucket: SprinklerReportBucketV1,
) -> (LibertasDateTime, LibertasDateTime) {
    let day = utc_day_start(at);
    match bucket {
        SprinklerReportBucketV1::Day => (day, day.saturating_add(SECONDS_PER_DAY)),
        SprinklerReportBucketV1::Week => {
            let unix_day = day / SECONDS_PER_DAY;
            let days_since_monday = (unix_day + 3) % 7;
            let starts_at = day.saturating_sub(days_since_monday * SECONDS_PER_DAY);
            (starts_at, starts_at.saturating_add(7 * SECONDS_PER_DAY))
        }
    }
}

struct UsageAccumulator {
    starts_at: LibertasDateTime,
    ends_at: LibertasDateTime,
    zone: LibertasDevice,
    rain: f32,
    irrigation: f32,
    forecast_rain: f32,
    scheduled_water: f32,
}

fn water_usage_total_millimeters(total: &UsageAccumulator) -> Option<f64> {
    let amount = f64::from(total.rain)
        + f64::from(total.irrigation)
        + f64::from(total.forecast_rain)
        + f64::from(total.scheduled_water);
    (amount > 0.0 && amount.is_finite()).then_some(amount)
}

fn water_usage_common_mark_span_seconds(totals: &[UsageAccumulator]) -> u64 {
    totals
        .iter()
        .filter(|total| water_usage_total_millimeters(total).is_some())
        .map(|total| total.ends_at.saturating_sub(total.starts_at))
        .filter(|span| *span > 0)
        .min()
        .unwrap_or_default()
        .saturating_mul(WATER_USAGE_BUCKET_MARK_SPAN_NUMERATOR)
        / WATER_USAGE_BUCKET_MARK_SPAN_DENOMINATOR
}

fn water_usage_display_duration_seconds(
    amount_millimeters: f32,
    maximum_total_millimeters: f64,
    common_mark_span_seconds: u64,
) -> u64 {
    if amount_millimeters <= 0.0
        || !amount_millimeters.is_finite()
        || maximum_total_millimeters <= 0.0
        || !maximum_total_millimeters.is_finite()
    {
        return 0;
    }
    let bounded_amount = f64::from(amount_millimeters).min(maximum_total_millimeters);
    let duration =
        (common_mark_span_seconds as f64 * bounded_amount / maximum_total_millimeters) as u64;
    duration.min(common_mark_span_seconds)
}

struct WaterInputInterval {
    starts_at: LibertasDateTime,
    duration_seconds: u32,
    amount_millimeters: f32,
    input_type: SprinklerWaterInputTypeV1,
}

fn accumulate_water_interval(
    totals: &mut Vec<UsageAccumulator>,
    zone: LibertasDevice,
    interval: WaterInputInterval,
    bucket: SprinklerReportBucketV1,
    range: SprinklerReportTimeRangeV1,
) {
    let WaterInputInterval {
        starts_at,
        duration_seconds,
        amount_millimeters,
        input_type,
    } = interval;
    if duration_seconds == 0 || amount_millimeters <= 0.0 || !amount_millimeters.is_finite() {
        return;
    }
    let event_ends_at = starts_at.saturating_add(u64::from(duration_seconds));
    let mut represented_at = starts_at.max(range.starts_at);
    let represented_end = event_ends_at.min(range.ends_before);
    while represented_at < represented_end {
        let (bucket_start, bucket_end) = usage_bucket_bounds(represented_at, bucket);
        let segment_end = represented_end.min(bucket_end);
        let segment_amount = amount_millimeters * segment_end.saturating_sub(represented_at) as f32
            / duration_seconds as f32;
        let displayed_start = bucket_start.max(range.starts_at);
        let displayed_end = bucket_end.min(range.ends_before);
        let total = if let Some(total) = totals.iter_mut().find(|total| {
            total.starts_at == displayed_start
                && total.ends_at == displayed_end
                && total.zone == zone
        }) {
            total
        } else {
            totals.push(UsageAccumulator {
                starts_at: displayed_start,
                ends_at: displayed_end,
                zone,
                rain: 0.0,
                irrigation: 0.0,
                forecast_rain: 0.0,
                scheduled_water: 0.0,
            });
            totals.last_mut().unwrap()
        };
        match input_type {
            SprinklerWaterInputTypeV1::Rain => total.rain += segment_amount,
            SprinklerWaterInputTypeV1::Irrigation => total.irrigation += segment_amount,
            SprinklerWaterInputTypeV1::ForecastRain => total.forecast_rain += segment_amount,
            SprinklerWaterInputTypeV1::ScheduledWater => total.scheduled_water += segment_amount,
        }
        represented_at = segment_end;
    }
}

fn accumulate_zone_water_inputs(
    zone: &ReportZoneData,
    history: &[SprinklerWeatherHistoryPeriodV1],
    forecast: Option<&SprinklerWeatherForecastV1>,
    bucket: SprinklerReportBucketV1,
    range: SprinklerReportTimeRangeV1,
    totals: &mut Vec<UsageAccumulator>,
) {
    for period in history {
        accumulate_water_interval(
            totals,
            zone.valve,
            WaterInputInterval {
                starts_at: period.starts_at,
                duration_seconds: period.duration_seconds,
                amount_millimeters: period.precipitation_millimeters,
                input_type: SprinklerWaterInputTypeV1::Rain,
            },
            bucket,
            range,
        );
    }
    if let Some(forecast) = forecast {
        for period in &forecast.periods {
            accumulate_water_interval(
                totals,
                zone.valve,
                WaterInputInterval {
                    starts_at: period.starts_at,
                    duration_seconds: period.duration_seconds,
                    amount_millimeters: period.expected_precipitation_millimeters,
                    input_type: SprinklerWaterInputTypeV1::ForecastRain,
                },
                bucket,
                range,
            );
        }
    }
    for activity in &zone.activities {
        if let (Some(starts_at), Some(duration_seconds), Some(amount_millimeters)) = (
            activity.actual_starts_at,
            activity.actual_duration_seconds,
            activity.applied_water_millimeters,
        ) {
            accumulate_water_interval(
                totals,
                zone.valve,
                WaterInputInterval {
                    starts_at,
                    duration_seconds,
                    amount_millimeters,
                    input_type: SprinklerWaterInputTypeV1::Irrigation,
                },
                bucket,
                range,
            );
        } else if matches!(
            activity.outcome,
            SprinklerWateringOutcomeV1::Scheduled | SprinklerWateringOutcomeV1::CommandPending
        ) && let (Some(starts_at), Some(duration_seconds), Some(amount_millimeters)) = (
            activity.scheduled_starts_at,
            activity.scheduled_duration_seconds,
            activity.planned_water_millimeters,
        ) {
            accumulate_water_interval(
                totals,
                zone.valve,
                WaterInputInterval {
                    starts_at,
                    duration_seconds,
                    amount_millimeters,
                    input_type: SprinklerWaterInputTypeV1::ScheduledWater,
                },
                bucket,
                range,
            );
        }
    }
}

fn build_water_usage(
    zones: &[ReportZoneData],
    history: &[SprinklerWeatherHistoryPeriodV1],
    forecast: Option<&SprinklerWeatherForecastV1>,
    bucket: SprinklerReportBucketV1,
    range: SprinklerReportTimeRangeV1,
) -> SprinklerWaterUsageChartV1 {
    let mut totals: Vec<UsageAccumulator> = Vec::new();
    for zone in zones {
        accumulate_zone_water_inputs(zone, history, forecast, bucket, range, &mut totals);
    }
    totals.sort_by(|left, right| {
        left.zone
            .cmp(&right.zone)
            .then(left.starts_at.cmp(&right.starts_at))
    });
    let maximum_total_millimeters = totals
        .iter()
        .filter_map(water_usage_total_millimeters)
        .fold(0.0_f64, f64::max);
    let common_mark_span_seconds = water_usage_common_mark_span_seconds(&totals);
    let mut inputs = Vec::new();
    for total in totals {
        let Some(_) = water_usage_total_millimeters(&total) else {
            continue;
        };
        let mut display_offset_seconds = 0_u64;
        for (input_type, amount_millimeters) in [
            (SprinklerWaterInputTypeV1::Rain, total.rain),
            (SprinklerWaterInputTypeV1::Irrigation, total.irrigation),
            (SprinklerWaterInputTypeV1::ForecastRain, total.forecast_rain),
            (
                SprinklerWaterInputTypeV1::ScheduledWater,
                total.scheduled_water,
            ),
        ] {
            if amount_millimeters <= 0.0 || !amount_millimeters.is_finite() {
                continue;
            }
            let duration_seconds = water_usage_display_duration_seconds(
                amount_millimeters,
                maximum_total_millimeters,
                common_mark_span_seconds,
            );
            let segment_starts_at = total.starts_at.saturating_add(display_offset_seconds);
            let segment_ends_at = segment_starts_at.saturating_add(duration_seconds);
            inputs.push(SprinklerWaterUsageRowV1 {
                at: total.starts_at,
                segment_starts_at,
                segment_ends_at,
                amount_millimeters,
                input_type,
                zone: total.zone,
            });
            display_offset_seconds = display_offset_seconds
                .saturating_add(duration_seconds)
                .min(common_mark_span_seconds);
        }
    }
    let empty_zones = zones
        .iter()
        .filter(|zone| !inputs.iter().any(|row| row.zone == zone.valve))
        .map(|zone| SprinklerWaterUsageEmptyZoneRowV1 {
            horizontal_center: true,
            zone: zone.valve,
            empty_state: SprinklerReportEmptyStateV1::NoRecordedWaterInput,
        })
        .collect();
    SprinklerWaterUsageChartV1 {
        inputs,
        empty_zones,
    }
}

fn wind_sample_key(at: LibertasDateTime, series: SprinklerWindSeriesV1) -> String {
    alloc::format!("{at}:{series:?}")
}

fn wind_series_order(series: SprinklerWindSeriesV1) -> u8 {
    match series {
        SprinklerWindSeriesV1::HistoricalWind => 0,
        SprinklerWindSeriesV1::HistoricalGust => 1,
        SprinklerWindSeriesV1::CurrentWind => 2,
        SprinklerWindSeriesV1::CurrentGust => 3,
        SprinklerWindSeriesV1::ForecastWind => 4,
        SprinklerWindSeriesV1::ForecastGust => 5,
    }
}

fn et_sample_key(at: LibertasDateTime, source: SprinklerWeatherChartSourceV1) -> String {
    alloc::format!("{at}:{source:?}")
}

fn modeled_et_sample_key(
    zone: LibertasDevice,
    at: LibertasDateTime,
    source: SprinklerWeatherChartSourceV1,
) -> String {
    alloc::format!("{zone}:{at}:{source:?}")
}

fn modeled_weather_chart_source(
    source: SprinklerWaterDemandSourceV1,
) -> SprinklerWeatherChartSourceV1 {
    match source {
        SprinklerWaterDemandSourceV1::RecentLocalWeather => {
            SprinklerWeatherChartSourceV1::RecentWeatherEstimate
        }
        SprinklerWaterDemandSourceV1::LocationAndSeason => {
            SprinklerWeatherChartSourceV1::LocationAndSeasonEstimate
        }
        SprinklerWaterDemandSourceV1::ConservativeDefault => {
            SprinklerWeatherChartSourceV1::ConservativeEstimate
        }
    }
}

fn reserve_report_rows(generated_rows: &mut usize, additional_rows: usize) -> Result<(), ()> {
    let total = generated_rows.checked_add(additional_rows).ok_or(())?;
    if total > MAX_REPORT_CHART_ROWS {
        return Err(());
    }
    *generated_rows = total;
    Ok(())
}

fn build_weather_et_chart(
    balance_history: &[SprinklerWeatherHistoryPeriodV1],
    full_history: &[SprinklerWeatherHistoryPeriodV2],
    observations: &[SprinklerCurrentWeatherV1],
    forecast: Option<&SprinklerWeatherForecastV1>,
    zones: &[ReportZoneData],
    range: SprinklerReportTimeRangeV1,
) -> Result<SprinklerWeatherEtChartV1, ()> {
    let mut reference_evapotranspiration = Vec::new();
    let mut modeled_reference_evapotranspiration = Vec::new();
    let mut temperature = Vec::new();
    let mut relative_humidity = Vec::new();
    let mut wind = Vec::new();
    let mut generated_rows = 0;
    let provider_intervals =
        merged_report_provider_intervals(balance_history, range.starts_at, range.ends_before);
    // Every legacy period still contributes its exact provider ET. Its absent
    // temperature, humidity, and wind are omitted rather than represented as
    // synthetic zero observations.
    for period in balance_history {
        let period_ends_at = period
            .starts_at
            .saturating_add(u64::from(period.duration_seconds));
        let starts_at = period.starts_at.max(range.starts_at);
        let ends_at = period_ends_at.min(range.ends_before);
        if starts_at >= ends_at {
            continue;
        }
        reserve_report_rows(&mut generated_rows, 1)?;
        let represented_fraction = ends_at.saturating_sub(starts_at) as f32
            / f32::max(period.duration_seconds as f32, 1.0);
        let source = SprinklerWeatherChartSourceV1::HistoricalObservation;
        reference_evapotranspiration.push(SprinklerEtRowV1 {
            starts_at,
            ends_at,
            reference_evapotranspiration_millimeters: period
                .reference_evapotranspiration_millimeters
                * represented_fraction,
            source,
            sample_key: et_sample_key(starts_at, source),
        });
    }
    for period in full_history {
        let period_ends_at = period
            .starts_at
            .saturating_add(u64::from(period.duration_seconds));
        let starts_at = period.starts_at.max(range.starts_at);
        let ends_at = period_ends_at.min(range.ends_before);
        if starts_at >= ends_at {
            continue;
        }
        reserve_report_rows(&mut generated_rows, 4)?;
        temperature.push(SprinklerTemperatureRowV1 {
            at: starts_at,
            temperature_celsius: period.temperature_celsius,
            source: SprinklerWeatherChartSourceV1::HistoricalObservation,
        });
        relative_humidity.push(SprinklerHumidityRowV1 {
            at: starts_at,
            relative_humidity_percent: period.relative_humidity_percent,
            source: SprinklerWeatherChartSourceV1::HistoricalObservation,
        });
        for (series, meters_per_second) in [
            (
                SprinklerWindSeriesV1::HistoricalWind,
                period.wind_speed_meters_per_second,
            ),
            (
                SprinklerWindSeriesV1::HistoricalGust,
                period.wind_gust_meters_per_second,
            ),
        ] {
            wind.push(SprinklerWindRowV1 {
                at: starts_at,
                meters_per_second,
                series,
                sample_key: wind_sample_key(starts_at, series),
            });
        }
    }
    for zone in zones {
        for gap in normalized_modeled_weather_gaps(
            &zone.modeled_weather_gaps,
            range.starts_at,
            range.ends_before,
        ) {
            let starts_at = gap.starts_at;
            let ends_at = gap.ends_before;
            // Provider history is authoritative even if a crash happened
            // between accepting its correction and removing the superseded
            // per-zone gap record. The cursor subtraction is bounded by the
            // remaining chart-row budget and never repeatedly rebuilds a
            // fragment vector for every provider period.
            let remaining_rows = MAX_REPORT_CHART_ROWS.saturating_sub(generated_rows);
            let fragments = provider_uncovered_fragments(
                starts_at,
                ends_at,
                &provider_intervals,
                remaining_rows,
            )?;
            reserve_report_rows(&mut generated_rows, fragments.len())?;
            let source = modeled_weather_chart_source(gap.demand_source);
            for (starts_at, ends_at) in fragments {
                modeled_reference_evapotranspiration.push(SprinklerModeledEtRowV1 {
                    starts_at,
                    ends_at,
                    reference_evapotranspiration_millimeters: gap
                        .reference_evapotranspiration_millimeters_per_day
                        * ends_at.saturating_sub(starts_at) as f32
                        / SECONDS_PER_DAY as f32,
                    source,
                    zone: zone.valve,
                    sample_key: modeled_et_sample_key(zone.valve, starts_at, source),
                });
            }
        }
    }
    // Current observations provide higher-frequency temperature, humidity,
    // wind, and gust evidence, including the precise samples that caused a
    // watering decision. Rain and ET remain sourced from completed history so
    // overlapping 15-minute samples cannot double-count water.
    for observation in observations.iter().filter(|observation| {
        observation.valid_at >= range.starts_at && observation.valid_at < range.ends_before
    }) {
        reserve_report_rows(&mut generated_rows, 4)?;
        temperature.push(SprinklerTemperatureRowV1 {
            at: observation.valid_at,
            temperature_celsius: observation.temperature_celsius,
            source: SprinklerWeatherChartSourceV1::CurrentObservation,
        });
        relative_humidity.push(SprinklerHumidityRowV1 {
            at: observation.valid_at,
            relative_humidity_percent: observation.relative_humidity_percent,
            source: SprinklerWeatherChartSourceV1::CurrentObservation,
        });
        for (series, meters_per_second) in [
            (
                SprinklerWindSeriesV1::CurrentWind,
                observation.wind_speed_meters_per_second,
            ),
            (
                SprinklerWindSeriesV1::CurrentGust,
                observation.wind_gust_meters_per_second,
            ),
        ] {
            wind.push(SprinklerWindRowV1 {
                at: observation.valid_at,
                meters_per_second,
                series,
                sample_key: wind_sample_key(observation.valid_at, series),
            });
        }
    }
    if let Some(forecast) = forecast {
        for period in &forecast.periods {
            let period_ends_at = period
                .starts_at
                .saturating_add(u64::from(period.duration_seconds));
            let starts_at = period.starts_at.max(range.starts_at);
            let ends_at = period_ends_at.min(range.ends_before);
            if starts_at >= ends_at {
                continue;
            }
            reserve_report_rows(&mut generated_rows, 5)?;
            let represented_fraction = ends_at.saturating_sub(starts_at) as f32
                / f32::max(period.duration_seconds as f32, 1.0);
            let source = SprinklerWeatherChartSourceV1::Forecast;
            reference_evapotranspiration.push(SprinklerEtRowV1 {
                starts_at,
                ends_at,
                reference_evapotranspiration_millimeters: period
                    .reference_evapotranspiration_millimeters
                    * represented_fraction,
                source,
                sample_key: et_sample_key(starts_at, source),
            });
            temperature.push(SprinklerTemperatureRowV1 {
                at: starts_at,
                temperature_celsius: period.temperature_celsius,
                source: SprinklerWeatherChartSourceV1::Forecast,
            });
            relative_humidity.push(SprinklerHumidityRowV1 {
                at: starts_at,
                relative_humidity_percent: period.relative_humidity_percent,
                source: SprinklerWeatherChartSourceV1::Forecast,
            });
            for (series, meters_per_second) in [
                (
                    SprinklerWindSeriesV1::ForecastWind,
                    period.wind_speed_meters_per_second,
                ),
                (
                    SprinklerWindSeriesV1::ForecastGust,
                    period.wind_gust_meters_per_second,
                ),
            ] {
                wind.push(SprinklerWindRowV1 {
                    at: starts_at,
                    meters_per_second,
                    series,
                    sample_key: wind_sample_key(starts_at, series),
                });
            }
        }
    }
    reference_evapotranspiration.sort_by_key(|row| row.starts_at);
    modeled_reference_evapotranspiration.sort_by(|left, right| {
        left.zone
            .cmp(&right.zone)
            .then(left.starts_at.cmp(&right.starts_at))
    });
    let modeled_et_empty_zones: Vec<_> = zones
        .iter()
        .filter(|zone| {
            !modeled_reference_evapotranspiration
                .iter()
                .any(|row| row.zone == zone.valve)
        })
        .map(|zone| SprinklerFacetedEmptyZoneRowV1 {
            horizontal_center: true,
            vertical_center: true,
            zone: zone.valve,
            empty_state: SprinklerReportEmptyStateV1::NoRecordedModeledEtGap,
        })
        .collect();
    reserve_report_rows(&mut generated_rows, modeled_et_empty_zones.len())?;
    temperature.sort_by_key(|row| row.at);
    relative_humidity.sort_by_key(|row| row.at);
    wind.sort_by(|left, right| {
        left.at
            .cmp(&right.at)
            .then(wind_series_order(left.series).cmp(&wind_series_order(right.series)))
    });
    Ok(SprinklerWeatherEtChartV1 {
        reference_evapotranspiration,
        modeled_reference_evapotranspiration: SprinklerModeledEtChartV1 {
            gaps: modeled_reference_evapotranspiration,
            empty_zones: modeled_et_empty_zones,
        },
        temperature,
        relative_humidity,
        wind,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SprinklerReportChartKind {
    WaterBalance,
    WateringTimeline,
    WaterUsage,
    WeatherEt,
}

impl SprinklerReportChartKind {
    fn includes_forecast(self) -> bool {
        matches!(self, Self::WaterUsage | Self::WeatherEt)
    }

    fn default_span_seconds(self) -> u64 {
        match self {
            Self::WeatherEt => DEFAULT_WEATHER_HISTORY_SECONDS
                .saturating_add(u64::from(SPRINKLER_FORECAST_HORIZON_SECONDS)),
            Self::WaterUsage => DEFAULT_REPORT_RANGE_SECONDS
                .saturating_add(u64::from(SPRINKLER_FORECAST_HORIZON_SECONDS)),
            Self::WaterBalance | Self::WateringTimeline => DEFAULT_REPORT_RANGE_SECONDS,
        }
    }

    fn default_range(self, now: LibertasDateTime) -> SprinklerReportTimeRangeV1 {
        match self {
            Self::WeatherEt => SprinklerReportTimeRangeV1 {
                starts_at: now.saturating_sub(DEFAULT_WEATHER_HISTORY_SECONDS),
                ends_before: now
                    .saturating_add(u64::from(SPRINKLER_FORECAST_HORIZON_SECONDS))
                    .saturating_add(1),
            },
            Self::WaterUsage => SprinklerReportTimeRangeV1 {
                starts_at: now
                    .saturating_add(1)
                    .saturating_sub(DEFAULT_REPORT_RANGE_SECONDS),
                ends_before: now
                    .saturating_add(u64::from(SPRINKLER_FORECAST_HORIZON_SECONDS))
                    .saturating_add(1),
            },
            Self::WaterBalance | Self::WateringTimeline => {
                let ends_before = now.saturating_add(1);
                SprinklerReportTimeRangeV1 {
                    starts_at: ends_before.saturating_sub(DEFAULT_REPORT_RANGE_SECONDS),
                    ends_before,
                }
            }
        }
    }
}

fn resolve_report_range(
    kind: SprinklerReportChartKind,
    starts_at: Option<LibertasDateTime>,
    ends_before: Option<LibertasDateTime>,
    trusted_now: Option<LibertasDateTime>,
) -> Option<SprinklerReportTimeRangeV1> {
    let range = match (starts_at, ends_before) {
        (None, None) => kind.default_range(trusted_now?),
        (Some(starts_at), None) => SprinklerReportTimeRangeV1 {
            starts_at,
            ends_before: starts_at.saturating_add(kind.default_span_seconds()),
        },
        (None, Some(ends_before)) => SprinklerReportTimeRangeV1 {
            starts_at: ends_before.saturating_sub(kind.default_span_seconds()),
            ends_before,
        },
        (Some(starts_at), Some(ends_before)) => SprinklerReportTimeRangeV1 {
            starts_at,
            ends_before,
        },
    };
    valid_report_range(range).then_some(range)
}

fn report_usage_bucket(range: SprinklerReportTimeRangeV1) -> SprinklerReportBucketV1 {
    if range.ends_before.saturating_sub(range.starts_at) <= 14 * SECONDS_PER_DAY {
        SprinklerReportBucketV1::Day
    } else {
        SprinklerReportBucketV1::Week
    }
}

fn report_response_within_chart_limits(response: &SprinklerReportProtocolV1) -> bool {
    match response {
        SprinklerReportProtocolV1::WaterBalanceV1(rows) => {
            if rows.len() > MAX_REPORT_CHART_ROWS {
                return false;
            }
            let mut path_counts: Vec<(LibertasDevice, SprinklerWaterBalanceSeriesV1, usize)> =
                Vec::new();
            for row in rows {
                if let Some((_, _, count)) = path_counts
                    .iter_mut()
                    .find(|(zone, series, _)| *zone == row.zone && *series == row.series)
                {
                    *count += 1;
                    if *count > MAX_REPORT_POINTS_PER_PATH {
                        return false;
                    }
                } else {
                    path_counts.push((row.zone, row.series, 1));
                }
            }
            true
        }
        SprinklerReportProtocolV1::WateringTimelineV1(chart) => chart
            .activities
            .len()
            .checked_add(chart.empty_zones.len())
            .is_some_and(|total| total <= MAX_REPORT_CHART_ROWS),
        SprinklerReportProtocolV1::WaterUsageV1(chart) => chart
            .inputs
            .len()
            .checked_add(chart.empty_zones.len())
            .is_some_and(|total| total <= MAX_REPORT_CHART_ROWS),
        SprinklerReportProtocolV1::WeatherEtV1(chart) => {
            let total_rows = [
                chart.reference_evapotranspiration.len(),
                chart.modeled_reference_evapotranspiration.gaps.len(),
                chart.modeled_reference_evapotranspiration.empty_zones.len(),
                chart.temperature.len(),
                chart.relative_humidity.len(),
                chart.wind.len(),
            ]
            .into_iter()
            .try_fold(0_usize, usize::checked_add);
            if total_rows.is_none_or(|total| total > MAX_REPORT_CHART_ROWS) {
                return false;
            }
            for source in [
                SprinklerWeatherChartSourceV1::HistoricalObservation,
                SprinklerWeatherChartSourceV1::CurrentObservation,
                SprinklerWeatherChartSourceV1::Forecast,
                SprinklerWeatherChartSourceV1::RecentWeatherEstimate,
                SprinklerWeatherChartSourceV1::LocationAndSeasonEstimate,
                SprinklerWeatherChartSourceV1::ConservativeEstimate,
            ] {
                if chart
                    .temperature
                    .iter()
                    .filter(|row| row.source == source)
                    .count()
                    > MAX_REPORT_POINTS_PER_PATH
                    || chart
                        .relative_humidity
                        .iter()
                        .filter(|row| row.source == source)
                        .count()
                        > MAX_REPORT_POINTS_PER_PATH
                {
                    return false;
                }
            }
            for series in [
                SprinklerWindSeriesV1::HistoricalWind,
                SprinklerWindSeriesV1::HistoricalGust,
                SprinklerWindSeriesV1::CurrentWind,
                SprinklerWindSeriesV1::CurrentGust,
                SprinklerWindSeriesV1::ForecastWind,
                SprinklerWindSeriesV1::ForecastGust,
            ] {
                if chart.wind.iter().filter(|row| row.series == series).count()
                    > MAX_REPORT_POINTS_PER_PATH
                {
                    return false;
                }
            }
            true
        }
        _ => false,
    }
}

fn build_sprinkler_report_response(
    kind: SprinklerReportChartKind,
    zones: &[ReportZoneData],
    history: &ReportWeatherPeriods,
    observations: &[SprinklerCurrentWeatherV1],
    forecast: Option<&SprinklerWeatherForecastV1>,
    range: SprinklerReportTimeRangeV1,
) -> Result<SprinklerReportProtocolV1, ()> {
    let response = match kind {
        SprinklerReportChartKind::WaterBalance => SprinklerReportProtocolV1::WaterBalanceV1(
            build_water_balance_chart(zones, &history.balance, range)?,
        ),
        SprinklerReportChartKind::WateringTimeline => {
            SprinklerReportProtocolV1::WateringTimelineV1(build_watering_timeline(zones, range))
        }
        SprinklerReportChartKind::WaterUsage => {
            SprinklerReportProtocolV1::WaterUsageV1(build_water_usage(
                zones,
                &history.balance,
                forecast,
                report_usage_bucket(range),
                range,
            ))
        }
        SprinklerReportChartKind::WeatherEt => {
            SprinklerReportProtocolV1::WeatherEtV1(build_weather_et_chart(
                &history.balance,
                &history.full,
                observations,
                forecast,
                zones,
                range,
            )?)
        }
    };
    if !report_response_within_chart_limits(&response) {
        return Err(());
    }
    Ok(response)
}

fn handle_report_endpoint(
    endpoint: LibertasEndpoint,
    opcode: u8,
    message: LibertasEndpointMessage<SprinklerReportProtocolV1>,
    context: &mut Box<dyn Any>,
    transaction_id: u32,
    peer: u32,
) -> LibertasEndpointHandlerResult {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .unwrap();
    if opcode == OP_ENDPOINT_PEER_DOWN {
        return LibertasEndpointHandlerResult::Handled;
    }
    if opcode != OP_ENDPOINT_REQ {
        return if opcode == OP_ENDPOINT_SUB_REQ {
            LibertasEndpointHandlerResult::Status(LibertasEndpointStandardStatus::InvalidArgument)
        } else {
            LibertasEndpointHandlerResult::Handled
        };
    }
    let (kind, starts_at, ends_before) = match message {
        LibertasEndpointMessage::Data(SprinklerReportProtocolV1::GetWaterBalanceV1 {
            starts_at,
            ends_before,
        }) => (
            SprinklerReportChartKind::WaterBalance,
            starts_at,
            ends_before,
        ),
        LibertasEndpointMessage::Data(SprinklerReportProtocolV1::GetWateringTimelineV1 {
            starts_at,
            ends_before,
        }) => (
            SprinklerReportChartKind::WateringTimeline,
            starts_at,
            ends_before,
        ),
        LibertasEndpointMessage::Data(SprinklerReportProtocolV1::GetWaterUsageV1 {
            starts_at,
            ends_before,
        }) => (SprinklerReportChartKind::WaterUsage, starts_at, ends_before),
        LibertasEndpointMessage::Data(SprinklerReportProtocolV1::GetWeatherEtV1 {
            starts_at,
            ends_before,
        }) => (SprinklerReportChartKind::WeatherEt, starts_at, ends_before),
        _ => return LibertasEndpointHandlerResult::InvalidMessage,
    };
    let trusted_now = utc_seconds();
    // The trusted clock is consulted only for the all-null default. A custom
    // bound is never promoted into a clock value and therefore cannot
    // fabricate live daily balance checkpoints in the future.
    let Some(range) = resolve_report_range(kind, starts_at, ends_before, trusted_now) else {
        if trusted_now.is_none() && starts_at.is_none() && ends_before.is_none() {
            return LibertasEndpointHandlerResult::Status(
                LibertasEndpointStandardStatus::Unavailable,
            );
        }
        return LibertasEndpointHandlerResult::Status(
            LibertasEndpointStandardStatus::InvalidArgument,
        );
    };
    let needs_history = matches!(
        kind,
        SprinklerReportChartKind::WaterBalance
            | SprinklerReportChartKind::WaterUsage
            | SprinklerReportChartKind::WeatherEt
    );
    let needs_activities = matches!(
        kind,
        SprinklerReportChartKind::WaterBalance
            | SprinklerReportChartKind::WateringTimeline
            | SprinklerReportChartKind::WaterUsage
    );
    let needs_modeled_gaps = matches!(
        kind,
        SprinklerReportChartKind::WaterBalance | SprinklerReportChartKind::WeatherEt
    );
    let needs_daily_reports = kind == SprinklerReportChartKind::WaterBalance;
    // A daily checkpoint supplies the opening balance. Replay from that UTC
    // day boundary so a sub-day query still includes every earlier raw event
    // that determines its exact value at the requested left edge.
    let archive_range = SprinklerReportTimeRangeV1 {
        starts_at: if kind == SprinklerReportChartKind::WaterBalance {
            utc_day_start(range.starts_at)
        } else {
            range.starts_at
        },
        ends_before: range.ends_before,
    };

    let (weather_endpoint, weather_generation, live_history, live_current, forecast, mut zones) = {
        let state = shared.borrow();
        let zones = state
            .zones
            .iter()
            .map(|zone| ReportZoneData {
                valve: zone.configuration.valve,
                capacity_millimeters: root_zone_capacity_millimeters(&zone.configuration),
                crop_coefficient: plant_profile(zone.configuration.plant_type).crop_coefficient,
                active_state: zone.active_state.clone(),
                water_events: if needs_activities {
                    zone.water_events.clone()
                } else {
                    Vec::new()
                },
                modeled_weather_gaps: if needs_modeled_gaps {
                    zone.modeled_weather_gaps.clone()
                } else {
                    Vec::new()
                },
                current_activity: needs_activities
                    .then(|| zone.current_activity.clone())
                    .flatten(),
                activities: Vec::new(),
                daily_reports: if needs_daily_reports {
                    trusted_now
                        .map(|now| build_daily_reports(zone, state.site_location, now))
                        .unwrap_or_default()
                } else {
                    Vec::new()
                },
            })
            .collect::<Vec<_>>();
        (
            state.weather_endpoint,
            state.report_weather_archive_state.generation,
            needs_history
                .then(|| state.weather.history.clone())
                .flatten(),
            (kind == SprinklerReportChartKind::WeatherEt)
                .then_some(state.weather.current)
                .flatten(),
            kind.includes_forecast()
                .then(|| state.weather.forecast.clone())
                .flatten(),
            zones,
        )
    };

    let mut history = if needs_history {
        match load_report_weather_periods(weather_endpoint, weather_generation, archive_range) {
            Ok(history) => history,
            Err(()) => {
                return LibertasEndpointHandlerResult::Status(
                    LibertasEndpointStandardStatus::ResourceExhausted,
                );
            }
        }
    } else {
        ReportWeatherPeriods::default()
    };
    if let Some(live_history) = live_history {
        for period in live_history.periods.into_iter().filter(|period| {
            period.starts_at < archive_range.ends_before
                && period
                    .starts_at
                    .saturating_add(u64::from(period.duration_seconds))
                    > archive_range.starts_at
        }) {
            if let Some(saved) = history
                .full
                .iter_mut()
                .find(|saved| saved.starts_at == period.starts_at)
            {
                *saved = period;
            } else {
                history.full.push(period);
            }
            let balance_period = period.into();
            if let Some(saved) = history
                .balance
                .iter_mut()
                .find(|saved| saved.starts_at == period.starts_at)
            {
                *saved = balance_period;
            } else {
                history.balance.push(balance_period);
            }
        }
        history.full.sort_by_key(|period| period.starts_at);
        history.balance.sort_by_key(|period| period.starts_at);
    }
    if history.balance.len() > MAX_REPORT_WEATHER_PERIODS
        || history.full.len() > MAX_REPORT_WEATHER_PERIODS
    {
        return LibertasEndpointHandlerResult::Status(
            LibertasEndpointStandardStatus::ResourceExhausted,
        );
    }
    let mut observations = if kind == SprinklerReportChartKind::WeatherEt {
        match load_report_weather_observations(weather_endpoint, weather_generation, range) {
            Ok(observations) => observations,
            Err(()) => {
                return LibertasEndpointHandlerResult::Status(
                    LibertasEndpointStandardStatus::ResourceExhausted,
                );
            }
        }
    } else {
        Vec::new()
    };
    if let Some(live_current) = live_current
        && live_current.valid_at >= range.starts_at
        && live_current.valid_at < range.ends_before
    {
        if let Some(saved) = observations
            .iter_mut()
            .find(|saved| saved.valid_at == live_current.valid_at)
        {
            *saved = live_current;
        } else {
            observations.push(live_current);
            observations.sort_by_key(|observation| observation.valid_at);
        }
    }
    if observations.len() > MAX_REPORT_WEATHER_OBSERVATIONS {
        return LibertasEndpointHandlerResult::Status(
            LibertasEndpointStandardStatus::ResourceExhausted,
        );
    }
    let mut remaining_activities = MAX_REPORT_ACTIVITIES;
    let mut remaining_activity_records_scanned = MAX_REPORT_ACTIVITY_RECORDS_SCANNED;
    let mut remaining_modeled_gaps = MAX_REPORT_MODELED_GAPS;
    for zone in &mut zones {
        if needs_modeled_gaps {
            let live_modeled_weather_gaps = core::mem::take(&mut zone.modeled_weather_gaps);
            zone.modeled_weather_gaps = match load_modeled_weather_gaps(
                zone.valve,
                archive_range.starts_at,
                archive_range.ends_before,
                remaining_modeled_gaps,
            ) {
                Ok(gaps) => gaps,
                Err(()) => {
                    return LibertasEndpointHandlerResult::Status(
                        LibertasEndpointStandardStatus::ResourceExhausted,
                    );
                }
            };
            for gap in live_modeled_weather_gaps.into_iter().filter(|gap| {
                gap.starts_at < archive_range.ends_before
                    && gap.ends_before > archive_range.starts_at
            }) {
                if let Some(saved) = zone
                    .modeled_weather_gaps
                    .iter_mut()
                    .find(|saved| saved.starts_at == gap.starts_at)
                {
                    *saved = gap;
                } else {
                    zone.modeled_weather_gaps.push(gap);
                }
            }
            zone.modeled_weather_gaps.sort_by_key(|gap| gap.starts_at);
            if zone.modeled_weather_gaps.len() > remaining_modeled_gaps {
                return LibertasEndpointHandlerResult::Status(
                    LibertasEndpointStandardStatus::ResourceExhausted,
                );
            }
            remaining_modeled_gaps -= zone.modeled_weather_gaps.len();
        }
        if needs_daily_reports {
            let live_daily_reports = core::mem::take(&mut zone.daily_reports);
            zone.daily_reports = match load_report_daily_records(zone.valve, range) {
                Ok(reports) => reports,
                Err(()) => {
                    return LibertasEndpointHandlerResult::Status(
                        LibertasEndpointStandardStatus::ResourceExhausted,
                    );
                }
            };
            for report in live_daily_reports.into_iter().filter(|report| {
                report.starts_at < range.ends_before && report.ends_before > range.starts_at
            }) {
                if let Some(saved) = zone
                    .daily_reports
                    .iter_mut()
                    .find(|saved| saved.starts_at == report.starts_at)
                {
                    *saved = report;
                } else {
                    zone.daily_reports.push(report);
                }
            }
            zone.daily_reports.sort_by_key(|report| report.starts_at);
            if zone.daily_reports.len() > MAX_REPORT_DAILY_RECORDS_PER_ZONE {
                return LibertasEndpointHandlerResult::Status(
                    LibertasEndpointStandardStatus::ResourceExhausted,
                );
            }
        }
        if needs_activities {
            zone.activities = match load_report_activities(
                zone.valve,
                archive_range,
                remaining_activities,
                &mut remaining_activity_records_scanned,
            ) {
                Ok(activities) => activities,
                Err(()) => {
                    return LibertasEndpointHandlerResult::Status(
                        LibertasEndpointStandardStatus::ResourceExhausted,
                    );
                }
            };
            merge_runtime_activity(zone, archive_range);
            if zone.activities.len() > remaining_activities {
                return LibertasEndpointHandlerResult::Status(
                    LibertasEndpointStandardStatus::ResourceExhausted,
                );
            }
            remaining_activities -= zone.activities.len();
        }
    }
    let response = match build_sprinkler_report_response(
        kind,
        &zones,
        &history,
        &observations,
        forecast.as_ref(),
        range,
    ) {
        Ok(response) => response,
        Err(()) => {
            return LibertasEndpointHandlerResult::Status(
                LibertasEndpointStandardStatus::ResourceExhausted,
            );
        }
    };
    libertas_endpoint_response(endpoint, &response, transaction_id, peer);
    LibertasEndpointHandlerResult::Handled
}

fn handle_zone_endpoint(
    endpoint: LibertasEndpoint,
    opcode: u8,
    message: LibertasEndpointMessage<SprinklerZoneProtocolV1>,
    context: &mut Box<dyn Any>,
    transaction_id: u32,
    peer: u32,
) -> LibertasEndpointHandlerResult {
    let context = context.downcast_mut::<ZoneContext>().unwrap();
    if opcode == OP_ENDPOINT_PEER_DOWN {
        // The host removes this opaque in-memory route after the callback.
        return LibertasEndpointHandlerResult::Handled;
    }
    if opcode != OP_ENDPOINT_REQ && opcode != OP_ENDPOINT_SUB_REQ {
        return LibertasEndpointHandlerResult::Handled;
    }
    let LibertasEndpointMessage::Data(message) = message else {
        return LibertasEndpointHandlerResult::InvalidMessage;
    };

    let is_subscription = opcode == OP_ENDPOINT_SUB_REQ;
    let mut persist = None;
    let mut persist_runtime = None;
    let mut persist_mode = None;
    let mut force_all_reports = false;
    let response_kind;
    match message {
        SprinklerZoneProtocolV1::GetStateV1 => {
            response_kind = ZoneResponseKind::State;
        }
        SprinklerZoneProtocolV1::GetAdvancedStateV1 => {
            if is_subscription {
                return LibertasEndpointHandlerResult::Status(
                    LibertasEndpointStandardStatus::InvalidArgument,
                );
            }
            response_kind = ZoneResponseKind::AdvancedState;
        }
        SprinklerZoneProtocolV1::GetConfigurationV1 => {
            if is_subscription {
                return LibertasEndpointHandlerResult::Status(
                    LibertasEndpointStandardStatus::InvalidArgument,
                );
            }
            response_kind = ZoneResponseKind::Configuration;
        }
        SprinklerZoneProtocolV1::SetWaterAmountAdjusterV1 { watering_percent } => {
            if is_subscription || !valid_watering_percent(watering_percent) {
                return LibertasEndpointHandlerResult::Status(
                    LibertasEndpointStandardStatus::InvalidArgument,
                );
            }
            let now_ticks = libertas_get_sys_ticks();
            let now_utc = utc_seconds();
            let mut state = context.shared.borrow_mut();
            let site_location = state.site_location;
            let zone = &mut state.zones[context.zone_index];
            if zone.memory.watering_percent != watering_percent {
                let previous_memory = zone.memory.clone();
                let previous_events = zone.water_events.clone();
                account_open_zone(zone, now_ticks, now_utc, site_location);
                zone.memory.watering_percent = watering_percent;
                persist_runtime = Some((
                    zone.configuration.valve,
                    previous_memory,
                    zone.memory.clone(),
                    previous_events,
                    zone.water_events.clone(),
                    core::mem::take(&mut zone.finalized_daily_reports),
                ));
            }
            response_kind = ZoneResponseKind::Configuration;
        }
        SprinklerZoneProtocolV1::ReplaceHoldOffPeriodsV1 { hold_off_periods } => {
            if is_subscription {
                return LibertasEndpointHandlerResult::Status(
                    LibertasEndpointStandardStatus::InvalidArgument,
                );
            }
            let Ok(hold_off_periods) = normalize_hold_offs(hold_off_periods) else {
                return LibertasEndpointHandlerResult::Status(
                    LibertasEndpointStandardStatus::InvalidArgument,
                );
            };
            let mut state = context.shared.borrow_mut();
            let zone = &mut state.zones[context.zone_index];
            zone.memory.hold_off_periods = hold_off_periods;
            persist = Some((zone.configuration.valve, zone.memory.clone()));
            response_kind = ZoneResponseKind::Configuration;
        }
        SprinklerZoneProtocolV1::SetWateringModeV1 { mode } => {
            if is_subscription {
                return LibertasEndpointHandlerResult::Status(
                    LibertasEndpointStandardStatus::InvalidArgument,
                );
            }
            let mut state = context.shared.borrow_mut();
            if state.watering_mode != mode {
                state.watering_mode = mode;
                persist_mode = Some((state.weather_endpoint, mode));
                force_all_reports = true;
            }
            response_kind = ZoneResponseKind::AdvancedState;
        }
        SprinklerZoneProtocolV1::StateV1 { .. }
        | SprinklerZoneProtocolV1::AdvancedStateV1 { .. }
        | SprinklerZoneProtocolV1::ConfigurationV1 { .. } => {
            return LibertasEndpointHandlerResult::InvalidMessage;
        }
    }
    if let Some((valve, memory)) = persist {
        persist_zone_memory(valve, &memory);
    }
    if let Some((valve, previous_memory, memory, previous_events, water_events, daily_reports)) =
        persist_runtime
    {
        persist_daily_reports(valve, &daily_reports);
        persist_zone_runtime_change(
            valve,
            &previous_memory,
            &memory,
            &previous_events,
            &water_events,
        );
    }
    if let Some((weather_endpoint, mode)) = persist_mode {
        persist_watering_mode(weather_endpoint, mode);
    }

    let mut outcome = evaluate_controller(&context.shared);
    if force_all_reports {
        let zone_count = context.shared.borrow().zones.len();
        for zone_index in 0..zone_count {
            if !outcome.changed_zones.contains(&zone_index) {
                outcome.changed_zones.push(zone_index);
            }
        }
    }
    let response = {
        let controller = context.shared.borrow();
        let zone = &controller.zones[context.zone_index];
        match response_kind {
            ZoneResponseKind::State => SprinklerZoneProtocolV1::StateV1 {
                state: public_zone_state(zone, controller.watering_mode),
            },
            ZoneResponseKind::AdvancedState => SprinklerZoneProtocolV1::AdvancedStateV1 {
                mode: controller.watering_mode,
                state: public_zone_advanced_state(zone, controller.watering_mode),
            },
            ZoneResponseKind::Configuration => SprinklerZoneProtocolV1::ConfigurationV1 {
                configuration: public_zone_configuration(zone),
            },
        }
    };
    libertas_endpoint_response(endpoint, &response, transaction_id, peer);
    dispatch_evaluation(&context.shared, outcome);
    LibertasEndpointHandlerResult::Handled
}

fn upsert_history_periods(
    history: &mut Option<SprinklerWeatherHistoryV2>,
    retrieved_at: LibertasDateTime,
    valid_until: LibertasDateTime,
    periods: Vec<SprinklerWeatherHistoryPeriodV2>,
) {
    let history = history.get_or_insert(SprinklerWeatherHistoryV2 {
        retrieved_at,
        valid_until,
        periods: Vec::new(),
    });
    history.retrieved_at = retrieved_at;
    history.valid_until = valid_until;
    for period in periods {
        if let Some(existing) = history
            .periods
            .iter_mut()
            .find(|existing| existing.starts_at == period.starts_at)
        {
            *existing = period;
        } else {
            history.periods.push(period);
        }
    }
    history.periods.sort_by_key(|period| period.starts_at);
}

fn upsert_forecast_periods(
    forecast: &mut Option<SprinklerWeatherForecastV1>,
    retrieved_at: LibertasDateTime,
    valid_until: LibertasDateTime,
    periods: Vec<SprinklerWeatherForecastPeriodV1>,
) {
    let forecast = forecast.get_or_insert(SprinklerWeatherForecastV1 {
        retrieved_at,
        valid_until,
        periods: Vec::new(),
    });
    forecast.retrieved_at = retrieved_at;
    forecast.valid_until = valid_until;
    for period in periods {
        if let Some(existing) = forecast
            .periods
            .iter_mut()
            .find(|existing| existing.starts_at == period.starts_at)
        {
            *existing = period;
        } else {
            forecast.periods.push(period);
        }
    }
    forecast.periods.sort_by_key(|period| period.starts_at);
}

fn apply_weather_change(
    snapshot: &mut SprinklerWeatherSnapshotV2,
    change: SprinklerWeatherChangeV1,
) {
    match change {
        SprinklerWeatherChangeV1::HistoryPeriodsUpsertV2 {
            retrieved_at,
            valid_until,
            periods,
        } => upsert_history_periods(&mut snapshot.history, retrieved_at, valid_until, periods),
        SprinklerWeatherChangeV1::HistoryPeriodsRemoveV1 { range } => {
            if let Some(history) = &mut snapshot.history {
                history.periods.retain(|period| {
                    !(range.starts_at..range.ends_before).contains(&period.starts_at)
                });
            }
        }
        SprinklerWeatherChangeV1::CurrentReplaceV1 { current } => {
            snapshot.current = Some(current);
        }
        SprinklerWeatherChangeV1::ForecastPeriodsUpsertV1 {
            retrieved_at,
            valid_until,
            periods,
        } => upsert_forecast_periods(&mut snapshot.forecast, retrieved_at, valid_until, periods),
        SprinklerWeatherChangeV1::ForecastPeriodsRemoveV1 { range } => {
            if let Some(forecast) = &mut snapshot.forecast {
                forecast.periods.retain(|period| {
                    !(range.starts_at..range.ends_before).contains(&period.starts_at)
                });
            }
        }
        SprinklerWeatherChangeV1::SectionClearV1 { section } => match section {
            SprinklerWeatherSectionV1::History => snapshot.history = None,
            SprinklerWeatherSectionV1::Current => snapshot.current = None,
            SprinklerWeatherSectionV1::Forecast => snapshot.forecast = None,
        },
        SprinklerWeatherChangeV1::HistoryReplaceV2 { history } => {
            snapshot.history = Some(history);
        }
        SprinklerWeatherChangeV1::ForecastReplaceV1 { forecast } => {
            snapshot.forecast = Some(forecast);
        }
        SprinklerWeatherChangeV1::SiteReplaceV1 { .. }
        | SprinklerWeatherChangeV1::HistoryPeriodsUpsertV1 { .. }
        | SprinklerWeatherChangeV1::HistoryReplaceV1 { .. } => {}
    }
}

fn synchronize_weather_memories(shared: &Rc<RefCell<ControllerState>>) {
    let now = utc_seconds().unwrap_or_default();
    let changed = {
        let mut state = shared.borrow_mut();
        let history = state.weather.history.clone();
        let site_location = state.site_location;
        let mut changed = Vec::new();
        for zone in &mut state.zones {
            let previous_memory = zone.memory.clone();
            let previous_events = zone.water_events.clone();
            if synchronize_history(&zone.memory, &mut zone.water_events, history.as_ref()) {
                let modeled_gap_change =
                    reconcile_zone_modeled_weather_gaps(zone, site_location, now);
                let mut daily_reports = prune_water_events(
                    &mut zone.memory,
                    &mut zone.water_events,
                    &mut zone.modeled_weather_gaps,
                    &zone.configuration,
                    site_location,
                    now,
                );
                for report in build_daily_reports(zone, site_location, now) {
                    if let Some(saved) = daily_reports
                        .iter_mut()
                        .find(|saved| saved.starts_at == report.starts_at)
                    {
                        *saved = report;
                    } else {
                        daily_reports.push(report);
                    }
                }
                changed.push((
                    zone.configuration.valve,
                    previous_memory,
                    zone.memory.clone(),
                    previous_events,
                    zone.water_events.clone(),
                    daily_reports,
                    modeled_gap_change,
                ));
            }
        }
        changed
    };
    for (
        valve,
        previous_memory,
        memory,
        previous_events,
        water_events,
        daily_reports,
        modeled_gap_change,
    ) in changed
    {
        if let Some(change) = modeled_gap_change {
            change.submit();
        }
        persist_daily_reports(valve, &daily_reports);
        persist_zone_runtime_change(
            valve,
            &previous_memory,
            &memory,
            &previous_events,
            &water_events,
        );
    }
}

fn clear_recent_weather_memories(shared: &Rc<RefCell<ControllerState>>) {
    let changed = {
        let mut state = shared.borrow_mut();
        let mut changed = Vec::new();
        for zone in &mut state.zones {
            if !zone
                .water_events
                .iter()
                .any(|event| matches!(event, SprinklerWaterEventV1::WeatherV1 { .. }))
            {
                continue;
            }
            let previous_memory = zone.memory.clone();
            let previous_events = zone.water_events.clone();
            zone.water_events
                .retain(|event| matches!(event, SprinklerWaterEventV1::IrrigationV1 { .. }));
            changed.push((
                zone.configuration.valve,
                previous_memory,
                zone.memory.clone(),
                previous_events,
                zone.water_events.clone(),
            ));
        }
        changed
    };
    for (valve, previous_memory, memory, previous_events, water_events) in changed {
        persist_zone_runtime_change(
            valve,
            &previous_memory,
            &memory,
            &previous_events,
            &water_events,
        );
    }
}

fn accept_weather_report(
    shared: &Rc<RefCell<ControllerState>>,
    report: SprinklerWeatherIncrementalReportV1,
) -> bool {
    if !report.changes.iter().all(valid_weather_change) {
        return false;
    }
    let (
        weather_endpoint,
        cursor,
        initial_archive_state,
        initial_site_location,
        hub_location_subscription_ready,
        initial_weather,
    ) = {
        let state = shared.borrow();
        (
            state.weather_endpoint,
            state.weather_cursor,
            state.report_weather_archive_state,
            state.site_location,
            state.hub_location_subscription_ready,
            state.weather.clone(),
        )
    };
    let Some(cursor) = cursor else {
        return false;
    };
    if !report.can_apply_after(cursor) {
        return false;
    }

    let Some(site_transition) =
        transition_report_weather_sites(initial_archive_state, &report.changes)
    else {
        return false;
    };
    if site_transition.archive_state.location.is_none()
        || !provider_site_matches_authoritative_hub(
            site_transition.archive_state.location,
            initial_site_location,
            hub_location_subscription_ready,
        )
    {
        return false;
    }
    let replacement_site = report.changes.iter().rev().find_map(|change| match change {
        SprinklerWeatherChangeV1::SiteReplaceV1 { location } => Some(*location),
        _ => None,
    });
    let next_site_location = replacement_site
        .or(initial_site_location)
        .or(site_transition.archive_state.location);
    let next_weather = apply_weather_report_changes(initial_weather, &report.changes);

    // Database submissions precede the in-memory cursor and weather commit so
    // a new-site snapshot is never exposed before its controller and archive
    // bindings have been submitted for persistence.
    if let Some(location) = replacement_site.or_else(|| {
        initial_site_location
            .is_none()
            .then_some(site_transition.archive_state.location)
            .flatten()
    }) {
        persist_site_location(weather_endpoint, location);
    }
    if site_transition.archive_state != initial_archive_state {
        persist_report_weather_archive_state(weather_endpoint, site_transition.archive_state);
    }
    let mut state = shared.borrow_mut();
    state.report_weather_archive_state = site_transition.archive_state;
    state.site_location = next_site_location;
    state.weather = next_weather;
    state.weather_cursor = Some(report.through_cursor);
    true
}

fn valid_weather_reset_cursor(
    previous: Option<SprinklerWeatherCursorV1>,
    reason: libertas_weather::SprinklerWeatherResetReasonV1,
    cursor: SprinklerWeatherCursorV1,
) -> bool {
    use libertas_weather::SprinklerWeatherResetReasonV1;

    match (previous, reason) {
        (None, SprinklerWeatherResetReasonV1::InitialSubscription) => true,
        (Some(previous), SprinklerWeatherResetReasonV1::CursorExpired) => {
            cursor.epoch_timestamp == previous.epoch_timestamp
                && cursor.sequence >= previous.sequence
        }
        (Some(previous), SprinklerWeatherResetReasonV1::ServerCursorReset) => {
            cursor.is_server_reset_after(previous)
        }
        _ => false,
    }
}

fn accept_site_bound_weather_reset(
    shared: &Rc<RefCell<ControllerState>>,
    reason: libertas_weather::SprinklerWeatherResetReasonV1,
    cursor: SprinklerWeatherCursorV1,
    location: SprinklerWeatherLocationV1,
    snapshot: SprinklerWeatherSnapshotV2,
) -> bool {
    let (
        weather_endpoint,
        previous_cursor,
        initial_archive_state,
        initial_site_location,
        hub_location_subscription_ready,
    ) = {
        let state = shared.borrow();
        (
            state.weather_endpoint,
            state.weather_cursor,
            state.report_weather_archive_state,
            state.site_location,
            state.hub_location_subscription_ready,
        )
    };
    if !valid_site_location(location)
        || !valid_weather_snapshot_v2(&snapshot)
        || !valid_weather_reset_cursor(previous_cursor, reason, cursor)
        || !provider_site_matches_authoritative_hub(
            Some(location),
            initial_site_location,
            hub_location_subscription_ready,
        )
    {
        return false;
    }
    let Some(site_transition) = transition_report_weather_site(initial_archive_state, location)
    else {
        return false;
    };

    persist_site_location(weather_endpoint, location);
    if site_transition.archive_state != initial_archive_state {
        persist_report_weather_archive_state(weather_endpoint, site_transition.archive_state);
    }
    let mut state = shared.borrow_mut();
    state.report_weather_archive_state = site_transition.archive_state;
    state.site_location = Some(location);
    state.weather_cursor = Some(cursor);
    state.weather = snapshot;
    true
}

fn accept_weather_recovery(
    shared: &Rc<RefCell<ControllerState>>,
    recovery: SprinklerWeatherRecoveryV1,
) -> bool {
    match recovery {
        SprinklerWeatherRecoveryV1::ReplayedV1 { report } => accept_weather_report(shared, report),
        // A legacy reset has no provider-site binding. Accepting even its
        // current or forecast section could mix a previous physical site's
        // weather into the controller after a Hub location change.
        SprinklerWeatherRecoveryV1::ResetV1 { .. } => false,
        SprinklerWeatherRecoveryV1::ResetAtSiteV1 {
            reason,
            cursor,
            location,
            snapshot,
            ..
        } => {
            if !valid_weather_snapshot_v1(&snapshot) {
                return false;
            }
            accept_site_bound_weather_reset(
                shared,
                reason,
                cursor,
                location,
                SprinklerWeatherSnapshotV2 {
                    history: None,
                    current: snapshot.current,
                    forecast: snapshot.forecast,
                },
            )
        }
        SprinklerWeatherRecoveryV1::ResetAtSiteV2 {
            reason,
            cursor,
            location,
            snapshot,
            ..
        } => accept_site_bound_weather_reset(shared, reason, cursor, location, snapshot),
        SprinklerWeatherRecoveryV1::ErrorV1 { .. } => false,
    }
}

fn weather_request(shared: &Rc<RefCell<ControllerState>>) -> SprinklerWeatherProtocolV1 {
    let state = shared.borrow();
    let now = utc_seconds();
    SprinklerWeatherProtocolV1::GetWeatherV1 {
        after_cursor: state.weather_cursor,
        history_range: now.map(|now| SprinklerWeatherTimeRangeV1 {
            starts_at: now.saturating_sub(u64::from(SPRINKLER_HISTORY_WINDOW_SECONDS)),
            ends_before: now,
        }),
        include_current: true,
        forecast_range: now.map(|now| SprinklerWeatherTimeRangeV1 {
            starts_at: now,
            ends_before: now.saturating_add(u64::from(SPRINKLER_FORECAST_HORIZON_SECONDS)),
        }),
    }
}

fn arm_weather_retry(shared: &Rc<RefCell<ControllerState>>, delay_seconds: u32) {
    let (timer, server_up) = {
        let state = shared.borrow();
        (state.weather_retry_timer, state.weather_server_up)
    };
    if timer != 0 {
        if !server_up {
            libertas_timer_cancel(timer);
            return;
        }
        libertas_timer_update_interval(
            timer,
            absolute_interval_ticks(libertas_get_sys_ticks(), delay_seconds.max(1)),
        );
    }
}

fn subscribe_weather(shared: &Rc<RefCell<ControllerState>>) {
    if !shared.borrow().weather_server_up {
        return;
    }
    let endpoint = shared.borrow().weather_endpoint;
    let request = weather_request(shared);
    libertas_endpoint_subscribe_request(endpoint, &request);
    arm_weather_retry(shared, WEATHER_RETRY_SECONDS);
}

fn apply_weather_recovery_error(
    state: &mut ControllerState,
    error: SprinklerWeatherRecoveryErrorV1,
    retry_after_seconds: Option<u32>,
) -> u32 {
    state.weather_stream_ready = false;
    if error == SprinklerWeatherRecoveryErrorV1::CursorAhead {
        state.weather_cursor = None;
        retry_after_seconds.unwrap_or(1)
    } else {
        retry_after_seconds.unwrap_or(WEATHER_RETRY_SECONDS)
    }
}

fn handle_weather_event(
    _endpoint: LibertasEndpoint,
    opcode: u8,
    message: LibertasEndpointMessage<SprinklerWeatherProtocolV1>,
    context: &mut Box<dyn Any>,
    _transaction_id: u32,
    _peer: u32,
) -> LibertasEndpointHandlerResult {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .unwrap();
    if opcode == OP_ENDPOINT_PEER_ALIVE {
        // Signaling only: rearm an established watchdog before any data path.
        if !matches!(message, LibertasEndpointMessage::NoPayload) {
            return LibertasEndpointHandlerResult::InvalidMessage;
        }
        let maximum_wait_seconds = {
            let state = shared.borrow();
            (state.weather_server_up && state.weather_stream_ready)
                .then_some(state.weather_maximum_wait_seconds)
        };
        if let Some(maximum_wait_seconds) = maximum_wait_seconds {
            arm_weather_retry(shared, maximum_wait_seconds);
        }
        return LibertasEndpointHandlerResult::Handled;
    }
    if opcode == OP_ENDPOINT_PEER_DOWN {
        let timer = {
            let mut state = shared.borrow_mut();
            state.weather_stream_ready = false;
            state.weather_server_up = false;
            state.weather_retry_timer
        };
        if timer != 0 {
            libertas_timer_cancel(timer);
        }
        evaluate_and_publish(shared);
        return LibertasEndpointHandlerResult::Handled;
    }
    if opcode == OP_ENDPOINT_PEER_UP {
        // Up can arrive without the preceding Down. It always represents a
        // newer server startup, so the old subscription is no longer ready.
        {
            let mut state = shared.borrow_mut();
            state.weather_stream_ready = false;
            state.weather_server_up = true;
        }
        subscribe_weather(shared);
        return LibertasEndpointHandlerResult::Handled;
    }

    let mut rejected_retry_seconds = WEATHER_RETRY_SECONDS;
    let accepted = match (opcode, message) {
        (
            OP_ENDPOINT_RSP,
            LibertasEndpointMessage::Data(SprinklerWeatherProtocolV1::WeatherRecoveryV1 {
                maximum_wait_interval_seconds,
                recovery,
            }),
        ) if maximum_wait_interval_seconds > 0 => {
            if let SprinklerWeatherRecoveryV1::ErrorV1 {
                error,
                retry_after_seconds,
            } = &recovery
            {
                rejected_retry_seconds = apply_weather_recovery_error(
                    &mut shared.borrow_mut(),
                    *error,
                    *retry_after_seconds,
                );
                false
            } else {
                let archive_recovery = recovery.clone();
                let (previous_current, previous_archive_state) = {
                    let state = shared.borrow();
                    (state.weather.current, state.report_weather_archive_state)
                };
                let accepted = accept_weather_recovery(shared, recovery);
                if accepted {
                    let (weather_endpoint, archive_state) = {
                        let state = shared.borrow();
                        (state.weather_endpoint, state.report_weather_archive_state)
                    };
                    if let SprinklerWeatherRecoveryV1::ReplayedV1 { report } = &archive_recovery {
                        archive_weather_changes_by_site(
                            weather_endpoint,
                            previous_archive_state,
                            previous_current,
                            &report.changes,
                        );
                    } else {
                        archive_weather_recovery(
                            weather_endpoint,
                            archive_state.generation,
                            &archive_recovery,
                        );
                    }
                    let history_cleared = matches!(
                        &archive_recovery,
                        SprinklerWeatherRecoveryV1::ReplayedV1 { report }
                            if report.changes.iter().any(|change| matches!(
                                change,
                                SprinklerWeatherChangeV1::SectionClearV1 {
                                    section: SprinklerWeatherSectionV1::History
                                }
                            ))
                    );
                    if archive_state != previous_archive_state || history_cleared {
                        clear_recent_weather_memories(shared);
                    }
                    shared.borrow_mut().weather_maximum_wait_seconds =
                        maximum_wait_interval_seconds;
                    arm_weather_retry(shared, maximum_wait_interval_seconds);
                }
                accepted
            }
        }
        (
            OP_ENDPOINT_DATA,
            LibertasEndpointMessage::Data(SprinklerWeatherProtocolV1::WeatherIncrementV1 {
                report,
            }),
        ) => {
            let archive_changes = report.changes.clone();
            let (previous_current, previous_archive_state) = {
                let state = shared.borrow();
                (state.weather.current, state.report_weather_archive_state)
            };
            let accepted = accept_weather_report(shared, report);
            if accepted {
                let (weather_endpoint, archive_state) = {
                    let state = shared.borrow();
                    (state.weather_endpoint, state.report_weather_archive_state)
                };
                archive_weather_changes_by_site(
                    weather_endpoint,
                    previous_archive_state,
                    previous_current,
                    &archive_changes,
                );
                let history_cleared = archive_changes.iter().any(|change| {
                    matches!(
                        change,
                        SprinklerWeatherChangeV1::SectionClearV1 {
                            section: SprinklerWeatherSectionV1::History
                        }
                    )
                });
                if archive_state != previous_archive_state || history_cleared {
                    clear_recent_weather_memories(shared);
                }
                let maximum_wait_seconds = shared.borrow().weather_maximum_wait_seconds;
                arm_weather_retry(shared, maximum_wait_seconds);
            }
            accepted
        }
        _ => false,
    };
    if accepted {
        shared.borrow_mut().weather_stream_ready = true;
        synchronize_weather_memories(shared);
        evaluate_and_publish(shared);
    } else {
        shared.borrow_mut().weather_stream_ready = false;
        arm_weather_retry(shared, rejected_retry_seconds);
        evaluate_and_publish(shared);
    }
    LibertasEndpointHandlerResult::Handled
}

fn weather_retry_timer(timer: u32, now_ticks: u64, context: &mut Box<dyn Any>) {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .unwrap();
    if !shared.borrow().weather_server_up {
        libertas_timer_cancel(timer);
        return;
    }
    let endpoint = shared.borrow().weather_endpoint;
    let request = weather_request(shared);
    libertas_endpoint_subscribe_request(endpoint, &request);
    libertas_timer_update_interval(
        timer,
        absolute_interval_ticks(now_ticks, WEATHER_RETRY_SECONDS),
    );
}

fn valve_accounting_timer(timer: u32, now_ticks: u64, context: &mut Box<dyn Any>) {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .unwrap();
    account_all_open_valves(shared);
    refresh_valve_subscriptions(shared, now_ticks);
    libertas_timer_update_interval(
        timer,
        absolute_interval_ticks(now_ticks, VALVE_ACCOUNTING_INTERVAL_SECONDS),
    );
}

fn valve_decision_timer(timer: u32, now_ticks: u64, context: &mut Box<dyn Any>) {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .unwrap();
    let deadline = shared.borrow().valve_decision_not_before_ticks;
    if deadline == 0 {
        libertas_timer_cancel(timer);
    } else if !valve_decision_allowed(now_ticks, deadline) {
        libertas_timer_update_interval(timer, deadline);
    } else {
        shared.borrow_mut().valve_decision_not_before_ticks = 0;
        libertas_timer_cancel(timer);
        evaluate_and_publish(shared);
    }
}

fn schedule_evaluation_timer(timer: u32, now_ticks: u64, context: &mut Box<dyn Any>) {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .unwrap();
    evaluate_and_publish(shared);
    let history_missing = shared.borrow().weather.history.is_none();
    if history_missing && utc_seconds().is_some() {
        subscribe_weather(shared);
    }
    libertas_timer_update_interval(
        timer,
        absolute_interval_ticks(now_ticks, SCHEDULE_EVALUATION_INTERVAL_SECONDS),
    );
}

fn initial_active_state(
    now: LibertasDateTime,
    memory: &SprinklerZoneMemoryV1,
    configuration: &SprinklerZoneV1,
) -> SprinklerZoneActiveStateV1 {
    let demand_estimate = WaterDemandEstimate {
        source: SprinklerWaterDemandSourceV1::ConservativeDefault,
        reference_evapotranspiration_millimeters_per_day:
            CONSERVATIVE_REFERENCE_ET_MILLIMETERS_PER_DAY,
    };
    let deficit = projected_deficit_millimeters(configuration, memory, &[], now, demand_estimate);
    let capacity = root_zone_capacity_millimeters(configuration);
    let crop_coefficient = plant_profile(configuration.plant_type).crop_coefficient;
    let trigger_deficit = capacity * TARGET_DEFICIT_RATIO;
    let (candidate, planning_deficit) = if deficit < trigger_deficit {
        (
            now.saturating_add(seconds_until_deficit(
                deficit,
                trigger_deficit,
                crop_coefficient,
                demand_estimate,
            )),
            trigger_deficit,
        )
    } else {
        (now, deficit)
    };
    let planned_water =
        planned_water_millimeters(capacity, planning_deficit, memory.watering_percent);
    let duration =
        watering_duration_seconds(configuration, planned_water).max(MIN_WATERING_DURATION_SECONDS);
    let active_hold_offs: Vec<_> = memory
        .hold_off_periods
        .iter()
        .copied()
        .filter(|hold_off| hold_off.ends_at().is_some_and(|ends_at| ends_at > now))
        .collect();
    let (starts_at, _) = shift_after_hold_offs(candidate, duration, &active_hold_offs);
    SprinklerZoneActiveStateV1 {
        water_demand_source: demand_estimate.source,
        estimated_reference_evapotranspiration_millimeters_per_day: demand_estimate
            .reference_evapotranspiration_millimeters_per_day,
        calculated_at: now,
        condition: SprinklerScheduleConditionV1::Initializing,
        next_watering: SprinklerTimeSlotV1 {
            starts_at,
            duration_seconds: duration,
        },
        planned_water_millimeters: planned_water,
        estimated_deficit_millimeters: deficit,
        recent_precipitation_millimeters: 0.0,
        recent_irrigation_millimeters: 0.0,
        valve_is_open: false,
        valve_state_known: false,
        valve_fault_bitmap: 0,
    }
}

/// Sprinkler agent
/// Runs a weather-aware multi-zone sprinkler controller. The weather endpoint
/// supplies the tailored sprinkler history, current conditions, and forecast
/// shared by all zones. Each zone exposes an essential state by default,
/// complete advanced details on demand, and persists its own recent-water
/// state.
#[libertas_data_schema(SprinklerDataV1)]
#[libertas_permissions(SPRINKLER_PERMISSIONS)]
#[libertas_string_resources(APP_STRINGS)]
#[libertas_export]
pub fn libertas_sprinkler(
    /*
     * Weather server
     * The client endpoint for `SprinklerWeatherProtocolV1`. The application
     * subscribes at startup for more precise planning. Missing or stale weather
     * falls back to an offline estimate; fresh unsafe conditions delay watering.
     */
    #[libertas_endpoint_schema(SprinklerWeatherProtocolV1)] weather_server: LibertasEndpoint,
    /*
     * Sprinkler Report
     * The system-wide server endpoint for `SprinklerReportProtocolV1`. It
     * returns four chart-ready report families from indefinitely retained
     * weather, activity, and daily balance archives.
     */
    #[libertas_endpoint_schema(SprinklerReportProtocolV1)]
    #[libertas_endpoint_server]
    report_server: LibertasEndpoint,
    /*
     * Reminder recipients
     * One or more Libertas users who receive application reminders. The
     * current version sends winterization reminders; the shared list can also
     * serve future reminder types.
     * #[libertas_size(min=1, max=16)]
     * #[libertas_unordered]
     * ----
     * Reminder recipient
     * One Libertas user authorized to receive sprinkler reminders.
     * #[libertas_unique]
     */
    reminder_recipients: Vec<LibertasUser>,
    /*
     * Sprinkler zones
     * One or more independently scheduled Matter Valve zones.
     * #[libertas_size(min=1, max=32)]
     * ----
     * Sprinkler zone
     * The physical and endpoint configuration for one watered area.
     */
    zones: Vec<SprinklerZoneV1>,
) {
    let weather_endpoint = weather_server;
    let report_endpoint = report_server;
    if !valid_reminder_recipients(&reminder_recipients) {
        libertas_log(
            LogLevel::Error,
            "Reminder recipients must contain 1 to 16 unique users",
        );
        return;
    }
    if !valid_zones(&zones) {
        libertas_log(
            LogLevel::Error,
            "Sprinkler zones must contain 1 to 32 entries with unique valves and state endpoints",
        );
        return;
    }
    if !valid_report_endpoint(report_endpoint, weather_endpoint, &zones) {
        libertas_log(
            LogLevel::Error,
            "Sprinkler Report must differ from the weather and zone state endpoints",
        );
        return;
    }
    let now = utc_seconds().unwrap_or_default();
    let watering_mode = load_watering_mode(weather_endpoint);
    let report_weather_archive_state = load_report_weather_archive_state(weather_endpoint);
    let winterization_reminder = load_winterization_reminder(weather_endpoint);
    let site_location = load_site_location(weather_endpoint);
    let mut runtime_zones = Vec::with_capacity(zones.len());
    for configuration in zones {
        let mut memory = load_zone_memory(configuration.valve, now);
        let mut water_events = load_water_events(configuration.valve, &memory);
        let restored_memory = memory.clone();
        let restored_events = water_events.clone();
        let previous_modeled_weather_gaps = load_modeled_weather_gaps(
            configuration.valve,
            memory.balance_baseline_at,
            now,
            MAX_REPORT_MODELED_GAPS,
        )
        .unwrap_or_default();
        let estimate = water_demand_estimate(&water_events, site_location, now);
        let mut modeled_weather_gaps = reconcile_modeled_weather_gaps(
            &previous_modeled_weather_gaps,
            &water_events,
            memory.balance_baseline_at,
            now,
            estimate,
            now,
        );
        persist_modeled_gap_delta(
            configuration.valve,
            &previous_modeled_weather_gaps,
            &modeled_weather_gaps,
        );
        let finalized_daily_reports = prune_water_events(
            &mut memory,
            &mut water_events,
            &mut modeled_weather_gaps,
            &configuration,
            site_location,
            now,
        );
        persist_daily_reports(configuration.valve, &finalized_daily_reports);
        persist_zone_runtime_change(
            configuration.valve,
            &restored_memory,
            &memory,
            &restored_events,
            &water_events,
        );
        let active_state = initial_active_state(now, &memory, &configuration);
        let current_activity = load_current_watering_activity(configuration.valve);
        let expected_irrigation = current_activity
            .as_ref()
            .and_then(restored_expected_irrigation);
        let runtime = ZoneRuntime {
            configuration,
            memory,
            water_events,
            modeled_weather_gaps,
            active_state,
            valve_state_known: false,
            valve_is_open: false,
            valve_opened_automatically: false,
            valve_fault_bitmap: 0,
            valve_last_report_ticks: None,
            accounted_at_ticks: None,
            accounted_at_utc: None,
            pending_command: None,
            expected_irrigation,
            current_activity,
            finalized_daily_reports: Vec::new(),
        };
        runtime_zones.push(runtime);
    }
    let shared = Rc::new(RefCell::new(ControllerState {
        weather_endpoint,
        report_weather_archive_state,
        reminder_recipients,
        watering_mode,
        winterization_reminder,
        site_location,
        hub_location_server_up: true,
        hub_location_subscription_ready: false,
        site_location_retry_timer: 0,
        weather: SprinklerWeatherSnapshotV2 {
            history: None,
            current: None,
            forecast: None,
        },
        weather_cursor: None,
        weather_stream_ready: false,
        weather_server_up: true,
        weather_maximum_wait_seconds: WEATHER_RETRY_SECONDS,
        weather_retry_timer: 0,
        valve_decision_not_before_ticks: 0,
        valve_decision_timer: 0,
        zones: runtime_zones,
    }));

    let zone_count = shared.borrow().zones.len();
    for zone_index in 0..zone_count {
        let (valve, endpoint) = {
            let state = shared.borrow();
            let zone = &state.zones[zone_index];
            (zone.configuration.valve, zone.configuration.state_endpoint)
        };
        libertas_register_device_listener(
            valve,
            handle_valve_event,
            Box::new(ZoneContext {
                shared: Rc::clone(&shared),
                zone_index,
            }),
        );
        libertas_register_endpoint_status_listener::<SprinklerZoneProtocolV1, _>(
            endpoint,
            handle_zone_endpoint,
            Box::new(ZoneContext {
                shared: Rc::clone(&shared),
                zone_index,
            }),
        );
    }
    libertas_register_endpoint_status_listener::<SprinklerReportProtocolV1, _>(
        report_endpoint,
        handle_report_endpoint,
        Box::new(Rc::clone(&shared)),
    );
    libertas_register_endpoint_status_listener::<SprinklerWeatherProtocolV1, _>(
        weather_endpoint,
        handle_weather_event,
        Box::new(Rc::clone(&shared)),
    );
    libertas_register_endpoint_status_listener::<HubProtocol, _>(
        LIBERTAS_HUB_ENDPOINT,
        handle_site_location_event,
        Box::new(Rc::clone(&shared)),
    );

    let site_location_timer =
        libertas_timer_new_interval(0, site_location_retry_timer, Box::new(Rc::clone(&shared)));
    shared.borrow_mut().site_location_retry_timer = site_location_timer;
    let weather_timer =
        libertas_timer_new_interval(0, weather_retry_timer, Box::new(Rc::clone(&shared)));
    shared.borrow_mut().weather_retry_timer = weather_timer;
    let valve_decision_timer =
        libertas_timer_new_interval(0, valve_decision_timer, Box::new(Rc::clone(&shared)));
    shared.borrow_mut().valve_decision_timer = valve_decision_timer;
    let now_ticks = libertas_get_sys_ticks();
    libertas_timer_new_interval(
        absolute_interval_ticks(now_ticks, VALVE_ACCOUNTING_INTERVAL_SECONDS),
        valve_accounting_timer,
        Box::new(Rc::clone(&shared)),
    );
    libertas_timer_new_interval(
        absolute_interval_ticks(now_ticks, SCHEDULE_EVALUATION_INTERVAL_SECONDS),
        schedule_evaluation_timer,
        Box::new(Rc::clone(&shared)),
    );

    request_valve_subscriptions(&shared);
    request_site_location(&shared);
    subscribe_weather(&shared);
    evaluate_and_publish(&shared);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use libertas_matter::{InlineByteBuffer, decode_command, encode_command};

    const NOW: LibertasDateTime = 1_800_000_000;
    const EQUINOX_DAY_START: LibertasDateTime = 1_773_964_800;

    fn zone() -> SprinklerZoneV1 {
        SprinklerZoneV1 {
            valve: 7,
            plant_type: SprinklerPlantTypeV1::Lawn,
            sprinkler_head_type: SprinklerHeadTypeV1::RotorsLowRate,
            state_endpoint: 17,
        }
    }

    fn memory() -> SprinklerZoneMemoryV1 {
        default_memory(NOW)
    }

    fn current() -> SprinklerCurrentWeatherV1 {
        SprinklerCurrentWeatherV1 {
            retrieved_at: NOW,
            valid_until: NOW + 1_800,
            valid_at: NOW,
            interval_seconds: 900,
            temperature_celsius: 20.0,
            relative_humidity_percent: 70,
            precipitation_millimeters: 0.0,
            reference_evapotranspiration_millimeters: 0.1,
            wind_speed_meters_per_second: 2.0,
            wind_gust_meters_per_second: 4.0,
        }
    }

    fn history() -> SprinklerWeatherHistoryV2 {
        SprinklerWeatherHistoryV2 {
            retrieved_at: NOW,
            valid_until: NOW + 7_200,
            periods: Vec::new(),
        }
    }

    fn report_weather_period(starts_at: LibertasDateTime) -> SprinklerWeatherHistoryPeriodV2 {
        SprinklerWeatherHistoryPeriodV2 {
            starts_at,
            duration_seconds: 3_600,
            temperature_celsius: 21.5,
            relative_humidity_percent: 64,
            precipitation_millimeters: 1.25,
            reference_evapotranspiration_millimeters: 0.3,
            wind_speed_meters_per_second: 3.5,
            wind_gust_meters_per_second: 6.25,
        }
    }

    fn completed_report_activity(starts_at: LibertasDateTime) -> SprinklerWateringActivityV1 {
        SprinklerWateringActivityV1 {
            activity_index: watering_activity_index(
                starts_at,
                SprinklerWateringOriginV1::Automatic,
                0,
            )
            .unwrap(),
            activity_ordinal: 0,
            origin: SprinklerWateringOriginV1::Automatic,
            outcome: SprinklerWateringOutcomeV1::Completed,
            reason: SprinklerWateringReasonV1::SmartSchedule,
            scheduled_starts_at: Some(starts_at),
            scheduled_duration_seconds: Some(600),
            planned_water_millimeters: Some(2.0),
            actual_starts_at: Some(starts_at),
            actual_duration_seconds: Some(540),
            applied_water_millimeters: Some(1.8),
            watering_percent: 100,
            updated_at: starts_at + 540,
        }
    }

    fn completed_daily_report(starts_at: LibertasDateTime) -> SprinklerDailyReportV1 {
        SprinklerDailyReportV1 {
            starts_at,
            ends_before: starts_at + SECONDS_PER_DAY,
            coverage_starts_at: starts_at,
            coverage_ends_before: starts_at + SECONDS_PER_DAY,
            capacity_millimeters: root_zone_capacity_millimeters(&zone()),
            opening_deficit_millimeters: 12.0,
            closing_deficit_millimeters: 9.0,
            precipitation_millimeters: 1.25,
            reference_evapotranspiration_millimeters: 0.3,
            modeled_reference_evapotranspiration_millimeters: 0.0,
            modeled_demand_source: None,
            provider_weather_coverage_seconds: SECONDS_PER_DAY as u32,
            irrigation_millimeters: 1.8,
            complete: true,
            calculated_at: starts_at + SECONDS_PER_DAY,
        }
    }

    fn location() -> SprinklerWeatherLocationV1 {
        SprinklerWeatherLocationV1 {
            longitude_degrees: -74.006,
            latitude_degrees: 40.7128,
        }
    }

    #[test]
    fn reminder_recipients_require_one_to_sixteen_unique_users() {
        assert!(!valid_reminder_recipients(&[]));
        assert!(valid_reminder_recipients(&[1]));
        assert!(valid_reminder_recipients(&[1, 2]));
        assert!(!valid_reminder_recipients(&[1, 1]));

        let maximum: Vec<LibertasUser> = (1..=MAX_REMINDER_RECIPIENTS as LibertasUser).collect();
        assert!(valid_reminder_recipients(&maximum));
        let mut too_many = maximum;
        too_many.push(MAX_REMINDER_RECIPIENTS as LibertasUser + 1);
        assert!(!valid_reminder_recipients(&too_many));
    }

    #[test]
    fn zones_require_one_to_thirty_two_unique_valves_and_endpoints() {
        assert!(!valid_zones(&[]));
        assert!(valid_zones(&[zone()]));

        let mut duplicate_valve = zone();
        duplicate_valve.state_endpoint += 1;
        assert!(!valid_zones(&[zone(), duplicate_valve]));

        let mut duplicate_endpoint = zone();
        duplicate_endpoint.valve += 1;
        assert!(!valid_zones(&[zone(), duplicate_endpoint]));

        let maximum: Vec<_> = (0..MAX_SPRINKLER_ZONES)
            .map(|index| {
                let mut zone = zone();
                zone.valve += index as LibertasDevice;
                zone.state_endpoint += index as LibertasEndpoint;
                zone
            })
            .collect();
        assert!(valid_zones(&maximum));
        let mut too_many = maximum;
        let mut extra = zone();
        extra.valve += MAX_SPRINKLER_ZONES as LibertasDevice;
        extra.state_endpoint += MAX_SPRINKLER_ZONES as LibertasEndpoint;
        too_many.push(extra);
        assert!(!valid_zones(&too_many));
    }

    fn equator_location() -> SprinklerWeatherLocationV1 {
        SprinklerWeatherLocationV1 {
            longitude_degrees: 0.0,
            latitude_degrees: 0.0,
        }
    }

    fn morning_forecast(
        retrieved_at: LibertasDateTime,
        relative_humidity_percent: u8,
    ) -> SprinklerWeatherForecastV1 {
        let mut periods = Vec::new();
        for hour in 4..=10 {
            periods.push(SprinklerWeatherForecastPeriodV1 {
                starts_at: EQUINOX_DAY_START + hour * 3_600,
                duration_seconds: 3_600,
                temperature_celsius: 18.0,
                relative_humidity_percent,
                precipitation_probability_percent: 5,
                expected_precipitation_millimeters: 0.0,
                reference_evapotranspiration_millimeters: 0.02,
                wind_speed_meters_per_second: 1.0,
                wind_gust_meters_per_second: 2.0,
            });
        }
        SprinklerWeatherForecastV1 {
            retrieved_at,
            valid_until: retrieved_at + 10_800,
            periods,
        }
    }

    fn safe_hourly_forecast(starts_at: LibertasDateTime, hours: u64) -> SprinklerWeatherForecastV1 {
        let periods = (0..hours)
            .map(|hour| SprinklerWeatherForecastPeriodV1 {
                starts_at: starts_at + hour * 3_600,
                duration_seconds: 3_600,
                temperature_celsius: 18.0,
                relative_humidity_percent: 60,
                precipitation_probability_percent: 0,
                expected_precipitation_millimeters: 0.0,
                reference_evapotranspiration_millimeters: 0.02,
                wind_speed_meters_per_second: 1.0,
                wind_gust_meters_per_second: 2.0,
            })
            .collect();
        SprinklerWeatherForecastV1 {
            retrieved_at: starts_at,
            valid_until: starts_at + 1_800,
            periods,
        }
    }

    fn current_at(now: LibertasDateTime) -> SprinklerCurrentWeatherV1 {
        SprinklerCurrentWeatherV1 {
            retrieved_at: now,
            valid_until: now + 1_800,
            valid_at: now,
            ..current()
        }
    }

    fn runtime(memory: SprinklerZoneMemoryV1) -> ZoneRuntime {
        let configuration = zone();
        let active_state = initial_active_state(NOW, &memory, &configuration);
        ZoneRuntime {
            configuration,
            memory,
            water_events: Vec::new(),
            modeled_weather_gaps: Vec::new(),
            active_state,
            valve_state_known: true,
            valve_is_open: false,
            valve_opened_automatically: false,
            valve_fault_bitmap: 0,
            valve_last_report_ticks: Some(0),
            accounted_at_ticks: None,
            accounted_at_utc: None,
            pending_command: None,
            expected_irrigation: None,
            current_activity: None,
            finalized_daily_reports: Vec::new(),
        }
    }

    fn controller_state() -> ControllerState {
        ControllerState {
            weather_endpoint: 1,
            report_weather_archive_state: SprinklerReportWeatherArchiveStateV2 {
                generation: 0,
                location: None,
            },
            reminder_recipients: vec![2, 3],
            watering_mode: SprinklerWateringModeV1::Active,
            winterization_reminder: None,
            site_location: None,
            hub_location_server_up: true,
            hub_location_subscription_ready: false,
            site_location_retry_timer: 0,
            weather: SprinklerWeatherSnapshotV2 {
                history: None,
                current: None,
                forecast: None,
            },
            weather_cursor: None,
            weather_stream_ready: true,
            weather_server_up: true,
            weather_maximum_wait_seconds: WEATHER_RETRY_SECONDS,
            weather_retry_timer: 0,
            valve_decision_not_before_ticks: 0,
            valve_decision_timer: 0,
            zones: Vec::new(),
        }
    }

    #[test]
    fn public_protocol_round_trips_through_avro() {
        let current = initial_active_state(NOW, &memory(), &zone());
        let active = SprinklerZoneStateV1::ActiveV1 {
            condition: current.condition,
            next_watering: current.next_watering,
        };
        let winterization = SprinklerZoneStateV1::WinterizationV1;
        for (index, state) in [&active, &winterization].into_iter().enumerate() {
            let encoded = state.to_avro();
            assert_eq!(encoded.first(), Some(&((index as u8) * 2)));
            assert_eq!(SprinklerZoneStateV1::from_avro(&encoded), Ok(state.clone()));
        }

        let advanced_active = SprinklerZoneAdvancedStateV1::ActiveV1 {
            current: current.clone(),
        };
        let advanced_winterization = SprinklerZoneAdvancedStateV1::WinterizationV1;
        for (index, state) in [&advanced_active, &advanced_winterization]
            .into_iter()
            .enumerate()
        {
            let encoded = state.to_avro();
            assert_eq!(encoded.first(), Some(&((index as u8) * 2)));
            assert_eq!(
                SprinklerZoneAdvancedStateV1::from_avro(&encoded),
                Ok(state.clone())
            );
        }
        let configuration = SprinklerZoneConfigurationV1 {
            watering_percent: 80,
            hold_off_periods: vec![SprinklerTimeSlotV1 {
                starts_at: NOW,
                duration_seconds: 600,
            }],
        };
        assert_eq!(
            SprinklerZoneConfigurationV1::from_avro(&configuration.to_avro()),
            Ok(configuration.clone())
        );
        let values = [
            SprinklerZoneProtocolV1::GetStateV1,
            SprinklerZoneProtocolV1::StateV1 { state: active },
            SprinklerZoneProtocolV1::GetAdvancedStateV1,
            SprinklerZoneProtocolV1::AdvancedStateV1 {
                mode: SprinklerWateringModeV1::Active,
                state: advanced_active,
            },
            SprinklerZoneProtocolV1::GetConfigurationV1,
            SprinklerZoneProtocolV1::ConfigurationV1 {
                configuration: configuration.clone(),
            },
            SprinklerZoneProtocolV1::SetWaterAmountAdjusterV1 {
                watering_percent: 80,
            },
            SprinklerZoneProtocolV1::ReplaceHoldOffPeriodsV1 {
                hold_off_periods: configuration.hold_off_periods,
            },
            SprinklerZoneProtocolV1::SetWateringModeV1 {
                mode: SprinklerWateringModeV1::Winterization,
            },
        ];
        for value in values {
            let encoded = value.to_avro();
            assert_eq!(SprinklerZoneProtocolV1::from_avro(&encoded), Ok(value));
        }
    }

    #[test]
    fn report_protocol_round_trips_all_four_chart_families() {
        let day = utc_day_start(NOW);
        let range = SprinklerReportTimeRangeV1 {
            starts_at: day,
            ends_before: day + SECONDS_PER_DAY,
        };
        let requests = [
            SprinklerReportProtocolV1::GetWaterBalanceV1 {
                starts_at: None,
                ends_before: None,
            },
            SprinklerReportProtocolV1::GetWateringTimelineV1 {
                starts_at: None,
                ends_before: None,
            },
            SprinklerReportProtocolV1::GetWaterUsageV1 {
                starts_at: None,
                ends_before: None,
            },
            SprinklerReportProtocolV1::GetWeatherEtV1 {
                starts_at: None,
                ends_before: None,
            },
        ];
        for (index, request) in requests.into_iter().enumerate() {
            let encoded = request.to_avro();
            assert_eq!(encoded, vec![(index as u8) * 4, 0, 0]);
            assert_eq!(SprinklerReportProtocolV1::from_avro(&encoded), Ok(request));
        }
        let custom_request = SprinklerReportProtocolV1::GetWaterUsageV1 {
            starts_at: Some(range.starts_at),
            ends_before: Some(range.ends_before),
        };
        assert_eq!(
            SprinklerReportProtocolV1::from_avro(&custom_request.to_avro()),
            Ok(custom_request)
        );

        let activity = completed_report_activity(day + 3_600);
        let report_zone = ReportZoneData {
            valve: zone().valve,
            capacity_millimeters: root_zone_capacity_millimeters(&zone()),
            crop_coefficient: plant_profile(zone().plant_type).crop_coefficient,
            active_state: runtime(memory()).active_state,
            water_events: Vec::new(),
            modeled_weather_gaps: Vec::new(),
            current_activity: None,
            activities: vec![activity],
            daily_reports: vec![completed_daily_report(day)],
        };
        let full_history = report_weather_period(day + 1_800);
        let history = ReportWeatherPeriods {
            balance: vec![full_history.into()],
            full: vec![full_history],
        };
        let zones = [report_zone];
        let responses = [
            build_sprinkler_report_response(
                SprinklerReportChartKind::WaterBalance,
                &zones,
                &history,
                &[],
                None,
                range,
            ),
            build_sprinkler_report_response(
                SprinklerReportChartKind::WateringTimeline,
                &zones,
                &history,
                &[],
                None,
                range,
            ),
            build_sprinkler_report_response(
                SprinklerReportChartKind::WaterUsage,
                &zones,
                &history,
                &[],
                None,
                range,
            ),
            build_sprinkler_report_response(
                SprinklerReportChartKind::WeatherEt,
                &zones,
                &history,
                &[],
                None,
                range,
            ),
        ]
        .map(|response| response.unwrap());
        let SprinklerReportProtocolV1::WaterBalanceV1(water_balance) = &responses[0] else {
            panic!("expected water-balance response");
        };
        assert!(water_balance.iter().any(|row| {
            row.series == SprinklerWaterBalanceSeriesV1::AvailableWater
                && row.zone == zones[0].valve
        }));
        let SprinklerReportProtocolV1::WateringTimelineV1(watering_timeline) = &responses[1] else {
            panic!("expected timeline response");
        };
        assert_eq!(watering_timeline.activities.len(), 1);
        assert!(watering_timeline.empty_zones.is_empty());
        let SprinklerReportProtocolV1::WaterUsageV1(water_usage) = &responses[2] else {
            panic!("expected water-usage response");
        };
        assert_eq!(water_usage.inputs.len(), 2);
        assert!(water_usage.empty_zones.is_empty());
        let SprinklerReportProtocolV1::WeatherEtV1(weather_et) = &responses[3] else {
            panic!("expected weather/ET response");
        };
        assert_eq!(weather_et.reference_evapotranspiration.len(), 1);
        assert!(
            weather_et
                .modeled_reference_evapotranspiration
                .gaps
                .is_empty()
        );
        assert_eq!(
            weather_et
                .modeled_reference_evapotranspiration
                .empty_zones
                .len(),
            1
        );
        assert_eq!(weather_et.temperature.len(), 1);
        assert_eq!(weather_et.relative_humidity.len(), 1);
        assert_eq!(weather_et.wind.len(), 2);

        for (index, response) in responses.into_iter().enumerate() {
            let encoded = response.to_avro();
            assert_eq!(encoded.first(), Some(&(2 + (index as u8) * 4)));
            assert_eq!(SprinklerReportProtocolV1::from_avro(&encoded), Ok(response));
        }
    }

    #[test]
    fn report_range_is_age_unlimited_but_one_response_is_bounded() {
        let oldest_representable = SprinklerReportTimeRangeV1 {
            starts_at: 1,
            ends_before: 1 + MAX_REPORT_RANGE_SECONDS,
        };
        assert!(valid_report_range(oldest_representable));
        assert!(!valid_report_range(SprinklerReportTimeRangeV1 {
            ends_before: oldest_representable.ends_before + 1,
            ..oldest_representable
        }));
        assert!(!valid_report_range(SprinklerReportTimeRangeV1 {
            starts_at: NOW,
            ends_before: NOW,
        }));
    }

    #[test]
    fn null_report_times_resolve_without_user_input() {
        for kind in [
            SprinklerReportChartKind::WaterBalance,
            SprinklerReportChartKind::WateringTimeline,
        ] {
            assert_eq!(
                resolve_report_range(kind, None, None, Some(NOW)),
                Some(SprinklerReportTimeRangeV1 {
                    starts_at: NOW + 1 - DEFAULT_REPORT_RANGE_SECONDS,
                    ends_before: NOW + 1,
                })
            );
        }
        assert_eq!(
            resolve_report_range(SprinklerReportChartKind::WaterUsage, None, None, Some(NOW)),
            Some(SprinklerReportTimeRangeV1 {
                starts_at: NOW + 1 - DEFAULT_REPORT_RANGE_SECONDS,
                ends_before: NOW + u64::from(SPRINKLER_FORECAST_HORIZON_SECONDS) + 1,
            })
        );
        assert_eq!(
            resolve_report_range(SprinklerReportChartKind::WeatherEt, None, None, Some(NOW),),
            Some(SprinklerReportTimeRangeV1 {
                starts_at: NOW - DEFAULT_WEATHER_HISTORY_SECONDS,
                ends_before: NOW + u64::from(SPRINKLER_FORECAST_HORIZON_SECONDS) + 1,
            })
        );
        assert_eq!(
            resolve_report_range(SprinklerReportChartKind::WaterBalance, Some(1), None, None,),
            Some(SprinklerReportTimeRangeV1 {
                starts_at: 1,
                ends_before: 1 + DEFAULT_REPORT_RANGE_SECONDS,
            })
        );
        assert_eq!(
            resolve_report_range(SprinklerReportChartKind::WaterUsage, Some(1), None, None,),
            Some(SprinklerReportTimeRangeV1 {
                starts_at: 1,
                ends_before: 1
                    + DEFAULT_REPORT_RANGE_SECONDS
                    + u64::from(SPRINKLER_FORECAST_HORIZON_SECONDS),
            })
        );
        assert!(SprinklerReportChartKind::WaterUsage.includes_forecast());
        assert!(SprinklerReportChartKind::WeatherEt.includes_forecast());
        assert!(!SprinklerReportChartKind::WaterBalance.includes_forecast());
        assert!(
            resolve_report_range(
                SprinklerReportChartKind::WaterUsage,
                Some(1),
                Some(1 + MAX_REPORT_RANGE_SECONDS + 1),
                Some(NOW),
            )
            .is_none()
        );
        assert!(
            resolve_report_range(SprinklerReportChartKind::WaterBalance, None, None, None,)
                .is_none()
        );
        assert_eq!(
            report_usage_bucket(SprinklerReportTimeRangeV1 {
                starts_at: 1,
                ends_before: 1 + 14 * SECONDS_PER_DAY,
            }),
            SprinklerReportBucketV1::Day
        );
        assert_eq!(
            report_usage_bucket(SprinklerReportTimeRangeV1 {
                starts_at: 1,
                ends_before: 2 + 14 * SECONDS_PER_DAY,
            }),
            SprinklerReportBucketV1::Week
        );
    }

    #[test]
    fn report_interval_sweeps_and_fragment_generation_are_bounded() {
        let points = replay_deficit_points(
            10.0,
            100.0,
            0,
            10,
            &[
                BalanceRateInterval {
                    starts_at: 2,
                    ends_before: 8,
                    deficit_millimeters_per_second: 1.0,
                },
                BalanceRateInterval {
                    starts_at: 5,
                    ends_before: 10,
                    deficit_millimeters_per_second: 0.5,
                },
            ],
        );
        assert_eq!(
            points.iter().map(|point| point.0).collect::<Vec<_>>(),
            vec![0, 2, 5, 8, 10]
        );
        for ((_, actual), expected) in points.iter().zip([10.0, 10.0, 13.0, 17.5, 18.5]) {
            assert!((*actual - expected).abs() < 0.001);
        }

        let provider = [(2, 4), (6, 8)];
        assert_eq!(
            provider_uncovered_fragments(0, 10, &provider, 3),
            Ok(vec![(0, 2), (4, 6), (8, 10)])
        );
        assert_eq!(provider_uncovered_fragments(0, 10, &provider, 2), Err(()));

        let gaps = [
            SprinklerModeledWeatherGapV1 {
                starts_at: 1,
                ends_before: 10,
                reference_evapotranspiration_millimeters_per_day: 4.0,
                demand_source: SprinklerWaterDemandSourceV1::ConservativeDefault,
                recorded_at: 1,
            },
            SprinklerModeledWeatherGapV1 {
                starts_at: 5,
                ends_before: 15,
                reference_evapotranspiration_millimeters_per_day: 6.0,
                demand_source: SprinklerWaterDemandSourceV1::RecentLocalWeather,
                recorded_at: 5,
            },
        ];
        let normalized = normalized_modeled_weather_gaps(&gaps, 1, 15);
        assert_eq!(normalized.len(), 2);
        assert_eq!(
            (normalized[0].starts_at, normalized[0].ends_before),
            (1, 10)
        );
        assert_eq!(
            (normalized[1].starts_at, normalized[1].ends_before),
            (10, 15)
        );
    }

    #[test]
    fn report_rejects_a_line_path_over_the_client_limit() {
        let row = SprinklerWaterBalancePointV1 {
            at: NOW,
            available_water_percent: 50.0,
            series: SprinklerWaterBalanceSeriesV1::AvailableWater,
            zone: zone().valve,
        };
        let response =
            SprinklerReportProtocolV1::WaterBalanceV1(vec![row; MAX_REPORT_POINTS_PER_PATH + 1]);
        assert!(!report_response_within_chart_limits(&response));
    }

    #[test]
    fn timeline_keys_are_unique_across_zones() {
        let activity = completed_report_activity(NOW - 600);
        let report_zone = |valve: LibertasDevice| ReportZoneData {
            valve,
            capacity_millimeters: root_zone_capacity_millimeters(&zone()),
            crop_coefficient: plant_profile(zone().plant_type).crop_coefficient,
            active_state: runtime(memory()).active_state,
            water_events: Vec::new(),
            modeled_weather_gaps: Vec::new(),
            current_activity: None,
            activities: vec![activity.clone()],
            daily_reports: Vec::new(),
        };
        let rows = build_watering_timeline(
            &[report_zone(7), report_zone(8)],
            SprinklerReportTimeRangeV1 {
                starts_at: NOW - SECONDS_PER_DAY,
                ends_before: NOW + SECONDS_PER_DAY,
            },
        );
        assert_eq!(rows.activities.len(), 2);
        assert!(rows.empty_zones.is_empty());
        assert_ne!(
            rows.activities[0].activity_key,
            rows.activities[1].activity_key
        );
    }

    #[test]
    fn every_report_chart_includes_populated_and_idle_configured_zones() {
        let day = utc_day_start(NOW);
        let range = SprinklerReportTimeRangeV1 {
            starts_at: day,
            ends_before: day + SECONDS_PER_DAY,
        };
        let report_zone = |valve: LibertasDevice, populated: bool| ReportZoneData {
            valve,
            capacity_millimeters: root_zone_capacity_millimeters(&zone()),
            crop_coefficient: plant_profile(zone().plant_type).crop_coefficient,
            active_state: runtime(memory()).active_state,
            water_events: Vec::new(),
            modeled_weather_gaps: populated
                .then_some(SprinklerModeledWeatherGapV1 {
                    starts_at: day,
                    ends_before: day + 3_600,
                    reference_evapotranspiration_millimeters_per_day: 4.0,
                    demand_source: SprinklerWaterDemandSourceV1::ConservativeDefault,
                    recorded_at: day,
                })
                .into_iter()
                .collect(),
            current_activity: None,
            activities: populated
                .then(|| completed_report_activity(day + 3_600))
                .into_iter()
                .collect(),
            daily_reports: populated
                .then(|| completed_daily_report(day))
                .into_iter()
                .collect(),
        };
        let zones = [report_zone(7, true), report_zone(8, false)];

        let balance = build_water_balance_chart(&zones, &[], range).unwrap();
        let timeline = build_watering_timeline(&zones, range);
        let usage = build_water_usage(&zones, &[], None, SprinklerReportBucketV1::Day, range);
        let weather = build_weather_et_chart(&[], &[], &[], None, &zones, range).unwrap();

        assert!(balance.iter().any(|row| row.zone == 7));
        assert!(balance.iter().any(|row| row.zone == 8));
        assert!(timeline.activities.iter().any(|row| row.zone == 7));
        assert!(!timeline.activities.iter().any(|row| row.zone == 8));
        assert_eq!(timeline.empty_zones.len(), 1);
        assert_eq!(timeline.empty_zones[0].zone, 8);
        assert!(usage.inputs.iter().any(|row| row.zone == 7));
        assert!(!usage.inputs.iter().any(|row| row.zone == 8));
        assert_eq!(usage.empty_zones.len(), 1);
        assert_eq!(usage.empty_zones[0].zone, 8);
        assert!(
            weather
                .modeled_reference_evapotranspiration
                .gaps
                .iter()
                .any(|row| row.zone == 7)
        );
        assert!(
            !weather
                .modeled_reference_evapotranspiration
                .gaps
                .iter()
                .any(|row| row.zone == 8)
        );
        assert_eq!(
            weather
                .modeled_reference_evapotranspiration
                .empty_zones
                .len(),
            1
        );
        assert_eq!(
            weather.modeled_reference_evapotranspiration.empty_zones[0].zone,
            8
        );
    }

    #[test]
    fn report_intervals_are_clipped_and_cross_midnight_water_is_prorated() {
        let day = utc_day_start(NOW);
        let events = vec![
            SprinklerWaterEventV1::WeatherV1 {
                starts_at: day - 30,
                duration_seconds: 60,
                precipitation_millimeters: 2.0,
                reference_evapotranspiration_millimeters: 4.0,
            },
            SprinklerWaterEventV1::IrrigationV1 {
                starts_at: day - 30,
                duration_seconds: 60,
                watering_percent: 100,
                applied_water_millimeters: 6.0,
            },
        ];
        assert_eq!(
            daily_report_totals(&events, day - SECONDS_PER_DAY, day),
            (1.0, 2.0, 3.0)
        );
        assert_eq!(
            daily_report_totals(&events, day, day + SECONDS_PER_DAY),
            (1.0, 2.0, 3.0)
        );

        let activity = completed_report_activity(day - 300);
        let report_zone = ReportZoneData {
            valve: zone().valve,
            capacity_millimeters: root_zone_capacity_millimeters(&zone()),
            crop_coefficient: plant_profile(zone().plant_type).crop_coefficient,
            active_state: runtime(memory()).active_state,
            water_events: Vec::new(),
            modeled_weather_gaps: Vec::new(),
            current_activity: None,
            activities: vec![activity],
            daily_reports: Vec::new(),
        };
        let range = SprinklerReportTimeRangeV1 {
            starts_at: day,
            ends_before: day + 120,
        };
        let timeline = build_watering_timeline(&[report_zone], range);
        assert_eq!(timeline.activities[0].starts_at, day);
        assert_eq!(timeline.activities[0].ends_at, day + 120);
    }

    #[test]
    fn weather_chart_keeps_current_observations_and_clips_forecast_left_edge() {
        let range = SprinklerReportTimeRangeV1 {
            starts_at: NOW,
            ends_before: NOW + 3_600,
        };
        let history = report_weather_period(NOW);
        let observation = SprinklerCurrentWeatherV1 {
            retrieved_at: NOW + 900,
            valid_until: NOW + 2_700,
            valid_at: NOW + 900,
            interval_seconds: 900,
            temperature_celsius: 31.0,
            relative_humidity_percent: 47,
            precipitation_millimeters: 9.0,
            reference_evapotranspiration_millimeters: 8.0,
            wind_speed_meters_per_second: 7.0,
            wind_gust_meters_per_second: 11.0,
        };
        let forecast = SprinklerWeatherForecastV1 {
            retrieved_at: NOW,
            valid_until: NOW + 3_600,
            periods: vec![SprinklerWeatherForecastPeriodV1 {
                starts_at: NOW - 900,
                duration_seconds: 3_600,
                temperature_celsius: 19.0,
                relative_humidity_percent: 55,
                precipitation_probability_percent: 20,
                expected_precipitation_millimeters: 2.0,
                reference_evapotranspiration_millimeters: 4.0,
                wind_speed_meters_per_second: 5.0,
                wind_gust_meters_per_second: 9.0,
            }],
        };

        let chart = build_weather_et_chart(
            &[history.into()],
            &[history],
            &[observation],
            Some(&forecast),
            &[],
            range,
        )
        .unwrap();
        assert_eq!(chart.reference_evapotranspiration.len(), 2);
        let forecast_et = chart
            .reference_evapotranspiration
            .iter()
            .find(|row| row.source == SprinklerWeatherChartSourceV1::Forecast)
            .unwrap();
        assert_eq!(forecast_et.starts_at, range.starts_at);
        assert_eq!(forecast_et.ends_at, NOW + 2_700);
        assert!((forecast_et.reference_evapotranspiration_millimeters - 3.0).abs() < 0.001);

        assert!(chart.temperature.iter().any(|row| {
            row.at == observation.valid_at
                && row.temperature_celsius == observation.temperature_celsius
                && row.source == SprinklerWeatherChartSourceV1::CurrentObservation
        }));
        assert!(chart.relative_humidity.iter().any(|row| {
            row.at == observation.valid_at
                && row.relative_humidity_percent == observation.relative_humidity_percent
                && row.source == SprinklerWeatherChartSourceV1::CurrentObservation
        }));
        let observation_wind: Vec<_> = chart
            .wind
            .iter()
            .filter(|row| row.at == observation.valid_at)
            .map(|row| row.meters_per_second)
            .collect();
        assert_eq!(observation_wind, vec![7.0, 11.0]);
    }

    #[test]
    fn provider_covered_subwindow_does_not_receive_modeled_et() {
        let day = utc_day_start(NOW);
        let range = SprinklerReportTimeRangeV1 {
            starts_at: day + 6 * 3_600,
            ends_before: day + 12 * 3_600,
        };
        let gap = SprinklerModeledWeatherGapV1 {
            starts_at: day,
            ends_before: day + 12 * 3_600,
            reference_evapotranspiration_millimeters_per_day: 4.0,
            demand_source: SprinklerWaterDemandSourceV1::ConservativeDefault,
            recorded_at: day,
        };
        let mut provider = report_weather_period(range.starts_at);
        provider.duration_seconds = 6 * 3_600;

        let report_zone = ReportZoneData {
            valve: zone().valve,
            capacity_millimeters: root_zone_capacity_millimeters(&zone()),
            crop_coefficient: plant_profile(zone().plant_type).crop_coefficient,
            active_state: runtime(memory()).active_state,
            water_events: Vec::new(),
            modeled_weather_gaps: vec![gap],
            current_activity: None,
            activities: Vec::new(),
            daily_reports: Vec::new(),
        };
        let chart = build_weather_et_chart(
            &[provider.into()],
            &[provider],
            &[],
            None,
            &[report_zone],
            range,
        )
        .unwrap();

        assert_eq!(chart.reference_evapotranspiration.len(), 1);
        assert_eq!(
            chart.reference_evapotranspiration[0].source,
            SprinklerWeatherChartSourceV1::HistoricalObservation
        );
        assert_eq!(
            chart.reference_evapotranspiration[0].starts_at,
            range.starts_at
        );
        assert_eq!(
            chart.reference_evapotranspiration[0].ends_at,
            range.ends_before
        );
        assert!(chart.modeled_reference_evapotranspiration.gaps.is_empty());
    }

    #[test]
    fn water_usage_uses_one_amount_scale_for_partial_and_full_buckets() {
        let partial_bucket = UsageAccumulator {
            starts_at: 100,
            ends_at: 1_000,
            zone: 7,
            rain: 1.0,
            irrigation: 2.0,
            forecast_rain: 0.0,
            scheduled_water: 0.0,
        };
        let full_bucket = UsageAccumulator {
            starts_at: 10_000,
            ends_at: 10_000 + SECONDS_PER_DAY,
            zone: 8,
            rain: 2.0,
            irrigation: 2.0,
            forecast_rain: 0.0,
            scheduled_water: 0.0,
        };
        let totals = [partial_bucket, full_bucket];
        let common_mark_span_seconds = water_usage_common_mark_span_seconds(&totals);
        let maximum_total_millimeters = totals
            .iter()
            .filter_map(water_usage_total_millimeters)
            .fold(0.0_f64, f64::max);

        assert_eq!(common_mark_span_seconds, 675);
        let equal_amount_duration = water_usage_display_duration_seconds(
            2.0,
            maximum_total_millimeters,
            common_mark_span_seconds,
        );
        assert_eq!(equal_amount_duration, 337);
        assert_eq!(
            totals[0].starts_at + equal_amount_duration - totals[0].starts_at,
            totals[1].starts_at + equal_amount_duration - totals[1].starts_at
        );
        assert_eq!(
            water_usage_display_duration_seconds(
                4.0,
                maximum_total_millimeters,
                common_mark_span_seconds,
            ),
            common_mark_span_seconds
        );
        assert_eq!(
            totals[0].ends_at - (totals[0].starts_at + common_mark_span_seconds),
            225
        );
    }

    #[test]
    fn water_usage_uses_exact_partial_raw_inputs_and_omits_zero_segments() {
        let day = utc_day_start(NOW);
        let range = SprinklerReportTimeRangeV1 {
            starts_at: day + 900,
            ends_before: day + 1_800,
        };
        let mut period: SprinklerWeatherHistoryPeriodV1 = report_weather_period(day).into();
        period.precipitation_millimeters = 4.0;
        let mut activity = completed_report_activity(day);
        activity.scheduled_duration_seconds = Some(3_600);
        activity.actual_duration_seconds = Some(3_600);
        activity.planned_water_millimeters = Some(8.0);
        activity.applied_water_millimeters = Some(8.0);
        activity.updated_at = day + 3_600;
        let mut report_zone = ReportZoneData {
            valve: zone().valve,
            capacity_millimeters: root_zone_capacity_millimeters(&zone()),
            crop_coefficient: plant_profile(zone().plant_type).crop_coefficient,
            active_state: runtime(memory()).active_state,
            water_events: Vec::new(),
            modeled_weather_gaps: Vec::new(),
            current_activity: None,
            activities: vec![activity],
            daily_reports: Vec::new(),
        };

        let rows = build_water_usage(
            core::slice::from_ref(&report_zone),
            core::slice::from_ref(&period),
            None,
            SprinklerReportBucketV1::Day,
            range,
        );
        assert_eq!(rows.inputs.len(), 2);
        assert!(rows.empty_zones.is_empty());
        assert!(rows.inputs.iter().all(|row| row.at == range.starts_at));
        let rain = rows
            .inputs
            .iter()
            .find(|row| row.input_type == SprinklerWaterInputTypeV1::Rain)
            .unwrap();
        assert!((rain.amount_millimeters - 1.0).abs() < 0.001);
        assert_eq!(rain.segment_starts_at, range.starts_at);
        assert_eq!(rain.segment_ends_at, range.starts_at + 225);
        let irrigation = rows
            .inputs
            .iter()
            .find(|row| row.input_type == SprinklerWaterInputTypeV1::Irrigation)
            .unwrap();
        assert!((irrigation.amount_millimeters - 2.0).abs() < 0.001);
        assert_eq!(irrigation.segment_starts_at, rain.segment_ends_at);
        assert_eq!(irrigation.segment_ends_at, range.starts_at + 675);
        assert!(irrigation.segment_ends_at < range.ends_before);

        period.precipitation_millimeters = 0.0;
        let forecast = SprinklerWeatherForecastV1 {
            retrieved_at: day,
            valid_until: day + 3_600,
            periods: vec![SprinklerWeatherForecastPeriodV1 {
                starts_at: day,
                duration_seconds: 3_600,
                temperature_celsius: 20.0,
                relative_humidity_percent: 50,
                precipitation_probability_percent: 80,
                expected_precipitation_millimeters: 12.0,
                reference_evapotranspiration_millimeters: 0.0,
                wind_speed_meters_per_second: 1.0,
                wind_gust_meters_per_second: 2.0,
            }],
        };
        let scheduled = &mut report_zone.activities[0];
        scheduled.outcome = SprinklerWateringOutcomeV1::Scheduled;
        scheduled.actual_starts_at = None;
        scheduled.actual_duration_seconds = None;
        scheduled.applied_water_millimeters = None;
        let planned = build_water_usage(
            core::slice::from_ref(&report_zone),
            core::slice::from_ref(&period),
            Some(&forecast),
            SprinklerReportBucketV1::Day,
            range,
        );
        assert_eq!(planned.inputs.len(), 2);
        let forecast_rain = planned
            .inputs
            .iter()
            .find(|row| row.input_type == SprinklerWaterInputTypeV1::ForecastRain)
            .unwrap();
        assert!((forecast_rain.amount_millimeters - 3.0).abs() < 0.001);
        assert_eq!(forecast_rain.segment_starts_at, range.starts_at);
        assert_eq!(forecast_rain.segment_ends_at, range.starts_at + 405);
        let scheduled_water = planned
            .inputs
            .iter()
            .find(|row| row.input_type == SprinklerWaterInputTypeV1::ScheduledWater)
            .unwrap();
        assert!((scheduled_water.amount_millimeters - 2.0).abs() < 0.001);
        assert_eq!(
            scheduled_water.segment_starts_at,
            forecast_rain.segment_ends_at
        );
        assert_eq!(scheduled_water.segment_ends_at, range.starts_at + 675);

        report_zone.activities[0].outcome = SprinklerWateringOutcomeV1::Superseded;
        let empty = build_water_usage(
            &[report_zone],
            &[period],
            None,
            SprinklerReportBucketV1::Day,
            range,
        );
        assert!(empty.inputs.is_empty());
        assert_eq!(empty.empty_zones.len(), 1);
    }

    #[test]
    fn daily_report_exposes_partial_coverage_and_modeled_weather_gap() {
        let day = utc_day_start(NOW);
        let coverage_starts_at = day + 6 * 3_600;
        let now = day + 12 * 3_600;
        let mut memory = default_memory(coverage_starts_at);
        memory.baseline_deficit_millimeters = 2.0;
        let mut zone_runtime = runtime(memory);
        zone_runtime.water_events = vec![SprinklerWaterEventV1::WeatherV1 {
            starts_at: coverage_starts_at,
            duration_seconds: 2 * 3_600,
            precipitation_millimeters: 0.5,
            reference_evapotranspiration_millimeters: 1.0,
        }];
        zone_runtime.modeled_weather_gaps = reconcile_modeled_weather_gaps(
            &[],
            &zone_runtime.water_events,
            coverage_starts_at,
            now,
            WaterDemandEstimate {
                source: SprinklerWaterDemandSourceV1::ConservativeDefault,
                reference_evapotranspiration_millimeters_per_day:
                    CONSERVATIVE_REFERENCE_ET_MILLIMETERS_PER_DAY,
            },
            now,
        );

        let reports = build_daily_reports(&zone_runtime, None, now);
        assert_eq!(reports.len(), 1);
        let report = reports[0];
        assert_eq!(report.starts_at, day);
        assert_eq!(report.ends_before, day + SECONDS_PER_DAY);
        assert_eq!(report.coverage_starts_at, coverage_starts_at);
        assert_eq!(report.coverage_ends_before, now);
        assert_eq!(report.provider_weather_coverage_seconds, 2 * 3_600);
        assert!((report.precipitation_millimeters - 0.5).abs() < 0.001);
        assert!((report.reference_evapotranspiration_millimeters - 1.0).abs() < 0.001);
        let expected_modeled = CONSERVATIVE_REFERENCE_ET_MILLIMETERS_PER_DAY * 4.0 / 24.0;
        assert!(
            (report.modeled_reference_evapotranspiration_millimeters - expected_modeled).abs()
                < 0.001
        );
        assert_eq!(
            report.modeled_demand_source,
            Some(SprinklerWaterDemandSourceV1::ConservativeDefault)
        );
        assert!(!report.complete);
        assert!(valid_daily_report(&report));
    }

    #[test]
    fn modeled_gap_provenance_is_frozen_and_provider_correction_only_clips_it() {
        let day = utc_day_start(NOW);
        let original = SprinklerModeledWeatherGapV1 {
            starts_at: day,
            ends_before: day + 6 * 3_600,
            reference_evapotranspiration_millimeters_per_day: 3.25,
            demand_source: SprinklerWaterDemandSourceV1::RecentLocalWeather,
            recorded_at: day + 60,
        };
        let changed_estimate = WaterDemandEstimate {
            source: SprinklerWaterDemandSourceV1::LocationAndSeason,
            reference_evapotranspiration_millimeters_per_day: 7.0,
        };
        let extended = reconcile_modeled_weather_gaps(
            &[original],
            &[],
            day,
            day + 8 * 3_600,
            changed_estimate,
            day + 8 * 3_600,
        );
        assert_eq!(extended.len(), 1);
        assert_eq!(extended[0].starts_at, day);
        assert_eq!(extended[0].ends_before, day + 8 * 3_600);
        assert_eq!(
            extended[0].reference_evapotranspiration_millimeters_per_day,
            original.reference_evapotranspiration_millimeters_per_day
        );
        assert_eq!(extended[0].demand_source, original.demand_source);
        assert_eq!(extended[0].recorded_at, original.recorded_at);

        let provider = SprinklerWaterEventV1::WeatherV1 {
            starts_at: day + 2 * 3_600,
            duration_seconds: 2 * 3_600,
            precipitation_millimeters: 0.0,
            reference_evapotranspiration_millimeters: 0.5,
        };
        let corrected = reconcile_modeled_weather_gaps(
            &extended,
            &[provider],
            day,
            day + 8 * 3_600,
            WaterDemandEstimate {
                source: SprinklerWaterDemandSourceV1::ConservativeDefault,
                reference_evapotranspiration_millimeters_per_day: 9.0,
            },
            day + 9 * 3_600,
        );
        assert_eq!(corrected.len(), 2);
        assert_eq!(
            corrected
                .iter()
                .map(|gap| (gap.starts_at, gap.ends_before))
                .collect::<Vec<_>>(),
            vec![(day, day + 2 * 3_600), (day + 4 * 3_600, day + 8 * 3_600)]
        );
        assert!(corrected.iter().all(|gap| {
            gap.reference_evapotranspiration_millimeters_per_day
                == original.reference_evapotranspiration_millimeters_per_day
                && gap.demand_source == original.demand_source
                && gap.recorded_at == original.recorded_at
        }));
    }

    #[test]
    fn projected_balance_applies_only_the_observed_part_of_an_interval() {
        let mut memory = memory();
        memory.baseline_deficit_millimeters = 10.0;
        let event = SprinklerWaterEventV1::IrrigationV1 {
            starts_at: NOW,
            duration_seconds: 60,
            watering_percent: 100,
            applied_water_millimeters: 6.0,
        };
        let estimate = WaterDemandEstimate {
            source: SprinklerWaterDemandSourceV1::ConservativeDefault,
            reference_evapotranspiration_millimeters_per_day: 0.0,
        };
        assert_eq!(
            projected_deficit_millimeters(
                &zone(),
                &memory,
                core::slice::from_ref(&event),
                NOW,
                estimate,
            ),
            10.0
        );
        assert_eq!(
            projected_deficit_millimeters(&zone(), &memory, &[event], NOW + 30, estimate),
            7.0
        );
    }

    #[test]
    fn weather_replacement_deletes_only_absent_keys_in_a_complete_span_scan() {
        let periods = vec![
            report_weather_period(NOW - 7_200),
            report_weather_period(NOW),
        ];
        let replacement_indexes = report_weather_replacement_indexes(&periods).unwrap();
        let first = replacement_indexes[0];
        let last = replacement_indexes[1];
        let existing = [first - 3_600, first, first + 3_600, last, last + 3_600];
        assert_eq!(
            stale_report_weather_period_indexes(&existing, &replacement_indexes, true),
            vec![first + 3_600]
        );
        assert!(
            stale_report_weather_period_indexes(&existing, &replacement_indexes, false).is_empty()
        );
    }

    #[test]
    fn invalid_or_unsorted_weather_replacement_cannot_delete_archive_rows() {
        let mut invalid = report_weather_period(NOW);
        invalid.wind_speed_meters_per_second = f32::NAN;
        assert!(report_weather_replacement_indexes(&[invalid]).is_none());
        assert!(
            report_weather_replacement_indexes(&[
                report_weather_period(NOW),
                report_weather_period(NOW - 3_600),
            ])
            .is_none()
        );
    }

    #[test]
    fn activity_ordinals_prevent_same_second_archive_overwrites() {
        let first = watering_activity_index(NOW, SprinklerWateringOriginV1::Automatic, 0).unwrap();
        let second = watering_activity_index(NOW, SprinklerWateringOriginV1::Automatic, 1).unwrap();
        assert_ne!(first, second);
        let mut activity = completed_report_activity(NOW);
        activity.activity_index = second;
        activity.activity_ordinal = 1;
        assert!(valid_watering_activity(&activity));
    }

    #[test]
    fn eight_hour_manual_activity_is_queryable_during_its_last_hour() {
        let day = utc_day_start(NOW);
        let starts_at = day + 20 * 3_600;
        let mut activity = completed_report_activity(starts_at);
        activity.origin = SprinklerWateringOriginV1::Manual;
        activity.reason = SprinklerWateringReasonV1::ManualOperation;
        activity.scheduled_starts_at = None;
        activity.scheduled_duration_seconds = None;
        activity.planned_water_millimeters = None;
        activity.actual_duration_seconds = Some(8 * 3_600);
        activity.applied_water_millimeters = Some(96.0);
        activity.updated_at = starts_at + 8 * 3_600;
        activity.activity_index = watering_activity_index(
            starts_at,
            SprinklerWateringOriginV1::Manual,
            activity.activity_ordinal,
        )
        .unwrap();
        let days = activity_report_days(&activity);
        assert_eq!(days, vec![day, day + SECONDS_PER_DAY]);

        let range = SprinklerReportTimeRangeV1 {
            starts_at: starts_at + 7 * 3_600,
            ends_before: starts_at + 8 * 3_600,
        };
        let record = IndexedData {
            index: activity.activity_index,
            data: SprinklerDataV1::WateringActivityV1 {
                activity: activity.clone(),
            },
        };
        let mut loaded = Vec::new();
        assert_eq!(merge_report_activity(&mut loaded, record, range, 1), Ok(()));
        assert_eq!(loaded, vec![activity]);
    }

    #[test]
    fn pressure_fold_finalizes_day_before_advancing_baseline_and_deleting_events() {
        let day = utc_day_start(NOW);
        let mut memory = default_memory(day);
        let mut events = vec![SprinklerWaterEventV1::WeatherV1 {
            starts_at: day + 3_600,
            duration_seconds: 3_600,
            precipitation_millimeters: 2.0,
            reference_evapotranspiration_millimeters: 1.0,
        }];
        for index in 0..MAX_WATER_EVENTS {
            events.push(SprinklerWaterEventV1::IrrigationV1 {
                starts_at: day + SECONDS_PER_DAY + index as u64 * 120,
                duration_seconds: 60,
                watering_percent: 100,
                applied_water_millimeters: 0.01,
            });
        }
        let now = day + 2 * SECONDS_PER_DAY + 12 * 3_600;
        let mut modeled_weather_gaps = vec![
            SprinklerModeledWeatherGapV1 {
                starts_at: day,
                ends_before: day + 3_600,
                reference_evapotranspiration_millimeters_per_day: 4.0,
                demand_source: SprinklerWaterDemandSourceV1::ConservativeDefault,
                recorded_at: day + 3_600,
            },
            SprinklerModeledWeatherGapV1 {
                starts_at: day + 2 * 3_600,
                ends_before: day + SECONDS_PER_DAY,
                reference_evapotranspiration_millimeters_per_day: 4.0,
                demand_source: SprinklerWaterDemandSourceV1::ConservativeDefault,
                recorded_at: day + 2 * 3_600,
            },
        ];

        let reports = prune_water_events(
            &mut memory,
            &mut events,
            &mut modeled_weather_gaps,
            &zone(),
            None,
            now,
        );
        assert_eq!(memory.balance_baseline_at, day + SECONDS_PER_DAY);
        assert_eq!(events.len(), MAX_WATER_EVENTS);
        assert!(modeled_weather_gaps.is_empty());
        assert!(events.iter().all(|event| {
            event
                .ends_at()
                .is_none_or(|ends_at| ends_at > memory.balance_baseline_at)
        }));
        assert_eq!(reports.len(), 1);
        let report = reports[0];
        assert_eq!(report.starts_at, day);
        assert_eq!(report.ends_before, day + SECONDS_PER_DAY);
        assert!(report.complete);
        assert!((report.precipitation_millimeters - 2.0).abs() < 0.001);
        assert!((report.reference_evapotranspiration_millimeters - 1.0).abs() < 0.001);
        assert!(
            (report.modeled_reference_evapotranspiration_millimeters - (4.0 * 23.0 / 24.0)).abs()
                < 0.001
        );
        assert_eq!(
            report.modeled_demand_source,
            Some(SprinklerWaterDemandSourceV1::ConservativeDefault)
        );
        assert_eq!(report.provider_weather_coverage_seconds, 3_600);
        assert!(valid_daily_report(&report));
        assert!(
            (memory.baseline_deficit_millimeters - report.closing_deficit_millimeters).abs()
                < 0.001
        );
    }

    #[test]
    fn restored_running_open_preserves_prior_observed_activity_totals() {
        let mut activity = completed_report_activity(NOW - 900);
        activity.outcome = SprinklerWateringOutcomeV1::Running;
        activity.reason = SprinklerWateringReasonV1::CommandTimeout;
        activity.actual_starts_at = Some(NOW - 600);
        activity.actual_duration_seconds = Some(300);
        activity.applied_water_millimeters = Some(1.0);
        activity.updated_at = NOW - 300;
        let mut zone_runtime = runtime(memory());
        zone_runtime.current_activity = Some(activity);

        let updated = mark_automatic_activity_open(&mut zone_runtime, NOW).unwrap();
        assert_eq!(updated.outcome, SprinklerWateringOutcomeV1::Running);
        assert_eq!(updated.reason, SprinklerWateringReasonV1::CommandTimeout);
        assert_eq!(updated.actual_starts_at, Some(NOW - 600));
        assert_eq!(updated.actual_duration_seconds, Some(300));
        assert_eq!(updated.applied_water_millimeters, Some(1.0));
        assert_eq!(updated.updated_at, NOW);
        assert_eq!(zone_runtime.current_activity, Some(updated));
    }

    #[test]
    fn only_automatic_pending_or_running_activity_owns_restart_expectation() {
        let mut automatic = completed_report_activity(NOW - 900);
        automatic.outcome = SprinklerWateringOutcomeV1::Running;
        let expected = restored_expected_irrigation(&automatic).unwrap();
        assert_eq!(expected.starts_at, NOW - 900);
        assert_eq!(expected.activity_index, automatic.activity_index);
        assert_eq!(expected.activity_ordinal, automatic.activity_ordinal);

        automatic.outcome = SprinklerWateringOutcomeV1::CommandPending;
        assert!(restored_expected_irrigation(&automatic).is_some());
        automatic.outcome = SprinklerWateringOutcomeV1::Scheduled;
        assert!(restored_expected_irrigation(&automatic).is_none());

        let mut manual = completed_report_activity(NOW - 900);
        manual.origin = SprinklerWateringOriginV1::Manual;
        manual.outcome = SprinklerWateringOutcomeV1::Running;
        manual.scheduled_starts_at = None;
        manual.activity_index = watering_activity_index(
            manual.actual_starts_at.unwrap(),
            SprinklerWateringOriginV1::Manual,
            manual.activity_ordinal,
        )
        .unwrap();
        assert!(restored_expected_irrigation(&manual).is_none());
    }

    #[test]
    fn authoritative_current_activity_repairs_a_missing_archive_row() {
        let mut running = completed_report_activity(NOW - 900);
        running.outcome = SprinklerWateringOutcomeV1::Running;
        running.updated_at = NOW;
        assert_eq!(
            watering_activity_load_plan(Some(SavedWateringActivityState::Authoritative(
                SprinklerWateringActivityStateV2 {
                    latest_activity: Some(running.clone()),
                    activity_is_current: true,
                },
            ))),
            WateringActivityLoadPlan::RepairArchive {
                activity: running,
                return_current: true,
                migrate_state: false,
            }
        );
    }

    #[test]
    fn authoritative_terminal_activity_repairs_a_missing_archive_row_without_resuming() {
        let completed = completed_report_activity(NOW - 900);
        assert_eq!(
            watering_activity_load_plan(Some(SavedWateringActivityState::Authoritative(
                SprinklerWateringActivityStateV2 {
                    latest_activity: Some(completed.clone()),
                    activity_is_current: false,
                },
            ))),
            WateringActivityLoadPlan::RepairArchive {
                activity: completed,
                return_current: false,
                migrate_state: false,
            }
        );
    }

    #[test]
    fn legacy_activity_state_migrates_without_changing_its_avro_layout() {
        let mut running = completed_report_activity(NOW - 900);
        running.outcome = SprinklerWateringOutcomeV1::Running;
        assert_eq!(
            watering_activity_load_plan(Some(SavedWateringActivityState::Legacy(
                SprinklerWateringActivityStateV1 {
                    current_activity: Some(running.clone()),
                },
            ))),
            WateringActivityLoadPlan::RepairArchive {
                activity: running,
                return_current: true,
                migrate_state: true,
            }
        );
        assert_eq!(
            watering_activity_load_plan(Some(SavedWateringActivityState::Legacy(
                SprinklerWateringActivityStateV1 {
                    current_activity: None,
                },
            ))),
            WateringActivityLoadPlan::ReturnEmpty {
                migrate_state: true,
            }
        );
    }

    #[test]
    fn authoritative_empty_or_inconsistent_activity_state_is_safe() {
        assert_eq!(
            watering_activity_load_plan(Some(SavedWateringActivityState::Authoritative(
                empty_watering_activity_state(),
            ))),
            WateringActivityLoadPlan::ReturnEmpty {
                migrate_state: false,
            }
        );
        assert_eq!(
            watering_activity_load_plan(Some(SavedWateringActivityState::Authoritative(
                SprinklerWateringActivityStateV2 {
                    latest_activity: Some(completed_report_activity(NOW - 900)),
                    activity_is_current: true,
                },
            ))),
            WateringActivityLoadPlan::ClearInvalidState
        );
        assert_eq!(
            watering_activity_load_plan(None),
            WateringActivityLoadPlan::MigrateArchive
        );
    }

    #[test]
    fn persistent_memory_round_trips_and_discriminants_are_stable() {
        let configuration = zone();
        // Keep the end-user zone configuration exactly valve, plant profile,
        // sprinkler-head profile, and state endpoint. In particular, chart
        // labels must resolve the valve identity instead of adding a duplicate
        // configured name here.
        assert_eq!(configuration.to_avro(), vec![14, 0, 6, 34]);
        assert_eq!(
            SprinklerZoneV1::from_avro(&configuration.to_avro()),
            Ok(configuration)
        );

        let value = SprinklerDataV1::ZoneMemoryV1 { memory: memory() };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&0));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::WateringModeV1 {
            mode: SprinklerWateringModeV1::Winterization,
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&6));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::WaterEventV1 {
            event: SprinklerWaterEventV1::WeatherV1 {
                starts_at: NOW,
                duration_seconds: 3_600,
                precipitation_millimeters: 1.0,
                reference_evapotranspiration_millimeters: 2.0,
            },
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&2));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::WaterEventV1 {
            event: SprinklerWaterEventV1::IrrigationV1 {
                starts_at: NOW,
                duration_seconds: 600,
                watering_percent: 80,
                applied_water_millimeters: 2.0,
            },
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&2));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::SiteLocationV1 {
            location: location(),
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&4));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::WinterizationReminderV1 {
            memory: SprinklerWinterizationReminderMemoryV1 {
                last_reminded_at: NOW,
                reason: SprinklerWinterizationReminderReasonV1::FreezingWeather,
            },
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&8));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::ReportWeatherPeriodV1 {
            period: report_weather_period(NOW - 3_600).into(),
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&10));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::WateringActivityV1 {
            activity: completed_report_activity(NOW - 600),
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&12));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::DailyReportV1 {
            report: completed_daily_report(utc_day_start(NOW) - SECONDS_PER_DAY),
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&14));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::WateringActivityStateV1 {
            state: SprinklerWateringActivityStateV1 {
                current_activity: Some(completed_report_activity(NOW)),
            },
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&16));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::ReportWeatherObservationV1 {
            observation: current(),
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&18));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::ReportWeatherArchiveStateV1 {
            state: SprinklerReportWeatherArchiveStateV1 {
                generation: 7,
                location: Some(location()),
                awaiting_history_clear: false,
            },
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&20));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::WateringActivityStateV2 {
            state: SprinklerWateringActivityStateV2 {
                latest_activity: Some(completed_report_activity(NOW)),
                activity_is_current: false,
            },
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&22));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::ModeledWeatherGapV1 {
            gap: SprinklerModeledWeatherGapV1 {
                starts_at: NOW,
                ends_before: NOW + 900,
                reference_evapotranspiration_millimeters_per_day: 4.0,
                demand_source: SprinklerWaterDemandSourceV1::ConservativeDefault,
                recorded_at: NOW,
            },
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&24));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::ReportWeatherPeriodV2 {
            period: report_weather_period(NOW - 3_600),
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&26));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let value = SprinklerDataV1::ReportWeatherArchiveStateV2 {
            state: SprinklerReportWeatherArchiveStateV2 {
                generation: 8,
                location: Some(location()),
            },
        };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&28));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let protocols = [
            SprinklerZoneProtocolV1::GetStateV1,
            SprinklerZoneProtocolV1::StateV1 {
                state: SprinklerZoneStateV1::ActiveV1 {
                    condition: SprinklerScheduleConditionV1::Scheduled,
                    next_watering: SprinklerTimeSlotV1 {
                        starts_at: NOW,
                        duration_seconds: 600,
                    },
                },
            },
            SprinklerZoneProtocolV1::GetAdvancedStateV1,
            SprinklerZoneProtocolV1::AdvancedStateV1 {
                mode: SprinklerWateringModeV1::Active,
                state: SprinklerZoneAdvancedStateV1::ActiveV1 {
                    current: initial_active_state(NOW, &memory(), &zone()),
                },
            },
            SprinklerZoneProtocolV1::GetConfigurationV1,
            SprinklerZoneProtocolV1::ConfigurationV1 {
                configuration: SprinklerZoneConfigurationV1 {
                    watering_percent: 100,
                    hold_off_periods: Vec::new(),
                },
            },
            SprinklerZoneProtocolV1::SetWaterAmountAdjusterV1 {
                watering_percent: 100,
            },
            SprinklerZoneProtocolV1::ReplaceHoldOffPeriodsV1 {
                hold_off_periods: Vec::new(),
            },
            SprinklerZoneProtocolV1::SetWateringModeV1 {
                mode: SprinklerWateringModeV1::Winterization,
            },
        ];
        for (index, protocol) in protocols.iter().enumerate() {
            assert_eq!(protocol.to_avro().first(), Some(&((index as u8) * 2)));
        }

        let modes = [
            SprinklerWateringModeV1::Active,
            SprinklerWateringModeV1::Winterization,
        ];
        for (index, mode) in modes.iter().enumerate() {
            assert_eq!(mode.to_avro().first(), Some(&((index as u8) * 2)));
        }

        let heads = [
            SprinklerHeadTypeV1::SurfaceDrip,
            SprinklerHeadTypeV1::Bubblers,
            SprinklerHeadTypeV1::PopupSpray,
            SprinklerHeadTypeV1::RotorsLowRate,
            SprinklerHeadTypeV1::RotorsHighRate,
        ];
        for (index, head) in heads.iter().enumerate() {
            assert_eq!(head.to_avro().first(), Some(&((index as u8) * 2)));
        }

        let plants = [
            SprinklerPlantTypeV1::Lawn,
            SprinklerPlantTypeV1::Flowers,
            SprinklerPlantTypeV1::Vegetables,
            SprinklerPlantTypeV1::FruitTrees,
            SprinklerPlantTypeV1::Citrus,
            SprinklerPlantTypeV1::TreesAndBushes,
            SprinklerPlantTypeV1::Xeriscape,
        ];
        for (index, plant) in plants.iter().enumerate() {
            assert_eq!(plant.to_avro().first(), Some(&((index as u8) * 2)));
        }

        let conditions = [
            SprinklerScheduleConditionV1::Initializing,
            SprinklerScheduleConditionV1::WaterNotNeeded,
            SprinklerScheduleConditionV1::ForecastRain,
            SprinklerScheduleConditionV1::WaitingForSafeWeather,
            SprinklerScheduleConditionV1::PreemptiveHoldOff,
            SprinklerScheduleConditionV1::HeldOff,
            SprinklerScheduleConditionV1::Scheduled,
            SprinklerScheduleConditionV1::ValveCommandPending,
            SprinklerScheduleConditionV1::ValveStateUnavailable,
            SprinklerScheduleConditionV1::ValveOpen,
            SprinklerScheduleConditionV1::ValveFault,
            SprinklerScheduleConditionV1::OfflineWeatherEstimate,
        ];
        for (index, condition) in conditions.iter().enumerate() {
            assert_eq!(condition.to_avro().first(), Some(&((index as u8) * 2)));
        }

        let water_events = [
            SprinklerWaterEventV1::WeatherV1 {
                starts_at: NOW,
                duration_seconds: 3_600,
                precipitation_millimeters: 1.0,
                reference_evapotranspiration_millimeters: 2.0,
            },
            SprinklerWaterEventV1::IrrigationV1 {
                starts_at: NOW,
                duration_seconds: 600,
                watering_percent: 100,
                applied_water_millimeters: 2.0,
            },
        ];
        for (index, event) in water_events.iter().enumerate() {
            assert_eq!(event.to_avro().first(), Some(&((index as u8) * 2)));
        }

        let demand_sources = [
            SprinklerWaterDemandSourceV1::RecentLocalWeather,
            SprinklerWaterDemandSourceV1::LocationAndSeason,
            SprinklerWaterDemandSourceV1::ConservativeDefault,
        ];
        for (index, source) in demand_sources.iter().enumerate() {
            assert_eq!(source.to_avro().first(), Some(&((index as u8) * 2)));
        }
    }

    #[test]
    fn indexed_water_events_are_reconstructed_at_startup() {
        let baseline = NOW - 3_600;
        let weather = SprinklerWaterEventV1::WeatherV1 {
            starts_at: NOW - 1_800,
            duration_seconds: 900,
            precipitation_millimeters: 1.0,
            reference_evapotranspiration_millimeters: 2.0,
        };
        let irrigation = SprinklerWaterEventV1::IrrigationV1 {
            starts_at: NOW - 1_800,
            duration_seconds: 600,
            watering_percent: 100,
            applied_water_millimeters: 2.0,
        };
        let folded = SprinklerWaterEventV1::WeatherV1 {
            starts_at: NOW - 7_200,
            duration_seconds: 3_600,
            precipitation_millimeters: 10.0,
            reference_evapotranspiration_millimeters: 0.0,
        };
        let mismatched = SprinklerWaterEventV1::IrrigationV1 {
            starts_at: NOW - 900,
            duration_seconds: 60,
            watering_percent: 100,
            applied_water_millimeters: 0.2,
        };
        let records = vec![
            IndexedData {
                index: water_event_index(&irrigation).unwrap(),
                data: SprinklerDataV1::WaterEventV1 {
                    event: irrigation.clone(),
                },
            },
            IndexedData {
                index: water_event_index(&folded).unwrap(),
                data: SprinklerDataV1::WaterEventV1 { event: folded },
            },
            IndexedData {
                index: water_event_index(&mismatched).unwrap() + 2,
                data: SprinklerDataV1::WaterEventV1 { event: mismatched },
            },
            IndexedData {
                index: water_event_index(&weather).unwrap(),
                data: SprinklerDataV1::WaterEventV1 {
                    event: weather.clone(),
                },
            },
        ];

        assert_ne!(water_event_index(&weather), water_event_index(&irrigation));
        assert_eq!(
            reconstruct_water_events(&records, baseline),
            vec![weather, irrigation]
        );
    }

    #[test]
    fn changing_one_water_event_produces_one_indexed_upsert() {
        let weather = SprinklerWaterEventV1::WeatherV1 {
            starts_at: NOW - 3_600,
            duration_seconds: 3_600,
            precipitation_millimeters: 1.0,
            reference_evapotranspiration_millimeters: 2.0,
        };
        let irrigation = SprinklerWaterEventV1::IrrigationV1 {
            starts_at: NOW - 120,
            duration_seconds: 60,
            watering_percent: 100,
            applied_water_millimeters: 0.2,
        };
        let merged = SprinklerWaterEventV1::IrrigationV1 {
            starts_at: NOW - 120,
            duration_seconds: 120,
            watering_percent: 100,
            applied_water_millimeters: 0.4,
        };

        let delta = water_event_delta(&[weather.clone(), irrigation], &[weather, merged.clone()]);
        assert_eq!(
            delta.upserts,
            vec![(water_event_index(&merged).unwrap(), merged)]
        );
        assert!(delta.removals.is_empty());
    }

    #[test]
    fn water_demand_uses_recent_then_location_then_conservative_fallback() {
        let recent_events = vec![SprinklerWaterEventV1::WeatherV1 {
            starts_at: NOW - SECONDS_PER_DAY,
            duration_seconds: SECONDS_PER_DAY as u32,
            precipitation_millimeters: 0.0,
            reference_evapotranspiration_millimeters: 4.0,
        }];
        let recent = water_demand_estimate(&recent_events, Some(location()), NOW);
        assert_eq!(
            recent.source,
            SprinklerWaterDemandSourceV1::RecentLocalWeather
        );
        assert!((recent.reference_evapotranspiration_millimeters_per_day - 4.0).abs() < 0.001);

        let located = water_demand_estimate(&[], Some(location()), NOW);
        assert_eq!(
            located.source,
            SprinklerWaterDemandSourceV1::LocationAndSeason
        );
        assert!(
            (MIN_REFERENCE_ET_MILLIMETERS_PER_DAY..=MAX_REFERENCE_ET_MILLIMETERS_PER_DAY)
                .contains(&located.reference_evapotranspiration_millimeters_per_day)
        );

        let conservative = water_demand_estimate(&[], None, NOW);
        assert_eq!(
            conservative.source,
            SprinklerWaterDemandSourceV1::ConservativeDefault
        );
        assert_eq!(
            conservative.reference_evapotranspiration_millimeters_per_day,
            CONSERVATIVE_REFERENCE_ET_MILLIMETERS_PER_DAY
        );
    }

    #[test]
    fn location_fallback_tracks_each_hemispheres_summer() {
        const JANUARY_1_2024: LibertasDateTime = 1_704_067_200;
        const JULY_1_2024: LibertasDateTime = 1_719_792_000;
        let northern = location();
        let southern = SprinklerWeatherLocationV1 {
            latitude_degrees: -northern.latitude_degrees,
            ..northern
        };
        assert!(
            location_reference_evapotranspiration_millimeters_per_day(northern, JULY_1_2024)
                > location_reference_evapotranspiration_millimeters_per_day(
                    northern,
                    JANUARY_1_2024
                )
        );
        assert!(
            location_reference_evapotranspiration_millimeters_per_day(southern, JANUARY_1_2024)
                > location_reference_evapotranspiration_millimeters_per_day(southern, JULY_1_2024)
        );
    }

    #[test]
    fn seasonal_winterization_reminder_has_a_latitude_cutoff() {
        const APRIL_15_2026: LibertasDateTime = 1_776_211_200;
        const MAY_15_2026: LibertasDateTime = 1_778_803_200;
        const OCTOBER_15_2026: LibertasDateTime = 1_792_022_400;
        const NOVEMBER_15_2026: LibertasDateTime = 1_794_700_800;
        let at_cutoff = SprinklerWeatherLocationV1 {
            latitude_degrees: 35.0,
            ..location()
        };
        let below_cutoff = SprinklerWeatherLocationV1 {
            latitude_degrees: 34.999,
            ..location()
        };
        assert!(!location_is_in_winterization_season(
            below_cutoff,
            NOVEMBER_15_2026
        ));
        assert!(!location_is_in_winterization_season(
            at_cutoff,
            OCTOBER_15_2026
        ));
        assert!(location_is_in_winterization_season(
            at_cutoff,
            NOVEMBER_15_2026
        ));

        let southern = SprinklerWeatherLocationV1 {
            latitude_degrees: -35.0,
            ..location()
        };
        assert!(!location_is_in_winterization_season(
            southern,
            APRIL_15_2026
        ));
        assert!(location_is_in_winterization_season(southern, MAY_15_2026));
    }

    #[test]
    fn fresh_freezing_weather_overrides_the_latitude_cutoff() {
        let tropical_location = SprinklerWeatherLocationV1 {
            latitude_degrees: 1.0,
            ..location()
        };
        let mut freezing_current = current();
        freezing_current.temperature_celsius = SAFE_MINIMUM_TEMPERATURE_CELSIUS;
        let weather = SprinklerWeatherSnapshotV2 {
            history: None,
            current: Some(freezing_current),
            forecast: None,
        };
        assert_eq!(
            winterization_reminder_evidence(
                SprinklerWateringModeV1::Active,
                &weather,
                Some(tropical_location),
                NOW,
            ),
            Some(WinterizationReminderEvidence::FreezingWeather {
                temperature_celsius: SAFE_MINIMUM_TEMPERATURE_CELSIUS,
            })
        );
        assert_eq!(
            winterization_reminder_evidence(
                SprinklerWateringModeV1::Winterization,
                &weather,
                Some(tropical_location),
                NOW,
            ),
            None
        );

        freezing_current.valid_until = NOW;
        let stale_weather = SprinklerWeatherSnapshotV2 {
            history: None,
            current: Some(freezing_current),
            forecast: None,
        };
        assert_eq!(
            winterization_reminder_evidence(
                SprinklerWateringModeV1::Active,
                &stale_weather,
                Some(tropical_location),
                NOW,
            ),
            None
        );

        let forecast_weather = SprinklerWeatherSnapshotV2 {
            history: None,
            current: None,
            forecast: Some(SprinklerWeatherForecastV1 {
                retrieved_at: NOW,
                valid_until: NOW + 1_800,
                periods: vec![SprinklerWeatherForecastPeriodV1 {
                    starts_at: NOW + 3_600,
                    duration_seconds: 3_600,
                    temperature_celsius: -1.0,
                    relative_humidity_percent: 80,
                    precipitation_probability_percent: 0,
                    expected_precipitation_millimeters: 0.0,
                    reference_evapotranspiration_millimeters: 0.0,
                    wind_speed_meters_per_second: 0.0,
                    wind_gust_meters_per_second: 0.0,
                }],
            }),
        };
        assert_eq!(
            winterization_reminder_evidence(
                SprinklerWateringModeV1::Active,
                &forecast_weather,
                Some(tropical_location),
                NOW,
            ),
            Some(WinterizationReminderEvidence::FreezingWeather {
                temperature_celsius: -1.0,
            })
        );
    }

    #[test]
    fn winterization_reminder_is_throttled_but_weather_escalates_immediately() {
        let seasonal = WinterizationReminderEvidence::LocationAndSeason;
        let freezing = WinterizationReminderEvidence::FreezingWeather {
            temperature_celsius: -2.0,
        };
        assert!(winterization_reminder_is_due(None, seasonal, NOW));
        let previous = SprinklerWinterizationReminderMemoryV1 {
            last_reminded_at: NOW,
            reason: SprinklerWinterizationReminderReasonV1::LocationAndSeason,
        };
        assert!(!winterization_reminder_is_due(
            Some(previous),
            seasonal,
            NOW + WINTERIZATION_REMINDER_INTERVAL_SECONDS - 1
        ));
        assert!(winterization_reminder_is_due(
            Some(previous),
            seasonal,
            NOW + WINTERIZATION_REMINDER_INTERVAL_SECONDS
        ));
        assert!(winterization_reminder_is_due(
            Some(previous),
            freezing,
            NOW + 60
        ));
    }

    #[test]
    fn no_weather_still_projects_and_schedules_watering() {
        assert_eq!(memory().balance_baseline_at, NOW);
        assert_eq!(memory().baseline_deficit_millimeters, 0.0);
        let active_state = calculate_active_state(
            &runtime(memory()),
            &SprinklerWeatherSnapshotV2 {
                history: None,
                current: None,
                forecast: None,
            },
            false,
            None,
            NOW,
        );
        assert_eq!(
            active_state.condition,
            SprinklerScheduleConditionV1::OfflineWeatherEstimate
        );
        assert_eq!(
            active_state.water_demand_source,
            SprinklerWaterDemandSourceV1::ConservativeDefault
        );
        assert_eq!(
            active_state.next_watering.starts_at,
            NOW + 4 * SECONDS_PER_DAY
        );
        assert!((active_state.planned_water_millimeters - 9.6).abs() < 0.001);
        assert_eq!(active_state.next_watering.duration_seconds, 48 * 60);
    }

    #[test]
    fn solar_position_uses_utc_and_location_without_a_timezone() {
        let dawn = solar_position(equator_location(), EQUINOX_DAY_START + 6 * 3_600).unwrap();
        let noon = solar_position(equator_location(), EQUINOX_DAY_START + 12 * 3_600).unwrap();

        assert!(dawn.rising);
        assert!(dawn.elevation_degrees.abs() < 5.0);
        assert!(noon.elevation_degrees > 85.0);
    }

    #[test]
    fn forecast_moves_a_noncritical_run_into_a_rising_sun_window() {
        let now = EQUINOX_DAY_START + 4 * 3_600;
        let mut memory = default_memory(now);
        memory.baseline_deficit_millimeters = 14.0;
        memory.balance_baseline_at = now;
        let zone = runtime(memory);
        let weather = SprinklerWeatherSnapshotV2 {
            history: None,
            current: None,
            forecast: Some(morning_forecast(now, 70)),
        };

        let state = calculate_active_state(&zone, &weather, true, Some(equator_location()), now);
        let solar = solar_position(equator_location(), state.next_watering.starts_at).unwrap();

        assert!(state.next_watering.starts_at > now);
        assert!(solar.rising);
        assert!(
            (OVERHEAD_MINIMUM_SOLAR_ELEVATION_DEGREES..=OVERHEAD_MAXIMUM_SOLAR_ELEVATION_DEGREES)
                .contains(&solar.elevation_degrees)
        );
    }

    #[test]
    fn high_humidity_moves_overhead_watering_closer_to_sunrise() {
        let now = EQUINOX_DAY_START + 4 * 3_600;
        let mut memory = default_memory(now);
        memory.baseline_deficit_millimeters = 14.0;
        memory.balance_baseline_at = now;
        let zone = runtime(memory);
        let scheduled_at = |relative_humidity_percent| {
            calculate_active_state(
                &zone,
                &SprinklerWeatherSnapshotV2 {
                    history: None,
                    current: None,
                    forecast: Some(morning_forecast(now, relative_humidity_percent)),
                },
                true,
                Some(equator_location()),
                now,
            )
            .next_watering
            .starts_at
        };

        assert!(scheduled_at(95) > scheduled_at(55));
    }

    #[test]
    fn optimized_morning_search_skips_hold_offs() {
        let now = EQUINOX_DAY_START + 4 * 3_600;
        let weather = SprinklerWeatherSnapshotV2 {
            history: None,
            current: None,
            forecast: Some(morning_forecast(now, 70)),
        };
        let base_memory = || {
            let mut memory = default_memory(now);
            memory.baseline_deficit_millimeters = 14.0;
            memory.balance_baseline_at = now;
            memory
        };
        let unheld = calculate_active_state(
            &runtime(base_memory()),
            &weather,
            true,
            Some(equator_location()),
            now,
        );
        let hold_off = SprinklerTimeSlotV1 {
            starts_at: unheld.next_watering.starts_at.saturating_sub(900),
            duration_seconds: 3_600,
        };
        let mut held_memory = base_memory();
        held_memory.hold_off_periods = vec![hold_off];
        let held = calculate_active_state(
            &runtime(held_memory),
            &weather,
            true,
            Some(equator_location()),
            now,
        );

        assert_ne!(held.next_watering.starts_at, unheld.next_watering.starts_at);
        assert!(!held.next_watering.overlaps(hold_off));
    }

    #[test]
    fn hold_off_preemption_requires_a_critical_post_hold_off_deficit() {
        let now = EQUINOX_DAY_START + 4 * 3_600;
        let hold_off = SprinklerTimeSlotV1 {
            starts_at: now + 2 * 3_600,
            duration_seconds: 72 * 3_600,
        };
        let mut memory = default_memory(now);
        memory.baseline_deficit_millimeters = 13.0;
        memory.balance_baseline_at = now;
        memory.hold_off_periods = vec![hold_off];
        let weather = SprinklerWeatherSnapshotV2 {
            history: None,
            current: Some(current_at(now)),
            forecast: Some(safe_hourly_forecast(now, 80)),
        };

        let state = calculate_active_state(
            &runtime(memory),
            &weather,
            true,
            Some(equator_location()),
            now,
        );

        assert_eq!(
            state.condition,
            SprinklerScheduleConditionV1::PreemptiveHoldOff
        );
        assert!(state.next_watering.ends_at().unwrap() <= hold_off.starts_at);
        assert!(state.estimated_deficit_millimeters >= 13.0);
    }

    #[test]
    fn noncritical_post_hold_off_deficit_does_not_preempt() {
        let now = EQUINOX_DAY_START + 4 * 3_600;
        let hold_off = SprinklerTimeSlotV1 {
            starts_at: now + 2 * 3_600,
            duration_seconds: 36 * 3_600,
        };
        let mut memory = default_memory(now);
        memory.baseline_deficit_millimeters = 13.0;
        memory.balance_baseline_at = now;
        memory.hold_off_periods = vec![hold_off];
        let weather = SprinklerWeatherSnapshotV2 {
            history: None,
            current: Some(current_at(now)),
            forecast: Some(safe_hourly_forecast(now, 42)),
        };

        let state = calculate_active_state(
            &runtime(memory),
            &weather,
            true,
            Some(equator_location()),
            now,
        );

        assert_eq!(state.condition, SprinklerScheduleConditionV1::HeldOff);
        assert!(state.next_watering.starts_at >= hold_off.ends_at().unwrap());
    }

    #[test]
    fn hold_off_does_not_preempt_before_the_preferred_deficit() {
        let now = EQUINOX_DAY_START + 4 * 3_600;
        let hold_off = SprinklerTimeSlotV1 {
            starts_at: now + 2 * 3_600,
            duration_seconds: 96 * 3_600,
        };
        let mut memory = default_memory(now);
        memory.baseline_deficit_millimeters = 10.0;
        memory.balance_baseline_at = now;
        memory.hold_off_periods = vec![hold_off];
        let weather = SprinklerWeatherSnapshotV2 {
            history: None,
            current: Some(current_at(now)),
            forecast: Some(safe_hourly_forecast(now, 104)),
        };

        let state = calculate_active_state(
            &runtime(memory),
            &weather,
            true,
            Some(equator_location()),
            now,
        );

        assert_eq!(state.condition, SprinklerScheduleConditionV1::HeldOff);
        assert!(state.next_watering.starts_at >= hold_off.ends_at().unwrap());
    }

    #[test]
    fn expected_rain_rejects_hold_off_preemption() {
        let now = EQUINOX_DAY_START + 4 * 3_600;
        let hold_off = SprinklerTimeSlotV1 {
            starts_at: now + 2 * 3_600,
            duration_seconds: 72 * 3_600,
        };
        let mut memory = default_memory(now);
        memory.baseline_deficit_millimeters = 13.0;
        memory.balance_baseline_at = now;
        memory.hold_off_periods = vec![hold_off];
        let mut forecast = safe_hourly_forecast(now, 80);
        forecast.periods[10].precipitation_probability_percent = 90;
        forecast.periods[10].expected_precipitation_millimeters = 20.0;
        let weather = SprinklerWeatherSnapshotV2 {
            history: None,
            current: Some(current_at(now)),
            forecast: Some(forecast),
        };

        let state = calculate_active_state(
            &runtime(memory),
            &weather,
            true,
            Some(equator_location()),
            now,
        );

        assert_eq!(state.condition, SprinklerScheduleConditionV1::HeldOff);
        assert!(state.next_watering.starts_at >= hold_off.ends_at().unwrap());
    }

    #[test]
    fn unsafe_forecast_rejects_hold_off_preemption() {
        let now = EQUINOX_DAY_START + 4 * 3_600;
        let hold_off = SprinklerTimeSlotV1 {
            starts_at: now + 2 * 3_600,
            duration_seconds: 72 * 3_600,
        };
        let mut memory = default_memory(now);
        memory.baseline_deficit_millimeters = 13.0;
        memory.balance_baseline_at = now;
        memory.hold_off_periods = vec![hold_off];
        let mut forecast = safe_hourly_forecast(now, 80);
        for period in &mut forecast.periods[..2] {
            period.wind_speed_meters_per_second = 20.0;
            period.wind_gust_meters_per_second = 30.0;
        }
        let weather = SprinklerWeatherSnapshotV2 {
            history: None,
            current: Some(current_at(now)),
            forecast: Some(forecast),
        };

        let state = calculate_active_state(
            &runtime(memory),
            &weather,
            true,
            Some(equator_location()),
            now,
        );

        assert_eq!(state.condition, SprinklerScheduleConditionV1::HeldOff);
        assert!(state.next_watering.starts_at >= hold_off.ends_at().unwrap());
    }

    #[test]
    fn post_hold_off_make_up_is_sized_at_its_delayed_start() {
        let hold_off = SprinklerTimeSlotV1 {
            starts_at: NOW,
            duration_seconds: 3 * SECONDS_PER_DAY as u32,
        };
        let mut memory = memory();
        memory.baseline_deficit_millimeters = 16.0;
        memory.balance_baseline_at = NOW;
        memory.hold_off_periods = vec![hold_off];
        let zone = runtime(memory);
        let demand_estimate = water_demand_estimate(&zone.water_events, None, NOW);
        let capacity = root_zone_capacity_millimeters(&zone.configuration);
        let expected = watering_plan_at(
            &zone,
            hold_off.ends_at().unwrap(),
            capacity,
            demand_estimate,
        );

        let state = calculate_active_state(
            &zone,
            &SprinklerWeatherSnapshotV2 {
                history: None,
                current: None,
                forecast: None,
            },
            false,
            None,
            NOW,
        );

        assert_eq!(state.condition, SprinklerScheduleConditionV1::HeldOff);
        assert_eq!(state.next_watering.starts_at, expected.starts_at);
        assert_eq!(
            state.next_watering.duration_seconds,
            expected.duration_seconds
        );
        assert_eq!(
            state.planned_water_millimeters,
            expected.planned_water_millimeters
        );
        assert!(state.planned_water_millimeters > 9.6);
    }

    #[test]
    fn critical_deficit_uses_the_first_safe_opportunity() {
        let now = EQUINOX_DAY_START + 12 * 3_600;
        let mut memory = default_memory(now);
        memory.baseline_deficit_millimeters =
            root_zone_capacity_millimeters(&zone()) * CRITICAL_DEFICIT_RATIO;
        memory.balance_baseline_at = now;
        let zone = runtime(memory);
        let weather = SprinklerWeatherSnapshotV2 {
            history: None,
            current: None,
            forecast: Some(morning_forecast(now, 60)),
        };

        let state = calculate_active_state(&zone, &weather, true, Some(equator_location()), now);

        assert_eq!(state.next_watering.starts_at, now);
    }

    #[test]
    fn drip_watering_tolerates_wind_that_blocks_overhead_heads() {
        let mut windy = current();
        windy.wind_speed_meters_per_second = 12.0;
        windy.wind_gust_meters_per_second = 18.0;
        let weather = SprinklerWeatherSnapshotV2 {
            history: None,
            current: Some(windy),
            forecast: None,
        };

        assert!(!weather_permits_immediate_watering(
            &weather,
            SprinklerHeadTypeV1::PopupSpray,
            NOW,
        ));
        assert!(weather_permits_immediate_watering(
            &weather,
            SprinklerHeadTypeV1::SurfaceDrip,
            NOW,
        ));
    }

    #[test]
    fn water_amount_adjuster_uses_twenty_to_two_hundred_percent() {
        assert!(valid_watering_percent(20));
        assert!(valid_watering_percent(100));
        assert!(valid_watering_percent(200));
        assert!(!valid_watering_percent(10));
        assert!(!valid_watering_percent(25));
        assert!(!valid_watering_percent(210));

        let weather = SprinklerWeatherSnapshotV2 {
            history: Some(history()),
            current: Some(current()),
            forecast: None,
        };
        let active_state = |watering_percent| {
            let mut memory = memory();
            memory.balance_baseline_at = NOW;
            memory.baseline_deficit_millimeters = 20.0;
            memory.watering_percent = watering_percent;
            calculate_active_state(&runtime(memory), &weather, true, None, NOW)
        };
        let less = active_state(20);
        let automatic = active_state(100);
        let more = active_state(200);
        assert!(
            (less.planned_water_millimeters - automatic.planned_water_millimeters * 0.2).abs()
                < 0.001
        );
        assert!(
            (more.planned_water_millimeters - automatic.planned_water_millimeters * 2.0).abs()
                < 0.001
        );
        assert!(less.next_watering.duration_seconds < automatic.next_watering.duration_seconds);
        assert!(automatic.next_watering.duration_seconds < more.next_watering.duration_seconds);
    }

    #[test]
    fn plant_type_drives_adaptive_weather_demand() {
        let memory = memory();
        let water_events = vec![SprinklerWaterEventV1::WeatherV1 {
            starts_at: NOW - 3_600,
            duration_seconds: 3_600,
            precipitation_millimeters: 0.0,
            reference_evapotranspiration_millimeters: 10.0,
        }];
        let mut lawn = zone();
        lawn.plant_type = SprinklerPlantTypeV1::Lawn;
        let mut xeriscape = zone();
        xeriscape.plant_type = SprinklerPlantTypeV1::Xeriscape;

        assert!((estimated_deficit_millimeters(&lawn, &memory, &water_events) - 8.0).abs() < 0.001);
        assert!(
            (estimated_deficit_millimeters(&xeriscape, &memory, &water_events) - 3.0).abs() < 0.001
        );
    }

    #[test]
    fn sprinkler_head_type_sets_duration_without_a_rate_input() {
        let mut slow = zone();
        slow.sprinkler_head_type = SprinklerHeadTypeV1::SurfaceDrip;
        let mut fast = zone();
        fast.sprinkler_head_type = SprinklerHeadTypeV1::PopupSpray;

        assert_eq!(watering_duration_seconds(&slow, 8.0), 3_600);
        assert_eq!(watering_duration_seconds(&fast, 8.0), 720);
    }

    #[test]
    fn hold_offs_are_sorted_merged_and_shift_the_schedule() {
        let normalized = normalize_hold_offs(vec![
            SprinklerTimeSlotV1 {
                starts_at: NOW + 300,
                duration_seconds: 300,
            },
            SprinklerTimeSlotV1 {
                starts_at: NOW,
                duration_seconds: 400,
            },
        ])
        .unwrap();
        assert_eq!(
            normalized,
            vec![SprinklerTimeSlotV1 {
                starts_at: NOW,
                duration_seconds: 600,
            }]
        );
        assert_eq!(
            shift_after_hold_offs(NOW + 100, 60, &normalized),
            (NOW + 600, true)
        );
    }

    #[test]
    fn expired_hold_offs_are_pruned_once_at_the_end_boundary() {
        let active = SprinklerTimeSlotV1 {
            starts_at: NOW + 1,
            duration_seconds: 300,
        };
        let mut memory = memory();
        memory.hold_off_periods = vec![
            SprinklerTimeSlotV1 {
                starts_at: NOW - 300,
                duration_seconds: 299,
            },
            SprinklerTimeSlotV1 {
                starts_at: NOW - 300,
                duration_seconds: 300,
            },
            active,
        ];

        assert!(prune_expired_hold_offs(&mut memory, NOW));
        assert_eq!(memory.hold_off_periods, vec![active]);
        assert!(!prune_expired_hold_offs(&mut memory, NOW));
    }

    #[test]
    fn water_balance_combines_weather_and_observed_irrigation() {
        let memory = memory();
        let water_events = vec![
            SprinklerWaterEventV1::WeatherV1 {
                starts_at: NOW - 3_600,
                duration_seconds: 3_600,
                precipitation_millimeters: 1.0,
                reference_evapotranspiration_millimeters: 6.0,
            },
            SprinklerWaterEventV1::IrrigationV1 {
                starts_at: NOW - 1_800,
                duration_seconds: 900,
                watering_percent: 100,
                applied_water_millimeters: 3.0,
            },
        ];
        let deficit = estimated_deficit_millimeters(&zone(), &memory, &water_events);
        assert!((deficit - 0.8).abs() < 0.001);
    }

    #[test]
    fn noon_irrigation_creates_an_exact_all_zone_balance_recovery() {
        let day = utc_day_start(NOW);
        let noon = day + 12 * 3_600;
        let range = SprinklerReportTimeRangeV1 {
            starts_at: day,
            ends_before: noon + 3_600,
        };
        let mut report = completed_daily_report(day);
        report.opening_deficit_millimeters = 10.0;
        let mut history = report_weather_period(day + 6 * 3_600);
        history.duration_seconds = 6 * 3_600;
        history.precipitation_millimeters = 0.0;
        history.reference_evapotranspiration_millimeters = 6.0;
        let mut activity = completed_report_activity(noon);
        activity.actual_duration_seconds = Some(3_600);
        activity.applied_water_millimeters = Some(8.0);
        activity.updated_at = noon + 3_600;
        let modeled_gap = SprinklerModeledWeatherGapV1 {
            starts_at: day,
            ends_before: day + 6 * 3_600,
            reference_evapotranspiration_millimeters_per_day: 4.0,
            demand_source: SprinklerWaterDemandSourceV1::ConservativeDefault,
            recorded_at: day,
        };
        let report_zone = ReportZoneData {
            valve: zone().valve,
            capacity_millimeters: root_zone_capacity_millimeters(&zone()),
            crop_coefficient: plant_profile(zone().plant_type).crop_coefficient,
            active_state: runtime(memory()).active_state,
            water_events: Vec::new(),
            modeled_weather_gaps: vec![modeled_gap],
            current_activity: None,
            activities: vec![activity],
            daily_reports: vec![report],
        };

        let chart = build_water_balance_chart(
            core::slice::from_ref(&report_zone),
            &[history.into()],
            range,
        )
        .unwrap();
        let start = chart
            .iter()
            .find(|point| {
                point.series == SprinklerWaterBalanceSeriesV1::AvailableWater && point.at == day
            })
            .unwrap();
        let before_irrigation = chart
            .iter()
            .find(|point| {
                point.series == SprinklerWaterBalanceSeriesV1::AvailableWater && point.at == noon
            })
            .unwrap();
        let after_irrigation = chart
            .iter()
            .find(|point| {
                point.series == SprinklerWaterBalanceSeriesV1::AvailableWater
                    && point.at == noon + 3_600
            })
            .unwrap();
        assert!(before_irrigation.available_water_percent < start.available_water_percent);
        let expected_recovery = 8.0 / report_zone.capacity_millimeters * 100.0;
        assert!(
            (after_irrigation.available_water_percent
                - before_irrigation.available_water_percent
                - expected_recovery)
                .abs()
                < 0.001
        );
        assert!(chart.iter().all(|point| point.zone == report_zone.valve));
    }

    #[test]
    fn observed_valve_time_becomes_recent_irrigation() {
        let mut runtime = runtime(memory());
        add_irrigation_event(&mut runtime, NOW - 600, 600, None, NOW);
        let SprinklerWaterEventV1::IrrigationV1 {
            watering_percent,
            applied_water_millimeters,
            ..
        } = runtime.water_events[0]
        else {
            panic!("expected irrigation event");
        };
        assert_eq!(watering_percent, 100);
        assert!((applied_water_millimeters - 2.0).abs() < 0.001);
    }

    #[test]
    fn watering_duration_rounds_up_and_respects_runtime_bounds() {
        let zone = zone();
        assert_eq!(watering_duration_seconds(&zone, 0.0), 0);
        assert_eq!(watering_duration_seconds(&zone, 0.001), 60);
        assert_eq!(watering_duration_seconds(&zone, 12.0 * 60.5 / 3_600.0), 61);
        assert_eq!(
            watering_duration_seconds(&zone, 25.0),
            MAX_WATERING_DURATION_SECONDS
        );
        assert_eq!(watering_duration_seconds(&zone, f32::MAX), 0);
    }

    #[test]
    fn consecutive_valve_checkpoints_form_one_irrigation_event() {
        let mut runtime = runtime(memory());
        add_irrigation_event(&mut runtime, NOW - 120, 60, None, NOW - 60);
        add_irrigation_event(&mut runtime, NOW - 60, 60, None, NOW);
        assert_eq!(runtime.water_events.len(), 1);
        let SprinklerWaterEventV1::IrrigationV1 {
            duration_seconds,
            watering_percent,
            applied_water_millimeters,
            ..
        } = runtime.water_events[0]
        else {
            panic!("expected irrigation event");
        };
        assert_eq!(duration_seconds, 120);
        assert_eq!(watering_percent, 100);
        assert!((applied_water_millimeters - 0.4).abs() < 0.001);
    }

    #[test]
    fn planned_open_does_not_count_as_water_until_valve_open_is_observed() {
        let mut zone_runtime = runtime(memory());
        let activity_index = watering_activity_index(NOW, SprinklerWateringOriginV1::Automatic, 0)
            .expect("valid activity index");
        zone_runtime.current_activity = Some(SprinklerWateringActivityV1 {
            activity_index,
            activity_ordinal: 0,
            origin: SprinklerWateringOriginV1::Automatic,
            outcome: SprinklerWateringOutcomeV1::Scheduled,
            reason: SprinklerWateringReasonV1::SmartSchedule,
            scheduled_starts_at: Some(NOW),
            scheduled_duration_seconds: Some(900),
            planned_water_millimeters: Some(3.0),
            actual_starts_at: None,
            actual_duration_seconds: None,
            applied_water_millimeters: None,
            watering_percent: 100,
            updated_at: NOW,
        });
        let scheduled_activity = zone_runtime.current_activity.clone();
        assert!(begin_expected_irrigation(&mut zone_runtime, NOW, 900));
        assert!(zone_runtime.water_events.is_empty());
        let reservation_delta = water_event_delta(&[], &zone_runtime.water_events);
        assert!(reservation_delta.upserts.is_empty());
        assert!(reservation_delta.removals.is_empty());

        zone_runtime.valve_is_open = true;
        zone_runtime.valve_opened_automatically = true;
        zone_runtime.current_activity.as_mut().unwrap().outcome =
            SprinklerWateringOutcomeV1::Running;
        zone_runtime.accounted_at_ticks = Some(10 * MICROSECONDS_PER_SECOND);
        zone_runtime.accounted_at_utc = Some(NOW);
        assert!(account_open_zone(
            &mut zone_runtime,
            310 * MICROSECONDS_PER_SECOND,
            Some(NOW + 300),
            None,
        ));
        assert_eq!(zone_runtime.water_events.len(), 1);

        assert!(account_open_zone(
            &mut zone_runtime,
            610 * MICROSECONDS_PER_SECOND,
            Some(NOW + 600),
            None,
        ));
        let observed_events = zone_runtime.water_events.clone();

        assert!(reconcile_expected_irrigation(
            &mut zone_runtime,
            610 * MICROSECONDS_PER_SECOND
        ));
        let SprinklerWaterEventV1::IrrigationV1 {
            duration_seconds,
            applied_water_millimeters,
            ..
        } = zone_runtime.water_events[0]
        else {
            panic!("expected irrigation event");
        };
        assert_eq!(duration_seconds, 600);
        assert!((applied_water_millimeters - 2.0).abs() < 0.001);
        let activity = zone_runtime.current_activity.as_ref().unwrap();
        assert_eq!(activity.scheduled_duration_seconds, Some(900));
        assert_eq!(activity.planned_water_millimeters, Some(3.0));
        assert_eq!(activity.actual_duration_seconds, Some(600));
        assert_eq!(activity.applied_water_millimeters, Some(2.0));
        assert_eq!(zone_runtime.expected_irrigation, None);
        assert_eq!(observed_events, zone_runtime.water_events);

        let mut unopened = runtime(memory());
        unopened.current_activity = scheduled_activity;
        assert!(begin_expected_irrigation(&mut unopened, NOW, 900));
        assert!(reconcile_expected_irrigation(
            &mut unopened,
            610 * MICROSECONDS_PER_SECOND
        ));
        assert!(unopened.water_events.is_empty());
    }

    #[test]
    fn manual_open_is_observed_and_counted_until_close() {
        let mut runtime = runtime(memory());
        runtime.valve_is_open = true;
        runtime.valve_opened_automatically = false;
        runtime.accounted_at_ticks = Some(10 * MICROSECONDS_PER_SECOND);
        runtime.accounted_at_utc = Some(NOW);

        assert!(account_open_zone(
            &mut runtime,
            40 * MICROSECONDS_PER_SECOND,
            Some(NOW + 30),
            None,
        ));
        assert!(account_open_zone(
            &mut runtime,
            80 * MICROSECONDS_PER_SECOND,
            Some(NOW + 70),
            None,
        ));
        assert_eq!(runtime.water_events.len(), 1);
        let SprinklerWaterEventV1::IrrigationV1 {
            duration_seconds,
            applied_water_millimeters,
            ..
        } = runtime.water_events[0]
        else {
            panic!("expected irrigation event");
        };
        assert_eq!(duration_seconds, 70);
        assert!((applied_water_millimeters - 12.0 * 70.0 / 3_600.0).abs() < 0.001);
    }

    #[test]
    fn valve_decisions_wait_through_the_ten_second_close_boundary() {
        let closed_at = 123 * MICROSECONDS_PER_SECOND;
        let deadline = absolute_interval_ticks(closed_at, VALVE_DECISION_DELAY_SECONDS);
        assert!(!valve_decision_allowed(closed_at, deadline));
        assert!(!valve_decision_allowed(deadline - 1, deadline));
        assert!(valve_decision_allowed(deadline, deadline));
    }

    #[test]
    fn watering_percentage_change_splits_adjacent_irrigation_events() {
        let mut runtime = runtime(memory());
        add_irrigation_event(&mut runtime, NOW - 120, 60, None, NOW - 60);
        runtime.memory.watering_percent = 80;
        add_irrigation_event(&mut runtime, NOW - 60, 60, None, NOW);

        assert_eq!(runtime.water_events.len(), 2);
        let percentages: Vec<_> = runtime
            .water_events
            .iter()
            .map(|event| match event {
                SprinklerWaterEventV1::IrrigationV1 {
                    watering_percent, ..
                } => *watering_percent,
                SprinklerWaterEventV1::WeatherV1 { .. } => 0,
            })
            .collect();
        assert_eq!(percentages, vec![100, 80]);
    }

    #[test]
    fn irrigation_event_rejects_invalid_watering_percentage() {
        assert!(!valid_water_event(&SprinklerWaterEventV1::IrrigationV1 {
            starts_at: NOW,
            duration_seconds: 60,
            watering_percent: 25,
            applied_water_millimeters: 0.2,
        }));
    }

    #[test]
    fn history_at_the_persisted_baseline_is_not_counted_again() {
        let mut memory = memory();
        memory.balance_baseline_at = NOW - 3_600;
        let mut water_events = Vec::new();
        let history = SprinklerWeatherHistoryV2 {
            retrieved_at: NOW,
            valid_until: NOW + 3_600,
            periods: vec![
                SprinklerWeatherHistoryPeriodV2 {
                    starts_at: NOW - 7_200,
                    duration_seconds: 3_600,
                    temperature_celsius: 18.0,
                    relative_humidity_percent: 70,
                    precipitation_millimeters: 10.0,
                    reference_evapotranspiration_millimeters: 0.0,
                    wind_speed_meters_per_second: 2.0,
                    wind_gust_meters_per_second: 4.0,
                },
                SprinklerWeatherHistoryPeriodV2 {
                    starts_at: NOW - 3_600,
                    duration_seconds: 3_600,
                    temperature_celsius: 20.0,
                    relative_humidity_percent: 60,
                    precipitation_millimeters: 1.0,
                    reference_evapotranspiration_millimeters: 2.0,
                    wind_speed_meters_per_second: 3.0,
                    wind_gust_meters_per_second: 5.0,
                },
            ],
        };
        assert!(synchronize_history(
            &memory,
            &mut water_events,
            Some(&history)
        ));
        assert_eq!(water_events.len(), 1);
        assert_eq!(water_events[0].starts_at(), NOW - 3_600);
    }

    #[test]
    fn weather_reset_cursor_rules_require_the_matching_reason_and_order() {
        let previous = SprinklerWeatherCursorV1 {
            epoch_timestamp: NOW,
            sequence: 10,
        };
        use libertas_weather::SprinklerWeatherResetReasonV1;

        assert!(!valid_weather_reset_cursor(
            Some(previous),
            SprinklerWeatherResetReasonV1::ServerCursorReset,
            SprinklerWeatherCursorV1 {
                epoch_timestamp: NOW,
                sequence: 3,
            },
        ));
        let accepted = SprinklerWeatherCursorV1 {
            epoch_timestamp: NOW + 1,
            sequence: 3,
        };
        assert!(valid_weather_reset_cursor(
            Some(previous),
            SprinklerWeatherResetReasonV1::ServerCursorReset,
            accepted,
        ));
        assert!(!valid_weather_reset_cursor(
            Some(previous),
            SprinklerWeatherResetReasonV1::CursorExpired,
            accepted,
        ));
        assert!(valid_weather_reset_cursor(
            None,
            SprinklerWeatherResetReasonV1::InitialSubscription,
            SprinklerWeatherCursorV1 {
                epoch_timestamp: NOW,
                sequence: 0,
            },
        ));
    }

    #[test]
    fn explicit_site_replacements_bind_every_generation_without_clear_acknowledgements() {
        let first = location();
        let second = SprinklerWeatherLocationV1 {
            latitude_degrees: 35.0,
            longitude_degrees: -80.0,
        };
        let third = SprinklerWeatherLocationV1 {
            latitude_degrees: 40.0,
            longitude_degrees: -75.0,
        };
        let initial = SprinklerReportWeatherArchiveStateV2 {
            generation: 7,
            location: Some(first),
        };
        let transition = transition_report_weather_sites(
            initial,
            &[
                SprinklerWeatherChangeV1::SiteReplaceV1 { location: second },
                SprinklerWeatherChangeV1::SectionClearV1 {
                    section: SprinklerWeatherSectionV1::History,
                },
                SprinklerWeatherChangeV1::SiteReplaceV1 { location: third },
            ],
        )
        .unwrap();
        assert_eq!(transition.archive_state.generation, 9);
        assert_eq!(transition.archive_state.location, Some(third));
        assert!(transition.binding_changed);
        assert!(transition.generation_changed);

        let same_site = transition_report_weather_sites(
            transition.archive_state,
            &[SprinklerWeatherChangeV1::SectionClearV1 {
                section: SprinklerWeatherSectionV1::History,
            }],
        )
        .unwrap();
        assert_eq!(same_site.archive_state, transition.archive_state);
        assert!(!same_site.binding_changed);
        assert!(!same_site.generation_changed);

        assert!(
            transition_report_weather_site(
                SprinklerReportWeatherArchiveStateV2 {
                    generation: u64::MAX,
                    location: Some(first),
                },
                second,
            )
            .is_none()
        );
    }

    #[test]
    fn pending_legacy_archive_state_migrates_to_reserved_but_unbound_generation() {
        let provider_site = location();
        let migrated = migrate_report_weather_archive_state(SprinklerReportWeatherArchiveStateV1 {
            generation: 4,
            location: Some(provider_site),
            awaiting_history_clear: true,
        });
        assert_eq!(migrated.generation, 4);
        assert_eq!(migrated.location, None);

        let bound = transition_report_weather_site(migrated, provider_site).unwrap();
        assert_eq!(bound.archive_state.generation, 4);
        assert_eq!(bound.archive_state.location, Some(provider_site));
        assert!(bound.binding_changed);
        assert!(!bound.generation_changed);
    }

    #[test]
    fn authoritative_hub_site_rejects_unmarked_old_site_weather() {
        let provider_site = location();
        let hub_site = SprinklerWeatherLocationV1 {
            latitude_degrees: 35.0,
            longitude_degrees: -80.0,
        };
        let cursor = SprinklerWeatherCursorV1 {
            epoch_timestamp: NOW,
            sequence: 5,
        };
        let mut state = controller_state();
        state.report_weather_archive_state = SprinklerReportWeatherArchiveStateV2 {
            generation: 3,
            location: Some(provider_site),
        };
        state.site_location = Some(hub_site);
        state.hub_location_subscription_ready = true;
        state.weather_cursor = Some(cursor);
        let original_weather = state.weather.clone();
        let shared = Rc::new(RefCell::new(state));
        let report = SprinklerWeatherIncrementalReportV1 {
            from_cursor: cursor,
            through_cursor: SprinklerWeatherCursorV1 {
                sequence: 6,
                ..cursor
            },
            changes: vec![SprinklerWeatherChangeV1::CurrentReplaceV1 { current: current() }],
        };

        assert!(!accept_weather_report(&shared, report));
        let state = shared.borrow();
        assert_eq!(state.weather_cursor, Some(cursor));
        assert_eq!(state.weather, original_weather);
        assert_eq!(state.report_weather_archive_state.generation, 3);
    }

    #[test]
    fn invalid_v2_weather_is_rejected_before_cursor_or_site_commit() {
        let site = location();
        let cursor = SprinklerWeatherCursorV1 {
            epoch_timestamp: NOW,
            sequence: 5,
        };
        let mut invalid_period = report_weather_period(NOW - 3_600);
        invalid_period.wind_gust_meters_per_second = f32::NAN;
        let mut state = controller_state();
        state.report_weather_archive_state = SprinklerReportWeatherArchiveStateV2 {
            generation: 3,
            location: Some(site),
        };
        state.site_location = Some(site);
        state.hub_location_subscription_ready = true;
        state.weather_cursor = Some(cursor);
        let shared = Rc::new(RefCell::new(state));
        let report = SprinklerWeatherIncrementalReportV1 {
            from_cursor: cursor,
            through_cursor: SprinklerWeatherCursorV1 {
                sequence: 6,
                ..cursor
            },
            changes: vec![SprinklerWeatherChangeV1::HistoryReplaceV2 {
                history: SprinklerWeatherHistoryV2 {
                    retrieved_at: NOW,
                    valid_until: NOW + 3_600,
                    periods: vec![invalid_period],
                },
            }],
        };

        assert!(!accept_weather_report(&shared, report));
        let state = shared.borrow();
        assert_eq!(state.weather_cursor, Some(cursor));
        assert_eq!(state.report_weather_archive_state.generation, 3);
    }

    #[test]
    fn unbound_legacy_reset_cannot_mix_current_or_forecast_across_sites() {
        use libertas_weather::{SprinklerWeatherResetReasonV1, SprinklerWeatherSnapshotV1};

        let cursor = SprinklerWeatherCursorV1 {
            epoch_timestamp: NOW,
            sequence: 5,
        };
        let mut state = controller_state();
        state.weather_cursor = Some(cursor);
        state.weather.current = Some(current());
        let original_weather = state.weather.clone();
        let shared = Rc::new(RefCell::new(state));
        let recovery = SprinklerWeatherRecoveryV1::ResetV1 {
            reason: SprinklerWeatherResetReasonV1::ServerCursorReset,
            cursor: SprinklerWeatherCursorV1 {
                epoch_timestamp: NOW + 1,
                sequence: 1,
            },
            snapshot: SprinklerWeatherSnapshotV1 {
                history: None,
                current: Some(SprinklerCurrentWeatherV1 {
                    temperature_celsius: -20.0,
                    ..current()
                }),
                forecast: None,
            },
        };

        assert!(!accept_weather_recovery(&shared, recovery));
        let state = shared.borrow();
        assert_eq!(state.weather_cursor, Some(cursor));
        assert_eq!(state.weather, original_weather);
    }

    #[test]
    fn peer_alive_refresh_path_does_not_touch_weather_data_or_cursor() {
        let cursor = SprinklerWeatherCursorV1 {
            epoch_timestamp: NOW,
            sequence: 10,
        };
        let weather = SprinklerWeatherSnapshotV2 {
            history: Some(history()),
            current: Some(current()),
            forecast: None,
        };
        let mut state = controller_state();
        state.weather = weather.clone();
        state.weather_cursor = Some(cursor);
        let shared = Rc::new(RefCell::new(state));
        let mut context: Box<dyn Any> = Box::new(Rc::clone(&shared));
        let mut peer_alive = || {
            handle_weather_event(
                1,
                OP_ENDPOINT_PEER_ALIVE,
                LibertasEndpointMessage::NoPayload,
                &mut context,
                0,
                99,
            )
        };

        assert_eq!(peer_alive(), LibertasEndpointHandlerResult::Handled);
        let state = shared.borrow();
        assert_eq!(state.weather, weather);
        assert_eq!(state.weather_cursor, Some(cursor));
        assert!(state.weather_stream_ready);
        drop(state);

        shared.borrow_mut().weather_stream_ready = false;
        assert_eq!(peer_alive(), LibertasEndpointHandlerResult::Handled);
        assert!(!shared.borrow().weather_stream_ready);
    }

    #[test]
    fn cursor_ahead_error_restarts_weather_recovery_without_a_cursor() {
        let mut state = controller_state();
        state.weather_cursor = Some(SprinklerWeatherCursorV1 {
            epoch_timestamp: NOW,
            sequence: 10,
        });
        assert_eq!(
            apply_weather_recovery_error(
                &mut state,
                SprinklerWeatherRecoveryErrorV1::CursorAhead,
                None,
            ),
            1
        );
        assert_eq!(state.weather_cursor, None);
        assert!(!state.weather_stream_ready);
    }

    #[test]
    fn fresh_unsafe_weather_blocks_but_missing_or_stale_weather_uses_fallback() {
        let weather = SprinklerWeatherSnapshotV2 {
            history: Some(history()),
            current: Some(current()),
            forecast: None,
        };
        assert!(weather_permits_immediate_watering(
            &weather,
            SprinklerHeadTypeV1::RotorsLowRate,
            NOW
        ));
        assert!(!weather_permits_immediate_watering(
            &SprinklerWeatherSnapshotV2 {
                current: Some(SprinklerCurrentWeatherV1 {
                    precipitation_millimeters: 0.1,
                    ..current()
                }),
                ..weather.clone()
            },
            SprinklerHeadTypeV1::RotorsLowRate,
            NOW
        ));
        assert!(weather_permits_immediate_watering(
            &weather,
            SprinklerHeadTypeV1::RotorsLowRate,
            NOW + 1_800
        ));
        assert!(weather_permits_immediate_watering(
            &SprinklerWeatherSnapshotV2 {
                history: None,
                current: None,
                forecast: None,
            },
            SprinklerHeadTypeV1::RotorsLowRate,
            NOW
        ));
    }

    #[test]
    fn unknown_valve_state_preserves_the_active_state() {
        let mut memory = memory();
        memory.baseline_deficit_millimeters = 20.0;
        let mut zone = runtime(memory);
        zone.valve_state_known = false;
        let active_state = calculate_active_state(
            &zone,
            &SprinklerWeatherSnapshotV2 {
                history: Some(history()),
                current: Some(current()),
                forecast: None,
            },
            true,
            None,
            NOW,
        );
        assert_eq!(
            active_state.condition,
            SprinklerScheduleConditionV1::ValveStateUnavailable
        );
        assert_eq!(active_state.next_watering.starts_at, NOW);
        assert!(active_state.next_watering.duration_seconds > 0);
        assert!(!valve_permits_automatic_watering(
            &zone,
            SprinklerWateringModeV1::Active
        ));
    }

    #[test]
    fn winterization_is_a_system_wide_hard_interlock() {
        let mut zone = runtime(memory());
        assert!(matches!(
            public_zone_state(&zone, SprinklerWateringModeV1::Active),
            SprinklerZoneStateV1::ActiveV1 { .. }
        ));
        assert_eq!(
            public_zone_state(&zone, SprinklerWateringModeV1::Winterization),
            SprinklerZoneStateV1::WinterizationV1
        );
        assert!(!valve_permits_automatic_watering(
            &zone,
            SprinklerWateringModeV1::Winterization
        ));

        zone.valve_is_open = true;
        zone.valve_opened_automatically = true;
        assert!(automatic_valve_must_close(
            &zone,
            true,
            SprinklerWateringModeV1::Winterization
        ));
        assert!(!automatic_valve_must_close(
            &zone,
            true,
            SprinklerWateringModeV1::Active
        ));
    }

    #[test]
    fn matter_open_command_uses_typed_valve_definition() {
        let command = Open {
            OpenDuration: Some(Nullable::some(900)),
            TargetLevel: None,
        };
        let mut bytes = InlineByteBuffer::new();
        encode_command(&command, &mut bytes).unwrap();
        assert_eq!(decode_command::<Open>(bytes.as_slice()), Ok(command));
    }

    #[test]
    fn rain_forecast_delays_a_needed_schedule() {
        let mut memory = memory();
        memory.baseline_deficit_millimeters = 30.0;
        let zone = runtime(memory);
        let weather = SprinklerWeatherSnapshotV2 {
            history: Some(history()),
            current: Some(current()),
            forecast: Some(SprinklerWeatherForecastV1 {
                retrieved_at: NOW,
                valid_until: NOW + 10_800,
                periods: vec![SprinklerWeatherForecastPeriodV1 {
                    starts_at: NOW,
                    duration_seconds: 3_600,
                    temperature_celsius: 20.0,
                    relative_humidity_percent: 70,
                    precipitation_probability_percent: 90,
                    expected_precipitation_millimeters: 20.0,
                    reference_evapotranspiration_millimeters: 0.2,
                    wind_speed_meters_per_second: 2.0,
                    wind_gust_meters_per_second: 4.0,
                }],
            }),
        };
        let active_state = calculate_active_state(&zone, &weather, true, None, NOW);
        assert_eq!(
            active_state.condition,
            SprinklerScheduleConditionV1::ForecastRain
        );
        assert!(active_state.next_watering.starts_at >= NOW + 3_600);
    }

    #[test]
    fn truncated_persistent_data_is_rejected() {
        let encoded = SprinklerDataV1::ZoneMemoryV1 { memory: memory() }.to_avro();
        assert!(SprinklerDataV1::from_avro(&encoded[..encoded.len() - 1]).is_err());
    }
}
