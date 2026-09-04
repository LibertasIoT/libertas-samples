# Libertas Chart3D demo

This `no_std` sample is an interactive gallery for the complete Libertas
Chart3D V1 contract. Deploy the application, select its chart-gallery endpoint,
and send one of the typed requests to receive a deterministic, ready-to-render
3D chart. The source is the public application and endpoint schema; there is no
parallel chart-configuration file.

The gallery contains 13 examples:

| Request | Chart capability |
| --- | --- |
| Spatial scatter | `point` positions, typed cluster color/detail, tooltips, and stable keys |
| 3D trajectory | `line` with explicit detail and order fields |
| Bar forms | `hconcat` comparing legacy two-band bars, sparse base-and-value bars, and explicit continuous cuboids |
| Bubble cloud | `point` with a quantitative size channel |
| Terrain height map | Rectangular-grid `surface` data |
| Terrain wireframe | Rectangular-grid `wireframe` topology |
| Contour terrain | `layer` combining a surface with server-sampled contour lines |
| Nonuniform surface | Uneven rectangular-grid `surface` data with one explicit topology hole |
| Spectrum waterfall | Ordered, detailed `line` series |
| Impulse response | `layer` combining `stem` marks with point tips |
| Mathematical surface | A server-sampled polynomial parameter surface with explicit `u` and `v` topology |
| Sensor point cloud | A dense `point` cloud with typed classifications |
| Multi-series comparison | `vconcat` views containing heterogeneous layered series |

Together these examples cover all six primitive marks—`point`, `line`, `bar`,
`surface`, `wireframe`, and `stem`—and all three composition marks—`layer`,
`hconcat`, and `vconcat`. They also exercise perspective and orthographic
views, shared and independent scales, axis and legend guides, rectangular and
parameter surface topology, and the supported bar geometries.

All data needed to render a response is transported by the endpoint. Derived
geometry is calculated by the application: formula surfaces are sampled into
rows, and contour paths are generated into ordered line points before sending
the response. The chart client does not evaluate formulas, bin data, infer
contours, or run another transform language.

Chart-facing categories and other fixed vocabularies use exported Rust enums,
so they retain typed identity and can be localized by the platform instead of
embedding user-visible string literals in data rows. This gallery needs no
chart-facing `String` or `FormattedText` field: guide and tooltip wording comes
from the package, type, field, and enum documentation. A future dynamic
application-authored label would use the Libertas LMF1 formatted-string
representation and stable string resources. Numeric measurements stay numeric
and declare their units in field names and documentation.

The application exposes one typed endpoint server. Each request selects one
gallery example; the correlated response contains exactly one Chart3D value.
The runtime preserves the request transaction ID and peer, returns one response
for each valid request, and rejects values with the wrong protocol role.

## Validation

Run the complete sample validation from this directory:

```sh
cargo fmt --all -- --check
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
git diff --check
```
