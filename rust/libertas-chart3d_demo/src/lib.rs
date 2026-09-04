//! Libertas 3D chart gallery.
//! Explore every supported 3D chart family with deterministic, typed sample data.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::{boxed::Box, vec, vec::Vec};
use core::any::Any;

use libertas::{
    LibertasEndpoint, LibertasEndpointStatus, OP_ENDPOINT_REQ, libertas_endpoint_response,
    libertas_register_endpoint_listener,
};
use libertas_macros::{
    LibertasAvroDecode, LibertasAvroEncode, LibertasExport, libertas_chart3d, libertas_export,
};

/// Scatter cluster
/// Distinguishes spatial groups in the scatter example.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum ScatterClusterV1 {
    /// North ridge
    /// Samples concentrated around the northern ridge.
    NorthRidge,
    /// Central basin
    /// Samples concentrated around the central basin.
    CentralBasin,
    /// South shelf
    /// Samples concentrated around the southern shelf.
    SouthShelf,
}

/// Trajectory
/// Identifies one ordered path through three-dimensional space.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum TrajectoryV1 {
    /// Survey
    /// The measured survey path.
    Survey,
    /// Forecast
    /// The predicted path.
    Forecast,
    /// Limit
    /// The operating-limit path.
    Limit,
}

/// Quarter
/// Categorical base position for sparse 3D columns.
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
}

/// Sales region
/// Geographic base position for sparse 3D columns.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SalesRegionV1 {
    /// Americas
    /// North and South American sales.
    Americas,
    /// Europe
    /// European sales.
    Europe,
}

/// Product family
/// Categorical base position for horizontal legacy bars.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum ProductFamilyV1 {
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

/// Bar scenario
/// Scenario depth for horizontal legacy bars.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BarScenarioV1 {
    /// Actual
    /// Recorded value.
    Actual,
    /// Plan
    /// Planned value.
    Plan,
}

/// Bubble segment
/// Market segment encoded by bubble color and detail.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BubbleSegmentV1 {
    /// Core
    /// Established core products.
    Core,
    /// Growth
    /// Rapidly expanding products.
    Growth,
    /// Research
    /// Experimental products.
    Research,
}

/// Bubble class
/// Product class encoded by a portable automatic point shape.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum BubbleClassV1 {
    /// Consumer
    /// Consumer-focused product.
    Consumer,
    /// Industrial
    /// Industrial product.
    Industrial,
    /// Scientific
    /// Scientific product.
    Scientific,
}

/// Contour level
/// Server-computed level of a contour path.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum ContourLevelV1 {
    /// One unit
    /// Contour at one height unit.
    One,
    /// Two units
    /// Contour at two height units.
    Two,
    /// Three units
    /// Contour at three height units.
    Three,
}

/// Spectrum trace
/// One ordered trace in the waterfall spectrum.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum SpectrumTraceV1 {
    /// Baseline
    /// Baseline trace.
    Baseline,
    /// Morning
    /// Morning trace.
    Morning,
    /// Afternoon
    /// Afternoon trace.
    Afternoon,
    /// Evening
    /// Evening trace.
    Evening,
}

/// Impulse sign
/// Distinguishes positive and negative impulses without relying on color alone.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum ImpulseSignV1 {
    /// Negative
    /// An impulse below the zero plane.
    Negative,
    /// Nonnegative
    /// An impulse on or above the zero plane.
    Nonnegative,
}

/// Cloud region
/// Spatial region used to group the point cloud.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum CloudRegionV1 {
    /// Upper east
    /// Positive height and nonnegative x position.
    UpperEast,
    /// Upper west
    /// Positive height and negative x position.
    UpperWest,
    /// Lower east
    /// Nonpositive height and nonnegative x position.
    LowerEast,
    /// Lower west
    /// Nonpositive height and negative x position.
    LowerWest,
}

/// Multiple-series role
/// Identifies the meaning of one independently typed series.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum MultipleSeriesRoleV1 {
    /// Volume
    /// Column volume.
    Volume,
    /// Trend
    /// Smoothed trend line.
    Trend,
    /// Observation
    /// Individual observed point.
    Observation,
}

/// Multiple-series lane
/// Shared categorical depth position for the layered series.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum MultipleSeriesLaneV1 {
    /// Main lane
    /// The common comparison lane.
    Main,
}

/// Scatter sample
/// One independently positioned sample with stable typed identity.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct ScatterPointV1 {
    /// Easting
    /// Horizontal east-west position.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = scatter_x, kind = linear, growMin = 0.05, growMax = 0.05)]
    #[libertas_chart3d_guide(target = x, source = scale, tickCount = 6)]
    pub x: f32,
    /// Northing
    /// Horizontal north-south position.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = scatter_y, kind = linear, growMin = 0.05, growMax = 0.05)]
    #[libertas_chart3d_guide(target = y, source = scale, tickCount = 6)]
    pub y: f32,
    /// Elevation
    /// Vertical position.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = scatter_z, kind = linear, growMin = 0.1, growMax = 0.1)]
    #[libertas_chart3d_guide(target = z, source = scale, tickCount = 5)]
    pub z: f32,
    /// Cluster
    /// Typed grouping and color for the sample.
    #[libertas_chart3d_channel(color, detail, tooltip)]
    #[libertas_chart3d_scale(kind = ordinal)]
    #[libertas_chart3d_guide(target = color, source = none)]
    pub cluster: ScatterClusterV1,
    /// Sample key
    /// Stable identity within the complete snapshot.
    #[libertas_chart3d_channel(key)]
    pub key: u16,
}

/// 3D scatter
/// Spatial samples grouped into three typed clusters.
#[libertas_chart3d(point)]
pub type ScatterChartV1 = Vec<ScatterPointV1>;

/// Trajectory point
/// One ordered vertex along a three-dimensional path.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct TrajectoryPointV1 {
    /// Progress
    /// Horizontal path progress.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = trajectory_x, kind = linear, min = 0, growMax = 0.03)]
    #[libertas_chart3d_guide(target = x, source = scale, tickCount = 7)]
    pub x: f32,
    /// Lateral position
    /// Side-to-side path position.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = trajectory_y, kind = linear, growMin = 0.08, growMax = 0.08)]
    #[libertas_chart3d_guide(target = y, source = scale, tickCount = 5)]
    pub y: f32,
    /// Height
    /// Vertical path position.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = trajectory_z, kind = linear, growMin = 0.05, growMax = 0.05)]
    #[libertas_chart3d_guide(target = z, source = scale, tickCount = 5)]
    pub z: f32,
    /// Trajectory
    /// Typed path identity and series legend.
    #[libertas_chart3d_channel(detail, tooltip)]
    #[libertas_chart3d_guide(target = detail, source = value)]
    pub trajectory: TrajectoryV1,
    /// Trajectory color
    /// Repeats the typed trajectory as a categorical line color.
    #[libertas_chart3d_channel(color)]
    #[libertas_chart3d_scale(kind = ordinal)]
    #[libertas_chart3d_guide(target = color, source = none)]
    pub trajectory_color: TrajectoryV1,
    /// Sequence
    /// Vertex order within this trajectory.
    #[libertas_chart3d_channel(order)]
    pub sequence: u16,
    /// Point key
    /// Stable vertex identity.
    #[libertas_chart3d_channel(key)]
    pub key: u16,
}

/// 3D trajectories
/// Multiple ordered paths partitioned by typed detail identity.
#[libertas_chart3d(line)]
pub type TrajectoryChartV1 = Vec<TrajectoryPointV1>;

/// Sparse column
/// One z-oriented column with two categorical base axes.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SparseColumnV1 {
    /// Quarter
    /// First categorical base axis.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = sparse_quarter, kind = band, padding = 0.18)]
    #[libertas_chart3d_guide(target = x, source = scale)]
    pub quarter: QuarterV1,
    /// Region
    /// Second categorical base axis.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = sparse_region, kind = band, padding = 0.18)]
    #[libertas_chart3d_guide(target = y, source = scale)]
    pub region: SalesRegionV1,
    /// Revenue
    /// Continuous column value on the z axis.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = sparse_revenue, kind = linear, min = 0, zero = true, growMax = 0.08)]
    #[libertas_chart3d_guide(target = z, source = scale, tickCount = 5)]
    pub revenue_millions: f32,
    /// Column key
    /// Stable column identity.
    #[libertas_chart3d_channel(key)]
    pub key: u16,
}

/// Sparse columns
/// Normal x/y-base sparse columns with an inferred zero plane.
#[libertas_chart3d(bar)]
pub type SparseColumnsChartV1 = Vec<SparseColumnV1>;

/// Horizontal legacy bar
/// One any-orientation bar with two band bases and explicit value endpoints.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct HorizontalLegacyBarV1 {
    /// Value
    /// Continuous endpoint along the x axis.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = legacy_value, kind = linear, min = 0, zero = true, growMax = 0.08)]
    #[libertas_chart3d_guide(target = x, source = scale, tickCount = 5)]
    pub value: f32,
    /// Baseline
    /// Authored continuous x-axis baseline.
    #[libertas_chart3d_channel(x2, tooltip)]
    pub baseline: f32,
    /// Product
    /// First banded base axis.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = legacy_product, kind = band, padding = 0.2)]
    #[libertas_chart3d_guide(target = y, source = scale)]
    pub product: ProductFamilyV1,
    /// Scenario
    /// Second banded base axis.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = legacy_scenario, kind = band, padding = 0.2)]
    #[libertas_chart3d_guide(target = z, source = scale)]
    pub scenario: BarScenarioV1,
    /// Bar key
    /// Stable bar identity.
    #[libertas_chart3d_channel(key)]
    pub key: u16,
}

/// Horizontal legacy bars
/// Any-orientation bars whose value axis is x and whose y/z axes are banded.
#[libertas_chart3d(bar)]
pub type HorizontalLegacyBarsChartV1 = Vec<HorizontalLegacyBarV1>;

/// Explicit cuboid
/// One bar with all six continuous endpoints authored by the application.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct ExplicitCuboidV1 {
    /// X start
    /// First x endpoint.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = cuboid_x, kind = linear, growMin = 0.05, growMax = 0.05)]
    #[libertas_chart3d_guide(target = x, source = scale)]
    pub x: f32,
    /// X end
    /// Second x endpoint.
    #[libertas_chart3d_channel(x2, tooltip)]
    pub x2: f32,
    /// Y start
    /// First y endpoint.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = cuboid_y, kind = linear, growMin = 0.05, growMax = 0.05)]
    #[libertas_chart3d_guide(target = y, source = scale)]
    pub y: f32,
    /// Y end
    /// Second y endpoint.
    #[libertas_chart3d_channel(y2, tooltip)]
    pub y2: f32,
    /// Z top
    /// First z endpoint.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = cuboid_z, kind = linear, growMin = 0.05, growMax = 0.05)]
    #[libertas_chart3d_guide(target = z, source = scale)]
    pub z: f32,
    /// Z bottom
    /// Second z endpoint.
    #[libertas_chart3d_channel(z2, tooltip)]
    pub z2: f32,
    /// Cuboid key
    /// Stable cuboid identity.
    #[libertas_chart3d_channel(key)]
    pub key: u16,
}

