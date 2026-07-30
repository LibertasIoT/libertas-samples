//! Libertas Weather
//! Defines versioned weather data tailored to the decisions made by Libertas
//! applications.
//!
//! The sprinkler schema persists its Hub-provided site location separately from
//! recent history, current conditions, and forecast data. Each weather section
//! has a different refresh interval and may succeed or fail independently.
//! Runtime snapshots may contain any combination of sections. Applications
//! persist each successful section separately and retain older sections when a
//! weather-data refresh fails. Incremental subscriptions use
//! epoch-timestamp-and-sequence cursors so clients can distinguish a server
//! cursor reset from stale or out-of-order data, replay retained changes, or
//! recover selected time ranges from cached data.
//!
//! The building-HVAC schema supplies only the outdoor inputs needed for thermal
//! load prediction, heat-pump operation, economizer decisions, and controlled
//! outdoor-air ventilation. It keeps weather and outdoor-air-quality sections
//! independent because they have different providers and freshness behavior.
//! Local equipment and life-safety controls must continue to use their own
//! sensors; cached internet weather is supervisory input, not a safety signal.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod building_hvac;

use alloc::vec::Vec;
use libertas::LibertasDateTime;
use libertas_macros::{LibertasAvroDecode, LibertasAvroEncode, LibertasExport};

pub use building_hvac::{
    BUILDING_HVAC_AIR_QUALITY_FRESHNESS_SECONDS, BUILDING_HVAC_AIR_QUALITY_HORIZON_SECONDS,
    BUILDING_HVAC_AIR_QUALITY_REFRESH_INTERVAL_SECONDS, BUILDING_HVAC_CURRENT_FRESHNESS_SECONDS,
    BUILDING_HVAC_CURRENT_REFRESH_INTERVAL_SECONDS, BUILDING_HVAC_FORECAST_FRESHNESS_SECONDS,
    BUILDING_HVAC_FORECAST_HORIZON_SECONDS, BUILDING_HVAC_FORECAST_REFRESH_INTERVAL_SECONDS,
    BUILDING_HVAC_HISTORY_FRESHNESS_SECONDS, BUILDING_HVAC_HISTORY_REFRESH_INTERVAL_SECONDS,
    BUILDING_HVAC_HISTORY_WINDOW_SECONDS, BUILDING_HVAC_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
    BUILDING_HVAC_SUBSCRIPTION_REPLAY_WINDOW_SECONDS, BuildingHvacCurrentWeatherV1,
    BuildingHvacOutdoorAirQualityPeriodV1, BuildingHvacOutdoorAirQualityV1,
    BuildingHvacOutdoorConditionsV1, BuildingHvacPrecipitationKindV1, BuildingHvacWeatherChangeV1,
    BuildingHvacWeatherCursorV1, BuildingHvacWeatherForecastPeriodV1,
    BuildingHvacWeatherForecastV1, BuildingHvacWeatherHistoryPeriodV1,
    BuildingHvacWeatherHistoryV1, BuildingHvacWeatherIncrementalReportV1,
    BuildingHvacWeatherLocationV1, BuildingHvacWeatherPersistentDataV1,
    BuildingHvacWeatherProtocolV1, BuildingHvacWeatherRecoveryErrorV1,
    BuildingHvacWeatherRecoveryV1, BuildingHvacWeatherResetReasonV1, BuildingHvacWeatherSectionV1,
    BuildingHvacWeatherSnapshotV1, BuildingHvacWeatherTimeRangeV1,
};

/// Current weather refresh interval
/// The default number of seconds between requests for current sprinkler
/// weather. Open-Meteo current conditions represent 15-minute model intervals,
/// so requesting them more frequently normally returns no new interval.
pub const SPRINKLER_CURRENT_REFRESH_INTERVAL_SECONDS: u32 = 15 * 60;

/// Historical weather refresh interval
/// The default number of seconds between requests for recent sprinkler weather
/// history. The history contains completed hourly periods, so it is refreshed
/// once per hour.
pub const SPRINKLER_HISTORY_REFRESH_INTERVAL_SECONDS: u32 = 60 * 60;

/// Weather forecast refresh interval
/// The default number of seconds between requests for sprinkler weather
/// forecasts. Best-match forecast models update at different rates; hourly
/// polling discovers fast updates without excessive requests for slower models.
pub const SPRINKLER_FORECAST_REFRESH_INTERVAL_SECONDS: u32 = 60 * 60;

/// Current weather freshness
/// The number of seconds after retrieval for which current conditions are
/// considered fresh. Current conditions at or beyond this age must not be used
/// as proof that watering is safe.
pub const SPRINKLER_CURRENT_FRESHNESS_SECONDS: u32 = 2 * 15 * 60;

/// Historical weather freshness
/// The number of seconds after retrieval for which recent weather history is
/// considered fresh. Older history remains useful as degraded cached data.
pub const SPRINKLER_HISTORY_FRESHNESS_SECONDS: u32 = 2 * 60 * 60;

/// Weather forecast freshness
/// The number of seconds after retrieval for which a weather forecast is
/// considered fresh. An older forecast may be used as degraded cached data only
/// when its age and reduced reliability are taken into account.
pub const SPRINKLER_FORECAST_FRESHNESS_SECONDS: u32 = 3 * 60 * 60;

/// Historical weather window
/// The requested number of seconds of recent hourly weather history. Seven days
/// lets a sprinkler reconstruct a recent irrigation water balance after a
/// restart or temporary outage.
pub const SPRINKLER_HISTORY_WINDOW_SECONDS: u32 = 7 * 24 * 60 * 60;

/// Weather forecast horizon
/// The requested number of seconds of future hourly weather data. Seven days
/// covers the normal sprinkler scheduling horizon without retaining unrelated
/// long-range weather.
pub const SPRINKLER_FORECAST_HORIZON_SECONDS: u32 = 7 * 24 * 60 * 60;

/// Subscription replay window
/// The default number of seconds for retaining the in-memory incremental-change
/// journal. A client whose cursor is older than this window recovers with a
/// range-limited snapshot instead of replaying every missed change.
pub const SPRINKLER_SUBSCRIPTION_REPLAY_WINDOW_SECONDS: u32 = 24 * 60 * 60;

/// Subscription maximum wait interval
/// The default maximum number of seconds a subscribed client waits after a
/// response or data report before retrying `GetWeatherV1` with its last
/// applied cursor. The server sends an incremental report, including an empty
/// heartbeat report when necessary, before this interval expires.
pub const SPRINKLER_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS: u32 = 20 * 60;

