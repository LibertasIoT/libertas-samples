# Libertas chart demo

This sample is an interactive gallery for the complete Libertas chart contract.
Deploy the application, select its **Chart gallery** endpoint, and choose any
request to receive a ready-to-render chart with deterministic demonstration
data. The temperature history aligns its final completed five-minute bucket to
the Hub's current UTC time.

The gallery covers every primitive mark and composition:

| Request | Chart capability |
| --- | --- |
| Bubble portfolio | `point`, quantitative position, color, size, shape, opacity, tooltips, keys, and facets |
| Energy history | `line`, UTC time, multiple series, dash and width encodings |
| Forecast range | `area`, ranged `y`/`y2` confidence band |
| Grouped sales | `bar`, categorical bands and `xOffset` grouping |
| Latency histogram | `bar`, explicit numeric `x`/`x2` bins, and server-computed counts |
| Explicit stacking | `hconcat` of server-computed stacked `bar` and `area` charts |
| Activity heatmap | `rect`, ranged cells and localized enum-backed value-sourced axis labels |
| Measurement uncertainty | `rule`, ranged error bars |
| Direct labels | `text` using canonical LMF1 `FormattedText` bytes, plus color, size, opacity, and angle |
| Capability radar | polar `polygon`, angular ordering, fill and stroke |
| Traffic sources | `arc` donut with automatic linked outside category labels, including 2% and 1% slices for collision testing |
| Arc families | `hconcat` of a full-radius pie and quantitative radial bars |
| Projected map | Cartesian `polygon` regions using server-projected coordinates |
| Forecast confidence | `layer` combining confidence-band `area` and estimate `line` marks |
| Candlestick | `layer` combining low-high `rule` wicks and open-close `rect` bodies |
| Five-minute temperature HLC | A reproducible 72-hour random walk rendered as high-low ranges and close ticks in two shared-scale `rule` layers |
| Box plot | `layer` combining `rect` quartiles and medians, `rule` whiskers, and `point` outliers |
| Network topology | arbitrary-endpoint `rule` links layered under positioned `point` nodes with localized enum vocabulary |
| Sankey flow | server-sampled `area` ribbons layered under explicit `rect` nodes with localized enum stages |
| Performance story | `layer`, points, trend line, and target rule |
| Regional comparison | `hconcat`, independent side-by-side views and both horizontal and vertical grouping offsets |
| Operations dashboard | `vconcat`, aligned time-series and interval panels |

Across those examples the sample also demonstrates all scale families
(`linear`, `log`, `symlog`, `sqrt`, `time`, `utc`, `ordinal`, `band`, `point`,
and `identity`), guide sources and positions, component visibility, physical
units, numeric formats, stable keys, details, ordering, and legends below the
chart area. Fixed chart vocabularies retain typed enum identity, while the
direct-text example transports locale-independent LMF1 tuples in
`FormattedText` byte fields and leaves rendering to the client.

The source is the application schema; there is no separate hand-written chart
configuration file.

## Design references

The examples adapt durable gallery patterns from the official
[Vega-Lite example gallery](https://vega.github.io/vega-lite/examples/),
[Highcharts demos](https://www.highcharts.com/demo),
[Plotly JavaScript chart gallery](https://plotly.com/javascript/), and
[Apache ECharts examples](https://echarts.apache.org/examples/en/index.html).
They are implemented entirely with the Libertas chart contract and original
sample data.
