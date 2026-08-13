//! Libertas Weather Agent
//! Provides application-tailored weather services for Libertas applications.
//! The first implemented service exposes sprinkler weather through a typed
//! endpoint while retaining independently persisted weather sections across
//! provider outages and cursor resets. Future application-specific weather
//! protocols can be added alongside it without turning this crate into a
//! universal weather model.
//!
//! A dedicated standard-library worker owns the reusable HTTPS client and
//! communicates through bounded channels. During normal operation the worker's
//! only Libertas call is `libertas_wake_up`, which is explicitly the
//! cross-thread wake-up primitive. The wake-up callback validates and persists
//! provider results, advances cursor state, and publishes reports on the single
//! Libertas application thread.
//!
//! Startup subscribes to the built-in Libertas Hub location endpoint. A valid
//! persisted location is used while that subscription is recovering; without a
//! cached location, the worker remains idle. Persisted retrieval timestamps
//! preserve each valid weather section's refresh schedule across restarts.
//! Missing, overdue, or future-dated cache entries refresh immediately;
//! otherwise the first request waits only for the remainder of the normal
//! refresh interval.
//! Completed history hours are indexed by start timestamp and reconstructed
//! into the bounded history section during startup. Current conditions and
//! forecast remain independently replaceable singleton records.
//!
//! The Libertas shutdown handler signals the HTTP worker without blocking the
//! application thread. After any bounded in-flight request returns, the worker
//! stops without publishing another result and calls `libertas_shutdown_complete`
//! as its final action. The current Libertas data-write API still has no
//! completion result with which to confirm durable storage.
#![forbid(unsafe_code)]

extern crate alloc;

use std::{
    any::Any,
    cell::{Cell, RefCell},
    io::Read,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use libertas::{
    IndexDirection, IndexedData, LIBERTAS_HUB_ENDPOINT, LibertasDateTime, LibertasEndpoint,
    LibertasEndpointHandlerResult, LibertasEndpointMessage, LibertasEndpointStandardStatus,
    LibertasEndpointStatus, LogLevel, NotificationArgument, OP_ENDPOINT_DATA,
    OP_ENDPOINT_PEER_ALIVE, OP_ENDPOINT_PEER_DOWN, OP_ENDPOINT_PEER_UP, OP_ENDPOINT_REQ,
    OP_ENDPOINT_RSP, OP_ENDPOINT_SUB_REQ, libertas_data_open_indexed,
    libertas_data_read_indexed_range, libertas_data_read_single,
    libertas_data_remove_indexed_records, libertas_data_remove_single, libertas_data_write_indexed,
    libertas_data_write_single, libertas_endpoint_peer_alive, libertas_endpoint_remove_subscriber,
    libertas_endpoint_report, libertas_endpoint_response, libertas_endpoint_subscribe_request,
    libertas_get_sys_ticks, libertas_get_utc_time, libertas_log,
    libertas_register_endpoint_listener, libertas_register_endpoint_status_listener,
    libertas_register_shutdown_handler, libertas_register_wakeup_callback,
    libertas_shutdown_complete, libertas_timer_cancel, libertas_timer_new_interval,
    libertas_timer_update_interval, libertas_wake_up,
};
use libertas_hub::HubProtocol;
use libertas_macros::{
    LibertasAvroDecode, LibertasAvroEncode, LibertasExport, libertas_data_schema,
    libertas_permissions, libertas_string_resources,
};
use libertas_weather::{
    SPRINKLER_CURRENT_FRESHNESS_SECONDS, SPRINKLER_CURRENT_REFRESH_INTERVAL_SECONDS,
    SPRINKLER_FORECAST_FRESHNESS_SECONDS, SPRINKLER_FORECAST_HORIZON_SECONDS,
    SPRINKLER_FORECAST_REFRESH_INTERVAL_SECONDS, SPRINKLER_HISTORY_FRESHNESS_SECONDS,
    SPRINKLER_HISTORY_REFRESH_INTERVAL_SECONDS, SPRINKLER_HISTORY_WINDOW_SECONDS,
    SPRINKLER_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
    SPRINKLER_SUBSCRIPTION_REPLAY_WINDOW_SECONDS, SprinklerCurrentWeatherV1,
    SprinklerWeatherChangeV1, SprinklerWeatherCursorV1, SprinklerWeatherForecastPeriodV1,
    SprinklerWeatherForecastV1, SprinklerWeatherHistoryMetadataV1, SprinklerWeatherHistoryPeriodV2,
    SprinklerWeatherHistoryV2, SprinklerWeatherIncrementalReportV1, SprinklerWeatherProtocolV1,
    SprinklerWeatherRecoveryErrorV1, SprinklerWeatherRecoveryV1, SprinklerWeatherResetReasonV1,
    SprinklerWeatherSectionV1, SprinklerWeatherSnapshotV2, SprinklerWeatherTimeRangeV1,
};
pub use libertas_weather::{SprinklerWeatherLocationV1, SprinklerWeatherPersistentDataV1};
use reqwest::{blocking::Client, redirect::Policy};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
#[allow(dead_code)]
const WEATHER_AGENT_PERMISSIONS: &[&str] = &["libertas.permission.ACCESS_FINE_LOCATION"];
const OPEN_METEO_URL: &str = "https://api.open-meteo.com/v1/forecast";
const HTTP_CONNECT_TIMEOUT_SECONDS: u64 = 5;
const HTTP_REQUEST_TIMEOUT_SECONDS: u64 = 20;
const HTTP_MAX_RESPONSE_BYTES: u64 = 1_048_576;
const PROVIDER_COMMAND_CAPACITY: usize = 4;
const PROVIDER_RESULT_CAPACITY: usize = 4;
const MAX_RECOVERY_PERIODS: usize = 7 * 24;
const MAX_HISTORY_RECORDS_SCANNED: usize = MAX_RECOVERY_PERIODS * 2;
const MAX_REPLAY_CHANGES: usize = 512;
const RETRY_WITHOUT_UTC_SECONDS: u32 = 60;
const HUB_LOCATION_MAX_REPORT_INTERVAL_SECONDS: u32 = 60 * 60;
const HUB_LOCATION_RETRY_SECONDS: u32 = 60;
const LOCATION_EQUALITY_TOLERANCE_DEGREES: f64 = 0.000_001;

/// Weather agent database names
/// Stable resource identifiers and their user-facing descriptions.
pub const APP_STRINGS: [(&str, &str); 6] = [
    (
        "SPRINKLER_WEATHER_HISTORY_METADATA_V1",
        "Sprinkler weather history freshness for %1$s.",
    ),
    (
        "SPRINKLER_WEATHER_HISTORY_PERIODS_V1",
        "Sprinkler weather history periods for %1$s.",
    ),
    (
        "SPRINKLER_CURRENT_WEATHER_V1",
        "Persisted current sprinkler weather for %1$s.",
    ),
    (
        "SPRINKLER_WEATHER_FORECAST_V1",
        "Persisted sprinkler weather forecast for %1$s.",
    ),
    (
        "SPRINKLER_WEATHER_LOCATION_V1",
        "Persisted sprinkler weather location for %1$s.",
    ),
    (
        "libertas.permission.ACCESS_FINE_LOCATION",
        "Use the Hub's location to request the correct local weather forecast.",
    ),
];
const HISTORY_METADATA_RESOURCE: &str = APP_STRINGS[0].0;
const HISTORY_PERIODS_RESOURCE: &str = APP_STRINGS[1].0;
const CURRENT_RESOURCE: &str = APP_STRINGS[2].0;
const FORECAST_RESOURCE: &str = APP_STRINGS[3].0;
const LOCATION_RESOURCE: &str = APP_STRINGS[4].0;

/// Sprinkler weather endpoint server
/// Configures the agent's sprinkler endpoint through which applications request
/// current data or establish incremental subscriptions.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWeatherEndpointServerV1 {
    /// Sprinkler weather endpoint
    /// The server endpoint for sprinkler weather. Both one-shot and
    /// subscription clients request weather through this endpoint.
    #[libertas_endpoint_schema(SprinklerWeatherProtocolV1)]
    #[libertas_endpoint_server]
    #[libertas_permissions(WEATHER_AGENT_PERMISSIONS)]
    #[libertas_ui_header]
    pub endpoint: LibertasEndpoint,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ProviderLocation {
    latitude_degrees: f64,
    longitude_degrees: f64,
}

#[derive(Clone, Copy)]
enum ProviderCommand {
    RefreshCurrent { location: ProviderLocation },
    RefreshHourly { location: ProviderLocation },
    Shutdown,
}

enum ProviderMessage {
    Current {
        location: ProviderLocation,
        result: Result<SprinklerCurrentWeatherV1, String>,
    },
    Hourly {
        location: ProviderLocation,
        history: Result<SprinklerWeatherHistoryV2, String>,
        forecast: Result<SprinklerWeatherForecastV1, String>,
    },
}

struct ProviderWakeupContext {
    shared: Rc<RefCell<WeatherServerState>>,
    location: Rc<Cell<Option<ProviderLocation>>>,
    results: Receiver<ProviderMessage>,
}

struct ProviderShutdownContext {
    commands: SyncSender<ProviderCommand>,
    stop_requested: Arc<AtomicBool>,
}

struct ProviderRuntime {
    commands: SyncSender<ProviderCommand>,
    results: Receiver<ProviderMessage>,
    stop_requested: Arc<AtomicBool>,
}

#[derive(Clone, Copy)]
enum ProviderRefreshKind {
    Current,
    Hourly,
}

struct RefreshTimerContext {
    commands: SyncSender<ProviderCommand>,
    location: Rc<Cell<Option<ProviderLocation>>>,
    refresh_kind: ProviderRefreshKind,
    interval_seconds: u32,
}

#[derive(Clone)]
struct ProviderControl {
    commands: SyncSender<ProviderCommand>,
    location: Rc<Cell<Option<ProviderLocation>>>,
    current_timer: u32,
    hourly_timer: u32,
}

struct LocationSubscriptionState {
    weather: Rc<RefCell<WeatherServerState>>,
    provider: Option<ProviderControl>,
    location: Option<SprinklerWeatherLocationV1>,
    retry_timer: u32,
    hub_server_up: bool,
    subscription_ready: bool,
}

#[derive(Serialize)]
struct OpenMeteoCurrentQuery {
    latitude: f64,
    longitude: f64,
    current: &'static str,
    timeformat: &'static str,
    wind_speed_unit: &'static str,
    precipitation_unit: &'static str,
}

#[derive(Serialize)]
struct OpenMeteoHourlyQuery {
    latitude: f64,
    longitude: f64,
    hourly: &'static str,
    past_hours: u16,
    forecast_hours: u16,
    timeformat: &'static str,
    wind_speed_unit: &'static str,
    precipitation_unit: &'static str,
}

#[derive(Deserialize)]
struct OpenMeteoCurrentResponse {
    current: OpenMeteoCurrent,
}

#[derive(Deserialize)]
struct OpenMeteoCurrent {
    time: u64,
    interval: u32,
    temperature_2m: Option<f32>,
    relative_humidity_2m: Option<f32>,
    precipitation: Option<f32>,
    et0_fao_evapotranspiration: Option<f32>,
    wind_speed_10m: Option<f32>,
    wind_gusts_10m: Option<f32>,
}

#[derive(Deserialize)]
struct OpenMeteoHourlyResponse {
    hourly: OpenMeteoHourly,
}

#[derive(Deserialize)]
struct OpenMeteoHourly {
    time: Vec<u64>,
    temperature_2m: Vec<Option<f32>>,
    relative_humidity_2m: Vec<Option<f32>>,
    precipitation_probability: Vec<Option<f32>>,
    precipitation: Vec<Option<f32>>,
    et0_fao_evapotranspiration: Vec<Option<f32>>,
    wind_speed_10m: Vec<Option<f32>>,
    wind_gusts_10m: Vec<Option<f32>>,
}

#[derive(Clone)]
struct JournalEntry {
    recorded_at_ticks: u64,
    report: SprinklerWeatherIncrementalReportV1,
}

struct WeatherServerState {
    endpoint: LibertasEndpoint,
    site_location: Option<SprinklerWeatherLocationV1>,
    cursor: Option<SprinklerWeatherCursorV1>,
    snapshot: SprinklerWeatherSnapshotV2,
    journal: Vec<JournalEntry>,
    peer_alive_timer: u32,
    next_peer_alive_ticks: Option<u64>,
}

struct PreparedResponse {
    message: SprinklerWeatherProtocolV1,
    accepted: bool,
}

struct ChangePublication {
    report: Option<SprinklerWeatherProtocolV1>,
}

fn unix_time_seconds() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| String::from("system time is earlier than the Unix epoch"))
}

fn build_http_client() -> Result<Client, String> {
    Client::builder()
        .connect_timeout(Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECONDS))
        .timeout(Duration::from_secs(HTTP_REQUEST_TIMEOUT_SECONDS))
        .redirect(Policy::limited(3))
        .user_agent(concat!(
            "libertas-weather-server/",
            env!("CARGO_PKG_VERSION")
        ))
        .https_only(true)
        .tls_backend_rustls()
        .gzip(true)
        .build()
        .map_err(|error| format!("failed to build HTTP client: {error}"))
}

fn read_json_response<T: DeserializeOwned>(
    response: reqwest::blocking::Response,
) -> Result<T, String> {
    let response = response
        .error_for_status()
        .map_err(|error| format!("Open-Meteo HTTP status error: {error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > HTTP_MAX_RESPONSE_BYTES)
    {
        return Err(String::from("Open-Meteo response exceeds size limit"));
    }

    let mut body = Vec::new();
    response
        .take(HTTP_MAX_RESPONSE_BYTES.saturating_add(1))
        .read_to_end(&mut body)
        .map_err(|error| format!("failed to read Open-Meteo response: {error}"))?;
    if body.len() as u64 > HTTP_MAX_RESPONSE_BYTES {
        return Err(String::from("Open-Meteo response exceeds size limit"));
    }

    serde_json::from_slice(&body)
        .map_err(|error| format!("failed to decode Open-Meteo response: {error}"))
}

fn required_measurement(value: Option<f32>, name: &str) -> Result<f32, String> {
    let value = value.ok_or_else(|| format!("Open-Meteo omitted {name}"))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("Open-Meteo returned non-finite {name}"))
    }
}

