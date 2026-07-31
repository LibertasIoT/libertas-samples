//! Libertas, Matter, weather, persistence, and worker integration.
//!
//! All callbacks execute on the single Libertas application thread. The only
//! owned worker is the machine-learning thread; it returns owned results
//! through a bounded channel and wakes this module's one registered callback.

use super::*;
use alloc::{format, rc::Rc};
use core::{any::Any, cell::RefCell, f32::consts::TAU};
use std::sync::mpsc::Receiver;

use libertas::{
    LibertasEndpointHandlerResult, LibertasEndpointMessage, LibertasEndpointStandardStatus,
    LibertasTransId, OP_ENDPOINT_DATA, OP_ENDPOINT_PEER_DOWN, OP_ENDPOINT_PEER_TIMEOUT,
    OP_ENDPOINT_REQ, OP_ENDPOINT_RSP, OP_ENDPOINT_SUB_REQ, libertas_data_remove,
    libertas_endpoint_report, libertas_endpoint_response, libertas_endpoint_subscribe_request,
    libertas_formatted_text, libertas_get_sys_ticks, libertas_get_utc_time,
    libertas_register_device_listener, libertas_register_endpoint_status_listener,
    libertas_register_shutdown_handler, libertas_register_wakeup_callback,
    libertas_timer_new_interval, libertas_timer_update_interval,
};
use libertas_matter::{
    InlineByteBuffer, MatterDevice, MatterDeviceSubscription, MatterResponse,
    MatterSubscriptionBatch, MatterSubscriptionCluster, decode_attribute_report,
    decode_write_response,
    definitions::{AirQuality, RelativeHumidityMeasurement, TemperatureMeasurement, Thermostat},
    frame::Operation,
};
use libertas_weather::{
    BUILDING_HVAC_AIR_QUALITY_HORIZON_SECONDS, BUILDING_HVAC_FORECAST_HORIZON_SECONDS,
    BUILDING_HVAC_HISTORY_WINDOW_SECONDS, BUILDING_HVAC_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
};

const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
const MATTER_SUBSCRIPTION_MAX_INTERVAL_SECONDS: u16 = 30;
const MATTER_READING_FRESHNESS_SECONDS: u64 = 90;
const EVALUATION_INTERVAL_SECONDS: u32 = 60;
const CONDITION_PERIOD_SECONDS: u64 = 15 * 60;
const WEATHER_RETRY_SECONDS: u32 = 60;
const ML_TRAINING_INTERVAL_SECONDS: u64 = 24 * 60 * 60;
const MAX_ML_PENDING_FEATURES: usize = 5;

const ROOM_CONTROL_RESOURCE: &str = "HVAC_ROOM_CONTROL";
const ROOM_STATISTICS_RESOURCE: &str = "HVAC_ROOM_STATISTICS";
const ROOM_LEARNING_RESOURCE: &str = "HVAC_ROOM_LEARNING";
const ROOM_SENSOR_STATE_RESOURCE: &str = "HVAC_ROOM_SENSOR_STATE";
const ROOM_URGENT_RESOURCE: &str = "HVAC_ROOM_URGENT_NOTIFICATION_STATE";
const LOCAL_OUTDOOR_TEMPERATURE_RESOURCE: &str = "HVAC_LOCAL_OUTDOOR_TEMPERATURE";
const LOCAL_OUTDOOR_HUMIDITY_RESOURCE: &str = "HVAC_LOCAL_OUTDOOR_HUMIDITY";
const LOCAL_OUTDOOR_AIR_QUALITY_RESOURCE: &str = "HVAC_LOCAL_OUTDOOR_AIR_QUALITY";
const WEATHER_HISTORY_RESOURCE: &str = "HVAC_WEATHER_HISTORY";
const WEATHER_CURRENT_RESOURCE: &str = "HVAC_WEATHER_CURRENT";
const WEATHER_FORECAST_RESOURCE: &str = "HVAC_WEATHER_FORECAST";
const OUTDOOR_AIR_QUALITY_RESOURCE: &str = "HVAC_OUTDOOR_AIR_QUALITY";

#[derive(Clone)]
struct Subscriber {
    peer: u32,
    last_report_ticks: u64,
}

#[derive(Clone, Copy, Default)]
struct ConcentrationDraft {
    kind: Option<BuildingHvacAirMeasurementKindV1>,
    value: Option<f32>,
    unit: Option<BuildingHvacAirMeasurementUnitV1>,
    medium_is_air: Option<bool>,
    level: Option<BuildingHvacConcentrationLevelV1>,
}

#[derive(Clone)]
struct AirDeviceDraft {
    device: LibertasDevice,
    overall: Option<BuildingHvacAirQualityV1>,
    concentrations: [ConcentrationDraft; BUILDING_HVAC_MAX_AIR_MEASUREMENTS],
}

impl AirDeviceDraft {
    fn new(device: LibertasDevice) -> Self {
        Self {
            device,
            overall: None,
            concentrations: core::array::from_fn(|_| ConcentrationDraft::default()),
        }
    }

    fn reading(&self, now: LibertasDateTime) -> Option<BuildingHvacAirQualityReadingV1> {
        let mut measurements: Vec<_> = self
            .concentrations
            .iter()
            .filter_map(|draft| {
                Some(BuildingHvacAirMeasurementV1 {
                    kind: draft.kind?,
                    measured_value_in_reported_unit: draft.value?,
                    reported_unit: draft.unit?,
                    level: draft.level,
                })
                .filter(|value| draft.medium_is_air == Some(true) && value.is_well_formed())
            })
            .collect();
        measurements.sort_by_key(|measurement| measurement.kind);
        measurements.dedup_by_key(|measurement| measurement.kind);
        if self.overall.is_none() && measurements.is_empty() {
            return None;
        }
        Some(BuildingHvacAirQualityReadingV1 {
            observed_at: now,
            valid_until: now.saturating_add(MATTER_READING_FRESHNESS_SECONDS),
            overall_air_quality: self.overall,
            measurements,
        })
    }
}

#[derive(Clone)]
struct ThermostatRuntime {
    configuration: BuildingHvacThermostatV1,
    observed_at: Option<LibertasDateTime>,
    valid_until: Option<LibertasDateTime>,
    last_report_ticks: Option<u64>,
    activity: BuildingHvacRoomActivityV1,
    running_mode: Option<u8>,
    running_state: Option<u16>,
    control_sequence: Option<u8>,
    heating_setpoint_celsius: Option<f32>,
    cooling_setpoint_celsius: Option<f32>,
    minimum_heating_setpoint_celsius: Option<f32>,
    maximum_heating_setpoint_celsius: Option<f32>,
    minimum_cooling_setpoint_celsius: Option<f32>,
    maximum_cooling_setpoint_celsius: Option<f32>,
    minimum_deadband_celsius: Option<f32>,
    pending_write: Option<(LibertasTransId, Option<f32>, Option<f32>)>,
}

impl ThermostatRuntime {
    fn new(configuration: BuildingHvacThermostatV1) -> Self {
        Self {
            configuration,
            observed_at: None,
            valid_until: None,
            last_report_ticks: None,
            activity: BuildingHvacRoomActivityV1::Unknown,
            running_mode: None,
            running_state: None,
            control_sequence: None,
            heating_setpoint_celsius: None,
            cooling_setpoint_celsius: None,
            minimum_heating_setpoint_celsius: None,
            maximum_heating_setpoint_celsius: None,
            minimum_cooling_setpoint_celsius: None,
            maximum_cooling_setpoint_celsius: None,
            minimum_deadband_celsius: None,
            pending_write: None,
        }
    }

    fn limits(&self) -> Option<BuildingHvacThermostatControlLimits> {
        let limits = BuildingHvacThermostatControlLimits {
            minimum_heating_setpoint_celsius: self.minimum_heating_setpoint_celsius?,
            maximum_heating_setpoint_celsius: self.maximum_heating_setpoint_celsius?,
            minimum_cooling_setpoint_celsius: self.minimum_cooling_setpoint_celsius?,
            maximum_cooling_setpoint_celsius: self.maximum_cooling_setpoint_celsius?,
            minimum_deadband_celsius: self.minimum_deadband_celsius?,
        };
        limits.is_well_formed().then_some(limits)
    }

    fn supports_heat(&self) -> bool {
        self.control_sequence
            .is_some_and(|value| matches!(value, 2..=5))
    }

    fn supports_cool(&self) -> bool {
        self.control_sequence
            .is_some_and(|value| matches!(value, 0 | 1 | 4 | 5))
    }

    fn refresh_activity(&mut self) {
        self.activity = if let Some(state) = self.running_state {
            if state & 0x09 != 0 {
                BuildingHvacRoomActivityV1::Heating
            } else if state & 0x12 != 0 {
                BuildingHvacRoomActivityV1::Cooling
            } else if state & 0x64 != 0 {
                BuildingHvacRoomActivityV1::FanOnly
            } else {
                BuildingHvacRoomActivityV1::Idle
            }
        } else {
            match self.running_mode {
                Some(4) => BuildingHvacRoomActivityV1::Heating,
                Some(3) => BuildingHvacRoomActivityV1::Cooling,
                Some(0) => BuildingHvacRoomActivityV1::Idle,
                _ => BuildingHvacRoomActivityV1::Unknown,
            }
        };
    }
}

#[derive(Clone)]
struct PendingFeatures {
    observed_at: LibertasDateTime,
    temperature_celsius: f32,
    features: BuildingHvacMachineLearningFeaturesV1,
    persisted_15: bool,
    persisted_30: bool,
    persisted_60: bool,
}

struct RoomRuntime {
    configuration: BuildingHvacRoomV1,
    thermostat_index: usize,
    sensor_states: Vec<BuildingHvacIndoorSensorStateV1>,
    control_revision: u64,
    control: BuildingHvacRoomControlV1,
    state: BuildingHvacRoomObservedStateV1,
    recent_conditions: Vec<BuildingHvacPersistedRoomConditionPeriodV1>,
    statistics: Option<BuildingHvacRoomStatisticsV1>,
    learning: BuildingHvacRoomLearningStateV1,
    urgent: BuildingHvacUrgentNotificationEngine,
    machine_learning: BuildingHvacRoomMachineLearningV1,
    plan: Option<BuildingHvacRoomPlanV1>,
    subscribers: Vec<Subscriber>,
    last_report: Option<BuildingHvacRoomProtocolV1>,
    last_condition_boundary: Option<LibertasDateTime>,
    pending_features: Vec<PendingFeatures>,
    last_training_at: Option<LibertasDateTime>,
}

struct ControllerState {
    recipients: Vec<LibertasUser>,
    weather_endpoint: LibertasEndpoint,
    weather: BuildingHvacWeatherSnapshotV1,
    weather_cursor: Option<BuildingHvacWeatherCursorV1>,
    weather_stream_ready: bool,
    weather_maximum_wait_seconds: u32,
    weather_retry_timer: u32,
    local_outdoor: Option<BuildingHvacLocalOutdoorSensorStateV1>,
    thermostats: Vec<ThermostatRuntime>,
    rooms: Vec<RoomRuntime>,
    air_drafts: Vec<AirDeviceDraft>,
    machine_learning_client: BuildingHvacMachineLearningClient,
    machine_learning_results: Receiver<BuildingHvacMachineLearningResult>,
    model_sets: Vec<BuildingHvacMachineLearningModelSetV1>,
    next_prediction_request_id: u64,
    next_prediction_room: usize,
    next_training_room: usize,
    last_ml_sample_boundary: Option<LibertasDateTime>,
    last_prediction_minute: Option<LibertasDateTime>,
    outdoor_configuration: Option<BuildingHvacOutdoorSensorV1>,
}

struct RoomContext {
    shared: Rc<RefCell<ControllerState>>,
    room_index: usize,
}

#[derive(Clone, Copy)]
enum DeviceRole {
    Thermostat(usize),
    IndoorTemperature { room: usize, sensor: usize },
    IndoorHumidity { room: usize, sensor: usize },
    IndoorAirQuality { room: usize, sensor: usize },
    OutdoorTemperature,
    OutdoorHumidity,
    OutdoorAirQuality,
}

struct DeviceContext {
    shared: Rc<RefCell<ControllerState>>,
    role: DeviceRole,
}

struct ShutdownContext {
    client: BuildingHvacMachineLearningClient,
}

struct RoomPersistence {
    resource: &'static str,
    endpoint: LibertasEndpoint,
    value: BuildingHvacPersistentDataV1,
}

struct UrgentSubmission {
    endpoint: LibertasEndpoint,
    room_name: String,
    engine: BuildingHvacUrgentNotificationEngine,
    evaluation: BuildingHvacUrgentNotificationEvaluation,
}

fn absolute_ticks(now_ticks: u64, seconds: u32) -> u64 {
    now_ticks.saturating_add(u64::from(seconds).saturating_mul(MICROSECONDS_PER_SECOND))
}

fn room_key(endpoint: LibertasEndpoint) -> [NotificationArgument<'static>; 1] {
    [NotificationArgument::Object(endpoint)]
}

fn singleton_key() -> &'static [NotificationArgument<'static>] {
    &[]
}

fn default_sensor_states(
    sensors: &[BuildingHvacIndoorSensorV1],
) -> Vec<BuildingHvacIndoorSensorStateV1> {
    sensors
        .iter()
        .map(|sensor| BuildingHvacIndoorSensorStateV1 {
            temperature_sensor: sensor.temperature_sensor,
            temperature: None,
            humidity_sensor: sensor.humidity_sensor,
            humidity: None,
            air_quality_sensor: sensor.air_quality_sensor,
            air_quality: None,
        })
        .collect()
}

fn restored_sensor_states(
    endpoint: LibertasEndpoint,
    sensors: &[BuildingHvacIndoorSensorV1],
) -> Vec<BuildingHvacIndoorSensorStateV1> {
    let defaults = default_sensor_states(sensors);
    let Some(BuildingHvacPersistentDataV1::RoomSensorStateV1 { sensors: restored }) =
        libertas_data_read(ROOM_SENSOR_STATE_RESOURCE, &room_key(endpoint))
    else {
        let value = BuildingHvacPersistentDataV1::RoomSensorStateV1 {
            sensors: defaults.clone(),
        };
        libertas_data_write(ROOM_SENSOR_STATE_RESOURCE, &room_key(endpoint), &value);
        return defaults;
    };
    let valid = restored.len() == defaults.len()
        && restored.iter().zip(&defaults).all(|(actual, expected)| {
            actual.is_well_formed()
                && actual.temperature_sensor == expected.temperature_sensor
                && actual.humidity_sensor == expected.humidity_sensor
                && actual.air_quality_sensor == expected.air_quality_sensor
        });
    if valid {
        restored
    } else {
        let value = BuildingHvacPersistentDataV1::RoomSensorStateV1 {
            sensors: defaults.clone(),
        };
        libertas_data_write(ROOM_SENSOR_STATE_RESOURCE, &room_key(endpoint), &value);
        defaults
    }
}

