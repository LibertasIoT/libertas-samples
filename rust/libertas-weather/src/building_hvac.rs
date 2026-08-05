//! Building HVAC weather V1 definitions.
//!
//! This family serves whole-house and whole-building HVAC applications. It
//! carries the outdoor conditions needed for supervisory control and predictive
//! scheduling, but deliberately excludes indoor measurements, occupancy,
//! equipment state, energy prices, and delivered heating or cooling. Those
//! values belong to the consuming HVAC application.
//!
//! Dry-bulb temperature, dew point, relative humidity, and surface pressure are
//! transported as provider observations. Humidity ratio, moist-air enthalpy,
//! and wet-bulb temperature are derived values and are not stored here, avoiding
//! redundant values that could disagree. A local outdoor sensor remains the
//! authority for freeze and equipment protection.

use alloc::vec::Vec;
use libertas::LibertasDateTime;
use libertas_macros::{LibertasAvroDecode, LibertasAvroEncode, LibertasExport};

/// Building HVAC current-weather refresh interval
/// The default number of seconds between provider requests for current outdoor
/// HVAC conditions. Current conditions normally represent 15-minute model
/// intervals, so requesting them more often generally adds no information.
pub const BUILDING_HVAC_CURRENT_REFRESH_INTERVAL_SECONDS: u32 = 15 * 60;

/// Building HVAC history refresh interval
/// The default number of seconds between requests for recently completed
/// outdoor-condition periods used by building-state estimation.
pub const BUILDING_HVAC_HISTORY_REFRESH_INTERVAL_SECONDS: u32 = 60 * 60;

/// Building HVAC forecast refresh interval
/// The default number of seconds between requests for future outdoor conditions
/// used by preheating, precooling, thermal-storage, and load planning.
pub const BUILDING_HVAC_FORECAST_REFRESH_INTERVAL_SECONDS: u32 = 60 * 60;

/// Building HVAC outdoor-air-quality refresh interval
/// The default number of seconds between outdoor pollutant forecast requests.
/// Air quality is refreshed independently from weather so one provider failure
/// does not hide usable data from the other provider.
pub const BUILDING_HVAC_AIR_QUALITY_REFRESH_INTERVAL_SECONDS: u32 = 60 * 60;

/// Building HVAC current-weather freshness
/// The number of seconds after retrieval for which internet current conditions
/// may be used by supervisory HVAC optimization. At or beyond this age,
/// weather-dependent optimization must degrade conservatively.
pub const BUILDING_HVAC_CURRENT_FRESHNESS_SECONDS: u32 = 2 * 15 * 60;

/// Building HVAC history freshness
/// The number of seconds after retrieval for which recent outdoor-condition
/// history is considered fresh. Older history remains usable as degraded model
/// input when its age is taken into account.
pub const BUILDING_HVAC_HISTORY_FRESHNESS_SECONDS: u32 = 2 * 60 * 60;

/// Building HVAC forecast freshness
/// The number of seconds after retrieval for which forecast conditions are
/// considered fresh. Older forecasts may support degraded planning but must not
/// authorize safety-sensitive equipment operation.
pub const BUILDING_HVAC_FORECAST_FRESHNESS_SECONDS: u32 = 3 * 60 * 60;

/// Building HVAC outdoor-air-quality freshness
/// The number of seconds after retrieval for which modeled outdoor pollutant
/// data may influence ventilation optimization. Stale or missing modeled air
/// quality must never be interpreted as proof that outdoor air is safe.
pub const BUILDING_HVAC_AIR_QUALITY_FRESHNESS_SECONDS: u32 = 2 * 60 * 60;

/// Building HVAC history window
/// The requested number of seconds of recent outdoor-condition history. Three
/// days covers the recent weather needed to restore a short-term building
/// thermal-state estimate after a restart or provider outage.
pub const BUILDING_HVAC_HISTORY_WINDOW_SECONDS: u32 = 3 * 24 * 60 * 60;

/// Building HVAC forecast horizon
/// The requested number of seconds of future outdoor conditions. Three days
/// covers normal model-predictive HVAC and preconditioning decisions while
/// avoiding unrelated long-range weather.
pub const BUILDING_HVAC_FORECAST_HORIZON_SECONDS: u32 = 3 * 24 * 60 * 60;

/// Building HVAC outdoor-air-quality forecast horizon
/// The requested number of seconds of outdoor pollutant data. Two days supports
/// ventilation planning without implying more confidence than coarse air
/// quality models normally provide.
pub const BUILDING_HVAC_AIR_QUALITY_HORIZON_SECONDS: u32 = 2 * 24 * 60 * 60;

/// Building HVAC subscription replay window
/// The default number of seconds for retaining the transient incremental-change
/// journal. Older clients recover from independently persisted sections instead
/// of requiring an unbounded replay.
pub const BUILDING_HVAC_SUBSCRIPTION_REPLAY_WINDOW_SECONDS: u32 = 24 * 60 * 60;

/// Building HVAC subscription maximum wait interval
/// The default maximum number of seconds a subscribed HVAC client waits after a
/// response or data report before retrying its request with the last fully
/// applied cursor. The server reports a change or empty heartbeat first.
pub const BUILDING_HVAC_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS: u32 = 20 * 60;

/// Building HVAC precipitation kind V1
/// Classifies precipitation when its phase can affect outdoor coils, air
/// intakes, dampers, or other exposed HVAC equipment.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacPrecipitationKindV1 {
    /// No precipitation
    /// No precipitation is represented for the period.
    None,
    /// Rain
    /// Liquid rain or showers are represented for the period.
    Rain,
    /// Freezing rain
    /// Supercooled liquid precipitation that can freeze on exposed equipment is
    /// represented for the period.
    FreezingRain,
    /// Snow
    /// Frozen precipitation dominated by snow is represented for the period.
    Snow,
    /// Mixed precipitation
    /// More than one liquid or frozen precipitation phase is represented.
    Mixed,
    /// Unknown precipitation
    /// Precipitation exists, but the provider cannot classify its phase.
    Unknown,
}