fn required_nonnegative_measurement(value: Option<f32>, name: &str) -> Result<f32, String> {
    let value = required_measurement(value, name)?;
    if value >= 0.0 {
        Ok(value)
    } else {
        Err(format!("Open-Meteo returned negative {name}"))
    }
}

fn required_percentage(value: Option<f32>, name: &str) -> Result<u8, String> {
    let value = required_nonnegative_measurement(value, name)?;
    if value > 100.0 {
        return Err(format!("Open-Meteo {name} exceeds 100 percent"));
    }
    Ok(value.round() as u8)
}

fn build_current(
    current: OpenMeteoCurrent,
    retrieved_at: u64,
) -> Result<SprinklerCurrentWeatherV1, String> {
    let value = SprinklerCurrentWeatherV1 {
        retrieved_at,
        valid_until: retrieved_at.saturating_add(u64::from(SPRINKLER_CURRENT_FRESHNESS_SECONDS)),
        valid_at: current.time,
        interval_seconds: current.interval,
        temperature_celsius: required_measurement(current.temperature_2m, "temperature_2m")?,
        relative_humidity_percent: required_percentage(
            current.relative_humidity_2m,
            "relative_humidity_2m",
        )?,
        precipitation_millimeters: required_nonnegative_measurement(
            current.precipitation,
            "precipitation",
        )?,
        reference_evapotranspiration_millimeters: required_nonnegative_measurement(
            current.et0_fao_evapotranspiration,
            "et0_fao_evapotranspiration",
        )?,
        wind_speed_meters_per_second: required_nonnegative_measurement(
            current.wind_speed_10m,
            "wind_speed_10m",
        )?,
        wind_gust_meters_per_second: required_nonnegative_measurement(
            current.wind_gusts_10m,
            "wind_gusts_10m",
        )?,
    };
    if valid_current(&value) {
        Ok(value)
    } else {
        Err(String::from("Open-Meteo current section failed validation"))
    }
}

fn fetch_current_weather(
    client: &Client,
    location: ProviderLocation,
) -> Result<SprinklerCurrentWeatherV1, String> {
    let query = OpenMeteoCurrentQuery {
        latitude: location.latitude_degrees,
        longitude: location.longitude_degrees,
        current: "temperature_2m,relative_humidity_2m,precipitation,et0_fao_evapotranspiration,wind_speed_10m,wind_gusts_10m",
        timeformat: "unixtime",
        wind_speed_unit: "ms",
        precipitation_unit: "mm",
    };
    let response: OpenMeteoCurrentResponse = read_json_response(
        client
            .get(OPEN_METEO_URL)
            .query(&query)
            .send()
            .map_err(|error| format!("Open-Meteo current request failed: {error}"))?,
    )?;
    let retrieved_at = unix_time_seconds()?;
    build_current(response.current, retrieved_at)
}

fn build_history(
    hourly: &OpenMeteoHourly,
    retrieved_at: u64,
    current_hour: u64,
) -> Result<SprinklerWeatherHistoryV2, String> {
    let expected_len = hourly.time.len();
    if hourly.temperature_2m.len() != expected_len
        || hourly.relative_humidity_2m.len() != expected_len
        || hourly.precipitation.len() != expected_len
        || hourly.et0_fao_evapotranspiration.len() != expected_len
        || hourly.wind_speed_10m.len() != expected_len
        || hourly.wind_gusts_10m.len() != expected_len
    {
        return Err(String::from(
            "Open-Meteo historical arrays have different lengths",
        ));
    }
    let earliest_end = current_hour.saturating_sub(u64::from(SPRINKLER_HISTORY_WINDOW_SECONDS));
    let mut periods = Vec::new();

    for (index, ends_at) in hourly.time.iter().copied().enumerate() {
        // Open-Meteo describes precipitation and ET0 as preceding-hour sums.
        // Treat its hourly timestamp as the exclusive end of that accumulation.
        if ends_at <= earliest_end || ends_at > current_hour {
            continue;
        }
        periods.push(SprinklerWeatherHistoryPeriodV2 {
            starts_at: ends_at.saturating_sub(3_600),
            duration_seconds: 3_600,
            temperature_celsius: required_measurement(
                hourly.temperature_2m[index],
                "historical temperature_2m",
            )?,
            relative_humidity_percent: required_percentage(
                hourly.relative_humidity_2m[index],
                "historical relative_humidity_2m",
            )?,
            precipitation_millimeters: required_nonnegative_measurement(
                hourly.precipitation[index],
                "historical precipitation",
            )?,
            reference_evapotranspiration_millimeters: required_nonnegative_measurement(
                hourly.et0_fao_evapotranspiration[index],
                "historical et0_fao_evapotranspiration",
            )?,
            wind_speed_meters_per_second: required_nonnegative_measurement(
                hourly.wind_speed_10m[index],
                "historical wind_speed_10m",
            )?,
            wind_gust_meters_per_second: required_nonnegative_measurement(
                hourly.wind_gusts_10m[index],
                "historical wind_gusts_10m",
            )?,
        });
    }
    if periods.is_empty() || periods.len() > MAX_RECOVERY_PERIODS {
        return Err(String::from(
            "Open-Meteo returned an invalid historical period count",
        ));
    }

    let history = SprinklerWeatherHistoryV2 {
        retrieved_at,
        valid_until: retrieved_at.saturating_add(u64::from(SPRINKLER_HISTORY_FRESHNESS_SECONDS)),
        periods,
    };
    if valid_history(&history) {
        Ok(history)
    } else {
        Err(String::from("Open-Meteo history failed validation"))
    }
}

fn build_forecast(
    hourly: &OpenMeteoHourly,
    retrieved_at: u64,
    forecast_starts_at: u64,
) -> Result<SprinklerWeatherForecastV1, String> {
    let expected_len = hourly.time.len();
    if hourly.temperature_2m.len() != expected_len
        || hourly.relative_humidity_2m.len() != expected_len
        || hourly.precipitation_probability.len() != expected_len
        || hourly.precipitation.len() != expected_len
        || hourly.et0_fao_evapotranspiration.len() != expected_len
        || hourly.wind_speed_10m.len() != expected_len
        || hourly.wind_gusts_10m.len() != expected_len
    {
        return Err(String::from(
            "Open-Meteo forecast arrays have different lengths",
        ));
    }
    let forecast_end =
        forecast_starts_at.saturating_add(u64::from(SPRINKLER_FORECAST_HORIZON_SECONDS));
    let mut periods = Vec::new();

    for (index, ends_at) in hourly.time.iter().copied().enumerate() {
        // Open-Meteo describes precipitation and ET0 as preceding-hour sums.
        // The period beginning one hour earlier keeps those sums aligned with
        // the temperature and wind values used for sprinkler planning.
        let starts_at = ends_at.saturating_sub(3_600);
        if starts_at < forecast_starts_at || starts_at >= forecast_end {
            continue;
        }
        let probability = required_percentage(
            hourly.precipitation_probability[index],
            "precipitation_probability",
        )?;
        periods.push(SprinklerWeatherForecastPeriodV1 {
            starts_at,
            duration_seconds: 3_600,
            temperature_celsius: required_measurement(
                hourly.temperature_2m[index],
                "forecast temperature_2m",
            )?,
            relative_humidity_percent: required_percentage(
                hourly.relative_humidity_2m[index],
                "forecast relative_humidity_2m",
            )?,
            precipitation_probability_percent: probability,
            expected_precipitation_millimeters: required_nonnegative_measurement(
                hourly.precipitation[index],
                "forecast precipitation",
            )?,
            reference_evapotranspiration_millimeters: required_nonnegative_measurement(
                hourly.et0_fao_evapotranspiration[index],
                "forecast et0_fao_evapotranspiration",
            )?,
            wind_speed_meters_per_second: required_nonnegative_measurement(
                hourly.wind_speed_10m[index],
                "forecast wind_speed_10m",
            )?,
            wind_gust_meters_per_second: required_nonnegative_measurement(
                hourly.wind_gusts_10m[index],
                "forecast wind_gusts_10m",
            )?,
        });
    }
    if periods.is_empty() || periods.len() > MAX_RECOVERY_PERIODS {
        return Err(String::from(
            "Open-Meteo returned an invalid forecast period count",
        ));
    }

    let forecast = SprinklerWeatherForecastV1 {
        retrieved_at,
        valid_until: retrieved_at.saturating_add(u64::from(SPRINKLER_FORECAST_FRESHNESS_SECONDS)),
        periods,
    };
    if valid_forecast(&forecast) {
        Ok(forecast)
    } else {
        Err(String::from("Open-Meteo forecast failed validation"))
    }
}

fn fetch_hourly_weather(
    client: &Client,
    location: ProviderLocation,
) -> (
    Result<SprinklerWeatherHistoryV2, String>,
    Result<SprinklerWeatherForecastV1, String>,
) {
    let query = OpenMeteoHourlyQuery {
        latitude: location.latitude_degrees,
        longitude: location.longitude_degrees,
        hourly: "temperature_2m,relative_humidity_2m,precipitation_probability,precipitation,et0_fao_evapotranspiration,wind_speed_10m,wind_gusts_10m",
        past_hours: MAX_RECOVERY_PERIODS as u16,
        // Open-Meteo includes the current hour in `forecast_hours`. Request two
        // extra timesteps so a mid-hour retrieval still yields 168 periods
        // whose start times are not earlier than `retrieved_at`.
        forecast_hours: MAX_RECOVERY_PERIODS as u16 + 2,
        timeformat: "unixtime",
        wind_speed_unit: "ms",
        precipitation_unit: "mm",
    };
    let response = client
        .get(OPEN_METEO_URL)
        .query(&query)
        .send()
        .map_err(|error| format!("Open-Meteo hourly request failed: {error}"))
        .and_then(read_json_response::<OpenMeteoHourlyResponse>);
    let response = match response {
        Ok(response) => response,
        Err(error) => return (Err(error.clone()), Err(error)),
    };
    let retrieved_at = match unix_time_seconds() {
        Ok(retrieved_at) => retrieved_at,
        Err(error) => return (Err(error.clone()), Err(error)),
    };
    let current_hour = retrieved_at / 3_600 * 3_600;

    (
        build_history(&response.hourly, retrieved_at, current_hour),
        build_forecast(&response.hourly, retrieved_at, retrieved_at),
    )
}

fn send_provider_message(results: &SyncSender<ProviderMessage>, message: ProviderMessage) -> bool {
    match results.try_send(message) {
        Ok(()) => {
            libertas_wake_up();
            true
        }
        Err(TrySendError::Full(_)) => true,
        Err(TrySendError::Disconnected(_)) => false,
    }
}

fn provider_worker(
    commands: Receiver<ProviderCommand>,
    results: SyncSender<ProviderMessage>,
    stop_requested: Arc<AtomicBool>,
) {
    let client = build_http_client();
    while let Ok(command) = commands.recv() {
        if stop_requested.load(Ordering::Acquire) || matches!(command, ProviderCommand::Shutdown) {
            return libertas_shutdown_complete();
        }
        let message = match (&client, command) {
            (Ok(client), ProviderCommand::RefreshCurrent { location }) => {
                ProviderMessage::Current {
                    location,
                    result: fetch_current_weather(client, location),
                }
            }
            (Ok(client), ProviderCommand::RefreshHourly { location }) => {
                let (history, forecast) = fetch_hourly_weather(client, location);
                ProviderMessage::Hourly {
                    location,
                    history,
                    forecast,
                }
            }
            (Err(error), ProviderCommand::RefreshCurrent { location }) => {
                ProviderMessage::Current {
                    location,
                    result: Err(error.clone()),
                }
            }
            (Err(error), ProviderCommand::RefreshHourly { location }) => ProviderMessage::Hourly {
                location,
                history: Err(error.clone()),
                forecast: Err(error.clone()),
            },
            (_, ProviderCommand::Shutdown) => unreachable!(),
        };
        if stop_requested.load(Ordering::Acquire) {
            return libertas_shutdown_complete();
        }
        if !send_provider_message(&results, message) {
            break;
        }
    }
}

fn start_provider_worker() -> Result<ProviderRuntime, String> {
    let (command_sender, command_receiver) = sync_channel(PROVIDER_COMMAND_CAPACITY);
    let (result_sender, result_receiver) = sync_channel(PROVIDER_RESULT_CAPACITY);
    let stop_requested = Arc::new(AtomicBool::new(false));
    let worker_stop_requested = Arc::clone(&stop_requested);
    thread::Builder::new()
        .name(String::from("libertas-weather-http"))
        .spawn(move || provider_worker(command_receiver, result_sender, worker_stop_requested))
        .map_err(|error| format!("failed to start weather HTTP worker: {error}"))?;
    Ok(ProviderRuntime {
        commands: command_sender,
        results: result_receiver,
        stop_requested,
    })
}

impl WeatherServerState {
    fn new(
        endpoint: LibertasEndpoint,
        epoch_timestamp: Option<LibertasDateTime>,
        snapshot: SprinklerWeatherSnapshotV2,
    ) -> Self {
        Self {
            endpoint,
            site_location: None,
            cursor: epoch_timestamp.map(|epoch_timestamp| SprinklerWeatherCursorV1 {
                epoch_timestamp,
                sequence: 0,
            }),
            snapshot,
            journal: Vec::new(),
            peer_alive_timer: 0,
            next_peer_alive_ticks: None,
        }
    }

