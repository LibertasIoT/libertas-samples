//! Libertas chart gallery.
//! Explore every chart mark and composition with polished, interactive sample data.
//! #[libertas_string_resources(DIRECT_ANNOTATION_STRINGS)]
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::{boxed::Box, format, string::String, vec, vec::Vec};
use core::any::Any;

use libertas::{
    LibertasDateTime, LibertasEndpoint, LibertasEndpointStatus, LibertasTimeOnly, OP_ENDPOINT_REQ,
    libertas_endpoint_response, libertas_formatted_text, libertas_get_utc_time,
    libertas_register_endpoint_listener,
};
use libertas_macros::{
    LibertasAvroDecode, LibertasAvroEncode, LibertasExport, libertas_chart, libertas_export,
};

/// Business portfolio
/// Identifies a product family in comparison charts.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum PortfolioV1 {
    /// Home
    /// Consumer products.
    Home,
    /// Business
    /// Products for professional teams.
    Business,
    /// Platform
    /// Shared platform services.
    Platform,
}

/// Market
/// Geographic market used for faceting.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum MarketV1 {
    /// Americas
    /// North and South America.
    Americas,
    /// Europe
    /// Europe, the Middle East, and Africa.
    Europe,
}

/// Customer tier
/// Visual point shape for a customer segment.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum CustomerTierV1 {
    /// Standard
    /// Standard service tier.
    Standard,
    /// Premium
    /// Premium service tier.
    Premium,
}

/// Release stage
/// Product maturity used for column faceting.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum ReleaseStageV1 {
    /// Established
    /// A broadly available product.
    Established,
    /// Emerging
    /// A newer product still expanding its audience.
    Emerging,
}

/// Scenario
/// Distinguishes recorded, expected, and stretch values.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum ScenarioV1 {
    /// Recorded
    /// A completed measurement.
    Recorded,
    /// Expected
    /// The current forecast or plan.
    Expected,
    /// Stretch
    /// An ambitious target.
    Stretch,
}

/// Quarter
/// Quarter used on categorical axes.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum QuarterV1 {
    /// First quarter
    /// January through March.
    Q1,
    /// Second quarter
    /// April through June.
    Q2,
    /// Third quarter
    /// July through September.
    Q3,
    /// Fourth quarter
    /// October through December.
    Q4,
}

/// Weekday
/// Day used on the activity heatmap.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum WeekdayV1 {
    /// Monday
    /// Monday activity.
    Monday,
    /// Tuesday
    /// Tuesday activity.
    Tuesday,
    /// Wednesday
    /// Wednesday activity.
    Wednesday,
    /// Thursday
    /// Thursday activity.
    Thursday,
    /// Friday
    /// Friday activity.
    Friday,
}

/// Time block
/// Fixed portion of the workday used by the activity heatmap.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum TimeBlockV1 {
    /// Morning
    /// Early workday activity.
    Morning,
    /// Midday
    /// Midday activity.
    Midday,
    /// Afternoon
    /// Afternoon activity.
    Afternoon,
    /// Evening
    /// Late workday activity.
    Evening,
}

/// Capability
/// Dimension around the capability radar.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum CapabilityV1 {
    /// Speed
    /// Responsiveness and throughput.
    Speed,
    /// Reliability
    /// Stability under normal operation.
    Reliability,
    /// Efficiency
    /// Resource efficiency.
    Efficiency,
    /// Simplicity
    /// Ease of use.
    Simplicity,
    /// Reach
    /// Breadth of supported use cases.
    Reach,
}

/// Traffic source
/// Source represented by a donut segment.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum TrafficSourceV1 {
    /// Direct
    /// People navigating directly to the product.
    Direct,
    /// Search
    /// Search-engine referrals.
    Search,
    /// Partner
    /// Partner referrals.
    Partner,
    /// Campaign
    /// Campaign referrals.
    Campaign,
    /// Social
    /// Social-media referrals.
    Social,
    /// Other
    /// Small uncategorized referral sources.
    Other,
}

/// Annotation mood
/// Semantic color for direct labels.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum AnnotationMoodV1 {
    /// Positive
    /// A favorable result.
    Positive,
    /// Neutral
    /// An informational result.
    Neutral,
    /// Attention
    /// A result needing attention.
    Attention,
}

/// Energy source
/// Contributor used in explicit stacked-energy charts.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum EnergySourceV1 {
    /// Solar
    /// Locally generated solar energy.
    Solar,
    /// Battery
    /// Energy discharged from storage.
    Battery,
    /// Grid
    /// Energy imported from the electrical grid.
    Grid,
}

/// Candle direction
/// Direction of an open-to-close market interval.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum CandleDirectionV1 {
    /// Rising
    /// The close is at or above the open.
    Rising,
    /// Falling
    /// The close is below the open.
    Falling,
}

/// Map region
/// Server-projected region in the map example.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum MapRegionV1 {
    /// North district
    /// The northern projected region.
    North,
    /// Central district
    /// The central projected region.
    Central,
    /// South district
    /// The southern projected region.
    South,
}

/// Network node
/// Fixed node identity used by the topology example.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum NetworkNodeNameV1 {
    /// Gateway
    /// Entry point into the demonstrated topology.
    Gateway,
    /// Rules
    /// Rules-processing service.
    Rules,
    /// Analytics
    /// Analytics-processing service.
    Analytics,
    /// Events
    /// Event storage.
    Events,
    /// History
    /// Historical storage.
    History,
}

/// Network connection
/// Fixed relationship identity used by the topology example.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum NetworkConnectionV1 {
    /// Gateway to rules
    /// Connects the gateway to rules processing.
    GatewayToRules,
    /// Gateway to analytics
    /// Connects the gateway to analytics processing.
    GatewayToAnalytics,
    /// Rules to events
    /// Connects rules processing to event storage.
    RulesToEvents,
    /// Analytics to history
    /// Connects analytics processing to historical storage.
    AnalyticsToHistory,
    /// Rules to history
    /// Connects rules processing to historical storage.
    RulesToHistory,
}

/// Network role
/// Functional role of a node in the topology example.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum NetworkRoleV1 {
    /// Gateway
    /// Entry point into the network.
    Gateway,
    /// Service
    /// Processing service in the network.
    Service,
    /// Storage
    /// Persistent storage node.
    Storage,
}

/// Flow stage
/// Fixed stage identity used by the Sankey example.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum FlowStageV1 {
    /// Visits
    /// Incoming visits.
    Visits,
    /// Evaluation
    /// Visits evaluating the product.
    Evaluation,
    /// Activated
    /// Visits that completed activation.
    Activated,
    /// Exited
    /// Visits that left before activation.
    Exited,
}

/// Flow stream
/// One independently sampled ribbon in the Sankey example.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum FlowStreamV1 {
    /// Direct activation
    /// Visits that activate directly.
    DirectActivation,
    /// Assisted activation
    /// Visits that activate after assistance.
    AssistedActivation,
    /// Early exit
    /// Visits that leave before activation.
    EarlyExit,
}

/// Bubble portfolio point
/// One product positioned by reach and signed growth, with visual encodings for
/// value, family, tier, confidence, and market.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BubblePointV1 {
    /// Product
    /// Product name and stable interaction key.
    #[libertas_chart_channel(tooltip, key)]
    pub product: String,
    /// Active customers
    /// Customer reach on a logarithmic horizontal scale.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = customer_reach, kind = log, min = 1)]
    #[libertas_chart_guide(target = x, source = scale, position = bottom)]
    pub active_customers: u32,
    /// Annual growth
    /// Signed growth on a symmetric logarithmic vertical scale.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = annual_growth, kind = symlog, zero = true)]
    #[libertas_chart_guide(target = y, source = scale, position = left)]
    #[libertas_format("0.#")]
    #[libertas_physical_unit("percent")]
    pub annual_growth_percent: f32,
    /// Annual data volume
    /// Controls bubble area with a square-root scale.
    #[libertas_chart_channel(size, tooltip)]
    #[libertas_chart_scale(id = annual_value, kind = sqrt, min = 0)]
    #[libertas_chart_guide(target = size, source = scale, position = bottom)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("terabyte")]
    pub annual_data_terabytes: f32,
    /// Product family
    /// Bubble color and interaction detail grouping.
    #[libertas_chart_channel(color, detail, tooltip)]
    #[libertas_chart_scale(id = portfolio_color, kind = ordinal)]
    #[libertas_chart_guide(target = color, source = scale, position = bottom)]
    pub portfolio: PortfolioV1,
    /// Customer tier
    /// Bubble shape.
    #[libertas_chart_channel(shape, tooltip)]
    #[libertas_chart_scale(id = customer_tier_shape, kind = ordinal)]
    #[libertas_chart_guide(target = shape, source = scale, position = bottom)]
    pub tier: CustomerTierV1,
    /// Confidence opacity
    /// Bubble opacity on an identity scale.
    #[libertas_chart_channel(opacity)]
    #[libertas_chart_scale(id = confidence_opacity, kind = identity)]
    pub confidence_opacity: f32,
    /// Confidence
    /// Human-readable confidence shown in the tooltip.
    #[libertas_chart_channel(tooltip)]
    #[libertas_physical_unit("percent")]
    pub confidence_percent: u8,
    /// Market
    /// Creates a row for each geographic market.
    #[libertas_chart_channel(row, tooltip)]
    #[libertas_chart_scale(id = market_rows, kind = band)]
    pub market: MarketV1,
    /// Release stage
    /// Creates a column for each maturity stage.
    #[libertas_chart_channel(column, tooltip)]
    #[libertas_chart_scale(id = stage_columns, kind = band)]
    pub stage: ReleaseStageV1,
}

/// Bubble portfolio
/// A faceted bubble chart demonstrating quantitative and categorical point encodings.
#[libertas_chart(point)]
pub type BubblePortfolioChartV1 = Vec<BubblePointV1>;

/// Energy history point
/// One time-series measurement for an energy scenario.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct EnergyHistoryPointV1 {
    /// Time
    /// UTC measurement time.
    #[libertas_chart_channel(x, tooltip, key)]
    #[libertas_chart_scale(id = gallery_utc_time, kind = utc)]
    #[libertas_chart_guide(target = x, source = scale, position = bottom)]
    pub energy_at: LibertasDateTime,
    /// Power
    /// Electrical demand at this time.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = power_scale, kind = linear, min = 0, zero = true)]
    #[libertas_chart_guide(target = y, source = scale, position = left)]
    #[libertas_format("0.#")]
    #[libertas_physical_unit("kilowatt")]
    pub power_kilowatts: f32,
    /// Scenario
    /// Line color and detail grouping.
    #[libertas_chart_channel(color, detail, tooltip)]
    #[libertas_chart_scale(id = scenario_style, kind = ordinal)]
    #[libertas_chart_guide(target = color, source = scale, position = bottom)]
    pub scenario: ScenarioV1,
    /// Scenario dash
    /// Repeats the scenario as a dash encoding with its legend below the plot.
    #[libertas_chart_channel(strokeDash)]
    #[libertas_chart_scale(id = scenario_dash, kind = ordinal)]
    #[libertas_chart_guide(target = strokeDash, source = scale, position = bottom)]
    pub dash_scenario: ScenarioV1,
    /// Emphasis
    /// Line width on an identity scale.
    #[libertas_chart_channel(strokeWidth)]
    #[libertas_chart_scale(kind = identity)]
    pub stroke_width: f32,
    /// Sequence
    /// Drawing order within each line.
    #[libertas_chart_channel(order)]
    pub sequence: u8,
}

/// Energy history
/// Recorded, expected, and stretch energy series on a shared UTC timeline.
#[libertas_chart(line)]
pub type EnergyHistoryChartV1 = Vec<EnergyHistoryPointV1>;

/// Forecast range point
/// One uncertainty interval in a forecast confidence band.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct ForecastRangePointV1 {
    /// Time
    /// UTC forecast time.
    #[libertas_chart_channel(x, tooltip, key)]
    #[libertas_chart_scale(id = forecast_local_time, kind = time)]
    #[libertas_chart_guide(target = x, source = scale, position = bottom)]
    pub forecast_at: LibertasDateTime,
    /// Low estimate
    /// Lower bound of the forecast interval.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = temperature_range, kind = linear, zero = false)]
    #[libertas_chart_guide(target = y, source = scale, position = left)]
    #[libertas_format("0.#")]
    #[libertas_physical_unit("celsius")]
    pub low_celsius: f32,
    /// High estimate
    /// Upper bound of the forecast interval.
    #[libertas_chart_channel(y2, tooltip)]
    #[libertas_format("0.#")]
    #[libertas_physical_unit("celsius")]
    pub high_celsius: f32,
    /// Forecast model
    /// Fill and detail grouping for the interval.
    #[libertas_chart_channel(fill, detail, tooltip)]
    #[libertas_chart_scale(id = portfolio_color, kind = ordinal)]
    #[libertas_chart_guide(target = fill, source = scale, position = bottom)]
    pub model: PortfolioV1,
    /// Forecast outline
    /// Repeats the model as the confidence-band outline.
    #[libertas_chart_channel(stroke)]
    #[libertas_chart_scale(id = forecast_outline, kind = ordinal)]
    #[libertas_chart_guide(target = stroke, source = scale, position = bottom)]
    pub outline_model: PortfolioV1,
    /// Confidence shading
    /// Opacity of the forecast band.
    #[libertas_chart_channel(fillOpacity)]
    #[libertas_chart_scale(kind = identity)]
    pub confidence_opacity: f32,
}

/// Forecast range
/// Layer-ready confidence bands using ranged area marks.
#[libertas_chart(area)]
pub type ForecastRangeChartV1 = Vec<ForecastRangePointV1>;

/// Capability radar point
/// One ordered vertex of a polar capability profile.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct CapabilityRadarPointV1 {
    /// Angle
    /// Angular position on a normalized turn.
    #[libertas_chart_channel(theta)]
    #[libertas_chart_scale(id = capability_angle, kind = identity)]
    pub angle_turn: f32,
    /// Capability
    /// Supplies human-readable labels around the angular guide.
    #[libertas_chart_channel(tooltip)]
    #[libertas_chart_guide(target = theta, source = value, position = auto)]
    pub capability: CapabilityV1,
    /// Score
    /// Radial capability score.
    #[libertas_chart_channel(radius, tooltip)]
    #[libertas_chart_scale(id = capability_score, kind = linear, min = 0, max = 100, zero = true)]
    #[libertas_chart_guide(target = radius, source = scale, position = auto)]
    #[libertas_physical_unit("percent")]
    pub score_percent: u8,
    /// Portfolio
    /// Filled profile color and detail group.
    #[libertas_chart_channel(fill, detail, tooltip)]
    #[libertas_chart_scale(id = portfolio_color, kind = ordinal)]
    #[libertas_chart_guide(target = fill, source = scale, position = bottom)]
    pub portfolio: PortfolioV1,
    /// Profile outline
    /// Repeats the portfolio as an outline encoding.
    #[libertas_chart_channel(stroke)]
    #[libertas_chart_scale(id = radar_outline, kind = ordinal)]
    #[libertas_chart_guide(target = stroke, source = scale, position = bottom)]
    pub outline_portfolio: PortfolioV1,
    /// Fill opacity
    /// Transparent fill retains overlapping profiles.
    #[libertas_chart_channel(fillOpacity)]
    #[libertas_chart_scale(kind = identity)]
    pub fill_opacity: f32,
    /// Vertex order
    /// Connects capability vertices in a stable order.
    #[libertas_chart_channel(order)]
    pub sequence: u8,
    /// Vertex key
    /// Stable identity for interactive focus.
    #[libertas_chart_channel(key)]
    pub vertex_key: String,
}