/// Building HVAC outdoor conditions V1
/// Contains the physical outdoor inputs required for HVAC load prediction,
/// psychrometric calculations, economizer decisions, infiltration estimation,
/// and weather-aware operation of exposed equipment.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacOutdoorConditionsV1 {
    /// Dry-bulb temperature
    /// Outdoor air temperature at two meters above ground in degrees Celsius.
    pub dry_bulb_temperature_celsius: f32,
    /// Dew-point temperature
    /// Outdoor dew-point temperature at two meters above ground in degrees
    /// Celsius. Together with pressure, this supports humidity-ratio, enthalpy,
    /// wet-bulb, condensation-risk, and latent-load calculations.
    pub dew_point_temperature_celsius: f32,
    /// Relative humidity
    /// Outdoor relative humidity at two meters above ground as an integer
    /// percentage from 0 through 100. This provider value supports display and
    /// validation; psychrometric calculations should use one consistent input
    /// set rather than mixing disagreeing redundant values.
    #[libertas_number(min = 0, max = 100)]
    pub relative_humidity_percent: u8,
    /// Surface pressure
    /// Atmospheric pressure at the building elevation in hectopascals. Surface
    /// pressure, rather than pressure reduced to sea level, is used for
    /// psychrometric calculations.
    #[libertas_number(min = 0)]
    pub surface_pressure_hectopascals: f32,
    /// Wind speed
    /// Sustained outdoor wind speed at 10 meters above ground in meters per
    /// second. A building model may use it to estimate infiltration and
    /// pressure-driven outdoor-air loads.
    #[libertas_number(min = 0)]
    pub wind_speed_meters_per_second: f32,
    /// Wind gust
    /// Peak outdoor wind gust at 10 meters above ground in meters per second.
    /// It supports conservative control of exposed dampers and equipment.
    #[libertas_number(min = 0)]
    pub wind_gust_meters_per_second: f32,
    /// Wind direction
    /// Direction from which the wind originates in degrees clockwise from true
    /// north. Zero and 360 both represent north. Direction is useful only when
    /// the building model knows façade orientation.
    #[libertas_number(min = 0, max = 360)]
    pub wind_direction_degrees: u16,
    /// Precipitation
    /// Total liquid water equivalent accumulated over the containing interval,
    /// in millimeters.
    #[libertas_number(min = 0)]
    pub precipitation_millimeters: f32,
    /// Precipitation kind
    /// The liquid or frozen precipitation phase affecting exposed HVAC
    /// equipment during the containing interval.
    pub precipitation_kind: BuildingHvacPrecipitationKindV1,
    /// Solar elevation
    /// Geometric elevation of the sun's center in degrees above the astronomical
    /// horizon at the represented observation time. Negative values place the
    /// sun below the horizon. Use the containing current record's `valid_at` or
    /// the midpoint of a containing history or forecast period together with
    /// the building site coordinates to calculate this value.
    #[libertas_number(min = -90, max = 90)]
    pub solar_elevation_degrees: f32,
    /// Solar azimuth
    /// Direction of the sun's center in degrees clockwise from true north at
    /// the same represented observation time. HVAC machine-learning consumers
    /// convert this cyclic angle to sine and cosine instead of interpreting the
    /// raw degree value as ordinal.
    #[libertas_number(min = 0, max = 360)]
    pub solar_azimuth_degrees: f32,
    /// Global horizontal irradiance
    /// Total direct and diffuse solar power incident on a horizontal surface in
    /// watts per square meter. This is the primary whole-building solar-gain
    /// input.
    #[libertas_number(min = 0)]
    pub global_horizontal_irradiance_watts_per_square_meter: f32,
    /// Direct normal irradiance
    /// Direct solar power incident on a surface normal to the sun in watts per
    /// square meter. Oriented façade and shading models may use this value.
    #[libertas_number(min = 0)]
    pub direct_normal_irradiance_watts_per_square_meter: f32,
    /// Diffuse horizontal irradiance
    /// Diffuse-sky solar power incident on a horizontal surface in watts per
    /// square meter. Oriented façade and shading models may use this value with
    /// direct normal irradiance.
    #[libertas_number(min = 0)]
    pub diffuse_horizontal_irradiance_watts_per_square_meter: f32,
}

/// Building HVAC historical weather period V1
/// Contains the outdoor conditions for one completed period used to restore or
/// update a building thermal-state estimate.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacWeatherHistoryPeriodV1 {
    /// Start time
    /// The inclusive date and time at which this historical period begins.
    pub starts_at: LibertasDateTime,
    /// Duration
    /// The length of this completed period in seconds. History normally uses
    /// 3,600-second periods.
    #[libertas_time_interval]
    pub duration_seconds: u32,
    /// Outdoor conditions
    /// The temperature, moisture, pressure, wind, precipitation, and solar
    /// conditions represented by this completed period.
    pub conditions: BuildingHvacOutdoorConditionsV1,
}

/// Building HVAC weather history V1
/// Contains recent completed outdoor-condition periods used for short-term
/// thermal-state recovery. Long-term building learning belongs to the HVAC
/// application's own telemetry and model persistence.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacWeatherHistoryV1 {
    /// Retrieved at
    /// The date and time when this complete history section was retrieved,
    /// validated, and accepted.
    pub retrieved_at: LibertasDateTime,
    /// Valid until
    /// The exclusive freshness deadline. Older history remains available as
    /// degraded model input after this time.
    pub valid_until: LibertasDateTime,
    /// History periods
    /// Completed periods ordered from oldest to newest. A normal response covers
    /// the previous 72 hours; a shorter list is valid partial history.
    /// ----
    /// History period
    /// Outdoor HVAC conditions for one completed period.
    pub periods: Vec<BuildingHvacWeatherHistoryPeriodV1>,
}

impl BuildingHvacWeatherHistoryV1 {
    /// History freshness
    /// Returns `true` when `now` is earlier than `valid_until`. Equality means
    /// the section has expired.
    pub fn is_fresh_at(&self, now: LibertasDateTime) -> bool {
        now < self.valid_until
    }
}

/// Building HVAC current weather V1
/// Contains the latest modeled outdoor conditions used by supervisory HVAC
/// control. It does not replace a local outdoor sensor used for equipment or
/// freeze protection.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacCurrentWeatherV1 {
    /// Retrieved at
    /// The date and time when this complete current-condition section was
    /// retrieved, validated, and accepted.
    pub retrieved_at: LibertasDateTime,
    /// Valid until
    /// The exclusive freshness deadline. At or after this time, the cached
    /// values must not authorize economizer operation or other weather-dependent
    /// optimizations that require known current conditions.
    pub valid_until: LibertasDateTime,
    /// Valid at
    /// The provider-supplied date and time represented by these current
    /// conditions.
    pub valid_at: LibertasDateTime,
    /// Observation interval
    /// The backward-looking interval in seconds represented by accumulated
    /// precipitation. Current weather normally uses a 900-second interval.
    #[libertas_time_interval]
    pub interval_seconds: u32,
    /// Outdoor conditions
    /// The latest modeled temperature, moisture, pressure, wind, precipitation,
    /// and solar inputs for supervisory HVAC control.
    pub conditions: BuildingHvacOutdoorConditionsV1,
}

impl BuildingHvacCurrentWeatherV1 {
    /// Current-weather freshness
    /// Returns `true` when `now` is earlier than `valid_until`. Equality means
    /// the section has expired.
    pub fn is_fresh_at(&self, now: LibertasDateTime) -> bool {
        now < self.valid_until
    }
}

/// Building HVAC forecast period V1
/// Contains predicted outdoor conditions for one model-predictive HVAC planning
/// period.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacWeatherForecastPeriodV1 {
    /// Start time
    /// The inclusive date and time at which this forecast period begins.
    pub starts_at: LibertasDateTime,
    /// Duration
    /// The length of this forecast period in seconds. Providers may supply
    /// 900-second periods near the present and 3,600-second periods later.
    #[libertas_time_interval]
    pub duration_seconds: u32,
    /// Precipitation probability
    /// Probability of measurable precipitation during this period as an integer
    /// percentage from 0 through 100.
    #[libertas_number(min = 0, max = 100)]
    pub precipitation_probability_percent: u8,
    /// Outdoor conditions
    /// Predicted temperature, moisture, pressure, wind, precipitation, and solar
    /// inputs for the period.
    pub conditions: BuildingHvacOutdoorConditionsV1,
}

/// Building HVAC weather forecast V1
/// Contains future outdoor conditions for load prediction, preheating,
/// precooling, thermal storage, and weather-aware ventilation planning.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacWeatherForecastV1 {
    /// Retrieved at
    /// The date and time when this complete forecast section was retrieved,
    /// validated, and accepted.
    pub retrieved_at: LibertasDateTime,
    /// Valid until
    /// The exclusive freshness deadline. Older forecasts remain available for
    /// degraded planning but must not authorize safety-sensitive operation.
    pub valid_until: LibertasDateTime,
    /// Forecast periods
    /// Future periods ordered from earliest to latest. A normal response covers
    /// 72 hours, optionally using 15-minute periods for the first six hours and
    /// hourly periods afterward.
    /// ----
    /// Forecast period
    /// Predicted outdoor HVAC conditions for one planning period.
    pub periods: Vec<BuildingHvacWeatherForecastPeriodV1>,
}