    fn prepare_response(
        &mut self,
        request: SprinklerWeatherProtocolV1,
        now_ticks: u64,
    ) -> Option<PreparedResponse> {
        let SprinklerWeatherProtocolV1::GetWeatherV1 {
            after_cursor,
            history_range,
            include_current,
            forecast_range,
        } = request
        else {
            return None;
        };
        let recovery = self.recover(
            after_cursor,
            history_range,
            include_current,
            forecast_range,
            now_ticks,
        );
        let accepted = !matches!(recovery, SprinklerWeatherRecoveryV1::ErrorV1 { .. });

        Some(PreparedResponse {
            message: SprinklerWeatherProtocolV1::WeatherRecoveryV1 {
                maximum_wait_interval_seconds: SPRINKLER_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
                recovery,
            },
            accepted,
        })
    }

    fn recover(
        &mut self,
        after_cursor: Option<SprinklerWeatherCursorV1>,
        history_range: Option<SprinklerWeatherTimeRangeV1>,
        include_current: bool,
        forecast_range: Option<SprinklerWeatherTimeRangeV1>,
        now_ticks: u64,
    ) -> SprinklerWeatherRecoveryV1 {
        let snapshot = match self.select_snapshot(history_range, include_current, forecast_range) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return SprinklerWeatherRecoveryV1::ErrorV1 {
                    error,
                    retry_after_seconds: None,
                };
            }
        };

        let Some(current_cursor) = self.cursor else {
            return SprinklerWeatherRecoveryV1::ErrorV1 {
                error: SprinklerWeatherRecoveryErrorV1::TemporarilyUnavailable,
                retry_after_seconds: Some(RETRY_WITHOUT_UTC_SECONDS),
            };
        };

        self.prune_journal(now_ticks);

        let Some(after_cursor) = after_cursor else {
            return self.reset_recovery(
                SprinklerWeatherResetReasonV1::InitialSubscription,
                current_cursor,
                snapshot,
            );
        };

        if after_cursor.epoch_timestamp == current_cursor.epoch_timestamp {
            if after_cursor.sequence > current_cursor.sequence {
                return cursor_ahead();
            }
            if after_cursor == current_cursor {
                return SprinklerWeatherRecoveryV1::ReplayedV1 {
                    report: empty_report(current_cursor),
                };
            }
            if let Some(report) = self.replay_after(after_cursor, current_cursor) {
                return SprinklerWeatherRecoveryV1::ReplayedV1 { report };
            }
            return self.reset_recovery(
                SprinklerWeatherResetReasonV1::CursorExpired,
                current_cursor,
                snapshot,
            );
        }

        if current_cursor.is_server_reset_after(after_cursor) {
            self.reset_recovery(
                SprinklerWeatherResetReasonV1::ServerCursorReset,
                current_cursor,
                snapshot,
            )
        } else {
            cursor_ahead()
        }
    }

    fn reset_recovery(
        &self,
        reason: SprinklerWeatherResetReasonV1,
        cursor: SprinklerWeatherCursorV1,
        snapshot: SprinklerWeatherSnapshotV2,
    ) -> SprinklerWeatherRecoveryV1 {
        if let Some(location) = self.site_location {
            SprinklerWeatherRecoveryV1::ResetAtSiteV2 {
                reason,
                cursor,
                location,
                snapshot,
            }
        } else {
            // A V2 snapshot is meaningful only once it is bound to a provider
            // site. Do not fall back to a legacy reset that cannot carry V2
            // history; ask the client to retry after Hub location recovery.
            SprinklerWeatherRecoveryV1::ErrorV1 {
                error: SprinklerWeatherRecoveryErrorV1::TemporarilyUnavailable,
                retry_after_seconds: Some(RETRY_WITHOUT_UTC_SECONDS),
            }
        }
    }

    fn select_snapshot(
        &self,
        history_range: Option<SprinklerWeatherTimeRangeV1>,
        include_current: bool,
        forecast_range: Option<SprinklerWeatherTimeRangeV1>,
    ) -> Result<SprinklerWeatherSnapshotV2, SprinklerWeatherRecoveryErrorV1> {
        validate_requested_range(history_range, SPRINKLER_HISTORY_WINDOW_SECONDS)?;
        validate_requested_range(forecast_range, SPRINKLER_FORECAST_HORIZON_SECONDS)?;

        let history = match (history_range, self.snapshot.history.as_ref()) {
            (Some(range), Some(history)) => {
                let periods: Vec<_> = history
                    .periods
                    .iter()
                    .copied()
                    .filter(|period| {
                        period.starts_at >= range.starts_at && period.starts_at < range.ends_before
                    })
                    .collect();
                if periods.len() > MAX_RECOVERY_PERIODS {
                    return Err(SprinklerWeatherRecoveryErrorV1::RequestTooLarge);
                }
                Some(SprinklerWeatherHistoryV2 {
                    retrieved_at: history.retrieved_at,
                    valid_until: history.valid_until,
                    periods,
                })
            }
            _ => None,
        };

        let forecast = match (forecast_range, self.snapshot.forecast.as_ref()) {
            (Some(range), Some(forecast)) => {
                let periods: Vec<_> = forecast
                    .periods
                    .iter()
                    .copied()
                    .filter(|period| {
                        period.starts_at >= range.starts_at && period.starts_at < range.ends_before
                    })
                    .collect();
                if periods.len() > MAX_RECOVERY_PERIODS {
                    return Err(SprinklerWeatherRecoveryErrorV1::RequestTooLarge);
                }
                Some(SprinklerWeatherForecastV1 {
                    retrieved_at: forecast.retrieved_at,
                    valid_until: forecast.valid_until,
                    periods,
                })
            }
            _ => None,
        };

        Ok(SprinklerWeatherSnapshotV2 {
            history,
            current: if include_current {
                self.snapshot.current
            } else {
                None
            },
            forecast,
        })
    }

    fn prune_journal(&mut self, now_ticks: u64) {
        let replay_window = u64::from(SPRINKLER_SUBSCRIPTION_REPLAY_WINDOW_SECONDS)
            .saturating_mul(MICROSECONDS_PER_SECOND);
        self.journal
            .retain(|entry| now_ticks.saturating_sub(entry.recorded_at_ticks) <= replay_window);
    }

    fn replay_after(
        &self,
        after_cursor: SprinklerWeatherCursorV1,
        current_cursor: SprinklerWeatherCursorV1,
    ) -> Option<SprinklerWeatherIncrementalReportV1> {
        let mut expected = after_cursor;
        let mut changes = Vec::new();
        let mut found_start = false;

        for entry in &self.journal {
            if !found_start {
                if entry.report.from_cursor != expected {
                    continue;
                }
                found_start = true;
            }
            if entry.report.from_cursor != expected || !entry.report.has_contiguous_cursor_range() {
                return None;
            }
            if changes.len().saturating_add(entry.report.changes.len()) > MAX_REPLAY_CHANGES {
                return None;
            }
            changes.extend(entry.report.changes.iter().cloned());
            expected = entry.report.through_cursor;
            if expected == current_cursor {
                return Some(SprinklerWeatherIncrementalReportV1 {
                    from_cursor: after_cursor,
                    through_cursor: current_cursor,
                    changes,
                });
            }
        }

        None
    }

    fn apply_change(
        &mut self,
        change: SprinklerWeatherChangeV1,
        now_ticks: u64,
        now_utc: LibertasDateTime,
    ) -> ChangePublication {
        let mut from_cursor = self.cursor.unwrap_or(SprinklerWeatherCursorV1 {
            epoch_timestamp: now_utc,
            sequence: 0,
        });
        if from_cursor.sequence == u64::MAX {
            self.journal.clear();
            from_cursor = SprinklerWeatherCursorV1 {
                epoch_timestamp: now_utc.max(from_cursor.epoch_timestamp.saturating_add(1)),
                sequence: 0,
            };
        }
        let through_cursor = SprinklerWeatherCursorV1 {
            epoch_timestamp: from_cursor.epoch_timestamp,
            sequence: from_cursor.sequence + 1,
        };

        match &change {
            SprinklerWeatherChangeV1::HistoryReplaceV2 { history } => {
                self.snapshot.history = Some(history.clone());
            }
            SprinklerWeatherChangeV1::CurrentReplaceV1 { current } => {
                self.snapshot.current = Some(*current);
            }
            SprinklerWeatherChangeV1::ForecastReplaceV1 { forecast } => {
                self.snapshot.forecast = Some(forecast.clone());
            }
            SprinklerWeatherChangeV1::SiteReplaceV1 { location } => {
                self.site_location = Some(*location);
                self.snapshot = SprinklerWeatherSnapshotV2 {
                    history: None,
                    current: None,
                    forecast: None,
                };
            }
            SprinklerWeatherChangeV1::SectionClearV1 { section } => match section {
                SprinklerWeatherSectionV1::History => self.snapshot.history = None,
                SprinklerWeatherSectionV1::Current => self.snapshot.current = None,
                SprinklerWeatherSectionV1::Forecast => self.snapshot.forecast = None,
            },
            // Legacy history changes cannot populate the V2 cache because
            // their positional payload has no temperature, humidity, or wind.
            // The V2 server never emits them and does not advance its cursor
            // for an attempted internal legacy publication.
            SprinklerWeatherChangeV1::HistoryPeriodsUpsertV1 { .. }
            | SprinklerWeatherChangeV1::HistoryPeriodsRemoveV1 { .. }
            | SprinklerWeatherChangeV1::HistoryReplaceV1 { .. }
            | SprinklerWeatherChangeV1::HistoryPeriodsUpsertV2 { .. }
            | SprinklerWeatherChangeV1::ForecastPeriodsUpsertV1 { .. }
            | SprinklerWeatherChangeV1::ForecastPeriodsRemoveV1 { .. } => {
                return ChangePublication { report: None };
            }
        }

        let report = SprinklerWeatherIncrementalReportV1 {
            from_cursor,
            through_cursor,
            changes: vec![change],
        };
        self.cursor = Some(through_cursor);
        self.journal.push(JournalEntry {
            recorded_at_ticks: now_ticks,
            report: report.clone(),
        });
        self.prune_journal(now_ticks);

        ChangePublication {
            report: Some(SprinklerWeatherProtocolV1::WeatherIncrementV1 { report }),
        }
    }

    fn peer_alive_interval_ticks() -> u64 {
        (u64::from(SPRINKLER_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS).saturating_mul(3) / 4)
            .saturating_mul(MICROSECONDS_PER_SECOND)
    }

    // The host owns fan-out; the App needs one cadence, not a peer roster.
    fn note_subscription(&mut self, now_ticks: u64) {
        let deadline = now_ticks.saturating_add(Self::peer_alive_interval_ticks());
        self.next_peer_alive_ticks = Some(
            self.next_peer_alive_ticks
                .map_or(deadline, |current| current.min(deadline)),
        );
    }

    fn note_data_report(&mut self, now_ticks: u64) {
        self.next_peer_alive_ticks =
            Some(now_ticks.saturating_add(Self::peer_alive_interval_ticks()));
    }

    fn take_due_peer_alive(&mut self, now_ticks: u64) -> bool {
        if self
            .next_peer_alive_ticks
            .is_none_or(|deadline| now_ticks < deadline)
        {
            return false;
        }
        self.note_data_report(now_ticks);
        true
    }
}

fn validate_requested_range(
    range: Option<SprinklerWeatherTimeRangeV1>,
    maximum_seconds: u32,
) -> Result<(), SprinklerWeatherRecoveryErrorV1> {
    let Some(range) = range else {
        return Ok(());
    };
    if !range.is_valid() {
        return Err(SprinklerWeatherRecoveryErrorV1::InvalidRange);
    }
    if range.ends_before.saturating_sub(range.starts_at) > u64::from(maximum_seconds) {
        return Err(SprinklerWeatherRecoveryErrorV1::RequestTooLarge);
    }
    Ok(())
}

fn cursor_ahead() -> SprinklerWeatherRecoveryV1 {
    SprinklerWeatherRecoveryV1::ErrorV1 {
        error: SprinklerWeatherRecoveryErrorV1::CursorAhead,
        retry_after_seconds: None,
    }
}

fn empty_report(cursor: SprinklerWeatherCursorV1) -> SprinklerWeatherIncrementalReportV1 {
    SprinklerWeatherIncrementalReportV1 {
        from_cursor: cursor,
        through_cursor: cursor,
        changes: Vec::new(),
    }
}

fn persistent_key(endpoint: LibertasEndpoint) -> [NotificationArgument<'static>; 1] {
    [NotificationArgument::Object(endpoint)]
}

fn load_location(endpoint: LibertasEndpoint) -> Option<SprinklerWeatherLocationV1> {
    let key = persistent_key(endpoint);
    match libertas_data_read_single(LOCATION_RESOURCE, &key) {
        Some(SprinklerWeatherPersistentDataV1::LocationV1 { location })
            if valid_weather_location(location) =>
        {
            Some(location)
        }
        Some(_) => {
            libertas_log(
                LogLevel::Warn,
                "Discarding an invalid persisted sprinkler weather location",
            );
            libertas_data_remove_single(LOCATION_RESOURCE, &key);
            None
        }
        None => None,
    }
}

fn history_metadata(history: &SprinklerWeatherHistoryV2) -> SprinklerWeatherHistoryMetadataV1 {
    SprinklerWeatherHistoryMetadataV1 {
        retrieved_at: history.retrieved_at,
        valid_until: history.valid_until,
    }
}

fn valid_history_metadata(metadata: SprinklerWeatherHistoryMetadataV1) -> bool {
    metadata.retrieved_at > 0 && metadata.valid_until > metadata.retrieved_at
}

fn history_period_index(period: &SprinklerWeatherHistoryPeriodV2) -> Option<i64> {
    i64::try_from(period.starts_at).ok()
}

fn valid_history_period(period: &SprinklerWeatherHistoryPeriodV2) -> bool {
    period.duration_seconds > 0
        && u64::from(period.duration_seconds) <= u64::from(SPRINKLER_HISTORY_WINDOW_SECONDS)
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
}