/// Capability radar
/// Overlapping polar polygons for comparing multidimensional profiles.
#[libertas_chart(polygon)]
pub type CapabilityRadarChartV1 = Vec<CapabilityRadarPointV1>;

/// Grouped sales bar
/// One portfolio contribution within a quarter.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct GroupedSalesBarV1 {
    /// Quarter
    /// Categorical group on the horizontal axis.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = quarter_band, kind = band)]
    #[libertas_chart_guide(target = x, source = scale, position = bottom)]
    pub quarter: QuarterV1,
    /// Portfolio offset
    /// Places portfolios beside one another within a quarter.
    #[libertas_chart_channel(xOffset, detail, tooltip)]
    #[libertas_chart_scale(id = portfolio_offset, kind = band)]
    pub portfolio: PortfolioV1,
    /// Portfolio color
    /// Repeats portfolio identity as color with its legend below the plot.
    #[libertas_chart_channel(color)]
    #[libertas_chart_scale(id = portfolio_color, kind = ordinal)]
    #[libertas_chart_guide(target = color, source = scale, position = bottom)]
    pub portfolio_color: PortfolioV1,
    /// Revenue
    /// Quarterly revenue.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = revenue_scale, kind = linear, min = 0, zero = true)]
    #[libertas_chart_guide(target = y, source = scale, position = left)]
    #[libertas_format("0.0")]
    pub revenue_millions: f32,
    /// Stable key
    /// Unique quarter and portfolio identity.
    #[libertas_chart_channel(key)]
    pub bar_key: String,
}

/// Grouped sales
/// Side-by-side categorical bars using a nested offset scale.
#[libertas_chart(bar)]
pub type GroupedSalesChartV1 = Vec<GroupedSalesBarV1>;

/// Horizontal grouped bar
/// One portfolio contribution offset vertically within a quarter.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct HorizontalGroupedBarV1 {
    /// Revenue
    /// Quarterly revenue on the horizontal quantitative axis.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = horizontal_revenue, kind = linear, min = 0, zero = true)]
    #[libertas_chart_guide(target = x, source = scale, position = bottom)]
    #[libertas_format("0.0")]
    pub revenue_millions: f32,
    /// Quarter
    /// Main categorical row.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = horizontal_quarter, kind = band)]
    #[libertas_chart_guide(target = y, source = scale, position = left)]
    pub quarter: QuarterV1,
    /// Portfolio offset
    /// Places portfolios beside one another within each horizontal group.
    #[libertas_chart_channel(yOffset, detail, tooltip)]
    #[libertas_chart_scale(id = horizontal_portfolio_offset, kind = band)]
    pub portfolio_offset: PortfolioV1,
    /// Portfolio color
    /// Repeats the portfolio identity as color with a legend below the plot.
    #[libertas_chart_channel(color)]
    #[libertas_chart_scale(id = portfolio_color, kind = ordinal)]
    #[libertas_chart_guide(target = color, source = scale, position = bottom)]
    pub portfolio_color: PortfolioV1,
    /// Stable key
    /// Unique quarter and portfolio identity.
    #[libertas_chart_channel(key)]
    pub bar_key: String,
}

/// Horizontal grouped sales
/// Horizontal bars demonstrating categorical vertical offsets.
#[libertas_chart(bar)]
pub type HorizontalGroupedSalesChartV1 = Vec<HorizontalGroupedBarV1>;

/// Activity heatmap cell
/// One rectangular weekday and time interval colored by activity.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct ActivityHeatmapCellV1 {
    /// Start time
    /// Beginning of the time-of-day cell.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = office_time, kind = linear)]
    pub starts_at: LibertasTimeOnly,
    /// End time
    /// End of the time-of-day cell.
    #[libertas_chart_channel(x2, tooltip)]
    pub ends_at: LibertasTimeOnly,
    /// Time block
    /// Supplies a localized fixed label for each horizontal interval.
    #[libertas_chart_channel(tooltip)]
    #[libertas_chart_guide(target = x, source = span, position = top, title = none)]
    pub time_block: TimeBlockV1,
    /// Day position
    /// Numeric start of the weekday band.
    #[libertas_chart_channel(y)]
    #[libertas_chart_scale(id = weekday_position, kind = linear, min = 0, max = 5)]
    pub day_start: f32,
    /// Day end
    /// Numeric end of the weekday band.
    #[libertas_chart_channel(y2)]
    pub day_end: f32,
    /// Day
    /// Supplies human-readable weekday labels for the vertical guide.
    #[libertas_chart_channel(tooltip)]
    #[libertas_chart_guide(
        target = y,
        source = value,
        position = left,
        title = none,
        domain = none,
        ticks = none,
        grid = none
    )]
    pub weekday: WeekdayV1,
    /// Activity
    /// Cell color and tooltip intensity.
    #[libertas_chart_channel(color, tooltip)]
    #[libertas_chart_scale(id = activity_color, kind = linear, min = 0, max = 100, zero = true)]
    #[libertas_chart_guide(target = color, source = scale, position = bottom)]
    #[libertas_physical_unit("percent")]
    pub activity_percent: u8,
    /// Cell key
    /// Stable identity for interactive focus.
    #[libertas_chart_channel(key)]
    pub cell_key: String,
}

/// Activity heatmap
/// Ranged rectangular cells with a time scale, quantitative color, and custom labels.
#[libertas_chart(rect)]
pub type ActivityHeatmapChartV1 = Vec<ActivityHeatmapCellV1>;

/// Measurement uncertainty rule
/// One vertical low-to-high error interval.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct UncertaintyRuleV1 {
    /// Sample
    /// Categorical sample position.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = sample_points, kind = point)]
    #[libertas_chart_guide(target = x, source = scale, position = bottom)]
    pub sample: QuarterV1,
    /// Low estimate
    /// Lower end of the error interval.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = measurement_scale, kind = linear, zero = false)]
    #[libertas_chart_guide(target = y, source = scale, position = right)]
    #[libertas_format("0.00")]
    #[libertas_physical_unit("volt")]
    pub low_volts: f32,
    /// High estimate
    /// Upper end of the error interval.
    #[libertas_chart_channel(y2, tooltip)]
    #[libertas_format("0.00")]
    #[libertas_physical_unit("volt")]
    pub high_volts: f32,
    /// Scenario
    /// Rule color and dash pattern.
    #[libertas_chart_channel(color, detail, tooltip)]
    #[libertas_chart_scale(id = scenario_style, kind = ordinal)]
    #[libertas_chart_guide(target = color, source = scale, position = bottom)]
    pub scenario: ScenarioV1,
    /// Scenario dash
    /// Repeats the scenario as a dash encoding.
    #[libertas_chart_channel(strokeDash)]
    #[libertas_chart_scale(id = uncertainty_dash, kind = ordinal)]
    #[libertas_chart_guide(target = strokeDash, source = scale, position = bottom)]
    pub dash_scenario: ScenarioV1,
    /// Rule width
    /// Emphasizes the interval.
    #[libertas_chart_channel(strokeWidth)]
    #[libertas_chart_scale(kind = identity)]
    pub stroke_width: f32,
    /// Rule key
    /// Stable identity for the sample interval.
    #[libertas_chart_channel(key)]
    pub rule_key: String,
}

/// Measurement uncertainty
/// Error intervals rendered as ranged rules.
#[libertas_chart(rule)]
pub type MeasurementUncertaintyChartV1 = Vec<UncertaintyRuleV1>;

/// Traffic donut segment
/// One explicitly positioned annular segment.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct TrafficDonutSegmentV1 {
    /// Arc start
    /// Beginning of the segment on a normalized turn.
    #[libertas_chart_channel(theta)]
    #[libertas_chart_scale(id = donut_angle, kind = linear, min = 0, max = 1, zero = true)]
    #[libertas_chart_guide(target = theta, source = none)]
    pub theta_start: f32,
    /// Arc end
    /// End of the segment on a normalized turn.
    #[libertas_chart_channel(theta2)]
    pub theta_end: f32,
    /// Inner radius
    /// Creates the donut hole.
    #[libertas_chart_channel(radius)]
    #[libertas_chart_scale(id = donut_radius, kind = linear, min = 0, max = 1, zero = true)]
    #[libertas_chart_guide(target = radius, source = none)]
    pub radius_start: f32,
    /// Outer radius
    /// Outer edge of the donut.
    #[libertas_chart_channel(radius2)]
    pub radius_end: f32,
    /// Traffic source
    /// Segment fill, linked outside label, detail group, and tooltip category.
    #[libertas_chart_channel(fill, detail, tooltip)]
    #[libertas_chart_scale(id = traffic_color, kind = ordinal)]
    #[libertas_chart_guide(target = fill, source = scale, position = auto)]
    pub source: TrafficSourceV1,
    /// Share
    /// Human-readable share in the tooltip.
    #[libertas_chart_channel(tooltip)]
    #[libertas_format("0.#")]
    #[libertas_physical_unit("percent")]
    pub share_percent: f32,
    /// Border opacity
    /// Separates adjacent segments.
    #[libertas_chart_channel(strokeOpacity)]
    #[libertas_chart_scale(kind = identity)]
    pub border_opacity: f32,
    /// Segment order
    /// Stable clockwise drawing order.
    #[libertas_chart_channel(order)]
    pub sequence: u8,
    /// Segment key
    /// Stable identity for interactive focus.
    #[libertas_chart_channel(key)]
    pub segment_key: String,
}

/// Traffic sources
/// A donut whose categorical fill guide lowers to linked outside labels.
#[libertas_chart(arc)]
pub type TrafficDonutChartV1 = Vec<TrafficDonutSegmentV1>;

/// Five-minute temperature range
/// The observed low and high within one five-minute bucket.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct TemperatureHighLowRuleV1 {
    /// Bucket midpoint
    /// UTC midpoint of this five-minute observation bucket.
    #[libertas_chart_channel(x, tooltip, key)]
    #[libertas_chart_scale(id = temperature_time, kind = utc)]
    #[libertas_chart_guide(target = x, source = scale, position = bottom)]
    pub middle_at: LibertasDateTime,
    /// Low temperature
    /// Lowest observed temperature in the bucket.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = observed_temperature, kind = linear, zero = false)]
    #[libertas_chart_guide(target = y, source = scale, position = left)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("celsius")]
    pub low_celsius: f32,
    /// High temperature
    /// Highest observed temperature in the bucket.
    #[libertas_chart_channel(y2, tooltip)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("celsius")]
    pub high_celsius: f32,
    /// Closing temperature
    /// Last observation in the bucket, repeated for a complete tooltip.
    #[libertas_chart_channel(tooltip)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("celsius")]
    pub close_celsius: f32,
}

/// Five-minute closing-temperature tick
/// A right-facing horizontal rule marking the final observation in a bucket.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct TemperatureCloseTickV1 {
    /// Bucket midpoint
    /// UTC start of the right-facing close tick.
    #[libertas_chart_channel(x, tooltip, key)]
    #[libertas_chart_scale(id = temperature_time, kind = utc)]
    pub middle_at: LibertasDateTime,
    /// Bucket end
    /// Exclusive UTC end of the five-minute bucket.
    #[libertas_chart_channel(x2)]
    pub ends_at: LibertasDateTime,
    /// Closing temperature
    /// Last observed temperature in the bucket.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = observed_temperature, kind = linear, zero = false)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("celsius")]
    pub close_celsius: f32,
    /// Low temperature
    /// Lowest bucket observation, repeated for a complete tooltip.
    #[libertas_chart_channel(tooltip)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("celsius")]
    pub low_celsius: f32,
    /// High temperature
    /// Highest bucket observation, repeated for a complete tooltip.
    #[libertas_chart_channel(tooltip)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("celsius")]
    pub high_celsius: f32,
}

/// Five-minute temperature ranges
/// Vertical low-to-high observations for the last 72 hours.
#[libertas_chart(rule)]
pub type TemperatureHighLowRangesV1 = Vec<TemperatureHighLowRuleV1>;

/// Five-minute temperature closes
/// Right-facing closing ticks for the last 72 hours.
#[libertas_chart(rule)]
pub type TemperatureCloseTicksV1 = Vec<TemperatureCloseTickV1>;

/// Five-minute temperature, past 72 hours
/// High-low-close observations layered on shared UTC and Celsius scales.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(layer)]
pub struct TemperatureHlcChartV1 {
    /// High-low ranges
    /// One vertical range for every completed five-minute bucket.
    pub ranges: TemperatureHighLowRangesV1,
    /// Closing ticks
    /// One right-facing close tick for every completed five-minute bucket.
    pub closes: TemperatureCloseTicksV1,
}

pub const DIRECT_ANNOTATION_STRINGS: &[(&str, &str)] = &[
    ("DIRECT_LABEL_FAST", "Fast"),
    ("DIRECT_LABEL_CLEAR", "Clear"),
    ("DIRECT_LABEL_ACT_NOW", "Act now"),
];

/// Direct annotation
/// A localized formatted label positioned and styled entirely by data.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct DirectAnnotationV1 {
    /// Horizontal position
    /// Label position on the horizontal axis.
    #[libertas_chart_channel(x)]
    #[libertas_chart_scale(id = annotation_x, kind = linear, min = 0, max = 100)]
    #[libertas_chart_guide(target = x, source = scale, position = bottom, grid = none)]
    pub x: f32,
    /// Vertical position
    /// Label position on the vertical axis.
    #[libertas_chart_channel(y)]
    #[libertas_chart_scale(id = annotation_y, kind = linear, min = 0, max = 100)]
    #[libertas_chart_guide(target = y, source = scale, position = left, grid = none)]
    pub y: f32,
    /// Label
    /// Canonical localized text tuple drawn at this position.
    #[libertas_formatted_text]
    #[libertas_chart_channel(text, tooltip)]
    pub label: Vec<u8>,
    /// Mood
    /// Semantic label color.
    #[libertas_chart_channel(color, tooltip)]
    #[libertas_chart_scale(id = annotation_color, kind = ordinal)]
    #[libertas_chart_guide(target = color, source = scale, position = bottom)]
    pub mood: AnnotationMoodV1,
    /// Text size
    /// Font size on an identity scale.
    #[libertas_chart_channel(size)]
    #[libertas_chart_scale(kind = identity)]
    pub size: f32,
    /// Text angle
    /// Rotation in degrees on an identity scale.
    #[libertas_chart_channel(angle)]
    #[libertas_chart_scale(kind = identity)]
    pub angle_degrees: f32,
    /// Text opacity
    /// Label opacity on an identity scale.
    #[libertas_chart_channel(opacity)]
    #[libertas_chart_scale(kind = identity)]
    pub opacity: f32,
    /// Label key
    /// Stable identity for interactive focus.
    #[libertas_chart_channel(key)]
    pub annotation_key: String,
}