/// Sprinkler weather history period V1
/// Contains the precipitation input and reference evapotranspiration loss for
/// one completed period in a sprinkler irrigation water balance.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWeatherHistoryPeriodV1 {
    /// Start time
    /// The inclusive date and time at which this historical period begins.
    pub starts_at: LibertasDateTime,
    /// Duration
    /// The length of this historical period in seconds. Open-Meteo history
    /// normally uses 3,600-second periods.
    #[libertas_time_interval]
    pub duration_seconds: u32,
    /// Precipitation
    /// Total precipitation, including the water equivalent of frozen
    /// precipitation, accumulated during this period in millimeters. This is a
    /// required water input to the irrigation balance.
    #[libertas_number(min = 0)]
    pub precipitation_millimeters: f32,
    /// Reference evapotranspiration
    /// FAO-56 reference evapotranspiration accumulated during this period in
    /// millimeters. This is the weather-driven water loss before applying a
    /// plant-specific crop coefficient.
    #[libertas_number(min = 0)]
    pub reference_evapotranspiration_millimeters: f32,
}

/// Sprinkler weather history V1
/// Contains recent completed hourly periods used to reconstruct and update the
/// sprinkler irrigation water balance. The last successful value is retained
/// when a later history refresh fails.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWeatherHistoryV1 {
    /// Retrieved at
    /// The date and time when the complete history section was last retrieved,
    /// validated, and accepted.
    pub retrieved_at: LibertasDateTime,
    /// Valid until
    /// The exclusive freshness deadline. The history is fresh while the current
    /// time is earlier than this value and stale at or after this value. Stale
    /// history remains available as degraded cached input.
    pub valid_until: LibertasDateTime,
    /// History periods
    /// Completed periods ordered from oldest to newest. A normal response covers
    /// the previous seven days at one-hour resolution; a shorter list is valid
    /// partial history.
    /// ----
    /// History period
    /// Precipitation and reference evapotranspiration for one completed period.
    pub periods: Vec<SprinklerWeatherHistoryPeriodV1>,
}

impl SprinklerWeatherHistoryV1 {
    /// History freshness
    /// Returns `true` when `now` is earlier than `valid_until`. Equality means
    /// the section has expired.
    pub fn is_fresh_at(&self, now: LibertasDateTime) -> bool {
        now < self.valid_until
    }
}

/// Sprinkler current weather V1
/// Contains immediate rain, freeze, wind, and water-balance inputs used to
/// decide whether an otherwise scheduled watering operation may safely start or
/// continue.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerCurrentWeatherV1 {
    /// Retrieved at
    /// The date and time when the complete current-weather section was last
    /// retrieved, validated, and accepted.
    pub retrieved_at: LibertasDateTime,
    /// Valid until
    /// The exclusive freshness deadline. Current weather is fresh while the
    /// current time is earlier than this value. At or after this value, the
    /// section remains cached but must not be treated as proof that watering is
    /// safe.
    pub valid_until: LibertasDateTime,
    /// Valid at
    /// The provider-supplied date and time represented by the current-condition
    /// values. This can differ from `retrieved_at`.
    pub valid_at: LibertasDateTime,
    /// Observation interval
    /// The backward-looking interval in seconds represented by accumulated
    /// precipitation and evapotranspiration. Open-Meteo current weather normally
    /// uses a 900-second interval.
    #[libertas_time_interval]
    pub interval_seconds: u32,
    /// Temperature
    /// Air temperature at two meters above ground in degrees Celsius. The
    /// sprinkler uses it to inhibit watering near or below freezing.
    pub temperature_celsius: f32,
    /// Precipitation
    /// Total rain, showers, and water-equivalent frozen precipitation
    /// accumulated during `interval_seconds`, in millimeters. The sprinkler uses
    /// a nonzero value to inhibit watering while precipitation is occurring.
    #[libertas_number(min = 0)]
    pub precipitation_millimeters: f32,
    /// Reference evapotranspiration
    /// FAO-56 reference evapotranspiration accumulated during
    /// `interval_seconds`, in millimeters. It can extend the water balance until
    /// the next completed historical period is available.
    #[libertas_number(min = 0)]
    pub reference_evapotranspiration_millimeters: f32,
    /// Wind speed
    /// Sustained wind speed at 10 meters above ground in meters per second. The
    /// consuming sprinkler applies its configured wind threshold.
    #[libertas_number(min = 0)]
    pub wind_speed_meters_per_second: f32,
    /// Wind gust
    /// Peak wind gust speed at 10 meters above ground in meters per second. The
    /// consuming sprinkler uses it with sustained wind to avoid spray drift.
    #[libertas_number(min = 0)]
    pub wind_gust_meters_per_second: f32,
}

impl SprinklerCurrentWeatherV1 {
    /// Current weather freshness
    /// Returns `true` when `now` is earlier than `valid_until`. Equality means
    /// the section has expired.
    pub fn is_fresh_at(&self, now: LibertasDateTime) -> bool {
        now < self.valid_until
    }
}

/// Sprinkler weather forecast period V1
/// Contains predicted water input, water loss, temperature, and wind hazards
/// used to decide when and how much to water during one planning period.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWeatherForecastPeriodV1 {
    /// Start time
    /// The inclusive date and time at which this forecast period begins.
    pub starts_at: LibertasDateTime,
    /// Duration
    /// The length of this forecast period in seconds. Open-Meteo forecasts
    /// normally use 3,600-second periods.
    #[libertas_time_interval]
    pub duration_seconds: u32,
    /// Temperature
    /// Predicted air temperature at two meters above ground in degrees Celsius.
    /// The sprinkler uses it to avoid watering during forecast freezing
    /// conditions.
    pub temperature_celsius: f32,
    /// Precipitation probability
    /// Probability of measurable precipitation during this period, expressed as
    /// an integer percentage from 0 through 100. This expresses forecast
    /// uncertainty separately from expected precipitation amount.
    #[libertas_number(min = 0, max = 100)]
    pub precipitation_probability_percent: u8,
    /// Expected precipitation
    /// Predicted total rain, showers, and water-equivalent frozen precipitation
    /// accumulated during this period in millimeters.
    #[libertas_number(min = 0)]
    pub expected_precipitation_millimeters: f32,
    /// Reference evapotranspiration
    /// Predicted FAO-56 reference evapotranspiration accumulated during this
    /// period in millimeters.
    #[libertas_number(min = 0)]
    pub reference_evapotranspiration_millimeters: f32,
    /// Wind speed
    /// Predicted sustained wind speed at 10 meters above ground in meters per
    /// second.
    #[libertas_number(min = 0)]
    pub wind_speed_meters_per_second: f32,
    /// Wind gust
    /// Predicted peak wind gust speed at 10 meters above ground in meters per
    /// second. The sprinkler uses this with sustained wind to avoid scheduling
    /// watering during likely spray drift.
    #[libertas_number(min = 0)]
    pub wind_gust_meters_per_second: f32,
}