fn restore_control(endpoint: LibertasEndpoint) -> (u64, BuildingHvacRoomControlV1) {
    if let Some(BuildingHvacPersistentDataV1::RoomControlV1 {
        control_revision,
        control,
    }) = libertas_data_read(ROOM_CONTROL_RESOURCE, &room_key(endpoint))
        && control.is_well_formed()
    {
        return (control_revision, control);
    }
    let control = BuildingHvacRoomControlV1::default();
    libertas_data_write(
        ROOM_CONTROL_RESOURCE,
        &room_key(endpoint),
        &BuildingHvacPersistentDataV1::RoomControlV1 {
            control_revision: 0,
            control,
        },
    );
    (0, control)
}

fn valid_condition_periods(periods: &[BuildingHvacPersistedRoomConditionPeriodV1]) -> bool {
    periods.len() <= BUILDING_HVAC_MAX_PERSISTED_ROOM_CONDITION_PERIODS
        && periods.iter().enumerate().all(|(index, period)| {
            period.duration_seconds != 0
                && period
                    .starts_at
                    .checked_add(u64::from(period.duration_seconds))
                    .is_some()
                && period.temperature_celsius.is_none_or(f32::is_finite)
                && period
                    .relative_humidity_percent
                    .is_none_or(|value| value.is_finite() && (0.0..=100.0).contains(&value))
                && period
                    .effective_heating_setpoint_celsius
                    .is_none_or(f32::is_finite)
                && period
                    .effective_cooling_setpoint_celsius
                    .is_none_or(f32::is_finite)
                && period
                    .outdoor_dry_bulb_temperature_celsius
                    .is_none_or(f32::is_finite)
                && (index == 0
                    || periods[index - 1]
                        .starts_at
                        .saturating_add(u64::from(periods[index - 1].duration_seconds))
                        <= period.starts_at)
        })
}

fn empty_learning() -> BuildingHvacRoomLearningStateV1 {
    BuildingHvacRoomLearningStateV1 {
        passive_outdoor_coupling: BuildingHvacOnlineRegressionStateV1::empty(),
        cross_zone_learners: Vec::new(),
    }
}

fn restore_room_history(
    endpoint: LibertasEndpoint,
) -> (
    Vec<BuildingHvacPersistedRoomConditionPeriodV1>,
    Option<BuildingHvacRoomStatisticsV1>,
) {
    if let Some(BuildingHvacPersistentDataV1::RoomStatisticsV1 {
        statistics,
        recent_conditions,
    }) = libertas_data_read(ROOM_STATISTICS_RESOURCE, &room_key(endpoint))
        && valid_condition_periods(&recent_conditions)
        && BuildingHvacAnalyticsEngine::new()
            .summarize_conditions(&recent_conditions)
            .as_ref()
            == Some(&statistics)
    {
        return (recent_conditions, Some(statistics));
    }
    (Vec::new(), None)
}

fn valid_regression(regression: &BuildingHvacOnlineRegressionStateV1) -> bool {
    regression.effective_sample_weight.is_finite()
        && regression.effective_sample_weight >= 0.0
        && regression.weighted_input_squared_sum.is_finite()
        && regression.weighted_input_squared_sum >= 0.0
        && regression.weighted_input_output_sum.is_finite()
        && regression.weighted_output_squared_sum.is_finite()
        && regression.weighted_output_squared_sum >= 0.0
        && (regression.updated_at.is_some()
            || regression.accepted_observation_count == 0
                && regression.effective_sample_weight == 0.0
                && regression.weighted_input_squared_sum == 0.0
                && regression.weighted_input_output_sum == 0.0
                && regression.weighted_output_squared_sum == 0.0)
}

fn valid_learning(
    learning: &BuildingHvacRoomLearningStateV1,
    own_thermostat: LibertasDevice,
    configured_thermostats: &[LibertasDevice],
) -> bool {
    valid_regression(&learning.passive_outdoor_coupling)
        && learning.cross_zone_learners.len() <= BUILDING_HVAC_MAX_THERMOSTATS
        && learning
            .cross_zone_learners
            .iter()
            .enumerate()
            .all(|(index, learner)| {
                learner.source_thermostat != own_thermostat
                    && configured_thermostats.contains(&learner.source_thermostat)
                    && valid_regression(&learner.heating)
                    && valid_regression(&learner.cooling)
                    && !learning.cross_zone_learners[..index]
                        .iter()
                        .any(|previous| previous.source_thermostat == learner.source_thermostat)
            })
}

fn restore_learning(
    endpoint: LibertasEndpoint,
    own_thermostat: LibertasDevice,
    configured_thermostats: &[LibertasDevice],
) -> BuildingHvacRoomLearningStateV1 {
    if let Some(BuildingHvacPersistentDataV1::RoomLearningV1 { learning }) =
        libertas_data_read(ROOM_LEARNING_RESOURCE, &room_key(endpoint))
        && valid_learning(&learning, own_thermostat, configured_thermostats)
    {
        return learning;
    }
    let learning = empty_learning();
    libertas_data_write(
        ROOM_LEARNING_RESOURCE,
        &room_key(endpoint),
        &BuildingHvacPersistentDataV1::RoomLearningV1 {
            learning: learning.clone(),
        },
    );
    learning
}

fn restore_urgent(endpoint: LibertasEndpoint) -> BuildingHvacUrgentNotificationEngine {
    let conditions = match libertas_data_read(ROOM_URGENT_RESOURCE, &room_key(endpoint)) {
        Some(BuildingHvacPersistentDataV1::RoomUrgentNotificationStateV1 { conditions }) => {
            conditions
        }
        _ => Vec::new(),
    };
    BuildingHvacUrgentNotificationEngine::restore(conditions)
}

fn initial_state(
    thermostat: LibertasDevice,
    sensor_states: Vec<BuildingHvacIndoorSensorStateV1>,
) -> BuildingHvacRoomObservedStateV1 {
    BuildingHvacAnalyticsEngine::new().analyze_room(
        0,
        thermostat,
        None,
        None,
        BuildingHvacRoomActivityV1::Unknown,
        None,
        None,
        &sensor_states,
    )
}

fn restore_local_outdoor(
    configuration: Option<BuildingHvacOutdoorSensorV1>,
) -> Option<BuildingHvacLocalOutdoorSensorStateV1> {
    let configuration = configuration?;
    let temperature = match libertas_data_read(LOCAL_OUTDOOR_TEMPERATURE_RESOURCE, singleton_key())
    {
        Some(BuildingHvacPersistentDataV1::LocalOutdoorTemperatureV1 { temperature })
            if temperature.is_well_formed() =>
        {
            Some(temperature)
        }
        _ => None,
    };
    let humidity = configuration.humidity_sensor.and_then(|_| {
        match libertas_data_read(LOCAL_OUTDOOR_HUMIDITY_RESOURCE, singleton_key()) {
            Some(BuildingHvacPersistentDataV1::LocalOutdoorHumidityV1 { humidity })
                if humidity.is_well_formed() =>
            {
                Some(humidity)
            }
            _ => None,
        }
    });
    let air_quality = configuration.air_quality_sensor.and_then(|_| {
        match libertas_data_read(LOCAL_OUTDOOR_AIR_QUALITY_RESOURCE, singleton_key()) {
            Some(BuildingHvacPersistentDataV1::LocalOutdoorAirQualityV1 { air_quality })
                if air_quality.is_well_formed() =>
            {
                Some(air_quality)
            }
            _ => None,
        }
    });
    Some(BuildingHvacLocalOutdoorSensorStateV1 {
        temperature,
        humidity,
        air_quality,
    })
}

fn finite_conditions(value: &BuildingHvacOutdoorConditionsV1) -> bool {
    value.dry_bulb_temperature_celsius.is_finite()
        && value.dew_point_temperature_celsius.is_finite()
        && value.dew_point_temperature_celsius <= value.dry_bulb_temperature_celsius + 0.05
        && value.surface_pressure_hectopascals.is_finite()
        && value.surface_pressure_hectopascals > 0.0
        && value.wind_speed_meters_per_second.is_finite()
        && value.wind_speed_meters_per_second >= 0.0
        && value.wind_gust_meters_per_second.is_finite()
        && value.wind_gust_meters_per_second >= 0.0
        && value.wind_direction_degrees <= 360
        && value.precipitation_millimeters.is_finite()
        && value.precipitation_millimeters >= 0.0
        && value
            .global_horizontal_irradiance_watts_per_square_meter
            .is_finite()
        && value.global_horizontal_irradiance_watts_per_square_meter >= 0.0
        && value
            .direct_normal_irradiance_watts_per_square_meter
            .is_finite()
        && value.direct_normal_irradiance_watts_per_square_meter >= 0.0
        && value
            .diffuse_horizontal_irradiance_watts_per_square_meter
            .is_finite()
        && value.diffuse_horizontal_irradiance_watts_per_square_meter >= 0.0
}

fn valid_weather_history(history: &BuildingHvacWeatherHistoryV1) -> bool {
    history.valid_until > history.retrieved_at
        && history.periods.len() <= 72
        && history.periods.iter().enumerate().all(|(index, period)| {
            period.duration_seconds != 0
                && finite_conditions(&period.conditions)
                && (index == 0
                    || history.periods[index - 1]
                        .starts_at
                        .saturating_add(u64::from(history.periods[index - 1].duration_seconds))
                        <= period.starts_at)
        })
}

fn valid_current_weather(current: &BuildingHvacCurrentWeatherV1) -> bool {
    current.valid_until > current.retrieved_at
        && current.interval_seconds != 0
        && finite_conditions(&current.conditions)
}

fn valid_weather_forecast(forecast: &BuildingHvacWeatherForecastV1) -> bool {
    forecast.valid_until > forecast.retrieved_at
        && forecast.periods.len() <= BUILDING_HVAC_MAX_ROOM_PLAN_PERIODS
        && forecast.periods.iter().enumerate().all(|(index, period)| {
            period.duration_seconds != 0
                && period.precipitation_probability_percent <= 100
                && finite_conditions(&period.conditions)
                && (index == 0
                    || forecast.periods[index - 1]
                        .starts_at
                        .saturating_add(u64::from(forecast.periods[index - 1].duration_seconds))
                        <= period.starts_at)
        })
}

fn valid_outdoor_air_quality(air_quality: &BuildingHvacOutdoorAirQualityV1) -> bool {
    air_quality.valid_until > air_quality.retrieved_at
        && air_quality.periods.len() <= 48
        && air_quality
            .periods
            .iter()
            .enumerate()
            .all(|(index, period)| {
                period.duration_seconds != 0
                    && [
                        period.particulate_matter_2_5_micrograms_per_cubic_meter,
                        period.particulate_matter_10_micrograms_per_cubic_meter,
                        period.ozone_micrograms_per_cubic_meter,
                        period.nitrogen_dioxide_micrograms_per_cubic_meter,
                    ]
                    .into_iter()
                    .all(|value| value.is_finite() && value >= 0.0)
                    && (index == 0
                        || air_quality.periods[index - 1]
                            .starts_at
                            .saturating_add(u64::from(
                                air_quality.periods[index - 1].duration_seconds,
                            ))
                            <= period.starts_at)
            })
}

fn valid_weather_snapshot(snapshot: &BuildingHvacWeatherSnapshotV1) -> bool {
    snapshot.history.as_ref().is_none_or(valid_weather_history)
        && snapshot.current.as_ref().is_none_or(valid_current_weather)
        && snapshot
            .forecast
            .as_ref()
            .is_none_or(valid_weather_forecast)
        && snapshot
            .outdoor_air_quality
            .as_ref()
            .is_none_or(valid_outdoor_air_quality)
}

fn restore_weather() -> BuildingHvacWeatherSnapshotV1 {
    let history = match libertas_data_read(WEATHER_HISTORY_RESOURCE, singleton_key()) {
        Some(BuildingHvacPersistentDataV1::WeatherHistoryV1 { history })
            if valid_weather_history(&history) =>
        {
            Some(history)
        }
        _ => None,
    };
    let current = match libertas_data_read(WEATHER_CURRENT_RESOURCE, singleton_key()) {
        Some(BuildingHvacPersistentDataV1::WeatherCurrentV1 { current })
            if valid_current_weather(&current) =>
        {
            Some(current)
        }
        _ => None,
    };
    let forecast = match libertas_data_read(WEATHER_FORECAST_RESOURCE, singleton_key()) {
        Some(BuildingHvacPersistentDataV1::WeatherForecastV1 { forecast })
            if valid_weather_forecast(&forecast) =>
        {
            Some(forecast)
        }
        _ => None,
    };
    let outdoor_air_quality =
        match libertas_data_read(OUTDOOR_AIR_QUALITY_RESOURCE, singleton_key()) {
            Some(BuildingHvacPersistentDataV1::OutdoorAirQualityV1 {
                outdoor_air_quality,
            }) if valid_outdoor_air_quality(&outdoor_air_quality) => Some(outdoor_air_quality),
            _ => None,
        };
    BuildingHvacWeatherSnapshotV1 {
        history,
        current,
        forecast,
        outdoor_air_quality,
    }
}

fn persist_weather(previous: &BuildingHvacWeatherSnapshotV1, next: &BuildingHvacWeatherSnapshotV1) {
    if previous.history != next.history {
        match &next.history {
            Some(history) => libertas_data_write(
                WEATHER_HISTORY_RESOURCE,
                singleton_key(),
                &BuildingHvacPersistentDataV1::WeatherHistoryV1 {
                    history: history.clone(),
                },
            ),
            None => libertas_data_remove(WEATHER_HISTORY_RESOURCE, singleton_key()),
        }
    }
    if previous.current != next.current {
        match next.current {
            Some(current) => libertas_data_write(
                WEATHER_CURRENT_RESOURCE,
                singleton_key(),
                &BuildingHvacPersistentDataV1::WeatherCurrentV1 { current },
            ),
            None => libertas_data_remove(WEATHER_CURRENT_RESOURCE, singleton_key()),
        }
    }
    if previous.forecast != next.forecast {
        match &next.forecast {
            Some(forecast) => libertas_data_write(
                WEATHER_FORECAST_RESOURCE,
                singleton_key(),
                &BuildingHvacPersistentDataV1::WeatherForecastV1 {
                    forecast: forecast.clone(),
                },
            ),
            None => libertas_data_remove(WEATHER_FORECAST_RESOURCE, singleton_key()),
        }
    }
    if previous.outdoor_air_quality != next.outdoor_air_quality {
        match &next.outdoor_air_quality {
            Some(outdoor_air_quality) => libertas_data_write(
                OUTDOOR_AIR_QUALITY_RESOURCE,
                singleton_key(),
                &BuildingHvacPersistentDataV1::OutdoorAirQualityV1 {
                    outdoor_air_quality: outdoor_air_quality.clone(),
                },
            ),
            None => libertas_data_remove(OUTDOOR_AIR_QUALITY_RESOURCE, singleton_key()),
        }
    }
}

fn room_association(
    building: &BuildingHvacBuildingV1,
    room_index: usize,
) -> Option<(usize, &BuildingHvacThermostatRoomV1)> {
    building
        .thermostats
        .iter()
        .enumerate()
        .find_map(|(thermostat_index, thermostat)| {
            thermostat
                .rooms
                .iter()
                .find(|association| usize::from(association.room_index) == room_index)
                .map(|association| (thermostat_index, association))
        })
}