/// Direct labels
/// A text-mark showcase with data-driven content and appearance.
#[libertas_chart(text)]
pub type DirectLabelsChartV1 = Vec<DirectAnnotationV1>;

/// Performance point
/// One recorded result in the layered performance story.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct PerformancePointV1 {
    /// Time
    /// UTC measurement time.
    #[libertas_chart_channel(x, tooltip, key)]
    #[libertas_chart_scale(id = performance_time, kind = utc)]
    #[libertas_chart_guide(target = x, source = scale, position = bottom)]
    pub observed_at: LibertasDateTime,
    /// Response time
    /// Recorded response time.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = response_time_scale, kind = linear, min = 0, zero = true)]
    #[libertas_chart_guide(target = y, source = scale, position = left)]
    #[libertas_physical_unit("millisecond")]
    pub response_milliseconds: u16,
    /// Portfolio
    /// Point color and detail group.
    #[libertas_chart_channel(color, detail, tooltip)]
    #[libertas_chart_scale(id = portfolio_color, kind = ordinal)]
    #[libertas_chart_guide(target = color, source = scale, position = bottom)]
    pub portfolio: PortfolioV1,
    /// Point size
    /// Data-driven point emphasis.
    #[libertas_chart_channel(size)]
    #[libertas_chart_scale(kind = identity)]
    pub point_size: f32,
}

/// Recorded performance
/// Interactive observations for the layered performance story.
#[libertas_chart(point)]
pub type PerformancePointsV1 = Vec<PerformancePointV1>;

/// Performance trend point
/// One position on a fitted trend line.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct PerformanceTrendPointV1 {
    /// Time
    /// UTC trend time.
    #[libertas_chart_channel(x)]
    #[libertas_chart_scale(id = performance_time, kind = utc)]
    pub trend_at: LibertasDateTime,
    /// Response time
    /// Smoothed response time.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = response_time_scale, kind = linear, min = 0, zero = true)]
    #[libertas_physical_unit("millisecond")]
    pub response_milliseconds: u16,
    /// Scenario
    /// Trend line style.
    #[libertas_chart_channel(color, detail)]
    #[libertas_chart_scale(id = scenario_style, kind = ordinal)]
    pub scenario: ScenarioV1,
    /// Sequence
    /// Drawing order within the trend.
    #[libertas_chart_channel(order)]
    pub sequence: u8,
}

/// Performance trend
/// A smooth line sharing the point chart's scales.
#[libertas_chart(line)]
pub type PerformanceTrendV1 = Vec<PerformanceTrendPointV1>;

/// Performance target
/// Horizontal target spanning the demonstrated time range.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct PerformanceTargetRuleV1 {
    /// Start time
    /// Beginning of the target rule.
    #[libertas_chart_channel(x)]
    #[libertas_chart_scale(id = performance_time, kind = utc)]
    pub starts_at: LibertasDateTime,
    /// End time
    /// End of the target rule.
    #[libertas_chart_channel(x2)]
    pub ends_at: LibertasDateTime,
    /// Target response time
    /// Desired maximum response time.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = response_time_scale, kind = linear, min = 0, zero = true)]
    #[libertas_physical_unit("millisecond")]
    pub target_milliseconds: u16,
    /// Scenario
    /// Target rule color and detail group.
    #[libertas_chart_channel(color, detail, tooltip)]
    #[libertas_chart_scale(id = scenario_style, kind = ordinal)]
    pub scenario: ScenarioV1,
    /// Target dash
    /// Repeats the target scenario as a dashed line encoding.
    #[libertas_chart_channel(strokeDash)]
    #[libertas_chart_scale(id = target_dash, kind = ordinal)]
    #[libertas_chart_guide(target = strokeDash, source = scale, position = bottom)]
    pub dash_scenario: ScenarioV1,
    /// Target width
    /// Emphasizes the target line.
    #[libertas_chart_channel(strokeWidth)]
    #[libertas_chart_scale(kind = identity)]
    pub stroke_width: f32,
}

/// Performance target rule
/// A target line sharing the performance story's scales.
#[libertas_chart(rule)]
pub type PerformanceTargetV1 = Vec<PerformanceTargetRuleV1>;

/// Performance story
/// Layers recorded points, a fitted trend, and a target on shared scales.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(layer)]
pub struct PerformanceStoryChartV1 {
    /// Recorded results
    /// Individual performance measurements.
    pub observations: PerformancePointsV1,
    /// Trend
    /// Smoothed movement through the observations.
    pub trend: PerformanceTrendV1,
    /// Target
    /// Desired response-time threshold.
    pub target: PerformanceTargetV1,
}

/// Regional comparison
/// Places grouped quarterly sales beside the traffic-source donut.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(hconcat)]
pub struct RegionalComparisonChartV1 {
    /// Quarterly sales
    /// Grouped sales bars with their own axes.
    pub sales: GroupedSalesChartV1,
    /// Horizontal sales
    /// The same grouping pattern turned horizontally to demonstrate y-offsets.
    pub horizontal_sales: HorizontalGroupedSalesChartV1,
    /// Traffic sources
    /// Donut segments with a legend below the chart.
    pub traffic: TrafficDonutChartV1,
}

/// Operations dashboard
/// Stacks energy history, forecast ranges, and activity intervals vertically.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(vconcat)]
pub struct OperationsDashboardChartV1 {
    /// Energy history
    /// Multi-series UTC trend panel.
    pub energy: EnergyHistoryChartV1,
    /// Forecast range
    /// Confidence-band panel with its own UTC x-axis.
    pub forecast: ForecastRangeChartV1,
    /// Activity heatmap
    /// Time-of-day interval panel with independently labeled axes.
    pub activity: ActivityHeatmapChartV1,
}

/// Latency histogram bin
/// One server-computed latency interval and its observed count.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct LatencyHistogramBinV1 {
    /// Bin start
    /// Inclusive lower latency bound.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = latency_bin, kind = linear, min = 0, zero = true)]
    #[libertas_chart_guide(target = x, source = scale, position = bottom)]
    #[libertas_physical_unit("millisecond")]
    pub starts_at_milliseconds: f32,
    /// Bin end
    /// Exclusive upper latency bound.
    #[libertas_chart_channel(x2, tooltip)]
    #[libertas_physical_unit("millisecond")]
    pub ends_at_milliseconds: f32,
    /// Observations
    /// Number of requests within the interval.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = latency_count, kind = linear, min = 0, zero = true)]
    #[libertas_chart_guide(target = y, source = scale, position = left)]
    pub observations: u32,
    /// Bin key
    /// Stable interval identity.
    #[libertas_chart_channel(key)]
    pub bin_key: String,
}

/// Latency histogram
/// Explicit server-computed latency bins rendered as ranged bars.
#[libertas_chart(bar)]
pub type LatencyHistogramChartV1 = Vec<LatencyHistogramBinV1>;

/// Stacked energy bar segment
/// One cumulative source interval within a quarterly total.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct StackedEnergyBarSegmentV1 {
    /// Quarter
    /// Categorical stack position.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = stacked_quarter, kind = band)]
    #[libertas_chart_guide(target = x, source = scale, position = bottom)]
    pub quarter: QuarterV1,
    /// Cumulative start
    /// Lower bound computed by the server.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = stacked_energy_total, kind = linear, min = 0, zero = true)]
    #[libertas_chart_guide(target = y, source = scale, position = left)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("kilowatt-hour")]
    pub starts_at_kilowatt_hours: f32,
    /// Cumulative end
    /// Upper bound computed by the server.
    #[libertas_chart_channel(y2, tooltip)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("kilowatt-hour")]
    pub ends_at_kilowatt_hours: f32,
    /// Energy source
    /// Fill and grouping identity for the segment.
    #[libertas_chart_channel(fill, detail, tooltip)]
    #[libertas_chart_scale(id = stacked_energy_fill, kind = ordinal)]
    #[libertas_chart_guide(target = fill, source = scale, position = bottom)]
    pub source: EnergySourceV1,
    /// Segment key
    /// Stable quarter and source identity.
    #[libertas_chart_channel(key)]
    pub segment_key: String,
}

/// Explicit stacked energy bars
/// Server-computed cumulative bounds rendered without client-side stacking.
#[libertas_chart(bar)]
pub type StackedEnergyBarsV1 = Vec<StackedEnergyBarSegmentV1>;

/// Stacked energy area point
/// One server-computed lower and upper source boundary over time.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct StackedEnergyAreaPointV1 {
    /// Time
    /// UTC sample time.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = stacked_energy_time, kind = utc)]
    #[libertas_chart_guide(target = x, source = scale, position = bottom)]
    pub measured_at: LibertasDateTime,
    /// Cumulative start
    /// Lower area boundary computed by the server.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = stacked_energy_total, kind = linear, min = 0, zero = true)]
    #[libertas_chart_guide(target = y, source = scale, position = left)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("kilowatt-hour")]
    pub starts_at_kilowatt_hours: f32,
    /// Cumulative end
    /// Upper area boundary computed by the server.
    #[libertas_chart_channel(y2, tooltip)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("kilowatt-hour")]
    pub ends_at_kilowatt_hours: f32,
    /// Energy source
    /// Fill and path-grouping identity.
    #[libertas_chart_channel(fill, detail, tooltip)]
    #[libertas_chart_scale(id = stacked_energy_fill, kind = ordinal)]
    #[libertas_chart_guide(target = fill, source = scale, position = bottom)]
    pub source: EnergySourceV1,
    /// Sequence
    /// Stable order within the source path.
    #[libertas_chart_channel(order)]
    pub sequence: u8,
    /// Point key
    /// Stable source and time identity.
    #[libertas_chart_channel(key)]
    pub point_key: String,
}

/// Explicit stacked energy areas
/// Server-computed cumulative bands over time.
#[libertas_chart(area)]
pub type StackedEnergyAreasV1 = Vec<StackedEnergyAreaPointV1>;

/// Explicit stacking
/// Compares server-computed stacked bars and stacked areas.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(hconcat)]
pub struct StackedEnergyChartV1 {
    /// Stacked bars
    /// Quarterly cumulative source intervals.
    pub bars: StackedEnergyBarsV1,
    /// Stacked areas
    /// Source contributions accumulated over time.
    pub areas: StackedEnergyAreasV1,
}

/// Pie slice
/// One explicit angular slice extending from the center to the outer radius.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct PieSliceV1 {
    /// Slice start
    /// Beginning of the slice in turns.
    #[libertas_chart_channel(theta)]
    #[libertas_chart_scale(id = pie_angle, kind = linear, min = 0, max = 1, zero = true)]
    #[libertas_chart_guide(target = theta, source = none)]
    pub theta_start: f32,
    /// Slice end
    /// End of the slice in turns.
    #[libertas_chart_channel(theta2)]
    pub theta_end: f32,
    /// Inner radius
    /// Zero makes the annular segment a pie slice.
    #[libertas_chart_channel(radius)]
    #[libertas_chart_scale(id = pie_radius, kind = identity)]
    pub radius_start: f32,
    /// Outer radius
    /// Full normalized plot radius.
    #[libertas_chart_channel(radius2)]
    pub radius_end: f32,
    /// Traffic source
    /// Slice fill and grouping identity.
    #[libertas_chart_channel(fill, detail, tooltip)]
    #[libertas_chart_scale(id = traffic_color, kind = ordinal)]
    #[libertas_chart_guide(target = fill, source = scale, position = bottom)]
    pub source: TrafficSourceV1,
    /// Share
    /// Source share shown in the tooltip.
    #[libertas_chart_channel(tooltip)]
    #[libertas_physical_unit("percent")]
    pub share_percent: f32,
    /// Slice key
    /// Stable source identity.
    #[libertas_chart_channel(key)]
    pub slice_key: String,
}

/// Traffic-source pie
/// Explicit pie geometry with a zero inner radius.
#[libertas_chart(arc)]
pub type TrafficPieChartV1 = Vec<PieSliceV1>;

/// Radial performance bar
/// One explicit angular lane and quantitative radial extent.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct RadialPerformanceBarV1 {
    /// Bar start
    /// Beginning of the angular lane in turns.
    #[libertas_chart_channel(theta)]
    #[libertas_chart_scale(id = radial_bar_angle, kind = linear, min = 0, max = 1, zero = true)]
    #[libertas_chart_guide(target = theta, source = none)]
    pub theta_start: f32,
    /// Bar end
    /// End of the angular lane in turns.
    #[libertas_chart_channel(theta2)]
    pub theta_end: f32,
    /// Radial baseline
    /// Zero-percent radial baseline.
    #[libertas_chart_channel(radius)]
    #[libertas_chart_scale(id = radial_bar_score, kind = linear, min = 0, max = 100, zero = true)]
    #[libertas_chart_guide(target = radius, source = scale, position = auto)]
    #[libertas_physical_unit("percent")]
    pub starts_at_percent: f32,
    /// Score
    /// Quantitative radial extent.
    #[libertas_chart_channel(radius2, tooltip)]
    #[libertas_physical_unit("percent")]
    pub ends_at_percent: f32,
    /// Portfolio
    /// Bar fill and grouping identity.
    #[libertas_chart_channel(fill, detail, tooltip)]
    #[libertas_chart_scale(id = portfolio_color, kind = ordinal)]
    #[libertas_chart_guide(target = fill, source = scale, position = bottom)]
    pub portfolio: PortfolioV1,
    /// Bar key
    /// Stable portfolio identity.
    #[libertas_chart_channel(key)]
    pub bar_key: String,
}

/// Radial performance bars
/// Explicit angular lanes with quantitative radial extents.
#[libertas_chart(arc)]
pub type RadialPerformanceChartV1 = Vec<RadialPerformanceBarV1>;

/// Arc-family comparison
/// Places a pie and quantitative radial bars side by side.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(hconcat)]
pub struct ArcFamilyChartV1 {
    /// Traffic pie
    /// Full-radius source shares.
    pub pie: TrafficPieChartV1,
    /// Radial performance
    /// Quantitative radial extents by portfolio.
    pub radial_bars: RadialPerformanceChartV1,
}

/// Forecast estimate point
/// One central estimate layered over a confidence band.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct ForecastEstimatePointV1 {
    /// Time
    /// Local-time forecast position.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = forecast_local_time, kind = time)]
    pub forecast_at: LibertasDateTime,
    /// Estimate
    /// Central forecast estimate.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = temperature_range, kind = linear, zero = false)]
    #[libertas_format("0.#")]
    #[libertas_physical_unit("celsius")]
    pub estimate_celsius: f32,
    /// Forecast model
    /// Line color and path grouping.
    #[libertas_chart_channel(color, detail, tooltip)]
    #[libertas_chart_scale(id = forecast_estimate_color, kind = ordinal)]
    #[libertas_chart_guide(target = color, source = scale, position = bottom)]
    pub model: PortfolioV1,
    /// Sequence
    /// Stable order within the model path.
    #[libertas_chart_channel(order)]
    pub sequence: u8,
    /// Estimate key
    /// Stable model and time identity.
    #[libertas_chart_channel(key)]
    pub estimate_key: String,
}