/// Sprinkler weather forecast V1
/// Contains future hourly weather inputs used to plan sprinkler irrigation over
/// the next seven days. The last successful value is retained when a later
/// forecast refresh fails.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWeatherForecastV1 {
    /// Retrieved at
    /// The date and time when the complete forecast section was last retrieved,
    /// validated, and accepted.
    pub retrieved_at: LibertasDateTime,
    /// Valid until
    /// The exclusive freshness deadline. The forecast is fresh while the
    /// current time is earlier than this value and stale at or after this value.
    /// A stale forecast remains available as degraded cached input.
    pub valid_until: LibertasDateTime,
    /// Forecast periods
    /// Future periods ordered from earliest to latest. A normal response covers
    /// the next seven days at one-hour resolution; a shorter list is a valid
    /// partial forecast.
    /// ----
    /// Forecast period
    /// Predicted precipitation, reference evapotranspiration, temperature, and
    /// wind for one planning period.
    pub periods: Vec<SprinklerWeatherForecastPeriodV1>,
}

impl SprinklerWeatherForecastV1 {
    /// Forecast freshness
    /// Returns `true` when `now` is earlier than `valid_until`. Equality means
    /// the section has expired.
    pub fn is_fresh_at(&self, now: LibertasDateTime) -> bool {
        now < self.valid_until
    }
}

/// Sprinkler weather cursor V1
/// Identifies one applied state in an incremental sprinkler-weather stream. A
/// client compares both fields and must not interpret a smaller sequence alone
/// as a server reset.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub struct SprinklerWeatherCursorV1 {
    /// Epoch timestamp
    /// The server-assigned date and time identifying the cursor generation.
    /// This single field is both the stream epoch and an ordered timestamp; no
    /// separate opaque epoch exists. It remains unchanged during normal
    /// sequence advancement. When the server loses its transient cursor or
    /// replay journal, it assigns an epoch timestamp strictly newer than the
    /// previous one and starts `sequence` again at zero. A client can first
    /// observe a later post-reset sequence rather than zero. This timestamp
    /// describes cursor state, not the observation time of weather values.
    pub epoch_timestamp: LibertasDateTime,
    /// Sequence
    /// Identifies the latest applied state change. The server increments this
    /// value once for every `SprinklerWeatherChangeV1`. It resets this value to
    /// zero when the transient server cursor is reset, then increments it for
    /// later changes; a reset does not clear historical, current, or forecast
    /// weather data.
    pub sequence: u64,
}

impl SprinklerWeatherCursorV1 {
    /// Server cursor reset
    /// Returns `true` when this cursor has a newer epoch timestamp and a lower
    /// sequence than `previous`. The server starts a reset cursor at zero, but
    /// the client may first observe it after subsequent changes have advanced
    /// the sequence. A smaller sequence with the same or an older epoch
    /// timestamp is stale or out of order and must not roll back local state.
    pub fn is_server_reset_after(&self, previous: Self) -> bool {
        self.epoch_timestamp > previous.epoch_timestamp && self.sequence < previous.sequence
    }

    /// Valid cursor successor
    /// Returns `true` when this cursor is unchanged, advances normally with a
    /// matching epoch timestamp, or is a valid server-reset marker. This checks
    /// cursor ordering only; incremental reports must additionally prove that
    /// every intervening sequence is present.
    pub fn is_valid_successor_of(&self, previous: Self) -> bool {
        *self == previous
            || self.is_server_reset_after(previous)
            || (self.epoch_timestamp == previous.epoch_timestamp
                && self.sequence > previous.sequence)
    }
}

/// Sprinkler weather time range V1
/// Selects a half-open interval of historical or forecast periods for recovery.
/// A period is selected when its start time is at least `starts_at` and earlier
/// than `ends_before`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub struct SprinklerWeatherTimeRangeV1 {
    /// Start time
    /// The inclusive lower bound for selected period start times.
    pub starts_at: LibertasDateTime,
    /// End time
    /// The exclusive upper bound for selected period start times. This value
    /// must be later than `starts_at`.
    pub ends_before: LibertasDateTime,
}

impl SprinklerWeatherTimeRangeV1 {
    /// Valid time range
    /// Returns `true` when the exclusive upper bound is later than the inclusive
    /// lower bound.
    pub fn is_valid(&self) -> bool {
        self.starts_at < self.ends_before
    }
}

/// Sprinkler weather snapshot V1
/// Contains the last successfully accepted value of each requested weather
/// section. Missing sections have no usable cached value; stale sections remain
/// present with their original `valid_until` timestamps.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWeatherSnapshotV1 {
    /// Recent history
    /// The requested historical periods, when usable cached history exists.
    pub history: Option<SprinklerWeatherHistoryV1>,
    /// Current conditions
    /// The last accepted current conditions when requested and available.
    pub current: Option<SprinklerCurrentWeatherV1>,
    /// Forecast
    /// The requested forecast periods, when usable cached forecast data exists.
    pub forecast: Option<SprinklerWeatherForecastV1>,
}

/// Sprinkler weather section V1
/// Identifies one independently cached sprinkler-weather section.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerWeatherSectionV1 {
    /// Recent history
    /// Selects the historical water-balance section.
    History,
    /// Current conditions
    /// Selects the immediate watering-safety section.
    Current,
    /// Forecast
    /// Selects the future irrigation-planning section.
    Forecast,
}