fn build_air_drafts(building: &BuildingHvacBuildingV1) -> Vec<AirDeviceDraft> {
    let mut devices = Vec::new();
    for thermostat in &building.thermostats {
        for association in &thermostat.rooms {
            for device in association
                .sensors
                .iter()
                .filter_map(|sensor| sensor.air_quality_sensor)
            {
                devices.push(AirDeviceDraft::new(device));
            }
        }
    }
    if let Some(device) = building
        .outdoor_sensor
        .and_then(|sensor| sensor.air_quality_sensor)
    {
        devices.push(AirDeviceDraft::new(device));
    }
    devices
}

fn persist_room_sensors(endpoint: LibertasEndpoint, sensors: Vec<BuildingHvacIndoorSensorStateV1>) {
    libertas_data_write(
        ROOM_SENSOR_STATE_RESOURCE,
        &room_key(endpoint),
        &BuildingHvacPersistentDataV1::RoomSensorStateV1 { sensors },
    );
}

fn setpoint_celsius(raw: i16) -> f32 {
    f32::from(raw) / 100.0
}

fn raw_setpoint(celsius: f32) -> Option<i16> {
    if !celsius.is_finite() {
        return None;
    }
    let value = (celsius * 100.0).round();
    (value >= f32::from(i16::MIN) && value <= f32::from(i16::MAX)).then_some(value as i16)
}

fn map_air_quality(raw: u8) -> Option<BuildingHvacAirQualityV1> {
    Some(match raw {
        0 => BuildingHvacAirQualityV1::Unknown,
        1 => BuildingHvacAirQualityV1::Good,
        2 => BuildingHvacAirQualityV1::Fair,
        3 => BuildingHvacAirQualityV1::Moderate,
        4 => BuildingHvacAirQualityV1::Poor,
        5 => BuildingHvacAirQualityV1::VeryPoor,
        6 => BuildingHvacAirQualityV1::ExtremelyPoor,
        _ => return None,
    })
}

fn map_concentration_unit(raw: u8) -> Option<BuildingHvacAirMeasurementUnitV1> {
    Some(match raw {
        0 => BuildingHvacAirMeasurementUnitV1::PartsPerMillion,
        1 => BuildingHvacAirMeasurementUnitV1::PartsPerBillion,
        2 => BuildingHvacAirMeasurementUnitV1::PartsPerTrillion,
        3 => BuildingHvacAirMeasurementUnitV1::MilligramsPerCubicMeter,
        4 => BuildingHvacAirMeasurementUnitV1::MicrogramsPerCubicMeter,
        5 => BuildingHvacAirMeasurementUnitV1::NanogramsPerCubicMeter,
        6 => BuildingHvacAirMeasurementUnitV1::PicogramsPerCubicMeter,
        7 => BuildingHvacAirMeasurementUnitV1::BecquerelsPerCubicMeter,
        _ => return None,
    })
}

fn map_concentration_level(raw: u8) -> Option<BuildingHvacConcentrationLevelV1> {
    Some(match raw {
        0 => BuildingHvacConcentrationLevelV1::Unknown,
        1 => BuildingHvacConcentrationLevelV1::Low,
        2 => BuildingHvacConcentrationLevelV1::Medium,
        3 => BuildingHvacConcentrationLevelV1::High,
        4 => BuildingHvacConcentrationLevelV1::Critical,
        _ => return None,
    })
}

enum ConcentrationUpdate {
    Value(Option<f32>),
    Unit(u8),
    Medium(u8),
    Level(u8),
}

macro_rules! decode_concentration {
    ($data:expr, $module:ident, $kind:expr) => {{
        use libertas_matter::definitions::$module::attributes::{
            LevelValue, MeasuredValue, MeasurementMedium, MeasurementUnit,
        };
        if let Ok(MatterResponse::Data(MeasuredValue(value))) =
            decode_attribute_report::<MeasuredValue>($data)
        {
            Some(($kind, ConcentrationUpdate::Value(value.into_option())))
        } else if let Ok(MatterResponse::Data(MeasurementUnit(value))) =
            decode_attribute_report::<MeasurementUnit>($data)
        {
            Some(($kind, ConcentrationUpdate::Unit(value.0)))
        } else if let Ok(MatterResponse::Data(MeasurementMedium(value))) =
            decode_attribute_report::<MeasurementMedium>($data)
        {
            Some(($kind, ConcentrationUpdate::Medium(value.0)))
        } else if let Ok(MatterResponse::Data(LevelValue(value))) =
            decode_attribute_report::<LevelValue>($data)
        {
            Some(($kind, ConcentrationUpdate::Level(value.0)))
        } else {
            None
        }
    }};
}

fn decode_concentration_update(
    data: &[u8],
) -> Option<(BuildingHvacAirMeasurementKindV1, ConcentrationUpdate)> {
    decode_concentration!(
        data,
        CarbonDioxideConcentrationMeasurement,
        BuildingHvacAirMeasurementKindV1::CarbonDioxide
    )
    .or_else(|| {
        decode_concentration!(
            data,
            CarbonMonoxideConcentrationMeasurement,
            BuildingHvacAirMeasurementKindV1::CarbonMonoxide
        )
    })
    .or_else(|| {
        decode_concentration!(
            data,
            NitrogenDioxideConcentrationMeasurement,
            BuildingHvacAirMeasurementKindV1::NitrogenDioxide
        )
    })
    .or_else(|| {
        decode_concentration!(
            data,
            OzoneConcentrationMeasurement,
            BuildingHvacAirMeasurementKindV1::Ozone
        )
    })
    .or_else(|| {
        decode_concentration!(
            data,
            PM1ConcentrationMeasurement,
            BuildingHvacAirMeasurementKindV1::ParticulateMatter1
        )
    })
    .or_else(|| {
        decode_concentration!(
            data,
            PM25ConcentrationMeasurement,
            BuildingHvacAirMeasurementKindV1::ParticulateMatter2_5
        )
    })
    .or_else(|| {
        decode_concentration!(
            data,
            PM10ConcentrationMeasurement,
            BuildingHvacAirMeasurementKindV1::ParticulateMatter10
        )
    })
    .or_else(|| {
        decode_concentration!(
            data,
            FormaldehydeConcentrationMeasurement,
            BuildingHvacAirMeasurementKindV1::Formaldehyde
        )
    })
    .or_else(|| {
        decode_concentration!(
            data,
            TotalVolatileOrganicCompoundsConcentrationMeasurement,
            BuildingHvacAirMeasurementKindV1::TotalVolatileOrganicCompounds
        )
    })
    .or_else(|| {
        decode_concentration!(
            data,
            RadonConcentrationMeasurement,
            BuildingHvacAirMeasurementKindV1::Radon
        )
    })
}

fn air_kind_index(kind: BuildingHvacAirMeasurementKindV1) -> usize {
    match kind {
        BuildingHvacAirMeasurementKindV1::CarbonDioxide => 0,
        BuildingHvacAirMeasurementKindV1::CarbonMonoxide => 1,
        BuildingHvacAirMeasurementKindV1::NitrogenDioxide => 2,
        BuildingHvacAirMeasurementKindV1::Ozone => 3,
        BuildingHvacAirMeasurementKindV1::ParticulateMatter1 => 4,
        BuildingHvacAirMeasurementKindV1::ParticulateMatter2_5 => 5,
        BuildingHvacAirMeasurementKindV1::ParticulateMatter10 => 6,
        BuildingHvacAirMeasurementKindV1::Formaldehyde => 7,
        BuildingHvacAirMeasurementKindV1::TotalVolatileOrganicCompounds => 8,
        BuildingHvacAirMeasurementKindV1::Radon => 9,
    }
}

fn handle_air_report(
    state: &mut ControllerState,
    device: LibertasDevice,
    data: &[u8],
    now: LibertasDateTime,
) -> bool {
    let Some(draft) = state
        .air_drafts
        .iter_mut()
        .find(|draft| draft.device == device)
    else {
        return false;
    };
    if let Ok(MatterResponse::Data(AirQuality::attributes::AirQuality(value))) =
        decode_attribute_report::<AirQuality::attributes::AirQuality>(data)
    {
        let Some(value) = map_air_quality(value.0) else {
            return false;
        };
        draft.overall = Some(value);
        return true;
    }
    let Some((kind, update)) = decode_concentration_update(data) else {
        return false;
    };
    let concentration = &mut draft.concentrations[air_kind_index(kind)];
    concentration.kind = Some(kind);
    match update {
        ConcentrationUpdate::Value(value) => {
            concentration.value = value.filter(|value| value.is_finite() && *value >= 0.0);
        }
        ConcentrationUpdate::Unit(raw) => concentration.unit = map_concentration_unit(raw),
        ConcentrationUpdate::Medium(raw) => concentration.medium_is_air = Some(raw == 0),
        ConcentrationUpdate::Level(raw) => concentration.level = map_concentration_level(raw),
    }
    let _ = now;
    true
}

fn assign_air_reading(
    state: &mut ControllerState,
    role: DeviceRole,
    device: LibertasDevice,
    now: LibertasDateTime,
) -> bool {
    let reading = state
        .air_drafts
        .iter()
        .find(|draft| draft.device == device)
        .and_then(|draft| draft.reading(now));
    match role {
        DeviceRole::IndoorAirQuality { room, sensor } => {
            let target = &mut state.rooms[room].sensor_states[sensor].air_quality;
            let changed = *target != reading;
            *target = reading;
            changed
        }
        DeviceRole::OutdoorAirQuality => {
            let Some(outdoor) = &mut state.local_outdoor else {
                return false;
            };
            let changed = outdoor.air_quality != reading;
            outdoor.air_quality = reading;
            changed
        }
        _ => false,
    }
}

fn handle_thermostat_report(
    thermostat: &mut ThermostatRuntime,
    data: &[u8],
    now: LibertasDateTime,
    now_ticks: u64,
) -> bool {
    use Thermostat::attributes::{
        ControlSequenceOfOperation, MaxCoolSetpointLimit, MaxHeatSetpointLimit,
        MinCoolSetpointLimit, MinHeatSetpointLimit, MinSetpointDeadBand, OccupiedCoolingSetpoint,
        OccupiedHeatingSetpoint, ThermostatRunningMode, ThermostatRunningState,
    };

    let mut changed = false;
    macro_rules! decode_scalar {
        ($ty:ty, $pattern:pat => $body:expr) => {
            if let Ok(MatterResponse::Data($pattern)) = decode_attribute_report::<$ty>(data) {
                $body;
                changed = true;
            }
        };
    }
    decode_scalar!(OccupiedHeatingSetpoint, OccupiedHeatingSetpoint(value) => {
        thermostat.heating_setpoint_celsius = Some(setpoint_celsius(value));
    });
    decode_scalar!(OccupiedCoolingSetpoint, OccupiedCoolingSetpoint(value) => {
        thermostat.cooling_setpoint_celsius = Some(setpoint_celsius(value));
    });
    decode_scalar!(MinHeatSetpointLimit, MinHeatSetpointLimit(value) => {
        thermostat.minimum_heating_setpoint_celsius = Some(setpoint_celsius(value));
    });
    decode_scalar!(MaxHeatSetpointLimit, MaxHeatSetpointLimit(value) => {
        thermostat.maximum_heating_setpoint_celsius = Some(setpoint_celsius(value));
    });
    decode_scalar!(MinCoolSetpointLimit, MinCoolSetpointLimit(value) => {
        thermostat.minimum_cooling_setpoint_celsius = Some(setpoint_celsius(value));
    });
    decode_scalar!(MaxCoolSetpointLimit, MaxCoolSetpointLimit(value) => {
        thermostat.maximum_cooling_setpoint_celsius = Some(setpoint_celsius(value));
    });
    decode_scalar!(MinSetpointDeadBand, MinSetpointDeadBand(value) => {
        thermostat.minimum_deadband_celsius = Some(f32::from(value) / 10.0);
    });
    decode_scalar!(ControlSequenceOfOperation, ControlSequenceOfOperation(value) => {
        thermostat.control_sequence = Some(value.0);
    });
    decode_scalar!(ThermostatRunningMode, ThermostatRunningMode(value) => {
        thermostat.running_mode = Some(value.0);
    });
    decode_scalar!(ThermostatRunningState, ThermostatRunningState(value) => {
        thermostat.running_state = Some(value.0);
    });
    if changed {
        thermostat.observed_at = Some(now);
        thermostat.valid_until = Some(now.saturating_add(MATTER_READING_FRESHNESS_SECONDS));
        thermostat.last_report_ticks = Some(now_ticks);
        thermostat.refresh_activity();
        if thermostat
            .pending_write
            .is_some_and(|(_, heating, cooling)| {
                heating.is_none_or(|target| {
                    thermostat.heating_setpoint_celsius.is_some_and(|actual| {
                        (actual - target).abs() <= BUILDING_HVAC_SETPOINT_COMMAND_TOLERANCE_CELSIUS
                    })
                }) && cooling.is_none_or(|target| {
                    thermostat.cooling_setpoint_celsius.is_some_and(|actual| {
                        (actual - target).abs() <= BUILDING_HVAC_SETPOINT_COMMAND_TOLERANCE_CELSIUS
                    })
                })
            })
        {
            thermostat.pending_write = None;
        }
    }
    changed
}