/// Forecast estimate lines
/// Central estimates sharing the confidence-band scales.
#[libertas_chart(line)]
pub type ForecastEstimateChartV1 = Vec<ForecastEstimatePointV1>;

/// Forecast confidence
/// Layers estimate lines over server-computed confidence bands.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(layer)]
pub struct ForecastConfidenceChartV1 {
    /// Confidence bands
    /// Lower and upper bounds by model.
    pub bands: ForecastRangeChartV1,
    /// Central estimates
    /// Model estimates drawn above their bands.
    pub estimates: ForecastEstimateChartV1,
}

/// Candlestick wick
/// One low-to-high rule with complete market tooltip values.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct CandlestickWickV1 {
    /// Interval midpoint
    /// Center of the candle's time interval.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = candle_time, kind = utc)]
    #[libertas_chart_guide(target = x, source = scale, position = bottom)]
    pub middle_at: LibertasDateTime,
    /// Low
    /// Lowest value in the interval.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = candle_price, kind = linear, zero = false)]
    #[libertas_chart_guide(target = y, source = scale, position = left)]
    #[libertas_format("0.00")]
    pub low: f32,
    /// High
    /// Highest value in the interval.
    #[libertas_chart_channel(y2, tooltip)]
    #[libertas_format("0.00")]
    pub high: f32,
    /// Open
    /// Opening value in the interval.
    #[libertas_chart_channel(tooltip)]
    #[libertas_format("0.00")]
    pub open: f32,
    /// Close
    /// Closing value in the interval.
    #[libertas_chart_channel(tooltip)]
    #[libertas_format("0.00")]
    pub close: f32,
    /// Volume
    /// Transaction volume in the interval.
    #[libertas_chart_channel(tooltip)]
    pub volume: u32,
    /// Direction
    /// Whether the interval rose or fell.
    #[libertas_chart_channel(tooltip)]
    pub direction: CandleDirectionV1,
    /// Wick color
    /// Literal stroke color for the wick.
    #[libertas_chart_channel(stroke)]
    #[libertas_chart_scale(kind = identity)]
    pub wick_color: String,
    /// Candle key
    /// Stable interval identity.
    #[libertas_chart_channel(key)]
    pub candle_key: String,
}

/// Candlestick wicks
/// Low-to-high market ranges.
#[libertas_chart(rule)]
pub type CandlestickWicksV1 = Vec<CandlestickWickV1>;

/// Candlestick body
/// One open-to-close rectangle with explicit time bounds.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct CandlestickBodyV1 {
    /// Interval start
    /// Beginning of the candle's time interval.
    #[libertas_chart_channel(x)]
    #[libertas_chart_scale(id = candle_time, kind = utc)]
    pub starts_at: LibertasDateTime,
    /// Interval end
    /// End of the candle's time interval.
    #[libertas_chart_channel(x2)]
    pub ends_at: LibertasDateTime,
    /// Open
    /// Opening body endpoint.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = candle_price, kind = linear, zero = false)]
    #[libertas_format("0.00")]
    pub open: f32,
    /// Close
    /// Closing body endpoint.
    #[libertas_chart_channel(y2, tooltip)]
    #[libertas_format("0.00")]
    pub close: f32,
    /// Interval midpoint
    /// Time shown in the tooltip.
    #[libertas_chart_channel(tooltip)]
    pub middle_at: LibertasDateTime,
    /// Low
    /// Lowest value in the interval.
    #[libertas_chart_channel(tooltip)]
    #[libertas_format("0.00")]
    pub low: f32,
    /// High
    /// Highest value in the interval.
    #[libertas_chart_channel(tooltip)]
    #[libertas_format("0.00")]
    pub high: f32,
    /// Volume
    /// Transaction volume in the interval.
    #[libertas_chart_channel(tooltip)]
    pub volume: u32,
    /// Direction
    /// Whether the interval rose or fell.
    #[libertas_chart_channel(tooltip)]
    pub direction: CandleDirectionV1,
    /// Body color
    /// Literal fill color for the body.
    #[libertas_chart_channel(fill)]
    #[libertas_chart_scale(kind = identity)]
    pub body_color: String,
    /// Candle key
    /// Stable interval identity.
    #[libertas_chart_channel(key)]
    pub candle_key: String,
}

/// Candlestick bodies
/// Open-to-close rectangles with server-computed widths.
#[libertas_chart(rect)]
pub type CandlestickBodiesV1 = Vec<CandlestickBodyV1>;

/// Candlestick chart
/// Layers low-high wicks and open-close bodies on shared scales.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(layer)]
pub struct CandlestickChartV1 {
    /// Wicks
    /// Low-to-high interval rules.
    pub wicks: CandlestickWicksV1,
    /// Bodies
    /// Open-to-close interval rectangles.
    pub bodies: CandlestickBodiesV1,
}

/// Box-plot body
/// Interquartile range for one sample group.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BoxPlotBodyV1 {
    /// Sample
    /// Categorical sample group.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = box_sample, kind = band)]
    #[libertas_chart_guide(target = x, source = scale, position = bottom)]
    pub sample: QuarterV1,
    /// First quartile
    /// Lower body endpoint.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = box_latency, kind = linear, zero = false)]
    #[libertas_chart_guide(target = y, source = scale, position = left)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("millisecond")]
    pub first_quartile_milliseconds: f32,
    /// Third quartile
    /// Upper body endpoint.
    #[libertas_chart_channel(y2, tooltip)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("millisecond")]
    pub third_quartile_milliseconds: f32,
    /// Portfolio
    /// Body fill category.
    #[libertas_chart_channel(fill, tooltip)]
    #[libertas_chart_scale(id = box_fill, kind = ordinal)]
    #[libertas_chart_guide(target = fill, source = scale, position = bottom)]
    pub portfolio: PortfolioV1,
    /// Body key
    /// Stable sample identity.
    #[libertas_chart_channel(key)]
    pub body_key: String,
}

/// Box-plot bodies
/// Interquartile ranges rendered as rectangles.
#[libertas_chart(rect)]
pub type BoxPlotBodiesV1 = Vec<BoxPlotBodyV1>;

/// Box-plot whisker
/// Low-to-high range for one sample group.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BoxPlotWhiskerV1 {
    /// Sample
    /// Categorical sample group.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = box_sample, kind = band)]
    pub sample: QuarterV1,
    /// Low whisker
    /// Lowest non-outlier value.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = box_latency, kind = linear, zero = false)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("millisecond")]
    pub low_milliseconds: f32,
    /// High whisker
    /// Highest non-outlier value.
    #[libertas_chart_channel(y2, tooltip)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("millisecond")]
    pub high_milliseconds: f32,
    /// Whisker color
    /// Literal stroke color.
    #[libertas_chart_channel(stroke)]
    #[libertas_chart_scale(kind = identity)]
    pub whisker_color: String,
    /// Whisker key
    /// Stable sample identity.
    #[libertas_chart_channel(key)]
    pub whisker_key: String,
}

/// Box-plot whiskers
/// Low-to-high ranges rendered as rules.
#[libertas_chart(rule)]
pub type BoxPlotWhiskersV1 = Vec<BoxPlotWhiskerV1>;

/// Box-plot median
/// Zero-height rectangle retaining a visible one-pixel median.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BoxPlotMedianV1 {
    /// Sample
    /// Categorical sample group.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = box_sample, kind = band)]
    pub sample: QuarterV1,
    /// Median start
    /// Median lower endpoint.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = box_latency, kind = linear, zero = false)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("millisecond")]
    pub starts_at_milliseconds: f32,
    /// Median end
    /// Equal median upper endpoint.
    #[libertas_chart_channel(y2)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("millisecond")]
    pub ends_at_milliseconds: f32,
    /// Median color
    /// Literal fill color.
    #[libertas_chart_channel(fill)]
    #[libertas_chart_scale(kind = identity)]
    pub median_color: String,
    /// Median key
    /// Stable sample identity.
    #[libertas_chart_channel(key)]
    pub median_key: String,
}

/// Box-plot medians
/// One visible zero-height rectangle per sample.
#[libertas_chart(rect)]
pub type BoxPlotMediansV1 = Vec<BoxPlotMedianV1>;

/// Box-plot outlier
/// One value outside the server-computed whiskers.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BoxPlotOutlierV1 {
    /// Sample
    /// Categorical sample group.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = box_sample, kind = band)]
    pub sample: QuarterV1,
    /// Latency
    /// Outlying measurement.
    #[libertas_chart_channel(y, tooltip)]
    #[libertas_chart_scale(id = box_latency, kind = linear, zero = false)]
    #[libertas_format("0.0")]
    #[libertas_physical_unit("millisecond")]
    pub latency_milliseconds: f32,
    /// Point size
    /// Literal outlier radius.
    #[libertas_chart_channel(size)]
    #[libertas_chart_scale(kind = identity)]
    pub point_size: f32,
    /// Outlier key
    /// Stable outlier identity.
    #[libertas_chart_channel(key)]
    pub outlier_key: String,
}

/// Box-plot outliers
/// Individual outlying measurements.
#[libertas_chart(point)]
pub type BoxPlotOutliersV1 = Vec<BoxPlotOutlierV1>;

/// Box plot
/// Layers whiskers, interquartile bodies, medians, and outliers.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(layer)]
pub struct BoxPlotChartV1 {
    /// Whiskers
    /// Low-to-high non-outlier ranges.
    pub whiskers: BoxPlotWhiskersV1,
    /// Bodies
    /// First-to-third-quartile ranges.
    pub bodies: BoxPlotBodiesV1,
    /// Medians
    /// Zero-height median rectangles.
    pub medians: BoxPlotMediansV1,
    /// Outliers
    /// Measurements beyond the whiskers.
    pub outliers: BoxPlotOutliersV1,
}

/// Projected map vertex
/// One server-projected vertex in a filled region.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct ProjectedMapVertexV1 {
    /// Projected x
    /// Server-computed horizontal coordinate.
    #[libertas_chart_channel(x)]
    #[libertas_chart_scale(id = projected_map_x, kind = linear, min = 0, max = 100)]
    #[libertas_chart_guide(target = x, source = none)]
    pub x: f32,
    /// Projected y
    /// Server-computed vertical coordinate.
    #[libertas_chart_channel(y)]
    #[libertas_chart_scale(id = projected_map_y, kind = linear, min = 0, max = 100)]
    #[libertas_chart_guide(target = y, source = none)]
    pub y: f32,
    /// Region
    /// Fill, polygon grouping, and tooltip identity.
    #[libertas_chart_channel(fill, detail, tooltip)]
    #[libertas_chart_scale(id = map_region_fill, kind = ordinal)]
    #[libertas_chart_guide(target = fill, source = scale, position = bottom)]
    pub region: MapRegionV1,
    /// Utilization
    /// Region utilization shown in the tooltip.
    #[libertas_chart_channel(tooltip)]
    #[libertas_physical_unit("percent")]
    pub utilization_percent: f32,
    /// Vertex order
    /// Stable order within the region boundary.
    #[libertas_chart_channel(order)]
    pub sequence: u8,
    /// Vertex key
    /// Stable region and vertex identity.
    #[libertas_chart_channel(key)]
    pub vertex_key: String,
}

/// Server-projected map
/// Filled Cartesian polygons whose projection is owned by the endpoint.
#[libertas_chart(polygon)]
pub type ProjectedMapChartV1 = Vec<ProjectedMapVertexV1>;

/// Network edge
/// One arbitrary server-positioned link between two nodes.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct NetworkEdgeV1 {
    /// Source x
    /// Horizontal source coordinate.
    #[libertas_chart_channel(x)]
    #[libertas_chart_scale(id = network_x, kind = linear, min = 0, max = 100)]
    #[libertas_chart_guide(target = x, source = none)]
    pub source_x: f32,
    /// Target x
    /// Horizontal target coordinate.
    #[libertas_chart_channel(x2)]
    pub target_x: f32,
    /// Source y
    /// Vertical source coordinate.
    #[libertas_chart_channel(y)]
    #[libertas_chart_scale(id = network_y, kind = linear, min = 0, max = 100)]
    #[libertas_chart_guide(target = y, source = none)]
    pub source_y: f32,
    /// Target y
    /// Vertical target coordinate.
    #[libertas_chart_channel(y2)]
    pub target_y: f32,
    /// Connection
    /// Localized fixed relationship shown in the tooltip.
    #[libertas_chart_channel(tooltip)]
    pub connection: NetworkConnectionV1,
    /// Link color
    /// Literal edge stroke.
    #[libertas_chart_channel(stroke)]
    #[libertas_chart_scale(kind = identity)]
    pub link_color: String,
    /// Link width
    /// Literal edge width.
    #[libertas_chart_channel(strokeWidth)]
    #[libertas_chart_scale(kind = identity)]
    pub link_width: f32,
    /// Edge key
    /// Stable link identity.
    #[libertas_chart_channel(key)]
    pub edge_key: String,
}

/// Network edges
/// Arbitrary two-endpoint rules connecting nodes.
#[libertas_chart(rule)]
pub type NetworkEdgesV1 = Vec<NetworkEdgeV1>;

/// Network node
/// One server-positioned topology node.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct NetworkNodeV1 {
    /// Node x
    /// Horizontal node coordinate.
    #[libertas_chart_channel(x)]
    #[libertas_chart_scale(id = network_x, kind = linear, min = 0, max = 100)]
    pub x: f32,
    /// Node y
    /// Vertical node coordinate.
    #[libertas_chart_channel(y)]
    #[libertas_chart_scale(id = network_y, kind = linear, min = 0, max = 100)]
    pub y: f32,
    /// Node
    /// Localized node label and stable interaction key.
    #[libertas_chart_channel(tooltip, key)]
    pub node: NetworkNodeNameV1,
    /// Role color
    /// Node fill category.
    #[libertas_chart_channel(color, tooltip)]
    #[libertas_chart_scale(id = network_role_color, kind = ordinal)]
    #[libertas_chart_guide(target = color, source = scale, position = bottom)]
    pub role_color: NetworkRoleV1,
    /// Role shape
    /// Node symbol category.
    #[libertas_chart_channel(shape)]
    #[libertas_chart_scale(id = network_role_shape, kind = ordinal)]
    pub role_shape: NetworkRoleV1,
    /// Node size
    /// Literal node radius.
    #[libertas_chart_channel(size)]
    #[libertas_chart_scale(kind = identity)]
    pub node_size: f32,
}

/// Network nodes
/// Server-positioned and typed topology nodes.
#[libertas_chart(point)]
pub type NetworkNodesV1 = Vec<NetworkNodeV1>;

/// Network topology
/// Layers arbitrary link rules below positioned node symbols.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(layer)]
pub struct NetworkTopologyChartV1 {
    /// Edges
    /// Server-computed node connections.
    pub edges: NetworkEdgesV1,
    /// Nodes
    /// Server-computed node positions.
    pub nodes: NetworkNodesV1,
}