impl BuildingHvacWeatherForecastV1 {
    /// Forecast freshness
    /// Returns `true` when `now` is earlier than `valid_until`. Equality means
    /// the section has expired.
    pub fn is_fresh_at(&self, now: LibertasDateTime) -> bool {
        now < self.valid_until
    }
}

/// Building HVAC outdoor-air-quality period V1
/// Contains modeled outdoor pollutant concentrations used to avoid increasing
/// mechanical outdoor-air intake during unhealthy conditions. These regional
/// model values do not replace local safety sensors or official emergency
/// instructions.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacOutdoorAirQualityPeriodV1 {
    /// Start time
    /// The inclusive date and time represented by this air-quality period.
    pub starts_at: LibertasDateTime,
    /// Duration
    /// The length of this period in seconds. Modeled outdoor air quality
    /// normally uses 3,600-second periods.
    #[libertas_time_interval]
    pub duration_seconds: u32,
    /// Fine particulate matter
    /// Modeled outdoor PM2.5 concentration in micrograms per cubic meter.
    #[libertas_number(min = 0)]
    pub particulate_matter_2_5_micrograms_per_cubic_meter: f32,
    /// Particulate matter
    /// Modeled outdoor PM10 concentration in micrograms per cubic meter.
    #[libertas_number(min = 0)]
    pub particulate_matter_10_micrograms_per_cubic_meter: f32,
    /// Ozone
    /// Modeled outdoor ozone concentration in micrograms per cubic meter.
    #[libertas_number(min = 0)]
    pub ozone_micrograms_per_cubic_meter: f32,
    /// Nitrogen dioxide
    /// Modeled outdoor nitrogen-dioxide concentration in micrograms per cubic
    /// meter.
    #[libertas_number(min = 0)]
    pub nitrogen_dioxide_micrograms_per_cubic_meter: f32,
}

/// Building HVAC outdoor air quality V1
/// Contains current and forecast modeled outdoor pollutant periods for
/// ventilation planning. It is independently optional and independently cached
/// from physical weather.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacOutdoorAirQualityV1 {
    /// Retrieved at
    /// The date and time when this complete air-quality section was retrieved,
    /// validated, and accepted.
    pub retrieved_at: LibertasDateTime,
    /// Valid until
    /// The exclusive freshness deadline. Stale modeled values must not be
    /// interpreted as proof that outdoor air is safe.
    pub valid_until: LibertasDateTime,
    /// Air-quality periods
    /// Current and future periods ordered from earliest to latest. A normal
    /// response covers no more than 48 hours.
    /// ----
    /// Air-quality period
    /// Modeled outdoor pollutant concentrations for one period.
    pub periods: Vec<BuildingHvacOutdoorAirQualityPeriodV1>,
}

impl BuildingHvacOutdoorAirQualityV1 {
    /// Outdoor-air-quality freshness
    /// Returns `true` when `now` is earlier than `valid_until`. Equality means
    /// the section has expired.
    pub fn is_fresh_at(&self, now: LibertasDateTime) -> bool {
        now < self.valid_until
    }
}

/// Building HVAC weather cursor V1
/// Identifies one fully applied state in an incremental building-HVAC weather
/// stream. Clients compare the epoch timestamp and sequence together.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub struct BuildingHvacWeatherCursorV1 {
    /// Epoch timestamp
    /// The server-assigned date and time identifying the stream generation. It
    /// remains constant during normal sequence advancement. A server that loses
    /// transient cursor state assigns a strictly newer timestamp.
    pub epoch_timestamp: LibertasDateTime,
    /// Sequence
    /// The sequence of the latest applied atomic change. A server cursor reset
    /// starts it again at zero, although a client can first observe a later
    /// post-reset sequence.
    pub sequence: u64,
}

impl BuildingHvacWeatherCursorV1 {
    /// Server cursor reset
    /// Returns `true` when this cursor has a newer epoch timestamp and a lower
    /// sequence than `previous`. A backward sequence without a newer timestamp
    /// is stale or out of order.
    pub fn is_server_reset_after(&self, previous: Self) -> bool {
        self.epoch_timestamp > previous.epoch_timestamp && self.sequence < previous.sequence
    }

    /// Valid cursor successor
    /// Returns `true` when the cursor is unchanged, advances within the same
    /// epoch, or identifies a server reset. Incremental reports must separately
    /// prove that every intervening sequence is present.
    pub fn is_valid_successor_of(&self, previous: Self) -> bool {
        *self == previous
            || self.is_server_reset_after(previous)
            || (self.epoch_timestamp == previous.epoch_timestamp
                && self.sequence > previous.sequence)
    }
}

/// Building HVAC weather time range V1
/// Selects a half-open interval of history, forecast, or outdoor-air-quality
/// periods by their start times.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub struct BuildingHvacWeatherTimeRangeV1 {
    /// Start time
    /// The inclusive lower bound for selected period start times.
    pub starts_at: LibertasDateTime,
    /// End time
    /// The exclusive upper bound for selected period start times. It must be
    /// later than `starts_at`.
    pub ends_before: LibertasDateTime,
}

impl BuildingHvacWeatherTimeRangeV1 {
    /// Valid time range
    /// Returns `true` when the exclusive upper bound is later than the inclusive
    /// lower bound.
    pub fn is_valid(&self) -> bool {
        self.starts_at < self.ends_before
    }
}

/// Building HVAC weather snapshot V1
/// Contains the last successfully accepted value of each requested section.
/// Missing sections have no usable cache; stale sections remain present with
/// their original freshness deadlines.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacWeatherSnapshotV1 {
    /// Recent history
    /// The requested recent outdoor-condition periods when usable cached history
    /// exists.
    pub history: Option<BuildingHvacWeatherHistoryV1>,
    /// Current conditions
    /// The last accepted current outdoor conditions when requested and
    /// available.
    pub current: Option<BuildingHvacCurrentWeatherV1>,
    /// Weather forecast
    /// The requested future outdoor-condition periods when usable cached
    /// forecast data exists.
    pub forecast: Option<BuildingHvacWeatherForecastV1>,
    /// Outdoor air quality
    /// The requested current and forecast pollutant periods when usable modeled
    /// air-quality data exists.
    pub outdoor_air_quality: Option<BuildingHvacOutdoorAirQualityV1>,
}

/// Building HVAC weather section V1
/// Identifies one independently cached building-HVAC weather section.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacWeatherSectionV1 {
    /// Recent history
    /// Selects recent physical outdoor-condition history.
    History,
    /// Current conditions
    /// Selects modeled current physical outdoor conditions.
    Current,
    /// Weather forecast
    /// Selects the future physical outdoor-condition forecast.
    Forecast,
    /// Outdoor air quality
    /// Selects modeled current and forecast outdoor pollutant concentrations.
    OutdoorAirQuality,
}