/// Explicit cuboids
/// Fully authored axis-aligned cuboids with no inferred width or baseline.
#[libertas_chart3d(bar)]
pub type ExplicitCuboidsChartV1 = Vec<ExplicitCuboidV1>;

/// Bar forms
/// Places all three valid 3D bar geometry forms in independent scenes.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart3d(hconcat)]
pub struct BarFormsChartV1 {
    /// Sparse columns
    /// Normal x/y-base columns with categorical bases.
    pub sparse_columns: SparseColumnsChartV1,
    /// Horizontal bars
    /// Any-orientation legacy bars with x as their value axis.
    pub horizontal_bars: HorizontalLegacyBarsChartV1,
    /// Explicit cuboids
    /// Bars with all six endpoints authored.
    pub explicit_cuboids: ExplicitCuboidsChartV1,
}

/// Bubble point
/// One point whose magnitude controls its rendered size.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct BubblePointV1 {
    /// Reach
    /// Product reach position.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = bubble_reach, kind = linear, min = 0, growMax = 0.08)]
    #[libertas_chart3d_guide(target = x, source = scale, tickCount = 5)]
    pub reach: f32,
    /// Growth
    /// Product growth position.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = bubble_growth, kind = linear, growMin = 0.08, growMax = 0.08)]
    #[libertas_chart3d_guide(target = y, source = scale, tickCount = 5)]
    #[libertas_physical_unit("percent")]
    pub growth_percent: f32,
    /// Retention
    /// Product retention position.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = bubble_retention, kind = linear, min = 0, max = 100)]
    #[libertas_chart3d_guide(target = z, source = scale, tickCount = 5)]
    #[libertas_physical_unit("percent")]
    pub retention_percent: f32,
    /// Annual value
    /// Nonnegative magnitude mapped to bubble size.
    #[libertas_chart3d_channel(size, tooltip)]
    #[libertas_chart3d_scale(kind = sqrt, min = 0)]
    #[libertas_chart3d_guide(target = size, source = scale)]
    pub annual_value: f32,
    /// Segment
    /// Typed detail identity and bubble color.
    #[libertas_chart3d_channel(color, detail, tooltip)]
    #[libertas_chart3d_scale(kind = ordinal)]
    #[libertas_chart3d_guide(target = color, source = none)]
    pub segment: BubbleSegmentV1,
    /// Product class
    /// Typed category mapped to a portable automatic point shape.
    #[libertas_chart3d_channel(shape, tooltip)]
    #[libertas_chart3d_scale(kind = ordinal)]
    #[libertas_chart3d_guide(target = shape, source = scale)]
    pub class: BubbleClassV1,
    /// Bubble key
    /// Stable product identity.
    #[libertas_chart3d_channel(key)]
    pub key: u16,
}

/// Bubble 3D
/// Three-dimensional points with nonnegative quantitative size.
#[libertas_chart3d(point)]
pub type BubbleChartV1 = Vec<BubblePointV1>;

/// Height-map sample
/// One vertex in a complete rectangular x/y grid.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct HeightMapPointV1 {
    /// X coordinate
    /// Rectangular-grid x position.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = height_map_x, kind = linear, growMin = 0.03, growMax = 0.03)]
    #[libertas_chart3d_guide(target = x, source = scale, tickStep = 2)]
    pub x: f32,
    /// Y coordinate
    /// Rectangular-grid y position.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = height_map_y, kind = linear, growMin = 0.03, growMax = 0.03)]
    #[libertas_chart3d_guide(target = y, source = scale, tickStep = 2)]
    pub y: f32,
    /// Height
    /// Sampled surface height.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = height_map_z, kind = linear, growMin = 0.05, growMax = 0.05)]
    #[libertas_chart3d_guide(target = z, source = scale, tickCount = 6)]
    pub height: f32,
    /// Height color
    /// Duplicated numeric height used by an independently configured color scale.
    #[libertas_chart3d_channel(color)]
    #[libertas_chart3d_scale(id = height_map_color, kind = linear)]
    #[libertas_chart3d_guide(target = color, source = scale)]
    pub height_color: f32,
}

/// Surface height map
/// A complete rectangular grid sampled from an application-owned function.
#[libertas_chart3d(surface)]
pub type HeightMapChartV1 = Vec<HeightMapPointV1>;

/// Wireframe sample
/// One vertex of a rectangular saddle grid.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct WireframePointV1 {
    /// X coordinate
    /// Rectangular-grid x position.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = wireframe_x, kind = linear)]
    #[libertas_chart3d_guide(target = x, source = scale, tickStep = 2)]
    pub x: f32,
    /// Y coordinate
    /// Rectangular-grid y position.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = wireframe_y, kind = linear)]
    #[libertas_chart3d_guide(target = y, source = scale, tickStep = 2)]
    pub y: f32,
    /// Height
    /// Saddle height at this grid vertex.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = wireframe_z, kind = linear, growMin = 0.05, growMax = 0.05)]
    #[libertas_chart3d_guide(target = z, source = scale, tickCount = 5)]
    pub height: f32,
    /// Height color
    /// Numeric color encoding for wire edges.
    #[libertas_chart3d_channel(color)]
    #[libertas_chart3d_scale(kind = linear)]
    #[libertas_chart3d_guide(target = color, source = scale)]
    pub height_color: f32,
}

/// Wireframe
/// Rectangular surface topology rendered as grid edges without face diagonals.
#[libertas_chart3d(wireframe)]
pub type WireframeChartV1 = Vec<WireframePointV1>;

/// Contour-surface sample
/// One vertex in the surface beneath the server-computed contours.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct ContourSurfacePointV1 {
    /// X coordinate
    /// Shared surface and contour x position.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = contour_x, kind = linear, min = -3.25, max = 3.25)]
    #[libertas_chart3d_guide(target = x, source = scale, tickStep = 1)]
    pub x: f32,
    /// Y coordinate
    /// Shared surface and contour y position.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = contour_y, kind = linear, min = -3.25, max = 3.25)]
    #[libertas_chart3d_guide(target = y, source = scale, tickStep = 1)]
    pub y: f32,
    /// Height
    /// Piecewise-affine diamond height.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = contour_z, kind = linear, min = 0, max = 6)]
    #[libertas_chart3d_guide(target = z, source = scale, tickStep = 1)]
    pub height: f32,
    /// Surface color
    /// Duplicated height controlling the surface color.
    #[libertas_chart3d_channel(color)]
    #[libertas_chart3d_scale(id = contour_surface_color, kind = linear)]
    #[libertas_chart3d_guide(target = color, source = scale)]
    pub surface_color: f32,
}

/// Contour surface
/// Server-sampled piecewise-affine surface beneath the contour paths.
#[libertas_chart3d(surface)]
pub type ContourSurfaceChartV1 = Vec<ContourSurfacePointV1>;

/// Contour vertex
/// One ordered point in a server-computed contour path.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct ContourLinePointV1 {
    /// X coordinate
    /// Shared surface and contour x position.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = contour_x, kind = linear, min = -3.25, max = 3.25)]
    #[libertas_chart3d_guide(target = x, source = scale, tickStep = 1)]
    pub x: f32,
    /// Y coordinate
    /// Shared surface and contour y position.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = contour_y, kind = linear, min = -3.25, max = 3.25)]
    #[libertas_chart3d_guide(target = y, source = scale, tickStep = 1)]
    pub y: f32,
    /// Height
    /// Constant height of this contour.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = contour_z, kind = linear, min = 0, max = 6)]
    #[libertas_chart3d_guide(target = z, source = scale, tickStep = 1)]
    pub height: f32,
    /// Contour level
    /// Typed path identity and contour color.
    #[libertas_chart3d_channel(color, detail, tooltip)]
    #[libertas_chart3d_scale(kind = ordinal)]
    #[libertas_chart3d_guide(target = color, source = none)]
    pub level: ContourLevelV1,
    /// Sequence
    /// Vertex order within this closed contour.
    #[libertas_chart3d_channel(order)]
    pub sequence: u16,
    /// Vertex key
    /// Stable contour vertex identity.
    #[libertas_chart3d_channel(key)]
    pub key: u16,
}

/// Contour paths
/// Server-computed, ordered lines drawn over a surface.
#[libertas_chart3d(line)]
pub type ContourLinesChartV1 = Vec<ContourLinePointV1>;

/// Contours on a surface
/// Layers server-computed contour paths after a sampled surface in one scene.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart3d(layer)]
pub struct SurfaceContoursChartV1 {
    /// Surface
    /// Sampled piecewise-affine surface drawn first.
    pub surface: ContourSurfaceChartV1,
    /// Contours
    /// Ordered constant-height paths drawn after the surface.
    pub contours: ContourLinesChartV1,
}

/// Non-uniform surface sample
/// One vertex in a rectangular topology whose physical spacing is non-uniform.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct NonuniformSurfacePointV1 {
    /// X coordinate
    /// Non-uniform rectangular-grid x position.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = nonuniform_x, kind = linear)]
    #[libertas_chart3d_guide(target = x, source = scale, tickCount = 6)]
    pub x: f32,
    /// Y coordinate
    /// Non-uniform rectangular-grid y position.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = nonuniform_y, kind = linear)]
    #[libertas_chart3d_guide(target = y, source = scale, tickCount = 6)]
    pub y: f32,
    /// Height
    /// Sampled height at the non-uniform coordinate.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = nonuniform_z, kind = linear, growMin = 0.05, growMax = 0.05)]
    #[libertas_chart3d_guide(target = z, source = scale, tickCount = 5)]
    pub height: f32,
    /// Height color
    /// Duplicated height controlling surface color.
    #[libertas_chart3d_channel(color)]
    #[libertas_chart3d_scale(kind = linear)]
    #[libertas_chart3d_guide(target = color, source = scale)]
    pub height_color: f32,
}

/// Non-uniform surface grid
/// A sparse non-uniform x/y grid containing one intentional topology hole.
#[libertas_chart3d(surface)]
pub type NonuniformSurfaceChartV1 = Vec<NonuniformSurfacePointV1>;