/// Sprinkler weather change V1
/// Defines one atomic mutation in the incremental sprinkler-weather stream.
/// Variant order and field order are part of the append-only Avro wire format.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum SprinklerWeatherChangeV1 {
    /// Upsert historical periods V1
    /// Marks a successful history refresh and inserts or replaces periods by
    /// `starts_at`. An empty period list updates only retrieval and freshness
    /// metadata.
    HistoryPeriodsUpsertV1 {
        /// Retrieved at
        /// The successful retrieval and validation time for the history section.
        retrieved_at: LibertasDateTime,
        /// Valid until
        /// The new exclusive freshness deadline for the history section.
        valid_until: LibertasDateTime,
        /// Historical periods
        /// Periods to insert or replace, ordered from oldest to newest.
        /// ----
        /// Historical period
        /// One completed water-balance period keyed by `starts_at`.
        periods: Vec<SprinklerWeatherHistoryPeriodV1>,
    },
    /// Remove historical periods V1
    /// Removes cached historical periods whose start times fall within the
    /// supplied half-open range.
    HistoryPeriodsRemoveV1 {
        /// Time range
        /// The half-open range of historical period start times to remove.
        range: SprinklerWeatherTimeRangeV1,
    },
    /// Replace current conditions V1
    /// Replaces the complete current-condition section after a successful
    /// retrieval and validation.
    CurrentReplaceV1 {
        /// Current conditions
        /// The newly accepted current-condition section.
        current: SprinklerCurrentWeatherV1,
    },
    /// Upsert forecast periods V1
    /// Marks a successful forecast refresh and inserts or replaces periods by
    /// `starts_at`. An empty period list updates only retrieval and freshness
    /// metadata.
    ForecastPeriodsUpsertV1 {
        /// Retrieved at
        /// The successful retrieval and validation time for the forecast
        /// section.
        retrieved_at: LibertasDateTime,
        /// Valid until
        /// The new exclusive freshness deadline for the forecast section.
        valid_until: LibertasDateTime,
        /// Forecast periods
        /// Periods to insert or replace, ordered from earliest to latest.
        /// ----
        /// Forecast period
        /// One irrigation-planning period keyed by `starts_at`.
        periods: Vec<SprinklerWeatherForecastPeriodV1>,
    },
    /// Remove forecast periods V1
    /// Removes cached forecast periods whose start times fall within the
    /// supplied half-open range.
    ForecastPeriodsRemoveV1 {
        /// Time range
        /// The half-open range of forecast period start times to remove.
        range: SprinklerWeatherTimeRangeV1,
    },
    /// Clear weather section V1
    /// Clears one section only when its cached value is known to be invalid,
    /// such as after failed validation or an incompatible migration. A provider
    /// refresh failure by itself must not emit this change.
    SectionClearV1 {
        /// Weather section
        /// The independently cached section to clear.
        section: SprinklerWeatherSectionV1,
    },
    /// Replace history V1
    /// Replaces the complete historical section after a successful provider
    /// refresh. Periods absent from the replacement are no longer cached.
    HistoryReplaceV1 {
        /// History
        /// The complete newly accepted historical section.
        history: SprinklerWeatherHistoryV1,
    },
    /// Replace forecast V1
    /// Replaces the complete forecast section after a successful provider
    /// refresh. Periods absent from the replacement are no longer cached.
    ForecastReplaceV1 {
        /// Forecast
        /// The complete newly accepted forecast section.
        forecast: SprinklerWeatherForecastV1,
    },
}

/// Sprinkler weather incremental report V1
/// Carries an ordered, atomic range of weather changes. A client applies the
/// report only when its stored cursor equals `from_cursor`; after applying every
/// change, it stores `through_cursor`. An empty report is a heartbeat that
/// preserves the cursor and restarts the client's maximum-wait timer.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWeatherIncrementalReportV1 {
    /// From cursor
    /// The exclusive lower cursor for this report and the exact cursor a client
    /// must already hold before applying it.
    pub from_cursor: SprinklerWeatherCursorV1,
    /// Through cursor
    /// The inclusive upper cursor reached after applying every change in this
    /// report. It must retain the same epoch timestamp as `from_cursor`. A
    /// server cursor reset is never carried as an incremental report; it
    /// requires `ResetV1`.
    pub through_cursor: SprinklerWeatherCursorV1,
    /// Weather changes
    /// Ordered changes to apply atomically. Each item advances the sequence by
    /// exactly one; an empty list is a caught-up acknowledgement and subscription
    /// heartbeat that does not advance the cursor.
    /// ----
    /// Weather change
    /// One atomic state mutation in cursor order.
    pub changes: Vec<SprinklerWeatherChangeV1>,
}

impl SprinklerWeatherIncrementalReportV1 {
    /// Contiguous cursor range
    /// Returns `true` when the sequence distance equals the number of changes
    /// and both cursors have the same epoch timestamp. An empty heartbeat
    /// therefore preserves both cursor fields exactly.
    pub fn has_contiguous_cursor_range(&self) -> bool {
        let Ok(change_count) = u64::try_from(self.changes.len()) else {
            return false;
        };

        self.from_cursor.epoch_timestamp == self.through_cursor.epoch_timestamp
            && self.from_cursor.sequence.checked_add(change_count)
                == Some(self.through_cursor.sequence)
    }

    /// Applicable after cursor
    /// Returns `true` when this report has a contiguous cursor range beginning
    /// at `cursor`. A client must request recovery instead of applying a report
    /// when this method returns `false`.
    pub fn can_apply_after(&self, cursor: SprinklerWeatherCursorV1) -> bool {
        self.from_cursor == cursor && self.has_contiguous_cursor_range()
    }
}

/// Sprinkler weather reset reason V1
/// Explains why recovery returned a range-limited snapshot instead of replaying
/// incremental changes after the requested cursor.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerWeatherResetReasonV1 {
    /// Initial subscription
    /// No prior cursor was supplied, so the snapshot establishes initial state.
    InitialSubscription,
    /// Cursor expired
    /// The requested sequence is older than the retained replay journal.
    CursorExpired,
    /// Server cursor reset
    /// The server lost or deliberately discarded only its transient cursor and
    /// replay journal. Persisted weather sections remain usable, and missing
    /// historical data can be retrieved again from Open-Meteo.
    ServerCursorReset,
}

/// Sprinkler weather recovery error V1
/// Identifies a recovery request that cannot be satisfied with either replayed
/// changes or a range-limited cached snapshot.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SprinklerWeatherRecoveryErrorV1 {
    /// Invalid time range
    /// At least one requested half-open time range is empty or reversed.
    InvalidRange,
    /// Cursor ahead
    /// The supplied epoch-timestamp-and-sequence cursor cannot be reconciled
    /// with the server's current cursor or retained replay journal.
    CursorAhead,
    /// Request too large
    /// The requested recovery ranges exceed the server's bounded response
    /// capacity.
    RequestTooLarge,
    /// Temporarily unavailable
    /// Cached data or another required recovery resource is temporarily
    /// unavailable; the client may retry after the supplied delay.
    TemporarilyUnavailable,
}