fn indexed_history_record_is_current(
    record: &IndexedData<SprinklerWeatherPersistentDataV1>,
    metadata: SprinklerWeatherHistoryMetadataV1,
) -> bool {
    let SprinklerWeatherPersistentDataV1::HistoryPeriodV2 { period } = &record.data else {
        return false;
    };
    let Some(ends_at) = period
        .starts_at
        .checked_add(u64::from(period.duration_seconds))
    else {
        return false;
    };
    history_period_index(period) == Some(record.index)
        && valid_history_period(period)
        && ends_at <= metadata.retrieved_at
        && ends_at
            > metadata
                .retrieved_at
                .saturating_sub(u64::from(SPRINKLER_HISTORY_WINDOW_SECONDS))
}

#[derive(Debug, PartialEq)]
struct IndexedHistoryReconstruction {
    history: Option<SprinklerWeatherHistoryV2>,
    records_to_remove: Vec<i64>,
    legacy_v1_records_found: bool,
}

fn reconstruct_indexed_history(
    metadata: SprinklerWeatherHistoryMetadataV1,
    records: &[IndexedData<SprinklerWeatherPersistentDataV1>],
) -> IndexedHistoryReconstruction {
    let mut accepted = Vec::new();
    let mut records_to_remove = Vec::new();
    let mut legacy_v1_records_found = false;
    for record in records {
        if matches!(
            record.data,
            SprinklerWeatherPersistentDataV1::HistoryPeriodV1 { .. }
        ) {
            // V1 has no temperature, humidity, wind, or gust fields. Never
            // reinterpret it as a V2 sample or manufacture zero-valued
            // observations during migration.
            legacy_v1_records_found = true;
            records_to_remove.push(record.index);
            continue;
        }
        if !indexed_history_record_is_current(record, metadata) {
            records_to_remove.push(record.index);
            continue;
        }
        let SprinklerWeatherPersistentDataV1::HistoryPeriodV2 { period } = &record.data else {
            unreachable!();
        };
        accepted.push((record.index, *period));
    }
    accepted.sort_by_key(|(index, _)| *index);
    if accepted.len() > MAX_RECOVERY_PERIODS {
        let excess = accepted.len() - MAX_RECOVERY_PERIODS;
        records_to_remove.extend(accepted.drain(..excess).map(|(index, _)| index));
    }
    records_to_remove.sort_unstable();
    records_to_remove.dedup();

    let history = SprinklerWeatherHistoryV2 {
        retrieved_at: metadata.retrieved_at,
        valid_until: metadata.valid_until,
        periods: accepted.into_iter().map(|(_, period)| period).collect(),
    };
    IndexedHistoryReconstruction {
        history: valid_history(&history).then_some(history),
        records_to_remove,
        legacy_v1_records_found,
    }
}

fn clear_indexed_history(endpoint: LibertasEndpoint) {
    let key = persistent_key(endpoint);
    libertas_data_remove_single(HISTORY_METADATA_RESOURCE, &key);
    let database = libertas_data_open_indexed(HISTORY_PERIODS_RESOURCE, &key);
    if database.count > 0 {
        libertas_data_remove_indexed_records(
            database.handle,
            database.min_index,
            database.max_index,
        );
    }
}

fn load_indexed_history(endpoint: LibertasEndpoint) -> Option<SprinklerWeatherHistoryV2> {
    let key = persistent_key(endpoint);
    let metadata = match libertas_data_read_single(HISTORY_METADATA_RESOURCE, &key) {
        Some(SprinklerWeatherPersistentDataV1::HistoryMetadataV1 { metadata })
            if valid_history_metadata(metadata) =>
        {
            metadata
        }
        _ => {
            clear_indexed_history(endpoint);
            return None;
        }
    };
    let database = libertas_data_open_indexed(HISTORY_PERIODS_RESOURCE, &key);
    if database.count == 0
        || database.count > u64::try_from(MAX_HISTORY_RECORDS_SCANNED).unwrap_or(u64::MAX)
    {
        clear_indexed_history(endpoint);
        return None;
    }
    let mut records = Vec::new();
    libertas_data_read_indexed_range::<SprinklerWeatherPersistentDataV1>(
        database.handle,
        database.max_index,
        IndexDirection::Below,
        MAX_HISTORY_RECORDS_SCANNED,
        &mut records,
    );
    let reconstruction = reconstruct_indexed_history(metadata, &records);
    if reconstruction.legacy_v1_records_found {
        // A mixed V1/V2 cache can be produced by a stop during migration. It
        // is not a complete V2 history, so clear the bounded cache and its
        // metadata. The missing section schedules an immediate provider fetch.
        clear_indexed_history(endpoint);
        return None;
    }
    for index in reconstruction.records_to_remove {
        libertas_data_remove_indexed_records(database.handle, index, index);
    }
    if reconstruction.history.is_none() {
        clear_indexed_history(endpoint);
    }
    reconstruction.history
}

fn load_snapshot(endpoint: LibertasEndpoint) -> SprinklerWeatherSnapshotV2 {
    let key = persistent_key(endpoint);
    let history = load_indexed_history(endpoint);
    let current = match libertas_data_read_single(CURRENT_RESOURCE, &key) {
        Some(SprinklerWeatherPersistentDataV1::CurrentV1 { current })
            if valid_current(&current) =>
        {
            Some(current)
        }
        _ => None,
    };
    let forecast = match libertas_data_read_single(FORECAST_RESOURCE, &key) {
        Some(SprinklerWeatherPersistentDataV1::ForecastV1 { forecast })
            if valid_forecast(&forecast) =>
        {
            Some(forecast)
        }
        _ => None,
    };

    SprinklerWeatherSnapshotV2 {
        history,
        current,
        forecast,
    }
}

fn publish_change(
    shared: &Rc<RefCell<WeatherServerState>>,
    change: SprinklerWeatherChangeV1,
    now_utc: LibertasDateTime,
) {
    let now_ticks = libertas_get_sys_ticks();
    let (endpoint, publication) = {
        let mut state = shared.borrow_mut();
        let publication = state.apply_change(change, now_ticks, now_utc);
        if publication.report.is_some() {
            state.note_data_report(now_ticks);
        }
        (state.endpoint, publication)
    };
    if let Some(report) = publication.report {
        libertas_endpoint_report(endpoint, &report, None);
    }
    update_peer_alive_timer(shared);
}

#[derive(Debug, Default, PartialEq)]
struct IndexedHistoryDelta {
    upserts: Vec<SprinklerWeatherHistoryPeriodV2>,
    removals: Vec<i64>,
}

fn indexed_history_delta(
    previous: Option<&SprinklerWeatherHistoryV2>,
    current: &SprinklerWeatherHistoryV2,
) -> IndexedHistoryDelta {
    let previous_periods = previous.map_or(&[][..], |history| history.periods.as_slice());
    let mut delta = IndexedHistoryDelta::default();
    for period in &current.periods {
        if previous_periods
            .iter()
            .find(|previous| previous.starts_at == period.starts_at)
            != Some(period)
        {
            delta.upserts.push(*period);
        }
    }
    for period in previous_periods {
        if !current
            .periods
            .iter()
            .any(|current| current.starts_at == period.starts_at)
            && let Some(index) = history_period_index(period)
        {
            delta.removals.push(index);
        }
    }
    delta
}

fn persist_indexed_history(
    endpoint: LibertasEndpoint,
    previous: Option<&SprinklerWeatherHistoryV2>,
    history: &SprinklerWeatherHistoryV2,
) {
    let key = persistent_key(endpoint);
    let database = libertas_data_open_indexed(HISTORY_PERIODS_RESOURCE, &key);
    let delta = indexed_history_delta(previous, history);
    for period in delta.upserts {
        let Some(index) = history_period_index(&period) else {
            continue;
        };
        libertas_data_write_indexed(
            database.handle,
            index,
            &SprinklerWeatherPersistentDataV1::HistoryPeriodV2 { period },
        );
    }
    for index in delta.removals {
        libertas_data_remove_indexed_records(database.handle, index, index);
    }
    libertas_data_write_single(
        HISTORY_METADATA_RESOURCE,
        &key,
        &SprinklerWeatherPersistentDataV1::HistoryMetadataV1 {
            metadata: history_metadata(history),
        },
    );
}

fn publish_persisted_history_change(
    shared: &Rc<RefCell<WeatherServerState>>,
    history: SprinklerWeatherHistoryV2,
) {
    let (endpoint, previous) = {
        let state = shared.borrow();
        (state.endpoint, state.snapshot.history.clone())
    };
    persist_indexed_history(endpoint, previous.as_ref(), &history);
    let now_utc = history.retrieved_at;
    publish_change(
        shared,
        SprinklerWeatherChangeV1::HistoryReplaceV2 { history },
        now_utc,
    );
}

fn publish_persisted_change(
    shared: &Rc<RefCell<WeatherServerState>>,
    resource: &'static str,
    persistent: SprinklerWeatherPersistentDataV1,
    change: SprinklerWeatherChangeV1,
    now_utc: LibertasDateTime,
) {
    let endpoint = shared.borrow().endpoint;
    let key = persistent_key(endpoint);
    libertas_data_write_single(resource, &key, &persistent);

    publish_change(shared, change, now_utc);
}

fn log_provider_error(section: &str, error: &str) {
    libertas_log(
        LogLevel::Warn,
        &format!("Open-Meteo {section} refresh failed; retaining cached data: {error}"),
    );
}

fn handle_provider_message(shared: &Rc<RefCell<WeatherServerState>>, message: ProviderMessage) {
    match message {
        ProviderMessage::Current {
            result: Ok(current),
            ..
        } => {
            let now_utc = current.retrieved_at;
            publish_persisted_change(
                shared,
                CURRENT_RESOURCE,
                SprinklerWeatherPersistentDataV1::CurrentV1 { current },
                SprinklerWeatherChangeV1::CurrentReplaceV1 { current },
                now_utc,
            );
        }
        ProviderMessage::Current {
            result: Err(error), ..
        } => log_provider_error("current", &error),
        ProviderMessage::Hourly {
            history, forecast, ..
        } => {
            match history {
                Ok(history) => {
                    publish_persisted_history_change(shared, history);
                }
                Err(error) => log_provider_error("history", &error),
            }
            match forecast {
                Ok(forecast) => {
                    let now_utc = forecast.retrieved_at;
                    publish_persisted_change(
                        shared,
                        FORECAST_RESOURCE,
                        SprinklerWeatherPersistentDataV1::ForecastV1 {
                            forecast: forecast.clone(),
                        },
                        SprinklerWeatherChangeV1::ForecastReplaceV1 { forecast },
                        now_utc,
                    );
                }
                Err(error) => log_provider_error("forecast", &error),
            }
        }
    }
}

fn provider_message_location(message: &ProviderMessage) -> ProviderLocation {
    match message {
        ProviderMessage::Current { location, .. } | ProviderMessage::Hourly { location, .. } => {
            *location
        }
    }
}

fn handle_provider_wakeup(context: &mut Box<dyn Any>) {
    let context = context.downcast_mut::<ProviderWakeupContext>().unwrap();
    while let Ok(message) = context.results.try_recv() {
        if context.location.get() != Some(provider_message_location(&message)) {
            continue;
        }
        handle_provider_message(&context.shared, message);
    }
}

fn handle_provider_shutdown(context: &mut Box<dyn Any>) {
    let context = context.downcast_mut::<ProviderShutdownContext>().unwrap();
    context.stop_requested.store(true, Ordering::Release);
    if matches!(
        context.commands.try_send(ProviderCommand::Shutdown),
        Err(TrySendError::Disconnected(_))
    ) {
        libertas_shutdown_complete();
    }
}

fn refresh_timer_fired(timer: u32, now_ticks: u64, context: &mut Box<dyn Any>) {
    let context = context.downcast_mut::<RefreshTimerContext>().unwrap();
    if let Some(location) = context.location.get() {
        let command = match context.refresh_kind {
            ProviderRefreshKind::Current => ProviderCommand::RefreshCurrent { location },
            ProviderRefreshKind::Hourly => ProviderCommand::RefreshHourly { location },
        };
        let _ = context.commands.try_send(command);
    }
    let next_ticks = now_ticks.saturating_add(
        u64::from(context.interval_seconds).saturating_mul(MICROSECONDS_PER_SECOND),
    );
    libertas_timer_update_interval(timer, next_ticks);
}

fn new_refresh_timer(
    commands: SyncSender<ProviderCommand>,
    location: Rc<Cell<Option<ProviderLocation>>>,
    refresh_kind: ProviderRefreshKind,
    interval_seconds: u32,
) -> u32 {
    libertas_timer_new_interval(
        0,
        refresh_timer_fired,
        Box::new(RefreshTimerContext {
            commands,
            location,
            refresh_kind,
            interval_seconds,
        }),
    )
}

fn rearm_refresh_timer(timer: u32, delay_seconds: u32, now_ticks: u64) {
    libertas_timer_update_interval(
        timer,
        now_ticks.saturating_add(u64::from(delay_seconds).saturating_mul(MICROSECONDS_PER_SECOND)),
    );
}

impl ProviderControl {
    fn schedule_from_cache(
        &self,
        location: ProviderLocation,
        snapshot: &SprinklerWeatherSnapshotV2,
        now_utc: Option<LibertasDateTime>,
        now_ticks: u64,
    ) {
        self.location.set(Some(location));
        let (current_delay, hourly_delay) = startup_refresh_delays(snapshot, now_utc);
        self.schedule_one(
            ProviderRefreshKind::Current,
            self.current_timer,
            SPRINKLER_CURRENT_REFRESH_INTERVAL_SECONDS,
            current_delay,
            now_ticks,
        );
        self.schedule_one(
            ProviderRefreshKind::Hourly,
            self.hourly_timer,
            SPRINKLER_HISTORY_REFRESH_INTERVAL_SECONDS
                .min(SPRINKLER_FORECAST_REFRESH_INTERVAL_SECONDS),
            hourly_delay,
            now_ticks,
        );
    }

    fn refresh_for_location(&self, location: ProviderLocation, now_ticks: u64) {
        self.location.set(Some(location));
        let _ = self
            .commands
            .try_send(ProviderCommand::RefreshCurrent { location });
        let _ = self
            .commands
            .try_send(ProviderCommand::RefreshHourly { location });
        rearm_refresh_timer(
            self.current_timer,
            SPRINKLER_CURRENT_REFRESH_INTERVAL_SECONDS,
            now_ticks,
        );
        rearm_refresh_timer(
            self.hourly_timer,
            SPRINKLER_HISTORY_REFRESH_INTERVAL_SECONDS
                .min(SPRINKLER_FORECAST_REFRESH_INTERVAL_SECONDS),
            now_ticks,
        );
    }