/// Sankey ribbon point
/// One sampled lower and upper boundary along a flow ribbon.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SankeyRibbonPointV1 {
    /// Flow position
    /// Monotonic sampled horizontal position.
    #[libertas_chart_channel(x, tooltip)]
    #[libertas_chart_scale(id = sankey_x, kind = linear, min = 0, max = 100)]
    #[libertas_chart_guide(target = x, source = none)]
    pub x: f32,
    /// Lower boundary
    /// Server-sampled lower ribbon edge.
    #[libertas_chart_channel(y)]
    #[libertas_chart_scale(id = sankey_y, kind = linear, min = 0, max = 100)]
    #[libertas_chart_guide(target = y, source = none)]
    pub lower_y: f32,
    /// Upper boundary
    /// Server-sampled upper ribbon edge.
    #[libertas_chart_channel(y2)]
    pub upper_y: f32,
    /// Stream
    /// Ribbon fill, path grouping, and tooltip identity.
    #[libertas_chart_channel(fill, detail, tooltip)]
    #[libertas_chart_scale(id = sankey_stream_fill, kind = ordinal)]
    #[libertas_chart_guide(target = fill, source = scale, position = bottom)]
    pub stream: FlowStreamV1,
    /// Visitors
    /// Flow quantity shown in the tooltip.
    #[libertas_chart_channel(tooltip)]
    pub visitors: u32,
    /// Ribbon opacity
    /// Literal alpha preserving overlaps.
    #[libertas_chart_channel(fillOpacity)]
    #[libertas_chart_scale(kind = identity)]
    pub fill_opacity: f32,
    /// Sample order
    /// Stable order within the ribbon path.
    #[libertas_chart_channel(order)]
    pub sequence: u8,
    /// Sample key
    /// Stable stream and sample identity.
    #[libertas_chart_channel(key)]
    pub sample_key: String,
}

/// Sankey ribbons
/// Server-sampled x-monotonic flow areas.
#[libertas_chart(area)]
pub type SankeyRibbonsV1 = Vec<SankeyRibbonPointV1>;

/// Sankey node
/// One explicit rectangular flow stage.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SankeyNodeV1 {
    /// Node start x
    /// Left node boundary.
    #[libertas_chart_channel(x)]
    #[libertas_chart_scale(id = sankey_x, kind = linear, min = 0, max = 100)]
    pub starts_at_x: f32,
    /// Node end x
    /// Right node boundary.
    #[libertas_chart_channel(x2)]
    pub ends_at_x: f32,
    /// Node start y
    /// Lower node boundary.
    #[libertas_chart_channel(y)]
    #[libertas_chart_scale(id = sankey_y, kind = linear, min = 0, max = 100)]
    pub starts_at_y: f32,
    /// Node end y
    /// Upper node boundary.
    #[libertas_chart_channel(y2)]
    pub ends_at_y: f32,
    /// Stage
    /// Localized flow-stage label.
    #[libertas_chart_channel(tooltip)]
    pub stage: FlowStageV1,
    /// Visitors
    /// Quantity represented by the node.
    #[libertas_chart_channel(tooltip)]
    pub visitors: u32,
    /// Node color
    /// Literal fill color.
    #[libertas_chart_channel(fill)]
    #[libertas_chart_scale(kind = identity)]
    pub node_color: String,
    /// Node key
    /// Stable stage identity.
    #[libertas_chart_channel(key)]
    pub node_key: String,
}

/// Sankey nodes
/// Explicit rectangular flow stages.
#[libertas_chart(rect)]
pub type SankeyNodesV1 = Vec<SankeyNodeV1>;

/// Sankey flow
/// Layers server-sampled ribbons below explicit stage nodes.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart(layer)]
pub struct SankeyFlowChartV1 {
    /// Ribbons
    /// Sampled flow paths.
    pub ribbons: SankeyRibbonsV1,
    /// Nodes
    /// Explicit stage rectangles.
    pub nodes: SankeyNodesV1,
}

/// Chart gallery
/// Request any gallery entry to explore a complete Libertas chart response.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[allow(clippy::large_enum_variant)]
pub enum ChartDemoProtocol {
    /// View bubble portfolio
    /// Explore a faceted bubble chart with rich point encodings.
    #[libertas_request]
    #[libertas_next_response(BubblePortfolioV1)]
    GetBubblePortfolioV1,
    /// Bubble portfolio
    /// Product reach, growth, value, confidence, family, tier, and market.
    #[libertas_response]
    #[libertas_chart(point)]
    BubblePortfolioV1(BubblePortfolioChartV1),

    /// View energy history
    /// Explore multi-series UTC lines and data-driven stroke styling.
    #[libertas_request]
    #[libertas_next_response(EnergyHistoryV1)]
    GetEnergyHistoryV1,
    /// Energy history
    /// Recorded, expected, and stretch energy demand.
    #[libertas_response]
    #[libertas_chart(line)]
    EnergyHistoryV1(EnergyHistoryChartV1),

    /// View forecast range
    /// Explore ranged area marks as overlapping confidence bands.
    #[libertas_request]
    #[libertas_next_response(ForecastRangeV1)]
    GetForecastRangeV1,
    /// Forecast range
    /// Temperature uncertainty from two forecast models.
    #[libertas_response]
    #[libertas_chart(area)]
    ForecastRangeV1(ForecastRangeChartV1),

    /// View grouped sales
    /// Explore categorical bars with nested offsets.
    #[libertas_request]
    #[libertas_next_response(GroupedSalesV1)]
    GetGroupedSalesV1,
    /// Grouped sales
    /// Quarterly portfolio revenue shown side by side.
    #[libertas_response]
    #[libertas_chart(bar)]
    GroupedSalesV1(GroupedSalesChartV1),

    /// View latency histogram
    /// Explore server-computed numeric bins as ranged bars.
    #[libertas_request]
    #[libertas_next_response(LatencyHistogramV1)]
    GetLatencyHistogramV1,
    /// Latency histogram
    /// Request counts within explicit latency intervals.
    #[libertas_response]
    #[libertas_chart(bar)]
    LatencyHistogramV1(LatencyHistogramChartV1),

    /// View explicit stacking
    /// Compare server-computed stacked bars and stacked areas.
    #[libertas_request]
    #[libertas_next_response(StackedEnergyV1)]
    GetStackedEnergyV1,
    /// Explicit stacking
    /// Cumulative energy bounds shown as bars and areas.
    #[libertas_response]
    #[libertas_chart(hconcat)]
    StackedEnergyV1(StackedEnergyChartV1),

    /// View activity heatmap
    /// Explore ranged rectangles, time scales, quantitative color, and custom labels.
    #[libertas_request]
    #[libertas_next_response(ActivityHeatmapV1)]
    GetActivityHeatmapV1,
    /// Activity heatmap
    /// Weekday and time-of-day activity intensity.
    #[libertas_response]
    #[libertas_chart(rect)]
    ActivityHeatmapV1(ActivityHeatmapChartV1),

    /// View measurement uncertainty
    /// Explore ranged rules as error bars.
    #[libertas_request]
    #[libertas_next_response(MeasurementUncertaintyV1)]
    GetMeasurementUncertaintyV1,
    /// Measurement uncertainty
    /// Low-to-high electrical measurement intervals.
    #[libertas_response]
    #[libertas_chart(rule)]
    MeasurementUncertaintyV1(MeasurementUncertaintyChartV1),

    /// View direct labels
    /// Explore localized LMF1 text content and data-driven styling.
    #[libertas_request]
    #[libertas_next_response(DirectLabelsV1)]
    GetDirectLabelsV1,
    /// Direct labels
    /// Positioned FormattedText annotations with semantic appearance.
    #[libertas_response]
    #[libertas_chart(text)]
    DirectLabelsV1(DirectLabelsChartV1),

    /// View capability radar
    /// Explore ordered polar polygons.
    #[libertas_request]
    #[libertas_next_response(CapabilityRadarV1)]
    GetCapabilityRadarV1,
    /// Capability radar
    /// Overlapping multidimensional portfolio profiles.
    #[libertas_response]
    #[libertas_chart(polygon)]
    CapabilityRadarV1(CapabilityRadarChartV1),

    /// View traffic sources
    /// Explore explicit angular and radial spans in a donut chart.
    #[libertas_request]
    #[libertas_next_response(TrafficSourcesV1)]
    GetTrafficSourcesV1,
    /// Traffic sources
    /// Traffic share by referral source with automatic linked outside labels.
    #[libertas_response]
    #[libertas_chart(arc)]
    TrafficSourcesV1(TrafficDonutChartV1),

    /// View arc-family comparison
    /// Explore a full-radius pie and quantitative radial bars.
    #[libertas_request]
    #[libertas_next_response(ArcFamilyV1)]
    GetArcFamilyV1,
    /// Arc-family comparison
    /// Pie slices and radial bars in independent polar plots.
    #[libertas_response]
    #[libertas_chart(hconcat)]
    ArcFamilyV1(ArcFamilyChartV1),

    /// View projected map
    /// Explore server-projected Cartesian region polygons.
    #[libertas_request]
    #[libertas_next_response(ProjectedMapV1)]
    GetProjectedMapV1,
    /// Projected map
    /// Filled regions using server-computed coordinates.
    #[libertas_response]
    #[libertas_chart(polygon)]
    ProjectedMapV1(ProjectedMapChartV1),

    /// View forecast confidence
    /// Explore estimate lines layered over confidence bands.
    #[libertas_request]
    #[libertas_next_response(ForecastConfidenceV1)]
    GetForecastConfidenceV1,
    /// Forecast confidence
    /// Central estimates and uncertainty bands on shared scales.
    #[libertas_response]
    #[libertas_chart(layer)]
    ForecastConfidenceV1(ForecastConfidenceChartV1),

    /// View candlestick chart
    /// Explore market wicks and open-close bodies.
    #[libertas_request]
    #[libertas_next_response(CandlestickV1)]
    GetCandlestickV1,
    /// Candlestick chart
    /// Low-high wicks and rising or falling open-close bodies.
    #[libertas_response]
    #[libertas_chart(layer)]
    CandlestickV1(CandlestickChartV1),

    /// View five-minute temperature HLC
    /// Explore a seeded high-low-close temperature random walk over the past 72 hours.
    #[libertas_request]
    #[libertas_next_response(TemperatureHlcV1)]
    GetTemperatureHlcV1,
    /// Five-minute temperature HLC, past 72 hours
    /// Completed five-minute high, low, and close observations from a reproducible random walk.
    #[libertas_response]
    #[libertas_chart(layer)]
    TemperatureHlcV1(TemperatureHlcChartV1),

    /// View box plot
    /// Explore quartiles, whiskers, medians, and outliers.
    #[libertas_request]
    #[libertas_next_response(BoxPlotV1)]
    GetBoxPlotV1,
    /// Box plot
    /// Server-computed distribution summaries composed from primitive marks.
    #[libertas_response]
    #[libertas_chart(layer)]
    BoxPlotV1(BoxPlotChartV1),

    /// View network topology
    /// Explore arbitrary link rules and positioned nodes.
    #[libertas_request]
    #[libertas_next_response(NetworkTopologyV1)]
    GetNetworkTopologyV1,
    /// Network topology
    /// Server-positioned nodes connected by explicit endpoints.
    #[libertas_response]
    #[libertas_chart(layer)]
    NetworkTopologyV1(NetworkTopologyChartV1),

    /// View Sankey flow
    /// Explore server-sampled ribbons and explicit stage nodes.
    #[libertas_request]
    #[libertas_next_response(SankeyFlowV1)]
    GetSankeyFlowV1,
    /// Sankey flow
    /// Sampled flow areas layered below rectangular nodes.
    #[libertas_response]
    #[libertas_chart(layer)]
    SankeyFlowV1(SankeyFlowChartV1),

    /// View performance story
    /// Explore a layered chart with observations, trend, and target.
    #[libertas_request]
    #[libertas_next_response(PerformanceStoryV1)]
    GetPerformanceStoryV1,
    /// Performance story
    /// Points, trend line, and target rule on shared scales.
    #[libertas_response]
    #[libertas_chart(layer)]
    PerformanceStoryV1(PerformanceStoryChartV1),

    /// View regional comparison
    /// Explore independent charts placed side by side.
    #[libertas_request]
    #[libertas_next_response(RegionalComparisonV1)]
    GetRegionalComparisonV1,
    /// Regional comparison
    /// Quarterly sales and traffic sources in a horizontal composition.
    #[libertas_response]
    #[libertas_chart(hconcat)]
    RegionalComparisonV1(RegionalComparisonChartV1),

    /// View operations dashboard
    /// Explore independently guided panels stacked into a dashboard.
    #[libertas_request]
    #[libertas_next_response(OperationsDashboardV1)]
    GetOperationsDashboardV1,
    /// Operations dashboard
    /// Energy, forecast confidence, and activity panels in a vertical composition.
    #[libertas_response]
    #[libertas_chart(vconcat)]
    OperationsDashboardV1(OperationsDashboardChartV1),
}

const BASE_TIME: LibertasDateTime = 1_787_616_000;
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
const HOUR: u64 = 3_600;
const FIVE_MINUTES: u64 = 5 * 60;
const TEMPERATURE_BUCKETS: usize = 72 * 12;
const TEMPERATURE_WALK_START_CELSIUS: f32 = 20.0;
const TEMPERATURE_WALK_MIN_CELSIUS: f32 = 12.0;
const TEMPERATURE_WALK_MAX_CELSIUS: f32 = 28.0;
const TEMPERATURE_WALK_MAX_STEP_CELSIUS: f32 = 0.2;

fn utc_seconds_or_base_time(utc_microseconds: Option<u64>) -> LibertasDateTime {
    utc_microseconds
        .map(|microseconds| microseconds / MICROSECONDS_PER_SECOND)
        .unwrap_or(BASE_TIME)
}

fn text(value: &str) -> String {
    String::from(value)
}

fn next_temperature_random(random_state: &mut u32) -> u32 {
    *random_state = random_state
        .wrapping_mul(1_664_525)
        .wrapping_add(1_013_904_223);
    *random_state
}

fn bubble_portfolio() -> BubblePortfolioChartV1 {
    vec![
        BubblePointV1 {
            product: text("Nest"),
            active_customers: 1_200,
            annual_growth_percent: 18.0,
            annual_data_terabytes: 8.4,
            portfolio: PortfolioV1::Home,
            tier: CustomerTierV1::Premium,
            confidence_opacity: 0.94,
            confidence_percent: 94,
            market: MarketV1::Americas,
            stage: ReleaseStageV1::Established,
        },
        BubblePointV1 {
            product: text("Beam"),
            active_customers: 8_500,
            annual_growth_percent: -2.5,
            annual_data_terabytes: 21.0,
            portfolio: PortfolioV1::Business,
            tier: CustomerTierV1::Standard,
            confidence_opacity: 0.78,
            confidence_percent: 78,
            market: MarketV1::Americas,
            stage: ReleaseStageV1::Established,
        },
        BubblePointV1 {
            product: text("Atlas"),
            active_customers: 42_000,
            annual_growth_percent: 31.0,
            annual_data_terabytes: 52.0,
            portfolio: PortfolioV1::Platform,
            tier: CustomerTierV1::Premium,
            confidence_opacity: 0.88,
            confidence_percent: 88,
            market: MarketV1::Americas,
            stage: ReleaseStageV1::Emerging,
        },
        BubblePointV1 {
            product: text("Haven"),
            active_customers: 2_600,
            annual_growth_percent: 12.0,
            annual_data_terabytes: 10.2,
            portfolio: PortfolioV1::Home,
            tier: CustomerTierV1::Standard,
            confidence_opacity: 0.91,
            confidence_percent: 91,
            market: MarketV1::Europe,
            stage: ReleaseStageV1::Established,
        },
        BubblePointV1 {
            product: text("Relay"),
            active_customers: 15_000,
            annual_growth_percent: -8.0,
            annual_data_terabytes: 30.5,
            portfolio: PortfolioV1::Business,
            tier: CustomerTierV1::Premium,
            confidence_opacity: 0.73,
            confidence_percent: 73,
            market: MarketV1::Europe,
            stage: ReleaseStageV1::Established,
        },
        BubblePointV1 {
            product: text("Orbit"),
            active_customers: 73_000,
            annual_growth_percent: 22.0,
            annual_data_terabytes: 67.0,
            portfolio: PortfolioV1::Platform,
            tier: CustomerTierV1::Premium,
            confidence_opacity: 0.86,
            confidence_percent: 86,
            market: MarketV1::Europe,
            stage: ReleaseStageV1::Emerging,
        },
    ]
}