/// Spectrum sample
/// One ordered amplitude sample in a waterfall trace.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct SpectrumPointV1 {
    /// Frequency
    /// Ordered horizontal frequency coordinate.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = spectrum_frequency, kind = linear, min = 0)]
    #[libertas_chart3d_guide(target = x, source = scale, tickStep = 10)]
    pub frequency: f32,
    /// Trace offset
    /// Continuous depth offset separating waterfall traces.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = spectrum_offset, kind = linear, min = 0)]
    #[libertas_chart3d_guide(target = y, source = scale, tickStep = 1)]
    pub trace_offset: f32,
    /// Amplitude
    /// Nonnegative spectrum amplitude.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = spectrum_amplitude, kind = linear, min = 0, zero = true, growMax = 0.08)]
    #[libertas_chart3d_guide(target = z, source = scale, tickCount = 6)]
    pub amplitude: f32,
    /// Trace
    /// Typed detail identity and series legend.
    #[libertas_chart3d_channel(detail, tooltip)]
    #[libertas_chart3d_guide(target = detail, source = value)]
    pub trace: SpectrumTraceV1,
    /// Trace color
    /// Repeats the typed trace as a categorical line color.
    #[libertas_chart3d_channel(color)]
    #[libertas_chart3d_scale(kind = ordinal)]
    #[libertas_chart3d_guide(target = color, source = none)]
    pub trace_color: SpectrumTraceV1,
    /// Sequence
    /// Frequency order within the trace.
    #[libertas_chart3d_channel(order)]
    pub sequence: u16,
    /// Sample key
    /// Stable sample identity.
    #[libertas_chart3d_channel(key)]
    pub key: u16,
}

/// Waterfall spectrum
/// Multiple typed and ordered line traces separated along the y axis.
#[libertas_chart3d(line)]
pub type SpectrumWaterfallChartV1 = Vec<SpectrumPointV1>;

/// Impulse stem
/// One segment from an explicit zero-plane base to a sampled tip.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct ImpulseStemV1 {
    /// X coordinate
    /// Impulse grid x position.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = impulse_x, kind = linear)]
    #[libertas_chart3d_guide(target = x, source = scale, tickStep = 1)]
    pub x: f32,
    /// Y coordinate
    /// Impulse grid y position.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = impulse_y, kind = linear)]
    #[libertas_chart3d_guide(target = y, source = scale, tickStep = 1)]
    pub y: f32,
    /// Tip height
    /// Sampled stem tip.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = impulse_z, kind = linear, min = -5, max = 5)]
    #[libertas_chart3d_guide(target = z, source = scale, tickStep = 2)]
    pub height: f32,
    /// Base height
    /// Explicit stem base on the zero plane.
    #[libertas_chart3d_channel(z2, tooltip)]
    pub baseline: f32,
    /// Sign
    /// Typed sign encoded by color and exposed in the tooltip.
    #[libertas_chart3d_channel(color, detail, tooltip)]
    #[libertas_chart3d_scale(id = impulse_sign_color, kind = ordinal)]
    #[libertas_chart3d_guide(target = color, source = none)]
    pub sign: ImpulseSignV1,
    /// Stem key
    /// Stable stem identity.
    #[libertas_chart3d_channel(key)]
    pub key: u16,
}

/// Impulse stems
/// Explicit zero-to-value segments without intrinsic tip markers.
#[libertas_chart3d(stem)]
pub type ImpulseStemsV1 = Vec<ImpulseStemV1>;

/// Impulse tip
/// Point marker corresponding exactly to one stem tip.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct ImpulseTipV1 {
    /// X coordinate
    /// Impulse grid x position.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = impulse_x, kind = linear)]
    #[libertas_chart3d_guide(target = x, source = scale, tickStep = 1)]
    pub x: f32,
    /// Y coordinate
    /// Impulse grid y position.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = impulse_y, kind = linear)]
    #[libertas_chart3d_guide(target = y, source = scale, tickStep = 1)]
    pub y: f32,
    /// Height
    /// Sampled tip height.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = impulse_z, kind = linear, min = -5, max = 5)]
    #[libertas_chart3d_guide(target = z, source = scale, tickStep = 2)]
    pub height: f32,
    /// Marker size
    /// Fixed relative size for the tip marker.
    #[libertas_chart3d_channel(size)]
    #[libertas_chart3d_scale(kind = identity)]
    pub marker_size: f32,
    /// Sign
    /// Typed sign encoded by color and exposed in the tooltip.
    #[libertas_chart3d_channel(color, detail, tooltip)]
    #[libertas_chart3d_scale(id = impulse_sign_color, kind = ordinal)]
    #[libertas_chart3d_guide(target = color, source = none)]
    pub sign: ImpulseSignV1,
    /// Tip key
    /// Stable point identity matching the corresponding stem key.
    #[libertas_chart3d_channel(key)]
    pub key: u16,
}

/// Impulse tips
/// Point markers layered exactly at the stem endpoints.
#[libertas_chart3d(point)]
pub type ImpulseTipsV1 = Vec<ImpulseTipV1>;

/// Impulse and stem chart
/// Layers explicit stems first and point tips second in one physical scene.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart3d(layer)]
pub struct ImpulseChartV1 {
    /// Stems
    /// Zero-plane segments drawn first.
    pub stems: ImpulseStemsV1,
    /// Tips
    /// Point markers drawn at the sampled endpoints.
    pub tips: ImpulseTipsV1,
}

/// Parametric surface sample
/// One vertex of a polynomial parameter grid evaluated by the application.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct MathematicalSurfacePointV1 {
    /// X coordinate
    /// Evaluated x position of the parametric function.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = mathematical_x, kind = linear, growMin = 0.04, growMax = 0.04)]
    #[libertas_chart3d_guide(target = x, source = scale, tickCount = 5)]
    pub x: f32,
    /// Y coordinate
    /// Evaluated y position of the parametric function.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = mathematical_y, kind = linear, growMin = 0.04, growMax = 0.04)]
    #[libertas_chart3d_guide(target = y, source = scale, tickCount = 5)]
    pub y: f32,
    /// Z coordinate
    /// Evaluated z position of the parametric function.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = mathematical_z, kind = linear, growMin = 0.04, growMax = 0.04)]
    #[libertas_chart3d_guide(target = z, source = scale, tickCount = 5)]
    pub z: f32,
    /// U parameter
    /// First finite topology coordinate; it owns no visual scale.
    #[libertas_chart3d_channel(u, tooltip)]
    pub u: f32,
    /// V parameter
    /// Second finite topology coordinate; it owns no visual scale.
    #[libertas_chart3d_channel(v, tooltip)]
    pub v: f32,
    /// Parameter color
    /// Duplicated u parameter controlling the surface color.
    #[libertas_chart3d_channel(color)]
    #[libertas_chart3d_scale(kind = linear)]
    #[libertas_chart3d_guide(target = color, source = scale)]
    pub parameter_color: f32,
}

/// Mathematical surface
/// An application-evaluated polynomial parametric surface with explicit u/v topology.
#[libertas_chart3d(surface)]
pub type MathematicalSurfaceChartV1 = Vec<MathematicalSurfacePointV1>;

/// Point-cloud sample
/// One deterministic sample in a moderately dense three-dimensional cloud.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct PointCloudPointV1 {
    /// X coordinate
    /// Horizontal cloud position.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = cloud_x, kind = linear, min = -1, max = 1)]
    #[libertas_chart3d_guide(target = x, source = scale, tickStep = 0.5)]
    pub x: f32,
    /// Y coordinate
    /// Horizontal cloud position.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = cloud_y, kind = linear, min = -1, max = 1)]
    #[libertas_chart3d_guide(target = y, source = scale, tickStep = 0.5)]
    pub y: f32,
    /// Z coordinate
    /// Vertical cloud position.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = cloud_z, kind = linear, min = -1, max = 1)]
    #[libertas_chart3d_guide(target = z, source = scale, tickStep = 0.5)]
    pub z: f32,
    /// Relative weight
    /// Nonnegative point magnitude.
    #[libertas_chart3d_channel(size, tooltip)]
    #[libertas_chart3d_scale(kind = sqrt, min = 0)]
    #[libertas_chart3d_guide(target = size, source = scale)]
    pub weight: f32,
    /// Region
    /// Typed detail identity and localized series legend.
    #[libertas_chart3d_channel(detail, tooltip)]
    #[libertas_chart3d_guide(target = detail, source = value)]
    pub region: CloudRegionV1,
    /// Region color
    /// Repeats the region for categorical point color.
    #[libertas_chart3d_channel(color)]
    #[libertas_chart3d_scale(kind = ordinal)]
    #[libertas_chart3d_guide(target = color, source = none)]
    pub region_color: CloudRegionV1,
    /// Region shape
    /// Repeats the region for a portable automatic point shape.
    #[libertas_chart3d_channel(shape)]
    #[libertas_chart3d_scale(kind = ordinal)]
    #[libertas_chart3d_guide(target = shape, source = none)]
    pub region_shape: CloudRegionV1,
    /// Point key
    /// Stable point identity.
    #[libertas_chart3d_channel(key)]
    pub key: u16,
}

/// 3D point cloud
/// Deterministically generated points with typed region, size, shape, and identity.
#[libertas_chart3d(point)]
pub type PointCloudChartV1 = Vec<PointCloudPointV1>;

/// Multiple-series column
/// One volume column in the heterogeneous layer.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct MultipleSeriesBarV1 {
    /// Period
    /// Continuous base position shared by all layered marks.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = multiple_x, kind = linear, min = 0, padding = 0.35)]
    #[libertas_chart3d_guide(target = x, source = scale, tickStep = 1)]
    pub period: f32,
    /// Lane
    /// Shared categorical depth lane.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = multiple_y, kind = band, padding = 0.2)]
    #[libertas_chart3d_guide(target = y, source = scale)]
    pub lane: MultipleSeriesLaneV1,
    /// Volume
    /// Continuous column height.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = multiple_z, kind = linear, min = 0, zero = true, growMax = 0.08)]
    #[libertas_chart3d_guide(target = z, source = scale, tickCount = 6)]
    pub value: f32,
    /// Series role
    /// Categorical color shared across independently typed marks.
    #[libertas_chart3d_channel(color, tooltip)]
    #[libertas_chart3d_scale(id = multiple_series_color, kind = ordinal)]
    #[libertas_chart3d_guide(target = color, source = scale)]
    pub role: MultipleSeriesRoleV1,
    /// Column key
    /// Stable column identity.
    #[libertas_chart3d_channel(key)]
    pub key: u16,
}

/// Multiple-series columns
/// Volume columns used by the heterogeneous layer.
#[libertas_chart3d(bar)]
pub type MultipleSeriesBarsV1 = Vec<MultipleSeriesBarV1>;

