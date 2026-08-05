//! Libertas Sprinkler
//! Calculates and executes weather-aware watering schedules for sprinkler zones
//! controlled by Matter Valve Configuration and Control devices.
//!
//! Configuration describes durable physical facts only: the shared sprinkler
//! weather endpoint and, for each zone, its valve, soil, planting, measured
//! application rate, and schedule endpoint. During runtime a user can express
//! one normalized preference from less water through more water and maintain
//! hold-off periods that constrain schedule calculation.
//!
//! Each zone persists its preference, hold-offs, water-balance baseline, and a
//! seven-day ledger of recent precipitation, evapotranspiration, and actual
//! valve-open irrigation. Valve subscriptions count both automatic and manual
//! watering, so a restart or manual run does not cause the controller to water
//! the same deficit twice.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::{boxed::Box, rc::Rc, vec::Vec};
use core::{any::Any, cell::RefCell};

use libertas::{
    LibertasDateTime, LibertasDevice, LibertasEndpoint, LibertasEndpointHandlerResult,
    LibertasEndpointMessage, LibertasEndpointStandardStatus, LogLevel, NotificationArgument,
    OP_ENDPOINT_DATA, OP_ENDPOINT_PEER_DOWN, OP_ENDPOINT_PEER_TIMEOUT, OP_ENDPOINT_REQ,
    OP_ENDPOINT_RSP, OP_ENDPOINT_SUB_REQ, libertas_data_read, libertas_data_write,
    libertas_endpoint_report, libertas_endpoint_response, libertas_endpoint_subscribe_request,
    libertas_get_sys_ticks, libertas_get_utc_time, libertas_log, libertas_register_device_listener,
    libertas_register_endpoint_status_listener, libertas_timer_new_interval,
    libertas_timer_update_interval,
};
use libertas_macros::{
    LibertasAvroDecode, LibertasAvroEncode, LibertasExport, libertas_data_schema, libertas_export,
    libertas_string_resources,
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
    SprinklerWeatherHistoryV1, SprinklerWeatherIncrementalReportV1, SprinklerWeatherProtocolV1,
    SprinklerWeatherRecoveryErrorV1, SprinklerWeatherRecoveryV1, SprinklerWeatherSectionV1,
    SprinklerWeatherSnapshotV1, SprinklerWeatherTimeRangeV1,
};

const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
const RECENT_WATER_WINDOW_SECONDS: u64 = 7 * 24 * 60 * 60;
const WEATHER_RETRY_SECONDS: u32 = 60;
const VALVE_COMMAND_TIMEOUT_SECONDS: u32 = 60;
const VALVE_ACCOUNTING_INTERVAL_SECONDS: u32 = 60;
const SCHEDULE_EVALUATION_INTERVAL_SECONDS: u32 = 60;
const VALVE_SUBSCRIPTION_MAX_INTERVAL_SECONDS: u16 = 30;
const VALVE_SUBSCRIPTION_STALE_SECONDS: u32 = (VALVE_SUBSCRIPTION_MAX_INTERVAL_SECONDS as u32) * 3;
const MAX_HOLD_OFFS: usize = 64;
const MAX_RECENT_WATER_EVENTS: usize = 512;
const MIN_WATERING_DURATION_SECONDS: u32 = 60;
const MAX_WATERING_DURATION_SECONDS: u32 = 2 * 60 * 60;
const SAFE_MINIMUM_TEMPERATURE_CELSIUS: f32 = 3.0;
const SAFE_MAXIMUM_WIND_METERS_PER_SECOND: f32 = 10.0;
const SAFE_MAXIMUM_GUST_METERS_PER_SECOND: f32 = 15.0;
const HIGH_RAIN_PROBABILITY_PERCENT: u8 = 50;
const FORECAST_LOOKAHEAD_SECONDS: u64 = 12 * 60 * 60;
const ZONE_DATA_RESOURCE: &str = "SPRINKLER_ZONE_MEMORY_V1";

/// Sprinkler database names
/// Stable resource identifiers and their user-facing descriptions.
pub static APP_STRINGS: [(&str, &str); 1] = [(
    ZONE_DATA_RESOURCE,
    "Persisted sprinkler water balance and preferences for %1$s.",
)];

/// Sprinkler time slot V1
/// Defines one half-open schedule or hold-off interval.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub struct SprinklerTimeSlotV1 {
    /// Start time
    /// The inclusive start date and time in seconds since the Unix epoch.
    pub starts_at: LibertasDateTime,
    /// Duration
    /// The interval length in seconds. A valid slot always has a nonzero
    /// duration and an end time representable by `LibertasDateTime`.
    #[libertas_time_interval]
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

/// Sprinkler soil type V1
/// Selects a curated plant-available water capacity. The controller combines
/// this profile with the planting's effective root depth; users do not enter a
/// raw field-capacity value.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerSoilTypeV1 {
    /// Sand
    /// Fast-draining soil with low plant-available water storage.
    Sand,
    /// Loamy sand
    /// Predominantly sandy soil with modest water storage.
    LoamySand,
    /// Sandy loam
    /// Moderately draining soil with medium water storage.
    SandyLoam,
    /// Loam
    /// Balanced soil with high plant-available water storage.
    Loam,
    /// Clay loam
    /// Fine soil with high water storage and slower infiltration.
    ClayLoam,
    /// Clay
    /// Dense fine soil with high storage but reduced plant availability.
    Clay,
    /// Silty clay
    /// Fine silty soil with high plant-available water storage.
    SiltyClay,
}

/// Sprinkler planting type V1
/// Selects the effective root depth and weather crop coefficient used by the
/// zone's water-balance calculation.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerPlantingTypeV1 {
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

/// Sprinkler schedule condition V1
/// Explains the current calculated schedule and why watering may be deferred.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerScheduleConditionV1 {
    /// Fresh weather unavailable
    /// The controller has no fresh current conditions or has not established a
    /// contiguous weather stream, and will not treat cached weather as proof
    /// that watering is safe.
    FreshWeatherUnavailable,
    /// Water not needed
    /// The estimated root-zone deficit is below the watering threshold.
    WaterNotNeeded,
    /// Forecast rain
    /// Significant high-probability rain is expected before watering is needed.
    ForecastRain,
    /// Waiting for safe weather
    /// Rain, freezing temperature, or excessive wind currently prevents
    /// watering; the displayed future slot is forecast-derived.
    WaitingForSafeWeather,
    /// Held off
    /// A user hold-off moved the calculated watering slot.
    HeldOff,
    /// Scheduled
    /// A watering slot has been calculated and is waiting to begin.
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
}

/// Sprinkler zone schedule V1
/// Exposes the complete current calculation for one zone, including its next
/// watering slot, user constraints, recent water, and valve state.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerZoneScheduleV1 {
    /// Calculated at
    /// The date and time represented by this schedule calculation.
    pub calculated_at: LibertasDateTime,
    /// Condition
    /// The current watering decision or constraint.
    pub condition: SprinklerScheduleConditionV1,
    /// Next watering
    /// The next calculated valve-open slot. It is absent when water is not
    /// needed or no safe slot can be calculated.
    pub next_watering: Option<SprinklerTimeSlotV1>,
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
    /// More or less water
    /// The user's normalized runtime preference from -1.0 for less water,
    /// through 0.0 for the calculated amount, to 1.0 for more water. It maps
    /// linearly to 50% through 150% of the calculated replenishment.
    #[libertas_number(min = -1, max = 1, step = 0.05)]
    pub more_or_less_water_normalized: f32,
    /// Hold-off periods
    /// Active sorted, non-overlapping intervals that the next watering slot
    /// must avoid. Expired persisted constraints are omitted.
    /// ----
    /// Hold-off period
    /// A half-open interval during which this zone cannot water.
    #[libertas_size(max = 64)]
    pub hold_off_periods: Vec<SprinklerTimeSlotV1>,
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