    fn schedule_one(
        &self,
        refresh_kind: ProviderRefreshKind,
        timer: u32,
        interval_seconds: u32,
        initial_delay_seconds: u32,
        now_ticks: u64,
    ) {
        if initial_delay_seconds == 0 {
            let location = self.location.get().unwrap();
            let command = match refresh_kind {
                ProviderRefreshKind::Current => ProviderCommand::RefreshCurrent { location },
                ProviderRefreshKind::Hourly => ProviderCommand::RefreshHourly { location },
            };
            let _ = self.commands.try_send(command);
        }
        let timer_delay_seconds = if initial_delay_seconds == 0 {
            interval_seconds
        } else {
            initial_delay_seconds
        };
        rearm_refresh_timer(timer, timer_delay_seconds, now_ticks);
    }
}

fn start_provider_control(
    shared: Rc<RefCell<WeatherServerState>>,
) -> Result<ProviderControl, String> {
    let provider = start_provider_worker()?;
    let commands = provider.commands;
    let location = Rc::new(Cell::new(None));
    libertas_register_wakeup_callback(
        handle_provider_wakeup,
        Box::new(ProviderWakeupContext {
            shared,
            location: Rc::clone(&location),
            results: provider.results,
        }),
    );
    libertas_register_shutdown_handler(
        handle_provider_shutdown,
        Box::new(ProviderShutdownContext {
            commands: commands.clone(),
            stop_requested: provider.stop_requested,
        }),
    );
    let current_timer = new_refresh_timer(
        commands.clone(),
        Rc::clone(&location),
        ProviderRefreshKind::Current,
        SPRINKLER_CURRENT_REFRESH_INTERVAL_SECONDS,
    );
    let hourly_timer = new_refresh_timer(
        commands.clone(),
        Rc::clone(&location),
        ProviderRefreshKind::Hourly,
        SPRINKLER_HISTORY_REFRESH_INTERVAL_SECONDS.min(SPRINKLER_FORECAST_REFRESH_INTERVAL_SECONDS),
    );
    Ok(ProviderControl {
        commands,
        location,
        current_timer,
        hourly_timer,
    })
}

fn remaining_refresh_delay_seconds(
    retrieved_at: Option<LibertasDateTime>,
    interval_seconds: u32,
    now_utc: Option<LibertasDateTime>,
) -> u32 {
    let (Some(retrieved_at), Some(now_utc)) = (retrieved_at, now_utc) else {
        return 0;
    };
    if retrieved_at > now_utc {
        return 0;
    }

    let due_at = retrieved_at.saturating_add(u64::from(interval_seconds));
    let remaining = due_at.saturating_sub(now_utc);
    u32::try_from(remaining.min(u64::from(interval_seconds))).unwrap_or(interval_seconds)
}

fn startup_refresh_delays(
    snapshot: &SprinklerWeatherSnapshotV2,
    now_utc: Option<LibertasDateTime>,
) -> (u32, u32) {
    let current_delay = remaining_refresh_delay_seconds(
        snapshot.current.map(|current| current.retrieved_at),
        SPRINKLER_CURRENT_REFRESH_INTERVAL_SECONDS,
        now_utc,
    );
    let history_delay = remaining_refresh_delay_seconds(
        snapshot
            .history
            .as_ref()
            .map(|history| history.retrieved_at),
        SPRINKLER_HISTORY_REFRESH_INTERVAL_SECONDS,
        now_utc,
    );
    let forecast_delay = remaining_refresh_delay_seconds(
        snapshot
            .forecast
            .as_ref()
            .map(|forecast| forecast.retrieved_at),
        SPRINKLER_FORECAST_REFRESH_INTERVAL_SECONDS,
        now_utc,
    );

    (current_delay, history_delay.min(forecast_delay))
}

fn valid_nonnegative(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}

impl From<SprinklerWeatherLocationV1> for ProviderLocation {
    fn from(location: SprinklerWeatherLocationV1) -> Self {
        Self {
            latitude_degrees: location.latitude_degrees,
            longitude_degrees: location.longitude_degrees,
        }
    }
}

fn valid_location(location: ProviderLocation) -> bool {
    location.latitude_degrees.is_finite()
        && (-90.0..=90.0).contains(&location.latitude_degrees)
        && location.longitude_degrees.is_finite()
        && (-180.0..=180.0).contains(&location.longitude_degrees)
}

fn valid_weather_location(location: SprinklerWeatherLocationV1) -> bool {
    valid_location(location.into())
}

fn same_weather_location(
    left: SprinklerWeatherLocationV1,
    right: SprinklerWeatherLocationV1,
) -> bool {
    (left.latitude_degrees - right.latitude_degrees).abs() <= LOCATION_EQUALITY_TOLERANCE_DEGREES
        && (left.longitude_degrees - right.longitude_degrees).abs()
            <= LOCATION_EQUALITY_TOLERANCE_DEGREES
}

fn valid_history(history: &SprinklerWeatherHistoryV2) -> bool {
    history.valid_until > history.retrieved_at
        && !history.periods.is_empty()
        && history.periods.len() <= MAX_RECOVERY_PERIODS
        && history.periods.iter().all(|period| {
            let Some(ends_at) = period
                .starts_at
                .checked_add(u64::from(period.duration_seconds))
            else {
                return false;
            };
            valid_history_period(period)
                && ends_at <= history.retrieved_at
                && ends_at
                    > history
                        .retrieved_at
                        .saturating_sub(u64::from(SPRINKLER_HISTORY_WINDOW_SECONDS))
        })
        && history.periods.windows(2).all(|pair| {
            pair[0]
                .starts_at
                .checked_add(u64::from(pair[0].duration_seconds))
                .is_some_and(|ends_at| ends_at <= pair[1].starts_at)
        })
}

fn valid_current(current: &SprinklerCurrentWeatherV1) -> bool {
    current.valid_until > current.retrieved_at
        && current.interval_seconds > 0
        && current.temperature_celsius.is_finite()
        && current.relative_humidity_percent <= 100
        && valid_nonnegative(current.precipitation_millimeters)
        && valid_nonnegative(current.reference_evapotranspiration_millimeters)
        && valid_nonnegative(current.wind_speed_meters_per_second)
        && valid_nonnegative(current.wind_gust_meters_per_second)
}

fn valid_forecast(forecast: &SprinklerWeatherForecastV1) -> bool {
    forecast.valid_until > forecast.retrieved_at
        && forecast.periods.iter().all(|period| {
            period.duration_seconds > 0
                && period.temperature_celsius.is_finite()
                && period.relative_humidity_percent <= 100
                && period.precipitation_probability_percent <= 100
                && valid_nonnegative(period.expected_precipitation_millimeters)
                && valid_nonnegative(period.reference_evapotranspiration_millimeters)
                && valid_nonnegative(period.wind_speed_meters_per_second)
                && valid_nonnegative(period.wind_gust_meters_per_second)
        })
        && forecast
            .periods
            .windows(2)
            .all(|pair| pair[0].starts_at < pair[1].starts_at)
}

fn weather_change_timestamp(shared: &Rc<RefCell<WeatherServerState>>) -> LibertasDateTime {
    libertas_get_utc_time()
        .map(|microseconds| microseconds / MICROSECONDS_PER_SECOND)
        .or_else(|| shared.borrow().cursor.map(|cursor| cursor.epoch_timestamp))
        .unwrap_or_default()
}

fn clear_weather_for_location_change(
    shared: &Rc<RefCell<WeatherServerState>>,
) -> Vec<SprinklerWeatherSectionV1> {
    let (endpoint, sections) = {
        let state = shared.borrow();
        let mut sections = Vec::new();
        if state.snapshot.history.is_some() {
            sections.push(SprinklerWeatherSectionV1::History);
        }
        if state.snapshot.current.is_some() {
            sections.push(SprinklerWeatherSectionV1::Current);
        }
        if state.snapshot.forecast.is_some() {
            sections.push(SprinklerWeatherSectionV1::Forecast);
        }
        (state.endpoint, sections)
    };
    let key = persistent_key(endpoint);
    clear_indexed_history(endpoint);
    libertas_data_remove_single(CURRENT_RESOURCE, &key);
    libertas_data_remove_single(FORECAST_RESOURCE, &key);
    shared.borrow_mut().snapshot = SprinklerWeatherSnapshotV2 {
        history: None,
        current: None,
        forecast: None,
    };
    sections
}

fn publish_weather_section_clears(
    shared: &Rc<RefCell<WeatherServerState>>,
    sections: &[SprinklerWeatherSectionV1],
) {
    let now_utc = weather_change_timestamp(shared);
    for section in sections {
        publish_change(
            shared,
            SprinklerWeatherChangeV1::SectionClearV1 { section: *section },
            now_utc,
        );
    }
}

fn accept_hub_location(
    state: &Rc<RefCell<LocationSubscriptionState>>,
    location: SprinklerWeatherLocationV1,
) -> bool {
    if !valid_weather_location(location) {
        libertas_log(
            LogLevel::Warn,
            "Libertas Hub reported an invalid sprinkler weather location",
        );
        return false;
    }

    let (previous, endpoint, weather, provider) = {
        let state = state.borrow();
        (
            state.location,
            state.weather.borrow().endpoint,
            Rc::clone(&state.weather),
            state.provider.clone(),
        )
    };
    if previous.is_some_and(|previous| same_weather_location(previous, location)) {
        return true;
    }

    // Remove every old-site section before durably associating this endpoint
    // with the new location. A stop at any point can therefore leave an empty
    // cache, but can never relabel old weather as belonging to the new site.
    let cleared_sections = if previous.is_some() {
        clear_weather_for_location_change(&weather)
    } else {
        Vec::new()
    };
    let key = persistent_key(endpoint);
    libertas_data_write_single(
        LOCATION_RESOURCE,
        &key,
        &SprinklerWeatherPersistentDataV1::LocationV1 { location },
    );
    state.borrow_mut().location = Some(location);

    if let Some(provider) = &provider {
        provider.location.set(Some(location.into()));
    }
    publish_change(
        &weather,
        SprinklerWeatherChangeV1::SiteReplaceV1 { location },
        weather_change_timestamp(&weather),
    );
    publish_weather_section_clears(&weather, &cleared_sections);
    if let Some(provider) = provider {
        provider.refresh_for_location(location.into(), libertas_get_sys_ticks());
    }
    true
}

fn arm_location_watchdog(state: &Rc<RefCell<LocationSubscriptionState>>, delay_seconds: u32) {
    let (timer, server_up) = {
        let state = state.borrow();
        (state.retry_timer, state.hub_server_up)
    };
    if timer != 0 {
        if !server_up {
            libertas_timer_cancel(timer);
            return;
        }
        rearm_refresh_timer(timer, delay_seconds, libertas_get_sys_ticks());
    }
}

fn subscribe_to_hub_location(state: &Rc<RefCell<LocationSubscriptionState>>) {
    if !state.borrow().hub_server_up {
        return;
    }
    libertas_endpoint_subscribe_request(
        LIBERTAS_HUB_ENDPOINT,
        &HubProtocol::LocationReq {
            max_report_interval_seconds: HUB_LOCATION_MAX_REPORT_INTERVAL_SECONDS,
        },
    );
    arm_location_watchdog(state, HUB_LOCATION_RETRY_SECONDS);
}

fn location_watchdog_fired(timer: u32, now_ticks: u64, context: &mut Box<dyn core::any::Any>) {
    let state = context
        .downcast_mut::<Rc<RefCell<LocationSubscriptionState>>>()
        .unwrap();
    if !state.borrow().hub_server_up {
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
        now_ticks.saturating_add(
            u64::from(HUB_LOCATION_RETRY_SECONDS).saturating_mul(MICROSECONDS_PER_SECOND),
        ),
    );
}

fn handle_hub_location_event(
    _endpoint: LibertasEndpoint,
    opcode: u8,
    message: LibertasEndpointMessage<HubProtocol>,
    context: &mut Box<dyn core::any::Any>,
    _transaction_id: u32,
    _peer: u32,
) -> LibertasEndpointHandlerResult {
    let state = context
        .downcast_mut::<Rc<RefCell<LocationSubscriptionState>>>()
        .unwrap();

    if opcode == OP_ENDPOINT_PEER_ALIVE {
        // Signaling only: rearm an established watchdog before any data path.
        if !matches!(message, LibertasEndpointMessage::NoPayload) {
            return LibertasEndpointHandlerResult::InvalidMessage;
        }
        if state.borrow().subscription_ready {
            arm_location_watchdog(state, HUB_LOCATION_MAX_REPORT_INTERVAL_SECONDS);
        }
        return LibertasEndpointHandlerResult::Handled;
    }

    if opcode == OP_ENDPOINT_RSP || opcode == OP_ENDPOINT_DATA {
        if let LibertasEndpointMessage::Data(HubProtocol::LocationRsp {
            longitude,
            latitude,
        }) = &message
        {
            let accepted = accept_hub_location(
                state,
                SprinklerWeatherLocationV1 {
                    longitude_degrees: *longitude,
                    latitude_degrees: *latitude,
                },
            );
            if accepted {
                state.borrow_mut().subscription_ready = true;
            }
            arm_location_watchdog(
                state,
                if accepted {
                    HUB_LOCATION_MAX_REPORT_INTERVAL_SECONDS
                } else {
                    HUB_LOCATION_RETRY_SECONDS
                },
            );
            return LibertasEndpointHandlerResult::Handled;
        }
        match message {
            LibertasEndpointMessage::Status(LibertasEndpointStandardStatus::PermissionDenied) => {
                libertas_log(
                    LogLevel::Error,
                    "Libertas Hub denied location access; the task requires ACCESS_FINE_LOCATION",
                )
            }
            LibertasEndpointMessage::Status(_) => libertas_log(
                LogLevel::Warn,
                "Libertas Hub location subscription returned a failure status",
            ),
            LibertasEndpointMessage::InvalidMessage => libertas_log(
                LogLevel::Warn,
                "Libertas Hub location subscription returned an invalid message",
            ),
            LibertasEndpointMessage::Data(_) | LibertasEndpointMessage::NoPayload => libertas_log(
                LogLevel::Warn,
                "Libertas Hub location subscription returned an unexpected protocol value",
            ),
        }
    } else if opcode == OP_ENDPOINT_PEER_DOWN {
        let timer = {
            let mut state = state.borrow_mut();
            state.hub_server_up = false;
            state.subscription_ready = false;
            state.retry_timer
        };
        if timer != 0 {
            libertas_timer_cancel(timer);
        }
        libertas_log(
            LogLevel::Warn,
            "Libertas Hub location server is down; subscription retry is suspended",
        );
        return LibertasEndpointHandlerResult::Handled;
    } else if opcode == OP_ENDPOINT_PEER_UP {
        // Up can arrive without the preceding Down. Re-establish the
        // subscription for this newer Hub endpoint startup.
        {
            let mut state = state.borrow_mut();
            state.hub_server_up = true;
            state.subscription_ready = false;
        }
        subscribe_to_hub_location(state);
        return LibertasEndpointHandlerResult::Handled;
    }

    arm_location_watchdog(state, HUB_LOCATION_RETRY_SECONDS);
    LibertasEndpointHandlerResult::Handled
}