fn handle_device_event(
    device: LibertasDevice,
    opcode: u8,
    data: &[u8],
    context: &mut Box<dyn Any>,
    transaction_id: LibertasTransId,
    _peer: u32,
) {
    let context = context
        .downcast_mut::<DeviceContext>()
        .expect("invalid smart building HVAC Matter context");
    if opcode == Operation::WriteResponse as u8 {
        if let DeviceRole::Thermostat(index) = context.role {
            let failed = [
                decode_write_response::<Thermostat::attributes::OccupiedHeatingSetpoint>(data),
                decode_write_response::<Thermostat::attributes::OccupiedCoolingSetpoint>(data),
            ]
            .into_iter()
            .flatten()
            .any(|status| status.status != 0);
            if failed {
                let rejected = {
                    let mut state = context.shared.borrow_mut();
                    let thermostat = &mut state.thermostats[index];
                    let rejected = thermostat
                        .pending_write
                        .is_some_and(|pending| pending.0 == transaction_id);
                    if rejected {
                        thermostat.pending_write = None;
                    }
                    rejected
                };
                if rejected {
                    libertas_log(
                        LogLevel::Warn,
                        "Matter thermostat rejected a setpoint write",
                    );
                }
            }
        }
        return;
    }
    if opcode != Operation::ReportData as u8 {
        return;
    }
    let Some(now) = libertas_get_utc_time() else {
        return;
    };
    let now_ticks = libertas_get_sys_ticks();
    let mut persist_room = None;
    let mut persist_outdoor_temperature = None;
    let mut persist_outdoor_humidity = None;
    let mut persist_outdoor_air_quality = None;
    let changed = {
        let mut state = context.shared.borrow_mut();
        match context.role {
            DeviceRole::Thermostat(index) => {
                handle_thermostat_report(&mut state.thermostats[index], data, now, now_ticks)
            }
            DeviceRole::IndoorTemperature { room, sensor } => {
                use TemperatureMeasurement::attributes::MeasuredValue;
                let Ok(MatterResponse::Data(MeasuredValue(value))) =
                    decode_attribute_report::<MeasuredValue>(data)
                else {
                    return;
                };
                let reading = value.into_option().and_then(|value| {
                    let reading = BuildingHvacTemperatureReadingV1 {
                        observed_at: now,
                        valid_until: now.saturating_add(MATTER_READING_FRESHNESS_SECONDS),
                        temperature_celsius: setpoint_celsius(value),
                    };
                    reading.is_well_formed().then_some(reading)
                });
                let target = &mut state.rooms[room].sensor_states[sensor].temperature;
                let changed = *target != reading;
                *target = reading;
                if changed {
                    persist_room = Some(room);
                }
                changed
            }
            DeviceRole::IndoorHumidity { room, sensor } => {
                use RelativeHumidityMeasurement::attributes::MeasuredValue;
                let Ok(MatterResponse::Data(MeasuredValue(value))) =
                    decode_attribute_report::<MeasuredValue>(data)
                else {
                    return;
                };
                let reading = value.into_option().and_then(|value| {
                    let reading = BuildingHvacHumidityReadingV1 {
                        observed_at: now,
                        valid_until: now.saturating_add(MATTER_READING_FRESHNESS_SECONDS),
                        relative_humidity_percent: f32::from(value) / 100.0,
                    };
                    reading.is_well_formed().then_some(reading)
                });
                let target = &mut state.rooms[room].sensor_states[sensor].humidity;
                let changed = *target != reading;
                *target = reading;
                if changed {
                    persist_room = Some(room);
                }
                changed
            }
            DeviceRole::IndoorAirQuality { room, .. } => {
                if !handle_air_report(&mut state, device, data, now) {
                    false
                } else {
                    let changed = assign_air_reading(&mut state, context.role, device, now);
                    if changed {
                        persist_room = Some(room);
                    }
                    changed
                }
            }
            DeviceRole::OutdoorTemperature => {
                use TemperatureMeasurement::attributes::MeasuredValue;
                let Ok(MatterResponse::Data(MeasuredValue(value))) =
                    decode_attribute_report::<MeasuredValue>(data)
                else {
                    return;
                };
                let reading = value.into_option().and_then(|value| {
                    let reading = BuildingHvacTemperatureReadingV1 {
                        observed_at: now,
                        valid_until: now.saturating_add(MATTER_READING_FRESHNESS_SECONDS),
                        temperature_celsius: setpoint_celsius(value),
                    };
                    reading.is_well_formed().then_some(reading)
                });
                let Some(outdoor) = &mut state.local_outdoor else {
                    return;
                };
                let changed = outdoor.temperature != reading;
                outdoor.temperature = reading;
                if changed {
                    persist_outdoor_temperature = Some(reading);
                }
                changed
            }
            DeviceRole::OutdoorHumidity => {
                use RelativeHumidityMeasurement::attributes::MeasuredValue;
                let Ok(MatterResponse::Data(MeasuredValue(value))) =
                    decode_attribute_report::<MeasuredValue>(data)
                else {
                    return;
                };
                let reading = value.into_option().and_then(|value| {
                    let reading = BuildingHvacHumidityReadingV1 {
                        observed_at: now,
                        valid_until: now.saturating_add(MATTER_READING_FRESHNESS_SECONDS),
                        relative_humidity_percent: f32::from(value) / 100.0,
                    };
                    reading.is_well_formed().then_some(reading)
                });
                let Some(outdoor) = &mut state.local_outdoor else {
                    return;
                };
                let changed = outdoor.humidity != reading;
                outdoor.humidity = reading;
                if changed {
                    persist_outdoor_humidity = Some(reading);
                }
                changed
            }
            DeviceRole::OutdoorAirQuality => {
                if !handle_air_report(&mut state, device, data, now) {
                    false
                } else {
                    let changed = assign_air_reading(&mut state, context.role, device, now);
                    if changed {
                        persist_outdoor_air_quality = Some(
                            state
                                .local_outdoor
                                .as_ref()
                                .and_then(|outdoor| outdoor.air_quality.clone()),
                        );
                    }
                    changed
                }
            }
        }
    };
    if let Some(room) = persist_room {
        let (endpoint, sensors) = {
            let state = context.shared.borrow();
            (
                state.rooms[room].configuration.control_endpoint,
                state.rooms[room].sensor_states.clone(),
            )
        };
        persist_room_sensors(endpoint, sensors);
    }
    if let Some(temperature) = persist_outdoor_temperature {
        match temperature {
            Some(temperature) => libertas_data_write(
                LOCAL_OUTDOOR_TEMPERATURE_RESOURCE,
                singleton_key(),
                &BuildingHvacPersistentDataV1::LocalOutdoorTemperatureV1 { temperature },
            ),
            None => libertas_data_remove(LOCAL_OUTDOOR_TEMPERATURE_RESOURCE, singleton_key()),
        }
    }
    if let Some(humidity) = persist_outdoor_humidity {
        match humidity {
            Some(humidity) => libertas_data_write(
                LOCAL_OUTDOOR_HUMIDITY_RESOURCE,
                singleton_key(),
                &BuildingHvacPersistentDataV1::LocalOutdoorHumidityV1 { humidity },
            ),
            None => libertas_data_remove(LOCAL_OUTDOOR_HUMIDITY_RESOURCE, singleton_key()),
        }
    }
    if let Some(air_quality) = persist_outdoor_air_quality {
        match air_quality {
            Some(air_quality) => libertas_data_write(
                LOCAL_OUTDOOR_AIR_QUALITY_RESOURCE,
                singleton_key(),
                &BuildingHvacPersistentDataV1::LocalOutdoorAirQualityV1 { air_quality },
            ),
            None => libertas_data_remove(LOCAL_OUTDOOR_AIR_QUALITY_RESOURCE, singleton_key()),
        }
    }
    if changed {
        evaluate_and_publish(&context.shared);
    }
}

macro_rules! concentration_subscription {
    ($module:ident) => {{
        use libertas_matter::definitions::$module::attributes::{
            LevelValue, MeasuredValue, MeasurementMedium, MeasurementUnit,
        };
        let mut cluster = MatterSubscriptionCluster::<10, 0>::for_attribute::<MeasuredValue>(
            0,
            MATTER_SUBSCRIPTION_MAX_INTERVAL_SECONDS,
        );
        cluster
            .add_attribute::<MeasuredValue>()?
            .add_attribute::<MeasurementUnit>()?
            .add_attribute::<MeasurementMedium>()?
            .add_attribute::<LevelValue>()?;
        cluster
    }};
}

fn subscription_clusters(
    role: DeviceRole,
) -> Result<Vec<MatterSubscriptionCluster<10, 0>>, libertas_matter::error::Error> {
    let mut clusters = Vec::new();
    match role {
        DeviceRole::Thermostat(_) => {
            use Thermostat::attributes::{
                ControlSequenceOfOperation, MaxCoolSetpointLimit, MaxHeatSetpointLimit,
                MinCoolSetpointLimit, MinHeatSetpointLimit, MinSetpointDeadBand,
                OccupiedCoolingSetpoint, OccupiedHeatingSetpoint, ThermostatRunningMode,
                ThermostatRunningState,
            };
            let mut cluster = MatterSubscriptionCluster::<10, 0>::for_attribute::<
                OccupiedHeatingSetpoint,
            >(0, MATTER_SUBSCRIPTION_MAX_INTERVAL_SECONDS);
            cluster
                .add_attribute::<OccupiedHeatingSetpoint>()?
                .add_attribute::<OccupiedCoolingSetpoint>()?
                .add_attribute::<MinHeatSetpointLimit>()?
                .add_attribute::<MaxHeatSetpointLimit>()?
                .add_attribute::<MinCoolSetpointLimit>()?
                .add_attribute::<MaxCoolSetpointLimit>()?
                .add_attribute::<MinSetpointDeadBand>()?
                .add_attribute::<ControlSequenceOfOperation>()?
                .add_attribute::<ThermostatRunningMode>()?
                .add_attribute::<ThermostatRunningState>()?;
            clusters.push(cluster);
        }
        DeviceRole::IndoorTemperature { .. } | DeviceRole::OutdoorTemperature => {
            use TemperatureMeasurement::attributes::MeasuredValue;
            let mut cluster = MatterSubscriptionCluster::<10, 0>::for_attribute::<MeasuredValue>(
                0,
                MATTER_SUBSCRIPTION_MAX_INTERVAL_SECONDS,
            );
            cluster.add_attribute::<MeasuredValue>()?;
            clusters.push(cluster);
        }
        DeviceRole::IndoorHumidity { .. } | DeviceRole::OutdoorHumidity => {
            use RelativeHumidityMeasurement::attributes::MeasuredValue;
            let mut cluster = MatterSubscriptionCluster::<10, 0>::for_attribute::<MeasuredValue>(
                0,
                MATTER_SUBSCRIPTION_MAX_INTERVAL_SECONDS,
            );
            cluster.add_attribute::<MeasuredValue>()?;
            clusters.push(cluster);
        }
        DeviceRole::IndoorAirQuality { .. } | DeviceRole::OutdoorAirQuality => {
            use AirQuality::attributes::AirQuality as OverallAirQuality;
            let mut overall = MatterSubscriptionCluster::<10, 0>::for_attribute::<OverallAirQuality>(
                0,
                MATTER_SUBSCRIPTION_MAX_INTERVAL_SECONDS,
            );
            overall.add_attribute::<OverallAirQuality>()?;
            clusters.push(overall);
            clusters.push(concentration_subscription!(
                CarbonDioxideConcentrationMeasurement
            ));
            clusters.push(concentration_subscription!(
                CarbonMonoxideConcentrationMeasurement
            ));
            clusters.push(concentration_subscription!(
                NitrogenDioxideConcentrationMeasurement
            ));
            clusters.push(concentration_subscription!(OzoneConcentrationMeasurement));
            clusters.push(concentration_subscription!(PM1ConcentrationMeasurement));
            clusters.push(concentration_subscription!(PM25ConcentrationMeasurement));
            clusters.push(concentration_subscription!(PM10ConcentrationMeasurement));
            clusters.push(concentration_subscription!(
                FormaldehydeConcentrationMeasurement
            ));
            clusters.push(concentration_subscription!(
                TotalVolatileOrganicCompoundsConcentrationMeasurement
            ));
            clusters.push(concentration_subscription!(RadonConcentrationMeasurement));
        }
    }
    Ok(clusters)
}

fn configured_devices(state: &ControllerState) -> Vec<(LibertasDevice, DeviceRole)> {
    let mut devices = Vec::new();
    for (index, thermostat) in state.thermostats.iter().enumerate() {
        devices.push((
            thermostat.configuration.thermostat,
            DeviceRole::Thermostat(index),
        ));
    }
    for (room_index, room) in state.rooms.iter().enumerate() {
        for (sensor_index, sensor) in room.sensor_states.iter().enumerate() {
            devices.push((
                sensor.temperature_sensor,
                DeviceRole::IndoorTemperature {
                    room: room_index,
                    sensor: sensor_index,
                },
            ));
            if let Some(device) = sensor.humidity_sensor {
                devices.push((
                    device,
                    DeviceRole::IndoorHumidity {
                        room: room_index,
                        sensor: sensor_index,
                    },
                ));
            }
            if let Some(device) = sensor.air_quality_sensor {
                devices.push((
                    device,
                    DeviceRole::IndoorAirQuality {
                        room: room_index,
                        sensor: sensor_index,
                    },
                ));
            }
        }
    }
    if let Some(outdoor) = state.local_outdoor.as_ref() {
        let _ = outdoor;
    }
    devices
}

fn request_matter_subscriptions(
    shared: &Rc<RefCell<ControllerState>>,
    outdoor: Option<BuildingHvacOutdoorSensorV1>,
) {
    let mut devices = configured_devices(&shared.borrow());
    if let Some(outdoor) = outdoor {
        devices.push((outdoor.temperature_sensor, DeviceRole::OutdoorTemperature));
        if let Some(device) = outdoor.humidity_sensor {
            devices.push((device, DeviceRole::OutdoorHumidity));
        }
        if let Some(device) = outdoor.air_quality_sensor {
            devices.push((device, DeviceRole::OutdoorAirQuality));
        }
    }
    let builders: Result<Vec<_>, _> = devices
        .iter()
        .map(|(_, role)| subscription_clusters(*role))
        .collect();
    let Ok(builders) = builders else {
        libertas_log(
            LogLevel::Error,
            "Could not build the Matter HVAC subscription",
        );
        return;
    };
    let cluster_requests: Result<Vec<Vec<_>>, _> = builders
        .iter()
        .map(|clusters| clusters.iter().map(|cluster| cluster.request()).collect())
        .collect();
    let Ok(cluster_requests) = cluster_requests else {
        libertas_log(
            LogLevel::Error,
            "Could not encode the Matter HVAC subscription",
        );
        return;
    };
    let device_requests: Result<Vec<_>, _> = devices
        .iter()
        .zip(&cluster_requests)
        .map(|((device, _), clusters)| {
            MatterDeviceSubscription::new(MatterDevice::new(*device), clusters)
        })
        .collect();
    let Ok(device_requests) = device_requests else {
        libertas_log(
            LogLevel::Error,
            "Could not assemble the Matter HVAC subscription",
        );
        return;
    };
    match MatterSubscriptionBatch::new(&device_requests) {
        Ok(batch) => {
            batch.send();
        }
        Err(error) => libertas_log(
            LogLevel::Error,
            &format!("Matter HVAC subscription failed: {error}"),
        ),
    }
}

fn activity_text(activity: BuildingHvacRoomActivityV1) -> &'static str {
    match activity {
        BuildingHvacRoomActivityV1::Unknown => "unavailable",
        BuildingHvacRoomActivityV1::Idle => "idle",
        BuildingHvacRoomActivityV1::Heating => "heating",
        BuildingHvacRoomActivityV1::Cooling => "cooling",
        BuildingHvacRoomActivityV1::FanOnly => "fan only",
    }
}

fn air_quality_text(room: &RoomRuntime) -> &'static str {
    if room
        .state
        .sensor_states
        .iter()
        .any(|sensor| sensor.air_quality.is_some())
    {
        "sensor data available"
    } else {
        "unavailable"
    }
}

fn formatted_room_status(room: &RoomRuntime) -> Vec<u8> {
    let comfort = match room.state.data_quality {
        BuildingHvacRoomDataQualityV1::Ready => "ready",
        BuildingHvacRoomDataQualityV1::Degraded => "degraded",
        BuildingHvacRoomDataQualityV1::Unavailable => "unavailable",
    };
    libertas_formatted_text(
        "HVAC_ROOM_STATUS",
        &[
            NotificationArgument::LiteralText(comfort),
            NotificationArgument::LiteralText(activity_text(room.state.activity)),
            NotificationArgument::LiteralText(air_quality_text(room)),
        ],
    )
}