fn energy_history() -> EnergyHistoryChartV1 {
    let recorded = [64.0, 58.0, 61.0, 55.0, 52.0, 49.0];
    let expected = [66.0, 62.0, 59.0, 56.0, 54.0, 51.0];
    let stretch = [63.0, 58.0, 54.0, 49.0, 45.0, 41.0];
    let mut rows = Vec::new();
    for (scenario, values, width) in [
        (ScenarioV1::Recorded, recorded, 4.0),
        (ScenarioV1::Expected, expected, 3.0),
        (ScenarioV1::Stretch, stretch, 2.0),
    ] {
        for (sequence, value) in values.into_iter().enumerate() {
            rows.push(EnergyHistoryPointV1 {
                energy_at: BASE_TIME + sequence as u64 * HOUR,
                power_kilowatts: value,
                scenario,
                dash_scenario: scenario,
                stroke_width: width,
                sequence: sequence as u8,
            });
        }
    }
    rows
}

fn forecast_range() -> ForecastRangeChartV1 {
    let center = [22.0, 23.5, 25.0, 24.0, 22.5, 21.0];
    let mut rows = Vec::new();
    for (model, spread, opacity) in [
        (PortfolioV1::Platform, 1.5, 0.35),
        (PortfolioV1::Business, 2.4, 0.20),
    ] {
        for (sequence, value) in center.into_iter().enumerate() {
            rows.push(ForecastRangePointV1 {
                forecast_at: BASE_TIME + sequence as u64 * HOUR,
                low_celsius: value - spread,
                high_celsius: value + spread,
                model,
                outline_model: model,
                confidence_opacity: opacity,
            });
        }
    }
    rows
}

fn capability_radar() -> CapabilityRadarChartV1 {
    let capabilities = [
        CapabilityV1::Speed,
        CapabilityV1::Reliability,
        CapabilityV1::Efficiency,
        CapabilityV1::Simplicity,
        CapabilityV1::Reach,
        CapabilityV1::Speed,
    ];
    let mut rows = Vec::new();
    for (portfolio, scores) in [
        (PortfolioV1::Home, [84, 92, 76, 95, 68, 84]),
        (PortfolioV1::Platform, [96, 88, 91, 72, 98, 96]),
    ] {
        for (sequence, (capability, score)) in capabilities.into_iter().zip(scores).enumerate() {
            rows.push(CapabilityRadarPointV1 {
                angle_turn: sequence as f32 / 5.0,
                capability,
                score_percent: score,
                portfolio,
                outline_portfolio: portfolio,
                fill_opacity: 0.25,
                sequence: sequence as u8,
                vertex_key: text(match (portfolio, sequence) {
                    (PortfolioV1::Home, 0) => "home-speed",
                    (PortfolioV1::Home, 1) => "home-reliability",
                    (PortfolioV1::Home, 2) => "home-efficiency",
                    (PortfolioV1::Home, 3) => "home-simplicity",
                    (PortfolioV1::Home, 4) => "home-reach",
                    (PortfolioV1::Home, _) => "home-close",
                    (PortfolioV1::Platform, 0) => "platform-speed",
                    (PortfolioV1::Platform, 1) => "platform-reliability",
                    (PortfolioV1::Platform, 2) => "platform-efficiency",
                    (PortfolioV1::Platform, 3) => "platform-simplicity",
                    (PortfolioV1::Platform, 4) => "platform-reach",
                    (PortfolioV1::Platform, _) => "platform-close",
                    _ => "radar-point",
                }),
            });
        }
    }
    rows
}

fn grouped_sales() -> GroupedSalesChartV1 {
    let quarters = [QuarterV1::Q1, QuarterV1::Q2, QuarterV1::Q3, QuarterV1::Q4];
    let mut rows = Vec::new();
    for (portfolio, values, prefix) in [
        (PortfolioV1::Home, [12.0, 14.5, 16.0, 18.5], "home"),
        (PortfolioV1::Business, [18.0, 17.0, 21.5, 24.0], "business"),
        (PortfolioV1::Platform, [9.0, 13.0, 19.0, 27.0], "platform"),
    ] {
        for (index, (quarter, revenue)) in quarters.into_iter().zip(values).enumerate() {
            let suffix = match index {
                0 => "q1",
                1 => "q2",
                2 => "q3",
                _ => "q4",
            };
            let mut key = text(prefix);
            key.push('-');
            key.push_str(suffix);
            rows.push(GroupedSalesBarV1 {
                quarter,
                portfolio,
                portfolio_color: portfolio,
                revenue_millions: revenue,
                bar_key: key,
            });
        }
    }
    rows
}

fn horizontal_grouped_sales() -> HorizontalGroupedSalesChartV1 {
    grouped_sales()
        .into_iter()
        .map(|row| HorizontalGroupedBarV1 {
            revenue_millions: row.revenue_millions,
            quarter: row.quarter,
            portfolio_offset: row.portfolio,
            portfolio_color: row.portfolio,
            bar_key: row.bar_key,
        })
        .collect()
}

fn activity_heatmap() -> ActivityHeatmapChartV1 {
    let weekdays = [
        WeekdayV1::Monday,
        WeekdayV1::Tuesday,
        WeekdayV1::Wednesday,
        WeekdayV1::Thursday,
        WeekdayV1::Friday,
    ];
    let activity = [
        [22, 48, 86, 64],
        [18, 55, 92, 70],
        [30, 68, 96, 74],
        [26, 62, 88, 67],
        [16, 41, 72, 38],
    ];
    let mut rows = Vec::new();
    for (day_index, (weekday, day_activity)) in weekdays.into_iter().zip(activity).enumerate() {
        for (block, activity_percent) in day_activity.into_iter().enumerate() {
            let start_hour = 8 + block * 3;
            let mut key = text("cell-");
            key.push(char::from(b'0' + day_index as u8));
            key.push('-');
            key.push(char::from(b'0' + block as u8));
            rows.push(ActivityHeatmapCellV1 {
                starts_at: start_hour as u32 * HOUR as u32,
                ends_at: (start_hour + 3) as u32 * HOUR as u32,
                time_block: match block {
                    0 => TimeBlockV1::Morning,
                    1 => TimeBlockV1::Midday,
                    2 => TimeBlockV1::Afternoon,
                    _ => TimeBlockV1::Evening,
                },
                day_start: day_index as f32,
                day_end: day_index as f32 + 0.9,
                weekday,
                activity_percent,
                cell_key: key,
            });
        }
    }
    rows
}

fn measurement_uncertainty() -> MeasurementUncertaintyChartV1 {
    vec![
        UncertaintyRuleV1 {
            sample: QuarterV1::Q1,
            low_volts: 4.82,
            high_volts: 5.14,
            scenario: ScenarioV1::Recorded,
            dash_scenario: ScenarioV1::Recorded,
            stroke_width: 4.0,
            rule_key: text("q1-recorded"),
        },
        UncertaintyRuleV1 {
            sample: QuarterV1::Q2,
            low_volts: 4.91,
            high_volts: 5.09,
            scenario: ScenarioV1::Recorded,
            dash_scenario: ScenarioV1::Recorded,
            stroke_width: 4.0,
            rule_key: text("q2-recorded"),
        },
        UncertaintyRuleV1 {
            sample: QuarterV1::Q3,
            low_volts: 4.76,
            high_volts: 5.18,
            scenario: ScenarioV1::Expected,
            dash_scenario: ScenarioV1::Expected,
            stroke_width: 3.0,
            rule_key: text("q3-expected"),
        },
        UncertaintyRuleV1 {
            sample: QuarterV1::Q4,
            low_volts: 4.88,
            high_volts: 5.12,
            scenario: ScenarioV1::Expected,
            dash_scenario: ScenarioV1::Expected,
            stroke_width: 3.0,
            rule_key: text("q4-expected"),
        },
    ]
}

fn traffic_sources() -> TrafficDonutChartV1 {
    let segments = [
        (TrafficSourceV1::Direct, 0.0, 0.34, 34.0, "direct"),
        (TrafficSourceV1::Search, 0.34, 0.65, 31.0, "search"),
        (TrafficSourceV1::Partner, 0.65, 0.84, 19.0, "partner"),
        (TrafficSourceV1::Campaign, 0.84, 0.97, 13.0, "campaign"),
        (TrafficSourceV1::Social, 0.97, 0.99, 2.0, "social"),
        (TrafficSourceV1::Other, 0.99, 1.0, 1.0, "other"),
    ];
    segments
        .into_iter()
        .enumerate()
        .map(
            |(sequence, (source, theta_start, theta_end, share_percent, key))| {
                TrafficDonutSegmentV1 {
                    theta_start,
                    theta_end,
                    radius_start: 0.55,
                    radius_end: 1.0,
                    source,
                    share_percent,
                    border_opacity: 0.9,
                    sequence: sequence as u8,
                    segment_key: text(key),
                }
            },
        )
        .collect()
}

fn temperature_hlc(now: LibertasDateTime) -> TemperatureHlcChartV1 {
    let ends_at = now / FIVE_MINUTES * FIVE_MINUTES;
    let starts_at = ends_at.saturating_sub(TEMPERATURE_BUCKETS as u64 * FIVE_MINUTES);
    let mut ranges = Vec::with_capacity(TEMPERATURE_BUCKETS);
    let mut closes = Vec::with_capacity(TEMPERATURE_BUCKETS);
    let mut random_state = (starts_at as u32) ^ ((starts_at >> u32::BITS) as u32) ^ 0x6d2b_79f5;
    let mut previous_close_celsius = TEMPERATURE_WALK_START_CELSIUS;

    for sequence in 0..TEMPERATURE_BUCKETS {
        let bucket_starts_at = starts_at + sequence as u64 * FIVE_MINUTES;
        let middle_at = bucket_starts_at + FIVE_MINUTES / 2;
        let bucket_ends_at = bucket_starts_at + FIVE_MINUTES;
        let step = (next_temperature_random(&mut random_state) % 9) as i8 - 4;
        let close_celsius = (previous_close_celsius
            + f32::from(step) * (TEMPERATURE_WALK_MAX_STEP_CELSIUS / 4.0))
            .clamp(TEMPERATURE_WALK_MIN_CELSIUS, TEMPERATURE_WALK_MAX_CELSIUS);
        let low_padding = 0.05 + (next_temperature_random(&mut random_state) % 6) as f32 * 0.02;
        let high_padding = 0.05 + (next_temperature_random(&mut random_state) % 6) as f32 * 0.02;
        let low_celsius = previous_close_celsius.min(close_celsius) - low_padding;
        let high_celsius = previous_close_celsius.max(close_celsius) + high_padding;

        ranges.push(TemperatureHighLowRuleV1 {
            middle_at,
            low_celsius,
            high_celsius,
            close_celsius,
        });
        closes.push(TemperatureCloseTickV1 {
            middle_at,
            ends_at: bucket_ends_at,
            close_celsius,
            low_celsius,
            high_celsius,
        });
        previous_close_celsius = close_celsius;
    }

    TemperatureHlcChartV1 { ranges, closes }
}

fn direct_labels() -> DirectLabelsChartV1 {
    vec![
        DirectAnnotationV1 {
            x: 20.0,
            y: 76.0,
            label: libertas_formatted_text("DIRECT_LABEL_FAST", &[]),
            mood: AnnotationMoodV1::Positive,
            size: 38.0,
            angle_degrees: -8.0,
            opacity: 1.0,
            annotation_key: text("fast"),
        },
        DirectAnnotationV1 {
            x: 52.0,
            y: 52.0,
            label: libertas_formatted_text("DIRECT_LABEL_CLEAR", &[]),
            mood: AnnotationMoodV1::Neutral,
            size: 30.0,
            angle_degrees: 0.0,
            opacity: 0.85,
            annotation_key: text("clear"),
        },
        DirectAnnotationV1 {
            x: 78.0,
            y: 28.0,
            label: libertas_formatted_text("DIRECT_LABEL_ACT_NOW", &[]),
            mood: AnnotationMoodV1::Attention,
            size: 24.0,
            angle_degrees: 12.0,
            opacity: 0.75,
            annotation_key: text("act-now"),
        },
    ]
}

fn performance_story() -> PerformanceStoryChartV1 {
    let values = [185, 172, 160, 147, 139, 126];
    let observations = values
        .into_iter()
        .enumerate()
        .map(|(index, response_milliseconds)| PerformancePointV1 {
            observed_at: BASE_TIME + index as u64 * HOUR,
            response_milliseconds,
            portfolio: if index % 2 == 0 {
                PortfolioV1::Platform
            } else {
                PortfolioV1::Business
            },
            point_size: if index == 5 { 96.0 } else { 48.0 },
        })
        .collect();
    let trend = [182, 173, 161, 150, 138, 127]
        .into_iter()
        .enumerate()
        .map(|(index, response_milliseconds)| PerformanceTrendPointV1 {
            trend_at: BASE_TIME + index as u64 * HOUR,
            response_milliseconds,
            scenario: ScenarioV1::Expected,
            sequence: index as u8,
        })
        .collect();
    let target = vec![PerformanceTargetRuleV1 {
        starts_at: BASE_TIME,
        ends_at: BASE_TIME + 5 * HOUR,
        target_milliseconds: 130,
        scenario: ScenarioV1::Stretch,
        dash_scenario: ScenarioV1::Stretch,
        stroke_width: 3.0,
    }];
    PerformanceStoryChartV1 {
        observations,
        trend,
        target,
    }
}

fn latency_histogram() -> LatencyHistogramChartV1 {
    [
        (0.0, 10.0, 41),
        (10.0, 20.0, 96),
        (20.0, 50.0, 33),
        (50.0, 100.0, 12),
    ]
    .into_iter()
    .enumerate()
    .map(
        |(index, (starts_at_milliseconds, ends_at_milliseconds, observations))| {
            LatencyHistogramBinV1 {
                starts_at_milliseconds,
                ends_at_milliseconds,
                observations,
                bin_key: format!("latency-bin-{index}"),
            }
        },
    )
    .collect()
}

