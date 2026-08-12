//! Libertas Sprinkler
//! Calculates and executes weather-aware watering schedules for sprinkler zones
//! controlled by Matter Valve Configuration and Control devices.
//!
//! Configuration identifies the shared sprinkler weather endpoint, reminder
//! recipients, and, for each zone, its valve, plant type,
//! sprinkler-head type, and state endpoint. The controller adapts each watering
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

use alloc::{boxed::Box, rc::Rc, vec::Vec};
use core::{any::Any, cell::RefCell};
use libm::{asin, cos, floor, sin};

use libertas::{
    IndexDirection, IndexedData, LIBERTAS_HUB_ENDPOINT, LibertasDateTime, LibertasDevice,
    LibertasEndpoint, LibertasEndpointHandlerResult, LibertasEndpointMessage,
    LibertasEndpointStandardStatus, LibertasUser, LogLevel, NotificationArgument,
    NotificationImportance, OP_ENDPOINT_DATA, OP_ENDPOINT_PEER_ALIVE, OP_ENDPOINT_PEER_DOWN,
    OP_ENDPOINT_PEER_UP, OP_ENDPOINT_REQ, OP_ENDPOINT_RSP, OP_ENDPOINT_SUB_REQ,
    libertas_data_open_indexed, libertas_data_read_indexed_range, libertas_data_read_single,
    libertas_data_remove_indexed_records, libertas_data_write_indexed, libertas_data_write_single,
    libertas_endpoint_report, libertas_endpoint_response, libertas_endpoint_subscribe_request,
    libertas_get_sys_ticks, libertas_get_utc_time, libertas_log, libertas_notification_send,
    libertas_register_device_listener, libertas_register_endpoint_status_listener,
    libertas_timer_cancel, libertas_timer_new_interval, libertas_timer_update_interval,
};
use libertas_hub::HubProtocol;
use libertas_macros::{
    LibertasAvroDecode, LibertasAvroEncode, LibertasExport, libertas_data_schema, libertas_export,
    libertas_permissions, libertas_string_resources,
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
    SprinklerWeatherHistoryV1, SprinklerWeatherIncrementalReportV1, SprinklerWeatherLocationV1,
    SprinklerWeatherProtocolV1, SprinklerWeatherRecoveryErrorV1, SprinklerWeatherRecoveryV1,
    SprinklerWeatherSectionV1, SprinklerWeatherSnapshotV1, SprinklerWeatherTimeRangeV1,
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
pub const APP_STRINGS: [(&str, &str); 8] = [
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
];
const ZONE_DATA_RESOURCE: &str = APP_STRINGS[0].0;
const WATER_EVENTS_RESOURCE: &str = APP_STRINGS[1].0;
const SITE_LOCATION_RESOURCE: &str = APP_STRINGS[2].0;
const WATERING_MODE_RESOURCE: &str = APP_STRINGS[3].0;
const WINTERIZATION_REMINDER_RESOURCE: &str = APP_STRINGS[4].0;
const WINTERIZATION_WEATHER_NOTIFICATION_RESOURCE: &str = APP_STRINGS[5].0;
const WINTERIZATION_SEASON_NOTIFICATION_RESOURCE: &str = APP_STRINGS[6].0;

/// Sprinkler time slot
/// Defines one half-open schedule or hold-off interval.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
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
    open_observed_at_ticks: Option<u64>,
}

struct ZoneRuntime {
    configuration: SprinklerZoneV1,
    memory: SprinklerZoneMemoryV1,
    water_events: Vec<SprinklerWaterEventV1>,
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
}