/// Multiple-series trend point
/// One ordered vertex of the trend line.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct MultipleSeriesLinePointV1 {
    /// Period
    /// Continuous position shared with the columns and observations.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = multiple_x, kind = linear, min = 0)]
    #[libertas_chart3d_guide(target = x, source = scale, tickStep = 1)]
    pub period: f32,
    /// Lane
    /// Shared categorical depth lane.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = multiple_y, kind = band)]
    #[libertas_chart3d_guide(target = y, source = scale)]
    pub lane: MultipleSeriesLaneV1,
    /// Trend
    /// Continuous trend height.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = multiple_z, kind = linear, min = 0, zero = true, growMax = 0.08)]
    #[libertas_chart3d_guide(target = z, source = scale, tickCount = 6)]
    pub value: f32,
    /// Series role
    /// Categorical color shared across independently typed marks.
    #[libertas_chart3d_channel(color, tooltip)]
    #[libertas_chart3d_scale(id = multiple_series_color, kind = ordinal)]
    #[libertas_chart3d_guide(target = color, source = scale)]
    pub role: MultipleSeriesRoleV1,
    /// Sequence
    /// Vertex order along the trend.
    #[libertas_chart3d_channel(order)]
    pub sequence: u16,
    /// Vertex key
    /// Stable trend identity.
    #[libertas_chart3d_channel(key)]
    pub key: u16,
}

/// Multiple-series trend
/// Ordered trend line used by the heterogeneous layer.
#[libertas_chart3d(line)]
pub type MultipleSeriesLineV1 = Vec<MultipleSeriesLinePointV1>;

/// Multiple-series observation
/// One independently positioned observation.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct MultipleSeriesPointV1 {
    /// Period
    /// Continuous position shared with the columns and trend.
    #[libertas_chart3d_channel(x, tooltip)]
    #[libertas_chart3d_scale(id = multiple_x, kind = linear, min = 0)]
    #[libertas_chart3d_guide(target = x, source = scale, tickStep = 1)]
    pub period: f32,
    /// Lane
    /// Shared categorical depth lane.
    #[libertas_chart3d_channel(y, tooltip)]
    #[libertas_chart3d_scale(id = multiple_y, kind = band)]
    #[libertas_chart3d_guide(target = y, source = scale)]
    pub lane: MultipleSeriesLaneV1,
    /// Observation
    /// Continuous observed height.
    #[libertas_chart3d_channel(z, tooltip)]
    #[libertas_chart3d_scale(id = multiple_z, kind = linear, min = 0, zero = true, growMax = 0.08)]
    #[libertas_chart3d_guide(target = z, source = scale, tickCount = 6)]
    pub value: f32,
    /// Marker size
    /// Fixed relative size for the observation marker.
    #[libertas_chart3d_channel(size)]
    #[libertas_chart3d_scale(kind = identity)]
    pub marker_size: f32,
    /// Series role
    /// Categorical color shared across independently typed marks.
    #[libertas_chart3d_channel(color, tooltip)]
    #[libertas_chart3d_scale(id = multiple_series_color, kind = ordinal)]
    #[libertas_chart3d_guide(target = color, source = scale)]
    pub role: MultipleSeriesRoleV1,
    /// Point key
    /// Stable observation identity.
    #[libertas_chart3d_channel(key)]
    pub key: u16,
}

/// Multiple-series observations
/// Point observations used by the heterogeneous layer.
#[libertas_chart3d(point)]
pub type MultipleSeriesPointsV1 = Vec<MultipleSeriesPointV1>;

/// Heterogeneous multiple-series layer
/// Combines columns, an ordered line, and individual points in one scene.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart3d(layer)]
pub struct HeterogeneousSeriesChartV1 {
    /// Volume
    /// Columns drawn first.
    pub volume: MultipleSeriesBarsV1,
    /// Trend
    /// Ordered line drawn after the columns.
    pub trend: MultipleSeriesLineV1,
    /// Observations
    /// Points drawn after the trend.
    pub observations: MultipleSeriesPointsV1,
}

/// Multiple 3D series
/// Vertically concatenates homogeneous detail-grouped traces and a heterogeneous layer.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[libertas_chart3d(vconcat)]
pub struct MultipleSeriesChartV1 {
    /// Homogeneous series
    /// Multiple spectrum traces partitioned by detail.
    pub homogeneous: SpectrumWaterfallChartV1,
    /// Heterogeneous series
    /// Columns, a line, and points sharing one physical scene.
    pub heterogeneous: HeterogeneousSeriesChartV1,
}

/// 3D chart gallery protocol
/// Request one complete typed snapshot for any supported 3D chart family.
#[derive(Clone, Debug, PartialEq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
#[allow(clippy::large_enum_variant)]
pub enum Chart3dDemoProtocol {
    /// View 3D scatter
    /// Request independently positioned samples grouped into typed clusters.
    #[libertas_request]
    #[libertas_access_privilege("Read")]
    #[libertas_next_response(ScatterV1)]
    GetScatterV1,
    /// 3D scatter
    /// Independently positioned samples grouped into typed clusters.
    #[libertas_response]
    #[libertas_next_request(GetScatterV1)]
    #[libertas_chart3d(point)]
    #[libertas_chart3d_view(
        projection = perspective,
        azimuth = 42,
        elevation = 27,
        zoom = 1.05,
        orbit = true,
        aspectZ = 0.85,
        pan = true,
        zoomEnabled = true
    )]
    ScatterV1(ScatterChartV1),

    /// View 3D trajectories
    /// Request multiple typed and ordered paths.
    #[libertas_request]
    #[libertas_access_privilege("Read")]
    #[libertas_next_response(TrajectoriesV1)]
    GetTrajectoriesV1,
    /// 3D trajectories
    /// Multiple typed and ordered paths through one scene.
    #[libertas_response]
    #[libertas_next_request(GetTrajectoriesV1)]
    #[libertas_chart3d(line)]
    #[libertas_chart3d_view(
        projection = perspective,
        azimuth = 35,
        elevation = 24,
        zoom = 1,
        orbit = true,
        aspectX = 1.5,
        aspectZ = 0.8,
        pan = true,
        zoomEnabled = true
    )]
    TrajectoriesV1(TrajectoryChartV1),

    /// View all 3D bar forms
    /// Request sparse columns, any-orientation bars, and explicit cuboids.
    #[libertas_request]
    #[libertas_access_privilege("Read")]
    #[libertas_next_response(BarFormsV1)]
    GetBarFormsV1,
    /// 3D bar forms
    /// Three independent scenes covering every valid bar geometry form.
    #[libertas_response]
    #[libertas_next_request(GetBarFormsV1)]
    #[libertas_chart3d(hconcat)]
    #[libertas_chart3d_view(
        projection = perspective,
        azimuth = 45,
        elevation = 28,
        orbit = true,
        pan = true,
        zoomEnabled = true
    )]
    BarFormsV1(BarFormsChartV1),

    /// View Bubble 3D
    /// Request points whose nonnegative magnitude controls their size.
    #[libertas_request]
    #[libertas_access_privilege("Read")]
    #[libertas_next_response(BubblesV1)]
    GetBubblesV1,
    /// Bubble 3D
    /// Quantitatively sized points with typed color, detail, and shape.
    #[libertas_response]
    #[libertas_next_request(GetBubblesV1)]
    #[libertas_chart3d(point)]
    #[libertas_chart3d_view(
        projection = perspective,
        azimuth = 38,
        elevation = 25,
        zoom = 1,
        orbit = true,
        pan = true,
        zoomEnabled = true
    )]
    BubblesV1(BubbleChartV1),

    /// View surface height map
    /// Request a complete rectangular application-sampled grid.
    #[libertas_request]
    #[libertas_access_privilege("Read")]
    #[libertas_next_response(HeightMapV1)]
    GetHeightMapV1,
    /// Surface height map
    /// Complete rectangular topology rendered with filled faces.
    #[libertas_response]
    #[libertas_next_request(GetHeightMapV1)]
    #[libertas_chart3d(surface)]
    #[libertas_chart3d_view(
        projection = perspective,
        azimuth = 45,
        elevation = 34,
        zoom = 1,
        orbit = true,
        aspectZ = 0.65,
        pan = true,
        zoomEnabled = true
    )]
    HeightMapV1(HeightMapChartV1),

    /// View wireframe
    /// Request a rectangular saddle grid rendered only as edges.
    #[libertas_request]
    #[libertas_access_privilege("Read")]
    #[libertas_next_response(WireframeV1)]
    GetWireframeV1,
    /// Wireframe
    /// Rectangular topology rendered without filled faces or internal diagonals.
    #[libertas_response]
    #[libertas_next_request(GetWireframeV1)]
    #[libertas_chart3d(wireframe)]
    #[libertas_chart3d_view(
        projection = orthographic,
        azimuth = 45,
        elevation = 30,
        zoom = 1,
        orbit = true,
        aspectZ = 0.7,
        pan = true,
        zoomEnabled = true
    )]
    WireframeV1(WireframeChartV1),

    /// View contours on a surface
    /// Request server-computed contour paths layered over a sampled surface.
    #[libertas_request]
    #[libertas_access_privilege("Read")]
    #[libertas_next_response(SurfaceContoursV1)]
    GetSurfaceContoursV1,
    /// Contours on a surface
    /// A physical layer with the surface first and stable contour lines second.
    #[libertas_response]
    #[libertas_next_request(GetSurfaceContoursV1)]
    #[libertas_chart3d(layer)]
    #[libertas_chart3d_view(
        projection = perspective,
        azimuth = 40,
        elevation = 33,
        zoom = 1,
        orbit = true,
        aspectZ = 0.6,
        pan = true,
        zoomEnabled = true
    )]
    SurfaceContoursV1(SurfaceContoursChartV1),

    /// View non-uniform surface
    /// Request a non-uniform x/y grid with one explicit topology hole.
    #[libertas_request]
    #[libertas_access_privilege("Read")]
    #[libertas_next_response(NonuniformSurfaceV1)]
    GetNonuniformSurfaceV1,
    /// Non-uniform surface
    /// Uneven rectangular coordinates and a deliberate missing grid pair.
    #[libertas_response]
    #[libertas_next_request(GetNonuniformSurfaceV1)]
    #[libertas_chart3d(surface)]
    #[libertas_chart3d_view(
        projection = perspective,
        azimuth = 48,
        elevation = 29,
        zoom = 1,
        orbit = true,
        aspectX = 1.25,
        aspectZ = 0.75,
        pan = true,
        zoomEnabled = true
    )]
    NonuniformSurfaceV1(NonuniformSurfaceChartV1),

    /// View waterfall spectrum
    /// Request multiple ordered spectrum traces separated in depth.
    #[libertas_request]
    #[libertas_access_privilege("Read")]
    #[libertas_next_response(SpectrumWaterfallV1)]
    GetSpectrumWaterfallV1,
    /// Waterfall spectrum
    /// Typed detail groups and explicit vertex order form the traces.
    #[libertas_response]
    #[libertas_next_request(GetSpectrumWaterfallV1)]
    #[libertas_chart3d(line)]
    #[libertas_chart3d_view(
        projection = perspective,
        azimuth = 32,
        elevation = 24,
        zoom = 1,
        orbit = true,
        aspectX = 1.6,
        aspectY = 1.1,
        aspectZ = 0.8,
        pan = true,
        zoomEnabled = true
    )]
    SpectrumWaterfallV1(SpectrumWaterfallChartV1),

    /// View impulse stems
    /// Request explicit stems layered with point markers at their tips.
    #[libertas_request]
    #[libertas_access_privilege("Read")]
    #[libertas_next_response(ImpulseStemsV1)]
    GetImpulseStemsV1,
    /// Impulse stems
    /// Zero-plane segments followed by corresponding endpoint markers.
    #[libertas_response]
    #[libertas_next_request(GetImpulseStemsV1)]
    #[libertas_chart3d(layer)]
    #[libertas_chart3d_view(
        projection = perspective,
        azimuth = 42,
        elevation = 30,
        zoom = 1,
        orbit = true,
        aspectZ = 0.9,
        pan = true,
        zoomEnabled = true
    )]
    ImpulseStemsV1(ImpulseChartV1),

    /// View mathematical surface
    /// Request an application-evaluated polynomial parametric surface.
    #[libertas_request]
    #[libertas_access_privilege("Read")]
    #[libertas_next_response(MathematicalSurfaceV1)]
    GetMathematicalSurfaceV1,
    /// Mathematical surface
    /// Explicit x/y/z samples connected by finite u/v topology coordinates.
    #[libertas_response]
    #[libertas_next_request(GetMathematicalSurfaceV1)]
    #[libertas_chart3d(surface)]
    #[libertas_chart3d_view(
        projection = perspective,
        azimuth = 50,
        elevation = 22,
        zoom = 1,
        orbit = true,
        aspectZ = 0.8,
        pan = true,
        zoomEnabled = true
    )]
    MathematicalSurfaceV1(MathematicalSurfaceChartV1),

    /// View 3D point cloud
    /// Request a deterministic moderately dense cloud with stable keys.
    #[libertas_request]
    #[libertas_access_privilege("Read")]
    #[libertas_next_response(PointCloudV1)]
    GetPointCloudV1,
    /// 3D point cloud
    /// Typed point regions with quantitative size and portable shape.
    #[libertas_response]
    #[libertas_next_request(GetPointCloudV1)]
    #[libertas_chart3d(point)]
    #[libertas_chart3d_view(
        projection = perspective,
        azimuth = 45,
        elevation = 25,
        zoom = 1,
        orbit = true,
        pan = true,
        zoomEnabled = true
    )]
    PointCloudV1(PointCloudChartV1),

    /// View multiple 3D series
    /// Request homogeneous traces and a heterogeneous layer in two rows.
    #[libertas_request]
    #[libertas_access_privilege("Read")]
    #[libertas_next_response(MultipleSeriesV1)]
    GetMultipleSeriesV1,
    /// Multiple 3D series
    /// A vertical composition of detail-grouped lines and layered mark kinds.
    #[libertas_response]
    #[libertas_next_request(GetMultipleSeriesV1)]
    #[libertas_chart3d(vconcat)]
    #[libertas_chart3d_view(
        projection = perspective,
        azimuth = 38,
        elevation = 26,
        zoom = 1,
        orbit = true,
        pan = true,
        zoomEnabled = true
    )]
    MultipleSeriesV1(MultipleSeriesChartV1),
}