fn stacked_energy() -> StackedEnergyChartV1 {
    let quarters = [QuarterV1::Q1, QuarterV1::Q2, QuarterV1::Q3, QuarterV1::Q4];
    let contributions = [
        (EnergySourceV1::Solar, [4.2, 5.1, 6.4, 5.8], "solar"),
        (EnergySourceV1::Battery, [2.6, 2.3, 2.9, 3.4], "battery"),
        (EnergySourceV1::Grid, [2.3, 1.8, 1.2, 1.6], "grid"),
    ];
    let mut bars = Vec::new();
    for (quarter_index, quarter) in quarters.into_iter().enumerate() {
        let mut starts_at = 0.0;
        for (source, values, source_key) in contributions {
            let ends_at = starts_at + values[quarter_index];
            bars.push(StackedEnergyBarSegmentV1 {
                quarter,
                starts_at_kilowatt_hours: starts_at,
                ends_at_kilowatt_hours: ends_at,
                source,
                segment_key: format!("stack-{quarter_index}-{source_key}"),
            });
            starts_at = ends_at;
        }
    }

    let solar = [3.8, 4.4, 5.0, 5.9, 6.3, 5.7];
    let battery = [1.4, 1.8, 2.0, 2.3, 2.1, 2.5];
    let grid = [3.1, 2.7, 2.2, 1.6, 1.4, 1.9];
    let area_sources = [
        (EnergySourceV1::Solar, solar, [0.0; 6], "solar"),
        (EnergySourceV1::Battery, battery, solar, "battery"),
        (
            EnergySourceV1::Grid,
            grid,
            [
                solar[0] + battery[0],
                solar[1] + battery[1],
                solar[2] + battery[2],
                solar[3] + battery[3],
                solar[4] + battery[4],
                solar[5] + battery[5],
            ],
            "grid",
        ),
    ];
    let mut areas = Vec::new();
    for (source, values, starts, source_key) in area_sources {
        for (sequence, (value, starts_at)) in values.into_iter().zip(starts).enumerate() {
            areas.push(StackedEnergyAreaPointV1 {
                measured_at: BASE_TIME + sequence as u64 * HOUR,
                starts_at_kilowatt_hours: starts_at,
                ends_at_kilowatt_hours: starts_at + value,
                source,
                sequence: sequence as u8,
                point_key: format!("stacked-area-{source_key}-{sequence}"),
            });
        }
    }
    StackedEnergyChartV1 { bars, areas }
}

fn arc_family() -> ArcFamilyChartV1 {
    let shares = [
        (TrafficSourceV1::Direct, 0.0, 0.34, 34.0, "direct"),
        (TrafficSourceV1::Search, 0.34, 0.65, 31.0, "search"),
        (TrafficSourceV1::Partner, 0.65, 0.84, 19.0, "partner"),
        (TrafficSourceV1::Campaign, 0.84, 1.0, 16.0, "campaign"),
    ];
    let pie = shares
        .into_iter()
        .map(
            |(source, theta_start, theta_end, share_percent, key)| PieSliceV1 {
                theta_start,
                theta_end,
                radius_start: 0.0,
                radius_end: 1.0,
                source,
                share_percent,
                slice_key: format!("pie-{key}"),
            },
        )
        .collect();
    let radial_bars = [
        (PortfolioV1::Home, 0.02, 0.30, 76.0, "home"),
        (PortfolioV1::Business, 0.35, 0.63, 91.0, "business"),
        (PortfolioV1::Platform, 0.68, 0.96, 64.0, "platform"),
    ]
    .into_iter()
    .map(
        |(portfolio, theta_start, theta_end, ends_at_percent, key)| RadialPerformanceBarV1 {
            theta_start,
            theta_end,
            starts_at_percent: 0.0,
            ends_at_percent,
            portfolio,
            bar_key: format!("radial-{key}"),
        },
    )
    .collect();
    ArcFamilyChartV1 { pie, radial_bars }
}

fn forecast_confidence() -> ForecastConfidenceChartV1 {
    let center = [22.0, 23.5, 25.0, 24.0, 22.5, 21.0];
    let mut estimates = Vec::new();
    for (model, adjustment, model_key) in [
        (PortfolioV1::Platform, 0.0, "platform"),
        (PortfolioV1::Business, -0.4, "business"),
    ] {
        for (sequence, estimate) in center.into_iter().enumerate() {
            estimates.push(ForecastEstimatePointV1 {
                forecast_at: BASE_TIME + sequence as u64 * HOUR,
                estimate_celsius: estimate + adjustment,
                model,
                sequence: sequence as u8,
                estimate_key: format!("forecast-{model_key}-{sequence}"),
            });
        }
    }
    ForecastConfidenceChartV1 {
        bands: forecast_range(),
        estimates,
    }
}

fn candlestick() -> CandlestickChartV1 {
    let values = [
        (102.1, 108.4, 101.2, 106.8, 18_342),
        (106.8, 109.1, 103.5, 104.2, 16_507),
        (104.2, 111.0, 103.8, 109.7, 22_184),
        (109.7, 110.2, 105.4, 106.1, 19_763),
        (106.1, 112.6, 105.8, 111.9, 24_031),
    ];
    let mut wicks = Vec::new();
    let mut bodies = Vec::new();
    for (index, (open, high, low, close, volume)) in values.into_iter().enumerate() {
        let middle_at = BASE_TIME + index as u64 * HOUR;
        let direction = if close >= open {
            CandleDirectionV1::Rising
        } else {
            CandleDirectionV1::Falling
        };
        let color = if direction == CandleDirectionV1::Rising {
            "#178A55"
        } else {
            "#C43C39"
        };
        let candle_key = format!("candle-{index}");
        wicks.push(CandlestickWickV1 {
            middle_at,
            low,
            high,
            open,
            close,
            volume,
            direction,
            wick_color: text(color),
            candle_key: candle_key.clone(),
        });
        bodies.push(CandlestickBodyV1 {
            starts_at: middle_at - 1_200,
            ends_at: middle_at + 1_200,
            open,
            close,
            middle_at,
            low,
            high,
            volume,
            direction,
            body_color: text(color),
            candle_key,
        });
    }
    CandlestickChartV1 { wicks, bodies }
}

fn box_plot() -> BoxPlotChartV1 {
    let summaries = [
        (
            QuarterV1::Q1,
            82.0,
            91.0,
            98.0,
            106.0,
            119.0,
            Some(132.0),
            PortfolioV1::Home,
        ),
        (
            QuarterV1::Q2,
            76.0,
            88.0,
            94.0,
            101.0,
            112.0,
            None,
            PortfolioV1::Business,
        ),
        (
            QuarterV1::Q3,
            68.0,
            79.0,
            86.0,
            95.0,
            108.0,
            Some(121.0),
            PortfolioV1::Platform,
        ),
        (
            QuarterV1::Q4,
            63.0,
            74.0,
            81.0,
            89.0,
            103.0,
            Some(55.0),
            PortfolioV1::Platform,
        ),
    ];
    let mut bodies = Vec::new();
    let mut whiskers = Vec::new();
    let mut medians = Vec::new();
    let mut outliers = Vec::new();
    for (index, (sample, low, q1, median, q3, high, outlier, portfolio)) in
        summaries.into_iter().enumerate()
    {
        bodies.push(BoxPlotBodyV1 {
            sample,
            first_quartile_milliseconds: q1,
            third_quartile_milliseconds: q3,
            portfolio,
            body_key: format!("box-body-{index}"),
        });
        whiskers.push(BoxPlotWhiskerV1 {
            sample,
            low_milliseconds: low,
            high_milliseconds: high,
            whisker_color: text("#52606D"),
            whisker_key: format!("box-whisker-{index}"),
        });
        medians.push(BoxPlotMedianV1 {
            sample,
            starts_at_milliseconds: median,
            ends_at_milliseconds: median,
            median_color: text("#15202B"),
            median_key: format!("box-median-{index}"),
        });
        if let Some(latency_milliseconds) = outlier {
            outliers.push(BoxPlotOutlierV1 {
                sample,
                latency_milliseconds,
                point_size: 5.0,
                outlier_key: format!("box-outlier-{index}"),
            });
        }
    }
    BoxPlotChartV1 {
        whiskers,
        bodies,
        medians,
        outliers,
    }
}

fn projected_map() -> ProjectedMapChartV1 {
    let regions = [
        (
            MapRegionV1::North,
            72.0,
            [
                (16.0, 58.0),
                (31.0, 66.0),
                (43.0, 72.0),
                (52.0, 91.0),
                (22.0, 88.0),
            ],
        ),
        (
            MapRegionV1::Central,
            54.0,
            [
                (25.0, 30.0),
                (61.0, 35.0),
                (70.0, 64.0),
                (43.0, 72.0),
                (16.0, 58.0),
            ],
        ),
        (
            MapRegionV1::South,
            83.0,
            [
                (25.0, 30.0),
                (61.0, 35.0),
                (82.0, 17.0),
                (48.0, 6.0),
                (18.0, 12.0),
            ],
        ),
    ];
    let mut rows = Vec::new();
    for (region_index, (region, utilization_percent, coordinates)) in
        regions.into_iter().enumerate()
    {
        for (sequence, (x, y)) in coordinates.into_iter().enumerate() {
            rows.push(ProjectedMapVertexV1 {
                x,
                y,
                region,
                utilization_percent,
                sequence: sequence as u8,
                vertex_key: format!("map-{region_index}-{sequence}"),
            });
        }
    }
    rows
}

fn network_topology() -> NetworkTopologyChartV1 {
    let nodes = [
        (
            18.0,
            52.0,
            NetworkNodeNameV1::Gateway,
            NetworkRoleV1::Gateway,
            12.0,
        ),
        (
            48.0,
            76.0,
            NetworkNodeNameV1::Rules,
            NetworkRoleV1::Service,
            10.0,
        ),
        (
            48.0,
            28.0,
            NetworkNodeNameV1::Analytics,
            NetworkRoleV1::Service,
            10.0,
        ),
        (
            82.0,
            67.0,
            NetworkNodeNameV1::Events,
            NetworkRoleV1::Storage,
            11.0,
        ),
        (
            82.0,
            33.0,
            NetworkNodeNameV1::History,
            NetworkRoleV1::Storage,
            11.0,
        ),
    ];
    let nodes = nodes
        .into_iter()
        .map(|(x, y, node, role, node_size)| NetworkNodeV1 {
            x,
            y,
            node,
            role_color: role,
            role_shape: role,
            node_size,
        })
        .collect();
    let edges = [
        (
            18.0,
            52.0,
            48.0,
            76.0,
            NetworkConnectionV1::GatewayToRules,
            "gateway-rules",
        ),
        (
            18.0,
            52.0,
            48.0,
            28.0,
            NetworkConnectionV1::GatewayToAnalytics,
            "gateway-analytics",
        ),
        (
            48.0,
            76.0,
            82.0,
            67.0,
            NetworkConnectionV1::RulesToEvents,
            "rules-events",
        ),
        (
            48.0,
            28.0,
            82.0,
            33.0,
            NetworkConnectionV1::AnalyticsToHistory,
            "analytics-history",
        ),
        (
            48.0,
            76.0,
            82.0,
            33.0,
            NetworkConnectionV1::RulesToHistory,
            "rules-history",
        ),
    ]
    .into_iter()
    .map(
        |(source_x, source_y, target_x, target_y, connection, edge_key)| NetworkEdgeV1 {
            source_x,
            target_x,
            source_y,
            target_y,
            connection,
            link_color: text("#7A8793"),
            link_width: 2.5,
            edge_key: text(edge_key),
        },
    )
    .collect();
    NetworkTopologyChartV1 { edges, nodes }
}

fn sankey_flow() -> SankeyFlowChartV1 {
    let streams = [
        (
            FlowStreamV1::DirectActivation,
            [42.0, 44.0, 48.0, 51.0],
            [62.0, 64.0, 68.0, 71.0],
            420,
            "direct",
        ),
        (
            FlowStreamV1::AssistedActivation,
            [20.0, 24.0, 31.0, 34.0],
            [34.0, 38.0, 45.0, 48.0],
            280,
            "assisted",
        ),
        (
            FlowStreamV1::EarlyExit,
            [68.0, 66.0, 57.0, 45.0],
            [82.0, 80.0, 71.0, 59.0],
            190,
            "exit",
        ),
    ];
    let mut ribbons = Vec::new();
    for (stream, lowers, uppers, visitors, stream_key) in streams {
        for (sequence, ((lower_y, upper_y), x)) in lowers
            .into_iter()
            .zip(uppers)
            .zip([8.0, 36.0, 64.0, 92.0])
            .enumerate()
        {
            ribbons.push(SankeyRibbonPointV1 {
                x,
                lower_y,
                upper_y,
                stream,
                visitors,
                fill_opacity: 0.62,
                sequence: sequence as u8,
                sample_key: format!("sankey-{stream_key}-{sequence}"),
            });
        }
    }
    let nodes = [
        (
            5.0,
            11.0,
            18.0,
            84.0,
            FlowStageV1::Visits,
            890,
            "#4E79A7",
            "visits",
        ),
        (
            47.0,
            53.0,
            20.0,
            80.0,
            FlowStageV1::Evaluation,
            700,
            "#76B7B2",
            "evaluation",
        ),
        (
            89.0,
            95.0,
            34.0,
            71.0,
            FlowStageV1::Activated,
            700,
            "#59A14F",
            "activated",
        ),
        (
            89.0,
            95.0,
            74.0,
            92.0,
            FlowStageV1::Exited,
            190,
            "#E15759",
            "exited",
        ),
    ]
    .into_iter()
    .map(
        |(starts_at_x, ends_at_x, starts_at_y, ends_at_y, stage, visitors, color, key)| {
            SankeyNodeV1 {
                starts_at_x,
                ends_at_x,
                starts_at_y,
                ends_at_y,
                stage,
                visitors,
                node_color: text(color),
                node_key: text(key),
            }
        },
    )
    .collect();
    SankeyFlowChartV1 { ribbons, nodes }
}