/// Sprinkler zone protocol V1
/// Reads or subscribes to the calculated schedule and updates the zone's one
/// normalized watering preference or hold-off constraints.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum SprinklerZoneProtocolV1 {
    /// Get schedule V1
    /// Requests the current calculated schedule. The same message may establish
    /// a subscription because the endpoint operation is outside this value.
    #[libertas_request]
    #[libertas_subscription_request]
    #[libertas_next_response(ScheduleV1)]
    GetScheduleV1,
    /// Set more or less water V1
    /// Updates the one normalized user tuning parameter for this zone.
    #[libertas_request]
    #[libertas_next_response(ScheduleV1)]
    SetMoreOrLessWaterV1 {
        /// More or less water
        /// A finite normalized number from -1.0 through 1.0. Zero selects the
        /// calculated amount; negative values apply less and positive values
        /// apply more.
        #[libertas_number(min = -1, max = 1, step = 0.05)]
        more_or_less_water_normalized: f32,
    },
    /// Replace hold-off periods V1
    /// Replaces all scheduling constraints for this zone. Overlapping or
    /// touching periods are normalized into sorted merged intervals.
    #[libertas_request]
    #[libertas_next_response(ScheduleV1)]
    ReplaceHoldOffPeriodsV1 {
        /// Hold-off periods
        /// The complete replacement list, limited to 64 valid intervals.
        /// ----
        /// Hold-off period
        /// A half-open interval during which the schedule cannot water.
        #[libertas_size(max = 64)]
        hold_off_periods: Vec<SprinklerTimeSlotV1>,
    },
    /// Schedule V1
    /// Returns the current calculation or reports a later schedule change.
    #[libertas_response]
    #[libertas_subscription_data]
    ScheduleV1 {
        /// Schedule
        /// The complete current schedule and recent-water status for the zone.
        schedule: SprinklerZoneScheduleV1,
    },
}

/// Recent sprinkler water event V1
/// Stores one independently correctable weather period or one observed
/// irrigation interval in the persisted seven-day water ledger.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum SprinklerRecentWaterEventV1 {
    /// Weather period V1
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
    /// Irrigation interval V1
    /// Records water inferred from actual Matter Valve open time, including
    /// manual openings.
    IrrigationV1 {
        /// Start time
        /// The inclusive start of the accounted valve-open interval.
        starts_at: LibertasDateTime,
        /// Duration
        /// The observed valve-open interval length in seconds.
        #[libertas_time_interval]
        duration_seconds: u32,
        /// Applied water
        /// Water depth calculated from observed open time and the configured
        /// zone application rate, in millimeters.
        #[libertas_number(min = 0)]
        applied_water_millimeters: f32,
    },
}

impl SprinklerRecentWaterEventV1 {
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

/// Sprinkler zone memory V1
/// Persists the complete restart-safe user preference and water-balance input
/// history for one configured valve.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerZoneMemoryV1 {
    /// More or less water
    /// The normalized runtime preference from -1.0 through 1.0.
    #[libertas_number(min = -1, max = 1, step = 0.05)]
    pub more_or_less_water_normalized: f32,
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
    /// Recent water events
    /// Weather and observed irrigation inputs retained for correction,
    /// explanation, and restart recovery.
    /// ----
    /// Recent water event
    /// One completed weather period or accounted valve-open interval.
    #[libertas_size(max = 512)]
    pub recent_water_events: Vec<SprinklerRecentWaterEventV1>,
}

/// Sprinkler persistent data V1
/// Defines every value written by the sprinkler application. Each zone is
/// stored independently under its Matter Valve object key.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum SprinklerDataV1 {
    /// Zone memory V1
    /// Stores one zone's runtime preference, constraints, and water ledger.
    ZoneMemoryV1 {
        /// Zone memory
        /// The complete restart-safe state for the configured valve.
        memory: SprinklerZoneMemoryV1,
    },
}

/// Sprinkler zone V1
/// Configures the physical facts needed to calculate and execute watering for
/// one area. User preferences and hold-offs are runtime data, not configuration.
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
    /// Soil type
    /// The curated plant-available water profile for this zone.
    #[libertas_default(Loam)]
    pub soil_type: SprinklerSoilTypeV1,
    /// Planting type
    /// The curated root-depth and crop-coefficient profile for this zone.
    #[libertas_default(Lawn)]
    pub planting_type: SprinklerPlantingTypeV1,
    /// Application rate
    /// Measured water depth applied by this zone per hour of valve-open time, in
    /// millimeters per hour. A catch-can measurement is preferred over a
    /// sprinkler-head category.
    #[libertas_number(min = 0.1, max = 100, step = 0.1)]
    pub application_rate_millimeters_per_hour: f32,
    /// Schedule endpoint
    /// Exposes the calculated schedule and accepts the normalized water
    /// preference and hold-off constraints.
    #[libertas_endpoint_schema(SprinklerZoneProtocolV1)]
    #[libertas_endpoint_server]
    #[libertas_endpoint_base_objects("^.valve")]
    pub schedule_endpoint: LibertasEndpoint,
}