/// Sprinkler weather recovery V1
/// Returns replayed changes, establishes a new cursor with a range-limited
/// snapshot, or reports a recoverable request error.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum SprinklerWeatherRecoveryV1 {
    /// Replayed changes V1
    /// Continues the requested stream by replaying every retained change after
    /// the supplied cursor. An empty report means the client is already caught
    /// up.
    ReplayedV1 {
        /// Incremental report
        /// The contiguous change range beginning at the requested cursor.
        report: SprinklerWeatherIncrementalReportV1,
    },
    /// Reset with snapshot V1
    /// Establishes a new cursor when replay is impossible or no cursor was
    /// supplied. History and forecast sections are limited to the fallback
    /// ranges requested by the client. A server cursor reset changes only
    /// transient cursor state: the returned snapshot is rebuilt from retained
    /// persistent sections and, when necessary, data retrieved from Open-Meteo.
    ResetV1 {
        /// Reset reason
        /// The reason a snapshot replaced incremental replay.
        reason: SprinklerWeatherResetReasonV1,
        /// Current cursor
        /// The cursor representing the returned snapshot. Subsequent reports
        /// begin with this value as `from_cursor`. For `ServerCursorReset`, its
        /// epoch timestamp is strictly newer and its sequence is lower than the
        /// request cursor. The sequence can be greater than zero when changes
        /// occurred after the server reset and before this response.
        cursor: SprinklerWeatherCursorV1,
        /// Weather snapshot
        /// The available cached sections constrained by the requested fallback
        /// ranges.
        snapshot: SprinklerWeatherSnapshotV1,
    },
    /// Recovery error V1
    /// Rejects the request without changing the client's cursor or local
    /// weather state.
    ErrorV1 {
        /// Error
        /// The reason recovery could not be completed.
        error: SprinklerWeatherRecoveryErrorV1,
        /// Retry delay
        /// The suggested delay in seconds before retrying. `None` means the
        /// request parameters must change before a retry can succeed.
        retry_after_seconds: Option<u32>,
    },
}

/// Sprinkler weather protocol V1
/// Defines the typed Libertas endpoint transaction for requesting or subscribing
/// to sprinkler weather. Responses expose independently available history,
/// current, and forecast sections so an outage does not hide usable cached data.
/// The Libertas endpoint status contract rejects malformed Avro and values used
/// in the wrong message role; those transport errors are not recovery variants.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum SprinklerWeatherProtocolV1 {
    /// Get sprinkler weather V1
    /// Performs a one-shot incremental read or starts or resumes an incremental
    /// subscription. The Libertas endpoint operation selects the behavior; it is
    /// not encoded in this message. The server replays retained changes after
    /// `after_cursor` when possible, or returns a range-limited cached snapshot.
    #[libertas_request]
    #[libertas_subscription_request]
    #[libertas_next_response(WeatherRecoveryV1)]
    GetWeatherV1 {
        /// Resume cursor
        /// The last cursor fully and atomically applied by the client. `None`
        /// requests an initial range-limited snapshot. The server compares both
        /// fields; a lower sequence indicates a reset only when the response
        /// cursor also has a newer epoch timestamp.
        after_cursor: Option<SprinklerWeatherCursorV1>,
        /// Historical recovery range
        /// The half-open range of historical period start times to include when
        /// replay is impossible. `None` excludes history from the reset
        /// snapshot.
        history_range: Option<SprinklerWeatherTimeRangeV1>,
        /// Include current conditions
        /// Whether a reset snapshot should include cached current conditions.
        include_current: bool,
        /// Forecast recovery range
        /// The half-open range of forecast period start times to include when
        /// replay is impossible. `None` excludes forecast data from the reset
        /// snapshot.
        forecast_range: Option<SprinklerWeatherTimeRangeV1>,
    },
    /// Sprinkler weather recovery V1
    /// Responds to `GetWeatherV1` with replayed changes, a reset snapshot, or
    /// a recoverable error. Every response supplies a maximum wait interval. A
    /// subscription client uses it after a successful replay or reset; a
    /// one-shot client ignores it. After an error, the recovery error and retry
    /// delay take precedence.
    #[libertas_response]
    WeatherRecoveryV1 {
        /// Maximum wait interval
        /// The maximum number of seconds a subscription client waits after a
        /// successful replay or reset response, or after any later incremental
        /// report. The server sends a change report or an empty heartbeat report
        /// before the interval expires. A one-shot client ignores this required
        /// field. The value must be greater than zero.
        #[libertas_time_interval]
        #[libertas_number(min = 1)]
        maximum_wait_interval_seconds: u32,
        /// Recovery
        /// The replay, reset, or error result for the resume request.
        recovery: SprinklerWeatherRecoveryV1,
    },
    /// Sprinkler weather increment V1
    /// Reports only state changes after a successful `GetWeatherV1`
    /// transaction. A cursor mismatch or non-contiguous range requires another
    /// subscription request; the client must not apply the report partially.
    /// Receipt of any report, including an empty heartbeat, restarts the
    /// maximum-wait timer supplied by `WeatherRecoveryV1`.
    #[libertas_subscription_data]
    WeatherIncrementV1 {
        /// Incremental report
        /// The ordered atomic cursor range and its weather changes.
        report: SprinklerWeatherIncrementalReportV1,
    },
}

/// Sprinkler weather location V1
/// Stores the Libertas Hub location used to obtain weather for one sprinkler
/// site. It is cached independently from provider data so the weather server can
/// continue refreshing during a temporary Hub outage.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SprinklerWeatherLocationV1 {
    /// Longitude
    /// WGS84 longitude in decimal degrees. Locations west of Greenwich use
    /// negative values.
    #[libertas_number(min = -180, max = 180)]
    pub longitude_degrees: f64,
    /// Latitude
    /// WGS84 latitude in decimal degrees.
    #[libertas_number(min = -90, max = 90)]
    pub latitude_degrees: f64,
}