fn response_for_at(
    request: ChartDemoProtocol,
    temperature_ends_at: LibertasDateTime,
) -> Option<ChartDemoProtocol> {
    Some(match request {
        ChartDemoProtocol::GetBubblePortfolioV1 => {
            ChartDemoProtocol::BubblePortfolioV1(bubble_portfolio())
        }
        ChartDemoProtocol::GetEnergyHistoryV1 => {
            ChartDemoProtocol::EnergyHistoryV1(energy_history())
        }
        ChartDemoProtocol::GetForecastRangeV1 => {
            ChartDemoProtocol::ForecastRangeV1(forecast_range())
        }
        ChartDemoProtocol::GetGroupedSalesV1 => ChartDemoProtocol::GroupedSalesV1(grouped_sales()),
        ChartDemoProtocol::GetLatencyHistogramV1 => {
            ChartDemoProtocol::LatencyHistogramV1(latency_histogram())
        }
        ChartDemoProtocol::GetStackedEnergyV1 => {
            ChartDemoProtocol::StackedEnergyV1(stacked_energy())
        }
        ChartDemoProtocol::GetActivityHeatmapV1 => {
            ChartDemoProtocol::ActivityHeatmapV1(activity_heatmap())
        }
        ChartDemoProtocol::GetMeasurementUncertaintyV1 => {
            ChartDemoProtocol::MeasurementUncertaintyV1(measurement_uncertainty())
        }
        ChartDemoProtocol::GetDirectLabelsV1 => ChartDemoProtocol::DirectLabelsV1(direct_labels()),
        ChartDemoProtocol::GetCapabilityRadarV1 => {
            ChartDemoProtocol::CapabilityRadarV1(capability_radar())
        }
        ChartDemoProtocol::GetTrafficSourcesV1 => {
            ChartDemoProtocol::TrafficSourcesV1(traffic_sources())
        }
        ChartDemoProtocol::GetArcFamilyV1 => ChartDemoProtocol::ArcFamilyV1(arc_family()),
        ChartDemoProtocol::GetProjectedMapV1 => ChartDemoProtocol::ProjectedMapV1(projected_map()),
        ChartDemoProtocol::GetForecastConfidenceV1 => {
            ChartDemoProtocol::ForecastConfidenceV1(forecast_confidence())
        }
        ChartDemoProtocol::GetCandlestickV1 => ChartDemoProtocol::CandlestickV1(candlestick()),
        ChartDemoProtocol::GetTemperatureHlcV1 => {
            ChartDemoProtocol::TemperatureHlcV1(temperature_hlc(temperature_ends_at))
        }
        ChartDemoProtocol::GetBoxPlotV1 => ChartDemoProtocol::BoxPlotV1(box_plot()),
        ChartDemoProtocol::GetNetworkTopologyV1 => {
            ChartDemoProtocol::NetworkTopologyV1(network_topology())
        }
        ChartDemoProtocol::GetSankeyFlowV1 => ChartDemoProtocol::SankeyFlowV1(sankey_flow()),
        ChartDemoProtocol::GetPerformanceStoryV1 => {
            ChartDemoProtocol::PerformanceStoryV1(performance_story())
        }
        ChartDemoProtocol::GetRegionalComparisonV1 => {
            ChartDemoProtocol::RegionalComparisonV1(RegionalComparisonChartV1 {
                sales: grouped_sales(),
                horizontal_sales: horizontal_grouped_sales(),
                traffic: traffic_sources(),
            })
        }
        ChartDemoProtocol::GetOperationsDashboardV1 => {
            ChartDemoProtocol::OperationsDashboardV1(OperationsDashboardChartV1 {
                energy: energy_history(),
                forecast: forecast_range(),
                activity: activity_heatmap(),
            })
        }
        _ => return None,
    })
}

#[cfg(test)]
fn response_for(request: ChartDemoProtocol) -> Option<ChartDemoProtocol> {
    response_for_at(request, BASE_TIME)
}

fn handle_gallery_request(
    endpoint: LibertasEndpoint,
    opcode: u8,
    message: Option<ChartDemoProtocol>,
    _context: &mut Box<dyn Any>,
    transaction_id: u32,
    peer: u32,
) -> LibertasEndpointStatus {
    if opcode != OP_ENDPOINT_REQ {
        return LibertasEndpointStatus::InvalidMessage;
    }
    let temperature_ends_at = utc_seconds_or_base_time(libertas_get_utc_time());
    let Some(response) = message.and_then(|message| response_for_at(message, temperature_ends_at))
    else {
        return LibertasEndpointStatus::InvalidMessage;
    };
    libertas_endpoint_response(endpoint, &response, transaction_id, peer);
    LibertasEndpointStatus::Success
}

/// Chart gallery
/// Lets people explore every Libertas chart type with polished, interactive
/// sample data. The user can choose individual examples or composed dashboards.
/// The gallery shows business portfolios, energy and sales histories,
/// forecasts, distributions and uncertainty, activity patterns, traffic
/// shares, geographic regions, network relationships, flows, direct labels,
/// and layered or polar comparisons. Each example demonstrates its applicable
/// axes, legends, tooltips, shapes, colors, labels, and layout.
#[libertas_export]
pub fn chart_demo(
    /*
     * Gallery endpoint
     * Select this endpoint and choose a chart to view its demonstration data.
     */
    #[libertas_endpoint_schema(ChartDemoProtocol)]
    #[libertas_endpoint_server]
    gallery: LibertasEndpoint,
) {
    libertas_register_endpoint_listener::<ChartDemoProtocol, _>(
        gallery,
        handle_gallery_request,
        Box::new(()),
    );
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    fn requests() -> Vec<ChartDemoProtocol> {
        vec![
            ChartDemoProtocol::GetBubblePortfolioV1,
            ChartDemoProtocol::GetEnergyHistoryV1,
            ChartDemoProtocol::GetForecastRangeV1,
            ChartDemoProtocol::GetGroupedSalesV1,
            ChartDemoProtocol::GetLatencyHistogramV1,
            ChartDemoProtocol::GetStackedEnergyV1,
            ChartDemoProtocol::GetActivityHeatmapV1,
            ChartDemoProtocol::GetMeasurementUncertaintyV1,
            ChartDemoProtocol::GetDirectLabelsV1,
            ChartDemoProtocol::GetCapabilityRadarV1,
            ChartDemoProtocol::GetTrafficSourcesV1,
            ChartDemoProtocol::GetArcFamilyV1,
            ChartDemoProtocol::GetProjectedMapV1,
            ChartDemoProtocol::GetForecastConfidenceV1,
            ChartDemoProtocol::GetCandlestickV1,
            ChartDemoProtocol::GetTemperatureHlcV1,
            ChartDemoProtocol::GetBoxPlotV1,
            ChartDemoProtocol::GetNetworkTopologyV1,
            ChartDemoProtocol::GetSankeyFlowV1,
            ChartDemoProtocol::GetPerformanceStoryV1,
            ChartDemoProtocol::GetRegionalComparisonV1,
            ChartDemoProtocol::GetOperationsDashboardV1,
        ]
    }

    #[test]
    fn every_gallery_request_has_a_nonempty_response() {
        for request in requests() {
            let response = response_for(request).expect("request must have a response");
            assert!(!response.to_avro().is_empty());
        }
    }

    #[test]
    fn every_gallery_response_round_trips_through_avro() {
        for request in requests() {
            let response = response_for(request).expect("request must have a response");
            let encoded = response.to_avro();
            let decoded = ChartDemoProtocol::from_avro(&encoded).expect("valid Avro response");
            assert_eq!(decoded, response);
        }
    }

    #[test]
    fn invalid_protocol_roles_do_not_produce_responses() {
        let response = ChartDemoProtocol::DirectLabelsV1(direct_labels());
        assert!(response_for(response).is_none());
    }

    #[test]
    fn direct_labels_use_canonical_formatted_text_tuples() {
        let labels = direct_labels();
        let expected_resources = [
            "DIRECT_LABEL_FAST",
            "DIRECT_LABEL_CLEAR",
            "DIRECT_LABEL_ACT_NOW",
        ];

        for (label, expected_resource) in labels.iter().zip(expected_resources) {
            assert_eq!(label.label, libertas_formatted_text(expected_resource, &[]));
        }
    }

    #[test]
    fn fixed_chart_vocabularies_remain_typed() {
        let activity = activity_heatmap();
        assert_eq!(activity[0].time_block, TimeBlockV1::Morning);
        assert_eq!(activity[3].time_block, TimeBlockV1::Evening);

        let network = network_topology();
        assert_eq!(network.nodes[0].node, NetworkNodeNameV1::Gateway);
        assert_eq!(
            network.edges[0].connection,
            NetworkConnectionV1::GatewayToRules
        );

        let sankey = sankey_flow();
        assert_eq!(sankey.nodes[0].stage, FlowStageV1::Visits);
        assert_eq!(sankey.nodes[3].stage, FlowStageV1::Exited);
    }

    #[test]
    fn radar_profiles_are_explicitly_closed() {
        let points = capability_radar();
        assert_eq!(points.len(), 12);
        for profile in points.chunks_exact(6) {
            assert_eq!(
                profile.first().unwrap().capability,
                profile.last().unwrap().capability
            );
            assert_eq!(
                profile.first().unwrap().score_percent,
                profile.last().unwrap().score_percent
            );
        }
    }

    #[test]
    fn donut_segments_cover_one_turn_without_gaps() {
        let segments = traffic_sources();
        assert_eq!(segments.first().unwrap().theta_start, 0.0);
        assert_eq!(segments.last().unwrap().theta_end, 1.0);
        for pair in segments.windows(2) {
            assert_eq!(pair[0].theta_end, pair[1].theta_start);
        }
        assert_eq!(
            segments
                .iter()
                .map(|segment| segment.share_percent)
                .sum::<f32>(),
            100.0
        );
        assert_eq!(segments.len(), 6);
        assert_eq!(segments[4].share_percent, 2.0);
        assert_eq!(segments[5].share_percent, 1.0);
        assert!(segments.iter().all(|segment| {
            segment.theta_start < segment.theta_end
                && segment.radius_start == 0.55
                && segment.radius_end == 1.0
        }));
    }

    #[test]
    fn histogram_bins_are_positive_and_contiguous() {
        let bins = latency_histogram();
        assert!(!bins.is_empty());
        for bin in &bins {
            assert!(bin.starts_at_milliseconds < bin.ends_at_milliseconds);
        }
        for pair in bins.windows(2) {
            assert_eq!(pair[0].ends_at_milliseconds, pair[1].starts_at_milliseconds);
        }
    }

    #[test]
    fn explicit_stacks_have_contiguous_positive_segments() {
        let chart = stacked_energy();
        for stack in chart.bars.chunks_exact(3) {
            assert_eq!(stack[0].starts_at_kilowatt_hours, 0.0);
            for segment in stack {
                assert!(segment.starts_at_kilowatt_hours < segment.ends_at_kilowatt_hours);
            }
            for pair in stack.windows(2) {
                assert_eq!(
                    pair[0].ends_at_kilowatt_hours,
                    pair[1].starts_at_kilowatt_hours
                );
            }
        }
        for point in chart.areas {
            assert!(point.starts_at_kilowatt_hours < point.ends_at_kilowatt_hours);
        }
    }

    #[test]
    fn arc_family_contains_a_pie_and_quantitative_radial_bars() {
        let chart = arc_family();
        assert_eq!(chart.pie.first().unwrap().theta_start, 0.0);
        assert_eq!(chart.pie.last().unwrap().theta_end, 1.0);
        assert!(
            chart
                .pie
                .iter()
                .all(|slice| slice.radius_start == 0.0 && slice.radius_end == 1.0)
        );
        assert!(chart.radial_bars.iter().all(|bar| {
            bar.starts_at_percent == 0.0
                && bar.ends_at_percent > 0.0
                && bar.ends_at_percent <= 100.0
        }));
    }

    #[test]
    fn layered_family_data_preserves_authored_bounds() {
        let forecast = forecast_confidence();
        assert_eq!(forecast.bands.len(), forecast.estimates.len());

        let candles = candlestick();
        assert_eq!(candles.wicks.len(), candles.bodies.len());
        for (wick, body) in candles.wicks.iter().zip(&candles.bodies) {
            assert_eq!(wick.candle_key, body.candle_key);
            assert!(wick.low <= body.open.min(body.close));
            assert!(body.open.max(body.close) <= wick.high);
            assert!(body.starts_at < wick.middle_at && wick.middle_at < body.ends_at);
        }

        let boxes = box_plot();
        assert_eq!(boxes.bodies.len(), boxes.whiskers.len());
        assert_eq!(boxes.bodies.len(), boxes.medians.len());
        for ((body, whisker), median) in
            boxes.bodies.iter().zip(&boxes.whiskers).zip(&boxes.medians)
        {
            assert!(whisker.low_milliseconds <= body.first_quartile_milliseconds);
            assert!(body.first_quartile_milliseconds <= median.starts_at_milliseconds);
            assert_eq!(median.starts_at_milliseconds, median.ends_at_milliseconds);
            assert!(median.ends_at_milliseconds <= body.third_quartile_milliseconds);
            assert!(body.third_quartile_milliseconds <= whisker.high_milliseconds);
        }
    }

    #[test]
    fn server_positioned_family_geometry_is_complete() {
        let map = projected_map();
        assert_eq!(map.len(), 15);
        assert!(map.chunks_exact(5).all(|region| {
            region
                .iter()
                .all(|vertex| vertex.region == region[0].region)
        }));

        let network = network_topology();
        assert_eq!(network.nodes.len(), 5);
        assert_eq!(network.edges.len(), 5);
        assert!(
            network
                .edges
                .iter()
                .all(|edge| edge.source_x != edge.target_x || edge.source_y != edge.target_y)
        );

        let sankey = sankey_flow();
        assert_eq!(sankey.ribbons.len(), 12);
        assert_eq!(sankey.nodes.len(), 4);
        for ribbon in sankey.ribbons.chunks_exact(4) {
            assert!(ribbon.iter().all(|point| point.lower_y < point.upper_y));
            assert!(ribbon.windows(2).all(|pair| pair[0].x < pair[1].x));
        }
    }

    #[test]
    fn temperature_hlc_covers_exactly_72_hours_in_five_minute_buckets() {
        let now = BASE_TIME + 47;
        let chart = temperature_hlc(now);
        assert_eq!(chart.ranges.len(), TEMPERATURE_BUCKETS);
        assert_eq!(chart.closes.len(), TEMPERATURE_BUCKETS);
        assert_eq!(chart.ranges[0].middle_at, BASE_TIME - 72 * HOUR + 150);
        assert_eq!(chart.closes.last().unwrap().ends_at, BASE_TIME);

        let mut previous_close_celsius = TEMPERATURE_WALK_START_CELSIUS;
        let mut includes_rise = false;
        let mut includes_fall = false;
        for (range, close) in chart.ranges.iter().zip(&chart.closes) {
            assert_eq!(range.middle_at, close.middle_at);
            assert_eq!(range.close_celsius, close.close_celsius);
            assert!(range.low_celsius <= previous_close_celsius);
            assert!(range.low_celsius <= range.close_celsius);
            assert!(previous_close_celsius <= range.high_celsius);
            assert!(range.close_celsius <= range.high_celsius);
            let step_celsius = range.close_celsius - previous_close_celsius;
            assert!(step_celsius.abs() <= TEMPERATURE_WALK_MAX_STEP_CELSIUS + 0.000_01);
            includes_rise |= step_celsius > 0.0;
            includes_fall |= step_celsius < 0.0;
            previous_close_celsius = range.close_celsius;
            assert_eq!(close.ends_at - close.middle_at, FIVE_MINUTES / 2);
        }
        assert!(includes_rise && includes_fall);
    }

    #[test]
    fn runtime_utc_microseconds_are_converted_to_chart_seconds() {
        let utc_microseconds = (BASE_TIME + 47) * MICROSECONDS_PER_SECOND + 999_999;
        assert_eq!(
            utc_seconds_or_base_time(Some(utc_microseconds)),
            BASE_TIME + 47
        );
        assert_eq!(utc_seconds_or_base_time(None), BASE_TIME);
    }
}