fn update_peer_alive_timer(shared: &Rc<RefCell<WeatherServerState>>) {
    let (timer, next_ticks) = {
        let state = shared.borrow();
        (state.peer_alive_timer, state.next_peer_alive_ticks)
    };
    if timer == 0 {
        return;
    }
    if let Some(next_ticks) = next_ticks {
        libertas_timer_update_interval(timer, next_ticks);
    } else {
        libertas_timer_cancel(timer);
    }
}

fn handle_endpoint_event(
    endpoint: LibertasEndpoint,
    opcode: u8,
    request: Option<SprinklerWeatherProtocolV1>,
    context: &mut Box<dyn core::any::Any>,
    transaction_id: u32,
    peer: u32,
) -> LibertasEndpointStatus {
    let shared = context
        .downcast_mut::<Rc<RefCell<WeatherServerState>>>()
        .unwrap();

    if opcode == OP_ENDPOINT_PEER_DOWN {
        // The host removes this opaque route after the callback; no App roster.
        return LibertasEndpointStatus::Success;
    }
    if opcode != OP_ENDPOINT_REQ && opcode != OP_ENDPOINT_SUB_REQ {
        return LibertasEndpointStatus::Success;
    }

    let Some(request) = request else {
        return LibertasEndpointStatus::InvalidMessage;
    };
    let is_subscription = opcode == OP_ENDPOINT_SUB_REQ;
    let now_ticks = libertas_get_sys_ticks();
    let Some(prepared) = shared.borrow_mut().prepare_response(request, now_ticks) else {
        return LibertasEndpointStatus::InvalidMessage;
    };
    libertas_endpoint_response(endpoint, &prepared.message, transaction_id, peer);

    if is_subscription {
        if prepared.accepted {
            shared.borrow_mut().note_subscription(now_ticks);
            update_peer_alive_timer(shared);
        } else {
            libertas_endpoint_remove_subscriber(endpoint, peer);
        }
    }
    LibertasEndpointStatus::Success
}