/// Building HVAC weather change V1
/// Defines one atomic mutation in the incremental building-HVAC weather stream.
/// Variant and field order are part of the Avro wire contract.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum BuildingHvacWeatherChangeV1 {
    /// Upsert historical periods V1
    /// Inserts or replaces completed periods by `starts_at` after a successful
    /// history refresh.
    HistoryPeriodsUpsertV1 {
        /// Retrieved at
        /// The retrieval and validation time for the updated history section.
        retrieved_at: LibertasDateTime,
        /// Valid until
        /// The new exclusive freshness deadline for the history section.
        valid_until: LibertasDateTime,
        /// Historical periods
        /// Periods to insert or replace, ordered from oldest to newest.
        /// ----
        /// Historical period
        /// One completed outdoor-condition period keyed by `starts_at`.
        periods: Vec<BuildingHvacWeatherHistoryPeriodV1>,
    },
    /// Remove historical periods V1
    /// Removes historical periods whose start times fall within the supplied
    /// half-open range.
    HistoryPeriodsRemoveV1 {
        /// Time range
        /// The half-open range of historical period start times to remove.
        range: BuildingHvacWeatherTimeRangeV1,
    },
    /// Replace current conditions V1
    /// Replaces the complete current-condition section after successful
    /// retrieval, validation, and persistence.
    CurrentReplaceV1 {
        /// Current conditions
        /// The complete newly accepted current-condition section.
        current: BuildingHvacCurrentWeatherV1,
    },
    /// Upsert forecast periods V1
    /// Inserts or replaces future periods by `starts_at` after a successful
    /// forecast refresh.
    ForecastPeriodsUpsertV1 {
        /// Retrieved at
        /// The retrieval and validation time for the updated forecast section.
        retrieved_at: LibertasDateTime,
        /// Valid until
        /// The new exclusive freshness deadline for the forecast section.
        valid_until: LibertasDateTime,
        /// Forecast periods
        /// Periods to insert or replace, ordered from earliest to latest.
        /// ----
        /// Forecast period
        /// One future outdoor-condition period keyed by `starts_at`.
        periods: Vec<BuildingHvacWeatherForecastPeriodV1>,
    },
    /// Remove forecast periods V1
    /// Removes forecast periods whose start times fall within the supplied
    /// half-open range.
    ForecastPeriodsRemoveV1 {
        /// Time range
        /// The half-open range of forecast period start times to remove.
        range: BuildingHvacWeatherTimeRangeV1,
    },
    /// Upsert outdoor-air-quality periods V1
    /// Inserts or replaces pollutant periods by `starts_at` after a successful
    /// air-quality refresh.
    OutdoorAirQualityPeriodsUpsertV1 {
        /// Retrieved at
        /// The retrieval and validation time for the updated air-quality
        /// section.
        retrieved_at: LibertasDateTime,
        /// Valid until
        /// The new exclusive freshness deadline for the air-quality section.
        valid_until: LibertasDateTime,
        /// Air-quality periods
        /// Periods to insert or replace, ordered from earliest to latest.
        /// ----
        /// Air-quality period
        /// One modeled pollutant period keyed by `starts_at`.
        periods: Vec<BuildingHvacOutdoorAirQualityPeriodV1>,
    },
    /// Remove outdoor-air-quality periods V1
    /// Removes pollutant periods whose start times fall within the supplied
    /// half-open range.
    OutdoorAirQualityPeriodsRemoveV1 {
        /// Time range
        /// The half-open range of air-quality period start times to remove.
        range: BuildingHvacWeatherTimeRangeV1,
    },
    /// Clear weather section V1
    /// Clears one section only after its cached value is proven invalid. A
    /// provider, internet, or refresh failure alone must not clear a section.
    SectionClearV1 {
        /// Weather section
        /// The independently cached section to clear.
        section: BuildingHvacWeatherSectionV1,
    },
    /// Replace history V1
    /// Replaces the complete historical section after a successful refresh.
    HistoryReplaceV1 {
        /// History
        /// The complete newly accepted history section.
        history: BuildingHvacWeatherHistoryV1,
    },
    /// Replace forecast V1
    /// Replaces the complete forecast section after a successful refresh.
    ForecastReplaceV1 {
        /// Forecast
        /// The complete newly accepted forecast section.
        forecast: BuildingHvacWeatherForecastV1,
    },
    /// Replace outdoor air quality V1
    /// Replaces the complete modeled pollutant section after a successful
    /// refresh.
    OutdoorAirQualityReplaceV1 {
        /// Outdoor air quality
        /// The complete newly accepted outdoor-air-quality section.
        outdoor_air_quality: BuildingHvacOutdoorAirQualityV1,
    },
}

/// Building HVAC weather incremental report V1
/// Carries an ordered, atomic range of building-HVAC weather changes. A client
/// applies it only when `from_cursor` exactly matches the last fully applied
/// cursor. An empty report is a cursor-preserving heartbeat.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacWeatherIncrementalReportV1 {
    /// From cursor
    /// The exclusive lower cursor and exact client state required before
    /// applying the report.
    pub from_cursor: BuildingHvacWeatherCursorV1,
    /// Through cursor
    /// The inclusive cursor reached after atomically applying every change.
    pub through_cursor: BuildingHvacWeatherCursorV1,
    /// Weather changes
    /// Ordered changes in cursor sequence. Each change advances the sequence by
    /// exactly one; an empty list preserves the cursor.
    /// ----
    /// Weather change
    /// One atomic state mutation in cursor order.
    pub changes: Vec<BuildingHvacWeatherChangeV1>,
}

impl BuildingHvacWeatherIncrementalReportV1 {
    /// Contiguous cursor range
    /// Returns `true` when both cursors use the same epoch timestamp and the
    /// sequence distance equals the number of changes.
    pub fn has_contiguous_cursor_range(&self) -> bool {
        let Ok(change_count) = u64::try_from(self.changes.len()) else {
            return false;
        };

        self.from_cursor.epoch_timestamp == self.through_cursor.epoch_timestamp
            && self.from_cursor.sequence.checked_add(change_count)
                == Some(self.through_cursor.sequence)
    }

    /// Applicable after cursor
    /// Returns `true` when the report begins at `cursor` and contains one exact
    /// contiguous range. Otherwise the client requests recovery.
    pub fn can_apply_after(&self, cursor: BuildingHvacWeatherCursorV1) -> bool {
        self.from_cursor == cursor && self.has_contiguous_cursor_range()
    }
}

/// Building HVAC weather reset reason V1
/// Explains why recovery established state from a range-limited snapshot instead
/// of replaying incremental changes.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacWeatherResetReasonV1 {
    /// Initial subscription
    /// No previous cursor was supplied.
    InitialSubscription,
    /// Cursor expired
    /// The requested sequence is older than the retained replay journal.
    CursorExpired,
    /// Server cursor reset
    /// The server lost or deliberately discarded only its transient cursor and
    /// replay journal; independently persisted weather sections remain usable.
    ServerCursorReset,
}

/// Building HVAC weather recovery error V1
/// Identifies a recovery request that cannot be satisfied by replay or a
/// range-limited cached snapshot.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BuildingHvacWeatherRecoveryErrorV1 {
    /// Invalid time range
    /// At least one requested half-open range is empty or reversed.
    InvalidRange,
    /// Cursor ahead
    /// The supplied cursor cannot be reconciled with current cursor state or the
    /// retained journal.
    CursorAhead,
    /// Request too large
    /// At least one requested recovery range exceeds bounded response capacity.
    RequestTooLarge,
    /// Temporarily unavailable
    /// A required cached record or recovery resource is temporarily
    /// unavailable.
    TemporarilyUnavailable,
}