struct ControllerState {
    weather_endpoint: LibertasEndpoint,
    reminder_recipients: Vec<LibertasUser>,
    watering_mode: SprinklerWateringModeV1,
    winterization_reminder: Option<SprinklerWinterizationReminderMemoryV1>,
    site_location: Option<SprinklerWeatherLocationV1>,
    hub_location_server_up: bool,
    hub_location_subscription_ready: bool,
    site_location_retry_timer: u32,
    weather: SprinklerWeatherSnapshotV1,
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
    action: Option<ControllerAction>,
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
    weather: &SprinklerWeatherSnapshotV1,
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
    weather: &SprinklerWeatherSnapshotV1,
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

fn system_key(weather_endpoint: LibertasEndpoint) -> [NotificationArgument<'static>; 1] {
    [NotificationArgument::Object(weather_endpoint)]
}

fn valid_site_location(location: SprinklerWeatherLocationV1) -> bool {
    location.latitude_degrees.is_finite()
        && (-90.0..=90.0).contains(&location.latitude_degrees)
        && location.longitude_degrees.is_finite()
        && (-180.0..=180.0).contains(&location.longitude_degrees)
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

fn prune_water_events(
    memory: &mut SprinklerZoneMemoryV1,
    water_events: &mut Vec<SprinklerWaterEventV1>,
    zone: &SprinklerZoneV1,
    now: LibertasDateTime,
) {
    let cutoff = now.saturating_sub(RECENT_WATER_WINDOW_SECONDS);
    let capacity = root_zone_capacity_millimeters(zone);
    let crop_coefficient = plant_profile(zone.plant_type).crop_coefficient;
    sort_water_events(water_events);

    let mut retained = Vec::with_capacity(water_events.len());
    for event in water_events.drain(..) {
        if event.ends_at().is_some_and(|ends_at| ends_at <= cutoff) {
            memory.baseline_deficit_millimeters = apply_deficit_delta(
                memory.baseline_deficit_millimeters,
                event_delta_millimeters(&event, crop_coefficient),
                capacity,
            );
            memory.balance_baseline_at = memory
                .balance_baseline_at
                .max(event.ends_at().unwrap_or(cutoff));
        } else {
            retained.push(event);
        }
    }
    if retained.len() > MAX_WATER_EVENTS {
        let remove_count = retained.len() - MAX_WATER_EVENTS;
        for event in retained.drain(..remove_count) {
            memory.baseline_deficit_millimeters = apply_deficit_delta(
                memory.baseline_deficit_millimeters,
                event_delta_millimeters(&event, crop_coefficient),
                capacity,
            );
            memory.balance_baseline_at = memory
                .balance_baseline_at
                .max(event.ends_at().unwrap_or(cutoff));
        }
    }
    *water_events = retained;
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
        if event.starts_at() > through {
            break;
        }
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
        deficit = apply_deficit_delta(
            deficit,
            event_delta_millimeters(event, crop_coefficient),
            capacity,
        );
        if matches!(event, SprinklerWaterEventV1::WeatherV1 { .. }) {
            demand_covered_through = demand_covered_through.max(event.ends_at().unwrap_or(through));
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

fn synchronize_history(
    memory: &SprinklerZoneMemoryV1,
    water_events: &mut Vec<SprinklerWaterEventV1>,
    history: Option<&SprinklerWeatherHistoryV1>,
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

fn add_irrigation_event(
    zone: &mut ZoneRuntime,
    starts_at: LibertasDateTime,
    duration_seconds: u32,
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
        prune_water_events(
            &mut zone.memory,
            &mut zone.water_events,
            &zone.configuration,
            now,
        );
        return;
    }
    zone.water_events.push(SprinklerWaterEventV1::IrrigationV1 {
        starts_at,
        duration_seconds,
        watering_percent,
        applied_water_millimeters,
    });
    prune_water_events(
        &mut zone.memory,
        &mut zone.water_events,
        &zone.configuration,
        now,
    );
}

fn begin_expected_irrigation(
    zone: &mut ZoneRuntime,
    starts_at: LibertasDateTime,
    duration_seconds: u32,
) -> bool {
    let applied_water_millimeters =
        nominal_delivery_millimeters_per_hour(zone.configuration.sprinkler_head_type)
            * duration_seconds as f32
            / 3_600.0;
    let candidate = SprinklerWaterEventV1::IrrigationV1 {
        starts_at,
        duration_seconds,
        watering_percent: zone.memory.watering_percent,
        applied_water_millimeters,
    };
    let Some(index) = water_event_index(&candidate) else {
        return false;
    };
    if duration_seconds == 0
        || !valid_nonnegative(applied_water_millimeters)
        || zone.expected_irrigation.is_some()
        || zone
            .water_events
            .iter()
            .any(|event| water_event_index(event) == Some(index))
    {
        return false;
    }
    zone.water_events.push(candidate);
    prune_water_events(
        &mut zone.memory,
        &mut zone.water_events,
        &zone.configuration,
        starts_at,
    );
    zone.expected_irrigation = Some(ExpectedIrrigation {
        starts_at,
        open_observed_at_ticks: None,
    });
    true
}

fn discard_expected_irrigation(zone: &mut ZoneRuntime) -> bool {
    let Some(expected) = zone.expected_irrigation.take() else {
        return false;
    };
    let previous_len = zone.water_events.len();
    zone.water_events.retain(|event| {
        !matches!(
            event,
            SprinklerWaterEventV1::IrrigationV1 { starts_at, .. }
                if *starts_at == expected.starts_at
        )
    });
    previous_len != zone.water_events.len()
}

fn reconcile_expected_irrigation(zone: &mut ZoneRuntime, now_ticks: u64) -> bool {
    let Some(expected) = zone.expected_irrigation.take() else {
        return false;
    };
    let actual_duration_seconds = expected
        .open_observed_at_ticks
        .map(|opened_at| now_ticks.saturating_sub(opened_at) / MICROSECONDS_PER_SECOND)
        .and_then(|duration| u32::try_from(duration).ok())
        .unwrap_or_default();
    let Some(index) = zone.water_events.iter().position(|event| {
        matches!(
            event,
            SprinklerWaterEventV1::IrrigationV1 { starts_at, .. }
                if *starts_at == expected.starts_at
        )
    }) else {
        return false;
    };
    if actual_duration_seconds == 0 {
        zone.water_events.remove(index);
        return true;
    }
    let SprinklerWaterEventV1::IrrigationV1 {
        duration_seconds,
        applied_water_millimeters,
        ..
    } = &mut zone.water_events[index]
    else {
        return false;
    };
    *duration_seconds = actual_duration_seconds;
    *applied_water_millimeters =
        nominal_delivery_millimeters_per_hour(zone.configuration.sprinkler_head_type)
            * actual_duration_seconds as f32
            / 3_600.0;
    true
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
    weather: &'a SprinklerWeatherSnapshotV1,
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
    weather: &SprinklerWeatherSnapshotV1,
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
    weather: &SprinklerWeatherSnapshotV1,
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
    for (zone_index, zone) in state.zones.iter_mut().enumerate() {
        if zone.pending_command.is_some_and(|pending| {
            now_ticks.saturating_sub(pending.sent_at_ticks)
                >= u64::from(VALVE_COMMAND_TIMEOUT_SECONDS).saturating_mul(MICROSECONDS_PER_SECOND)
        }) {
            zone.pending_command = None;
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
        if watering_mode == SprinklerWateringModeV1::Winterization {
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
            .map(|(zone_index, _)| ControllerAction::Close { zone_index })
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
    let change = {
        let mut state = shared.borrow_mut();
        let Some(zone) = state.zones.get_mut(zone_index) else {
            return false;
        };
        let previous_memory = zone.memory.clone();
        let previous_events = zone.water_events.clone();
        zone.pending_command = None;
        discard_expected_irrigation(zone)
            .then(|| zone_persistence_change(zone, previous_memory, previous_events))
    };
    if let Some(change) = change {
        change.submit();
        true
    } else {
        false
    }
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
        let previous_memory = zone.memory.clone();
        let previous_events = zone.water_events.clone();
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
        (
            zone.configuration.valve,
            zone_persistence_change(zone, previous_memory, previous_events),
        )
    };
    let (valve, expected_change) = prepared;
    // Submit the expected delivered amount before issuing the timed device open.
    expected_change.submit();
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

fn execute_close(shared: &Rc<RefCell<ControllerState>>, zone_index: usize) {
    let (valve, sent_at_ticks) = {
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
        (zone.configuration.valve, sent_at_ticks)
    };
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
            shared.borrow_mut().zones[zone_index].pending_command = None;
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
        ControllerAction::Close { zone_index } => execute_close(shared, zone_index),
    }
}

fn apply_evaluation_outcome(
    shared: &Rc<RefCell<ControllerState>>,
    outcome: EvaluationOutcome,
) -> Option<ControllerAction> {
    for (valve, memory) in outcome.zone_memories_to_persist {
        persist_zone_memory(valve, &memory);
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
) -> bool {
    if !zone.valve_is_open {
        return false;
    }
    // A timed automatic open was already persisted at command issue. Its one
    // record is amended when the observed valve close supplies actual duration.
    if zone.valve_opened_automatically && zone.expected_irrigation.is_some() {
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
    add_irrigation_event(zone, starts_at, duration_seconds, event_now);
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
        let mut changed = Vec::new();
        for (zone_index, zone) in state.zones.iter_mut().enumerate() {
            let previous_memory = zone.memory.clone();
            let previous_events = zone.water_events.clone();
            if account_open_zone(zone, now_ticks, now_utc) {
                changed.push((
                    zone_index,
                    zone.configuration.valve,
                    previous_memory,
                    zone.memory.clone(),
                    previous_events,
                    zone.water_events.clone(),
                ));
            }
        }
        changed
    };
    for (_, valve, previous_memory, memory, previous_events, water_events) in &changed {
        persist_zone_runtime_change(
            *valve,
            previous_memory,
            memory,
            previous_events,
            water_events,
        );
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
    let (change_to_persist, closed_transition) = {
        let mut state = shared.borrow_mut();
        let Some(zone) = state.zones.get_mut(zone_index) else {
            return;
        };
        let was_known = zone.valve_state_known;
        let was_open = zone.valve_is_open;
        let previous_memory = zone.memory.clone();
        let previous_events = zone.water_events.clone();
        zone.valve_state_known = true;
        zone.valve_last_report_ticks = Some(now_ticks);
        let mut irrigation_changed = false;
        if was_open && !is_open {
            irrigation_changed =
                if zone.valve_opened_automatically && zone.expected_irrigation.is_some() {
                    reconcile_expected_irrigation(zone, now_ticks)
                } else {
                    account_open_zone(zone, now_ticks, now_utc)
                };
        } else if !is_open && zone.expected_irrigation.is_some() {
            // A confirmed closed report without an observed open means the
            // timed open delivered no water; remove its optimistic record.
            irrigation_changed = reconcile_expected_irrigation(zone, now_ticks);
        }
        if was_open != is_open {
            let opened_automatically = is_open && zone.expected_irrigation.is_some();
            zone.valve_is_open = is_open;
            zone.pending_command = None;
            if is_open {
                zone.valve_opened_automatically = opened_automatically;
                if opened_automatically {
                    if let Some(expected) = &mut zone.expected_irrigation {
                        expected.open_observed_at_ticks = Some(now_ticks);
                    }
                    zone.accounted_at_ticks = None;
                    zone.accounted_at_utc = None;
                } else {
                    // A manual open is never closed by this application. Track
                    // it from observation and persist minute checkpoints.
                    zone.accounted_at_ticks = Some(now_ticks);
                    zone.accounted_at_utc = now_utc;
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
            !is_open && (!was_known || was_open),
        )
    };
    if let Some(change) = change_to_persist {
        change.submit();
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
            let (weather_endpoint, changed) = {
                let state = shared.borrow();
                (
                    state.weather_endpoint,
                    state.site_location != Some(location),
                )
            };
            if changed {
                persist_site_location(weather_endpoint, location);
                shared.borrow_mut().site_location = Some(location);
                evaluate_and_publish(shared);
            }
            arm_site_location_retry(shared, HUB_LOCATION_MAX_REPORT_INTERVAL_SECONDS);
            return LibertasEndpointHandlerResult::Handled;
        }
        return LibertasEndpointHandlerResult::InvalidMessage;
    }
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
            let zone = &mut state.zones[context.zone_index];
            if zone.memory.watering_percent != watering_percent {
                let previous_memory = zone.memory.clone();
                let previous_events = zone.water_events.clone();
                account_open_zone(zone, now_ticks, now_utc);
                zone.memory.watering_percent = watering_percent;
                persist_runtime = Some((
                    zone.configuration.valve,
                    previous_memory,
                    zone.memory.clone(),
                    previous_events,
                    zone.water_events.clone(),
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
    if let Some((valve, previous_memory, memory, previous_events, water_events)) = persist_runtime {
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
    history: &mut Option<SprinklerWeatherHistoryV1>,
    retrieved_at: LibertasDateTime,
    valid_until: LibertasDateTime,
    periods: Vec<SprinklerWeatherHistoryPeriodV1>,
) {
    let history = history.get_or_insert(SprinklerWeatherHistoryV1 {
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
    snapshot: &mut SprinklerWeatherSnapshotV1,
    change: SprinklerWeatherChangeV1,
) {
    match change {
        SprinklerWeatherChangeV1::HistoryPeriodsUpsertV1 {
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
        SprinklerWeatherChangeV1::HistoryReplaceV1 { history } => {
            snapshot.history = Some(history);
        }
        SprinklerWeatherChangeV1::ForecastReplaceV1 { forecast } => {
            snapshot.forecast = Some(forecast);
        }
    }
}

fn synchronize_weather_memories(shared: &Rc<RefCell<ControllerState>>) {
    let now = utc_seconds().unwrap_or_default();
    let changed = {
        let mut state = shared.borrow_mut();
        let history = state.weather.history.clone();
        let mut changed = Vec::new();
        for zone in &mut state.zones {
            let previous_memory = zone.memory.clone();
            let previous_events = zone.water_events.clone();
            if synchronize_history(&zone.memory, &mut zone.water_events, history.as_ref()) {
                prune_water_events(
                    &mut zone.memory,
                    &mut zone.water_events,
                    &zone.configuration,
                    now,
                );
                changed.push((
                    zone.configuration.valve,
                    previous_memory,
                    zone.memory.clone(),
                    previous_events,
                    zone.water_events.clone(),
                ));
            }
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
    let mut state = shared.borrow_mut();
    let Some(cursor) = state.weather_cursor else {
        return false;
    };
    if !report.can_apply_after(cursor) {
        return false;
    }
    for change in report.changes {
        apply_weather_change(&mut state.weather, change);
    }
    state.weather_cursor = Some(report.through_cursor);
    true
}

fn accept_weather_recovery(
    shared: &Rc<RefCell<ControllerState>>,
    recovery: SprinklerWeatherRecoveryV1,
) -> bool {
    match recovery {
        SprinklerWeatherRecoveryV1::ReplayedV1 { report } => accept_weather_report(shared, report),
        SprinklerWeatherRecoveryV1::ResetV1 {
            cursor, snapshot, ..
        } => {
            let previous = shared.borrow().weather_cursor;
            let accepted = previous.is_none_or(|previous| {
                (cursor.epoch_timestamp == previous.epoch_timestamp
                    && cursor.sequence >= previous.sequence)
                    || cursor.is_server_reset_after(previous)
            });
            if accepted {
                let mut state = shared.borrow_mut();
                state.weather_cursor = Some(cursor);
                state.weather = snapshot;
            }
            accepted
        }
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
                let accepted = accept_weather_recovery(shared, recovery);
                if accepted {
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
            let accepted = accept_weather_report(shared, report);
            if accepted {
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
    let now = utc_seconds().unwrap_or_default();
    let watering_mode = load_watering_mode(weather_endpoint);
    let winterization_reminder = load_winterization_reminder(weather_endpoint);
    let site_location = load_site_location(weather_endpoint);
    let mut runtime_zones = Vec::with_capacity(zones.len());
    for configuration in zones {
        let mut memory = load_zone_memory(configuration.valve, now);
        let mut water_events = load_water_events(configuration.valve, &memory);
        let restored_memory = memory.clone();
        let restored_events = water_events.clone();
        prune_water_events(&mut memory, &mut water_events, &configuration, now);
        persist_zone_runtime_change(
            configuration.valve,
            &restored_memory,
            &memory,
            &restored_events,
            &water_events,
        );
        let active_state = initial_active_state(now, &memory, &configuration);
        runtime_zones.push(ZoneRuntime {
            configuration,
            memory,
            water_events,
            active_state,
            valve_state_known: false,
            valve_is_open: false,
            valve_opened_automatically: false,
            valve_fault_bitmap: 0,
            valve_last_report_ticks: None,
            accounted_at_ticks: None,
            accounted_at_utc: None,
            pending_command: None,
            expected_irrigation: None,
        });
    }
    let shared = Rc::new(RefCell::new(ControllerState {
        weather_endpoint,
        reminder_recipients,
        watering_mode,
        winterization_reminder,
        site_location,
        hub_location_server_up: true,
        hub_location_subscription_ready: false,
        site_location_retry_timer: 0,
        weather: SprinklerWeatherSnapshotV1 {
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

    fn history() -> SprinklerWeatherHistoryV1 {
        SprinklerWeatherHistoryV1 {
            retrieved_at: NOW,
            valid_until: NOW + 7_200,
            periods: Vec::new(),
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
        }
    }

    fn controller_state() -> ControllerState {
        ControllerState {
            weather_endpoint: 1,
            reminder_recipients: vec![2, 3],
            watering_mode: SprinklerWateringModeV1::Active,
            winterization_reminder: None,
            site_location: None,
            hub_location_server_up: true,
            hub_location_subscription_ready: false,
            site_location_retry_timer: 0,
            weather: SprinklerWeatherSnapshotV1 {
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
    fn persistent_memory_round_trips_and_discriminants_are_stable() {
        let configuration = zone();
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
        let weather = SprinklerWeatherSnapshotV1 {
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
        let stale_weather = SprinklerWeatherSnapshotV1 {
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

        let forecast_weather = SprinklerWeatherSnapshotV1 {
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
            &SprinklerWeatherSnapshotV1 {
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
        let weather = SprinklerWeatherSnapshotV1 {
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
                &SprinklerWeatherSnapshotV1 {
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
        let weather = SprinklerWeatherSnapshotV1 {
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
        let weather = SprinklerWeatherSnapshotV1 {
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
        let weather = SprinklerWeatherSnapshotV1 {
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
        let weather = SprinklerWeatherSnapshotV1 {
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
        let weather = SprinklerWeatherSnapshotV1 {
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
        let weather = SprinklerWeatherSnapshotV1 {
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
            &SprinklerWeatherSnapshotV1 {
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
        let weather = SprinklerWeatherSnapshotV1 {
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
        let weather = SprinklerWeatherSnapshotV1 {
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

        let weather = SprinklerWeatherSnapshotV1 {
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
    fn observed_valve_time_becomes_recent_irrigation() {
        let mut runtime = runtime(memory());
        add_irrigation_event(&mut runtime, NOW - 600, 600, NOW);
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
        add_irrigation_event(&mut runtime, NOW - 120, 60, NOW - 60);
        add_irrigation_event(&mut runtime, NOW - 60, 60, NOW);
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
    fn timed_open_is_reserved_then_amended_to_observed_duration() {
        let mut zone_runtime = runtime(memory());
        assert!(begin_expected_irrigation(&mut zone_runtime, NOW, 900));
        assert_eq!(zone_runtime.water_events.len(), 1);
        let SprinklerWaterEventV1::IrrigationV1 {
            duration_seconds,
            applied_water_millimeters,
            ..
        } = zone_runtime.water_events[0]
        else {
            panic!("expected irrigation event");
        };
        assert_eq!(duration_seconds, 900);
        assert!((applied_water_millimeters - 3.0).abs() < 0.001);
        let expected_events = zone_runtime.water_events.clone();
        let reservation_delta = water_event_delta(&[], &expected_events);
        assert_eq!(reservation_delta.upserts.len(), 1);
        assert!(reservation_delta.removals.is_empty());

        zone_runtime.valve_is_open = true;
        zone_runtime.valve_opened_automatically = true;
        zone_runtime
            .expected_irrigation
            .as_mut()
            .unwrap()
            .open_observed_at_ticks = Some(10 * MICROSECONDS_PER_SECOND);
        assert!(!account_open_zone(
            &mut zone_runtime,
            310 * MICROSECONDS_PER_SECOND,
            Some(NOW + 300)
        ));
        assert_eq!(zone_runtime.water_events.len(), 1);

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
        assert_eq!(zone_runtime.expected_irrigation, None);
        let amendment_delta = water_event_delta(&expected_events, &zone_runtime.water_events);
        assert_eq!(amendment_delta.upserts.len(), 1);
        assert!(amendment_delta.removals.is_empty());

        let mut unopened = runtime(memory());
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
            Some(NOW + 30)
        ));
        assert!(account_open_zone(
            &mut runtime,
            80 * MICROSECONDS_PER_SECOND,
            Some(NOW + 70)
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
        add_irrigation_event(&mut runtime, NOW - 120, 60, NOW - 60);
        runtime.memory.watering_percent = 80;
        add_irrigation_event(&mut runtime, NOW - 60, 60, NOW);

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
        let history = SprinklerWeatherHistoryV1 {
            retrieved_at: NOW,
            valid_until: NOW + 3_600,
            periods: vec![
                SprinklerWeatherHistoryPeriodV1 {
                    starts_at: NOW - 7_200,
                    duration_seconds: 3_600,
                    precipitation_millimeters: 10.0,
                    reference_evapotranspiration_millimeters: 0.0,
                },
                SprinklerWeatherHistoryPeriodV1 {
                    starts_at: NOW - 3_600,
                    duration_seconds: 3_600,
                    precipitation_millimeters: 1.0,
                    reference_evapotranspiration_millimeters: 2.0,
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
    fn weather_reset_requires_newer_epoch_and_backward_sequence() {
        let previous = SprinklerWeatherCursorV1 {
            epoch_timestamp: NOW,
            sequence: 10,
        };
        let mut state = controller_state();
        state.weather_cursor = Some(previous);
        let shared = Rc::new(RefCell::new(state));
        let reset = |cursor| SprinklerWeatherRecoveryV1::ResetV1 {
            reason: libertas_weather::SprinklerWeatherResetReasonV1::ServerCursorReset,
            cursor,
            snapshot: SprinklerWeatherSnapshotV1 {
                history: Some(history()),
                current: Some(current()),
                forecast: None,
            },
        };

        assert!(!accept_weather_recovery(
            &shared,
            reset(SprinklerWeatherCursorV1 {
                epoch_timestamp: NOW,
                sequence: 3,
            })
        ));
        assert_eq!(shared.borrow().weather_cursor, Some(previous));
        let accepted = SprinklerWeatherCursorV1 {
            epoch_timestamp: NOW + 1,
            sequence: 3,
        };
        assert!(accept_weather_recovery(&shared, reset(accepted)));
        assert_eq!(shared.borrow().weather_cursor, Some(accepted));
    }

    #[test]
    fn peer_alive_refresh_path_does_not_touch_weather_data_or_cursor() {
        let cursor = SprinklerWeatherCursorV1 {
            epoch_timestamp: NOW,
            sequence: 10,
        };
        let weather = SprinklerWeatherSnapshotV1 {
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
        let weather = SprinklerWeatherSnapshotV1 {
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
            &SprinklerWeatherSnapshotV1 {
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
            &SprinklerWeatherSnapshotV1 {
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
            &SprinklerWeatherSnapshotV1 {
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
        let weather = SprinklerWeatherSnapshotV1 {
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