/// Weather server
/// Provides application-tailored weather services. This initial version exposes
/// one sprinkler-weather server endpoint; future application protocols can add
/// their own typed endpoints and independently cached data. On startup it
/// dynamically reconstructs indexed hourly history and validates independently
/// persisted current conditions, forecast, and Hub location data. It subscribes
/// to the built-in Libertas Hub location endpoint at every startup. A valid
/// cached location keeps weather refreshes available during a temporary Hub
/// outage; without one, Open-Meteo requests wait for the first valid Hub report.
/// On a changed Hub location, weather for the old site is cleared before the
/// new location is persisted and replacement refreshes begin. A restart can
/// therefore leave an empty cache, but cannot relabel old-site weather as new.
///
/// HTTPS runs on a dedicated worker; all persistence, cursor, endpoint, timer,
/// and subscription operations run on the Libertas application thread.
/// Persisted retrieval timestamps preserve refresh schedules across restarts,
/// avoiding immediate rewrites while cached sections are not yet due. The
/// transient cursor and replay journal intentionally restart at sequence zero;
/// the agent publishes changes and one shared PeerAlive through host fan-out.
/// Clients recover with epoch-timestamp-and-sequence reset detection.
#[libertas_data_schema("libertas_weather::SprinklerWeatherPersistentDataV1")]
#[libertas_permissions(WEATHER_AGENT_PERMISSIONS)]
#[libertas_string_resources(APP_STRINGS)]
pub fn libertas_weather_server(sprinkler_weather: SprinklerWeatherEndpointServerV1) {
    let endpoint = sprinkler_weather.endpoint;
    let cached_location = load_location(endpoint);
    let mut snapshot = load_snapshot(endpoint);
    if cached_location.is_none()
        && (snapshot.history.is_some() || snapshot.current.is_some() || snapshot.forecast.is_some())
    {
        libertas_log(
            LogLevel::Warn,
            "Discarding persisted sprinkler weather that has no associated location",
        );
        let key = persistent_key(endpoint);
        clear_indexed_history(endpoint);
        libertas_data_remove_single(CURRENT_RESOURCE, &key);
        libertas_data_remove_single(FORECAST_RESOURCE, &key);
        snapshot = SprinklerWeatherSnapshotV2 {
            history: None,
            current: None,
            forecast: None,
        };
    }
    let utc_microseconds = libertas_get_utc_time();
    let now_utc = utc_microseconds.map(|microseconds| microseconds / MICROSECONDS_PER_SECOND);
    let epoch_timestamp = utc_microseconds.map(|microseconds| {
        microseconds.saturating_add(MICROSECONDS_PER_SECOND - 1) / MICROSECONDS_PER_SECOND
    });
    let shared = Rc::new(RefCell::new(WeatherServerState::new(
        endpoint,
        epoch_timestamp,
        snapshot,
    )));
    shared.borrow_mut().site_location = cached_location;

    let timer_shared = Rc::clone(&shared);
    let peer_alive_timer = libertas_timer_new_interval(
        0,
        move |timer, now_ticks, context| {
            let shared = context
                .downcast_mut::<Rc<RefCell<WeatherServerState>>>()
                .unwrap();
            let (endpoint, due) = {
                let mut state = shared.borrow_mut();
                let due = state.take_due_peer_alive(now_ticks);
                (state.endpoint, due)
            };
            if due {
                libertas_endpoint_peer_alive(endpoint, None);
            }
            let next_ticks = shared.borrow().next_peer_alive_ticks;
            if let Some(next_ticks) = next_ticks {
                libertas_timer_update_interval(timer, next_ticks);
            }
        },
        Box::new(timer_shared),
    );
    shared.borrow_mut().peer_alive_timer = peer_alive_timer;

    libertas_register_endpoint_listener::<SprinklerWeatherProtocolV1, _>(
        endpoint,
        handle_endpoint_event,
        Box::new(Rc::clone(&shared)),
    );

    let provider = match start_provider_control(Rc::clone(&shared)) {
        Ok(provider) => Some(provider),
        Err(error) => {
            libertas_log(LogLevel::Error, &error);
            None
        }
    };
    let location_state = Rc::new(RefCell::new(LocationSubscriptionState {
        weather: Rc::clone(&shared),
        provider: provider.clone(),
        location: cached_location,
        retry_timer: 0,
        hub_server_up: true,
        subscription_ready: false,
    }));
    let location_retry_timer = libertas_timer_new_interval(
        0,
        location_watchdog_fired,
        Box::new(Rc::clone(&location_state)),
    );
    location_state.borrow_mut().retry_timer = location_retry_timer;
    libertas_register_endpoint_status_listener::<HubProtocol, _>(
        LIBERTAS_HUB_ENDPOINT,
        handle_hub_location_event,
        Box::new(Rc::clone(&location_state)),
    );
    subscribe_to_hub_location(&location_state);

    if let (Some(provider), Some(location)) = (provider, cached_location)
        && provider.location.get().is_none()
    {
        let restored_snapshot = shared.borrow().snapshot.clone();
        provider.schedule_from_cache(
            location.into(),
            &restored_snapshot,
            now_utc,
            libertas_get_sys_ticks(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use libertas::AvroDecode;
    use libertas_weather::{
        SprinklerWeatherChangeV1, SprinklerWeatherForecastPeriodV1,
        SprinklerWeatherHistoryPeriodV1, SprinklerWeatherHistoryPeriodV2,
        SprinklerWeatherHistoryV1,
    };

    const OLD_EPOCH: LibertasDateTime = 1_784_972_800;
    const NEW_EPOCH: LibertasDateTime = OLD_EPOCH + 60;
    const ENDPOINT: LibertasEndpoint = 41;

    fn cursor(epoch_timestamp: LibertasDateTime, sequence: u64) -> SprinklerWeatherCursorV1 {
        SprinklerWeatherCursorV1 {
            epoch_timestamp,
            sequence,
        }
    }

    fn history_period(starts_at: LibertasDateTime) -> SprinklerWeatherHistoryPeriodV2 {
        SprinklerWeatherHistoryPeriodV2 {
            starts_at,
            duration_seconds: 3_600,
            temperature_celsius: 19.0,
            relative_humidity_percent: 72,
            precipitation_millimeters: 2.0,
            reference_evapotranspiration_millimeters: 0.2,
            wind_speed_meters_per_second: 2.5,
            wind_gust_meters_per_second: 4.5,
        }
    }

    fn legacy_history_period(starts_at: LibertasDateTime) -> SprinklerWeatherHistoryPeriodV1 {
        SprinklerWeatherHistoryPeriodV1 {
            starts_at,
            duration_seconds: 3_600,
            precipitation_millimeters: 2.0,
            reference_evapotranspiration_millimeters: 0.2,
        }
    }

    fn history() -> SprinklerWeatherHistoryV2 {
        SprinklerWeatherHistoryV2 {
            retrieved_at: OLD_EPOCH,
            valid_until: OLD_EPOCH + 7_200,
            periods: vec![
                history_period(OLD_EPOCH - 7_200),
                history_period(OLD_EPOCH - 3_600),
            ],
        }
    }

    fn current() -> SprinklerCurrentWeatherV1 {
        SprinklerCurrentWeatherV1 {
            retrieved_at: OLD_EPOCH,
            valid_until: OLD_EPOCH + 1_800,
            valid_at: OLD_EPOCH,
            interval_seconds: 900,
            temperature_celsius: 21.0,
            relative_humidity_percent: 67,
            precipitation_millimeters: 0.1,
            reference_evapotranspiration_millimeters: 0.05,
            wind_speed_meters_per_second: 2.5,
            wind_gust_meters_per_second: 4.0,
        }
    }

    fn forecast() -> SprinklerWeatherForecastV1 {
        SprinklerWeatherForecastV1 {
            retrieved_at: OLD_EPOCH,
            valid_until: OLD_EPOCH + 10_800,
            periods: vec![
                SprinklerWeatherForecastPeriodV1 {
                    starts_at: OLD_EPOCH,
                    duration_seconds: 3_600,
                    temperature_celsius: 22.0,
                    relative_humidity_percent: 65,
                    precipitation_probability_percent: 20,
                    expected_precipitation_millimeters: 0.0,
                    reference_evapotranspiration_millimeters: 0.3,
                    wind_speed_meters_per_second: 3.0,
                    wind_gust_meters_per_second: 5.0,
                },
                SprinklerWeatherForecastPeriodV1 {
                    starts_at: OLD_EPOCH + 3_600,
                    duration_seconds: 3_600,
                    temperature_celsius: 23.0,
                    relative_humidity_percent: 62,
                    precipitation_probability_percent: 50,
                    expected_precipitation_millimeters: 1.0,
                    reference_evapotranspiration_millimeters: 0.25,
                    wind_speed_meters_per_second: 3.5,
                    wind_gust_meters_per_second: 5.5,
                },
            ],
        }
    }

    fn snapshot() -> SprinklerWeatherSnapshotV2 {
        SprinklerWeatherSnapshotV2 {
            history: Some(history()),
            current: Some(current()),
            forecast: Some(forecast()),
        }
    }

    fn location() -> SprinklerWeatherLocationV1 {
        SprinklerWeatherLocationV1 {
            longitude_degrees: -74.006,
            latitude_degrees: 40.7128,
        }
    }

    fn full_history_range() -> SprinklerWeatherTimeRangeV1 {
        SprinklerWeatherTimeRangeV1 {
            starts_at: OLD_EPOCH - 7_200,
            ends_before: OLD_EPOCH,
        }
    }

    fn full_forecast_range() -> SprinklerWeatherTimeRangeV1 {
        SprinklerWeatherTimeRangeV1 {
            starts_at: OLD_EPOCH,
            ends_before: OLD_EPOCH + 7_200,
        }
    }

    fn recover(
        state: &mut WeatherServerState,
        after_cursor: Option<SprinklerWeatherCursorV1>,
    ) -> SprinklerWeatherRecoveryV1 {
        state.recover(
            after_cursor,
            Some(full_history_range()),
            true,
            Some(full_forecast_range()),
            0,
        )
    }

    fn open_meteo_hourly() -> OpenMeteoHourly {
        let json = format!(
            r#"{{
                "hourly": {{
                    "time": [{}, {}, {}, {}],
                    "temperature_2m": [19.0, 20.0, 21.0, 22.0],
                    "relative_humidity_2m": [82, 76, 69, 64],
                    "precipitation_probability": [10, 20, 30, 40],
                    "precipitation": [0.4, 0.2, 0.0, 1.0],
                    "et0_fao_evapotranspiration": [0.1, 0.2, 0.3, 0.4],
                    "wind_speed_10m": [2.0, 2.5, 3.0, 3.5],
                    "wind_gusts_10m": [4.0, 4.5, 5.0, 5.5]
                }}
            }}"#,
            OLD_EPOCH - 3_600,
            OLD_EPOCH,
            OLD_EPOCH + 3_600,
            OLD_EPOCH + 7_200,
        );
        serde_json::from_str::<OpenMeteoHourlyResponse>(&json)
            .unwrap()
            .hourly
    }

    #[test]
    fn open_meteo_current_json_maps_to_sprinkler_units() {
        let response: OpenMeteoCurrentResponse = serde_json::from_str(
            r#"{
                "current": {
                    "time": 1784972800,
                    "interval": 900,
                    "temperature_2m": 21.5,
                    "relative_humidity_2m": 73,
                    "precipitation": 0.4,
                    "et0_fao_evapotranspiration": 0.05,
                    "wind_speed_10m": 3.2,
                    "wind_gusts_10m": 5.8
                }
            }"#,
        )
        .unwrap();

        let current = build_current(response.current, OLD_EPOCH).unwrap();

        assert_eq!(current.valid_at, 1_784_972_800);
        assert_eq!(current.interval_seconds, 900);
        assert_eq!(current.temperature_celsius, 21.5);
        assert_eq!(current.relative_humidity_percent, 73);
        assert_eq!(current.precipitation_millimeters, 0.4);
        assert_eq!(current.wind_speed_meters_per_second, 3.2);
        assert_eq!(
            current.valid_until,
            OLD_EPOCH + u64::from(SPRINKLER_CURRENT_FRESHNESS_SECONDS)
        );
    }

    #[test]
    fn open_meteo_hourly_json_splits_completed_history_and_future_forecast() {
        let hourly = open_meteo_hourly();

        let history = build_history(&hourly, OLD_EPOCH, OLD_EPOCH).unwrap();
        let forecast = build_forecast(&hourly, OLD_EPOCH, OLD_EPOCH).unwrap();

        assert_eq!(history.periods.len(), 2);
        assert_eq!(history.periods[0].starts_at, OLD_EPOCH - 7_200);
        assert_eq!(history.periods[1].starts_at, OLD_EPOCH - 3_600);
        assert_eq!(history.periods[0].temperature_celsius, 19.0);
        assert_eq!(history.periods[0].relative_humidity_percent, 82);
        assert_eq!(history.periods[0].wind_speed_meters_per_second, 2.0);
        assert_eq!(history.periods[0].wind_gust_meters_per_second, 4.0);
        assert_eq!(forecast.periods.len(), 2);
        assert_eq!(forecast.periods[0].starts_at, OLD_EPOCH);
        assert_eq!(forecast.periods[1].starts_at, OLD_EPOCH + 3_600);
        assert_eq!(forecast.periods[0].relative_humidity_percent, 69);
        assert_eq!(forecast.periods[0].precipitation_probability_percent, 30);
    }

    #[test]
    fn incomplete_provider_section_is_rejected_without_partial_acceptance() {
        let mut hourly = open_meteo_hourly();
        hourly.et0_fao_evapotranspiration[1] = None;
        hourly.relative_humidity_2m[2] = None;
        hourly.wind_gusts_10m.pop();

        assert!(build_history(&hourly, OLD_EPOCH, OLD_EPOCH).is_err());
        assert!(build_forecast(&hourly, OLD_EPOCH, OLD_EPOCH).is_err());
    }

    #[test]
    fn invalid_forecast_humidity_does_not_invalidate_history() {
        let mut hourly = open_meteo_hourly();
        hourly.relative_humidity_2m[2] = Some(101.0);

        assert!(build_history(&hourly, OLD_EPOCH, OLD_EPOCH).is_ok());
        assert!(build_forecast(&hourly, OLD_EPOCH, OLD_EPOCH).is_err());
    }

    #[test]
    fn invalid_historical_wind_does_not_invalidate_forecast() {
        let mut hourly = open_meteo_hourly();
        hourly.wind_speed_10m[0] = Some(-0.1);

        assert!(build_history(&hourly, OLD_EPOCH, OLD_EPOCH).is_err());
        assert!(build_forecast(&hourly, OLD_EPOCH, OLD_EPOCH).is_ok());
    }

    #[test]
    fn provider_locations_require_finite_wgs84_coordinates() {
        assert!(valid_location(ProviderLocation {
            latitude_degrees: 40.7128,
            longitude_degrees: -74.0060,
        }));
        assert!(!valid_location(ProviderLocation {
            latitude_degrees: f64::NAN,
            longitude_degrees: 0.0,
        }));
        assert!(!valid_location(ProviderLocation {
            latitude_degrees: 0.0,
            longitude_degrees: 181.0,
        }));
    }

    #[test]
    fn insignificant_hub_location_noise_does_not_change_the_provider_site() {
        let original = location();
        let close = SprinklerWeatherLocationV1 {
            longitude_degrees: original.longitude_degrees
                + LOCATION_EQUALITY_TOLERANCE_DEGREES / 2.0,
            latitude_degrees: original.latitude_degrees - LOCATION_EQUALITY_TOLERANCE_DEGREES / 2.0,
        };
        let changed = SprinklerWeatherLocationV1 {
            longitude_degrees: original.longitude_degrees
                + LOCATION_EQUALITY_TOLERANCE_DEGREES * 2.0,
            ..original
        };

        assert!(valid_weather_location(original));
        assert!(same_weather_location(original, close));
        assert!(!same_weather_location(original, changed));
    }

    #[test]
    fn shutdown_handler_sets_stop_before_waking_the_worker() {
        let (commands, receiver) = sync_channel(1);
        let stop_requested = Arc::new(AtomicBool::new(false));
        let mut context: Box<dyn Any> = Box::new(ProviderShutdownContext {
            commands,
            stop_requested: Arc::clone(&stop_requested),
        });

        handle_provider_shutdown(&mut context);

        assert!(stop_requested.load(Ordering::Acquire));
        assert!(matches!(receiver.try_recv(), Ok(ProviderCommand::Shutdown)));
    }

    #[test]
    fn startup_preserves_remaining_refresh_intervals_from_cached_data() {
        let mut restored = snapshot();
        restored.current.as_mut().unwrap().retrieved_at = OLD_EPOCH;
        restored.history.as_mut().unwrap().retrieved_at = OLD_EPOCH + 120;
        restored.forecast.as_mut().unwrap().retrieved_at = OLD_EPOCH + 60;

        assert_eq!(
            startup_refresh_delays(&restored, Some(OLD_EPOCH + 300)),
            (600, 3_360)
        );
    }

    #[test]
    fn startup_refreshes_missing_overdue_or_untrusted_cache_immediately() {
        assert_eq!(
            remaining_refresh_delay_seconds(
                None,
                SPRINKLER_CURRENT_REFRESH_INTERVAL_SECONDS,
                Some(OLD_EPOCH),
            ),
            0
        );
        assert_eq!(
            remaining_refresh_delay_seconds(
                Some(OLD_EPOCH),
                SPRINKLER_CURRENT_REFRESH_INTERVAL_SECONDS,
                None,
            ),
            0
        );
        assert_eq!(
            remaining_refresh_delay_seconds(
                Some(OLD_EPOCH),
                SPRINKLER_CURRENT_REFRESH_INTERVAL_SECONDS,
                Some(OLD_EPOCH + u64::from(SPRINKLER_CURRENT_REFRESH_INTERVAL_SECONDS)),
            ),
            0
        );
        assert_eq!(
            remaining_refresh_delay_seconds(
                Some(OLD_EPOCH + 1),
                SPRINKLER_CURRENT_REFRESH_INTERVAL_SECONDS,
                Some(OLD_EPOCH),
            ),
            0
        );

        let mut restored = snapshot();
        restored.forecast = None;
        assert_eq!(
            startup_refresh_delays(&restored, Some(OLD_EPOCH + 300)).1,
            0
        );
    }

    #[test]
    fn provider_change_advances_cursor_journal_and_broadcast_atomically() {
        let replacement = history();
        let mut state = WeatherServerState::new(
            ENDPOINT,
            Some(NEW_EPOCH),
            SprinklerWeatherSnapshotV2 {
                history: None,
                current: None,
                forecast: None,
            },
        );
        let publication = state.apply_change(
            SprinklerWeatherChangeV1::HistoryReplaceV2 {
                history: replacement.clone(),
            },
            100,
            NEW_EPOCH,
        );

        assert_eq!(state.snapshot.history, Some(replacement.clone()));
        assert_eq!(state.cursor, Some(cursor(NEW_EPOCH, 1)));
        assert_eq!(state.journal.len(), 1);
        let Some(SprinklerWeatherProtocolV1::WeatherIncrementV1 { report }) = &publication.report
        else {
            panic!("expected incremental broadcast report");
        };
        assert_eq!(report.from_cursor, cursor(NEW_EPOCH, 0));
        assert_eq!(report.through_cursor, cursor(NEW_EPOCH, 1));
        assert_eq!(
            report.changes,
            vec![SprinklerWeatherChangeV1::HistoryReplaceV2 {
                history: replacement,
            }]
        );
    }

    #[test]
    fn legacy_history_change_is_never_published_as_v2() {
        let original = snapshot();
        let mut state = WeatherServerState::new(ENDPOINT, Some(NEW_EPOCH), original.clone());
        let publication = state.apply_change(
            SprinklerWeatherChangeV1::HistoryReplaceV1 {
                history: SprinklerWeatherHistoryV1 {
                    retrieved_at: OLD_EPOCH,
                    valid_until: OLD_EPOCH + 7_200,
                    periods: vec![legacy_history_period(OLD_EPOCH - 3_600)],
                },
            },
            100,
            NEW_EPOCH,
        );

        assert!(publication.report.is_none());
        assert_eq!(state.snapshot, original);
        assert_eq!(state.cursor, Some(cursor(NEW_EPOCH, 0)));
        assert!(state.journal.is_empty());
    }

    #[test]
    fn location_change_clear_is_an_incremental_weather_change() {
        let mut state = WeatherServerState::new(ENDPOINT, Some(NEW_EPOCH), snapshot());
        let publication = state.apply_change(
            SprinklerWeatherChangeV1::SectionClearV1 {
                section: SprinklerWeatherSectionV1::Current,
            },
            100,
            NEW_EPOCH,
        );

        assert!(state.snapshot.history.is_some());
        assert!(state.snapshot.current.is_none());
        assert!(state.snapshot.forecast.is_some());
        assert_eq!(state.cursor, Some(cursor(NEW_EPOCH, 1)));
        let Some(SprinklerWeatherProtocolV1::WeatherIncrementV1 { report }) = &publication.report
        else {
            panic!("expected incremental clear broadcast");
        };
        assert_eq!(
            report.changes,
            vec![SprinklerWeatherChangeV1::SectionClearV1 {
                section: SprinklerWeatherSectionV1::Current,
            }]
        );
    }

    #[test]
    fn endpoint_configuration_round_trips_through_avro() {
        let value = SprinklerWeatherEndpointServerV1 { endpoint: ENDPOINT };
        let encoded = value.to_avro();
        let mut offset = 0;
        let decoded = SprinklerWeatherEndpointServerV1::avro_decode(&encoded, &mut offset).unwrap();

        assert_eq!(decoded, value);
        assert_eq!(offset, encoded.len());
    }

    #[test]
    fn initial_request_returns_the_requested_cached_sections() {
        let mut state = WeatherServerState::new(ENDPOINT, Some(NEW_EPOCH), snapshot());
        state.site_location = Some(location());

        let SprinklerWeatherRecoveryV1::ResetAtSiteV2 {
            reason,
            cursor: initial_cursor,
            location: reset_location,
            snapshot: selected,
        } = recover(&mut state, None)
        else {
            panic!("expected initial reset snapshot");
        };

        assert_eq!(reason, SprinklerWeatherResetReasonV1::InitialSubscription);
        assert_eq!(initial_cursor, cursor(NEW_EPOCH, 0));
        assert_eq!(reset_location, location());
        assert_eq!(selected, snapshot());
    }

    #[test]
    fn reset_snapshot_carries_the_provider_site_when_known() {
        let mut state = WeatherServerState::new(ENDPOINT, Some(NEW_EPOCH), snapshot());
        state.apply_change(
            SprinklerWeatherChangeV1::SiteReplaceV1 {
                location: location(),
            },
            100,
            NEW_EPOCH,
        );

        let SprinklerWeatherRecoveryV1::ResetAtSiteV2 {
            reason,
            cursor: initial_cursor,
            location: reset_location,
            snapshot: selected,
        } = recover(&mut state, None)
        else {
            panic!("expected a site-bound initial reset snapshot");
        };

        assert_eq!(reason, SprinklerWeatherResetReasonV1::InitialSubscription);
        assert_eq!(initial_cursor, cursor(NEW_EPOCH, 1));
        assert_eq!(reset_location, location());
        assert_eq!(
            selected,
            SprinklerWeatherSnapshotV2 {
                history: None,
                current: None,
                forecast: None,
            }
        );
    }

    #[test]
    fn server_reset_preserves_weather_and_supports_a_nonzero_backward_cursor() {
        let original = snapshot();
        let mut state = WeatherServerState::new(ENDPOINT, Some(NEW_EPOCH), original.clone());
        state.site_location = Some(location());
        state.cursor = Some(cursor(NEW_EPOCH, 3));

        let SprinklerWeatherRecoveryV1::ResetAtSiteV2 {
            reason,
            cursor: reset_cursor,
            location: reset_location,
            snapshot: recovered,
        } = recover(&mut state, Some(cursor(OLD_EPOCH, 10)))
        else {
            panic!("expected server cursor reset");
        };

        assert_eq!(reason, SprinklerWeatherResetReasonV1::ServerCursorReset);
        assert!(reset_cursor.is_server_reset_after(cursor(OLD_EPOCH, 10)));
        assert_eq!(reset_location, location());
        assert_eq!(recovered, original);
    }

    #[test]
    fn stale_or_inconsistent_cursor_does_not_roll_state_backward() {
        let original = snapshot();
        let mut state = WeatherServerState::new(ENDPOINT, Some(NEW_EPOCH), original.clone());
        state.cursor = Some(cursor(NEW_EPOCH, 12));

        assert_eq!(
            recover(&mut state, Some(cursor(OLD_EPOCH, 10))),
            SprinklerWeatherRecoveryV1::ErrorV1 {
                error: SprinklerWeatherRecoveryErrorV1::CursorAhead,
                retry_after_seconds: None,
            }
        );
        assert_eq!(state.snapshot, original);
    }

    #[test]
    fn caught_up_cursor_receives_an_empty_contiguous_replay() {
        let mut state = WeatherServerState::new(ENDPOINT, Some(NEW_EPOCH), snapshot());
        state.cursor = Some(cursor(NEW_EPOCH, 12));

        let SprinklerWeatherRecoveryV1::ReplayedV1 { report } =
            recover(&mut state, Some(cursor(NEW_EPOCH, 12)))
        else {
            panic!("expected caught-up replay");
        };

        assert!(report.can_apply_after(cursor(NEW_EPOCH, 12)));
        assert!(report.changes.is_empty());
    }

    #[test]
    fn retained_journal_replays_an_exact_contiguous_range() {
        let mut state = WeatherServerState::new(ENDPOINT, Some(NEW_EPOCH), snapshot());
        let first_change = SprinklerWeatherChangeV1::CurrentReplaceV1 { current: current() };
        let second_change = SprinklerWeatherChangeV1::ForecastPeriodsUpsertV1 {
            retrieved_at: forecast().retrieved_at,
            valid_until: forecast().valid_until,
            periods: forecast().periods,
        };
        state.cursor = Some(cursor(NEW_EPOCH, 2));
        state.journal = vec![
            JournalEntry {
                recorded_at_ticks: 10,
                report: SprinklerWeatherIncrementalReportV1 {
                    from_cursor: cursor(NEW_EPOCH, 0),
                    through_cursor: cursor(NEW_EPOCH, 1),
                    changes: vec![first_change.clone()],
                },
            },
            JournalEntry {
                recorded_at_ticks: 20,
                report: SprinklerWeatherIncrementalReportV1 {
                    from_cursor: cursor(NEW_EPOCH, 1),
                    through_cursor: cursor(NEW_EPOCH, 2),
                    changes: vec![second_change.clone()],
                },
            },
        ];

        let SprinklerWeatherRecoveryV1::ReplayedV1 { report } =
            recover(&mut state, Some(cursor(NEW_EPOCH, 0)))
        else {
            panic!("expected retained replay");
        };

        assert_eq!(report.from_cursor, cursor(NEW_EPOCH, 0));
        assert_eq!(report.through_cursor, cursor(NEW_EPOCH, 2));
        assert_eq!(report.changes, vec![first_change, second_change]);
        assert!(report.has_contiguous_cursor_range());
    }

    #[test]
    fn expired_journal_recovers_with_a_snapshot() {
        let mut state = WeatherServerState::new(ENDPOINT, Some(NEW_EPOCH), snapshot());
        state.site_location = Some(location());
        state.cursor = Some(cursor(NEW_EPOCH, 1));
        state.journal.push(JournalEntry {
            recorded_at_ticks: 1,
            report: SprinklerWeatherIncrementalReportV1 {
                from_cursor: cursor(NEW_EPOCH, 0),
                through_cursor: cursor(NEW_EPOCH, 1),
                changes: vec![SprinklerWeatherChangeV1::CurrentReplaceV1 { current: current() }],
            },
        });
        let after_replay_window =
            u64::from(SPRINKLER_SUBSCRIPTION_REPLAY_WINDOW_SECONDS) * MICROSECONDS_PER_SECOND + 2;

        let recovery = state.recover(
            Some(cursor(NEW_EPOCH, 0)),
            Some(full_history_range()),
            true,
            Some(full_forecast_range()),
            after_replay_window,
        );

        assert!(matches!(
            recovery,
            SprinklerWeatherRecoveryV1::ResetAtSiteV2 {
                reason: SprinklerWeatherResetReasonV1::CursorExpired,
                ..
            }
        ));
        assert!(state.journal.is_empty());
    }

    #[test]
    fn recovery_ranges_are_validated_and_bounded() {
        let state = WeatherServerState::new(ENDPOINT, Some(NEW_EPOCH), snapshot());

        assert_eq!(
            state.select_snapshot(
                Some(SprinklerWeatherTimeRangeV1 {
                    starts_at: OLD_EPOCH,
                    ends_before: OLD_EPOCH,
                }),
                false,
                None,
            ),
            Err(SprinklerWeatherRecoveryErrorV1::InvalidRange)
        );
        assert_eq!(
            state.select_snapshot(
                Some(SprinklerWeatherTimeRangeV1 {
                    starts_at: OLD_EPOCH,
                    ends_before: OLD_EPOCH + u64::from(SPRINKLER_HISTORY_WINDOW_SECONDS) + 1,
                }),
                false,
                None,
            ),
            Err(SprinklerWeatherRecoveryErrorV1::RequestTooLarge)
        );
    }

    #[test]
    fn cached_sections_are_validated_before_use() {
        assert!(valid_history(&history()));
        assert!(valid_current(&current()));
        assert!(valid_forecast(&forecast()));

        let mut invalid_history = history();
        invalid_history.periods[1].starts_at = invalid_history.periods[0].starts_at;
        assert!(!valid_history(&invalid_history));
        let mut invalid_history_temperature = history();
        invalid_history_temperature.periods[0].temperature_celsius = f32::NAN;
        assert!(!valid_history(&invalid_history_temperature));
        let mut invalid_history_humidity = history();
        invalid_history_humidity.periods[0].relative_humidity_percent = 101;
        assert!(!valid_history(&invalid_history_humidity));
        let mut invalid_history_wind = history();
        invalid_history_wind.periods[0].wind_gust_meters_per_second = -0.1;
        assert!(!valid_history(&invalid_history_wind));

        let mut invalid_current = current();
        invalid_current.wind_speed_meters_per_second = f32::NAN;
        assert!(!valid_current(&invalid_current));
        let mut invalid_current_humidity = current();
        invalid_current_humidity.relative_humidity_percent = 101;
        assert!(!valid_current(&invalid_current_humidity));

        let mut invalid_forecast = forecast();
        invalid_forecast.periods[0].duration_seconds = 0;
        assert!(!valid_forecast(&invalid_forecast));
        let mut invalid_forecast_humidity = forecast();
        invalid_forecast_humidity.periods[0].relative_humidity_percent = 101;
        assert!(!valid_forecast(&invalid_forecast_humidity));
    }

    #[test]
    fn indexed_history_is_reconstructed_in_time_order_and_invalid_records_are_removed() {
        let expected = history();
        let metadata = history_metadata(&expected);
        let first = expected.periods[0];
        let second = expected.periods[1];
        let mismatched = history_period(OLD_EPOCH - 10_800);
        let wrong_variant_index = i64::try_from(OLD_EPOCH - 14_400).unwrap();
        let records = vec![
            IndexedData {
                index: history_period_index(&second).unwrap(),
                data: SprinklerWeatherPersistentDataV1::HistoryPeriodV2 { period: second },
            },
            IndexedData {
                index: history_period_index(&mismatched).unwrap() + 1,
                data: SprinklerWeatherPersistentDataV1::HistoryPeriodV2 { period: mismatched },
            },
            IndexedData {
                index: wrong_variant_index,
                data: SprinklerWeatherPersistentDataV1::CurrentV1 { current: current() },
            },
            IndexedData {
                index: history_period_index(&first).unwrap(),
                data: SprinklerWeatherPersistentDataV1::HistoryPeriodV2 { period: first },
            },
        ];

        let reconstructed = reconstruct_indexed_history(metadata, &records);
        assert_eq!(reconstructed.history, Some(expected));
        assert!(!reconstructed.legacy_v1_records_found);
        assert_eq!(
            reconstructed.records_to_remove,
            vec![
                wrong_variant_index,
                history_period_index(&mismatched).unwrap() + 1
            ]
        );
    }

    #[test]
    fn legacy_v1_history_is_flagged_for_cache_reset_without_fabricated_metrics() {
        let expected = history();
        let metadata = history_metadata(&expected);
        let v2_period = expected.periods[1];
        let legacy_period = legacy_history_period(OLD_EPOCH - 7_200);
        let legacy_index = i64::try_from(legacy_period.starts_at).unwrap();
        let records = vec![
            IndexedData {
                index: legacy_index,
                data: SprinklerWeatherPersistentDataV1::HistoryPeriodV1 {
                    period: legacy_period,
                },
            },
            IndexedData {
                index: history_period_index(&v2_period).unwrap(),
                data: SprinklerWeatherPersistentDataV1::HistoryPeriodV2 { period: v2_period },
            },
        ];

        let reconstructed = reconstruct_indexed_history(metadata, &records);

        assert!(reconstructed.legacy_v1_records_found);
        assert_eq!(reconstructed.records_to_remove, vec![legacy_index]);
        assert_eq!(
            reconstructed
                .history
                .expect("the pure scan may retain independently valid V2 rows")
                .periods,
            vec![v2_period]
        );

        let mut post_migration_snapshot = snapshot();
        post_migration_snapshot.history = None;
        assert_eq!(
            startup_refresh_delays(&post_migration_snapshot, Some(OLD_EPOCH + 300)).1,
            0,
            "clearing legacy history must schedule an immediate hourly refetch"
        );
    }

    #[test]
    fn one_history_correction_produces_one_indexed_upsert() {
        let previous = history();
        let mut corrected = previous.periods[0];
        corrected.precipitation_millimeters += 1.0;
        let added = history_period(OLD_EPOCH);
        let current = SprinklerWeatherHistoryV2 {
            retrieved_at: OLD_EPOCH + 3_600,
            valid_until: OLD_EPOCH + 10_800,
            periods: vec![corrected, added],
        };

        let delta = indexed_history_delta(Some(&previous), &current);
        assert_eq!(delta.upserts, vec![corrected, added]);
        assert_eq!(
            delta.removals,
            vec![history_period_index(&previous.periods[1]).unwrap()]
        );

        let correction_only = SprinklerWeatherHistoryV2 {
            periods: vec![corrected, previous.periods[1]],
            ..previous.clone()
        };
        let delta = indexed_history_delta(Some(&previous), &correction_only);
        assert_eq!(delta.upserts, vec![corrected]);
        assert!(delta.removals.is_empty());
    }

    #[test]
    fn peer_alive_is_due_before_the_maximum_wait_without_touching_the_cursor() {
        let start_ticks = 100;
        let interval = WeatherServerState::peer_alive_interval_ticks();
        let maximum_wait = u64::from(SPRINKLER_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS)
            .saturating_mul(MICROSECONDS_PER_SECOND);
        let mut state = WeatherServerState::new(ENDPOINT, Some(NEW_EPOCH), snapshot());
        state.note_subscription(start_ticks);
        let cursor = state.cursor;

        assert!(interval < maximum_wait);
        assert!(!state.take_due_peer_alive(start_ticks + interval - 1));
        assert!(state.take_due_peer_alive(start_ticks + interval));
        assert_eq!(state.cursor, cursor);
        assert_eq!(
            state.next_peer_alive_ticks,
            Some(start_ticks + interval.saturating_mul(2))
        );
    }

    #[test]
    fn confirmed_client_down_leaves_membership_removal_and_shared_cadence_to_the_host() {
        let shared = Rc::new(RefCell::new(WeatherServerState::new(
            ENDPOINT,
            Some(NEW_EPOCH),
            snapshot(),
        )));
        shared.borrow_mut().note_subscription(100);
        let deadline = shared.borrow().next_peer_alive_ticks;
        let mut context: Box<dyn Any> = Box::new(Rc::clone(&shared));

        assert_eq!(
            handle_endpoint_event(ENDPOINT, OP_ENDPOINT_PEER_DOWN, None, &mut context, 0, 7),
            LibertasEndpointStatus::Success
        );
        assert_eq!(shared.borrow().next_peer_alive_ticks, deadline);
    }

    #[test]
    fn request_roles_are_enforced_and_responses_have_a_maximum_wait_interval() {
        let mut state = WeatherServerState::new(ENDPOINT, Some(NEW_EPOCH), snapshot());
        state.site_location = Some(location());
        assert!(
            state
                .prepare_response(
                    SprinklerWeatherProtocolV1::WeatherIncrementV1 {
                        report: empty_report(cursor(NEW_EPOCH, 0)),
                    },
                    0,
                )
                .is_none()
        );

        let prepared = state
            .prepare_response(
                SprinklerWeatherProtocolV1::GetWeatherV1 {
                    after_cursor: None,
                    history_range: None,
                    include_current: true,
                    forecast_range: None,
                },
                0,
            )
            .unwrap();
        assert!(prepared.accepted);
        assert!(matches!(
            prepared.message,
            SprinklerWeatherProtocolV1::WeatherRecoveryV1 {
                maximum_wait_interval_seconds: SPRINKLER_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
                ..
            }
        ));
    }

    #[test]
    fn missing_utc_returns_a_retryable_error_without_accepting_subscription() {
        let mut state = WeatherServerState::new(ENDPOINT, None, snapshot());
        let prepared = state
            .prepare_response(
                SprinklerWeatherProtocolV1::GetWeatherV1 {
                    after_cursor: None,
                    history_range: None,
                    include_current: true,
                    forecast_range: None,
                },
                0,
            )
            .unwrap();

        assert!(!prepared.accepted);
        assert!(matches!(
            prepared.message,
            SprinklerWeatherProtocolV1::WeatherRecoveryV1 {
                recovery: SprinklerWeatherRecoveryV1::ErrorV1 {
                    error: SprinklerWeatherRecoveryErrorV1::TemporarilyUnavailable,
                    retry_after_seconds: Some(RETRY_WITHOUT_UTC_SECONDS),
                },
                ..
            }
        ));
    }
}