/// Building HVAC weather recovery V1
/// Replays retained changes, establishes a new cursor and snapshot, or reports a
/// recoverable request error.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum BuildingHvacWeatherRecoveryV1 {
    /// Replayed changes V1
    /// Continues the stream with every retained change after the supplied
    /// cursor. An empty report means the client is caught up.
    ReplayedV1 {
        /// Incremental report
        /// The contiguous retained change range beginning at the request cursor.
        report: BuildingHvacWeatherIncrementalReportV1,
    },
    /// Reset with snapshot V1
    /// Establishes state from independently cached sections when replay is
    /// impossible or no cursor was supplied.
    ResetV1 {
        /// Reset reason
        /// The reason the response contains a snapshot rather than replay.
        reason: BuildingHvacWeatherResetReasonV1,
        /// Current cursor
        /// The cursor representing the returned snapshot.
        cursor: BuildingHvacWeatherCursorV1,
        /// Weather snapshot
        /// The available cached sections constrained by requested ranges.
        snapshot: BuildingHvacWeatherSnapshotV1,
    },
    /// Recovery error V1
    /// Rejects the request without changing client cursor or cached weather.
    ErrorV1 {
        /// Error
        /// The reason recovery could not be completed.
        error: BuildingHvacWeatherRecoveryErrorV1,
        /// Retry delay
        /// Suggested seconds before retrying. `None` means the request must
        /// change before a retry can succeed.
        retry_after_seconds: Option<u32>,
    },
}

/// Building HVAC weather protocol V1
/// Defines one-shot and subscription transactions for whole-house or
/// whole-building HVAC weather. The Libertas endpoint operation, not a field in
/// this arbitrary message contract, selects one-shot or subscription behavior.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum BuildingHvacWeatherProtocolV1 {
    /// Get building HVAC weather V1
    /// Performs a one-shot incremental read or starts or resumes a subscription.
    /// The server replays retained changes after `after_cursor` when possible or
    /// returns independently cached sections constrained to the requested
    /// fallback ranges.
    #[libertas_request]
    #[libertas_subscription_request]
    #[libertas_next_response(BuildingHvacWeatherRecoveryV1)]
    GetBuildingHvacWeatherV1 {
        /// Resume cursor
        /// The last cursor fully and atomically applied by the client. `None`
        /// requests an initial snapshot.
        after_cursor: Option<BuildingHvacWeatherCursorV1>,
        /// Historical recovery range
        /// The half-open historical range to include when replay is impossible.
        /// `None` excludes history from a reset snapshot.
        history_range: Option<BuildingHvacWeatherTimeRangeV1>,
        /// Include current conditions
        /// Whether a reset snapshot should include cached current outdoor
        /// conditions.
        include_current: bool,
        /// Forecast recovery range
        /// The half-open physical-weather forecast range to include when replay
        /// is impossible. `None` excludes forecast data.
        forecast_range: Option<BuildingHvacWeatherTimeRangeV1>,
        /// Outdoor-air-quality recovery range
        /// The half-open pollutant period range to include when replay is
        /// impossible. `None` excludes modeled outdoor air quality.
        outdoor_air_quality_range: Option<BuildingHvacWeatherTimeRangeV1>,
    },
    /// Building HVAC weather recovery V1
    /// Responds to every valid get request with replay, a reset snapshot, or a
    /// typed error. Subscription clients restart their timeout after successful
    /// recovery and every later data report.
    #[libertas_response]
    BuildingHvacWeatherRecoveryV1 {
        /// Maximum wait interval
        /// Maximum seconds a subscription client waits before retrying with its
        /// last fully applied cursor. The server sends a change or empty
        /// heartbeat first. A one-shot client ignores this required nonzero
        /// value.
        #[libertas_time_interval]
        #[libertas_number(min = 1)]
        maximum_wait_interval_seconds: u32,
        /// Recovery
        /// The replay, reset snapshot, or typed error result.
        recovery: BuildingHvacWeatherRecoveryV1,
    },
    /// Building HVAC weather increment V1
    /// Reports changes following successful subscription recovery. A client
    /// applies the complete contiguous range or applies none of it.
    #[libertas_subscription_data]
    BuildingHvacWeatherIncrementV1 {
        /// Incremental report
        /// The ordered, atomic cursor range and its weather changes.
        report: BuildingHvacWeatherIncrementalReportV1,
    },
}

/// Building HVAC weather location V1
/// Stores the Libertas Hub location used to retrieve weather for one whole
/// house or building.
#[derive(Clone, Copy, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BuildingHvacWeatherLocationV1 {
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