fn scatter() -> ScatterChartV1 {
    let clusters = [
        ScatterClusterV1::NorthRidge,
        ScatterClusterV1::CentralBasin,
        ScatterClusterV1::SouthShelf,
    ];
    let mut points = Vec::with_capacity(clusters.len() * 9);

    for (cluster_index, cluster) in clusters.into_iter().enumerate() {
        let (center_x, center_y, center_z) = match cluster {
            ScatterClusterV1::NorthRidge => (-2.8, 2.6, 2.2),
            ScatterClusterV1::CentralBasin => (0.2, 0.0, -1.4),
            ScatterClusterV1::SouthShelf => (3.0, -2.4, 1.0),
        };
        for offset in 0..9 {
            let local_x = (offset % 3) as f32 - 1.0;
            let local_y = (offset / 3) as f32 - 1.0;
            let local_z = (local_x * local_y) * 0.32 + (offset as f32 - 4.0) * 0.04;
            points.push(ScatterPointV1 {
                x: center_x + local_x * 0.72,
                y: center_y + local_y * 0.66,
                z: center_z + local_z,
                cluster,
                key: (cluster_index * 9 + offset) as u16,
            });
        }
    }

    points
}

fn trajectories() -> TrajectoryChartV1 {
    let paths = [
        TrajectoryV1::Survey,
        TrajectoryV1::Forecast,
        TrajectoryV1::Limit,
    ];
    let mut points = Vec::with_capacity(paths.len() * 13);

    for (path_index, trajectory) in paths.into_iter().enumerate() {
        for sequence in 0..=12_u16 {
            let t = f32::from(sequence);
            let centered = t - 6.0;
            let (y, z) = match trajectory {
                TrajectoryV1::Survey => (
                    centered * centered * 0.075 - 2.1,
                    0.35 * t + centered * centered * 0.025,
                ),
                TrajectoryV1::Forecast => (
                    centered * centered * 0.055 - 0.5 + t * 0.08,
                    0.31 * t + 1.0 - centered * centered * 0.018,
                ),
                TrajectoryV1::Limit => (
                    2.2 - centered * centered * 0.04,
                    4.7 - centered * centered * 0.035,
                ),
            };
            points.push(TrajectoryPointV1 {
                x: t,
                y,
                z,
                trajectory,
                trajectory_color: trajectory,
                sequence,
                key: (path_index * 13 + usize::from(sequence)) as u16,
            });
        }
    }

    points
}

fn sparse_columns() -> SparseColumnsChartV1 {
    let quarters = [QuarterV1::Q1, QuarterV1::Q2, QuarterV1::Q3];
    let regions = [SalesRegionV1::Americas, SalesRegionV1::Europe];
    let revenue = [[42.0, 35.0], [48.0, 41.0], [57.0, 46.0]];
    let mut columns = Vec::with_capacity(6);

    for (quarter_index, quarter) in quarters.into_iter().enumerate() {
        for (region_index, region) in regions.into_iter().enumerate() {
            columns.push(SparseColumnV1 {
                quarter,
                region,
                revenue_millions: revenue[quarter_index][region_index],
                key: (quarter_index * regions.len() + region_index) as u16,
            });
        }
    }

    columns
}

fn horizontal_legacy_bars() -> HorizontalLegacyBarsChartV1 {
    let products = [
        ProductFamilyV1::Home,
        ProductFamilyV1::Business,
        ProductFamilyV1::Platform,
    ];
    let scenarios = [BarScenarioV1::Actual, BarScenarioV1::Plan];
    let values = [[72.0, 80.0], [61.0, 68.0], [84.0, 88.0]];
    let mut bars = Vec::with_capacity(6);

    for (product_index, product) in products.into_iter().enumerate() {
        for (scenario_index, scenario) in scenarios.into_iter().enumerate() {
            bars.push(HorizontalLegacyBarV1 {
                value: values[product_index][scenario_index],
                baseline: 0.0,
                product,
                scenario,
                key: (product_index * scenarios.len() + scenario_index) as u16,
            });
        }
    }

    bars
}

fn explicit_cuboids() -> ExplicitCuboidsChartV1 {
    vec![
        ExplicitCuboidV1 {
            x: -3.0,
            x2: -1.8,
            y: -2.2,
            y2: -0.7,
            z: 3.6,
            z2: 0.4,
            key: 0,
        },
        ExplicitCuboidV1 {
            x: -0.8,
            x2: 0.9,
            y: -0.4,
            y2: 1.0,
            z: 5.2,
            z2: 1.1,
            key: 1,
        },
        ExplicitCuboidV1 {
            x: 1.4,
            x2: 2.8,
            y: 1.2,
            y2: 2.5,
            z: 4.4,
            z2: -0.6,
            key: 2,
        },
    ]
}

fn bar_forms() -> BarFormsChartV1 {
    BarFormsChartV1 {
        sparse_columns: sparse_columns(),
        horizontal_bars: horizontal_legacy_bars(),
        explicit_cuboids: explicit_cuboids(),
    }
}

fn bubbles() -> BubbleChartV1 {
    let segments = [
        BubbleSegmentV1::Core,
        BubbleSegmentV1::Growth,
        BubbleSegmentV1::Research,
    ];
    let classes = [
        BubbleClassV1::Consumer,
        BubbleClassV1::Industrial,
        BubbleClassV1::Scientific,
    ];
    let mut points = Vec::with_capacity(12);

    for key in 0..12_u16 {
        let index = usize::from(key);
        let segment = segments[index % segments.len()];
        let class = classes[(index / segments.len()) % classes.len()];
        let band = (index / 3) as f32;
        let within = (index % 3) as f32;
        points.push(BubblePointV1 {
            reach: 18.0 + band * 18.0 + within * 4.5,
            growth_percent: -4.0 + within * 9.0 + band * 2.5,
            retention_percent: 58.0 + band * 7.0 + within * 5.0,
            annual_value: 4.0 + (index as f32 + 1.0) * (1.5 + within),
            segment,
            class,
            key,
        });
    }

    points
}

fn height_map() -> HeightMapChartV1 {
    let mut points = Vec::with_capacity(81);
    for x_index in -4_i16..=4 {
        let x = f32::from(x_index);
        for y_index in -4_i16..=4 {
            let y = f32::from(y_index);
            let height = 11.0 - 0.55 * (x * x + y * y) + 0.18 * x * y;
            points.push(HeightMapPointV1 {
                x,
                y,
                height,
                height_color: height,
            });
        }
    }
    points
}

fn wireframe() -> WireframeChartV1 {
    let mut points = Vec::with_capacity(81);
    for x_index in -4_i16..=4 {
        let x = f32::from(x_index);
        for y_index in -4_i16..=4 {
            let y = f32::from(y_index);
            let height = (x * x - y * y) * 0.28 + x * y * 0.08;
            points.push(WireframePointV1 {
                x,
                y,
                height,
                height_color: height,
            });
        }
    }
    points
}