#[derive(Clone, Copy)]
struct PlantProfile {
    root_depth_meters: f32,
    crop_coefficient: f32,
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

struct ZoneRuntime {
    configuration: SprinklerZoneV1,
    memory: SprinklerZoneMemoryV1,
    schedule: SprinklerZoneScheduleV1,
    subscribers: Vec<u32>,
    valve_state_known: bool,
    valve_is_open: bool,
    valve_opened_automatically: bool,
    valve_fault_bitmap: u16,
    valve_last_report_ticks: Option<u64>,
    accounted_at_ticks: Option<u64>,
    accounted_at_utc: Option<LibertasDateTime>,
    pending_command: Option<PendingValveCommand>,
}

struct ControllerState {
    weather_endpoint: LibertasEndpoint,
    weather: SprinklerWeatherSnapshotV1,
    weather_cursor: Option<SprinklerWeatherCursorV1>,
    weather_stream_ready: bool,
    weather_maximum_wait_seconds: u32,
    weather_retry_timer: u32,
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

struct EvaluationOutcome {
    changed_zones: Vec<usize>,
    action: Option<ControllerAction>,
}

fn utc_seconds() -> Option<LibertasDateTime> {
    libertas_get_utc_time()
        .map(|microseconds| microseconds / MICROSECONDS_PER_SECOND)
        .filter(|seconds| *seconds > 0)
}

fn absolute_interval_ticks(now_ticks: u64, interval_seconds: u32) -> u64 {
    now_ticks.saturating_add(u64::from(interval_seconds).saturating_mul(MICROSECONDS_PER_SECOND))
}

fn soil_available_water_millimeters_per_meter(soil: SprinklerSoilTypeV1) -> f32 {
    match soil {
        SprinklerSoilTypeV1::Sand => 70.0,
        SprinklerSoilTypeV1::LoamySand => 90.0,
        SprinklerSoilTypeV1::SandyLoam => 120.0,
        SprinklerSoilTypeV1::Loam => 160.0,
        SprinklerSoilTypeV1::ClayLoam => 180.0,
        SprinklerSoilTypeV1::Clay => 170.0,
        SprinklerSoilTypeV1::SiltyClay => 190.0,
    }
}

fn plant_profile(planting: SprinklerPlantingTypeV1) -> PlantProfile {
    match planting {
        SprinklerPlantingTypeV1::Lawn => PlantProfile {
            root_depth_meters: 0.20,
            crop_coefficient: 0.80,
        },
        SprinklerPlantingTypeV1::Flowers => PlantProfile {
            root_depth_meters: 0.30,
            crop_coefficient: 0.70,
        },
        SprinklerPlantingTypeV1::Vegetables => PlantProfile {
            root_depth_meters: 0.45,
            crop_coefficient: 0.90,
        },
        SprinklerPlantingTypeV1::FruitTrees => PlantProfile {
            root_depth_meters: 0.80,
            crop_coefficient: 0.75,
        },
        SprinklerPlantingTypeV1::Citrus => PlantProfile {
            root_depth_meters: 0.75,
            crop_coefficient: 0.80,
        },
        SprinklerPlantingTypeV1::TreesAndBushes => PlantProfile {
            root_depth_meters: 1.0,
            crop_coefficient: 0.60,
        },
        SprinklerPlantingTypeV1::Xeriscape => PlantProfile {
            root_depth_meters: 0.50,
            crop_coefficient: 0.30,
        },
    }
}

fn root_zone_capacity_millimeters(zone: &SprinklerZoneV1) -> f32 {
    soil_available_water_millimeters_per_meter(zone.soil_type)
        * plant_profile(zone.planting_type).root_depth_meters
}

fn valid_normalized_adjustment(value: f32) -> bool {
    value.is_finite() && (-1.0..=1.0).contains(&value)
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

fn valid_recent_event(event: &SprinklerRecentWaterEventV1) -> bool {
    if event.duration_seconds() == 0 || event.ends_at().is_none() {
        return false;
    }
    match event {
        SprinklerRecentWaterEventV1::WeatherV1 {
            precipitation_millimeters,
            reference_evapotranspiration_millimeters,
            ..
        } => {
            valid_nonnegative(*precipitation_millimeters)
                && valid_nonnegative(*reference_evapotranspiration_millimeters)
        }
        SprinklerRecentWaterEventV1::IrrigationV1 {
            applied_water_millimeters,
            ..
        } => valid_nonnegative(*applied_water_millimeters),
    }
}

fn valid_memory(memory: &SprinklerZoneMemoryV1) -> bool {
    valid_normalized_adjustment(memory.more_or_less_water_normalized)
        && valid_nonnegative(memory.baseline_deficit_millimeters)
        && memory.hold_off_periods.len() <= MAX_HOLD_OFFS
        && memory.hold_off_periods.iter().copied().all(valid_slot)
        && memory.recent_water_events.len() <= MAX_RECENT_WATER_EVENTS
        && memory.recent_water_events.iter().all(valid_recent_event)
}

fn default_memory(now: LibertasDateTime) -> SprinklerZoneMemoryV1 {
    SprinklerZoneMemoryV1 {
        more_or_less_water_normalized: 0.0,
        hold_off_periods: Vec::new(),
        balance_baseline_at: now.saturating_sub(RECENT_WATER_WINDOW_SECONDS),
        baseline_deficit_millimeters: 0.0,
        recent_water_events: Vec::new(),
    }
}

fn zone_key(valve: LibertasDevice) -> [NotificationArgument<'static>; 1] {
    [NotificationArgument::Object(valve)]
}

fn persist_zone_memory(valve: LibertasDevice, memory: &SprinklerZoneMemoryV1) {
    libertas_data_write(
        ZONE_DATA_RESOURCE,
        &zone_key(valve),
        &SprinklerDataV1::ZoneMemoryV1 {
            memory: memory.clone(),
        },
    );
}

fn load_zone_memory(valve: LibertasDevice, now: LibertasDateTime) -> SprinklerZoneMemoryV1 {
    match libertas_data_read(ZONE_DATA_RESOURCE, &zone_key(valve)) {
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

fn event_delta_millimeters(event: &SprinklerRecentWaterEventV1, crop_coefficient: f32) -> f32 {
    match event {
        SprinklerRecentWaterEventV1::WeatherV1 {
            precipitation_millimeters,
            reference_evapotranspiration_millimeters,
            ..
        } => {
            reference_evapotranspiration_millimeters * crop_coefficient - precipitation_millimeters
        }
        SprinklerRecentWaterEventV1::IrrigationV1 {
            applied_water_millimeters,
            ..
        } => -*applied_water_millimeters,
    }
}

fn apply_deficit_delta(deficit: f32, delta: f32, capacity: f32) -> f32 {
    (deficit + delta).clamp(0.0, capacity)
}

fn sort_recent_events(events: &mut [SprinklerRecentWaterEventV1]) {
    events.sort_by(|left, right| {
        left.starts_at().cmp(&right.starts_at()).then_with(|| {
            let left_kind = matches!(left, SprinklerRecentWaterEventV1::IrrigationV1 { .. });
            let right_kind = matches!(right, SprinklerRecentWaterEventV1::IrrigationV1 { .. });
            left_kind.cmp(&right_kind)
        })
    });
}

fn prune_recent_events(
    memory: &mut SprinklerZoneMemoryV1,
    zone: &SprinklerZoneV1,
    now: LibertasDateTime,
) {
    let cutoff = now.saturating_sub(RECENT_WATER_WINDOW_SECONDS);
    let capacity = root_zone_capacity_millimeters(zone);
    let crop_coefficient = plant_profile(zone.planting_type).crop_coefficient;
    sort_recent_events(&mut memory.recent_water_events);

    let mut retained = Vec::with_capacity(memory.recent_water_events.len());
    for event in memory.recent_water_events.drain(..) {
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
    if retained.len() > MAX_RECENT_WATER_EVENTS {
        let remove_count = retained.len() - MAX_RECENT_WATER_EVENTS;
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
    memory.recent_water_events = retained;
}

fn estimated_deficit_millimeters(zone: &SprinklerZoneV1, memory: &SprinklerZoneMemoryV1) -> f32 {
    let capacity = root_zone_capacity_millimeters(zone);
    let crop_coefficient = plant_profile(zone.planting_type).crop_coefficient;
    memory
        .recent_water_events
        .iter()
        .fold(memory.baseline_deficit_millimeters, |deficit, event| {
            apply_deficit_delta(
                deficit,
                event_delta_millimeters(event, crop_coefficient),
                capacity,
            )
        })
}

fn recent_water_totals(memory: &SprinklerZoneMemoryV1) -> (f32, f32) {
    memory
        .recent_water_events
        .iter()
        .fold(
            (0.0, 0.0),
            |(precipitation, irrigation), event| match event {
                SprinklerRecentWaterEventV1::WeatherV1 {
                    precipitation_millimeters,
                    ..
                } => (precipitation + precipitation_millimeters, irrigation),
                SprinklerRecentWaterEventV1::IrrigationV1 {
                    applied_water_millimeters,
                    ..
                } => (precipitation, irrigation + applied_water_millimeters),
            },
        )
}

fn synchronize_history(
    memory: &mut SprinklerZoneMemoryV1,
    history: Option<&SprinklerWeatherHistoryV1>,
) -> bool {
    let Some(history) = history else {
        return false;
    };
    let before = memory.recent_water_events.clone();
    memory
        .recent_water_events
        .retain(|event| matches!(event, SprinklerRecentWaterEventV1::IrrigationV1 { .. }));
    memory.recent_water_events.extend(
        history
            .periods
            .iter()
            .filter(|period| {
                period
                    .starts_at
                    .saturating_add(u64::from(period.duration_seconds))
                    > memory.balance_baseline_at
            })
            .map(|period| SprinklerRecentWaterEventV1::WeatherV1 {
                starts_at: period.starts_at,
                duration_seconds: period.duration_seconds,
                precipitation_millimeters: period.precipitation_millimeters,
                reference_evapotranspiration_millimeters: period
                    .reference_evapotranspiration_millimeters,
            }),
    );
    sort_recent_events(&mut memory.recent_water_events);
    memory.recent_water_events != before
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
    let applied_water_millimeters = zone.configuration.application_rate_millimeters_per_hour
        * duration_seconds as f32
        / 3_600.0;
    if !valid_nonnegative(applied_water_millimeters) {
        return;
    }
    if let Some(SprinklerRecentWaterEventV1::IrrigationV1 {
        starts_at: previous_starts_at,
        duration_seconds: previous_duration_seconds,
        applied_water_millimeters: previous_applied_water_millimeters,
    }) = zone
        .memory
        .recent_water_events
        .iter_mut()
        .rev()
        .find(|event| matches!(event, SprinklerRecentWaterEventV1::IrrigationV1 { .. }))
        && previous_starts_at.checked_add(u64::from(*previous_duration_seconds)) == Some(starts_at)
        && let Some(merged_duration_seconds) =
            previous_duration_seconds.checked_add(duration_seconds)
    {
        *previous_duration_seconds = merged_duration_seconds;
        *previous_applied_water_millimeters += applied_water_millimeters;
        prune_recent_events(&mut zone.memory, &zone.configuration, now);
        return;
    }
    zone.memory
        .recent_water_events
        .push(SprinklerRecentWaterEventV1::IrrigationV1 {
            starts_at,
            duration_seconds,
            applied_water_millimeters,
        });
    prune_recent_events(&mut zone.memory, &zone.configuration, now);
}

fn current_is_safe(current: &SprinklerCurrentWeatherV1, now: LibertasDateTime) -> bool {
    current.is_fresh_at(now)
        && current.temperature_celsius.is_finite()
        && current.temperature_celsius > SAFE_MINIMUM_TEMPERATURE_CELSIUS
        && current.precipitation_millimeters == 0.0
        && current.wind_speed_meters_per_second.is_finite()
        && current.wind_speed_meters_per_second <= SAFE_MAXIMUM_WIND_METERS_PER_SECOND
        && current.wind_gust_meters_per_second.is_finite()
        && current.wind_gust_meters_per_second <= SAFE_MAXIMUM_GUST_METERS_PER_SECOND
}

fn forecast_period_is_safe(period: &SprinklerWeatherForecastPeriodV1) -> bool {
    period.temperature_celsius.is_finite()
        && period.temperature_celsius > SAFE_MINIMUM_TEMPERATURE_CELSIUS
        && period.precipitation_probability_percent < HIGH_RAIN_PROBABILITY_PERCENT
        && period.expected_precipitation_millimeters <= 0.1
        && period.wind_speed_meters_per_second.is_finite()
        && period.wind_speed_meters_per_second <= SAFE_MAXIMUM_WIND_METERS_PER_SECOND
        && period.wind_gust_meters_per_second.is_finite()
        && period.wind_gust_meters_per_second <= SAFE_MAXIMUM_GUST_METERS_PER_SECOND
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

fn next_safe_forecast_start(
    forecast: Option<&SprinklerWeatherForecastV1>,
    not_before: LibertasDateTime,
) -> Option<LibertasDateTime> {
    forecast?
        .periods
        .iter()
        .find(|period| period.starts_at >= not_before && forecast_period_is_safe(period))
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
    let seconds = (water_millimeters / zone.application_rate_millimeters_per_hour * 3_600.0).ceil();
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (seconds as u32).clamp(MIN_WATERING_DURATION_SECONDS, MAX_WATERING_DURATION_SECONDS)
}

fn calculate_schedule(
    zone: &ZoneRuntime,
    weather: &SprinklerWeatherSnapshotV1,
    weather_stream_ready: bool,
    now: LibertasDateTime,
) -> SprinklerZoneScheduleV1 {
    let deficit = estimated_deficit_millimeters(&zone.configuration, &zone.memory);
    let capacity = root_zone_capacity_millimeters(&zone.configuration);
    let (recent_precipitation, recent_irrigation) = recent_water_totals(&zone.memory);
    let active_hold_offs: Vec<_> = zone
        .memory
        .hold_off_periods
        .iter()
        .copied()
        .filter(|hold_off| hold_off.ends_at().is_some_and(|ends_at| ends_at > now))
        .collect();
    let base = |condition, next_watering, planned_water_millimeters| SprinklerZoneScheduleV1 {
        calculated_at: now,
        condition,
        next_watering,
        planned_water_millimeters,
        estimated_deficit_millimeters: deficit,
        recent_precipitation_millimeters: recent_precipitation,
        recent_irrigation_millimeters: recent_irrigation,
        more_or_less_water_normalized: zone.memory.more_or_less_water_normalized,
        hold_off_periods: active_hold_offs.clone(),
        valve_is_open: zone.valve_is_open,
        valve_state_known: zone.valve_state_known,
        valve_fault_bitmap: zone.valve_fault_bitmap,
    };

    if zone.valve_fault_bitmap != 0 {
        return base(SprinklerScheduleConditionV1::ValveFault, None, 0.0);
    }
    if !zone.valve_state_known {
        return base(
            SprinklerScheduleConditionV1::ValveStateUnavailable,
            None,
            0.0,
        );
    }
    if zone.valve_is_open {
        return base(SprinklerScheduleConditionV1::ValveOpen, None, 0.0);
    }
    if zone.pending_command.is_some() {
        return base(SprinklerScheduleConditionV1::ValveCommandPending, None, 0.0);
    }
    if weather.history.is_none() {
        return base(
            SprinklerScheduleConditionV1::FreshWeatherUnavailable,
            None,
            0.0,
        );
    }

    let trigger_deficit = capacity * 0.50;
    if deficit < trigger_deficit {
        return base(SprinklerScheduleConditionV1::WaterNotNeeded, None, 0.0);
    }
    let replenishment = (deficit - capacity * 0.20).max(0.0);
    let multiplier = 1.0 + zone.memory.more_or_less_water_normalized * 0.5;
    let planned_water = (replenishment * multiplier).clamp(0.0, capacity);
    let duration = watering_duration_seconds(&zone.configuration, planned_water);
    if duration == 0 {
        return base(SprinklerScheduleConditionV1::WaterNotNeeded, None, 0.0);
    }

    let current_safe = weather_stream_ready
        && weather
            .current
            .as_ref()
            .is_some_and(|current| current_is_safe(current, now));
    let current_fresh = weather_stream_ready
        && weather
            .current
            .as_ref()
            .is_some_and(|current| current.is_fresh_at(now));
    let mut condition = SprinklerScheduleConditionV1::Scheduled;
    let mut candidate = now;

    if let Some(rainy_until) = forecast_rain_delay(weather.forecast.as_ref(), now, planned_water) {
        candidate =
            next_safe_forecast_start(weather.forecast.as_ref(), rainy_until).unwrap_or(rainy_until);
        condition = SprinklerScheduleConditionV1::ForecastRain;
    } else if !current_safe {
        let Some(safe_start) = next_safe_forecast_start(weather.forecast.as_ref(), now) else {
            return base(
                if current_fresh {
                    SprinklerScheduleConditionV1::WaitingForSafeWeather
                } else {
                    SprinklerScheduleConditionV1::FreshWeatherUnavailable
                },
                None,
                planned_water,
            );
        };
        candidate = safe_start;
        condition = if current_fresh {
            SprinklerScheduleConditionV1::WaitingForSafeWeather
        } else {
            SprinklerScheduleConditionV1::FreshWeatherUnavailable
        };
    }

    let (candidate, held_off) = shift_after_hold_offs(candidate, duration, &active_hold_offs);
    if held_off {
        condition = SprinklerScheduleConditionV1::HeldOff;
    }
    base(
        condition,
        Some(SprinklerTimeSlotV1 {
            starts_at: candidate,
            duration_seconds: duration,
        }),
        planned_water,
    )
}

fn weather_permits_immediate_watering(
    weather: &SprinklerWeatherSnapshotV1,
    weather_stream_ready: bool,
    now: LibertasDateTime,
) -> bool {
    weather_stream_ready
        && weather.history.is_some()
        && weather
            .current
            .as_ref()
            .is_some_and(|current| current_is_safe(current, now))
}

fn evaluate_controller(shared: &Rc<RefCell<ControllerState>>) -> EvaluationOutcome {
    let now = utc_seconds().unwrap_or_default();
    let now_ticks = libertas_get_sys_ticks();
    let mut state = shared.borrow_mut();
    for zone in &mut state.zones {
        if zone.pending_command.is_some_and(|pending| {
            now_ticks.saturating_sub(pending.sent_at_ticks)
                >= u64::from(VALVE_COMMAND_TIMEOUT_SECONDS).saturating_mul(MICROSECONDS_PER_SECOND)
        }) {
            zone.pending_command = None;
        }
    }
    let weather = state.weather.clone();
    let weather_stream_ready = state.weather_stream_ready;
    let any_open = state.zones.iter().any(|zone| zone.valve_is_open);
    let any_pending = state
        .zones
        .iter()
        .any(|zone| zone.pending_command.is_some());
    let weather_safe = weather_permits_immediate_watering(&weather, weather_stream_ready, now);
    let mut changed_zones = Vec::new();

    for (zone_index, zone) in state.zones.iter_mut().enumerate() {
        let schedule = calculate_schedule(zone, &weather, weather_stream_ready, now);
        if schedule != zone.schedule {
            zone.schedule = schedule;
            changed_zones.push(zone_index);
        }
    }

    let action = if any_open && !weather_safe {
        state
            .zones
            .iter()
            .enumerate()
            .find(|(_, zone)| {
                zone.valve_is_open
                    && zone.valve_opened_automatically
                    && zone.pending_command.is_none()
            })
            .map(|(zone_index, _)| ControllerAction::Close { zone_index })
    } else if !any_open && !any_pending && weather_safe {
        state
            .zones
            .iter()
            .enumerate()
            .find_map(|(zone_index, zone)| {
                zone.schedule.next_watering.and_then(|slot| {
                    (slot.starts_at <= now).then_some(ControllerAction::Open {
                        zone_index,
                        duration_seconds: slot.duration_seconds,
                    })
                })
            })
    } else {
        None
    };

    EvaluationOutcome {
        changed_zones,
        action,
    }
}

fn publish_zone_schedule(shared: &Rc<RefCell<ControllerState>>, zone_index: usize) {
    let (endpoint, peers, message) = {
        let state = shared.borrow();
        let Some(zone) = state.zones.get(zone_index) else {
            return;
        };
        (
            zone.configuration.schedule_endpoint,
            zone.subscribers.clone(),
            SprinklerZoneProtocolV1::ScheduleV1 {
                schedule: zone.schedule.clone(),
            },
        )
    };
    for peer in peers {
        libertas_endpoint_report(endpoint, &message, Some(peer));
    }
}

fn execute_controller_action(shared: &Rc<RefCell<ControllerState>>, action: ControllerAction) {
    let (zone_index, kind, valve, duration_seconds) = match action {
        ControllerAction::Open {
            zone_index,
            duration_seconds,
        } => (
            zone_index,
            ValveCommandKind::Open,
            shared.borrow().zones[zone_index].configuration.valve,
            Some(duration_seconds),
        ),
        ControllerAction::Close { zone_index } => (
            zone_index,
            ValveCommandKind::Close,
            shared.borrow().zones[zone_index].configuration.valve,
            None,
        ),
    };
    {
        let mut state = shared.borrow_mut();
        if state.zones[zone_index].pending_command.is_some() {
            return;
        }
        state.zones[zone_index].pending_command = Some(PendingValveCommand {
            kind,
            transaction_id: None,
            sent_at_ticks: libertas_get_sys_ticks(),
        });
    }

    let result = match duration_seconds {
        Some(duration_seconds) => MatterDevice::new(valve).invoke(&Open {
            OpenDuration: Some(Nullable::some(duration_seconds)),
            TargetLevel: None,
        }),
        None => MatterDevice::new(valve).invoke(&Close {}),
    };
    match result {
        Ok(transaction_id) => {
            shared.borrow_mut().zones[zone_index].pending_command = Some(PendingValveCommand {
                kind,
                transaction_id: Some(transaction_id),
                sent_at_ticks: libertas_get_sys_ticks(),
            });
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

fn dispatch_evaluation(shared: &Rc<RefCell<ControllerState>>, outcome: EvaluationOutcome) {
    for zone_index in outcome.changed_zones {
        publish_zone_schedule(shared, zone_index);
    }
    if let Some(action) = outcome.action {
        execute_controller_action(shared, action);
        let follow_up = evaluate_controller(shared);
        for zone_index in follow_up.changed_zones {
            publish_zone_schedule(shared, zone_index);
        }
    }
}

fn evaluate_and_publish(shared: &Rc<RefCell<ControllerState>>) {
    let outcome = evaluate_controller(shared);
    dispatch_evaluation(shared, outcome);
}

fn account_open_zone(
    zone: &mut ZoneRuntime,
    now_ticks: u64,
    now_utc: Option<LibertasDateTime>,
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
            if account_open_zone(zone, now_ticks, now_utc) {
                changed.push((zone_index, zone.configuration.valve, zone.memory.clone()));
            }
        }
        changed
    };
    for (_, valve, memory) in &changed {
        persist_zone_memory(*valve, memory);
    }
    if !changed.is_empty() {
        evaluate_and_publish(shared);
    }
}

fn set_valve_open_state(shared: &Rc<RefCell<ControllerState>>, zone_index: usize, is_open: bool) {
    let now_ticks = libertas_get_sys_ticks();
    let now_utc = utc_seconds();
    let memory_to_persist = {
        let mut state = shared.borrow_mut();
        let Some(zone) = state.zones.get_mut(zone_index) else {
            return;
        };
        zone.valve_state_known = true;
        zone.valve_last_report_ticks = Some(now_ticks);
        let mut changed_memory = false;
        if zone.valve_is_open && !is_open {
            changed_memory = account_open_zone(zone, now_ticks, now_utc);
        }
        if zone.valve_is_open != is_open {
            let opened_automatically = is_open
                && zone
                    .pending_command
                    .is_some_and(|pending| pending.kind == ValveCommandKind::Open);
            zone.valve_is_open = is_open;
            zone.pending_command = None;
            if is_open {
                zone.valve_opened_automatically = opened_automatically;
                zone.accounted_at_ticks = Some(now_ticks);
                zone.accounted_at_utc = now_utc;
            } else {
                zone.valve_opened_automatically = false;
                zone.accounted_at_ticks = None;
                zone.accounted_at_utc = None;
            }
        }
        changed_memory.then(|| (zone.configuration.valve, zone.memory.clone()))
    };
    if let Some((valve, memory)) = memory_to_persist {
        persist_zone_memory(valve, &memory);
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
        shared.borrow_mut().zones[zone_index].pending_command = None;
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

fn add_or_replace_subscriber(zone: &mut ZoneRuntime, peer: u32) {
    if !zone.subscribers.contains(&peer) {
        zone.subscribers.push(peer);
    }
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
        if let Some(zone) = context
            .shared
            .borrow_mut()
            .zones
            .get_mut(context.zone_index)
        {
            zone.subscribers.retain(|subscriber| *subscriber != peer);
        }
        return LibertasEndpointHandlerResult::Handled;
    }
    if opcode == OP_ENDPOINT_PEER_TIMEOUT {
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
    match message {
        SprinklerZoneProtocolV1::GetScheduleV1 => {}
        SprinklerZoneProtocolV1::SetMoreOrLessWaterV1 {
            more_or_less_water_normalized,
        } => {
            if is_subscription || !valid_normalized_adjustment(more_or_less_water_normalized) {
                return LibertasEndpointHandlerResult::Status(
                    LibertasEndpointStandardStatus::InvalidArgument,
                );
            }
            let mut state = context.shared.borrow_mut();
            let zone = &mut state.zones[context.zone_index];
            zone.memory.more_or_less_water_normalized = more_or_less_water_normalized;
            persist = Some((zone.configuration.valve, zone.memory.clone()));
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
        }
        SprinklerZoneProtocolV1::ScheduleV1 { .. } => {
            return LibertasEndpointHandlerResult::InvalidMessage;
        }
    }
    if let Some((valve, memory)) = persist {
        persist_zone_memory(valve, &memory);
    }

    let outcome = evaluate_controller(&context.shared);
    let schedule = context.shared.borrow().zones[context.zone_index]
        .schedule
        .clone();
    libertas_endpoint_response(
        endpoint,
        &SprinklerZoneProtocolV1::ScheduleV1 { schedule },
        transaction_id,
        peer,
    );
    if is_subscription {
        add_or_replace_subscriber(
            &mut context.shared.borrow_mut().zones[context.zone_index],
            peer,
        );
    }
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
            if synchronize_history(&mut zone.memory, history.as_ref()) {
                prune_recent_events(&mut zone.memory, &zone.configuration, now);
                changed.push((zone.configuration.valve, zone.memory.clone()));
            }
        }
        changed
    };
    for (valve, memory) in changed {
        persist_zone_memory(valve, &memory);
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
    let timer = shared.borrow().weather_retry_timer;
    if timer != 0 {
        libertas_timer_update_interval(
            timer,
            absolute_interval_ticks(libertas_get_sys_ticks(), delay_seconds.max(1)),
        );
    }
}

fn subscribe_weather(shared: &Rc<RefCell<ControllerState>>) {
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
    if opcode == OP_ENDPOINT_PEER_DOWN || opcode == OP_ENDPOINT_PEER_TIMEOUT {
        shared.borrow_mut().weather_stream_ready = false;
        arm_weather_retry(shared, WEATHER_RETRY_SECONDS);
        evaluate_and_publish(shared);
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

fn initial_schedule(
    now: LibertasDateTime,
    memory: &SprinklerZoneMemoryV1,
) -> SprinklerZoneScheduleV1 {
    SprinklerZoneScheduleV1 {
        calculated_at: now,
        condition: SprinklerScheduleConditionV1::FreshWeatherUnavailable,
        next_watering: None,
        planned_water_millimeters: 0.0,
        estimated_deficit_millimeters: memory.baseline_deficit_millimeters,
        recent_precipitation_millimeters: 0.0,
        recent_irrigation_millimeters: 0.0,
        more_or_less_water_normalized: memory.more_or_less_water_normalized,
        hold_off_periods: memory.hold_off_periods.clone(),
        valve_is_open: false,
        valve_state_known: false,
        valve_fault_bitmap: 0,
    }
}

/// Libertas sprinkler
/// Runs a weather-aware multi-zone sprinkler controller. The weather endpoint
/// supplies the tailored sprinkler history, current conditions, and forecast
/// shared by all zones. Each zone exposes its calculated schedule and persists
/// its own recent-water state.
#[libertas_data_schema(SprinklerDataV1)]
#[libertas_string_resources(APP_STRINGS)]
#[libertas_export]
pub fn libertas_sprinkler(
    /*
     * Sprinkler weather
     * The client endpoint for `SprinklerWeatherProtocolV1`. The application
     * subscribes at startup and will not automatically water without fresh
     * safe current conditions.
     */
    #[libertas_endpoint_schema(SprinklerWeatherProtocolV1)]
    #[libertas_foreign_type("libertas-weather::SprinklerWeatherProtocolV1")]
    weather_endpoint: LibertasEndpoint,
    /*
     * Sprinkler zones
     * One or more independently scheduled Matter Valve zones.
     * ----
     * Sprinkler zone
     * The physical and endpoint configuration for one watered area.
     * #[libertas_size(min=1, max=32)]
     */
    zones: Vec<SprinklerZoneV1>,
) {
    let now = utc_seconds().unwrap_or_default();
    let mut runtime_zones = Vec::with_capacity(zones.len());
    let mut valves = Vec::new();
    let mut schedule_endpoints = Vec::new();
    for configuration in zones {
        if valves.contains(&configuration.valve)
            || schedule_endpoints.contains(&configuration.schedule_endpoint)
        {
            libertas_log(
                LogLevel::Error,
                "Skipping duplicate sprinkler valve or schedule endpoint",
            );
            continue;
        }
        if !configuration
            .application_rate_millimeters_per_hour
            .is_finite()
            || configuration.application_rate_millimeters_per_hour <= 0.0
        {
            libertas_log(
                LogLevel::Error,
                "Skipping sprinkler zone with an invalid application rate",
            );
            continue;
        }
        valves.push(configuration.valve);
        schedule_endpoints.push(configuration.schedule_endpoint);
        let mut memory = load_zone_memory(configuration.valve, now);
        let restored_memory = memory.clone();
        prune_recent_events(&mut memory, &configuration, now);
        if memory != restored_memory {
            persist_zone_memory(configuration.valve, &memory);
        }
        let schedule = initial_schedule(now, &memory);
        runtime_zones.push(ZoneRuntime {
            configuration,
            memory,
            schedule,
            subscribers: Vec::new(),
            valve_state_known: false,
            valve_is_open: false,
            valve_opened_automatically: false,
            valve_fault_bitmap: 0,
            valve_last_report_ticks: None,
            accounted_at_ticks: None,
            accounted_at_utc: None,
            pending_command: None,
        });
    }
    if runtime_zones.is_empty() {
        libertas_log(LogLevel::Error, "No valid sprinkler zones were configured");
        return;
    }

    let shared = Rc::new(RefCell::new(ControllerState {
        weather_endpoint,
        weather: SprinklerWeatherSnapshotV1 {
            history: None,
            current: None,
            forecast: None,
        },
        weather_cursor: None,
        weather_stream_ready: false,
        weather_maximum_wait_seconds: WEATHER_RETRY_SECONDS,
        weather_retry_timer: 0,
        zones: runtime_zones,
    }));

    let zone_count = shared.borrow().zones.len();
    for zone_index in 0..zone_count {
        let (valve, endpoint) = {
            let state = shared.borrow();
            let zone = &state.zones[zone_index];
            (
                zone.configuration.valve,
                zone.configuration.schedule_endpoint,
            )
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

    let weather_timer =
        libertas_timer_new_interval(0, weather_retry_timer, Box::new(Rc::clone(&shared)));
    shared.borrow_mut().weather_retry_timer = weather_timer;
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
    subscribe_weather(&shared);
    evaluate_and_publish(&shared);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use libertas_matter::{InlineByteBuffer, decode_command, encode_command};

    const NOW: LibertasDateTime = 1_800_000_000;

    fn zone() -> SprinklerZoneV1 {
        SprinklerZoneV1 {
            valve: 7,
            soil_type: SprinklerSoilTypeV1::Loam,
            planting_type: SprinklerPlantingTypeV1::Lawn,
            application_rate_millimeters_per_hour: 12.0,
            schedule_endpoint: 17,
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

    fn runtime(memory: SprinklerZoneMemoryV1) -> ZoneRuntime {
        let schedule = initial_schedule(NOW, &memory);
        ZoneRuntime {
            configuration: zone(),
            memory,
            schedule,
            subscribers: Vec::new(),
            valve_state_known: true,
            valve_is_open: false,
            valve_opened_automatically: false,
            valve_fault_bitmap: 0,
            valve_last_report_ticks: Some(0),
            accounted_at_ticks: None,
            accounted_at_utc: None,
            pending_command: None,
        }
    }

    #[test]
    fn public_protocol_round_trips_through_avro() {
        let schedule = initial_schedule(NOW, &memory());
        let values = [
            SprinklerZoneProtocolV1::GetScheduleV1,
            SprinklerZoneProtocolV1::SetMoreOrLessWaterV1 {
                more_or_less_water_normalized: -0.25,
            },
            SprinklerZoneProtocolV1::ReplaceHoldOffPeriodsV1 {
                hold_off_periods: vec![SprinklerTimeSlotV1 {
                    starts_at: NOW,
                    duration_seconds: 600,
                }],
            },
            SprinklerZoneProtocolV1::ScheduleV1 { schedule },
        ];
        for value in values {
            let encoded = value.to_avro();
            assert_eq!(SprinklerZoneProtocolV1::from_avro(&encoded), Ok(value));
        }
    }

    #[test]
    fn persistent_memory_round_trips_and_discriminants_are_stable() {
        let value = SprinklerDataV1::ZoneMemoryV1 { memory: memory() };
        let encoded = value.to_avro();
        assert_eq!(encoded.first(), Some(&0));
        assert_eq!(SprinklerDataV1::from_avro(&encoded), Ok(value));

        let protocols = [
            SprinklerZoneProtocolV1::GetScheduleV1,
            SprinklerZoneProtocolV1::SetMoreOrLessWaterV1 {
                more_or_less_water_normalized: 0.0,
            },
            SprinklerZoneProtocolV1::ReplaceHoldOffPeriodsV1 {
                hold_off_periods: Vec::new(),
            },
            SprinklerZoneProtocolV1::ScheduleV1 {
                schedule: initial_schedule(NOW, &memory()),
            },
        ];
        for (index, protocol) in protocols.iter().enumerate() {
            assert_eq!(protocol.to_avro().first(), Some(&((index as u8) * 2)));
        }

        let soils = [
            SprinklerSoilTypeV1::Sand,
            SprinklerSoilTypeV1::LoamySand,
            SprinklerSoilTypeV1::SandyLoam,
            SprinklerSoilTypeV1::Loam,
            SprinklerSoilTypeV1::ClayLoam,
            SprinklerSoilTypeV1::Clay,
            SprinklerSoilTypeV1::SiltyClay,
        ];
        for (index, soil) in soils.iter().enumerate() {
            assert_eq!(soil.to_avro().first(), Some(&((index as u8) * 2)));
        }

        let plantings = [
            SprinklerPlantingTypeV1::Lawn,
            SprinklerPlantingTypeV1::Flowers,
            SprinklerPlantingTypeV1::Vegetables,
            SprinklerPlantingTypeV1::FruitTrees,
            SprinklerPlantingTypeV1::Citrus,
            SprinklerPlantingTypeV1::TreesAndBushes,
            SprinklerPlantingTypeV1::Xeriscape,
        ];
        for (index, planting) in plantings.iter().enumerate() {
            assert_eq!(planting.to_avro().first(), Some(&((index as u8) * 2)));
        }

        let conditions = [
            SprinklerScheduleConditionV1::FreshWeatherUnavailable,
            SprinklerScheduleConditionV1::WaterNotNeeded,
            SprinklerScheduleConditionV1::ForecastRain,
            SprinklerScheduleConditionV1::WaitingForSafeWeather,
            SprinklerScheduleConditionV1::HeldOff,
            SprinklerScheduleConditionV1::Scheduled,
            SprinklerScheduleConditionV1::ValveCommandPending,
            SprinklerScheduleConditionV1::ValveStateUnavailable,
            SprinklerScheduleConditionV1::ValveOpen,
            SprinklerScheduleConditionV1::ValveFault,
        ];
        for (index, condition) in conditions.iter().enumerate() {
            assert_eq!(condition.to_avro().first(), Some(&((index as u8) * 2)));
        }

        let recent_events = [
            SprinklerRecentWaterEventV1::WeatherV1 {
                starts_at: NOW,
                duration_seconds: 3_600,
                precipitation_millimeters: 1.0,
                reference_evapotranspiration_millimeters: 2.0,
            },
            SprinklerRecentWaterEventV1::IrrigationV1 {
                starts_at: NOW,
                duration_seconds: 600,
                applied_water_millimeters: 2.0,
            },
        ];
        for (index, event) in recent_events.iter().enumerate() {
            assert_eq!(event.to_avro().first(), Some(&((index as u8) * 2)));
        }
    }

    #[test]
    fn normalized_adjustment_has_one_bounded_scale() {
        assert!(valid_normalized_adjustment(-1.0));
        assert!(valid_normalized_adjustment(0.0));
        assert!(valid_normalized_adjustment(1.0));
        assert!(!valid_normalized_adjustment(-1.01));
        assert!(!valid_normalized_adjustment(1.01));
        assert!(!valid_normalized_adjustment(f32::NAN));

        let weather = SprinklerWeatherSnapshotV1 {
            history: Some(history()),
            current: Some(current()),
            forecast: None,
        };
        let planned_water = |adjustment| {
            let mut memory = memory();
            memory.baseline_deficit_millimeters = 20.0;
            memory.more_or_less_water_normalized = adjustment;
            calculate_schedule(&runtime(memory), &weather, true, NOW).planned_water_millimeters
        };
        assert!((planned_water(-1.0) - planned_water(0.0) * 0.5).abs() < 0.001);
        assert!((planned_water(1.0) - planned_water(0.0) * 1.5).abs() < 0.001);
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
    fn water_balance_combines_weather_and_observed_irrigation() {
        let mut memory = memory();
        memory.recent_water_events = vec![
            SprinklerRecentWaterEventV1::WeatherV1 {
                starts_at: NOW - 3_600,
                duration_seconds: 3_600,
                precipitation_millimeters: 1.0,
                reference_evapotranspiration_millimeters: 6.0,
            },
            SprinklerRecentWaterEventV1::IrrigationV1 {
                starts_at: NOW - 1_800,
                duration_seconds: 900,
                applied_water_millimeters: 3.0,
            },
        ];
        let deficit = estimated_deficit_millimeters(&zone(), &memory);
        assert!((deficit - 0.8).abs() < 0.001);
    }

    #[test]
    fn observed_valve_time_becomes_recent_irrigation() {
        let mut runtime = runtime(memory());
        add_irrigation_event(&mut runtime, NOW - 600, 600, NOW);
        let SprinklerRecentWaterEventV1::IrrigationV1 {
            applied_water_millimeters,
            ..
        } = runtime.memory.recent_water_events[0]
        else {
            panic!("expected irrigation event");
        };
        assert!((applied_water_millimeters - 2.0).abs() < 0.001);
    }

    #[test]
    fn consecutive_valve_checkpoints_form_one_irrigation_event() {
        let mut runtime = runtime(memory());
        add_irrigation_event(&mut runtime, NOW - 120, 60, NOW - 60);
        add_irrigation_event(&mut runtime, NOW - 60, 60, NOW);
        assert_eq!(runtime.memory.recent_water_events.len(), 1);
        let SprinklerRecentWaterEventV1::IrrigationV1 {
            duration_seconds,
            applied_water_millimeters,
            ..
        } = runtime.memory.recent_water_events[0]
        else {
            panic!("expected irrigation event");
        };
        assert_eq!(duration_seconds, 120);
        assert!((applied_water_millimeters - 0.4).abs() < 0.001);
    }

    #[test]
    fn history_at_the_persisted_baseline_is_not_counted_again() {
        let mut memory = memory();
        memory.balance_baseline_at = NOW - 3_600;
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
        assert!(synchronize_history(&mut memory, Some(&history)));
        assert_eq!(memory.recent_water_events.len(), 1);
        assert_eq!(memory.recent_water_events[0].starts_at(), NOW - 3_600);
    }

    #[test]
    fn weather_reset_requires_newer_epoch_and_backward_sequence() {
        let previous = SprinklerWeatherCursorV1 {
            epoch_timestamp: NOW,
            sequence: 10,
        };
        let shared = Rc::new(RefCell::new(ControllerState {
            weather_endpoint: 1,
            weather: SprinklerWeatherSnapshotV1 {
                history: None,
                current: None,
                forecast: None,
            },
            weather_cursor: Some(previous),
            weather_stream_ready: true,
            weather_maximum_wait_seconds: WEATHER_RETRY_SECONDS,
            weather_retry_timer: 0,
            zones: Vec::new(),
        }));
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
    fn cursor_ahead_error_restarts_weather_recovery_without_a_cursor() {
        let mut state = ControllerState {
            weather_endpoint: 1,
            weather: SprinklerWeatherSnapshotV1 {
                history: None,
                current: None,
                forecast: None,
            },
            weather_cursor: Some(SprinklerWeatherCursorV1 {
                epoch_timestamp: NOW,
                sequence: 10,
            }),
            weather_stream_ready: true,
            weather_maximum_wait_seconds: WEATHER_RETRY_SECONDS,
            weather_retry_timer: 0,
            zones: Vec::new(),
        };
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
    fn fresh_current_weather_is_required_for_immediate_watering() {
        let weather = SprinklerWeatherSnapshotV1 {
            history: Some(history()),
            current: Some(current()),
            forecast: None,
        };
        assert!(weather_permits_immediate_watering(&weather, true, NOW));
        assert!(!weather_permits_immediate_watering(&weather, false, NOW));
        assert!(!weather_permits_immediate_watering(
            &SprinklerWeatherSnapshotV1 {
                current: Some(SprinklerCurrentWeatherV1 {
                    precipitation_millimeters: 0.1,
                    ..current()
                }),
                ..weather.clone()
            },
            true,
            NOW
        ));
        assert!(!weather_permits_immediate_watering(
            &weather,
            true,
            NOW + 1_800
        ));
    }

    #[test]
    fn unknown_valve_state_inhibits_an_automatic_schedule() {
        let mut memory = memory();
        memory.baseline_deficit_millimeters = 20.0;
        let mut zone = runtime(memory);
        zone.valve_state_known = false;
        let schedule = calculate_schedule(
            &zone,
            &SprinklerWeatherSnapshotV1 {
                history: Some(history()),
                current: Some(current()),
                forecast: None,
            },
            true,
            NOW,
        );
        assert_eq!(
            schedule.condition,
            SprinklerScheduleConditionV1::ValveStateUnavailable
        );
        assert_eq!(schedule.next_watering, None);
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
                    precipitation_probability_percent: 90,
                    expected_precipitation_millimeters: 20.0,
                    reference_evapotranspiration_millimeters: 0.2,
                    wind_speed_meters_per_second: 2.0,
                    wind_gust_meters_per_second: 4.0,
                }],
            }),
        };
        let schedule = calculate_schedule(&zone, &weather, true, NOW);
        assert_eq!(
            schedule.condition,
            SprinklerScheduleConditionV1::ForecastRain
        );
        assert!(schedule.next_watering.unwrap().starts_at >= NOW + 3_600);
    }

    #[test]
    fn truncated_persistent_data_is_rejected() {
        let encoded = SprinklerDataV1::ZoneMemoryV1 { memory: memory() }.to_avro();
        assert!(SprinklerDataV1::from_avro(&encoded[..encoded.len() - 1]).is_err());
    }
}
