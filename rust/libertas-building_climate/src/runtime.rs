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
    LibertasTransId, OP_ENDPOINT_DATA, OP_ENDPOINT_PEER_DOWN, OP_ENDPOINT_PEER_UP, OP_ENDPOINT_REQ,
    OP_ENDPOINT_RSP, OP_ENDPOINT_SUB_REQ, libertas_data_remove, libertas_endpoint_report,
    libertas_endpoint_response, libertas_endpoint_subscribe_request, libertas_formatted_text,
    libertas_get_sys_ticks, libertas_get_utc_time, libertas_register_device_listener,
    libertas_register_endpoint_status_listener, libertas_register_shutdown_handler,
    libertas_register_wakeup_callback, libertas_timer_cancel, libertas_timer_new_interval,
    libertas_timer_update_interval,
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
const EXTERNAL_FEATURE_RETRY_SECONDS: u32 = 60;
const ML_TRAINING_INTERVAL_SECONDS: u64 = 24 * 60 * 60;
const ML_TARGET_MAXIMUM_DELAY_SECONDS: u64 = 2 * 60;
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
const EXTERNAL_FEATURE_INPUTS_RESOURCE: &str = "HVAC_EXTERNAL_FEATURE_INPUTS";

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
    local_temperature_celsius: Option<f32>,
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
            local_temperature_celsius: None,
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
    features: BuildingHvacMachineLearningFeatureVectorV1,
    predicted_change_15_minutes_celsius: Option<f32>,
    predicted_change_30_minutes_celsius: Option<f32>,
    predicted_change_60_minutes_celsius: Option<f32>,
    persisted_15: bool,
    persisted_30: bool,
    persisted_60: bool,
}

#[derive(Clone, Copy)]
struct PredictionResidualObservation {
    observed_at: LibertasDateTime,
    horizon: BuildingHvacThermalPredictionHorizonV1,
    residual_celsius: f32,
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
    last_report: Option<BuildingHvacRoomProtocolV1>,
    last_endpoint_report_ticks: Option<u64>,
    last_condition_boundary: Option<LibertasDateTime>,
    pending_features: Vec<PendingFeatures>,
    prediction_residuals: Vec<PredictionResidualObservation>,
    last_training_at: Option<LibertasDateTime>,
}

struct ControllerState {
    recipients: Vec<LibertasUser>,
    weather_endpoint: LibertasEndpoint,
    weather: BuildingHvacWeatherSnapshotV1,
    weather_cursor: Option<BuildingHvacWeatherCursorV1>,
    weather_stream_ready: bool,
    weather_server_up: bool,
    weather_maximum_wait_seconds: u32,
    weather_retry_timer: u32,
    external_feature_endpoint: Option<LibertasEndpoint>,
    external_feature_server_up: bool,
    external_features: BuildingHvacExternalFeatureSnapshotV1,
    external_feature_maximum_wait_seconds: u32,
    external_feature_retry_timer: u32,
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
    feature_history: Vec<BuildingFeatureObservation>,
    outdoor_configuration: Option<BuildingHvacOutdoorSensorV1>,
}

#[derive(Clone)]
struct ThermostatFeatureObservation {
    thermostat: LibertasDevice,
    activity: BuildingHvacRoomActivityV1,
    local_temperature_celsius: Option<f32>,
    heating_setpoint_celsius: Option<f32>,
    cooling_setpoint_celsius: Option<f32>,
    active_setpoint_delta_celsius: f32,
    write_pending: bool,
}

#[derive(Clone)]
struct RoomFeatureObservation {
    endpoint: LibertasEndpoint,
    temperature_celsius: Option<f32>,
    relative_humidity_percent: Option<f32>,
    effective_heating_setpoint_celsius: Option<f32>,
    effective_cooling_setpoint_celsius: Option<f32>,
    activity: BuildingHvacRoomActivityV1,
}