fn contour_surface() -> ContourSurfaceChartV1 {
    let mut points = Vec::with_capacity(169);
    for x_index in -6_i16..=6 {
        let x = f32::from(x_index) * 0.5;
        for y_index in -6_i16..=6 {
            let y = f32::from(y_index) * 0.5;
            // This function is affine within every grid quadrant, and both
            // axes are grid lines. The renderer's fixed triangles therefore
            // coincide exactly with the authored surface between vertices.
            let height = x.abs() + y.abs();
            points.push(ContourSurfacePointV1 {
                x,
                y,
                height,
                surface_color: height,
            });
        }
    }
    points
}

fn contour_lines() -> ContourLinesChartV1 {
    // Every diamond edge satisfies z = |x| + |y| on the exact piecewise-planar
    // mesh. Drawing these rows after the surface invokes the specified
    // equal-depth layer tie-break instead of hiding analytic curves below it.
    const DIAMOND: [(f32, f32); 17] = [
        (1.0, 0.0),
        (0.75, 0.25),
        (0.5, 0.5),
        (0.25, 0.75),
        (0.0, 1.0),
        (-0.25, 0.75),
        (-0.5, 0.5),
        (-0.75, 0.25),
        (-1.0, 0.0),
        (-0.75, -0.25),
        (-0.5, -0.5),
        (-0.25, -0.75),
        (0.0, -1.0),
        (0.25, -0.75),
        (0.5, -0.5),
        (0.75, -0.25),
        (1.0, 0.0),
    ];
    let levels = [
        (ContourLevelV1::One, 1.0, 1.0),
        (ContourLevelV1::Two, 2.0, 2.0),
        (ContourLevelV1::Three, 3.0, 3.0),
    ];
    let mut points = Vec::with_capacity(levels.len() * DIAMOND.len());

    for (level_index, (level, radius, height)) in levels.into_iter().enumerate() {
        for (sequence, (unit_x, unit_y)) in DIAMOND.into_iter().enumerate() {
            points.push(ContourLinePointV1 {
                x: radius * unit_x,
                y: radius * unit_y,
                height,
                level,
                sequence: sequence as u16,
                key: (level_index * DIAMOND.len() + sequence) as u16,
            });
        }
    }

    points
}

fn surface_contours() -> SurfaceContoursChartV1 {
    SurfaceContoursChartV1 {
        surface: contour_surface(),
        contours: contour_lines(),
    }
}

fn nonuniform_surface() -> NonuniformSurfaceChartV1 {
    const X_VALUES: [f32; 7] = [-4.0, -2.5, -1.0, -0.25, 0.75, 2.0, 4.0];
    const Y_VALUES: [f32; 6] = [-3.0, -1.4, -0.2, 0.9, 2.2, 3.0];
    let mut points = Vec::with_capacity(X_VALUES.len() * Y_VALUES.len() - 1);

    for (x_index, x) in X_VALUES.into_iter().enumerate() {
        for (y_index, y) in Y_VALUES.into_iter().enumerate() {
            if x_index == 3 && y_index == 2 {
                continue;
            }
            let height = x * y * 0.24 + x * x * 0.11 - y * y * 0.08;
            points.push(NonuniformSurfacePointV1 {
                x,
                y,
                height,
                height_color: height,
            });
        }
    }

    points
}

fn positive_part(value: f32) -> f32 {
    if value > 0.0 { value } else { 0.0 }
}

fn spectrum_waterfall() -> SpectrumWaterfallChartV1 {
    let traces = [
        SpectrumTraceV1::Baseline,
        SpectrumTraceV1::Morning,
        SpectrumTraceV1::Afternoon,
        SpectrumTraceV1::Evening,
    ];
    let mut points = Vec::with_capacity(traces.len() * 41);

    for (trace_index, trace) in traces.into_iter().enumerate() {
        let offset = trace_index as f32;
        let primary_center = 8.0 + offset * 3.0;
        let secondary_center = 29.0 - offset * 2.0;
        for sequence in 0..=40_u16 {
            let frequency = f32::from(sequence);
            let primary = positive_part(9.0 - (frequency - primary_center).abs()) * 0.9;
            let secondary = positive_part(6.0 - (frequency - secondary_center).abs()) * 0.55;
            let ripple = f32::from((sequence + trace_index as u16) % 5) * 0.12;
            points.push(SpectrumPointV1 {
                frequency,
                trace_offset: offset,
                amplitude: primary + secondary + ripple + offset * 0.3,
                trace,
                trace_color: trace,
                sequence,
                key: (trace_index * 41 + usize::from(sequence)) as u16,
            });
        }
    }

    points
}

fn impulses() -> ImpulseChartV1 {
    let mut stems = Vec::with_capacity(25);
    let mut tips = Vec::with_capacity(25);
    let mut key = 0_u16;

    for x_index in -2_i16..=2 {
        let x = f32::from(x_index);
        for y_index in -2_i16..=2 {
            let y = f32::from(y_index);
            let height = (x * x - y * y) * 0.55 + x * 0.45 - y * 0.25;
            let sign = if height < 0.0 {
                ImpulseSignV1::Negative
            } else {
                ImpulseSignV1::Nonnegative
            };
            stems.push(ImpulseStemV1 {
                x,
                y,
                height,
                baseline: 0.0,
                sign,
                key,
            });
            tips.push(ImpulseTipV1 {
                x,
                y,
                height,
                marker_size: 1.4,
                sign,
                key,
            });
            key += 1;
        }
    }

    ImpulseChartV1 { stems, tips }
}

fn mathematical_surface() -> MathematicalSurfaceChartV1 {
    const U_VALUES: [f32; 9] = [-2.0, -1.5, -1.0, -0.5, 0.0, 0.5, 1.0, 1.5, 2.0];
    const V_VALUES: [f32; 5] = [-1.0, -0.5, 0.0, 0.5, 1.0];
    let mut points = Vec::with_capacity(U_VALUES.len() * V_VALUES.len());

    for u in U_VALUES {
        for v in V_VALUES {
            points.push(MathematicalSurfacePointV1 {
                x: u * u - v * v,
                y: 2.0 * u * v,
                z: u + 0.25 * v,
                u,
                v,
                parameter_color: u,
            });
        }
    }

    points
}

fn next_random(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *state
}

fn random_coordinate(state: &mut u32) -> f32 {
    let fraction = (next_random(state) >> 8) as f32 / 16_777_215.0;
    fraction * 2.0 - 1.0
}

fn point_cloud() -> PointCloudChartV1 {
    let mut state = 0xC0FF_EE11_u32;
    let mut points = Vec::with_capacity(96);

    for key in 0..96_u16 {
        let x = random_coordinate(&mut state);
        let y = random_coordinate(&mut state);
        let z = random_coordinate(&mut state);
        let region = match (z > 0.0, x >= 0.0) {
            (true, true) => CloudRegionV1::UpperEast,
            (true, false) => CloudRegionV1::UpperWest,
            (false, true) => CloudRegionV1::LowerEast,
            (false, false) => CloudRegionV1::LowerWest,
        };
        let weight = 0.35 + (x.abs() + y.abs() + z.abs()) * 0.9;
        points.push(PointCloudPointV1 {
            x,
            y,
            z,
            weight,
            region,
            region_color: region,
            region_shape: region,
            key,
        });
    }

    points
}

fn heterogeneous_series() -> HeterogeneousSeriesChartV1 {
    const VOLUME: [f32; 8] = [7.0, 9.0, 8.0, 12.0, 11.0, 15.0, 14.0, 17.0];
    const TREND: [f32; 8] = [7.5, 8.0, 9.0, 10.0, 11.5, 13.0, 14.5, 16.0];
    const OBSERVATION: [f32; 8] = [6.8, 9.4, 8.5, 11.7, 12.2, 14.6, 13.8, 17.3];
    let mut volume = Vec::with_capacity(8);
    let mut trend = Vec::with_capacity(8);
    let mut observations = Vec::with_capacity(8);

    for index in 0..8_u16 {
        let position = f32::from(index);
        let array_index = usize::from(index);
        volume.push(MultipleSeriesBarV1 {
            period: position,
            lane: MultipleSeriesLaneV1::Main,
            value: VOLUME[array_index],
            role: MultipleSeriesRoleV1::Volume,
            key: index,
        });
        trend.push(MultipleSeriesLinePointV1 {
            period: position,
            lane: MultipleSeriesLaneV1::Main,
            value: TREND[array_index],
            role: MultipleSeriesRoleV1::Trend,
            sequence: index,
            key: index,
        });
        observations.push(MultipleSeriesPointV1 {
            period: position,
            lane: MultipleSeriesLaneV1::Main,
            value: OBSERVATION[array_index],
            marker_size: 1.7,
            role: MultipleSeriesRoleV1::Observation,
            key: index,
        });
    }

    HeterogeneousSeriesChartV1 {
        volume,
        trend,
        observations,
    }
}

fn multiple_series() -> MultipleSeriesChartV1 {
    MultipleSeriesChartV1 {
        homogeneous: spectrum_waterfall(),
        heterogeneous: heterogeneous_series(),
    }
}

fn response_for(request: Chart3dDemoProtocol) -> Option<Chart3dDemoProtocol> {
    Some(match request {
        Chart3dDemoProtocol::GetScatterV1 => Chart3dDemoProtocol::ScatterV1(scatter()),
        Chart3dDemoProtocol::GetTrajectoriesV1 => {
            Chart3dDemoProtocol::TrajectoriesV1(trajectories())
        }
        Chart3dDemoProtocol::GetBarFormsV1 => Chart3dDemoProtocol::BarFormsV1(bar_forms()),
        Chart3dDemoProtocol::GetBubblesV1 => Chart3dDemoProtocol::BubblesV1(bubbles()),
        Chart3dDemoProtocol::GetHeightMapV1 => Chart3dDemoProtocol::HeightMapV1(height_map()),
        Chart3dDemoProtocol::GetWireframeV1 => Chart3dDemoProtocol::WireframeV1(wireframe()),
        Chart3dDemoProtocol::GetSurfaceContoursV1 => {
            Chart3dDemoProtocol::SurfaceContoursV1(surface_contours())
        }
        Chart3dDemoProtocol::GetNonuniformSurfaceV1 => {
            Chart3dDemoProtocol::NonuniformSurfaceV1(nonuniform_surface())
        }
        Chart3dDemoProtocol::GetSpectrumWaterfallV1 => {
            Chart3dDemoProtocol::SpectrumWaterfallV1(spectrum_waterfall())
        }
        Chart3dDemoProtocol::GetImpulseStemsV1 => Chart3dDemoProtocol::ImpulseStemsV1(impulses()),
        Chart3dDemoProtocol::GetMathematicalSurfaceV1 => {
            Chart3dDemoProtocol::MathematicalSurfaceV1(mathematical_surface())
        }
        Chart3dDemoProtocol::GetPointCloudV1 => Chart3dDemoProtocol::PointCloudV1(point_cloud()),
        Chart3dDemoProtocol::GetMultipleSeriesV1 => {
            Chart3dDemoProtocol::MultipleSeriesV1(multiple_series())
        }
        _ => return None,
    })
}