/// Sprinkler weather persistent data V1
/// Defines the complete set of values that the sprinkler weather server may
/// write to the Libertas database. The consuming application links this union
/// with `#[libertas_data_schema(SprinklerWeatherPersistentDataV1)]` and stores
/// each variant under its own stable resource identifier so the location and
/// weather sections can be updated independently. Subscription cursors and
/// replay journals are intentionally absent: resetting them must not erase
/// these records.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum SprinklerWeatherPersistentDataV1 {
    /// Sprinkler site location V1
    /// Stores the last valid location reported by the Libertas Hub. The cached
    /// value lets provider refreshes continue while the Hub is temporarily
    /// unavailable.
    LocationV1 {
        /// Location
        /// The WGS84 coordinates used for provider requests.
        location: SprinklerWeatherLocationV1,
    },
    /// Recent history V1
    /// Stores the last successfully retrieved and validated recent-history
    /// section. A failed refresh leaves the existing database record unchanged.
    HistoryV1 {
        /// History
        /// Recent hourly precipitation and reference-evapotranspiration inputs,
        /// including their retrieval and freshness timestamps.
        history: SprinklerWeatherHistoryV1,
    },
    /// Current conditions V1
    /// Stores the last successfully retrieved and validated current-condition
    /// section. A failed refresh leaves the existing database record unchanged.
    CurrentV1 {
        /// Current conditions
        /// Immediate rain, freeze, wind, and water-balance inputs, including
        /// their retrieval and freshness timestamps.
        current: SprinklerCurrentWeatherV1,
    },
    /// Forecast V1
    /// Stores the last successfully retrieved and validated forecast section. A
    /// failed refresh leaves the existing database record unchanged.
    ForecastV1 {
        /// Forecast
        /// Future precipitation, reference-evapotranspiration, temperature, and
        /// wind inputs, including their retrieval and freshness timestamps.
        forecast: SprinklerWeatherForecastV1,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use libertas::AvroDecode;

    const CURSOR_TIMESTAMP: LibertasDateTime = 1_784_972_800;
    const LATER_CURSOR_TIMESTAMP: LibertasDateTime = CURSOR_TIMESTAMP + 60;

    fn history() -> SprinklerWeatherHistoryV1 {
        SprinklerWeatherHistoryV1 {
            retrieved_at: 1_784_972_800,
            valid_until: 1_784_980_000,
            periods: vec![SprinklerWeatherHistoryPeriodV1 {
                starts_at: 1_784_969_200,
                duration_seconds: 3_600,
                precipitation_millimeters: 4.2,
                reference_evapotranspiration_millimeters: 0.2,
            }],
        }
    }

    fn current() -> SprinklerCurrentWeatherV1 {
        SprinklerCurrentWeatherV1 {
            retrieved_at: 1_784_972_800,
            valid_until: 1_784_974_600,
            valid_at: 1_784_972_800,
            interval_seconds: 900,
            temperature_celsius: 22.5,
            precipitation_millimeters: 0.4,
            reference_evapotranspiration_millimeters: 0.05,
            wind_speed_meters_per_second: 3.2,
            wind_gust_meters_per_second: 5.8,
        }
    }

    fn forecast() -> SprinklerWeatherForecastV1 {
        SprinklerWeatherForecastV1 {
            retrieved_at: 1_784_972_800,
            valid_until: 1_784_983_600,
            periods: vec![SprinklerWeatherForecastPeriodV1 {
                starts_at: 1_784_972_800,
                duration_seconds: 3_600,
                temperature_celsius: 23.0,
                precipitation_probability_percent: 70,
                expected_precipitation_millimeters: 2.5,
                reference_evapotranspiration_millimeters: 0.3,
                wind_speed_meters_per_second: 4.0,
                wind_gust_meters_per_second: 7.5,
            }],
        }
    }

    fn location() -> SprinklerWeatherLocationV1 {
        SprinklerWeatherLocationV1 {
            longitude_degrees: -74.006,
            latitude_degrees: 40.7128,
        }
    }

    fn cursor(epoch_timestamp: LibertasDateTime, sequence: u64) -> SprinklerWeatherCursorV1 {
        SprinklerWeatherCursorV1 {
            epoch_timestamp,
            sequence,
        }
    }

    fn history_range() -> SprinklerWeatherTimeRangeV1 {
        SprinklerWeatherTimeRangeV1 {
            starts_at: 1_784_368_000,
            ends_before: 1_784_972_800,
        }
    }

    fn forecast_range() -> SprinklerWeatherTimeRangeV1 {
        SprinklerWeatherTimeRangeV1 {
            starts_at: 1_784_972_800,
            ends_before: 1_785_577_600,
        }
    }

    fn snapshot() -> SprinklerWeatherSnapshotV1 {
        SprinklerWeatherSnapshotV1 {
            history: Some(history()),
            current: Some(current()),
            forecast: Some(forecast()),
        }
    }

    fn incremental_report() -> SprinklerWeatherIncrementalReportV1 {
        SprinklerWeatherIncrementalReportV1 {
            from_cursor: cursor(CURSOR_TIMESTAMP, 10),
            through_cursor: cursor(CURSOR_TIMESTAMP, 12),
            changes: vec![
                SprinklerWeatherChangeV1::CurrentReplaceV1 { current: current() },
                SprinklerWeatherChangeV1::ForecastPeriodsUpsertV1 {
                    retrieved_at: forecast().retrieved_at,
                    valid_until: forecast().valid_until,
                    periods: forecast().periods,
                },
            ],
        }
    }

    fn assert_protocol_round_trip(value: SprinklerWeatherProtocolV1) {
        let encoded = value.to_avro();
        let mut offset = 0;
        let decoded = SprinklerWeatherProtocolV1::avro_decode(&encoded, &mut offset).unwrap();

        assert_eq!(decoded, value);
        assert_eq!(offset, encoded.len());
    }

    fn assert_persistent_round_trip(value: SprinklerWeatherPersistentDataV1) {
        let encoded = value.to_avro();
        let mut offset = 0;
        let decoded = SprinklerWeatherPersistentDataV1::avro_decode(&encoded, &mut offset).unwrap();

        assert_eq!(decoded, value);
        assert_eq!(offset, encoded.len());
    }

    #[test]
    fn weather_protocol_v1_round_trips_through_avro() {
        let values = [
            SprinklerWeatherProtocolV1::GetWeatherV1 {
                after_cursor: Some(cursor(CURSOR_TIMESTAMP, 10)),
                history_range: Some(history_range()),
                include_current: true,
                forecast_range: Some(forecast_range()),
            },
            SprinklerWeatherProtocolV1::WeatherRecoveryV1 {
                maximum_wait_interval_seconds: SPRINKLER_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
                recovery: SprinklerWeatherRecoveryV1::ReplayedV1 {
                    report: incremental_report(),
                },
            },
            SprinklerWeatherProtocolV1::WeatherRecoveryV1 {
                maximum_wait_interval_seconds: SPRINKLER_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
                recovery: SprinklerWeatherRecoveryV1::ResetV1 {
                    reason: SprinklerWeatherResetReasonV1::CursorExpired,
                    cursor: cursor(CURSOR_TIMESTAMP, 12),
                    snapshot: snapshot(),
                },
            },
            SprinklerWeatherProtocolV1::WeatherRecoveryV1 {
                maximum_wait_interval_seconds: SPRINKLER_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
                recovery: SprinklerWeatherRecoveryV1::ResetV1 {
                    reason: SprinklerWeatherResetReasonV1::InitialSubscription,
                    cursor: cursor(CURSOR_TIMESTAMP, 12),
                    snapshot: SprinklerWeatherSnapshotV1 {
                        history: Some(history()),
                        current: None,
                        forecast: Some(forecast()),
                    },
                },
            },
            SprinklerWeatherProtocolV1::WeatherRecoveryV1 {
                maximum_wait_interval_seconds: SPRINKLER_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
                recovery: SprinklerWeatherRecoveryV1::ErrorV1 {
                    error: SprinklerWeatherRecoveryErrorV1::TemporarilyUnavailable,
                    retry_after_seconds: Some(60),
                },
            },
            SprinklerWeatherProtocolV1::WeatherIncrementV1 {
                report: incremental_report(),
            },
        ];

        for value in values {
            assert_protocol_round_trip(value);
        }
    }

    #[test]
    fn persistent_sections_round_trip_independently() {
        let values = [
            SprinklerWeatherPersistentDataV1::HistoryV1 { history: history() },
            SprinklerWeatherPersistentDataV1::CurrentV1 { current: current() },
            SprinklerWeatherPersistentDataV1::ForecastV1 {
                forecast: forecast(),
            },
            SprinklerWeatherPersistentDataV1::LocationV1 {
                location: location(),
            },
        ];

        for value in values {
            assert_persistent_round_trip(value);
        }
    }

    #[test]
    fn union_discriminants_are_stable() {
        assert_eq!(
            SprinklerWeatherProtocolV1::GetWeatherV1 {
                after_cursor: None,
                history_range: None,
                include_current: false,
                forecast_range: None,
            }
            .to_avro()
            .first(),
            Some(&0)
        );
        assert_eq!(
            SprinklerWeatherProtocolV1::WeatherRecoveryV1 {
                maximum_wait_interval_seconds: SPRINKLER_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
                recovery: SprinklerWeatherRecoveryV1::ErrorV1 {
                    error: SprinklerWeatherRecoveryErrorV1::InvalidRange,
                    retry_after_seconds: None,
                },
            }
            .to_avro()
            .first(),
            Some(&2)
        );
        assert_eq!(
            SprinklerWeatherProtocolV1::WeatherIncrementV1 {
                report: incremental_report(),
            }
            .to_avro()
            .first(),
            Some(&4)
        );

        assert_eq!(
            SprinklerWeatherPersistentDataV1::LocationV1 {
                location: location()
            }
            .to_avro()
            .first(),
            Some(&0)
        );
        assert_eq!(
            SprinklerWeatherPersistentDataV1::HistoryV1 { history: history() }
                .to_avro()
                .first(),
            Some(&2)
        );
        assert_eq!(
            SprinklerWeatherPersistentDataV1::CurrentV1 { current: current() }
                .to_avro()
                .first(),
            Some(&4)
        );
        assert_eq!(
            SprinklerWeatherPersistentDataV1::ForecastV1 {
                forecast: forecast()
            }
            .to_avro()
            .first(),
            Some(&6)
        );
    }

    #[test]
    fn incremental_change_discriminants_are_stable() {
        let range = history_range();
        let changes = [
            SprinklerWeatherChangeV1::HistoryPeriodsUpsertV1 {
                retrieved_at: history().retrieved_at,
                valid_until: history().valid_until,
                periods: history().periods,
            },
            SprinklerWeatherChangeV1::HistoryPeriodsRemoveV1 { range },
            SprinklerWeatherChangeV1::CurrentReplaceV1 { current: current() },
            SprinklerWeatherChangeV1::ForecastPeriodsUpsertV1 {
                retrieved_at: forecast().retrieved_at,
                valid_until: forecast().valid_until,
                periods: forecast().periods,
            },
            SprinklerWeatherChangeV1::ForecastPeriodsRemoveV1 {
                range: forecast_range(),
            },
            SprinklerWeatherChangeV1::SectionClearV1 {
                section: SprinklerWeatherSectionV1::Current,
            },
            SprinklerWeatherChangeV1::HistoryReplaceV1 { history: history() },
            SprinklerWeatherChangeV1::ForecastReplaceV1 {
                forecast: forecast(),
            },
        ];

        for (index, change) in changes.iter().enumerate() {
            assert_eq!(change.to_avro().first(), Some(&((index as u8) * 2)));
        }
    }

    #[test]
    fn recovery_discriminants_are_stable() {
        let outcomes = [
            SprinklerWeatherRecoveryV1::ReplayedV1 {
                report: incremental_report(),
            },
            SprinklerWeatherRecoveryV1::ResetV1 {
                reason: SprinklerWeatherResetReasonV1::ServerCursorReset,
                cursor: cursor(LATER_CURSOR_TIMESTAMP, 3),
                snapshot: snapshot(),
            },
            SprinklerWeatherRecoveryV1::ErrorV1 {
                error: SprinklerWeatherRecoveryErrorV1::CursorAhead,
                retry_after_seconds: None,
            },
        ];

        for (index, outcome) in outcomes.iter().enumerate() {
            assert_eq!(outcome.to_avro().first(), Some(&((index as u8) * 2)));
        }
    }

    #[test]
    fn recovery_enumeration_discriminants_are_stable() {
        let sections = [
            SprinklerWeatherSectionV1::History,
            SprinklerWeatherSectionV1::Current,
            SprinklerWeatherSectionV1::Forecast,
        ];
        let reset_reasons = [
            SprinklerWeatherResetReasonV1::InitialSubscription,
            SprinklerWeatherResetReasonV1::CursorExpired,
            SprinklerWeatherResetReasonV1::ServerCursorReset,
        ];
        let recovery_errors = [
            SprinklerWeatherRecoveryErrorV1::InvalidRange,
            SprinklerWeatherRecoveryErrorV1::CursorAhead,
            SprinklerWeatherRecoveryErrorV1::RequestTooLarge,
            SprinklerWeatherRecoveryErrorV1::TemporarilyUnavailable,
        ];

        for (index, section) in sections.iter().enumerate() {
            assert_eq!(section.to_avro().first(), Some(&((index as u8) * 2)));
        }
        for (index, reason) in reset_reasons.iter().enumerate() {
            assert_eq!(reason.to_avro().first(), Some(&((index as u8) * 2)));
        }
        for (index, error) in recovery_errors.iter().enumerate() {
            assert_eq!(error.to_avro().first(), Some(&((index as u8) * 2)));
        }
    }

    #[test]
    fn incremental_reports_require_an_exact_contiguous_cursor_range() {
        let report = incremental_report();
        assert!(report.has_contiguous_cursor_range());
        assert!(report.can_apply_after(cursor(CURSOR_TIMESTAMP, 10)));
        assert!(!report.can_apply_after(cursor(CURSOR_TIMESTAMP, 9)));
        assert!(!report.can_apply_after(cursor(CURSOR_TIMESTAMP - 1, 10)));

        let sequence_gap = SprinklerWeatherIncrementalReportV1 {
            through_cursor: cursor(CURSOR_TIMESTAMP, 13),
            ..report.clone()
        };
        assert!(!sequence_gap.has_contiguous_cursor_range());

        let timestamp_regression = SprinklerWeatherIncrementalReportV1 {
            through_cursor: cursor(CURSOR_TIMESTAMP - 1, 12),
            ..report
        };
        assert!(!timestamp_regression.has_contiguous_cursor_range());
    }

    #[test]
    fn empty_incremental_report_is_a_cursor_preserving_heartbeat() {
        let report = SprinklerWeatherIncrementalReportV1 {
            from_cursor: cursor(CURSOR_TIMESTAMP, 12),
            through_cursor: cursor(CURSOR_TIMESTAMP, 12),
            changes: Vec::new(),
        };

        assert!(report.can_apply_after(cursor(CURSOR_TIMESTAMP, 12)));
        assert_eq!(report.from_cursor, report.through_cursor);

        let timestamp_only_change = SprinklerWeatherIncrementalReportV1 {
            through_cursor: cursor(LATER_CURSOR_TIMESTAMP, 12),
            ..report
        };
        assert!(!timestamp_only_change.has_contiguous_cursor_range());
    }

    #[test]
    fn incremental_sequence_overflow_is_rejected() {
        let report = SprinklerWeatherIncrementalReportV1 {
            from_cursor: cursor(CURSOR_TIMESTAMP, u64::MAX),
            through_cursor: cursor(CURSOR_TIMESTAMP, u64::MAX),
            changes: vec![SprinklerWeatherChangeV1::CurrentReplaceV1 { current: current() }],
        };

        assert!(!report.has_contiguous_cursor_range());
    }

    #[test]
    fn client_detects_a_server_reset_from_newer_timestamp_and_backward_sequence() {
        let previous = cursor(CURSOR_TIMESTAMP, 10);

        assert!(cursor(LATER_CURSOR_TIMESTAMP, 0).is_server_reset_after(previous));
        assert!(cursor(LATER_CURSOR_TIMESTAMP, 3).is_server_reset_after(previous));
        assert!(cursor(LATER_CURSOR_TIMESTAMP, 3).is_valid_successor_of(previous));

        assert!(!cursor(CURSOR_TIMESTAMP, 3).is_server_reset_after(previous));
        assert!(!cursor(CURSOR_TIMESTAMP, 3).is_valid_successor_of(previous));
        assert!(!cursor(CURSOR_TIMESTAMP - 1, 3).is_server_reset_after(previous));
        assert!(!cursor(CURSOR_TIMESTAMP - 1, 3).is_valid_successor_of(previous));
        assert!(!cursor(LATER_CURSOR_TIMESTAMP, 11).is_server_reset_after(previous));
        assert!(!cursor(LATER_CURSOR_TIMESTAMP, 11).is_valid_successor_of(previous));
        assert!(cursor(CURSOR_TIMESTAMP, 11).is_valid_successor_of(previous));
    }

    #[test]
    fn server_cursor_reset_preserves_the_weather_snapshot() {
        let before_reset = snapshot();
        let recovery = SprinklerWeatherRecoveryV1::ResetV1 {
            reason: SprinklerWeatherResetReasonV1::ServerCursorReset,
            cursor: cursor(LATER_CURSOR_TIMESTAMP, 3),
            snapshot: before_reset.clone(),
        };

        let SprinklerWeatherRecoveryV1::ResetV1 {
            reason,
            cursor: reset_cursor,
            snapshot: after_reset,
        } = recovery
        else {
            panic!("expected reset recovery");
        };

        assert_eq!(reason, SprinklerWeatherResetReasonV1::ServerCursorReset);
        assert!(reset_cursor.is_server_reset_after(cursor(CURSOR_TIMESTAMP, 10)));
        assert_eq!(after_reset, before_reset);
    }

    #[test]
    fn recovery_ranges_are_half_open_and_non_empty() {
        assert!(history_range().is_valid());
        assert!(
            !SprinklerWeatherTimeRangeV1 {
                starts_at: 100,
                ends_before: 100,
            }
            .is_valid()
        );
        assert!(
            !SprinklerWeatherTimeRangeV1 {
                starts_at: 101,
                ends_before: 100,
            }
            .is_valid()
        );
    }

    #[test]
    fn freshness_expires_at_valid_until_boundary() {
        let history = history();
        let current = current();
        let forecast = forecast();

        assert!(history.is_fresh_at(history.valid_until - 1));
        assert!(!history.is_fresh_at(history.valid_until));
        assert!(current.is_fresh_at(current.valid_until - 1));
        assert!(!current.is_fresh_at(current.valid_until));
        assert!(forecast.is_fresh_at(forecast.valid_until - 1));
        assert!(!forecast.is_fresh_at(forecast.valid_until));
    }

    #[test]
    fn refresh_and_coverage_policy_is_stable() {
        assert_eq!(SPRINKLER_CURRENT_REFRESH_INTERVAL_SECONDS, 900);
        assert_eq!(SPRINKLER_HISTORY_REFRESH_INTERVAL_SECONDS, 3_600);
        assert_eq!(SPRINKLER_FORECAST_REFRESH_INTERVAL_SECONDS, 3_600);
        assert_eq!(SPRINKLER_CURRENT_FRESHNESS_SECONDS, 1_800);
        assert_eq!(SPRINKLER_HISTORY_FRESHNESS_SECONDS, 7_200);
        assert_eq!(SPRINKLER_FORECAST_FRESHNESS_SECONDS, 10_800);
        assert_eq!(SPRINKLER_HISTORY_WINDOW_SECONDS, 604_800);
        assert_eq!(SPRINKLER_FORECAST_HORIZON_SECONDS, 604_800);
        assert_eq!(SPRINKLER_SUBSCRIPTION_REPLAY_WINDOW_SECONDS, 86_400);
        assert_eq!(SPRINKLER_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS, 1_200);
    }

    #[test]
    fn truncated_persistent_data_is_rejected() {
        let encoded = SprinklerWeatherPersistentDataV1::ForecastV1 {
            forecast: forecast(),
        }
        .to_avro();
        let mut offset = 0;

        assert!(
            SprinklerWeatherPersistentDataV1::avro_decode(
                &encoded[..encoded.len() - 1],
                &mut offset
            )
            .is_err()
        );
    }
}