fn room_report(
    state: &ControllerState,
    room_index: usize,
    now: Option<LibertasDateTime>,
) -> BuildingHvacRoomProtocolV1 {
    let room = &state.rooms[room_index];
    let outdoor_air_analytics = state.weather.current.as_ref().and_then(|current| {
        now.and_then(|now| BuildingHvacAnalyticsEngine::new().analyze_outdoor_air(now, current))
    });
    BuildingHvacRoomProtocolV1::RoomDataV1 {
        formatted_room_status: formatted_room_status(room),
        maximum_wait_interval_seconds: BUILDING_HVAC_ROOM_MAXIMUM_WAIT_INTERVAL_SECONDS,
        control_revision: room.control_revision,
        control: room.control,
        state: Box::new(room.state.clone()),
        active_urgent_conditions: room.urgent.active_conditions(),
        local_outdoor_sensor: state.local_outdoor.clone().map(Box::new),
        outdoor_air_analytics,
        statistics: room.statistics.clone().map(Box::new),
        passive_outdoor_temperature_coupling_per_hour: room
            .learning
            .passive_outdoor_coupling
            .estimated_coefficient()
            .filter(|value| *value > 0.0)
            .map(|value| value as f32),
        passive_model_confidence_normalized: room
            .learning
            .passive_outdoor_coupling
            .confidence_normalized() as f32,
        cross_zone_influences: room.learning.runtime_influences(),
        machine_learning: room.machine_learning.clone(),
        plan: room.plan.clone().map(Box::new),
    }
}

fn report_changed_rooms(shared: &Rc<RefCell<ControllerState>>) {
    let now_ticks = libertas_get_sys_ticks();
    let now = libertas_get_utc_time();
    let mut reports = Vec::new();
    {
        let state = shared.borrow();
        for index in 0..state.rooms.len() {
            let report = room_report(&state, index, now);
            if state.rooms[index].last_report.as_ref() != Some(&report) {
                let peers: Vec<_> = state.rooms[index]
                    .subscribers
                    .iter()
                    .map(|subscriber| subscriber.peer)
                    .collect();
                reports.push((
                    index,
                    state.rooms[index].configuration.control_endpoint,
                    peers,
                    report,
                ));
            }
        }
    }
    for (index, endpoint, peers, report) in reports {
        for peer in peers {
            libertas_endpoint_report(endpoint, &report, Some(peer));
        }
        let mut state = shared.borrow_mut();
        state.rooms[index].last_report = Some(report);
        for subscriber in &mut state.rooms[index].subscribers {
            subscriber.last_report_ticks = now_ticks;
        }
    }
}

fn report_due_heartbeats(shared: &Rc<RefCell<ControllerState>>, now_ticks: u64) {
    let interval = u64::from(BUILDING_HVAC_ROOM_MAXIMUM_WAIT_INTERVAL_SECONDS)
        .saturating_mul(MICROSECONDS_PER_SECOND);
    let mut reports = Vec::new();
    let now = libertas_get_utc_time();
    {
        let state = shared.borrow();
        for (index, room) in state.rooms.iter().enumerate() {
            let due: Vec<_> = room
                .subscribers
                .iter()
                .filter(|subscriber| {
                    now_ticks.saturating_sub(subscriber.last_report_ticks) >= interval
                })
                .map(|subscriber| subscriber.peer)
                .collect();
            if !due.is_empty() {
                reports.push((
                    index,
                    room.configuration.control_endpoint,
                    due,
                    room_report(&state, index, now),
                ));
            }
        }
    }
    for (index, endpoint, peers, report) in reports {
        for peer in &peers {
            libertas_endpoint_report(endpoint, &report, Some(*peer));
        }
        let mut state = shared.borrow_mut();
        for subscriber in &mut state.rooms[index].subscribers {
            if peers.contains(&subscriber.peer) {
                subscriber.last_report_ticks = now_ticks;
            }
        }
    }
}

fn room_control_error(
    thermostat: &ThermostatRuntime,
    control: BuildingHvacRoomControlV1,
) -> Option<BuildingHvacRoomControlErrorV1> {
    if !control.is_well_formed() {
        return Some(
            if !control.comfort_or_savings_normalized.is_finite()
                || !(-1.0..=1.0).contains(&control.comfort_or_savings_normalized)
            {
                BuildingHvacRoomControlErrorV1::InvalidNormalizedPreference
            } else {
                BuildingHvacRoomControlErrorV1::InvalidTemperatureBand
            },
        );
    }
    let Some(limits) = thermostat.limits() else {
        return Some(BuildingHvacRoomControlErrorV1::TemporarilyUnavailable);
    };
    let heat_requested = matches!(
        control.operating_preference,
        BuildingHvacRoomOperatingPreferenceV1::Auto | BuildingHvacRoomOperatingPreferenceV1::Heat
    );
    let cool_requested = matches!(
        control.operating_preference,
        BuildingHvacRoomOperatingPreferenceV1::Auto | BuildingHvacRoomOperatingPreferenceV1::Cool
    );
    if (heat_requested && !thermostat.supports_heat())
        || (cool_requested && !thermostat.supports_cool())
    {
        return Some(BuildingHvacRoomControlErrorV1::UnsupportedOperatingPreference);
    }
    if control.preferred_heating_temperature_celsius < limits.minimum_heating_setpoint_celsius
        || control.preferred_heating_temperature_celsius > limits.maximum_heating_setpoint_celsius
        || control.preferred_cooling_temperature_celsius < limits.minimum_cooling_setpoint_celsius
        || control.preferred_cooling_temperature_celsius > limits.maximum_cooling_setpoint_celsius
        || control.preferred_cooling_temperature_celsius
            - control.preferred_heating_temperature_celsius
            < limits.minimum_deadband_celsius
    {
        return Some(BuildingHvacRoomControlErrorV1::InvalidTemperatureBand);
    }
    None
}

fn formatted_rejection(error: BuildingHvacRoomControlErrorV1) -> Vec<u8> {
    let resource = match error {
        BuildingHvacRoomControlErrorV1::RevisionConflict => "HVAC_CONTROL_REVISION_CONFLICT",
        BuildingHvacRoomControlErrorV1::InvalidTemperatureBand => {
            "HVAC_CONTROL_INVALID_TEMPERATURE_BAND"
        }
        BuildingHvacRoomControlErrorV1::InvalidNormalizedPreference => {
            "HVAC_CONTROL_INVALID_NORMALIZED_PREFERENCE"
        }
        BuildingHvacRoomControlErrorV1::UnsupportedOperatingPreference => {
            "HVAC_CONTROL_UNSUPPORTED_OPERATING_PREFERENCE"
        }
        BuildingHvacRoomControlErrorV1::TemporarilyUnavailable => {
            "HVAC_CONTROL_TEMPORARILY_UNAVAILABLE"
        }
    };
    libertas_formatted_text(resource, &[])
}

fn reject_control(
    endpoint: LibertasEndpoint,
    transaction_id: LibertasTransId,
    peer: u32,
    current_control_revision: u64,
    current_control: BuildingHvacRoomControlV1,
    error: BuildingHvacRoomControlErrorV1,
) {
    libertas_endpoint_response(
        endpoint,
        &BuildingHvacRoomProtocolV1::RoomControlRejectedV1 {
            formatted_rejection: formatted_rejection(error),
            error,
            current_control_revision,
            current_control,
        },
        transaction_id,
        peer,
    );
}