/// Building HVAC weather persistent data V1
/// Defines every value a building-HVAC weather server may write to the Libertas
/// database. Each variant is stored under its own stable resource identifier so
/// partial provider failure cannot erase another section. Subscription cursors,
/// journals, peers, and heartbeat deadlines are deliberately not persistent.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum BuildingHvacWeatherPersistentDataV1 {
    /// Building location V1
    /// Stores the last valid site coordinates received from the Libertas Hub.
    LocationV1 {
        /// Location
        /// The WGS84 site coordinates used for provider requests.
        location: BuildingHvacWeatherLocationV1,
    },
    /// Recent history V1
    /// Stores the last successfully accepted physical-weather history.
    HistoryV1 {
        /// History
        /// Recent outdoor-condition periods with retrieval and freshness
        /// timestamps.
        history: BuildingHvacWeatherHistoryV1,
    },
    /// Current conditions V1
    /// Stores the last successfully accepted current physical weather.
    CurrentV1 {
        /// Current conditions
        /// Current outdoor HVAC inputs with validity and retrieval timestamps.
        current: BuildingHvacCurrentWeatherV1,
    },
    /// Forecast V1
    /// Stores the last successfully accepted physical-weather forecast.
    ForecastV1 {
        /// Forecast
        /// Future outdoor HVAC inputs with retrieval and freshness timestamps.
        forecast: BuildingHvacWeatherForecastV1,
    },
    /// Outdoor air quality V1
    /// Stores the last successfully accepted modeled pollutant data separately
    /// from physical weather.
    OutdoorAirQualityV1 {
        /// Outdoor air quality
        /// Current and future outdoor pollutant periods with retrieval and
        /// freshness timestamps.
        outdoor_air_quality: BuildingHvacOutdoorAirQualityV1,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use libertas::AvroDecode;

    const CURSOR_TIMESTAMP: LibertasDateTime = 1_785_059_200;
    const LATER_CURSOR_TIMESTAMP: LibertasDateTime = CURSOR_TIMESTAMP + 60;

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

    fn conditions() -> BuildingHvacOutdoorConditionsV1 {
        BuildingHvacOutdoorConditionsV1 {
            dry_bulb_temperature_celsius: 31.5,
            dew_point_temperature_celsius: 19.0,
            relative_humidity_percent: 47,
            surface_pressure_hectopascals: 1002.4,
            wind_speed_meters_per_second: 3.5,
            wind_gust_meters_per_second: 7.1,
            wind_direction_degrees: 225,
            precipitation_millimeters: 0.8,
            precipitation_kind: BuildingHvacPrecipitationKindV1::Rain,
            solar_elevation_degrees: 42.0,
            solar_azimuth_degrees: 210.0,
            global_horizontal_irradiance_watts_per_square_meter: 620.0,
            direct_normal_irradiance_watts_per_square_meter: 710.0,
            diffuse_horizontal_irradiance_watts_per_square_meter: 140.0,
        }
    }

    fn history_period() -> BuildingHvacWeatherHistoryPeriodV1 {
        BuildingHvacWeatherHistoryPeriodV1 {
            starts_at: 1_785_055_600,
            duration_seconds: 3_600,
            conditions: conditions(),
        }
    }

    fn history() -> BuildingHvacWeatherHistoryV1 {
        BuildingHvacWeatherHistoryV1 {
            retrieved_at: CURSOR_TIMESTAMP,
            valid_until: CURSOR_TIMESTAMP + BUILDING_HVAC_HISTORY_FRESHNESS_SECONDS as u64,
            periods: vec![history_period()],
        }
    }

    fn current() -> BuildingHvacCurrentWeatherV1 {
        BuildingHvacCurrentWeatherV1 {
            retrieved_at: CURSOR_TIMESTAMP,
            valid_until: CURSOR_TIMESTAMP + BUILDING_HVAC_CURRENT_FRESHNESS_SECONDS as u64,
            valid_at: CURSOR_TIMESTAMP,
            interval_seconds: 900,
            conditions: conditions(),
        }
    }

    fn forecast_period() -> BuildingHvacWeatherForecastPeriodV1 {
        BuildingHvacWeatherForecastPeriodV1 {
            starts_at: CURSOR_TIMESTAMP,
            duration_seconds: 3_600,
            precipitation_probability_percent: 65,
            conditions: conditions(),
        }
    }

    fn forecast() -> BuildingHvacWeatherForecastV1 {
        BuildingHvacWeatherForecastV1 {
            retrieved_at: CURSOR_TIMESTAMP,
            valid_until: CURSOR_TIMESTAMP + BUILDING_HVAC_FORECAST_FRESHNESS_SECONDS as u64,
            periods: vec![forecast_period()],
        }
    }

    fn air_quality_period() -> BuildingHvacOutdoorAirQualityPeriodV1 {
        BuildingHvacOutdoorAirQualityPeriodV1 {
            starts_at: CURSOR_TIMESTAMP,
            duration_seconds: 3_600,
            particulate_matter_2_5_micrograms_per_cubic_meter: 12.5,
            particulate_matter_10_micrograms_per_cubic_meter: 21.0,
            ozone_micrograms_per_cubic_meter: 72.0,
            nitrogen_dioxide_micrograms_per_cubic_meter: 18.0,
        }
    }

    fn air_quality() -> BuildingHvacOutdoorAirQualityV1 {
        BuildingHvacOutdoorAirQualityV1 {
            retrieved_at: CURSOR_TIMESTAMP,
            valid_until: CURSOR_TIMESTAMP + BUILDING_HVAC_AIR_QUALITY_FRESHNESS_SECONDS as u64,
            periods: vec![air_quality_period()],
        }
    }

    fn cursor(epoch_timestamp: LibertasDateTime, sequence: u64) -> BuildingHvacWeatherCursorV1 {
        BuildingHvacWeatherCursorV1 {
            epoch_timestamp,
            sequence,
        }
    }

    fn history_range() -> BuildingHvacWeatherTimeRangeV1 {
        BuildingHvacWeatherTimeRangeV1 {
            starts_at: CURSOR_TIMESTAMP - BUILDING_HVAC_HISTORY_WINDOW_SECONDS as u64,
            ends_before: CURSOR_TIMESTAMP,
        }
    }

    fn forecast_range() -> BuildingHvacWeatherTimeRangeV1 {
        BuildingHvacWeatherTimeRangeV1 {
            starts_at: CURSOR_TIMESTAMP,
            ends_before: CURSOR_TIMESTAMP + BUILDING_HVAC_FORECAST_HORIZON_SECONDS as u64,
        }
    }

    fn air_quality_range() -> BuildingHvacWeatherTimeRangeV1 {
        BuildingHvacWeatherTimeRangeV1 {
            starts_at: CURSOR_TIMESTAMP,
            ends_before: CURSOR_TIMESTAMP + BUILDING_HVAC_AIR_QUALITY_HORIZON_SECONDS as u64,
        }
    }

    fn snapshot() -> BuildingHvacWeatherSnapshotV1 {
        BuildingHvacWeatherSnapshotV1 {
            history: Some(history()),
            current: Some(current()),
            forecast: Some(forecast()),
            outdoor_air_quality: Some(air_quality()),
        }
    }

    fn incremental_report() -> BuildingHvacWeatherIncrementalReportV1 {
        BuildingHvacWeatherIncrementalReportV1 {
            from_cursor: cursor(CURSOR_TIMESTAMP, 20),
            through_cursor: cursor(CURSOR_TIMESTAMP, 22),
            changes: vec![
                BuildingHvacWeatherChangeV1::CurrentReplaceV1 { current: current() },
                BuildingHvacWeatherChangeV1::OutdoorAirQualityPeriodsUpsertV1 {
                    retrieved_at: air_quality().retrieved_at,
                    valid_until: air_quality().valid_until,
                    periods: air_quality().periods,
                },
            ],
        }
    }

    #[test]
    fn every_public_data_shape_round_trips_through_avro() {
        assert_round_trip!(
            BuildingHvacPrecipitationKindV1,
            BuildingHvacPrecipitationKindV1::FreezingRain
        );
        assert_round_trip!(BuildingHvacOutdoorConditionsV1, conditions());
        assert_round_trip!(BuildingHvacWeatherHistoryPeriodV1, history_period());
        assert_round_trip!(BuildingHvacWeatherHistoryV1, history());
        assert_round_trip!(BuildingHvacCurrentWeatherV1, current());
        assert_round_trip!(BuildingHvacWeatherForecastPeriodV1, forecast_period());
        assert_round_trip!(BuildingHvacWeatherForecastV1, forecast());
        assert_round_trip!(BuildingHvacOutdoorAirQualityPeriodV1, air_quality_period());
        assert_round_trip!(BuildingHvacOutdoorAirQualityV1, air_quality());
        assert_round_trip!(BuildingHvacWeatherCursorV1, cursor(CURSOR_TIMESTAMP, 20));
        assert_round_trip!(BuildingHvacWeatherTimeRangeV1, history_range());
        assert_round_trip!(BuildingHvacWeatherSnapshotV1, snapshot());
        assert_round_trip!(
            BuildingHvacWeatherSectionV1,
            BuildingHvacWeatherSectionV1::OutdoorAirQuality
        );
        assert_round_trip!(
            BuildingHvacWeatherChangeV1,
            BuildingHvacWeatherChangeV1::CurrentReplaceV1 { current: current() }
        );
        assert_round_trip!(BuildingHvacWeatherIncrementalReportV1, incremental_report());
        assert_round_trip!(
            BuildingHvacWeatherResetReasonV1,
            BuildingHvacWeatherResetReasonV1::ServerCursorReset
        );
        assert_round_trip!(
            BuildingHvacWeatherRecoveryErrorV1,
            BuildingHvacWeatherRecoveryErrorV1::TemporarilyUnavailable
        );
        assert_round_trip!(
            BuildingHvacWeatherRecoveryV1,
            BuildingHvacWeatherRecoveryV1::ResetV1 {
                reason: BuildingHvacWeatherResetReasonV1::InitialSubscription,
                cursor: cursor(CURSOR_TIMESTAMP, 22),
                snapshot: snapshot()
            }
        );
        assert_round_trip!(
            BuildingHvacWeatherProtocolV1,
            BuildingHvacWeatherProtocolV1::BuildingHvacWeatherIncrementV1 {
                report: incremental_report()
            }
        );
        assert_round_trip!(
            BuildingHvacWeatherLocationV1,
            BuildingHvacWeatherLocationV1 {
                longitude_degrees: -74.006,
                latitude_degrees: 40.7128
            }
        );
        assert_round_trip!(
            BuildingHvacWeatherPersistentDataV1,
            BuildingHvacWeatherPersistentDataV1::OutdoorAirQualityV1 {
                outdoor_air_quality: air_quality()
            }
        );
    }

    #[test]
    fn all_protocol_transactions_round_trip_through_avro() {
        let values = [
            BuildingHvacWeatherProtocolV1::GetBuildingHvacWeatherV1 {
                after_cursor: Some(cursor(CURSOR_TIMESTAMP, 20)),
                history_range: Some(history_range()),
                include_current: true,
                forecast_range: Some(forecast_range()),
                outdoor_air_quality_range: Some(air_quality_range()),
            },
            BuildingHvacWeatherProtocolV1::BuildingHvacWeatherRecoveryV1 {
                maximum_wait_interval_seconds:
                    BUILDING_HVAC_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
                recovery: BuildingHvacWeatherRecoveryV1::ReplayedV1 {
                    report: incremental_report(),
                },
            },
            BuildingHvacWeatherProtocolV1::BuildingHvacWeatherRecoveryV1 {
                maximum_wait_interval_seconds:
                    BUILDING_HVAC_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
                recovery: BuildingHvacWeatherRecoveryV1::ResetV1 {
                    reason: BuildingHvacWeatherResetReasonV1::CursorExpired,
                    cursor: cursor(CURSOR_TIMESTAMP, 22),
                    snapshot: snapshot(),
                },
            },
            BuildingHvacWeatherProtocolV1::BuildingHvacWeatherRecoveryV1 {
                maximum_wait_interval_seconds:
                    BUILDING_HVAC_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
                recovery: BuildingHvacWeatherRecoveryV1::ErrorV1 {
                    error: BuildingHvacWeatherRecoveryErrorV1::TemporarilyUnavailable,
                    retry_after_seconds: Some(60),
                },
            },
            BuildingHvacWeatherProtocolV1::BuildingHvacWeatherIncrementV1 {
                report: incremental_report(),
            },
        ];

        for value in values {
            assert_round_trip!(BuildingHvacWeatherProtocolV1, value);
        }
    }

    #[test]
    fn every_persistent_section_round_trips_independently() {
        let values = [
            BuildingHvacWeatherPersistentDataV1::LocationV1 {
                location: BuildingHvacWeatherLocationV1 {
                    longitude_degrees: -74.006,
                    latitude_degrees: 40.7128,
                },
            },
            BuildingHvacWeatherPersistentDataV1::HistoryV1 { history: history() },
            BuildingHvacWeatherPersistentDataV1::CurrentV1 { current: current() },
            BuildingHvacWeatherPersistentDataV1::ForecastV1 {
                forecast: forecast(),
            },
            BuildingHvacWeatherPersistentDataV1::OutdoorAirQualityV1 {
                outdoor_air_quality: air_quality(),
            },
        ];

        for value in values {
            assert_round_trip!(BuildingHvacWeatherPersistentDataV1, value);
        }
    }

    #[test]
    fn enum_and_union_discriminants_are_stable() {
        let precipitation_kinds = [
            BuildingHvacPrecipitationKindV1::None,
            BuildingHvacPrecipitationKindV1::Rain,
            BuildingHvacPrecipitationKindV1::FreezingRain,
            BuildingHvacPrecipitationKindV1::Snow,
            BuildingHvacPrecipitationKindV1::Mixed,
            BuildingHvacPrecipitationKindV1::Unknown,
        ];
        let sections = [
            BuildingHvacWeatherSectionV1::History,
            BuildingHvacWeatherSectionV1::Current,
            BuildingHvacWeatherSectionV1::Forecast,
            BuildingHvacWeatherSectionV1::OutdoorAirQuality,
        ];
        let reset_reasons = [
            BuildingHvacWeatherResetReasonV1::InitialSubscription,
            BuildingHvacWeatherResetReasonV1::CursorExpired,
            BuildingHvacWeatherResetReasonV1::ServerCursorReset,
        ];
        let recovery_errors = [
            BuildingHvacWeatherRecoveryErrorV1::InvalidRange,
            BuildingHvacWeatherRecoveryErrorV1::CursorAhead,
            BuildingHvacWeatherRecoveryErrorV1::RequestTooLarge,
            BuildingHvacWeatherRecoveryErrorV1::TemporarilyUnavailable,
        ];

        for (index, value) in precipitation_kinds.iter().enumerate() {
            assert_eq!(value.to_avro().first(), Some(&((index as u8) * 2)));
        }
        for (index, value) in sections.iter().enumerate() {
            assert_eq!(value.to_avro().first(), Some(&((index as u8) * 2)));
        }
        for (index, value) in reset_reasons.iter().enumerate() {
            assert_eq!(value.to_avro().first(), Some(&((index as u8) * 2)));
        }
        for (index, value) in recovery_errors.iter().enumerate() {
            assert_eq!(value.to_avro().first(), Some(&((index as u8) * 2)));
        }

        let changes = [
            BuildingHvacWeatherChangeV1::HistoryPeriodsUpsertV1 {
                retrieved_at: history().retrieved_at,
                valid_until: history().valid_until,
                periods: history().periods,
            },
            BuildingHvacWeatherChangeV1::HistoryPeriodsRemoveV1 {
                range: history_range(),
            },
            BuildingHvacWeatherChangeV1::CurrentReplaceV1 { current: current() },
            BuildingHvacWeatherChangeV1::ForecastPeriodsUpsertV1 {
                retrieved_at: forecast().retrieved_at,
                valid_until: forecast().valid_until,
                periods: forecast().periods,
            },
            BuildingHvacWeatherChangeV1::ForecastPeriodsRemoveV1 {
                range: forecast_range(),
            },
            BuildingHvacWeatherChangeV1::OutdoorAirQualityPeriodsUpsertV1 {
                retrieved_at: air_quality().retrieved_at,
                valid_until: air_quality().valid_until,
                periods: air_quality().periods,
            },
            BuildingHvacWeatherChangeV1::OutdoorAirQualityPeriodsRemoveV1 {
                range: air_quality_range(),
            },
            BuildingHvacWeatherChangeV1::SectionClearV1 {
                section: BuildingHvacWeatherSectionV1::Current,
            },
            BuildingHvacWeatherChangeV1::HistoryReplaceV1 { history: history() },
            BuildingHvacWeatherChangeV1::ForecastReplaceV1 {
                forecast: forecast(),
            },
            BuildingHvacWeatherChangeV1::OutdoorAirQualityReplaceV1 {
                outdoor_air_quality: air_quality(),
            },
        ];

        for (index, value) in changes.iter().enumerate() {
            assert_eq!(value.to_avro().first(), Some(&((index as u8) * 2)));
        }

        let recoveries = [
            BuildingHvacWeatherRecoveryV1::ReplayedV1 {
                report: incremental_report(),
            },
            BuildingHvacWeatherRecoveryV1::ResetV1 {
                reason: BuildingHvacWeatherResetReasonV1::CursorExpired,
                cursor: cursor(CURSOR_TIMESTAMP, 22),
                snapshot: snapshot(),
            },
            BuildingHvacWeatherRecoveryV1::ErrorV1 {
                error: BuildingHvacWeatherRecoveryErrorV1::InvalidRange,
                retry_after_seconds: None,
            },
        ];
        for (index, value) in recoveries.iter().enumerate() {
            assert_eq!(value.to_avro().first(), Some(&((index as u8) * 2)));
        }

        let protocols = [
            BuildingHvacWeatherProtocolV1::GetBuildingHvacWeatherV1 {
                after_cursor: None,
                history_range: None,
                include_current: false,
                forecast_range: None,
                outdoor_air_quality_range: None,
            },
            BuildingHvacWeatherProtocolV1::BuildingHvacWeatherRecoveryV1 {
                maximum_wait_interval_seconds:
                    BUILDING_HVAC_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
                recovery: BuildingHvacWeatherRecoveryV1::ErrorV1 {
                    error: BuildingHvacWeatherRecoveryErrorV1::InvalidRange,
                    retry_after_seconds: None,
                },
            },
            BuildingHvacWeatherProtocolV1::BuildingHvacWeatherIncrementV1 {
                report: incremental_report(),
            },
        ];
        for (index, value) in protocols.iter().enumerate() {
            assert_eq!(value.to_avro().first(), Some(&((index as u8) * 2)));
        }

        let persistent_values = [
            BuildingHvacWeatherPersistentDataV1::LocationV1 {
                location: BuildingHvacWeatherLocationV1 {
                    longitude_degrees: -74.006,
                    latitude_degrees: 40.7128,
                },
            },
            BuildingHvacWeatherPersistentDataV1::HistoryV1 { history: history() },
            BuildingHvacWeatherPersistentDataV1::CurrentV1 { current: current() },
            BuildingHvacWeatherPersistentDataV1::ForecastV1 {
                forecast: forecast(),
            },
            BuildingHvacWeatherPersistentDataV1::OutdoorAirQualityV1 {
                outdoor_air_quality: air_quality(),
            },
        ];
        for (index, value) in persistent_values.iter().enumerate() {
            assert_eq!(value.to_avro().first(), Some(&((index as u8) * 2)));
        }
    }

    #[test]
    fn incremental_reports_require_an_exact_contiguous_cursor_range() {
        let report = incremental_report();
        assert!(report.has_contiguous_cursor_range());
        assert!(report.can_apply_after(cursor(CURSOR_TIMESTAMP, 20)));
        assert!(!report.can_apply_after(cursor(CURSOR_TIMESTAMP, 19)));

        let sequence_gap = BuildingHvacWeatherIncrementalReportV1 {
            through_cursor: cursor(CURSOR_TIMESTAMP, 23),
            ..report.clone()
        };
        assert!(!sequence_gap.has_contiguous_cursor_range());

        let timestamp_change = BuildingHvacWeatherIncrementalReportV1 {
            through_cursor: cursor(LATER_CURSOR_TIMESTAMP, 22),
            ..report
        };
        assert!(!timestamp_change.has_contiguous_cursor_range());
    }

    #[test]
    fn empty_incremental_report_is_a_cursor_preserving_heartbeat() {
        let report = BuildingHvacWeatherIncrementalReportV1 {
            from_cursor: cursor(CURSOR_TIMESTAMP, 22),
            through_cursor: cursor(CURSOR_TIMESTAMP, 22),
            changes: Vec::new(),
        };

        assert!(report.can_apply_after(cursor(CURSOR_TIMESTAMP, 22)));
        assert_eq!(report.from_cursor, report.through_cursor);
    }

    #[test]
    fn cursor_reset_requires_newer_timestamp_and_backward_sequence() {
        let previous = cursor(CURSOR_TIMESTAMP, 20);

        assert!(cursor(LATER_CURSOR_TIMESTAMP, 0).is_server_reset_after(previous));
        assert!(cursor(LATER_CURSOR_TIMESTAMP, 3).is_server_reset_after(previous));
        assert!(cursor(LATER_CURSOR_TIMESTAMP, 3).is_valid_successor_of(previous));
        assert!(!cursor(CURSOR_TIMESTAMP, 3).is_server_reset_after(previous));
        assert!(!cursor(CURSOR_TIMESTAMP, 3).is_valid_successor_of(previous));
        assert!(!cursor(CURSOR_TIMESTAMP - 1, 3).is_server_reset_after(previous));
        assert!(!cursor(LATER_CURSOR_TIMESTAMP, 21).is_server_reset_after(previous));
        assert!(cursor(CURSOR_TIMESTAMP, 21).is_valid_successor_of(previous));
    }

    #[test]
    fn freshness_expires_at_the_valid_until_boundary() {
        let history = history();
        let current = current();
        let forecast = forecast();
        let air_quality = air_quality();

        assert!(history.is_fresh_at(history.valid_until - 1));
        assert!(!history.is_fresh_at(history.valid_until));
        assert!(current.is_fresh_at(current.valid_until - 1));
        assert!(!current.is_fresh_at(current.valid_until));
        assert!(forecast.is_fresh_at(forecast.valid_until - 1));
        assert!(!forecast.is_fresh_at(forecast.valid_until));
        assert!(air_quality.is_fresh_at(air_quality.valid_until - 1));
        assert!(!air_quality.is_fresh_at(air_quality.valid_until));
    }

    #[test]
    fn recovery_ranges_are_half_open_and_nonempty() {
        assert!(history_range().is_valid());
        assert!(forecast_range().is_valid());
        assert!(air_quality_range().is_valid());
        assert!(
            !BuildingHvacWeatherTimeRangeV1 {
                starts_at: 100,
                ends_before: 100,
            }
            .is_valid()
        );
        assert!(
            !BuildingHvacWeatherTimeRangeV1 {
                starts_at: 101,
                ends_before: 100,
            }
            .is_valid()
        );
    }

    #[test]
    fn refresh_coverage_and_subscription_policy_is_stable() {
        assert_eq!(BUILDING_HVAC_CURRENT_REFRESH_INTERVAL_SECONDS, 900);
        assert_eq!(BUILDING_HVAC_HISTORY_REFRESH_INTERVAL_SECONDS, 3_600);
        assert_eq!(BUILDING_HVAC_FORECAST_REFRESH_INTERVAL_SECONDS, 3_600);
        assert_eq!(BUILDING_HVAC_AIR_QUALITY_REFRESH_INTERVAL_SECONDS, 3_600);
        assert_eq!(BUILDING_HVAC_CURRENT_FRESHNESS_SECONDS, 1_800);
        assert_eq!(BUILDING_HVAC_HISTORY_FRESHNESS_SECONDS, 7_200);
        assert_eq!(BUILDING_HVAC_FORECAST_FRESHNESS_SECONDS, 10_800);
        assert_eq!(BUILDING_HVAC_AIR_QUALITY_FRESHNESS_SECONDS, 7_200);
        assert_eq!(BUILDING_HVAC_HISTORY_WINDOW_SECONDS, 259_200);
        assert_eq!(BUILDING_HVAC_FORECAST_HORIZON_SECONDS, 259_200);
        assert_eq!(BUILDING_HVAC_AIR_QUALITY_HORIZON_SECONDS, 172_800);
        assert_eq!(BUILDING_HVAC_SUBSCRIPTION_REPLAY_WINDOW_SECONDS, 86_400);
        assert_eq!(
            BUILDING_HVAC_SUBSCRIPTION_MAXIMUM_WAIT_INTERVAL_SECONDS,
            1_200
        );
    }

    #[test]
    fn truncated_persistent_data_is_rejected() {
        let encoded = BuildingHvacWeatherPersistentDataV1::ForecastV1 {
            forecast: forecast(),
        }
        .to_avro();
        let mut offset = 0;

        assert!(
            BuildingHvacWeatherPersistentDataV1::avro_decode(
                &encoded[..encoded.len() - 1],
                &mut offset
            )
            .is_err()
        );
    }
}