#[derive(Clone)]
struct BuildingFeatureObservation {
    observed_at: LibertasDateTime,
    thermostats: Vec<ThermostatFeatureObservation>,
    rooms: Vec<RoomFeatureObservation>,
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
        && value.solar_elevation_degrees.is_finite()
        && (-90.0..=90.0).contains(&value.solar_elevation_degrees)
        && value.solar_azimuth_degrees.is_finite()
        && (0.0..=360.0).contains(&value.solar_azimuth_degrees)
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

fn restore_external_features() -> BuildingHvacExternalFeatureSnapshotV1 {
    match libertas_data_read(EXTERNAL_FEATURE_INPUTS_RESOURCE, singleton_key()) {
        Some(BuildingHvacPersistentDataV1::ExternalFeatureInputsV1 { snapshot })
            if snapshot.is_well_formed() =>
        {
            snapshot
        }
        _ => BuildingHvacExternalFeatureSnapshotV1 {
            retrieved_at: 0,
            inputs: Vec::new(),
        },
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
        ControlSequenceOfOperation, LocalTemperature, MaxCoolSetpointLimit, MaxHeatSetpointLimit,
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
    decode_scalar!(LocalTemperature, LocalTemperature(value) => {
        thermostat.local_temperature_celsius = value.into_option().map(setpoint_celsius);
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
        .expect("invalid building climate Matter context");
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
        let mut cluster = MatterSubscriptionCluster::<11, 0>::for_attribute::<MeasuredValue>(
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
) -> Result<Vec<MatterSubscriptionCluster<11, 0>>, libertas_matter::error::Error> {
    let mut clusters = Vec::new();
    match role {
        DeviceRole::Thermostat(_) => {
            use Thermostat::attributes::{
                ControlSequenceOfOperation, LocalTemperature, MaxCoolSetpointLimit,
                MaxHeatSetpointLimit, MinCoolSetpointLimit, MinHeatSetpointLimit,
                MinSetpointDeadBand, OccupiedCoolingSetpoint, OccupiedHeatingSetpoint,
                ThermostatRunningMode, ThermostatRunningState,
            };
            let mut cluster = MatterSubscriptionCluster::<11, 0>::for_attribute::<
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
                .add_attribute::<LocalTemperature>()?
                .add_attribute::<ThermostatRunningMode>()?
                .add_attribute::<ThermostatRunningState>()?;
            clusters.push(cluster);
        }
        DeviceRole::IndoorTemperature { .. } | DeviceRole::OutdoorTemperature => {
            use TemperatureMeasurement::attributes::MeasuredValue;
            let mut cluster = MatterSubscriptionCluster::<11, 0>::for_attribute::<MeasuredValue>(
                0,
                MATTER_SUBSCRIPTION_MAX_INTERVAL_SECONDS,
            );
            cluster.add_attribute::<MeasuredValue>()?;
            clusters.push(cluster);
        }
        DeviceRole::IndoorHumidity { .. } | DeviceRole::OutdoorHumidity => {
            use RelativeHumidityMeasurement::attributes::MeasuredValue;
            let mut cluster = MatterSubscriptionCluster::<11, 0>::for_attribute::<MeasuredValue>(
                0,
                MATTER_SUBSCRIPTION_MAX_INTERVAL_SECONDS,
            );
            cluster.add_attribute::<MeasuredValue>()?;
            clusters.push(cluster);
        }
        DeviceRole::IndoorAirQuality { .. } | DeviceRole::OutdoorAirQuality => {
            use AirQuality::attributes::AirQuality as OverallAirQuality;
            let mut overall = MatterSubscriptionCluster::<11, 0>::for_attribute::<OverallAirQuality>(
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
                reports.push((
                    index,
                    state.rooms[index].configuration.control_endpoint,
                    report,
                ));
            }
        }
    }
    for (index, endpoint, report) in reports {
        libertas_endpoint_report(endpoint, &report, None);
        let mut state = shared.borrow_mut();
        state.rooms[index].last_report = Some(report);
        state.rooms[index].last_endpoint_report_ticks = Some(now_ticks);
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
            if room
                .last_endpoint_report_ticks
                .is_some_and(|last_report| now_ticks.saturating_sub(last_report) >= interval)
            {
                reports.push((
                    index,
                    room.configuration.control_endpoint,
                    room_report(&state, index, now),
                ));
            }
        }
    }
    for (index, endpoint, report) in reports {
        libertas_endpoint_report(endpoint, &report, None);
        let mut state = shared.borrow_mut();
        state.rooms[index].last_endpoint_report_ticks = Some(now_ticks);
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
        .expect("invalid building climate room context");
    if opcode == OP_ENDPOINT_PEER_DOWN {
        // The host has confirmed this client is currently stopped or absent.
        // No ephemeral per-client state is kept here, and the host still owns
        // permanent-until-changed membership.
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
                if room.last_endpoint_report_ticks.is_none() {
                    room.last_endpoint_report_ticks = Some(now_ticks);
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
            absolute_ticks(libertas_get_sys_ticks(), seconds.max(1)),
        );
    }
}

fn subscribe_weather(shared: &Rc<RefCell<ControllerState>>) {
    if !shared.borrow().weather_server_up {
        return;
    }
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
        .expect("invalid building climate weather context");
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
        // Every delivered Up represents a newer server startup.
        shared.borrow_mut().weather_server_up = true;
        subscribe_weather(shared);
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

fn external_feature_name_is_supported(state: &ControllerState, name: &str) -> bool {
    if [
        "time.local_hour_of_day_sine",
        "time.local_hour_of_day_cosine",
        "time.local_day_of_week_sine",
        "time.local_day_of_week_cosine",
        "time.local_weekend_indicator",
        "time.local_holiday_indicator",
        "time.occupancy_schedule_active_indicator",
        "time.seconds_to_next_occupancy_transition",
    ]
    .contains(&name)
        || [
            "utility.current_price_per_kilowatt_hour",
            "utility.forecast_price_plus_15m_per_kilowatt_hour",
            "utility.forecast_price_plus_30m_per_kilowatt_hour",
            "utility.forecast_price_plus_60m_per_kilowatt_hour",
            "utility.forecast_price_plus_2h_per_kilowatt_hour",
            "utility.forecast_price_plus_3h_per_kilowatt_hour",
            "utility.forecast_price_plus_6h_per_kilowatt_hour",
            "utility.forecast_price_plus_12h_per_kilowatt_hour",
            "utility.forecast_price_plus_24h_per_kilowatt_hour",
            "utility.minimum_price_next_6h_per_kilowatt_hour",
            "utility.maximum_price_next_6h_per_kilowatt_hour",
            "utility.minimum_price_next_24h_per_kilowatt_hour",
            "utility.maximum_price_next_24h_per_kilowatt_hour",
            "utility.current_carbon_intensity_kilograms_per_kilowatt_hour",
            "utility.forecast_carbon_intensity_plus_60m_kilograms_per_kilowatt_hour",
            "utility.forecast_carbon_intensity_plus_6h_kilograms_per_kilowatt_hour",
            "utility.demand_response_active_indicator",
            "utility.seconds_to_demand_response_transition",
            "utility.building_electric_demand_kilowatts",
            "utility.building_electric_demand_mean_15m_kilowatts",
            "utility.building_electric_demand_mean_60m_kilowatts",
            "utility.building_peak_window_demand_kilowatts",
        ]
        .contains(&name)
    {
        return true;
    }
    if let Some(rest) = name.strip_prefix("room.")
        && let Some((endpoint, suffix)) = rest.split_once('.')
        && endpoint
            .parse::<LibertasEndpoint>()
            .ok()
            .is_some_and(|endpoint| {
                state
                    .rooms
                    .iter()
                    .any(|room| room.configuration.control_endpoint == endpoint)
            })
    {
        return [
            "occupancy_state_normalized",
            "occupancy_fraction_15m",
            "occupancy_fraction_60m",
            "occupant_count",
            "window_open_fraction_15m",
            "window_open_fraction_60m",
            "override_active_indicator",
            "override_remaining_seconds",
            "recent_delivered_heating_kilowatt_hours_thermal",
            "recent_delivered_cooling_kilowatt_hours_thermal",
        ]
        .contains(&suffix);
    }
    if let Some(rest) = name.strip_prefix("thermostat.")
        && let Some((device, suffix)) = rest.split_once('.')
        && device.parse::<LibertasDevice>().ok().is_some_and(|device| {
            state
                .thermostats
                .iter()
                .any(|thermostat| thermostat.configuration.thermostat == device)
        })
    {
        return [
            "pi_heating_demand_normalized",
            "pi_cooling_demand_normalized",
            "signed_pi_demand_normalized",
            "local_relative_humidity_percent",
            "last_command_succeeded_indicator",
            "electric_power_kilowatts",
            "electric_energy_kilowatt_hours",
            "gas_power_kilowatts",
            "gas_energy_kilowatt_hours",
            "delivered_heating_power_kilowatts_thermal",
            "delivered_cooling_power_kilowatts_thermal",
        ]
        .contains(&suffix);
    }
    let Some(rest) = name.strip_prefix("equipment.central.") else {
        return false;
    };
    let Some((aggregation, measurement)) = rest.split_once('.') else {
        return false;
    };
    ["current", "mean_15m", "mean_60m", "change_15m"].contains(&aggregation)
        && [
            "supply_air_temperature_celsius",
            "return_air_temperature_celsius",
            "mixed_air_temperature_celsius",
            "supply_airflow_cubic_meters_per_second",
            "outdoor_airflow_cubic_meters_per_second",
            "supply_fan_speed_normalized",
            "return_fan_speed_normalized",
            "duct_static_pressure_pascals",
            "duct_static_pressure_setpoint_pascals",
            "outdoor_air_damper_position_normalized",
            "return_air_damper_position_normalized",
            "zone_damper_mean_position_normalized",
            "heating_valve_position_normalized",
            "cooling_valve_position_normalized",
            "compressor_capacity_normalized",
            "heating_stage",
            "cooling_stage",
            "supply_water_temperature_celsius",
            "return_water_temperature_celsius",
            "pump_speed_normalized",
            "electric_power_kilowatts",
            "electric_energy_kilowatt_hours",
            "gas_power_kilowatts",
            "gas_energy_kilowatt_hours",
            "delivered_heating_power_kilowatts_thermal",
            "delivered_cooling_power_kilowatts_thermal",
            "coefficient_of_performance",
            "active_fault_count",
        ]
        .contains(&measurement)
}

fn accept_external_features(
    shared: &Rc<RefCell<ControllerState>>,
    snapshot: BuildingHvacExternalFeatureSnapshotV1,
) -> bool {
    let now = libertas_get_utc_time();
    let unchanged = {
        let state = shared.borrow();
        if !snapshot.is_well_formed()
            || now.is_none_or(|now| snapshot.retrieved_at > now)
            || snapshot.retrieved_at < state.external_features.retrieved_at
            || snapshot.retrieved_at == state.external_features.retrieved_at
                && snapshot != state.external_features
            || snapshot
                .inputs
                .iter()
                .any(|input| !external_feature_name_is_supported(&state, &input.feature_name))
        {
            return false;
        }
        snapshot == state.external_features
    };
    if unchanged {
        return true;
    }
    libertas_data_write(
        EXTERNAL_FEATURE_INPUTS_RESOURCE,
        singleton_key(),
        &BuildingHvacPersistentDataV1::ExternalFeatureInputsV1 {
            snapshot: snapshot.clone(),
        },
    );
    let mut state = shared.borrow_mut();
    state.external_features = snapshot;
    for room in &mut state.rooms {
        room.plan = None;
    }
    true
}

fn arm_external_feature_retry(shared: &Rc<RefCell<ControllerState>>, seconds: u32) {
    let (timer, server_up) = {
        let state = shared.borrow();
        (
            state.external_feature_retry_timer,
            state.external_feature_server_up,
        )
    };
    if timer != 0 {
        if !server_up {
            libertas_timer_cancel(timer);
            return;
        }
        libertas_timer_update_interval(
            timer,
            absolute_ticks(libertas_get_sys_ticks(), seconds.max(1)),
        );
    }
}

fn subscribe_external_features(shared: &Rc<RefCell<ControllerState>>) {
    if !shared.borrow().external_feature_server_up {
        return;
    }
    let Some(endpoint) = shared.borrow().external_feature_endpoint else {
        return;
    };
    libertas_endpoint_subscribe_request(
        endpoint,
        &BuildingHvacExternalFeatureProtocolV1::GetExternalFeaturesV1,
    );
    arm_external_feature_retry(shared, EXTERNAL_FEATURE_RETRY_SECONDS);
}

fn handle_external_feature_endpoint(
    _endpoint: LibertasEndpoint,
    opcode: u8,
    message: LibertasEndpointMessage<BuildingHvacExternalFeatureProtocolV1>,
    context: &mut Box<dyn Any>,
    _transaction_id: LibertasTransId,
    _peer: u32,
) -> LibertasEndpointHandlerResult {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .expect("invalid building climate external-feature context");
    if opcode == OP_ENDPOINT_PEER_DOWN {
        let timer = {
            let mut state = shared.borrow_mut();
            state.external_feature_server_up = false;
            state.external_feature_retry_timer
        };
        if timer != 0 {
            libertas_timer_cancel(timer);
        }
        return LibertasEndpointHandlerResult::Handled;
    }
    if opcode == OP_ENDPOINT_PEER_UP {
        // Every delivered Up represents a newer server startup.
        shared.borrow_mut().external_feature_server_up = true;
        subscribe_external_features(shared);
        return LibertasEndpointHandlerResult::Handled;
    }
    let mut retry_seconds = EXTERNAL_FEATURE_RETRY_SECONDS;
    let accepted = match (opcode, message) {
        (
            OP_ENDPOINT_RSP,
            LibertasEndpointMessage::Data(
                BuildingHvacExternalFeatureProtocolV1::ExternalFeaturesV1 {
                    maximum_wait_interval_seconds,
                    snapshot,
                },
            ),
        ) if maximum_wait_interval_seconds != 0 => {
            let accepted = accept_external_features(shared, snapshot);
            if accepted {
                shared.borrow_mut().external_feature_maximum_wait_seconds =
                    maximum_wait_interval_seconds;
                arm_external_feature_retry(shared, maximum_wait_interval_seconds);
            }
            accepted
        }
        (
            OP_ENDPOINT_DATA,
            LibertasEndpointMessage::Data(
                BuildingHvacExternalFeatureProtocolV1::ExternalFeatureUpdateV1 { snapshot },
            ),
        ) => {
            let accepted = accept_external_features(shared, snapshot);
            if accepted {
                let wait = shared.borrow().external_feature_maximum_wait_seconds;
                arm_external_feature_retry(shared, wait);
            }
            accepted
        }
        (
            OP_ENDPOINT_RSP,
            LibertasEndpointMessage::Data(
                BuildingHvacExternalFeatureProtocolV1::ExternalFeaturesErrorV1 {
                    retry_after_seconds,
                    ..
                },
            ),
        ) => {
            retry_seconds = retry_after_seconds.max(1);
            false
        }
        _ => false,
    };
    if accepted {
        evaluate_and_publish(shared);
    } else {
        arm_external_feature_retry(shared, retry_seconds);
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
        .find(|prediction| {
            prediction.horizon == horizon
                && prediction.source == BuildingHvacThermalPredictionSourceV1::Xgboost
        })
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
                        let learned_change = prediction_for(
                            room,
                            BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
                        );
                        BuildingHvacRoomControlCandidate {
                            room_endpoint: room.configuration.control_endpoint,
                            control,
                            state: &room.state,
                            predicted_cross_zone_temperature_change_celsius: if learned_change
                                .is_some()
                            {
                                0.0
                            } else {
                                predicted_cross_zone_change(&state, room)
                            },
                            predicted_machine_learning_temperature_change_celsius: learned_change,
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
    let day_of_year = utc_day_of_year(now);
    let hour_angle = TAU * seconds_of_day / 86_400.0;
    let day_angle = TAU * f32::from(day_of_year) / 365.25;
    (
        hour_angle.sin(),
        hour_angle.cos(),
        day_angle.sin(),
        day_angle.cos(),
    )
}

fn utc_day_of_year(now: LibertasDateTime) -> u16 {
    // Convert Unix days to the proleptic Gregorian civil date. This preserves
    // the real January-through-December phase across leap years.
    let days_since_epoch = i64::try_from(now / 86_400).unwrap_or(i64::MAX / 2);
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
    let day_of_year = month_offsets[month_index] + day + leap_offset;
    u16::try_from(day_of_year).unwrap_or(1)
}

fn active_thermostat_setpoint_delta_celsius(
    activity: BuildingHvacRoomActivityV1,
    heating_setpoint_celsius: Option<f32>,
    cooling_setpoint_celsius: Option<f32>,
    associated_room_temperatures_celsius: impl Iterator<Item = f32>,
) -> f32 {
    match activity {
        BuildingHvacRoomActivityV1::Heating => heating_setpoint_celsius
            .filter(|setpoint| setpoint.is_finite())
            .and_then(|setpoint| {
                associated_room_temperatures_celsius
                    .filter(|temperature| temperature.is_finite())
                    .map(|temperature| setpoint - temperature)
                    .reduce(f32::max)
            })
            .unwrap_or(0.0)
            .clamp(0.0, 50.0),
        BuildingHvacRoomActivityV1::Cooling => cooling_setpoint_celsius
            .filter(|setpoint| setpoint.is_finite())
            .and_then(|setpoint| {
                associated_room_temperatures_celsius
                    .filter(|temperature| temperature.is_finite())
                    .map(|temperature| setpoint - temperature)
                    .reduce(f32::min)
            })
            .unwrap_or(0.0)
            .clamp(-50.0, 0.0),
        _ => 0.0,
    }
}

struct MachineLearningFeatureBuilder {
    target_room: LibertasEndpoint,
    values: Vec<BuildingHvacMachineLearningFeatureV1>,
}

impl MachineLearningFeatureBuilder {
    fn new(target_room: LibertasEndpoint) -> Self {
        Self {
            target_room,
            values: Vec::new(),
        }
    }

    fn add(&mut self, name: impl Into<String>, value: Option<f32>) {
        self.values.push(BuildingHvacMachineLearningFeatureV1 {
            name: name.into(),
            value: value.filter(|value| value.is_finite()),
        });
    }

    fn finish(mut self) -> Option<BuildingHvacMachineLearningFeaturesV1> {
        self.values
            .sort_by(|left, right| left.name.cmp(&right.name));
        let features = BuildingHvacMachineLearningFeaturesV1 {
            target_room: self.target_room,
            values: self.values,
        };
        features.is_well_formed().then_some(features)
    }
}

fn activity_value(activity: BuildingHvacRoomActivityV1) -> Option<f32> {
    match activity {
        BuildingHvacRoomActivityV1::Heating => Some(1.0),
        BuildingHvacRoomActivityV1::Cooling => Some(-1.0),
        BuildingHvacRoomActivityV1::Idle | BuildingHvacRoomActivityV1::FanOnly => Some(0.0),
        BuildingHvacRoomActivityV1::Unknown => None,
    }
}

const fn binary_indicator(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}

fn activity_indicator(
    activity: BuildingHvacRoomActivityV1,
    expected: BuildingHvacRoomActivityV1,
) -> Option<f32> {
    (activity != BuildingHvacRoomActivityV1::Unknown)
        .then_some(binary_indicator(activity == expected))
}

fn room_observation_at(
    state: &ControllerState,
    endpoint: LibertasEndpoint,
    at_or_before: LibertasDateTime,
) -> Option<&RoomFeatureObservation> {
    state
        .feature_history
        .iter()
        .rev()
        .find(|observation| observation.observed_at <= at_or_before)?
        .rooms
        .iter()
        .find(|room| room.endpoint == endpoint)
}

fn thermostat_observation_at(
    state: &ControllerState,
    thermostat: LibertasDevice,
    at_or_before: LibertasDateTime,
) -> Option<&ThermostatFeatureObservation> {
    state
        .feature_history
        .iter()
        .rev()
        .find(|observation| observation.observed_at <= at_or_before)?
        .thermostats
        .iter()
        .find(|entry| entry.thermostat == thermostat)
}

fn add_room_air_measurements(
    builder: &mut MachineLearningFeatureBuilder,
    prefix: &str,
    room: &RoomRuntime,
    now: LibertasDateTime,
) {
    let kinds = [
        (BuildingHvacAirMeasurementKindV1::CarbonDioxide, "co2"),
        (BuildingHvacAirMeasurementKindV1::CarbonMonoxide, "co"),
        (BuildingHvacAirMeasurementKindV1::NitrogenDioxide, "no2"),
        (BuildingHvacAirMeasurementKindV1::Ozone, "ozone"),
        (BuildingHvacAirMeasurementKindV1::ParticulateMatter1, "pm1"),
        (
            BuildingHvacAirMeasurementKindV1::ParticulateMatter2_5,
            "pm2_5",
        ),
        (
            BuildingHvacAirMeasurementKindV1::ParticulateMatter10,
            "pm10",
        ),
        (
            BuildingHvacAirMeasurementKindV1::Formaldehyde,
            "formaldehyde",
        ),
        (
            BuildingHvacAirMeasurementKindV1::TotalVolatileOrganicCompounds,
            "tvoc",
        ),
        (BuildingHvacAirMeasurementKindV1::Radon, "radon"),
    ];
    for (kind, name) in kinds {
        let mut mixing_ppm = Vec::new();
        let mut mass_micrograms = Vec::new();
        let mut radioactivity = Vec::new();
        for measurement in room.sensor_states.iter().filter_map(|sensor| {
            sensor
                .air_quality
                .as_ref()
                .filter(|reading| {
                    reading.observed_at <= now
                        && reading.valid_until > now
                        && reading.is_well_formed()
                })
                .and_then(|reading| reading.measurement(kind))
        }) {
            let value = measurement.measured_value_in_reported_unit;
            match measurement.reported_unit {
                BuildingHvacAirMeasurementUnitV1::PartsPerMillion => mixing_ppm.push(value),
                BuildingHvacAirMeasurementUnitV1::PartsPerBillion => {
                    mixing_ppm.push(value / 1_000.0);
                }
                BuildingHvacAirMeasurementUnitV1::PartsPerTrillion => {
                    mixing_ppm.push(value / 1_000_000.0);
                }
                BuildingHvacAirMeasurementUnitV1::MilligramsPerCubicMeter => {
                    mass_micrograms.push(value * 1_000.0);
                }
                BuildingHvacAirMeasurementUnitV1::MicrogramsPerCubicMeter => {
                    mass_micrograms.push(value);
                }
                BuildingHvacAirMeasurementUnitV1::NanogramsPerCubicMeter => {
                    mass_micrograms.push(value / 1_000.0);
                }
                BuildingHvacAirMeasurementUnitV1::PicogramsPerCubicMeter => {
                    mass_micrograms.push(value / 1_000_000.0);
                }
                BuildingHvacAirMeasurementUnitV1::BecquerelsPerCubicMeter => {
                    radioactivity.push(value);
                }
            }
        }
        let mean = |values: &[f32]| {
            (!values.is_empty()).then(|| {
                values.iter().copied().map(f64::from).sum::<f64>() as f32 / values.len() as f32
            })
        };
        builder.add(
            format!("{prefix}.air_quality.{name}_parts_per_million"),
            mean(&mixing_ppm),
        );
        builder.add(
            format!("{prefix}.air_quality.{name}_micrograms_per_cubic_meter"),
            mean(&mass_micrograms),
        );
        builder.add(
            format!("{prefix}.air_quality.{name}_becquerels_per_cubic_meter"),
            mean(&radioactivity),
        );
    }
}

fn weather_psychrometrics(conditions: &BuildingHvacOutdoorConditionsV1) -> Option<(f32, f32, f32)> {
    let pressure_pascals = conditions.surface_pressure_hectopascals * 100.0;
    let vapor_pressure =
        saturation_vapor_pressure_pascals(conditions.dew_point_temperature_celsius)?;
    if !pressure_pascals.is_finite()
        || vapor_pressure >= pressure_pascals
        || conditions.dew_point_temperature_celsius > conditions.dry_bulb_temperature_celsius + 0.05
    {
        return None;
    }
    let humidity_ratio = 0.621_945 * vapor_pressure / (pressure_pascals - vapor_pressure);
    let enthalpy = moist_air_enthalpy_kilojoules_per_kilogram_dry_air(
        conditions.dry_bulb_temperature_celsius,
        humidity_ratio,
    );
    let wet_bulb = solve_wet_bulb_temperature_celsius(
        conditions.dry_bulb_temperature_celsius,
        conditions.dew_point_temperature_celsius,
        pressure_pascals,
        enthalpy,
    )?;
    Some((humidity_ratio, enthalpy, wet_bulb))
}

fn add_weather_conditions(
    builder: &mut MachineLearningFeatureBuilder,
    prefix: &str,
    conditions: Option<&BuildingHvacOutdoorConditionsV1>,
    precipitation_probability_percent: Option<f32>,
) {
    let psychrometrics = conditions.and_then(weather_psychrometrics);
    let direction_radians =
        conditions.map(|value| f32::from(value.wind_direction_degrees).to_radians());
    let solar_azimuth_radians = conditions.map(|value| value.solar_azimuth_degrees.to_radians());
    builder.add(
        format!("{prefix}.dry_bulb_temperature_celsius"),
        conditions.map(|value| value.dry_bulb_temperature_celsius),
    );
    builder.add(
        format!("{prefix}.dew_point_temperature_celsius"),
        conditions.map(|value| value.dew_point_temperature_celsius),
    );
    builder.add(
        format!("{prefix}.relative_humidity_percent"),
        conditions.map(|value| f32::from(value.relative_humidity_percent)),
    );
    builder.add(
        format!("{prefix}.humidity_ratio_kg_per_kg"),
        psychrometrics.map(|value| value.0),
    );
    builder.add(
        format!("{prefix}.moist_air_enthalpy_kilojoules_per_kilogram"),
        psychrometrics.map(|value| value.1),
    );
    builder.add(
        format!("{prefix}.wet_bulb_temperature_celsius"),
        psychrometrics.map(|value| value.2),
    );
    builder.add(
        format!("{prefix}.surface_pressure_hectopascals"),
        conditions.map(|value| value.surface_pressure_hectopascals),
    );
    builder.add(
        format!("{prefix}.wind_speed_meters_per_second"),
        conditions.map(|value| value.wind_speed_meters_per_second),
    );
    builder.add(
        format!("{prefix}.wind_gust_meters_per_second"),
        conditions.map(|value| value.wind_gust_meters_per_second),
    );
    builder.add(
        format!("{prefix}.wind_direction_sine"),
        direction_radians.map(f32::sin),
    );
    builder.add(
        format!("{prefix}.wind_direction_cosine"),
        direction_radians.map(f32::cos),
    );
    builder.add(
        format!("{prefix}.precipitation_millimeters"),
        conditions.map(|value| value.precipitation_millimeters),
    );
    for (name, kind) in [
        ("none", BuildingHvacPrecipitationKindV1::None),
        ("rain", BuildingHvacPrecipitationKindV1::Rain),
        (
            "freezing_rain",
            BuildingHvacPrecipitationKindV1::FreezingRain,
        ),
        ("snow", BuildingHvacPrecipitationKindV1::Snow),
        ("mixed", BuildingHvacPrecipitationKindV1::Mixed),
        ("unknown", BuildingHvacPrecipitationKindV1::Unknown),
    ] {
        builder.add(
            format!("{prefix}.precipitation_kind_{name}_indicator"),
            conditions.map(|value| binary_indicator(value.precipitation_kind == kind)),
        );
    }
    builder.add(
        format!("{prefix}.precipitation_probability_percent"),
        precipitation_probability_percent,
    );
    builder.add(
        format!("{prefix}.solar_elevation_degrees"),
        conditions.map(|value| value.solar_elevation_degrees),
    );
    builder.add(
        format!("{prefix}.solar_azimuth_sine"),
        solar_azimuth_radians.map(f32::sin),
    );
    builder.add(
        format!("{prefix}.solar_azimuth_cosine"),
        solar_azimuth_radians.map(f32::cos),
    );
    builder.add(
        format!("{prefix}.global_horizontal_irradiance_watts_per_square_meter"),
        conditions.map(|value| value.global_horizontal_irradiance_watts_per_square_meter),
    );
    builder.add(
        format!("{prefix}.direct_normal_irradiance_watts_per_square_meter"),
        conditions.map(|value| value.direct_normal_irradiance_watts_per_square_meter),
    );
    builder.add(
        format!("{prefix}.diffuse_horizontal_irradiance_watts_per_square_meter"),
        conditions.map(|value| value.diffuse_horizontal_irradiance_watts_per_square_meter),
    );
}

fn weather_history_conditions(
    state: &ControllerState,
    at: LibertasDateTime,
) -> Option<&BuildingHvacOutdoorConditionsV1> {
    state
        .weather
        .history
        .as_ref()?
        .periods
        .iter()
        .rev()
        .find(|period| {
            period.starts_at <= at
                && at
                    < period
                        .starts_at
                        .saturating_add(u64::from(period.duration_seconds))
        })
        .map(|period| &period.conditions)
}

fn weather_forecast_period(
    state: &ControllerState,
    at: LibertasDateTime,
) -> Option<&BuildingHvacWeatherForecastPeriodV1> {
    state
        .weather
        .forecast
        .as_ref()?
        .periods
        .iter()
        .find(|period| {
            period.starts_at <= at
                && at
                    < period
                        .starts_at
                        .saturating_add(u64::from(period.duration_seconds))
        })
}

fn outdoor_air_quality_period(
    state: &ControllerState,
    at: LibertasDateTime,
) -> Option<&BuildingHvacOutdoorAirQualityPeriodV1> {
    state
        .weather
        .outdoor_air_quality
        .as_ref()?
        .periods
        .iter()
        .find(|period| {
            period.starts_at <= at
                && at
                    < period
                        .starts_at
                        .saturating_add(u64::from(period.duration_seconds))
        })
}

fn add_outdoor_air_quality(
    builder: &mut MachineLearningFeatureBuilder,
    prefix: &str,
    period: Option<&BuildingHvacOutdoorAirQualityPeriodV1>,
) {
    builder.add(
        format!("{prefix}.pm2_5_micrograms_per_cubic_meter"),
        period.map(|value| value.particulate_matter_2_5_micrograms_per_cubic_meter),
    );
    builder.add(
        format!("{prefix}.pm10_micrograms_per_cubic_meter"),
        period.map(|value| value.particulate_matter_10_micrograms_per_cubic_meter),
    );
    builder.add(
        format!("{prefix}.ozone_micrograms_per_cubic_meter"),
        period.map(|value| value.ozone_micrograms_per_cubic_meter),
    );
    builder.add(
        format!("{prefix}.no2_micrograms_per_cubic_meter"),
        period.map(|value| value.nitrogen_dioxide_micrograms_per_cubic_meter),
    );
}

fn mean_history_condition(
    state: &ControllerState,
    now: LibertasDateTime,
    window_seconds: u64,
    value: impl Fn(&BuildingHvacOutdoorConditionsV1) -> f32,
) -> Option<f32> {
    let starts_at = now.saturating_sub(window_seconds);
    let values: Vec<_> = state
        .weather
        .history
        .as_ref()?
        .periods
        .iter()
        .filter(|period| period.starts_at >= starts_at && period.starts_at < now)
        .map(|period| value(&period.conditions))
        .filter(|value| value.is_finite())
        .collect();
    (!values.is_empty())
        .then(|| values.iter().copied().map(f64::from).sum::<f64>() as f32 / values.len() as f32)
}

fn mean_history_derived(
    state: &ControllerState,
    now: LibertasDateTime,
    window_seconds: u64,
    value: impl Fn(&BuildingHvacOutdoorConditionsV1) -> Option<f32>,
) -> Option<f32> {
    let starts_at = now.saturating_sub(window_seconds);
    let values: Vec<_> = state
        .weather
        .history
        .as_ref()?
        .periods
        .iter()
        .filter(|period| period.starts_at >= starts_at && period.starts_at < now)
        .filter_map(|period| value(&period.conditions))
        .filter(|value| value.is_finite())
        .collect();
    (!values.is_empty())
        .then(|| values.iter().copied().map(f64::from).sum::<f64>() as f32 / values.len() as f32)
}

fn add_weather_features(
    builder: &mut MachineLearningFeatureBuilder,
    state: &ControllerState,
    now: LibertasDateTime,
) {
    let current = state.weather.current.as_ref();
    add_weather_conditions(
        builder,
        "weather.current",
        current.map(|value| &value.conditions),
        None,
    );
    builder.add(
        "weather.current.section_age_seconds",
        current.map(|value| now.saturating_sub(value.retrieved_at) as f32),
    );
    builder.add(
        "weather.history.section_age_seconds",
        state
            .weather
            .history
            .as_ref()
            .map(|value| now.saturating_sub(value.retrieved_at) as f32),
    );
    builder.add(
        "weather.forecast.section_age_seconds",
        state
            .weather
            .forecast
            .as_ref()
            .map(|value| now.saturating_sub(value.retrieved_at) as f32),
    );
    builder.add(
        "weather.air_quality.section_age_seconds",
        state
            .weather
            .outdoor_air_quality
            .as_ref()
            .map(|value| now.saturating_sub(value.retrieved_at) as f32),
    );
    for (label, seconds) in [("15m", 15 * 60), ("30m", 30 * 60), ("60m", 60 * 60)] {
        add_weather_conditions(
            builder,
            &format!("weather.history.lag_{label}"),
            weather_history_conditions(state, now.saturating_sub(seconds)),
            None,
        );
    }
    for (label, seconds) in [
        ("15m", 15 * 60),
        ("30m", 30 * 60),
        ("60m", 60 * 60),
        ("2h", 2 * 60 * 60),
        ("3h", 3 * 60 * 60),
        ("6h", 6 * 60 * 60),
        ("12h", 12 * 60 * 60),
        ("24h", 24 * 60 * 60),
    ] {
        let at = now.saturating_add(seconds);
        let forecast = weather_forecast_period(state, at);
        add_weather_conditions(
            builder,
            &format!("weather.forecast.plus_{label}"),
            forecast.map(|value| &value.conditions),
            forecast.map(|value| f32::from(value.precipitation_probability_percent)),
        );
        add_outdoor_air_quality(
            builder,
            &format!("weather.air_quality.plus_{label}"),
            outdoor_air_quality_period(state, at),
        );
    }
    add_outdoor_air_quality(
        builder,
        "weather.air_quality.current",
        outdoor_air_quality_period(state, now),
    );
    for (label, seconds) in [
        ("3h", 3 * 60 * 60),
        ("6h", 6 * 60 * 60),
        ("24h", 24 * 60 * 60),
    ] {
        builder.add(
            format!("weather.history.mean_dry_bulb_temperature_celsius_{label}"),
            mean_history_condition(state, now, seconds, |value| {
                value.dry_bulb_temperature_celsius
            }),
        );
        builder.add(
            format!("weather.history.mean_relative_humidity_percent_{label}"),
            mean_history_condition(state, now, seconds, |value| {
                f32::from(value.relative_humidity_percent)
            }),
        );
        builder.add(
            format!("weather.history.mean_dew_point_temperature_celsius_{label}"),
            mean_history_condition(state, now, seconds, |value| {
                value.dew_point_temperature_celsius
            }),
        );
        builder.add(
            format!("weather.history.mean_humidity_ratio_kg_per_kg_{label}"),
            mean_history_derived(state, now, seconds, |value| {
                weather_psychrometrics(value).map(|derived| derived.0)
            }),
        );
        builder.add(
            format!("weather.history.mean_moist_air_enthalpy_kilojoules_per_kilogram_{label}"),
            mean_history_derived(state, now, seconds, |value| {
                weather_psychrometrics(value).map(|derived| derived.1)
            }),
        );
        builder.add(
            format!("weather.history.mean_wet_bulb_temperature_celsius_{label}"),
            mean_history_derived(state, now, seconds, |value| {
                weather_psychrometrics(value).map(|derived| derived.2)
            }),
        );
        builder.add(
            format!("weather.history.mean_wind_speed_meters_per_second_{label}"),
            mean_history_condition(state, now, seconds, |value| {
                value.wind_speed_meters_per_second
            }),
        );
        builder.add(
            format!(
                "weather.history.mean_global_horizontal_irradiance_watts_per_square_meter_{label}"
            ),
            mean_history_condition(state, now, seconds, |value| {
                value.global_horizontal_irradiance_watts_per_square_meter
            }),
        );
        let previous = weather_history_conditions(state, now.saturating_sub(seconds));
        builder.add(
            format!("weather.history.dry_bulb_temperature_change_celsius_{label}"),
            current.zip(previous).map(|(current, previous)| {
                current.conditions.dry_bulb_temperature_celsius
                    - previous.dry_bulb_temperature_celsius
            }),
        );
    }
    for (label, seconds) in [("1h", 60 * 60), ("6h", 6 * 60 * 60), ("24h", 24 * 60 * 60)] {
        let starts_at = now.saturating_sub(seconds);
        let periods: Vec<_> = state
            .weather
            .history
            .as_ref()
            .map(|history| {
                history
                    .periods
                    .iter()
                    .filter(|period| period.starts_at >= starts_at && period.starts_at < now)
                    .collect()
            })
            .unwrap_or_default();
        let integrated = |value: fn(&BuildingHvacOutdoorConditionsV1) -> f32| {
            (!periods.is_empty()).then(|| {
                periods
                    .iter()
                    .map(|period| {
                        value(&period.conditions) * period.duration_seconds as f32 / 3_600.0
                    })
                    .sum()
            })
        };
        builder.add(
            format!("weather.history.solar_energy_wh_per_square_meter_{label}"),
            integrated(|value| value.global_horizontal_irradiance_watts_per_square_meter),
        );
        builder.add(
            format!("weather.history.precipitation_millimeters_{label}"),
            (!periods.is_empty()).then(|| {
                periods
                    .iter()
                    .map(|period| period.conditions.precipitation_millimeters)
                    .sum()
            }),
        );
        builder.add(
            format!("weather.history.heating_degree_hours_base18_celsius_{label}"),
            integrated(|value| (18.0 - value.dry_bulb_temperature_celsius).max(0.0)),
        );
        builder.add(
            format!("weather.history.cooling_degree_hours_base18_celsius_{label}"),
            integrated(|value| (value.dry_bulb_temperature_celsius - 18.0).max(0.0)),
        );
    }
}

type ThermostatWindowStatistics = (
    Option<f32>,
    Option<f32>,
    Option<f32>,
    Option<f32>,
    Option<u32>,
);

fn thermostat_window_statistics(
    state: &ControllerState,
    thermostat: LibertasDevice,
    now: LibertasDateTime,
    window_seconds: u64,
) -> ThermostatWindowStatistics {
    let starts_at = now.saturating_sub(window_seconds);
    let samples: Vec<_> = state
        .feature_history
        .iter()
        .filter(|observation| observation.observed_at >= starts_at)
        .filter_map(|observation| {
            observation
                .thermostats
                .iter()
                .find(|entry| entry.thermostat == thermostat)
        })
        .collect();
    let known: Vec<_> = samples
        .iter()
        .filter(|entry| entry.activity != BuildingHvacRoomActivityV1::Unknown)
        .collect();
    let fraction = |expected: BuildingHvacRoomActivityV1| {
        (!known.is_empty()).then(|| {
            known
                .iter()
                .filter(|entry| entry.activity == expected)
                .count() as f32
                / known.len() as f32
        })
    };
    let starts = samples
        .windows(2)
        .filter(|pair| {
            activity_value(pair[0].activity).is_some_and(|value| value == 0.0)
                && activity_value(pair[1].activity).is_some_and(|value| value != 0.0)
        })
        .count();
    (
        fraction(BuildingHvacRoomActivityV1::Heating),
        fraction(BuildingHvacRoomActivityV1::Cooling),
        fraction(BuildingHvacRoomActivityV1::FanOnly),
        fraction(BuildingHvacRoomActivityV1::Idle),
        u32::try_from(starts).ok(),
    )
}

fn capture_feature_observation(state: &mut ControllerState, now: LibertasDateTime) {
    let observed_at = now - now % 60;
    if state
        .feature_history
        .last()
        .is_some_and(|observation| observation.observed_at == observed_at)
    {
        return;
    }
    let thermostats = state
        .thermostats
        .iter()
        .enumerate()
        .map(
            |(thermostat_index, thermostat)| ThermostatFeatureObservation {
                thermostat: thermostat.configuration.thermostat,
                activity: thermostat.activity,
                local_temperature_celsius: thermostat.local_temperature_celsius,
                heating_setpoint_celsius: thermostat.heating_setpoint_celsius,
                cooling_setpoint_celsius: thermostat.cooling_setpoint_celsius,
                active_setpoint_delta_celsius: active_thermostat_setpoint_delta_celsius(
                    thermostat.activity,
                    thermostat.heating_setpoint_celsius,
                    thermostat.cooling_setpoint_celsius,
                    state
                        .rooms
                        .iter()
                        .filter(move |room| room.thermostat_index == thermostat_index)
                        .filter_map(|room| room.state.temperature_celsius),
                ),
                write_pending: thermostat.pending_write.is_some(),
            },
        )
        .collect();
    let rooms = state
        .rooms
        .iter()
        .map(|room| RoomFeatureObservation {
            endpoint: room.configuration.control_endpoint,
            temperature_celsius: room.state.temperature_celsius,
            relative_humidity_percent: room.state.relative_humidity_percent,
            effective_heating_setpoint_celsius: room.state.effective_heating_setpoint_celsius,
            effective_cooling_setpoint_celsius: room.state.effective_cooling_setpoint_celsius,
            activity: room.state.activity,
        })
        .collect();
    state.feature_history.push(BuildingFeatureObservation {
        observed_at,
        thermostats,
        rooms,
    });
    let oldest = observed_at.saturating_sub(24 * 60 * 60);
    let first_retained = state
        .feature_history
        .partition_point(|observation| observation.observed_at < oldest);
    if first_retained != 0 {
        state.feature_history.drain(..first_retained);
    }
}

fn machine_learning_features(
    state: &ControllerState,
    room_index: usize,
    now: LibertasDateTime,
) -> Option<BuildingHvacMachineLearningFeaturesV1> {
    let target_room = &state.rooms[room_index];
    let target_temperature = target_room.state.temperature_celsius?;
    let target_endpoint = target_room.configuration.control_endpoint;
    let mut builder = MachineLearningFeatureBuilder::new(target_endpoint);
    let (hour_sin, hour_cos, day_sin, day_cos) = cyclic_time(now);
    builder.add("time.utc_hour_of_day_sine", Some(hour_sin));
    builder.add("time.utc_hour_of_day_cosine", Some(hour_cos));
    builder.add("time.day_of_year_sine", Some(day_sin));
    builder.add("time.day_of_year_cosine", Some(day_cos));
    let day_of_week = ((now / (24 * 60 * 60) + 4) % 7) as f32;
    let day_angle = day_of_week / 7.0 * TAU;
    builder.add("time.utc_day_of_week_sine", Some(day_angle.sin()));
    builder.add("time.utc_day_of_week_cosine", Some(day_angle.cos()));
    builder.add(
        "time.utc_weekend_indicator",
        Some(binary_indicator(day_of_week == 0.0 || day_of_week == 6.0)),
    );
    builder.add("time.local_hour_of_day_sine", None);
    builder.add("time.local_hour_of_day_cosine", None);
    builder.add("time.local_day_of_week_sine", None);
    builder.add("time.local_day_of_week_cosine", None);
    builder.add("time.local_weekend_indicator", None);
    builder.add("time.local_holiday_indicator", None);
    builder.add("time.occupancy_schedule_active_indicator", None);
    builder.add("time.seconds_to_next_occupancy_transition", None);

    for (thermostat_index, thermostat) in state.thermostats.iter().enumerate() {
        let device = thermostat.configuration.thermostat;
        let prefix = format!("thermostat.{device}");
        let active_delta = active_thermostat_setpoint_delta_celsius(
            thermostat.activity,
            thermostat.heating_setpoint_celsius,
            thermostat.cooling_setpoint_celsius,
            state
                .rooms
                .iter()
                .filter(move |room| room.thermostat_index == thermostat_index)
                .filter_map(|room| room.state.temperature_celsius),
        );
        builder.add(
            format!("{prefix}.active_setpoint_delta_celsius"),
            Some(active_delta),
        );
        builder.add(
            format!("{prefix}.heating_error_celsius"),
            (active_delta > 0.0).then_some(active_delta),
        );
        builder.add(
            format!("{prefix}.cooling_error_celsius"),
            (active_delta < 0.0).then_some(-active_delta),
        );
        builder.add(format!("{prefix}.pi_heating_demand_normalized"), None);
        builder.add(format!("{prefix}.pi_cooling_demand_normalized"), None);
        builder.add(format!("{prefix}.signed_pi_demand_normalized"), None);
        builder.add(
            format!("{prefix}.heating_indicator"),
            activity_indicator(thermostat.activity, BuildingHvacRoomActivityV1::Heating),
        );
        builder.add(
            format!("{prefix}.cooling_indicator"),
            activity_indicator(thermostat.activity, BuildingHvacRoomActivityV1::Cooling),
        );
        builder.add(
            format!("{prefix}.fan_only_indicator"),
            activity_indicator(thermostat.activity, BuildingHvacRoomActivityV1::FanOnly),
        );
        builder.add(
            format!("{prefix}.local_temperature_celsius"),
            thermostat.local_temperature_celsius,
        );
        builder.add(format!("{prefix}.local_relative_humidity_percent"), None);
        builder.add(
            format!("{prefix}.heating_setpoint_celsius"),
            thermostat.heating_setpoint_celsius,
        );
        builder.add(
            format!("{prefix}.cooling_setpoint_celsius"),
            thermostat.cooling_setpoint_celsius,
        );
        builder.add(
            format!("{prefix}.minimum_heating_setpoint_celsius"),
            thermostat.minimum_heating_setpoint_celsius,
        );
        builder.add(
            format!("{prefix}.maximum_heating_setpoint_celsius"),
            thermostat.maximum_heating_setpoint_celsius,
        );
        builder.add(
            format!("{prefix}.minimum_cooling_setpoint_celsius"),
            thermostat.minimum_cooling_setpoint_celsius,
        );
        builder.add(
            format!("{prefix}.maximum_cooling_setpoint_celsius"),
            thermostat.maximum_cooling_setpoint_celsius,
        );
        builder.add(
            format!("{prefix}.minimum_deadband_celsius"),
            thermostat.minimum_deadband_celsius,
        );
        builder.add(
            format!("{prefix}.control_sequence"),
            thermostat.control_sequence.map(f32::from),
        );
        builder.add(
            format!("{prefix}.running_mode"),
            thermostat.running_mode.map(f32::from),
        );
        builder.add(
            format!("{prefix}.running_state_bitmap"),
            thermostat.running_state.map(f32::from),
        );
        for measurement in [
            "electric_power_kilowatts",
            "electric_energy_kilowatt_hours",
            "gas_power_kilowatts",
            "gas_energy_kilowatt_hours",
            "delivered_heating_power_kilowatts_thermal",
            "delivered_cooling_power_kilowatts_thermal",
        ] {
            builder.add(format!("{prefix}.{measurement}"), None);
        }
        builder.add(
            format!("{prefix}.report_age_seconds"),
            thermostat
                .observed_at
                .map(|observed_at| now.saturating_sub(observed_at) as f32),
        );
        builder.add(
            format!("{prefix}.command_pending_indicator"),
            Some(binary_indicator(thermostat.pending_write.is_some())),
        );
        builder.add(format!("{prefix}.last_command_succeeded_indicator"), None);
        for (label, seconds) in [
            ("5m", 5 * 60),
            ("15m", 15 * 60),
            ("30m", 30 * 60),
            ("60m", 60 * 60),
        ] {
            let (heating, cooling, fan, idle, _) =
                thermostat_window_statistics(state, device, now, seconds);
            builder.add(
                format!("{prefix}.heating_runtime_fraction_{label}"),
                heating,
            );
            builder.add(
                format!("{prefix}.cooling_runtime_fraction_{label}"),
                cooling,
            );
            builder.add(format!("{prefix}.fan_runtime_fraction_{label}"), fan);
            builder.add(format!("{prefix}.idle_runtime_fraction_{label}"), idle);
            builder.add(
                format!("{prefix}.signed_runtime_fraction_{label}"),
                heating
                    .zip(cooling)
                    .map(|(heating, cooling)| heating - cooling),
            );
        }
        for (label, seconds) in [("1h", 60 * 60), ("24h", 24 * 60 * 60)] {
            let (_, _, _, _, starts) = thermostat_window_statistics(state, device, now, seconds);
            builder.add(
                format!("{prefix}.equipment_starts_{label}"),
                starts.map(|value| value as f32),
            );
        }
        let history = state
            .feature_history
            .iter()
            .rev()
            .filter_map(|observation| {
                observation
                    .thermostats
                    .iter()
                    .find(|entry| entry.thermostat == device)
                    .map(|entry| (observation.observed_at, entry))
            })
            .collect::<Vec<_>>();
        let seconds_since =
            |changed: fn(&ThermostatFeatureObservation, &ThermostatFeatureObservation) -> bool| {
                history
                    .windows(2)
                    .find(|pair| changed(pair[1].1, pair[0].1))
                    .map(|pair| now.saturating_sub(pair[0].0) as f32)
            };
        builder.add(
            format!("{prefix}.seconds_since_state_transition"),
            seconds_since(|older, newer| older.activity != newer.activity),
        );
        builder.add(
            format!("{prefix}.seconds_since_setpoint_change"),
            seconds_since(|older, newer| {
                older.heating_setpoint_celsius != newer.heating_setpoint_celsius
                    || older.cooling_setpoint_celsius != newer.cooling_setpoint_celsius
            }),
        );
        for (label, seconds) in [("15m", 15 * 60), ("60m", 60 * 60)] {
            let starts_at = now.saturating_sub(seconds);
            let pending: Vec<_> = history
                .iter()
                .filter(|(observed_at, _)| *observed_at >= starts_at)
                .collect();
            builder.add(
                format!("{prefix}.command_pending_fraction_{label}"),
                (!pending.is_empty()).then(|| {
                    pending
                        .iter()
                        .filter(|(_, observation)| observation.write_pending)
                        .count() as f32
                        / pending.len() as f32
                }),
            );
        }
        for (label, seconds) in [("15m", 15 * 60), ("30m", 30 * 60), ("60m", 60 * 60)] {
            let previous = thermostat_observation_at(state, device, now.saturating_sub(seconds));
            builder.add(
                format!("{prefix}.local_temperature_celsius_lag_{label}"),
                previous.and_then(|value| value.local_temperature_celsius),
            );
            builder.add(
                format!("{prefix}.local_temperature_change_celsius_{label}"),
                thermostat
                    .local_temperature_celsius
                    .zip(previous.and_then(|value| value.local_temperature_celsius))
                    .map(|(current, previous)| current - previous),
            );
            builder.add(
                format!("{prefix}.active_setpoint_delta_change_celsius_{label}"),
                previous.map(|previous| active_delta - previous.active_setpoint_delta_celsius),
            );
            builder.add(
                format!("{prefix}.heating_setpoint_change_celsius_{label}"),
                thermostat
                    .heating_setpoint_celsius
                    .zip(previous.and_then(|value| value.heating_setpoint_celsius))
                    .map(|(current, previous)| current - previous),
            );
            builder.add(
                format!("{prefix}.cooling_setpoint_change_celsius_{label}"),
                thermostat
                    .cooling_setpoint_celsius
                    .zip(previous.and_then(|value| value.cooling_setpoint_celsius))
                    .map(|(current, previous)| current - previous),
            );
        }
    }

    for room in &state.rooms {
        let endpoint = room.configuration.control_endpoint;
        let prefix = format!("room.{endpoint}");
        let temperature = room.state.temperature_celsius;
        let humidity = room.state.relative_humidity_percent;
        builder.add(format!("{prefix}.temperature_celsius"), temperature);
        builder.add(format!("{prefix}.relative_humidity_percent"), humidity);
        let dew_point = temperature
            .zip(humidity)
            .and_then(|(temperature, humidity)| {
                if humidity <= 0.0 {
                    return None;
                }
                let gamma = (humidity / 100.0).ln() + 17.625 * temperature / (243.04 + temperature);
                let dew = 243.04 * gamma / (17.625 - gamma);
                dew.is_finite().then_some(dew)
            });
        let pressure = state
            .weather
            .current
            .as_ref()
            .map(|current| current.conditions.surface_pressure_hectopascals * 100.0)
            .unwrap_or(101_325.0);
        let humidity_ratio = dew_point.and_then(|dew_point| {
            let vapor = saturation_vapor_pressure_pascals(dew_point)?;
            (vapor < pressure).then_some(0.621_945 * vapor / (pressure - vapor))
        });
        builder.add(format!("{prefix}.dew_point_temperature_celsius"), dew_point);
        builder.add(format!("{prefix}.humidity_ratio_kg_per_kg"), humidity_ratio);
        builder.add(
            format!("{prefix}.effective_heating_setpoint_celsius"),
            room.state.effective_heating_setpoint_celsius,
        );
        builder.add(
            format!("{prefix}.effective_cooling_setpoint_celsius"),
            room.state.effective_cooling_setpoint_celsius,
        );
        builder.add(
            format!("{prefix}.active_setpoint_delta_celsius"),
            Some(active_thermostat_setpoint_delta_celsius(
                room.state.activity,
                room.state.effective_heating_setpoint_celsius,
                room.state.effective_cooling_setpoint_celsius,
                room.state.temperature_celsius.into_iter(),
            )),
        );
        builder.add(
            format!("{prefix}.heating_indicator"),
            activity_indicator(room.state.activity, BuildingHvacRoomActivityV1::Heating),
        );
        builder.add(
            format!("{prefix}.cooling_indicator"),
            activity_indicator(room.state.activity, BuildingHvacRoomActivityV1::Cooling),
        );
        builder.add(
            format!("{prefix}.fan_only_indicator"),
            activity_indicator(room.state.activity, BuildingHvacRoomActivityV1::FanOnly),
        );
        builder.add(
            format!("{prefix}.fresh_temperature_sensor_count"),
            Some(f32::from(room.state.fresh_temperature_sensor_count)),
        );
        builder.add(
            format!("{prefix}.configured_temperature_sensor_count"),
            Some(f32::from(room.state.configured_temperature_sensor_count)),
        );
        builder.add(
            format!("{prefix}.fresh_humidity_sensor_count"),
            Some(f32::from(room.state.fresh_humidity_sensor_count)),
        );
        builder.add(
            format!("{prefix}.configured_humidity_sensor_count"),
            Some(f32::from(room.state.configured_humidity_sensor_count)),
        );
        builder.add(
            format!("{prefix}.control_revision"),
            Some(room.control_revision.min(16_777_216) as f32),
        );
        if let Some(statistics) = &room.statistics {
            let duration = statistics.ends_before.saturating_sub(statistics.starts_at) as f32;
            builder.add(
                format!("{prefix}.statistics_heating_runtime_fraction"),
                (duration > 0.0).then_some(statistics.heating_active_seconds as f32 / duration),
            );
            builder.add(
                format!("{prefix}.statistics_cooling_runtime_fraction"),
                (duration > 0.0).then_some(statistics.cooling_active_seconds as f32 / duration),
            );
            builder.add(
                format!("{prefix}.statistics_fan_runtime_fraction"),
                (duration > 0.0).then_some(statistics.fan_only_active_seconds as f32 / duration),
            );
        } else {
            builder.add(
                format!("{prefix}.statistics_heating_runtime_fraction"),
                None,
            );
            builder.add(
                format!("{prefix}.statistics_cooling_runtime_fraction"),
                None,
            );
            builder.add(format!("{prefix}.statistics_fan_runtime_fraction"), None);
        }
        for (label, seconds) in [
            ("5m", 5 * 60),
            ("15m", 15 * 60),
            ("30m", 30 * 60),
            ("60m", 60 * 60),
            ("120m", 120 * 60),
        ] {
            let at = now.saturating_sub(seconds);
            let previous = room_observation_at(state, endpoint, at);
            let persisted = room.recent_conditions.iter().rev().find(|period| {
                period.starts_at <= at
                    && at
                        < period
                            .starts_at
                            .saturating_add(u64::from(period.duration_seconds))
            });
            let previous_temperature = previous
                .and_then(|value| value.temperature_celsius)
                .or_else(|| persisted.and_then(|value| value.temperature_celsius));
            let previous_humidity = previous
                .and_then(|value| value.relative_humidity_percent)
                .or_else(|| persisted.and_then(|value| value.relative_humidity_percent));
            builder.add(
                format!("{prefix}.temperature_celsius_lag_{label}"),
                previous_temperature,
            );
            if seconds <= 60 * 60 {
                builder.add(
                    format!("{prefix}.temperature_slope_celsius_per_hour_{label}"),
                    temperature
                        .zip(previous_temperature)
                        .map(|(current, previous)| (current - previous) * 3_600.0 / seconds as f32),
                );
            }
            if seconds == 15 * 60 || seconds == 60 * 60 {
                builder.add(
                    format!("{prefix}.relative_humidity_slope_percent_per_hour_{label}"),
                    humidity
                        .zip(previous_humidity)
                        .map(|(current, previous)| (current - previous) * 3_600.0 / seconds as f32),
                );
            }
            if seconds == 15 * 60 || seconds == 60 * 60 {
                builder.add(
                    format!("{prefix}.heating_setpoint_change_celsius_{label}"),
                    room.state
                        .effective_heating_setpoint_celsius
                        .zip(
                            previous
                                .and_then(|value| value.effective_heating_setpoint_celsius)
                                .or_else(|| {
                                    persisted
                                        .and_then(|value| value.effective_heating_setpoint_celsius)
                                }),
                        )
                        .map(|(current, previous)| current - previous),
                );
                builder.add(
                    format!("{prefix}.cooling_setpoint_change_celsius_{label}"),
                    room.state
                        .effective_cooling_setpoint_celsius
                        .zip(
                            previous
                                .and_then(|value| value.effective_cooling_setpoint_celsius)
                                .or_else(|| {
                                    persisted
                                        .and_then(|value| value.effective_cooling_setpoint_celsius)
                                }),
                        )
                        .map(|(current, previous)| current - previous),
                );
            }
        }
        builder.add(format!("{prefix}.occupancy_state_normalized"), None);
        builder.add(format!("{prefix}.occupancy_fraction_15m"), None);
        builder.add(format!("{prefix}.occupancy_fraction_60m"), None);
        builder.add(format!("{prefix}.occupant_count"), None);
        builder.add(format!("{prefix}.window_open_fraction_15m"), None);
        builder.add(format!("{prefix}.window_open_fraction_60m"), None);
        builder.add(format!("{prefix}.override_active_indicator"), None);
        builder.add(format!("{prefix}.override_remaining_seconds"), None);
        builder.add(
            format!("{prefix}.comfort_or_savings_normalized"),
            Some(room.control.comfort_or_savings_normalized),
        );
        builder.add(
            format!("{prefix}.recent_delivered_heating_kilowatt_hours_thermal"),
            None,
        );
        builder.add(
            format!("{prefix}.recent_delivered_cooling_kilowatt_hours_thermal"),
            None,
        );
        add_room_air_measurements(&mut builder, &prefix, room, now);
    }

    let target_prefix = "target";
    builder.add(
        format!("{target_prefix}.temperature_celsius"),
        Some(target_temperature),
    );
    builder.add(
        format!("{target_prefix}.preferred_heating_temperature_celsius"),
        Some(target_room.control.preferred_heating_temperature_celsius),
    );
    builder.add(
        format!("{target_prefix}.preferred_cooling_temperature_celsius"),
        Some(target_room.control.preferred_cooling_temperature_celsius),
    );
    builder.add(
        format!("{target_prefix}.comfort_or_savings_normalized"),
        Some(target_room.control.comfort_or_savings_normalized),
    );
    builder.add(
        format!("{target_prefix}.requested_mode_signed"),
        Some(match target_room.control.operating_preference {
            BuildingHvacRoomOperatingPreferenceV1::Heat => 1.0,
            BuildingHvacRoomOperatingPreferenceV1::Cool => -1.0,
            BuildingHvacRoomOperatingPreferenceV1::Auto
            | BuildingHvacRoomOperatingPreferenceV1::Off => 0.0,
        }),
    );
    for (name, preference) in [
        ("auto", BuildingHvacRoomOperatingPreferenceV1::Auto),
        ("heat", BuildingHvacRoomOperatingPreferenceV1::Heat),
        ("cool", BuildingHvacRoomOperatingPreferenceV1::Cool),
        ("off", BuildingHvacRoomOperatingPreferenceV1::Off),
    ] {
        builder.add(
            format!("{target_prefix}.requested_mode_{name}_indicator"),
            Some(binary_indicator(
                target_room.control.operating_preference == preference,
            )),
        );
    }
    builder.add(format!("{target_prefix}.override_active_indicator"), None);
    builder.add(format!("{target_prefix}.override_remaining_seconds"), None);
    builder.add(
        format!("{target_prefix}.below_heating_comfort_degree_minutes_celsius"),
        target_room
            .statistics
            .as_ref()
            .map(|value| value.below_heating_comfort_degree_minutes_celsius),
    );
    builder.add(
        format!("{target_prefix}.above_cooling_comfort_degree_minutes_celsius"),
        target_room
            .statistics
            .as_ref()
            .map(|value| value.above_cooling_comfort_degree_minutes_celsius),
    );
    for (label, seconds) in [("15m", 15 * 60), ("30m", 30 * 60), ("60m", 60 * 60)] {
        let thermostat = state.thermostats[target_room.thermostat_index]
            .configuration
            .thermostat;
        let (heating, cooling, _, _, _) =
            thermostat_window_statistics(state, thermostat, now, seconds);
        builder.add(
            format!("{target_prefix}.recent_delivered_heating_runtime_hours_{label}"),
            heating.map(|fraction| fraction * seconds as f32 / 3_600.0),
        );
        builder.add(
            format!("{target_prefix}.recent_delivered_cooling_runtime_hours_{label}"),
            cooling.map(|fraction| fraction * seconds as f32 / 3_600.0),
        );
    }
    for (label, horizon) in [
        (
            "15m",
            BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
        ),
        ("30m", BuildingHvacThermalPredictionHorizonV1::ThirtyMinutes),
        ("60m", BuildingHvacThermalPredictionHorizonV1::SixtyMinutes),
    ] {
        builder.add(
            format!("{target_prefix}.predicted_temperature_change_celsius_{label}"),
            target_room
                .machine_learning
                .predictions
                .iter()
                .find(|prediction| prediction.horizon == horizon)
                .map(|prediction| prediction.temperature_change_celsius),
        );
    }
    for (label, horizon) in [
        (
            "15m",
            BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
        ),
        ("30m", BuildingHvacThermalPredictionHorizonV1::ThirtyMinutes),
        ("60m", BuildingHvacThermalPredictionHorizonV1::SixtyMinutes),
    ] {
        builder.add(
            format!("{target_prefix}.prediction_residual_celsius_{label}"),
            target_room
                .prediction_residuals
                .iter()
                .rev()
                .find(|residual| residual.horizon == horizon)
                .map(|residual| residual.residual_celsius),
        );
        let residuals: Vec<_> = target_room
            .prediction_residuals
            .iter()
            .filter(|residual| residual.horizon == horizon)
            .map(|residual| residual.residual_celsius.abs())
            .collect();
        builder.add(
            format!("{target_prefix}.prediction_rolling_mae_celsius_24h_{label}"),
            (!residuals.is_empty()).then(|| {
                residuals.iter().copied().map(f64::from).sum::<f64>() as f32
                    / residuals.len() as f32
            }),
        );
    }
    let planned = target_room
        .plan
        .as_ref()
        .and_then(|plan| plan.periods.first());
    builder.add(
        format!("{target_prefix}.planned_heating_setpoint_celsius"),
        planned.and_then(|period| period.heating_setpoint_celsius),
    );
    builder.add(
        format!("{target_prefix}.planned_cooling_setpoint_celsius"),
        planned.and_then(|period| period.cooling_setpoint_celsius),
    );
    builder.add(
        format!("{target_prefix}.previous_activity_signed"),
        room_observation_at(state, target_endpoint, now.saturating_sub(60))
            .and_then(|room| activity_value(room.activity)),
    );

    add_weather_features(&mut builder, state, now);
    builder.add(
        "weather.local_outdoor_temperature_celsius",
        state
            .local_outdoor
            .as_ref()
            .and_then(|outdoor| outdoor.temperature)
            .filter(|reading| reading.observed_at <= now && reading.valid_until > now)
            .map(|reading| reading.temperature_celsius),
    );

    for name in [
        "current_price_per_kilowatt_hour",
        "forecast_price_plus_15m_per_kilowatt_hour",
        "forecast_price_plus_30m_per_kilowatt_hour",
        "forecast_price_plus_60m_per_kilowatt_hour",
        "forecast_price_plus_2h_per_kilowatt_hour",
        "forecast_price_plus_3h_per_kilowatt_hour",
        "forecast_price_plus_6h_per_kilowatt_hour",
        "forecast_price_plus_12h_per_kilowatt_hour",
        "forecast_price_plus_24h_per_kilowatt_hour",
        "minimum_price_next_6h_per_kilowatt_hour",
        "maximum_price_next_6h_per_kilowatt_hour",
        "minimum_price_next_24h_per_kilowatt_hour",
        "maximum_price_next_24h_per_kilowatt_hour",
        "current_carbon_intensity_kilograms_per_kilowatt_hour",
        "forecast_carbon_intensity_plus_60m_kilograms_per_kilowatt_hour",
        "forecast_carbon_intensity_plus_6h_kilograms_per_kilowatt_hour",
        "demand_response_active_indicator",
        "seconds_to_demand_response_transition",
        "building_electric_demand_kilowatts",
        "building_electric_demand_mean_15m_kilowatts",
        "building_electric_demand_mean_60m_kilowatts",
        "building_peak_window_demand_kilowatts",
    ] {
        builder.add(format!("utility.{name}"), None);
    }

    for measurement in [
        "supply_air_temperature_celsius",
        "return_air_temperature_celsius",
        "mixed_air_temperature_celsius",
        "supply_airflow_cubic_meters_per_second",
        "outdoor_airflow_cubic_meters_per_second",
        "supply_fan_speed_normalized",
        "return_fan_speed_normalized",
        "duct_static_pressure_pascals",
        "duct_static_pressure_setpoint_pascals",
        "outdoor_air_damper_position_normalized",
        "return_air_damper_position_normalized",
        "zone_damper_mean_position_normalized",
        "heating_valve_position_normalized",
        "cooling_valve_position_normalized",
        "compressor_capacity_normalized",
        "heating_stage",
        "cooling_stage",
        "supply_water_temperature_celsius",
        "return_water_temperature_celsius",
        "pump_speed_normalized",
        "electric_power_kilowatts",
        "electric_energy_kilowatt_hours",
        "gas_power_kilowatts",
        "gas_energy_kilowatt_hours",
        "delivered_heating_power_kilowatts_thermal",
        "delivered_cooling_power_kilowatts_thermal",
        "coefficient_of_performance",
        "active_fault_count",
    ] {
        builder.add(format!("equipment.central.current.{measurement}"), None);
        builder.add(format!("equipment.central.mean_15m.{measurement}"), None);
        builder.add(format!("equipment.central.mean_60m.{measurement}"), None);
        builder.add(format!("equipment.central.change_15m.{measurement}"), None);
    }

    for input in &state.external_features.inputs {
        if input.observed_at <= now
            && input.valid_until > now
            && input.is_well_formed()
            && let Some(feature) = builder
                .values
                .iter_mut()
                .find(|feature| feature.name == input.feature_name)
            && feature.value.is_none()
        {
            feature.value = Some(input.value);
        }
    }
    for suffix in ["override_active_indicator", "override_remaining_seconds"] {
        let room_feature_name = format!("room.{target_endpoint}.{suffix}");
        let target_feature_name = format!("target.{suffix}");
        let value = builder
            .values
            .iter()
            .find(|feature| feature.name == room_feature_name)
            .and_then(|feature| feature.value);
        if let Some(target) = builder
            .values
            .iter_mut()
            .find(|feature| feature.name == target_feature_name)
        {
            target.value = value;
        }
    }

    builder.finish()
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
        let Some(current_temperature) = features.value("target.temperature_celsius") else {
            continue;
        };
        let endpoint = state.rooms[room_index].configuration.control_endpoint;
        let predictions = |horizon| {
            state.rooms[room_index]
                .machine_learning
                .predictions
                .iter()
                .find(|prediction| prediction.horizon == horizon)
                .map(|prediction| prediction.temperature_change_celsius)
        };
        let predicted_15 = predictions(BuildingHvacThermalPredictionHorizonV1::FifteenMinutes);
        let predicted_30 = predictions(BuildingHvacThermalPredictionHorizonV1::ThirtyMinutes);
        let predicted_60 = predictions(BuildingHvacThermalPredictionHorizonV1::SixtyMinutes);
        let mut residuals = Vec::new();
        for pending in &mut state.rooms[room_index].pending_features {
            let elapsed = now.saturating_sub(pending.observed_at);
            let change = (current_temperature - pending.temperature_celsius).clamp(
                -BUILDING_HVAC_ML_MAXIMUM_PREDICTED_CHANGE_CELSIUS,
                BUILDING_HVAC_ML_MAXIMUM_PREDICTED_CHANGE_CELSIUS,
            );
            let mut sample = BuildingHvacMachineLearningSampleV1 {
                observed_at: pending.observed_at,
                room_endpoint: endpoint,
                features: pending.features.clone(),
                temperature_change_15_minutes_celsius: None,
                temperature_change_30_minutes_celsius: None,
                temperature_change_60_minutes_celsius: None,
            };
            if elapsed >= 15 * 60 && !pending.persisted_15 {
                if elapsed <= 15 * 60 + ML_TARGET_MAXIMUM_DELAY_SECONDS {
                    sample.temperature_change_15_minutes_celsius = Some(change);
                    if let Some(predicted) = pending.predicted_change_15_minutes_celsius {
                        residuals.push(PredictionResidualObservation {
                            observed_at: now,
                            horizon: BuildingHvacThermalPredictionHorizonV1::FifteenMinutes,
                            residual_celsius: change - predicted,
                        });
                    }
                }
                pending.persisted_15 = true;
            }
            if elapsed >= 30 * 60 && !pending.persisted_30 {
                if elapsed <= 30 * 60 + ML_TARGET_MAXIMUM_DELAY_SECONDS {
                    sample.temperature_change_30_minutes_celsius = Some(change);
                    if let Some(predicted) = pending.predicted_change_30_minutes_celsius {
                        residuals.push(PredictionResidualObservation {
                            observed_at: now,
                            horizon: BuildingHvacThermalPredictionHorizonV1::ThirtyMinutes,
                            residual_celsius: change - predicted,
                        });
                    }
                }
                pending.persisted_30 = true;
            }
            if elapsed >= 60 * 60 && !pending.persisted_60 {
                if elapsed <= 60 * 60 + ML_TARGET_MAXIMUM_DELAY_SECONDS {
                    sample.temperature_change_60_minutes_celsius = Some(change);
                    if let Some(predicted) = pending.predicted_change_60_minutes_celsius {
                        residuals.push(PredictionResidualObservation {
                            observed_at: now,
                            horizon: BuildingHvacThermalPredictionHorizonV1::SixtyMinutes,
                            residual_celsius: change - predicted,
                        });
                    }
                }
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
            .prediction_residuals
            .extend(residuals);
        let oldest_residual = now.saturating_sub(24 * 60 * 60);
        state.rooms[room_index]
            .prediction_residuals
            .retain(|residual| residual.observed_at >= oldest_residual);
        state.rooms[room_index]
            .pending_features
            .push(PendingFeatures {
                observed_at: now,
                temperature_celsius: current_temperature,
                features: features.compact(),
                predicted_change_15_minutes_celsius: predicted_15,
                predicted_change_30_minutes_celsius: predicted_30,
                predicted_change_60_minutes_celsius: predicted_60,
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
    feature_names: &[String],
) -> bool {
    let samples =
        BuildingHvacMachineLearningHistory::load_training_samples(endpoint, now, feature_names);
    if samples.len() < BUILDING_HVAC_ML_MINIMUM_TRAINING_SAMPLES {
        return true;
    }
    client
        .try_train_all(now, feature_names.to_vec(), samples)
        .is_ok()
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
        capture_feature_observation(&mut state, now);
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
        let samples = match state.last_ml_sample_boundary {
            None => {
                // Establish the cadence without labeling a startup observation
                // against a shorter first interval.
                state.last_ml_sample_boundary = Some(sample_boundary);
                Vec::new()
            }
            Some(previous) if previous != sample_boundary => {
                let samples = update_machine_learning_samples(&mut state, now);
                state.last_ml_sample_boundary = Some(sample_boundary);
                samples
            }
            Some(_) => Vec::new(),
        };
        let prediction_minute = now - now % 60;
        if state.last_prediction_minute != Some(prediction_minute) {
            request_predictions(&mut state, now);
            state.last_prediction_minute = Some(prediction_minute);
        }
        let training = (!state.machine_learning_client.training_pending())
            .then(|| select_training_room(&mut state, now))
            .flatten()
            .and_then(|endpoint| {
                let room_index = state
                    .rooms
                    .iter()
                    .position(|room| room.configuration.control_endpoint == endpoint)?;
                machine_learning_features(&state, room_index, now)
                    .map(|features| (endpoint, features.feature_names()))
            });
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
    if let Some((endpoint, feature_names)) = training
        && !queue_training(&client, endpoint, now, &feature_names)
        && let Some(room) = shared
            .borrow_mut()
            .rooms
            .iter_mut()
            .find(|room| room.configuration.control_endpoint == endpoint)
    {
        room.last_training_at = None;
    }
    apply_thermostat_decisions(shared);
    report_changed_rooms(shared);
}

fn handle_wakeup(context: &mut Box<dyn Any>) {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .expect("invalid building climate wake-up context");
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
        .expect("invalid building climate shutdown context");
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
        .expect("invalid building climate weather timer context");
    if !shared.borrow().weather_server_up {
        libertas_timer_cancel(timer);
        return;
    }
    let endpoint = shared.borrow().weather_endpoint;
    libertas_endpoint_subscribe_request(endpoint, &weather_request(shared));
    libertas_timer_update_interval(timer, absolute_ticks(now_ticks, WEATHER_RETRY_SECONDS));
}

fn external_feature_retry_timer(timer: u32, now_ticks: u64, context: &mut Box<dyn Any>) {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .expect("invalid building climate external-feature timer context");
    if !shared.borrow().external_feature_server_up {
        libertas_timer_cancel(timer);
        return;
    }
    subscribe_external_features(shared);
    libertas_timer_update_interval(
        timer,
        absolute_ticks(now_ticks, EXTERNAL_FEATURE_RETRY_SECONDS),
    );
}

fn evaluation_timer(timer: u32, now_ticks: u64, context: &mut Box<dyn Any>) {
    let shared = context
        .downcast_mut::<Rc<RefCell<ControllerState>>>()
        .expect("invalid building climate evaluation timer context");
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
    external_feature_client: Option<BuildingHvacExternalFeatureClientV1>,
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
            last_report: None,
            last_endpoint_report_ticks: None,
            last_condition_boundary: None,
            pending_features: Vec::new(),
            prediction_residuals: Vec::new(),
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
        weather_server_up: true,
        weather_maximum_wait_seconds: BUILDING_HVAC_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
        weather_retry_timer: 0,
        external_feature_endpoint: external_feature_client.map(|client| client.endpoint),
        external_feature_server_up: true,
        external_features: restore_external_features(),
        external_feature_maximum_wait_seconds: EXTERNAL_FEATURE_RETRY_SECONDS,
        external_feature_retry_timer: 0,
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
        feature_history: Vec::new(),
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
    if let Some(client) = external_feature_client {
        libertas_register_endpoint_status_listener::<BuildingHvacExternalFeatureProtocolV1, _>(
            client.endpoint,
            handle_external_feature_endpoint,
            Box::new(Rc::clone(&shared)),
        );
    }
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
    if external_feature_client.is_some() {
        let external_feature_timer = libertas_timer_new_interval(
            0,
            external_feature_retry_timer,
            Box::new(Rc::clone(&shared)),
        );
        shared.borrow_mut().external_feature_retry_timer = external_feature_timer;
    }
    let now_ticks = libertas_get_sys_ticks();
    libertas_timer_new_interval(
        absolute_ticks(now_ticks, EVALUATION_INTERVAL_SECONDS),
        evaluation_timer,
        Box::new(Rc::clone(&shared)),
    );
    request_matter_subscriptions(&shared, outdoor_configuration);
    subscribe_weather(&shared);
    subscribe_external_features(&shared);
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
            solar_elevation_degrees: 35.0,
            solar_azimuth_degrees: 190.0,
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
    fn active_setpoint_delta_preserves_demand_direction_and_magnitude() {
        assert_eq!(
            active_thermostat_setpoint_delta_celsius(
                BuildingHvacRoomActivityV1::Heating,
                Some(21.0),
                Some(24.0),
                [20.0, 18.5].into_iter(),
            ),
            2.5
        );
        assert_eq!(
            active_thermostat_setpoint_delta_celsius(
                BuildingHvacRoomActivityV1::Cooling,
                Some(21.0),
                Some(24.0),
                [25.0, 27.5].into_iter(),
            ),
            -3.5
        );
        for activity in [
            BuildingHvacRoomActivityV1::Idle,
            BuildingHvacRoomActivityV1::FanOnly,
            BuildingHvacRoomActivityV1::Unknown,
        ] {
            assert_eq!(
                active_thermostat_setpoint_delta_celsius(
                    activity,
                    Some(21.0),
                    Some(24.0),
                    [18.0, 28.0].into_iter(),
                ),
                0.0
            );
        }
        assert_eq!(
            active_thermostat_setpoint_delta_celsius(
                BuildingHvacRoomActivityV1::Heating,
                None,
                Some(24.0),
                [18.0].into_iter(),
            ),
            0.0
        );
    }

    #[test]
    fn solar_position_uses_elevation_and_cyclic_azimuth_features() {
        let conditions = outdoor_conditions();
        let mut builder = MachineLearningFeatureBuilder::new(1);
        add_weather_conditions(&mut builder, "weather.test", Some(&conditions), None);
        let features = builder.finish().expect("valid solar feature manifest");

        assert_eq!(
            features.value("weather.test.solar_elevation_degrees"),
            Some(conditions.solar_elevation_degrees)
        );
        let radians = conditions.solar_azimuth_degrees.to_radians();
        assert!(
            (features.value("weather.test.solar_azimuth_sine").unwrap() - radians.sin()).abs()
                < 0.000_001
        );
        assert!(
            (features.value("weather.test.solar_azimuth_cosine").unwrap() - radians.cos()).abs()
                < 0.000_001
        );
        assert!(
            features
                .value("weather.test.solar_azimuth_degrees")
                .is_none()
        );
    }

    #[test]
    fn day_of_year_uses_gregorian_day_and_365_25_cycle() {
        assert_eq!(utc_day_of_year(1_704_067_200), 1);
        assert_eq!(utc_day_of_year(1_709_164_800), 60);
        assert_eq!(utc_day_of_year(1_735_603_200), 366);
        assert_eq!(utc_day_of_year(1_735_689_600), 1);

        let (_, _, sine, cosine) = cyclic_time(1_704_067_200);
        let expected_angle = TAU / 365.25;
        assert!((sine - expected_angle.sin()).abs() < 0.000_001);
        assert!((cosine - expected_angle.cos()).abs() < 0.000_001);
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
        let mut invalid_solar = current;
        invalid_solar.conditions.solar_elevation_degrees = 91.0;
        assert!(!valid_current_weather(&invalid_solar));
        invalid_solar.conditions = outdoor_conditions();
        invalid_solar.conditions.solar_azimuth_degrees = f32::NAN;
        assert!(!valid_current_weather(&invalid_solar));
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