fn handle_gallery_request(
    endpoint: LibertasEndpoint,
    opcode: u8,
    message: Option<Chart3dDemoProtocol>,
    _context: &mut Box<dyn Any>,
    transaction_id: u32,
    peer: u32,
) -> LibertasEndpointStatus {
    if opcode != OP_ENDPOINT_REQ {
        return LibertasEndpointStatus::InvalidMessage;
    }
    let Some(response) = message.and_then(response_for) else {
        return LibertasEndpointStatus::InvalidMessage;
    };
    libertas_endpoint_response(endpoint, &response, transaction_id, peer);
    LibertasEndpointStatus::Success
}

/// 3D chart gallery
/// Lets people explore every 3D family in the Libertas V1 chart contract.
/// The gallery serves thirteen complete, deterministic snapshots covering all
/// six primitive marks and all three composition forms. It includes all three
/// bar geometries, rectangular and parameter-grid surfaces, server-computed
/// contours, homogeneous and heterogeneous series, typed labels and details,
/// explicit ordering and keys, shared scales, independent scenes, and several
/// camera configurations. Every mathematical transformation is evaluated by
/// this application; the client receives only typed rows and chart metadata.
/// [DefaultTaskName]
/// 3D chart gallery
#[libertas_export]
pub fn chart3d_demo(
    /*
     * Gallery endpoint
     * Select this endpoint and request any 3D chart family.
     * [DefaultText]
     * 3D chart gallery
     */
    #[libertas_endpoint_schema(Chart3dDemoProtocol)]
    #[libertas_endpoint_server]
    gallery: LibertasEndpoint,
) {
    libertas_register_endpoint_listener::<Chart3dDemoProtocol, _>(
        gallery,
        handle_gallery_request,
        Box::new(()),
    );
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    fn requests() -> Vec<Chart3dDemoProtocol> {
        vec![
            Chart3dDemoProtocol::GetScatterV1,
            Chart3dDemoProtocol::GetTrajectoriesV1,
            Chart3dDemoProtocol::GetBarFormsV1,
            Chart3dDemoProtocol::GetBubblesV1,
            Chart3dDemoProtocol::GetHeightMapV1,
            Chart3dDemoProtocol::GetWireframeV1,
            Chart3dDemoProtocol::GetSurfaceContoursV1,
            Chart3dDemoProtocol::GetNonuniformSurfaceV1,
            Chart3dDemoProtocol::GetSpectrumWaterfallV1,
            Chart3dDemoProtocol::GetImpulseStemsV1,
            Chart3dDemoProtocol::GetMathematicalSurfaceV1,
            Chart3dDemoProtocol::GetPointCloudV1,
            Chart3dDemoProtocol::GetMultipleSeriesV1,
        ]
    }

    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() < 0.000_01
    }

    #[test]
    fn every_gallery_request_has_a_nonempty_response() {
        let requests = requests();
        assert_eq!(requests.len(), 13);
        for request in requests {
            let response = response_for(request).expect("request must have a response");
            assert!(!response.to_avro().is_empty());
        }
    }

    #[test]
    fn every_gallery_response_round_trips_through_avro() {
        for request in requests() {
            let response = response_for(request).expect("request must have a response");
            let encoded = response.to_avro();
            let decoded = Chart3dDemoProtocol::from_avro(&encoded).expect("valid Avro response");
            assert_eq!(decoded, response);
        }
    }

    #[test]
    fn every_gallery_snapshot_is_deterministic() {
        for request in requests() {
            let repeated_request = request.clone();
            assert_eq!(response_for(request), response_for(repeated_request));
        }
    }

    #[test]
    fn protocol_variant_order_is_stable_and_append_only() {
        for (pair_index, request) in requests().into_iter().enumerate() {
            let request_bytes = request.clone().to_avro();
            assert_eq!(request_bytes.first(), Some(&((pair_index * 4) as u8)));
            assert_eq!(
                Chart3dDemoProtocol::from_avro(&request_bytes),
                Ok(request.clone())
            );

            let response = response_for(request).expect("request must have a response");
            let response_bytes = response.to_avro();
            assert_eq!(response_bytes.first(), Some(&((pair_index * 4 + 2) as u8)));
        }
    }

    #[test]
    fn response_values_cannot_be_used_as_requests() {
        for request in requests() {
            let response = response_for(request).expect("request must have a response");
            assert!(response_for(response).is_none());
        }
    }

    #[test]
    fn endpoint_callback_rejects_wrong_operations_and_messages() {
        let mut context: Box<dyn Any> = Box::new(());
        assert_eq!(
            handle_gallery_request(
                7,
                0xff,
                Some(Chart3dDemoProtocol::GetScatterV1),
                &mut context,
                11,
                13,
            ),
            LibertasEndpointStatus::InvalidMessage
        );
        assert_eq!(
            handle_gallery_request(7, OP_ENDPOINT_REQ, None, &mut context, 11, 13),
            LibertasEndpointStatus::InvalidMessage
        );
        assert_eq!(
            handle_gallery_request(
                7,
                OP_ENDPOINT_REQ,
                Some(Chart3dDemoProtocol::ScatterV1(scatter())),
                &mut context,
                11,
                13,
            ),
            LibertasEndpointStatus::InvalidMessage
        );
    }

    #[test]
    fn scatter_is_finite_clustered_and_uniquely_keyed() {
        let points = scatter();
        assert_eq!(points.len(), 27);
        for (index, point) in points.iter().enumerate() {
            assert!(point.x.is_finite() && point.y.is_finite() && point.z.is_finite());
            assert_eq!(usize::from(point.key), index);
            let expected_cluster = match index / 9 {
                0 => ScatterClusterV1::NorthRidge,
                1 => ScatterClusterV1::CentralBasin,
                _ => ScatterClusterV1::SouthShelf,
            };
            assert_eq!(point.cluster, expected_cluster);
        }
    }

    #[test]
    fn trajectories_are_partitioned_and_strictly_ordered() {
        let points = trajectories();
        assert_eq!(points.len(), 39);
        let expected = [
            TrajectoryV1::Survey,
            TrajectoryV1::Forecast,
            TrajectoryV1::Limit,
        ];
        for (path, trajectory) in points.chunks_exact(13).zip(expected) {
            for (sequence, point) in path.iter().enumerate() {
                assert_eq!(point.trajectory, trajectory);
                assert_eq!(point.trajectory_color, trajectory);
                assert_eq!(usize::from(point.sequence), sequence);
                assert!(point.x.is_finite() && point.y.is_finite() && point.z.is_finite());
            }
        }
        for (index, point) in points.iter().enumerate() {
            assert_eq!(usize::from(point.key), index);
        }
    }

    #[test]
    fn all_three_bar_forms_have_valid_authored_geometry() {
        let chart = bar_forms();
        assert_eq!(chart.sparse_columns.len(), 6);
        assert!(
            chart
                .sparse_columns
                .iter()
                .all(|column| column.revenue_millions.is_finite() && column.revenue_millions > 0.0)
        );
        assert!(
            chart
                .sparse_columns
                .iter()
                .enumerate()
                .all(|(index, column)| usize::from(column.key) == index)
        );

        assert_eq!(chart.horizontal_bars.len(), 6);
        assert!(chart.horizontal_bars.iter().all(|bar| {
            bar.value.is_finite() && bar.baseline.is_finite() && bar.value > bar.baseline
        }));
        assert!(
            chart
                .horizontal_bars
                .iter()
                .enumerate()
                .all(|(index, bar)| usize::from(bar.key) == index)
        );

        assert_eq!(chart.explicit_cuboids.len(), 3);
        assert!(chart.explicit_cuboids.iter().all(|cuboid| {
            [
                cuboid.x, cuboid.x2, cuboid.y, cuboid.y2, cuboid.z, cuboid.z2,
            ]
            .into_iter()
            .all(f32::is_finite)
                && cuboid.x != cuboid.x2
                && cuboid.y != cuboid.y2
                && cuboid.z != cuboid.z2
        }));
        assert!(
            chart
                .explicit_cuboids
                .iter()
                .enumerate()
                .all(|(index, cuboid)| usize::from(cuboid.key) == index)
        );
    }

    #[test]
    fn bubbles_have_nonnegative_finite_sizes_and_typed_encodings() {
        let points = bubbles();
        assert_eq!(points.len(), 12);
        assert!(points.iter().all(|point| {
            point.reach.is_finite()
                && point.growth_percent.is_finite()
                && point.retention_percent.is_finite()
                && point.annual_value.is_finite()
                && point.annual_value >= 0.0
                && (0.0..=100.0).contains(&point.retention_percent)
        }));
        assert!(
            points
                .iter()
                .enumerate()
                .all(|(index, point)| usize::from(point.key) == index)
        );
        assert!(
            points
                .iter()
                .any(|point| point.class == BubbleClassV1::Scientific)
        );
        assert!(
            points
                .iter()
                .any(|point| point.segment == BubbleSegmentV1::Research)
        );
    }

    #[test]
    fn rectangular_surface_grids_are_complete_and_unique() {
        let height = height_map();
        let wire = wireframe();
        assert_eq!(height.len(), 81);
        assert_eq!(wire.len(), 81);

        for (index, point) in height.iter().enumerate() {
            assert_eq!(point.x, (index / 9) as f32 - 4.0);
            assert_eq!(point.y, (index % 9) as f32 - 4.0);
            assert!(
                point.x.is_finite()
                    && point.y.is_finite()
                    && point.height.is_finite()
                    && point.height_color.is_finite()
            );
            assert!(close(point.height, point.height_color));
            assert!(
                !height[..index]
                    .iter()
                    .any(|prior| prior.x == point.x && prior.y == point.y)
            );
        }
        for (index, point) in wire.iter().enumerate() {
            assert_eq!(point.x, (index / 9) as f32 - 4.0);
            assert_eq!(point.y, (index % 9) as f32 - 4.0);
            assert!(
                point.x.is_finite()
                    && point.y.is_finite()
                    && point.height.is_finite()
                    && point.height_color.is_finite()
            );
            assert!(close(point.height, point.height_color));
            assert!(
                !wire[..index]
                    .iter()
                    .any(|prior| prior.x == point.x && prior.y == point.y)
            );
        }
    }

    #[test]
    fn contour_paths_are_closed_ordered_and_match_the_sampled_function() {
        let chart = surface_contours();
        assert_eq!(chart.surface.len(), 169);
        assert_eq!(chart.contours.len(), 51);
        for (index, surface_point) in chart.surface.iter().enumerate() {
            assert_eq!(surface_point.x, (index / 13) as f32 * 0.5 - 3.0);
            assert_eq!(surface_point.y, (index % 13) as f32 * 0.5 - 3.0);
            assert!(close(
                surface_point.height,
                surface_point.x.abs() + surface_point.y.abs()
            ));
            assert!(close(surface_point.height, surface_point.surface_color));
        }

        let expected = [
            (ContourLevelV1::One, 1.0),
            (ContourLevelV1::Two, 2.0),
            (ContourLevelV1::Three, 3.0),
        ];
        for (path, (level, height)) in chart.contours.chunks_exact(17).zip(expected) {
            assert_eq!(path.first().unwrap().level, level);
            assert!(close(path.first().unwrap().x, path.last().unwrap().x));
            assert!(close(path.first().unwrap().y, path.last().unwrap().y));
            for (sequence, point) in path.iter().enumerate() {
                assert_eq!(point.level, level);
                assert_eq!(usize::from(point.sequence), sequence);
                assert!(close(point.height, height));
                assert!(close(point.x.abs() + point.y.abs(), height));
            }
        }
        for (index, point) in chart.contours.iter().enumerate() {
            assert_eq!(usize::from(point.key), index);
        }
    }

    #[test]
    fn nonuniform_grid_has_unique_pairs_and_one_intentional_hole() {
        let points = nonuniform_surface();
        assert_eq!(points.len(), 41);
        assert!(
            !points
                .iter()
                .any(|point| close(point.x, -0.25) && close(point.y, -0.2))
        );
        const X_VALUES: [f32; 7] = [-4.0, -2.5, -1.0, -0.25, 0.75, 2.0, 4.0];
        const Y_VALUES: [f32; 6] = [-3.0, -1.4, -0.2, 0.9, 2.2, 3.0];
        for (x_index, x) in X_VALUES.into_iter().enumerate() {
            for (y_index, y) in Y_VALUES.into_iter().enumerate() {
                let matches = points
                    .iter()
                    .filter(|point| point.x == x && point.y == y)
                    .count();
                assert_eq!(matches, usize::from(!(x_index == 3 && y_index == 2)));
            }
        }
        for (index, point) in points.iter().enumerate() {
            assert!(
                point.x.is_finite()
                    && point.y.is_finite()
                    && point.height.is_finite()
                    && point.height_color.is_finite()
            );
            assert!(close(
                point.height,
                point.x * point.y * 0.24 + point.x * point.x * 0.11 - point.y * point.y * 0.08
            ));
            assert!(close(point.height, point.height_color));
            assert!(
                !points[..index]
                    .iter()
                    .any(|prior| prior.x == point.x && prior.y == point.y)
            );
        }
    }

    #[test]
    fn spectrum_traces_are_nonnegative_and_strictly_ordered() {
        let points = spectrum_waterfall();
        assert_eq!(points.len(), 164);
        let traces = [
            SpectrumTraceV1::Baseline,
            SpectrumTraceV1::Morning,
            SpectrumTraceV1::Afternoon,
            SpectrumTraceV1::Evening,
        ];
        for (trace_index, (path, trace)) in points.chunks_exact(41).zip(traces).enumerate() {
            for (sequence, point) in path.iter().enumerate() {
                assert_eq!(point.trace, trace);
                assert_eq!(point.trace_color, trace);
                assert_eq!(usize::from(point.sequence), sequence);
                assert_eq!(point.frequency, sequence as f32);
                assert_eq!(point.trace_offset, trace_index as f32);
                assert!(point.amplitude.is_finite() && point.amplitude >= 0.0);
            }
        }
        for (index, point) in points.iter().enumerate() {
            assert_eq!(usize::from(point.key), index);
        }
    }

    #[test]
    fn every_impulse_tip_matches_one_explicit_stem() {
        let chart = impulses();
        assert_eq!(chart.stems.len(), 25);
        assert_eq!(chart.tips.len(), 25);
        for (index, (stem, tip)) in chart.stems.iter().zip(&chart.tips).enumerate() {
            assert_eq!(usize::from(stem.key), index);
            assert_eq!(stem.key, tip.key);
            assert!(close(stem.x, tip.x));
            assert!(close(stem.y, tip.y));
            assert!(close(stem.height, tip.height));
            assert_eq!(stem.baseline, 0.0);
            assert_eq!(stem.sign, tip.sign);
            assert_eq!(
                stem.sign,
                if stem.height < 0.0 {
                    ImpulseSignV1::Negative
                } else {
                    ImpulseSignV1::Nonnegative
                }
            );
            assert!(tip.marker_size.is_finite() && tip.marker_size > 0.0);
        }
    }

    #[test]
    fn mathematical_surface_has_complete_unique_parameter_topology() {
        let points = mathematical_surface();
        assert_eq!(points.len(), 45);
        const U_VALUES: [f32; 9] = [-2.0, -1.5, -1.0, -0.5, 0.0, 0.5, 1.0, 1.5, 2.0];
        const V_VALUES: [f32; 5] = [-1.0, -0.5, 0.0, 0.5, 1.0];
        for (index, point) in points.iter().enumerate() {
            assert!(
                point.x.is_finite()
                    && point.y.is_finite()
                    && point.z.is_finite()
                    && point.u.is_finite()
                    && point.v.is_finite()
            );
            assert_eq!(point.u, U_VALUES[index / V_VALUES.len()]);
            assert_eq!(point.v, V_VALUES[index % V_VALUES.len()]);
            assert!(close(point.x, point.u * point.u - point.v * point.v));
            assert!(close(point.y, 2.0 * point.u * point.v));
            assert!(close(point.z, point.u + 0.25 * point.v));
            assert_eq!(point.parameter_color, point.u);
            assert!(
                !points[..index]
                    .iter()
                    .any(|prior| prior.u == point.u && prior.v == point.v)
            );
        }
    }

    #[test]
    fn point_cloud_is_reproducible_bounded_and_uniquely_keyed() {
        let first = point_cloud();
        let second = point_cloud();
        assert_eq!(first, second);
        assert_eq!(first.len(), 96);
        for (index, point) in first.iter().enumerate() {
            assert_eq!(usize::from(point.key), index);
            assert!((-1.0..=1.0).contains(&point.x));
            assert!((-1.0..=1.0).contains(&point.y));
            assert!((-1.0..=1.0).contains(&point.z));
            assert!(point.weight.is_finite() && point.weight > 0.0);
            assert_eq!(
                point.region,
                match (point.z > 0.0, point.x >= 0.0) {
                    (true, true) => CloudRegionV1::UpperEast,
                    (true, false) => CloudRegionV1::UpperWest,
                    (false, true) => CloudRegionV1::LowerEast,
                    (false, false) => CloudRegionV1::LowerWest,
                }
            );
            assert_eq!(point.region, point.region_color);
            assert_eq!(point.region, point.region_shape);
        }
    }

    #[test]
    fn multiple_series_contains_homogeneous_and_heterogeneous_data() {
        let chart = multiple_series();
        assert_eq!(chart.homogeneous.len(), 164);
        assert_eq!(chart.heterogeneous.volume.len(), 8);
        assert_eq!(chart.heterogeneous.trend.len(), 8);
        assert_eq!(chart.heterogeneous.observations.len(), 8);

        for (index, ((bar, line), point)) in chart
            .heterogeneous
            .volume
            .iter()
            .zip(&chart.heterogeneous.trend)
            .zip(&chart.heterogeneous.observations)
            .enumerate()
        {
            assert_eq!(usize::from(bar.key), index);
            assert_eq!(bar.period, line.period);
            assert_eq!(bar.period, point.period);
            assert_eq!(bar.lane, line.lane);
            assert_eq!(bar.lane, point.lane);
            assert_eq!(bar.role, MultipleSeriesRoleV1::Volume);
            assert_eq!(line.role, MultipleSeriesRoleV1::Trend);
            assert_eq!(point.role, MultipleSeriesRoleV1::Observation);
            assert_eq!(bar.key, line.key);
            assert_eq!(bar.key, point.key);
            assert_eq!(line.sequence, line.key);
            assert!(bar.value.is_finite() && bar.value >= 0.0);
            assert!(line.value.is_finite() && line.value >= 0.0);
            assert!(point.value.is_finite() && point.value >= 0.0);
            assert!(point.marker_size.is_finite() && point.marker_size > 0.0);
        }
    }

    #[test]
    fn closed_enumeration_discriminants_are_stable() {
        macro_rules! assert_discriminants {
            ($($value:expr),+ $(,)?) => {{
                let values = [$($value),+];
                for (index, value) in values.into_iter().enumerate() {
                    assert_eq!(value.to_avro(), vec![(index * 2) as u8]);
                }
            }};
        }

        assert_discriminants!(
            ScatterClusterV1::NorthRidge,
            ScatterClusterV1::CentralBasin,
            ScatterClusterV1::SouthShelf,
        );
        assert_discriminants!(
            TrajectoryV1::Survey,
            TrajectoryV1::Forecast,
            TrajectoryV1::Limit,
        );
        assert_discriminants!(QuarterV1::Q1, QuarterV1::Q2, QuarterV1::Q3);
        assert_discriminants!(SalesRegionV1::Americas, SalesRegionV1::Europe);
        assert_discriminants!(
            ProductFamilyV1::Home,
            ProductFamilyV1::Business,
            ProductFamilyV1::Platform,
        );
        assert_discriminants!(BarScenarioV1::Actual, BarScenarioV1::Plan);
        assert_discriminants!(
            BubbleSegmentV1::Core,
            BubbleSegmentV1::Growth,
            BubbleSegmentV1::Research,
        );
        assert_discriminants!(
            BubbleClassV1::Consumer,
            BubbleClassV1::Industrial,
            BubbleClassV1::Scientific,
        );
        assert_discriminants!(
            ContourLevelV1::One,
            ContourLevelV1::Two,
            ContourLevelV1::Three,
        );
        assert_discriminants!(
            SpectrumTraceV1::Baseline,
            SpectrumTraceV1::Morning,
            SpectrumTraceV1::Afternoon,
            SpectrumTraceV1::Evening,
        );
        assert_discriminants!(ImpulseSignV1::Negative, ImpulseSignV1::Nonnegative);
        assert_discriminants!(
            CloudRegionV1::UpperEast,
            CloudRegionV1::UpperWest,
            CloudRegionV1::LowerEast,
            CloudRegionV1::LowerWest,
        );
        assert_discriminants!(
            MultipleSeriesRoleV1::Volume,
            MultipleSeriesRoleV1::Trend,
            MultipleSeriesRoleV1::Observation,
        );
        assert_discriminants!(MultipleSeriesLaneV1::Main);
    }
}