fn handle_room_endpoint(
    endpoint: LibertasEndpoint,
    opcode: u8,
    message: LibertasEndpointMessage<BuildingHvacRoomProtocolV1>,
    context: &mut Box<dyn Any>,
    transaction_id: LibertasTransId,
    peer: u32,
) -> LibertasEndpointHandlerResult {
    let context = context
        .downcast_mut::<RoomContext>()
        .expect("invalid smart building HVAC room context");
    if opcode == OP_ENDPOINT_PEER_DOWN {
        context.shared.borrow_mut().rooms[context.room_index]
            .subscribers
            .retain(|subscriber| subscriber.peer != peer);
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
    let subscription = opcode == OP_ENDPOINT_SUB_REQ;
    match message {
        BuildingHvacRoomProtocolV1::GetRoomV1 => {
            let now = libertas_get_utc_time();
            let response = room_report(&context.shared.borrow(), context.room_index, now);
            libertas_endpoint_response(endpoint, &response, transaction_id, peer);
            if subscription {
                let now_ticks = libertas_get_sys_ticks();
                let mut state = context.shared.borrow_mut();
                let room = &mut state.rooms[context.room_index];
                if let Some(subscriber) = room
                    .subscribers
                    .iter_mut()
                    .find(|subscriber| subscriber.peer == peer)
                {
                    subscriber.last_report_ticks = now_ticks;
                } else {
                    room.subscribers.push(Subscriber {
                        peer,
                        last_report_ticks: now_ticks,
                    });
                }
                room.last_report = Some(response);
            }
        }
        BuildingHvacRoomProtocolV1::ReplaceRoomControlV1 {
            expected_revision,
            control,
        } => {
            if subscription {
                return LibertasEndpointHandlerResult::Status(
                    LibertasEndpointStandardStatus::InvalidArgument,
                );
            }
            let (revision, thermostat_index) = {
                let state = context.shared.borrow();
                let room = &state.rooms[context.room_index];
                (room.control_revision, room.thermostat_index)
            };
            if expected_revision != revision {
                let current_control = context.shared.borrow().rooms[context.room_index].control;
                reject_control(
                    endpoint,
                    transaction_id,
                    peer,
                    revision,
                    current_control,
                    BuildingHvacRoomControlErrorV1::RevisionConflict,
                );
                return LibertasEndpointHandlerResult::Handled;
            }
            if let Some(error) = room_control_error(
                &context.shared.borrow().thermostats[thermostat_index],
                control,
            ) {
                let room = context.shared.borrow();
                let current = &room.rooms[context.room_index];
                let current_revision = current.control_revision;
                let current_control = current.control;
                drop(room);
                reject_control(
                    endpoint,
                    transaction_id,
                    peer,
                    current_revision,
                    current_control,
                    error,
                );
                return LibertasEndpointHandlerResult::Handled;
            }
            let Some(next_revision) = revision.checked_add(1) else {
                let current_control = context.shared.borrow().rooms[context.room_index].control;
                reject_control(
                    endpoint,
                    transaction_id,
                    peer,
                    revision,
                    current_control,
                    BuildingHvacRoomControlErrorV1::TemporarilyUnavailable,
                );
                return LibertasEndpointHandlerResult::Handled;
            };
            libertas_data_write(
                ROOM_CONTROL_RESOURCE,
                &room_key(endpoint),
                &BuildingHvacPersistentDataV1::RoomControlV1 {
                    control_revision: next_revision,
                    control,
                },
            );
            {
                let mut state = context.shared.borrow_mut();
                let room = &mut state.rooms[context.room_index];
                room.control_revision = next_revision;
                room.control = control;
                room.plan = None;
            }
            evaluate_and_publish(&context.shared);
            let now = libertas_get_utc_time();
            let response = room_report(&context.shared.borrow(), context.room_index, now);
            libertas_endpoint_response(endpoint, &response, transaction_id, peer);
        }
        BuildingHvacRoomProtocolV1::RoomDataV1 { .. }
        | BuildingHvacRoomProtocolV1::RoomControlRejectedV1 { .. } => {
            return LibertasEndpointHandlerResult::InvalidMessage;
        }
    }
    LibertasEndpointHandlerResult::Handled
}

fn upsert_history(
    history: &mut Option<BuildingHvacWeatherHistoryV1>,
    retrieved_at: LibertasDateTime,
    valid_until: LibertasDateTime,
    periods: Vec<BuildingHvacWeatherHistoryPeriodV1>,
) {
    let history = history.get_or_insert(BuildingHvacWeatherHistoryV1 {
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
    if history.periods.len() > 72 {
        history.periods.drain(..history.periods.len() - 72);
    }
}

fn upsert_forecast(
    forecast: &mut Option<BuildingHvacWeatherForecastV1>,
    retrieved_at: LibertasDateTime,
    valid_until: LibertasDateTime,
    periods: Vec<BuildingHvacWeatherForecastPeriodV1>,
) {
    let forecast = forecast.get_or_insert(BuildingHvacWeatherForecastV1 {
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
    forecast
        .periods
        .truncate(BUILDING_HVAC_MAX_ROOM_PLAN_PERIODS);
}

fn upsert_outdoor_air_quality(
    air_quality: &mut Option<BuildingHvacOutdoorAirQualityV1>,
    retrieved_at: LibertasDateTime,
    valid_until: LibertasDateTime,
    periods: Vec<BuildingHvacOutdoorAirQualityPeriodV1>,
) {
    let air_quality = air_quality.get_or_insert(BuildingHvacOutdoorAirQualityV1 {
        retrieved_at,
        valid_until,
        periods: Vec::new(),
    });
    air_quality.retrieved_at = retrieved_at;
    air_quality.valid_until = valid_until;
    for period in periods {
        if let Some(existing) = air_quality
            .periods
            .iter_mut()
            .find(|existing| existing.starts_at == period.starts_at)
        {
            *existing = period;
        } else {
            air_quality.periods.push(period);
        }
    }
    air_quality.periods.sort_by_key(|period| period.starts_at);
    air_quality.periods.truncate(48);
}

fn apply_weather_change(
    snapshot: &mut BuildingHvacWeatherSnapshotV1,
    change: BuildingHvacWeatherChangeV1,
) {
    match change {
        BuildingHvacWeatherChangeV1::HistoryPeriodsUpsertV1 {
            retrieved_at,
            valid_until,
            periods,
        } => upsert_history(&mut snapshot.history, retrieved_at, valid_until, periods),
        BuildingHvacWeatherChangeV1::HistoryPeriodsRemoveV1 { range } => {
            if let Some(history) = &mut snapshot.history {
                history.periods.retain(|period| {
                    !(range.starts_at..range.ends_before).contains(&period.starts_at)
                });
            }
        }
        BuildingHvacWeatherChangeV1::CurrentReplaceV1 { current } => {
            snapshot.current = Some(current);
        }
        BuildingHvacWeatherChangeV1::ForecastPeriodsUpsertV1 {
            retrieved_at,
            valid_until,
            periods,
        } => upsert_forecast(&mut snapshot.forecast, retrieved_at, valid_until, periods),
        BuildingHvacWeatherChangeV1::ForecastPeriodsRemoveV1 { range } => {
            if let Some(forecast) = &mut snapshot.forecast {
                forecast.periods.retain(|period| {
                    !(range.starts_at..range.ends_before).contains(&period.starts_at)
                });
            }
        }
        BuildingHvacWeatherChangeV1::OutdoorAirQualityPeriodsUpsertV1 {
            retrieved_at,
            valid_until,
            periods,
        } => upsert_outdoor_air_quality(
            &mut snapshot.outdoor_air_quality,
            retrieved_at,
            valid_until,
            periods,
        ),
        BuildingHvacWeatherChangeV1::OutdoorAirQualityPeriodsRemoveV1 { range } => {
            if let Some(air_quality) = &mut snapshot.outdoor_air_quality {
                air_quality.periods.retain(|period| {
                    !(range.starts_at..range.ends_before).contains(&period.starts_at)
                });
            }
        }
        BuildingHvacWeatherChangeV1::SectionClearV1 { section } => match section {
            BuildingHvacWeatherSectionV1::History => snapshot.history = None,
            BuildingHvacWeatherSectionV1::Current => snapshot.current = None,
            BuildingHvacWeatherSectionV1::Forecast => snapshot.forecast = None,
            BuildingHvacWeatherSectionV1::OutdoorAirQuality => {
                snapshot.outdoor_air_quality = None;
            }
        },
        BuildingHvacWeatherChangeV1::HistoryReplaceV1 { history } => {
            snapshot.history = Some(history);
        }
        BuildingHvacWeatherChangeV1::ForecastReplaceV1 { forecast } => {
            snapshot.forecast = Some(forecast);
        }
        BuildingHvacWeatherChangeV1::OutdoorAirQualityReplaceV1 {
            outdoor_air_quality,
        } => {
            snapshot.outdoor_air_quality = Some(outdoor_air_quality);
        }
    }
}

fn accept_weather_report(
    shared: &Rc<RefCell<ControllerState>>,
    report: BuildingHvacWeatherIncrementalReportV1,
) -> bool {
    let (cursor, previous) = {
        let state = shared.borrow();
        let Some(cursor) = state.weather_cursor else {
            return false;
        };
        (cursor, state.weather.clone())
    };
    if !report.can_apply_after(cursor) {
        return false;
    }
    let mut next = previous.clone();
    for change in report.changes {
        apply_weather_change(&mut next, change);
    }
    if !valid_weather_snapshot(&next) {
        return false;
    }
    persist_weather(&previous, &next);
    let mut state = shared.borrow_mut();
    state.weather = next;
    state.weather_cursor = Some(report.through_cursor);
    for room in &mut state.rooms {
        room.plan = None;
    }
    true
}

fn accept_weather_recovery(
    shared: &Rc<RefCell<ControllerState>>,
    recovery: BuildingHvacWeatherRecoveryV1,
) -> bool {
    match recovery {
        BuildingHvacWeatherRecoveryV1::ReplayedV1 { report } => {
            accept_weather_report(shared, report)
        }
        BuildingHvacWeatherRecoveryV1::ResetV1 {
            cursor, snapshot, ..
        } => {
            let (previous_cursor, previous) = {
                let state = shared.borrow();
                (state.weather_cursor, state.weather.clone())
            };
            let cursor_accepted = previous_cursor.is_none_or(|previous| {
                (cursor.epoch_timestamp == previous.epoch_timestamp
                    && cursor.sequence >= previous.sequence)
                    || cursor.is_server_reset_after(previous)
            });
            if !cursor_accepted || !valid_weather_snapshot(&snapshot) {
                return false;
            }
            persist_weather(&previous, &snapshot);
            let mut state = shared.borrow_mut();
            state.weather = snapshot;
            state.weather_cursor = Some(cursor);
            for room in &mut state.rooms {
                room.plan = None;
            }
            true
        }
        BuildingHvacWeatherRecoveryV1::ErrorV1 { .. } => false,
    }
}

fn weather_request(shared: &Rc<RefCell<ControllerState>>) -> BuildingHvacWeatherProtocolV1 {
    let now = libertas_get_utc_time();
    let state = shared.borrow();
    BuildingHvacWeatherProtocolV1::GetBuildingHvacWeatherV1 {
        after_cursor: state.weather_cursor,
        history_range: now.map(|now| BuildingHvacWeatherTimeRangeV1 {
            starts_at: now.saturating_sub(u64::from(BUILDING_HVAC_HISTORY_WINDOW_SECONDS)),
            ends_before: now,
        }),
        include_current: true,
        forecast_range: now.map(|now| BuildingHvacWeatherTimeRangeV1 {
            starts_at: now,
            ends_before: now.saturating_add(u64::from(BUILDING_HVAC_FORECAST_HORIZON_SECONDS)),
        }),
        outdoor_air_quality_range: now.map(|now| BuildingHvacWeatherTimeRangeV1 {
            starts_at: now,
            ends_before: now.saturating_add(u64::from(BUILDING_HVAC_AIR_QUALITY_HORIZON_SECONDS)),
        }),
    }
}

fn arm_weather_retry(shared: &Rc<RefCell<ControllerState>>, seconds: u32) {
    let timer = shared.borrow().weather_retry_timer;
    if timer != 0 {
        libertas_timer_update_interval(
            timer,
            absolute_ticks(libertas_get_sys_ticks(), seconds.max(1)),
        );
    }
}

fn subscribe_weather(shared: &Rc<RefCell<ControllerState>>) {
    let endpoint = shared.borrow().weather_endpoint;
    libertas_endpoint_subscribe_request(endpoint, &weather_request(shared));
    arm_weather_retry(shared, WEATHER_RETRY_SECONDS);
}

fn handle_weather_endpoint(
    _endpoint: LibertasEndpoint,
    opcode: u8,
    message: LibertasEndpointMessage<BuildingHvacWeatherProtocolV1>,
    context: &mut Box<dyn Any>,
    _transaction_id: LibertasTransId,
    _peer: u32,
) -> LibertasEndpointHandlerResult {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .expect("invalid smart building HVAC weather context");
    if opcode == OP_ENDPOINT_PEER_DOWN || opcode == OP_ENDPOINT_PEER_TIMEOUT {
        shared.borrow_mut().weather_stream_ready = false;
        arm_weather_retry(shared, WEATHER_RETRY_SECONDS);
        evaluate_and_publish(shared);
        return LibertasEndpointHandlerResult::Handled;
    }
    let mut retry_seconds = WEATHER_RETRY_SECONDS;
    let accepted = match (opcode, message) {
        (
            OP_ENDPOINT_RSP,
            LibertasEndpointMessage::Data(
                BuildingHvacWeatherProtocolV1::BuildingHvacWeatherRecoveryV1 {
                    maximum_wait_interval_seconds,
                    recovery,
                },
            ),
        ) if maximum_wait_interval_seconds != 0 => {
            if let BuildingHvacWeatherRecoveryV1::ErrorV1 {
                error,
                retry_after_seconds,
            } = &recovery
            {
                retry_seconds = retry_after_seconds.unwrap_or(WEATHER_RETRY_SECONDS).max(1);
                if *error == BuildingHvacWeatherRecoveryErrorV1::CursorAhead {
                    shared.borrow_mut().weather_cursor = None;
                }
                false
            } else {
                let accepted = accept_weather_recovery(shared, recovery);
                if accepted {
                    let mut state = shared.borrow_mut();
                    state.weather_maximum_wait_seconds = maximum_wait_interval_seconds;
                    state.weather_stream_ready = true;
                    drop(state);
                    arm_weather_retry(shared, maximum_wait_interval_seconds);
                }
                accepted
            }
        }
        (
            OP_ENDPOINT_DATA,
            LibertasEndpointMessage::Data(
                BuildingHvacWeatherProtocolV1::BuildingHvacWeatherIncrementV1 { report },
            ),
        ) => {
            let accepted = accept_weather_report(shared, report);
            if accepted {
                let wait = shared.borrow().weather_maximum_wait_seconds;
                shared.borrow_mut().weather_stream_ready = true;
                arm_weather_retry(shared, wait);
            }
            accepted
        }
        _ => false,
    };
    if accepted {
        evaluate_and_publish(shared);
    } else {
        shared.borrow_mut().weather_stream_ready = false;
        arm_weather_retry(shared, retry_seconds);
    }
    LibertasEndpointHandlerResult::Handled
}

fn outdoor_temperature(state: &ControllerState, now: LibertasDateTime) -> Option<f32> {
    state
        .local_outdoor
        .as_ref()
        .and_then(|outdoor| outdoor.temperature)
        .filter(|reading| reading.observed_at <= now && reading.valid_until > now)
        .map(|reading| reading.temperature_celsius)
        .or_else(|| {
            state
                .weather
                .current
                .as_ref()
                .filter(|current| current.is_fresh_at(now))
                .map(|current| current.conditions.dry_bulb_temperature_celsius)
        })
}

fn prediction_for(
    room: &RoomRuntime,
    horizon: BuildingHvacThermalPredictionHorizonV1,
) -> Option<f32> {
    room.machine_learning
        .predictions
        .iter()
        .find(|prediction| prediction.horizon == horizon)
        .map(|prediction| prediction.temperature_change_celsius)
}

fn predicted_cross_zone_change(state: &ControllerState, room: &RoomRuntime) -> f32 {
    room.learning
        .cross_zone_learners
        .iter()
        .filter_map(|learner| {
            let source = state.thermostats.iter().find(|thermostat| {
                thermostat.configuration.thermostat == learner.source_thermostat
            })?;
            let change_per_hour = match source.activity {
                BuildingHvacRoomActivityV1::Heating => learner
                    .heating
                    .estimated_coefficient()
                    .filter(|value| *value > 0.0),
                BuildingHvacRoomActivityV1::Cooling => learner
                    .cooling
                    .estimated_coefficient()
                    .filter(|value| *value > 0.0)
                    .map(|value| -value),
                _ => None,
            }?;
            Some((change_per_hour * 0.25) as f32)
        })
        .sum::<f32>()
        .clamp(-5.0, 5.0)
}

fn planned_control(room: &RoomRuntime, now: LibertasDateTime) -> BuildingHvacRoomControlV1 {
    let mut control = room.control;
    if let Some(period) = room.plan.as_ref().and_then(|plan| {
        plan.periods.iter().find(|period| {
            period.starts_at <= now
                && period
                    .starts_at
                    .saturating_add(u64::from(period.duration_seconds))
                    > now
        })
    }) {
        if let Some(heating) = period.heating_setpoint_celsius {
            control.preferred_heating_temperature_celsius = heating;
        }
        if let Some(cooling) = period.cooling_setpoint_celsius {
            control.preferred_cooling_temperature_celsius = cooling;
        }
    }
    control
}

fn apply_thermostat_decisions(shared: &Rc<RefCell<ControllerState>>) {
    let now = libertas_get_utc_time().unwrap_or_default();
    let decisions: Vec<_> = {
        let state = shared.borrow();
        state
            .thermostats
            .iter()
            .enumerate()
            .map(|(thermostat_index, thermostat)| {
                let room_indices: Vec<_> = state
                    .rooms
                    .iter()
                    .enumerate()
                    .filter(|(_, room)| room.thermostat_index == thermostat_index)
                    .map(|(index, _)| index)
                    .collect();
                let controls: Vec<_> = room_indices
                    .iter()
                    .map(|index| planned_control(&state.rooms[*index], now))
                    .collect();
                let candidates: Vec<_> = room_indices
                    .iter()
                    .zip(&controls)
                    .map(|(index, control)| {
                        let room = &state.rooms[*index];
                        BuildingHvacRoomControlCandidate {
                            room_endpoint: room.configuration.control_endpoint,
                            control,
                            state: &room.state,
                            predicted_cross_zone_temperature_change_celsius:
                                predicted_cross_zone_change(&state, room),
                            predicted_machine_learning_temperature_change_celsius: prediction_for(
                                room,
                                BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
                            ),
                        }
                    })
                    .collect();
                let decision = thermostat.limits().map(|limits| {
                    BuildingHvacControlEngine::new().arbitrate_thermostat(
                        thermostat.configuration.thermostat,
                        limits,
                        &candidates,
                    )
                });
                (thermostat_index, decision)
            })
            .collect()
    };
    for (thermostat_index, decision) in decisions {
        let Some(decision) = decision else {
            continue;
        };
        let BuildingHvacThermostatControlDecision::ApplySetpoints {
            heating_setpoint_celsius,
            cooling_setpoint_celsius,
            ..
        } = decision
        else {
            continue;
        };
        let (device, pending) = {
            let state = shared.borrow();
            let thermostat = &state.thermostats[thermostat_index];
            (
                thermostat.configuration.thermostat,
                thermostat.pending_write,
            )
        };
        if pending.is_some_and(|(_, heat, cool)| {
            heat == heating_setpoint_celsius && cool == cooling_setpoint_celsius
        }) {
            continue;
        }
        let mut buffer = InlineByteBuffer::new();
        let matter_device = MatterDevice::new(device);
        let Ok(mut batch) = matter_device.write_batch(&mut buffer) else {
            continue;
        };
        if let Some(value) = heating_setpoint_celsius.and_then(raw_setpoint)
            && batch
                .attribute(&Thermostat::attributes::OccupiedHeatingSetpoint(value))
                .is_err()
        {
            continue;
        }
        if let Some(value) = cooling_setpoint_celsius.and_then(raw_setpoint)
            && batch
                .attribute(&Thermostat::attributes::OccupiedCoolingSetpoint(value))
                .is_err()
        {
            continue;
        }
        match batch.send() {
            Ok(transaction_id) => {
                shared.borrow_mut().thermostats[thermostat_index].pending_write = Some((
                    transaction_id,
                    heating_setpoint_celsius,
                    cooling_setpoint_celsius,
                ));
            }
            Err(error) => libertas_log(
                LogLevel::Warn,
                &format!("Matter thermostat setpoint write failed: {error}"),
            ),
        }
    }
}

fn build_plan(
    room: &RoomRuntime,
    weather: &BuildingHvacWeatherSnapshotV1,
    now: LibertasDateTime,
) -> BuildingHvacRoomPlanV1 {
    let start = now - now % CONDITION_PERIOD_SECONDS;
    let mut periods = Vec::with_capacity(BUILDING_HVAC_MAX_ROOM_PLAN_PERIODS);
    for index in 0..BUILDING_HVAC_MAX_ROOM_PLAN_PERIODS {
        let starts_at = start.saturating_add(index as u64 * CONDITION_PERIOD_SECONDS);
        let forecast = weather
            .forecast
            .as_ref()
            .filter(|forecast| forecast.is_fresh_at(now))
            .and_then(|forecast| {
                forecast.periods.iter().find(|period| {
                    period.starts_at <= starts_at
                        && period
                            .starts_at
                            .saturating_add(u64::from(period.duration_seconds))
                            > starts_at
                })
            });
        let mut heating = matches!(
            room.control.operating_preference,
            BuildingHvacRoomOperatingPreferenceV1::Auto
                | BuildingHvacRoomOperatingPreferenceV1::Heat
        )
        .then_some(room.control.preferred_heating_temperature_celsius);
        let mut cooling = matches!(
            room.control.operating_preference,
            BuildingHvacRoomOperatingPreferenceV1::Auto
                | BuildingHvacRoomOperatingPreferenceV1::Cool
        )
        .then_some(room.control.preferred_cooling_temperature_celsius);
        let mut reason = if room.state.data_quality == BuildingHvacRoomDataQualityV1::Unavailable {
            BuildingHvacRoomPlanReasonV1::DegradedFallback
        } else {
            BuildingHvacRoomPlanReasonV1::RoomComfort
        };
        if let Some(forecast) = forecast {
            let outdoor = forecast.conditions.dry_bulb_temperature_celsius;
            if outdoor > room.control.preferred_cooling_temperature_celsius + 5.0
                && cooling.is_some()
            {
                cooling = cooling.map(|value| value - 0.5);
                reason = BuildingHvacRoomPlanReasonV1::WeatherPreconditioning;
            } else if outdoor < room.control.preferred_heating_temperature_celsius - 5.0
                && heating.is_some()
            {
                heating = heating.map(|value| value + 0.5);
                reason = BuildingHvacRoomPlanReasonV1::WeatherPreconditioning;
            }
        }
        periods.push(BuildingHvacRoomPlanPeriodV1 {
            starts_at,
            duration_seconds: CONDITION_PERIOD_SECONDS as u32,
            heating_setpoint_celsius: heating,
            cooling_setpoint_celsius: cooling,
            reason,
        });
    }
    let summary = format!(
        "{} periods through the next 24 hours",
        BUILDING_HVAC_MAX_ROOM_PLAN_PERIODS
    );
    BuildingHvacRoomPlanV1 {
        formatted_schedule: libertas_formatted_text(
            "HVAC_ROOM_SCHEDULE",
            &[NotificationArgument::LiteralText(&summary)],
        ),
        calculated_at: now,
        valid_until: now.saturating_add(CONDITION_PERIOD_SECONDS),
        periods,
    }
}

fn append_condition_periods(
    state: &mut ControllerState,
    now: LibertasDateTime,
) -> Vec<RoomPersistence> {
    let mut persistence = Vec::new();
    let boundary = now - now % CONDITION_PERIOD_SECONDS;
    let outdoors = outdoor_temperature(state, now);
    let activities: Vec<_> = state
        .thermostats
        .iter()
        .map(|thermostat| thermostat.activity)
        .collect();
    for room in &mut state.rooms {
        let Some(previous_boundary) = room.last_condition_boundary else {
            room.last_condition_boundary = Some(boundary);
            continue;
        };
        if boundary <= previous_boundary {
            continue;
        }
        let duration = boundary
            .saturating_sub(previous_boundary)
            .min(CONDITION_PERIOD_SECONDS);
        let period = BuildingHvacPersistedRoomConditionPeriodV1 {
            starts_at: boundary.saturating_sub(duration),
            duration_seconds: duration as u32,
            temperature_celsius: room.state.temperature_celsius,
            relative_humidity_percent: room.state.relative_humidity_percent,
            activity: room.state.activity,
            effective_heating_setpoint_celsius: room.state.effective_heating_setpoint_celsius,
            effective_cooling_setpoint_celsius: room.state.effective_cooling_setpoint_celsius,
            outdoor_dry_bulb_temperature_celsius: outdoors,
        };
        let previous = room.recent_conditions.last().copied();
        room.recent_conditions.push(period);
        if room.recent_conditions.len() > BUILDING_HVAC_MAX_PERSISTED_ROOM_CONDITION_PERIODS {
            room.recent_conditions.drain(
                ..room.recent_conditions.len() - BUILDING_HVAC_MAX_PERSISTED_ROOM_CONDITION_PERIODS,
            );
        }
        room.statistics =
            BuildingHvacAnalyticsEngine::new().summarize_conditions(&room.recent_conditions);
        room.last_condition_boundary = Some(boundary);

        if let Some(previous) = previous
            && let (Some(starting), Some(ending), Some(outdoor)) = (
                previous.temperature_celsius,
                period.temperature_celsius,
                previous.outdoor_dry_bulb_temperature_celsius,
            )
        {
            let change = ending - starting;
            let every_inactive = activities
                .iter()
                .all(|activity| *activity == BuildingHvacRoomActivityV1::Idle);
            let mut learned = room.learning.observe_identifiable_passive_period(
                boundary,
                every_inactive,
                period.duration_seconds,
                starting,
                outdoor,
                change,
                1.0,
            );
            let active_sources: Vec<_> = state
                .thermostats
                .iter()
                .enumerate()
                .filter(|(index, thermostat)| {
                    *index != room.thermostat_index
                        && matches!(
                            thermostat.activity,
                            BuildingHvacRoomActivityV1::Heating
                                | BuildingHvacRoomActivityV1::Cooling
                        )
                })
                .collect();
            if active_sources.len() == 1 {
                let (_, source) = active_sources[0];
                let passive = room
                    .learning
                    .predict_passive_temperature_change_celsius(
                        f64::from(outdoor - starting) * f64::from(period.duration_seconds)
                            / 3_600.0,
                    )
                    .unwrap_or(0.0) as f32;
                learned |= room.learning.observe_identifiable_cross_zone_period(
                    boundary,
                    room.state.physical_thermostat,
                    previous.activity,
                    source.configuration.thermostat,
                    source.activity,
                    1,
                    period.duration_seconds,
                    1.0,
                    change,
                    passive,
                    1.0,
                );
            }
            if learned {
                persistence.push(RoomPersistence {
                    resource: ROOM_LEARNING_RESOURCE,
                    endpoint: room.configuration.control_endpoint,
                    value: BuildingHvacPersistentDataV1::RoomLearningV1 {
                        learning: room.learning.clone(),
                    },
                });
            }
        }
        if let Some(statistics) = &room.statistics {
            persistence.push(RoomPersistence {
                resource: ROOM_STATISTICS_RESOURCE,
                endpoint: room.configuration.control_endpoint,
                value: BuildingHvacPersistentDataV1::RoomStatisticsV1 {
                    statistics: statistics.clone(),
                    recent_conditions: room.recent_conditions.clone(),
                },
            });
        }
    }
    persistence
}

fn cyclic_time(now: LibertasDateTime) -> (f32, f32, f32, f32) {
    let seconds_of_day = (now % 86_400) as f32;
    let day = ((now / 86_400) % 365) as f32;
    let hour_angle = TAU * seconds_of_day / 86_400.0;
    let day_angle = TAU * day / 365.0;
    (
        hour_angle.sin(),
        hour_angle.cos(),
        day_angle.sin(),
        day_angle.cos(),
    )
}

fn machine_learning_features(
    state: &ControllerState,
    room_index: usize,
    now: LibertasDateTime,
) -> Option<BuildingHvacMachineLearningFeaturesV1> {
    let room = &state.rooms[room_index];
    let room_temperature = room.state.temperature_celsius?;
    let current = state
        .weather
        .current
        .as_ref()
        .filter(|current| current.is_fresh_at(now));
    let analytics = current
        .and_then(|current| BuildingHvacAnalyticsEngine::new().analyze_outdoor_air(now, current));
    let (hour_sin, hour_cos, day_sin, day_cos) = cyclic_time(now);
    let own_activity = state.thermostats[room.thermostat_index].activity;
    let other_heating = state
        .thermostats
        .iter()
        .enumerate()
        .any(|(index, thermostat)| {
            index != room.thermostat_index
                && thermostat.activity == BuildingHvacRoomActivityV1::Heating
        });
    let other_cooling = state
        .thermostats
        .iter()
        .enumerate()
        .any(|(index, thermostat)| {
            index != room.thermostat_index
                && thermostat.activity == BuildingHvacRoomActivityV1::Cooling
        });
    let features = BuildingHvacMachineLearningFeaturesV1 {
        room_temperature_celsius: room_temperature,
        room_relative_humidity_percent: room.state.relative_humidity_percent,
        outdoor_temperature_celsius: outdoor_temperature(state, now),
        outdoor_humidity_ratio_kilograms_per_kilogram: analytics
            .map(|value| value.humidity_ratio_kilograms_water_per_kilogram_dry_air),
        outdoor_wind_speed_meters_per_second: current
            .map(|value| value.conditions.wind_speed_meters_per_second),
        global_horizontal_solar_irradiance_watts_per_square_meter: current.map(|value| {
            value
                .conditions
                .global_horizontal_irradiance_watts_per_square_meter
        }),
        hour_of_day_sine: hour_sin,
        hour_of_day_cosine: hour_cos,
        day_of_year_sine: day_sin,
        day_of_year_cosine: day_cos,
        own_heating_runtime_fraction: (own_activity == BuildingHvacRoomActivityV1::Heating) as u8
            as f32,
        own_cooling_runtime_fraction: (own_activity == BuildingHvacRoomActivityV1::Cooling) as u8
            as f32,
        other_zone_heating_runtime_fraction: other_heating as u8 as f32,
        other_zone_cooling_runtime_fraction: other_cooling as u8 as f32,
        heating_setpoint_offset_celsius: room
            .state
            .effective_heating_setpoint_celsius
            .map(|value| value - room_temperature),
        cooling_setpoint_offset_celsius: room
            .state
            .effective_cooling_setpoint_celsius
            .map(|value| value - room_temperature),
    };
    features.is_well_formed().then_some(features)
}

fn update_machine_learning_samples(
    state: &mut ControllerState,
    now: LibertasDateTime,
) -> Vec<BuildingHvacMachineLearningSampleV1> {
    let mut completed = Vec::new();
    for room_index in 0..state.rooms.len() {
        let Some(features) = machine_learning_features(state, room_index, now) else {
            continue;
        };
        let current_temperature = features.room_temperature_celsius;
        let endpoint = state.rooms[room_index].configuration.control_endpoint;
        for pending in &mut state.rooms[room_index].pending_features {
            let elapsed = now.saturating_sub(pending.observed_at);
            let change = (current_temperature - pending.temperature_celsius).clamp(
                -BUILDING_HVAC_ML_MAXIMUM_PREDICTED_CHANGE_CELSIUS,
                BUILDING_HVAC_ML_MAXIMUM_PREDICTED_CHANGE_CELSIUS,
            );
            let mut sample = BuildingHvacMachineLearningSampleV1 {
                observed_at: pending.observed_at,
                room_endpoint: endpoint,
                features: pending.features,
                temperature_change_15_minutes_celsius: None,
                temperature_change_30_minutes_celsius: None,
                temperature_change_60_minutes_celsius: None,
            };
            if elapsed >= 15 * 60 && !pending.persisted_15 {
                sample.temperature_change_15_minutes_celsius = Some(change);
                pending.persisted_15 = true;
            }
            if elapsed >= 30 * 60 && !pending.persisted_30 {
                sample.temperature_change_30_minutes_celsius = Some(change);
                pending.persisted_30 = true;
            }
            if elapsed >= 60 * 60 && !pending.persisted_60 {
                sample.temperature_change_60_minutes_celsius = Some(change);
                pending.persisted_60 = true;
            }
            if sample.is_well_formed() {
                completed.push(sample);
            }
        }
        state.rooms[room_index]
            .pending_features
            .retain(|pending| !pending.persisted_60);
        state.rooms[room_index]
            .pending_features
            .push(PendingFeatures {
                observed_at: now,
                temperature_celsius: current_temperature,
                features,
                persisted_15: false,
                persisted_30: false,
                persisted_60: false,
            });
        if state.rooms[room_index].pending_features.len() > MAX_ML_PENDING_FEATURES {
            let remove_count =
                state.rooms[room_index].pending_features.len() - MAX_ML_PENDING_FEATURES;
            state.rooms[room_index]
                .pending_features
                .drain(..remove_count);
        }
    }
    completed
}

fn request_predictions(state: &mut ControllerState, now: LibertasDateTime) {
    if state.rooms.is_empty() {
        return;
    }
    let horizons = [
        BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
        BuildingHvacThermalPredictionHorizonV1::ThirtyMinutes,
        BuildingHvacThermalPredictionHorizonV1::SixtyMinutes,
    ];
    for _ in 0..6 {
        let room_index = state.next_prediction_room % state.rooms.len();
        state.next_prediction_room = state.next_prediction_room.wrapping_add(1);
        let Some(features) = machine_learning_features(state, room_index, now) else {
            continue;
        };
        let horizon = horizons[state.next_prediction_request_id as usize % horizons.len()];
        let request_id = state.next_prediction_request_id;
        state.next_prediction_request_id = state.next_prediction_request_id.wrapping_add(1);
        if state
            .machine_learning_client
            .try_predict(
                request_id,
                state.rooms[room_index].configuration.control_endpoint,
                horizon,
                features,
            )
            .is_err()
        {
            break;
        }
    }
}

fn select_training_room(
    state: &mut ControllerState,
    now: LibertasDateTime,
) -> Option<LibertasEndpoint> {
    if state.rooms.is_empty() {
        return None;
    }
    let mut selected = None;
    for _ in 0..state.rooms.len() {
        let room_index = state.next_training_room % state.rooms.len();
        state.next_training_room = state.next_training_room.wrapping_add(1);
        if state.rooms[room_index]
            .last_training_at
            .is_none_or(|last| now.saturating_sub(last) >= ML_TRAINING_INTERVAL_SECONDS)
        {
            selected = Some(room_index);
            break;
        }
    }
    let room_index = selected?;
    let endpoint = state.rooms[room_index].configuration.control_endpoint;
    state.rooms[room_index].last_training_at = Some(now);
    Some(endpoint)
}

fn queue_training(
    client: &BuildingHvacMachineLearningClient,
    endpoint: LibertasEndpoint,
    now: LibertasDateTime,
) {
    let samples = BuildingHvacMachineLearningHistory::load_recent_samples(
        endpoint,
        now,
        BUILDING_HVAC_ML_MAXIMUM_TRAINING_SAMPLES_PER_ROOM,
    );
    if samples.len() < BUILDING_HVAC_ML_MINIMUM_TRAINING_SAMPLES {
        return;
    }
    for horizon in [
        BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
        BuildingHvacThermalPredictionHorizonV1::ThirtyMinutes,
        BuildingHvacThermalPredictionHorizonV1::SixtyMinutes,
    ] {
        if client.try_train(horizon, now, samples.clone()).is_err() {
            break;
        }
    }
}

fn evaluate_and_publish(shared: &Rc<RefCell<ControllerState>>) {
    let Some(now) = libertas_get_utc_time() else {
        return;
    };
    let (persistence, urgent_submissions, samples, training, client, recipients) = {
        let mut state = shared.borrow_mut();
        let analytics = BuildingHvacAnalyticsEngine::new();
        for room_index in 0..state.rooms.len() {
            let thermostat_index = state.rooms[room_index].thermostat_index;
            let thermostat = &state.thermostats[thermostat_index];
            let operating_preference = state.rooms[room_index].control.operating_preference;
            let heating_setpoint = matches!(
                operating_preference,
                BuildingHvacRoomOperatingPreferenceV1::Auto
                    | BuildingHvacRoomOperatingPreferenceV1::Heat
            )
            .then_some(thermostat.heating_setpoint_celsius)
            .flatten();
            let cooling_setpoint = matches!(
                operating_preference,
                BuildingHvacRoomOperatingPreferenceV1::Auto
                    | BuildingHvacRoomOperatingPreferenceV1::Cool
            )
            .then_some(thermostat.cooling_setpoint_celsius)
            .flatten();
            let next = analytics.analyze_room(
                now,
                thermostat.configuration.thermostat,
                thermostat.observed_at,
                thermostat.valid_until,
                thermostat.activity,
                heating_setpoint,
                cooling_setpoint,
                &state.rooms[room_index].sensor_states,
            );
            state.rooms[room_index].state = next;
            if state.rooms[room_index].state.temperature_celsius.is_none() {
                state.rooms[room_index].machine_learning.predictions.clear();
            }
        }
        let persistence = append_condition_periods(&mut state, now);
        let recipients = state.recipients.clone();
        let weather = state.weather.clone();
        let mut urgent_submissions = Vec::new();
        for room in &mut state.rooms {
            let evaluation = room
                .urgent
                .evaluate(now, &room.state, &room.recent_conditions);
            if evaluation.state_changed() {
                urgent_submissions.push(UrgentSubmission {
                    endpoint: room.configuration.control_endpoint,
                    room_name: room.configuration.name.clone(),
                    engine: room.urgent.clone(),
                    evaluation,
                });
            }
            if room
                .plan
                .as_ref()
                .is_none_or(|plan| now >= plan.valid_until)
            {
                room.plan = Some(build_plan(room, &weather, now));
            }
        }
        let sample_boundary = now - now % CONDITION_PERIOD_SECONDS;
        let samples = if state.last_ml_sample_boundary != Some(sample_boundary) {
            let samples = update_machine_learning_samples(&mut state, now);
            state.last_ml_sample_boundary = Some(sample_boundary);
            samples
        } else {
            Vec::new()
        };
        let prediction_minute = now - now % 60;
        if state.last_prediction_minute != Some(prediction_minute) {
            request_predictions(&mut state, now);
            state.last_prediction_minute = Some(prediction_minute);
        }
        let training = select_training_room(&mut state, now);
        let client = state.machine_learning_client.clone();
        (
            persistence,
            urgent_submissions,
            samples,
            training,
            client,
            recipients,
        )
    };
    for write in persistence {
        libertas_data_write(write.resource, &room_key(write.endpoint), &write.value);
    }
    for submission in urgent_submissions {
        submission.engine.persist_and_submit(
            submission.endpoint,
            &submission.room_name,
            &recipients,
            &submission.evaluation,
        );
    }
    for sample in samples {
        if BuildingHvacMachineLearningHistory::persist_sample(now, sample).is_err() {
            libertas_log(LogLevel::Warn, "Could not persist an HVAC learning sample");
        }
    }
    if let Some(endpoint) = training {
        queue_training(&client, endpoint, now);
    }
    apply_thermostat_decisions(shared);
    report_changed_rooms(shared);
}

fn handle_wakeup(context: &mut Box<dyn Any>) {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .expect("invalid smart building HVAC wake-up context");
    let mut changed = false;
    loop {
        let result = shared.borrow_mut().machine_learning_results.try_recv();
        let Ok(result) = result else {
            break;
        };
        match result {
            BuildingHvacMachineLearningResult::Candidate(candidate) => {
                let Some((index, mut updated, client)) = ({
                    let state = shared.borrow();
                    state
                        .model_sets
                        .iter()
                        .position(|models| models.room_endpoint == candidate.room_endpoint)
                        .map(|index| {
                            (
                                index,
                                state.model_sets[index].clone(),
                                state.machine_learning_client.clone(),
                            )
                        })
                }) else {
                    continue;
                };
                if !updated.promote(candidate.clone()) {
                    continue;
                }
                libertas_data_write(
                    BUILDING_HVAC_ML_MODELS_RESOURCE,
                    &room_key(updated.room_endpoint),
                    &BuildingHvacPersistentDataV1::MachineLearningModelsV1 {
                        models: updated.clone(),
                    },
                );
                shared.borrow_mut().model_sets[index] = updated;
                if client.try_activate(candidate).is_err() {
                    libertas_log(
                        LogLevel::Warn,
                        "Persisted HVAC model could not be activated until restart",
                    );
                }
            }
            BuildingHvacMachineLearningResult::TrainingRejected { horizon, reason } => {
                libertas_log(
                    LogLevel::Warn,
                    &format!("HVAC XGBoost training rejected for {horizon:?}: {reason:?}"),
                );
            }
            BuildingHvacMachineLearningResult::Prediction {
                room_endpoint,
                prediction,
                ..
            } => {
                let mut state = shared.borrow_mut();
                let Some(room) = state
                    .rooms
                    .iter_mut()
                    .find(|room| room.configuration.control_endpoint == room_endpoint)
                else {
                    continue;
                };
                if let Some(existing) = room
                    .machine_learning
                    .predictions
                    .iter_mut()
                    .find(|existing| existing.horizon == prediction.horizon)
                {
                    changed |= *existing != prediction;
                    *existing = prediction;
                } else if room.machine_learning.predictions.len() < 3 {
                    room.machine_learning.predictions.push(prediction);
                    changed = true;
                }
                room.machine_learning
                    .predictions
                    .sort_by_key(|prediction| prediction.horizon.seconds());
            }
        }
    }
    if changed {
        evaluate_and_publish(shared);
    }
}

fn handle_shutdown(context: &mut Box<dyn Any>) {
    let context = context
        .downcast_mut::<ShutdownContext>()
        .expect("invalid smart building HVAC shutdown context");
    if matches!(
        context.client.request_shutdown(),
        Err(BuildingHvacMachineLearningQueueError::Disconnected)
    ) {
        libertas_shutdown_complete();
    }
}

fn weather_retry_timer(timer: u32, now_ticks: u64, context: &mut Box<dyn Any>) {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .expect("invalid smart building HVAC weather timer context");
    let endpoint = shared.borrow().weather_endpoint;
    libertas_endpoint_subscribe_request(endpoint, &weather_request(shared));
    libertas_timer_update_interval(timer, absolute_ticks(now_ticks, WEATHER_RETRY_SECONDS));
}

fn evaluation_timer(timer: u32, now_ticks: u64, context: &mut Box<dyn Any>) {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .expect("invalid smart building HVAC evaluation timer context");
    evaluate_and_publish(shared);
    report_due_heartbeats(shared, now_ticks);
    let now = libertas_get_utc_time();
    let stale_ticks = MATTER_READING_FRESHNESS_SECONDS.saturating_mul(MICROSECONDS_PER_SECOND);
    let resubscribe = {
        let state = shared.borrow();
        state.thermostats.iter().any(|thermostat| {
            thermostat
                .last_report_ticks
                .is_none_or(|last| now_ticks.saturating_sub(last) >= stale_ticks)
        }) || now.is_none_or(|now| {
            state.rooms.iter().any(|room| {
                room.sensor_states.iter().any(|sensor| {
                    sensor.temperature.is_none_or(|reading| {
                        reading.observed_at > now || reading.valid_until <= now
                    })
                })
            }) || state.outdoor_configuration.is_some()
                && state
                    .local_outdoor
                    .as_ref()
                    .and_then(|outdoor| outdoor.temperature)
                    .is_none_or(|reading| reading.observed_at > now || reading.valid_until <= now)
        })
    };
    if resubscribe {
        let outdoor = shared.borrow().outdoor_configuration;
        request_matter_subscriptions(shared, outdoor);
    }
    libertas_timer_update_interval(
        timer,
        absolute_ticks(now_ticks, EVALUATION_INTERVAL_SECONDS),
    );
}

fn register_devices(
    shared: &Rc<RefCell<ControllerState>>,
    outdoor: Option<BuildingHvacOutdoorSensorV1>,
) {
    let mut devices = configured_devices(&shared.borrow());
    if let Some(outdoor) = outdoor {
        devices.push((outdoor.temperature_sensor, DeviceRole::OutdoorTemperature));
        if let Some(device) = outdoor.humidity_sensor {
            devices.push((device, DeviceRole::OutdoorHumidity));
        }
        if let Some(device) = outdoor.air_quality_sensor {
            devices.push((device, DeviceRole::OutdoorAirQuality));
        }
    }
    for (device, role) in devices {
        libertas_register_device_listener(
            device,
            handle_device_event,
            Box::new(DeviceContext {
                shared: Rc::clone(shared),
                role,
            }),
        );
    }
}

pub(super) fn start(
    building: BuildingHvacBuildingV1,
    weather: BuildingHvacWeatherClientV1,
    machine_learning_client: BuildingHvacMachineLearningClient,
    machine_learning_results: Receiver<BuildingHvacMachineLearningResult>,
    model_sets: Vec<BuildingHvacMachineLearningModelSetV1>,
    active_models: Vec<BuildingHvacMachineLearningModelV1>,
) {
    let thermostats: Vec<_> = building
        .thermostats
        .iter()
        .cloned()
        .map(ThermostatRuntime::new)
        .collect();
    let configured_thermostats: Vec<_> = building
        .thermostats
        .iter()
        .map(|thermostat| thermostat.thermostat)
        .collect();
    let mut rooms = Vec::with_capacity(building.rooms.len());
    for (room_index, configuration) in building.rooms.iter().cloned().enumerate() {
        let Some((thermostat_index, association)) = room_association(&building, room_index) else {
            libertas_log(LogLevel::Error, "HVAC room has no thermostat association");
            return;
        };
        let sensors = restored_sensor_states(configuration.control_endpoint, &association.sensors);
        let (control_revision, control) = restore_control(configuration.control_endpoint);
        let (recent_conditions, statistics) = restore_room_history(configuration.control_endpoint);
        let state = initial_state(
            building.thermostats[thermostat_index].thermostat,
            sensors.clone(),
        );
        rooms.push(RoomRuntime {
            configuration,
            thermostat_index,
            sensor_states: sensors,
            control_revision,
            control,
            state,
            recent_conditions,
            statistics,
            learning: restore_learning(
                building.rooms[room_index].control_endpoint,
                building.thermostats[thermostat_index].thermostat,
                &configured_thermostats,
            ),
            urgent: restore_urgent(building.rooms[room_index].control_endpoint),
            machine_learning: BuildingHvacRoomMachineLearningV1::default(),
            plan: None,
            subscribers: Vec::new(),
            last_report: None,
            last_condition_boundary: None,
            pending_features: Vec::new(),
            last_training_at: None,
        });
    }
    let outdoor_configuration = building.outdoor_sensor;
    let shared = Rc::new(RefCell::new(ControllerState {
        recipients: building.urgent_notification_recipients.clone(),
        weather_endpoint: weather.endpoint,
        weather: restore_weather(),
        weather_cursor: None,
        weather_stream_ready: false,
        weather_maximum_wait_seconds: BUILDING_HVAC_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
        weather_retry_timer: 0,
        local_outdoor: restore_local_outdoor(outdoor_configuration),
        thermostats,
        rooms,
        air_drafts: build_air_drafts(&building),
        machine_learning_client: machine_learning_client.clone(),
        machine_learning_results,
        model_sets,
        next_prediction_request_id: 1,
        next_prediction_room: 0,
        next_training_room: 0,
        last_ml_sample_boundary: None,
        last_prediction_minute: None,
        outdoor_configuration,
    }));

    register_devices(&shared, outdoor_configuration);
    for room_index in 0..shared.borrow().rooms.len() {
        let endpoint = shared.borrow().rooms[room_index]
            .configuration
            .control_endpoint;
        libertas_register_endpoint_status_listener::<BuildingHvacRoomProtocolV1, _>(
            endpoint,
            handle_room_endpoint,
            Box::new(RoomContext {
                shared: Rc::clone(&shared),
                room_index,
            }),
        );
    }
    libertas_register_endpoint_status_listener::<BuildingHvacWeatherProtocolV1, _>(
        weather.endpoint,
        handle_weather_endpoint,
        Box::new(Rc::clone(&shared)),
    );
    libertas_register_wakeup_callback(handle_wakeup, Box::new(Rc::clone(&shared)));
    libertas_register_shutdown_handler(
        handle_shutdown,
        Box::new(ShutdownContext {
            client: machine_learning_client.clone(),
        }),
    );
    for model in active_models {
        if machine_learning_client.try_activate(model).is_err() {
            libertas_log(
                LogLevel::Warn,
                "Could not queue a restored HVAC XGBoost model for activation",
            );
        }
    }

    let weather_timer =
        libertas_timer_new_interval(0, weather_retry_timer, Box::new(Rc::clone(&shared)));
    shared.borrow_mut().weather_retry_timer = weather_timer;
    let now_ticks = libertas_get_sys_ticks();
    libertas_timer_new_interval(
        absolute_ticks(now_ticks, EVALUATION_INTERVAL_SECONDS),
        evaluation_timer,
        Box::new(Rc::clone(&shared)),
    );
    request_matter_subscriptions(&shared, outdoor_configuration);
    subscribe_weather(&shared);
    evaluate_and_publish(&shared);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outdoor_conditions() -> BuildingHvacOutdoorConditionsV1 {
        BuildingHvacOutdoorConditionsV1 {
            dry_bulb_temperature_celsius: 30.0,
            dew_point_temperature_celsius: 18.0,
            relative_humidity_percent: 49,
            surface_pressure_hectopascals: 1_000.0,
            wind_speed_meters_per_second: 2.0,
            wind_gust_meters_per_second: 4.0,
            wind_direction_degrees: 180,
            precipitation_millimeters: 0.0,
            precipitation_kind: BuildingHvacPrecipitationKindV1::None,
            global_horizontal_irradiance_watts_per_square_meter: 500.0,
            direct_normal_irradiance_watts_per_square_meter: 600.0,
            diffuse_horizontal_irradiance_watts_per_square_meter: 100.0,
        }
    }

    #[test]
    fn raw_matter_setpoints_round_to_hundredths() {
        assert_eq!(raw_setpoint(20.125), Some(2013));
        assert_eq!(setpoint_celsius(2013), 20.13);
        assert_eq!(raw_setpoint(f32::NAN), None);
    }

    #[test]
    fn concentration_maps_preserve_standard_units_and_levels() {
        assert_eq!(
            map_concentration_unit(4),
            Some(BuildingHvacAirMeasurementUnitV1::MicrogramsPerCubicMeter)
        );
        assert_eq!(
            map_concentration_level(4),
            Some(BuildingHvacConcentrationLevelV1::Critical)
        );
        assert_eq!(map_concentration_unit(8), None);
    }

    #[test]
    fn weather_reset_requires_newer_epoch_with_backward_sequence() {
        let previous = BuildingHvacWeatherCursorV1 {
            epoch_timestamp: 100,
            sequence: 20,
        };
        assert!(
            BuildingHvacWeatherCursorV1 {
                epoch_timestamp: 101,
                sequence: 3,
            }
            .is_server_reset_after(previous)
        );
        assert!(
            !BuildingHvacWeatherCursorV1 {
                epoch_timestamp: 100,
                sequence: 3,
            }
            .is_server_reset_after(previous)
        );
    }

    #[test]
    fn weather_sections_validate_independently() {
        let current = BuildingHvacCurrentWeatherV1 {
            retrieved_at: 100,
            valid_until: 200,
            valid_at: 100,
            interval_seconds: 900,
            conditions: outdoor_conditions(),
        };
        let invalid_forecast = BuildingHvacWeatherForecastV1 {
            retrieved_at: 100,
            valid_until: 200,
            periods: vec![BuildingHvacWeatherForecastPeriodV1 {
                starts_at: 100,
                duration_seconds: 0,
                precipitation_probability_percent: 0,
                conditions: outdoor_conditions(),
            }],
        };
        assert!(valid_current_weather(&current));
        assert!(!valid_weather_forecast(&invalid_forecast));
        assert!(!valid_weather_snapshot(&BuildingHvacWeatherSnapshotV1 {
            history: None,
            current: Some(current),
            forecast: Some(invalid_forecast),
            outdoor_air_quality: None,
        }));
    }

    #[test]
    fn air_measurement_requires_value_unit_and_air_medium() {
        let mut draft = AirDeviceDraft::new(1);
        let kind = BuildingHvacAirMeasurementKindV1::ParticulateMatter2_5;
        let concentration = &mut draft.concentrations[air_kind_index(kind)];
        concentration.kind = Some(kind);
        concentration.value = Some(12.0);
        concentration.unit = Some(BuildingHvacAirMeasurementUnitV1::MicrogramsPerCubicMeter);
        concentration.medium_is_air = Some(false);
        assert!(draft.reading(100).is_none());
        draft.concentrations[air_kind_index(kind)].medium_is_air = Some(true);
        let reading = draft.reading(100).expect("complete air measurement");
        assert_eq!(reading.measurements.len(), 1);
        assert_eq!(reading.measurements[0].kind, kind);
    }
}
