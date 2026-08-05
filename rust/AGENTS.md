# Libertas Rust Application Development

## Scope

These instructions apply to every Rust application below this directory. Build
self-describing applications on the Libertas platform. Use `no_std` unless an
application-specific section explicitly requires `std`;
`libertas-weather_server` uses `std` for its HTTP worker. Libertas is
broader than Matter: an application may use configuration schemas, persistent
data, endpoints, notifications, timers, users, actions, physical devices, or
other protocols without using Matter at all. Add `libertas-matter` only when the
application actually integrates Matter logical devices.

Prefer public, protocol-specific SDK APIs. Never reproduce native bridge
layouts; in Matter code, never reproduce protocol constants or generated
definitions.

## Required workflow

1. Inspect the target crate, its `Cargo.toml`, nearby samples, and `git status`.
   Preserve unrelated user changes.
2. Resolve the exact Libertas APIs and any applicable protocol APIs from the
   checked-out dependencies before coding. Do not assume an API from an older
   sample.
3. Define the public application schema before runtime implementation. If the
   app persists data, define its data union at this stage too.
4. Implement the smallest requested runtime surface. For Matter applications,
   keep the virtual-device descriptor synchronized with implemented features,
   attributes, events, and commands.
5. Test pure state logic, Avro persistence, endpoint transactions, protocol
   encoding/decoding, error paths, and timer boundaries as applicable.
6. Finish with:

   ```sh
   cargo fmt --all -- --check
   cargo check
   cargo test
   cargo clippy --all-targets -- -D warnings
   git diff --check
   ```

## Crate baseline

Use Rust 2024 and align Libertas SDK dependencies to the same source revision.

```toml
[package]
edition = "2024"

[dependencies]
libertas = { git = "https://github.com/LibertasIoT/libertas-rs.git", branch = "main", package = "libertas" }
libertas_macros = { git = "https://github.com/LibertasIoT/libertas-rs.git", branch = "main", package = "libertas_macros" }

[lib]
crate-type = ["rlib"]
```

Matter applications additionally use:

```toml
libertas-matter = { git = "https://github.com/LibertasIoT/libertas-matter", package = "libertas-matter", default-features = false, features = ["alloc"] }
```

Keep the direct `libertas` dependency on the same revision expected by
`libertas-matter`; otherwise Cargo may create duplicate runtime types.
Use the application name for `[lib].name`. Commit `Cargo.lock` for sample
applications. Run `cargo tree -d` after dependency changes and remove duplicate
Libertas revisions.

Start application libraries with:

```rust
//! Application display name.
//! A concise user-facing description.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
```

Use `alloc` collections and `Rc<RefCell<_>>` when shared callback state is
needed. Keep unsafe code inside the SDK. Do not introduce `std` into production
paths except where an application-specific section explicitly requires it.

## Self-describing Libertas source

Source is the schema; do not create parallel JSON schema files.

- `//!` at the entry file defines package documentation. Put
  `//! #[libertas_default_locale(en)]` there when needed.
- The first `///` line is the UI display name; subsequent lines are its
  description.
- `//` comments are implementation-only and ignored by schema generation.
- Public functions in `lib.rs` are application entry points. Their parameters
  are user configuration.
- Derive `LibertasExport` on every public struct or enum that participates in
  application schema. Import attributes and derives from `libertas_macros`;
  do not recreate them locally.
- Use semantic platform types so the generated UI is correct:
  `LibertasDevice`, `LibertasVirtualDevice`, `LibertasEndpoint`,
  `LibertasUser`, `LibertasDateTime`, `LibertasTimeOnly`,
  `LibertasLanDevice`, and `LibertasAction`.
- Use `Option<T>` only for values that are genuinely optional. A required
  runtime or database invariant must use `T`.
- A fieldless enum is an enumeration. An enum with payload variants is a union.
  Prefer inline named variant fields when they are used only by that union.
- For `Vec<T>` documentation, place list documentation above a `----` line and
  element documentation below it.

Common schema attributes:

- `#[libertas_default(...)]`, `#[libertas_number(min=..., max=..., step=...)]`
- `#[libertas_size(min=..., max=...)]`, `#[libertas_time_interval]`
- `#[libertas_formatted_text]` on a runtime `String` or `Vec<u8>`
- `#[libertas_ui_header]`, `#[libertas_read_only]`, `#[libertas_hidden]`
- `#[libertas_unordered]`, `#[libertas_unique]`
- `#[libertas_device_type("...")]`,
  `#[libertas_virtual_device_type("...")]`
- `#[libertas_endpoint_schema(ProtocolUnion)]`,
  `#[libertas_endpoint_server]`,
  `#[libertas_endpoint_base_objects("path")]`

Endpoint protocol schemas must be payload unions derived with
`LibertasAvroEncode`, `LibertasAvroDecode`, and `LibertasExport`. Mark variants
with the appropriate roles: `#[libertas_request]`, `#[libertas_response]`,
`#[libertas_subscription_request]`, or `#[libertas_subscription_data]`. Use
`#[libertas_next_request(...)]`, `#[libertas_next_response(...)]`,
`#[libertas_cacheable]`, and `#[libertas_copy_from("path")]` only when their
transaction semantics are actually implemented.

## Persistent data

Every type written to the Libertas database must be a variant of one
payload-carrying enum defined before the application function:

```rust
#[derive(
    Clone, Debug, PartialEq,
    LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum AppData {
    Settings {
        timeout_seconds: u32,
    },
}

#[libertas_data_schema(AppData)]
pub fn application(/* configuration */) {
    // ...
}
```

The function-level `#[libertas_data_schema(AppData)]` links the union to the
application and marks it as Libertas data in generated schema. All values passed
to `libertas_data_write` must be `AppData` variants. Do not store an unlisted
struct or primitive directly.

Use stable resource identifiers plus typed `NotificationArgument` values as
database keys:

```rust
pub static APP_STRINGS: [(&str, &str); 1] = [
    ("APP_SETTINGS", "Settings for %1$s."),
];

let key = [NotificationArgument::Object(device)];
let value = AppData::Settings { timeout_seconds: 600 };
libertas_data_write("APP_SETTINGS", &key, &value);
let saved: Option<AppData> = libertas_data_read("APP_SETTINGS", &key);
```

Attach `#[libertas_string_resources(APP_STRINGS)]` to the application function.
Resource identifiers are stable database-name IDs; templates are user-facing
and may be translated.

Persistence rules:

- Treat enum variant order and variant-field order as an on-disk ABI: Avro
  encoding is positional. Never reorder or repurpose existing variants/fields.
  Append compatible variants or implement an explicit migration.
- `libertas_data_read` returns `None` for a missing record. Initialize every
  required record immediately with a documented default.
- Validate decoded values before use. Repair or reject values that violate
  current invariants.
- Persist configuration after a successful write and before reporting it
  changed. Do not persist transient timers, subscriptions, transaction IDs, or
  derived attributes unless explicitly required.
- Use standalone data for one current value per key. Use indexed data APIs only
  for ordered histories or multiple records.
- Keep database calls outside active `RefCell` mutable borrows when practical.

## Libertas runtime

Choose the highest-level Libertas API that matches the application:

- typed endpoints for application-defined Avro request/response/subscription
  protocols;
- protocol-specific device APIs for physical or logical devices;
- data APIs for persistence;
- notification APIs for user-facing messages;
- timers and wake-up callbacks for scheduling and external coordination.

Do not route ordinary application behavior through hidden raw device-send APIs.
They are transport primitives for protocol libraries.

For a typed endpoint, define its protocol union and register
`libertas_register_endpoint_listener::<Protocol, _>`. The callback receives:

```text
(endpoint, opcode, decoded_protocol_value, boxed_context, transaction_id, peer)
    -> LibertasEndpointStatus
```

Handle `OP_ENDPOINT_REQ`, `OP_ENDPOINT_SUB_REQ`, `OP_ENDPOINT_RSP`,
`OP_ENDPOINT_DATA`, `OP_ENDPOINT_PEER_DOWN`, and
`OP_ENDPOINT_PEER_TIMEOUT` according to the declared variant roles. Use
`libertas_endpoint_request`,
`libertas_endpoint_subscribe_request`, `libertas_endpoint_response`, and
`libertas_endpoint_report`; preserve transaction ID and peer for correlated
responses. If a subscription is rejected, call
`libertas_endpoint_remove_subscriber`. Clean up peer-specific state on
`OP_ENDPOINT_PEER_DOWN`. A peer timeout reports uncertain network reachability,
not confirmed peer termination; preserve recoverable peer state unless the
application protocol deliberately expires it.

The typed endpoint runtime validates the platform status byte, complete Avro
decoding, and absence of trailing bytes before delivering requests. It
automatically answers malformed requests and subscription requests with
`LibertasEndpointStatus::InvalidMessage`. Return
`LibertasEndpointStatus::InvalidMessage` for a decoded request whose message
role or semantics are invalid; return `Success` after handling a valid message.
Do not send a second application response for the platform error.

Endpoint payloads are arbitrary developer-defined Avro unions. Do not force a
Matter read/write/invoke model onto a Libertas endpoint unless that is the
application's deliberate protocol.

For protocol devices, collect and deduplicate configured devices before
registering listeners: only one listener may be registered per device. A
`libertas_register_device_listener` callback receives:

```text
(device, opcode, bytes, boxed_context, transaction_id, peer)
```

Downcast context once and dispatch with the protocol-specific library. Preserve
the original device, transaction ID, and peer in correlated responses. Send
exactly one response for each request and do not silently drop decoding or
encoding failures.

Send localized user notifications with `libertas_notification_send`, a stable
resource ID from `APP_STRINGS`, typed `NotificationArgument` values, and the
least severe appropriate `NotificationImportance`. Use literal notifications
only for non-localizable diagnostic text.

Time is in microseconds:

- `libertas_get_sys_ticks()` is monotonic and is used for interval timers.
- `libertas_get_utc_time()` is optional wall-clock time and is used for
  persisted calendar deadlines.
- `libertas_timer_new_interval(expiration, ...)` and
  `libertas_timer_update_interval(...)` take an absolute monotonic expiration,
  not a relative duration.
- Cancel timers whenever state makes them obsolete. Keep timer callbacks
  idempotent and re-check state.
- Use saturating arithmetic for durations and round externally visible
  remaining time according to the device specification.

Callbacks can trigger further host activity. Avoid holding mutable borrows while
sending responses, reporting changes, calling database APIs, or invoking code
that may re-enter application state.

Applications that own worker threads must register
`libertas_register_shutdown_handler`. Signal shutdown without blocking the
Libertas application thread, stop every worker from making further Libertas
calls, and call `libertas_shutdown_complete` only after cleanup finishes. The
thread calling `libertas_shutdown_complete` must make no later Libertas API call
and must return immediately.

## Matter development (Matter applications only)

Libertas logical devices are already host-routed Matter endpoints. Matter frame
paths contain cluster and attribute/command/event IDs, but no endpoint ID.

Always use generated names through the runtime:

```rust
use libertas_matter::{
    MatterDevice, MatterRequestContext,
    consts::{attributes, clusters, commands, events},
    definitions::OnOff::{
        attributes::OnOff,
        commands::Toggle,
    },
};
```

- Use `libertas_matter::consts` for IDs and catalogs.
- Use `libertas_matter::definitions` for typed attributes, commands, responses,
  events, enums, bitmaps, and structs.
- Do not hardcode Matter IDs or create local copies of generated types.
- Do not edit `libertas-matter-consts` generated files.
- Use `Nullable<T>` for Matter nullable fields; do not confuse it with an
  optional command field (`Option<T>`).
- Reject command fields gated by unsupported features. Never emulate an
  unsupported feature implicitly.

For client operations, prefer typed APIs:

- `MatterDevice::read_attribute::<A>()`
- `MatterDevice::write_attribute(&A(...))`
- `MatterDevice::invoke(&Command { ... })`
- `MatterReadCluster`, `MatterWriteBatch`, and
  `MatterSubscriptionCluster` for bounded batches
- `MatterDeviceSubscription` and `MatterSubscriptionBatch` for one app-wide
  subscription send

Const-generic capacities are exact limits. Builders return `Error::NoSpace`
when exceeded. Their request descriptors borrow caller-owned arrays; keep
builders and backing buffers alive through the send. Use `InlineByteBuffer` or
a caller-owned `SliceWriter`; do not allocate merely to encode TLV.

For virtual Matter servers:

- In the device listener, construct `MatterRequestContext` from the original
  device, transaction ID, and peer.
- Dispatch `ReadRequest`/`SubscribeRequest`, `WriteRequest`, and `InvokeRequest`
  separately.
- Use `Element`/`FromTLV` for decoding and typed definitions for values. Use
  `decode_command::<C>` after checking the command path.
- A request may contain multiple attribute paths or writes. Return one entry for
  every path, preserving order, with either typed data or a path-specific
  status.
- Use `MatterRequestContext` response helpers for single typed responses. For
  multi-path responses, use `frame` and `tlv` helpers with typed values rather
  than hand-encoding numeric values.
- Report successful state changes with `attribute_changed::<A>()` or one
  `changed_batch`; use `LIBERTAS_BROADCAST_DEST` only when every subscriber
  should receive the change.

Use precise Interaction Model statuses:

- unknown cluster: `UnsupportedCluster`
- unknown attribute/command/event: the matching `Unsupported*`
- write to read-only attribute: `UnsupportedWrite`
- malformed or wrong TLV type: `InvalidDataType`
- valid type but invalid value: `ConstraintError`
- unsupported feature-specific command fields: `InvalidCommand`
- buffer/capacity exhaustion: `ResourceExhausted`
- unrecognized operation: `InvalidAction`

Map internal errors deliberately and respond once. Do not turn all failures into
generic `Failure`.

## Matter virtual-device descriptor (Matter applications only)

Generate `#[libertas_virtual_device_type("...")]` with the Libertas **Virtual
Device Type Editor**. Do not invent or casually copy Base64 descriptors.

The descriptor is a contract and must match implementation:

- correct Matter device type and revision;
- server/client cluster direction;
- cluster revision and feature map;
- exactly the readable/writable attributes and accepted/generated commands;
- no level, scene, time, or other optional feature unless implemented;
- no exposed path that always returns an unsupported status.

Whenever the descriptor changes, update handler dispatch, typed reports, tests,
and documentation in the same change.

## Completion checklist

- Public schema compiles and accurately describes configuration.
- If persistence is used, required records initialize deterministically and
  Avro round-trip tests cover each persisted union variant.
- Startup state is intentional and independent of stale transient data.
- If endpoints are used, variant roles, transaction flow, responses, reports,
  and subscriber cleanup agree with the declared protocol.
- For Matter apps, descriptor, feature map, attributes, events, commands, and
  handlers agree; multi-path and unsupported paths return correct statuses.
- If timers are used, tests cover zero, expiration, overflow, cancellation, and
  restart behavior.
- Reports or notifications happen only after successful state changes.
- No hidden transport API misuse, unsafe code, or accidental `std` dependency.
  Matter apps additionally have no hardcoded Matter IDs or generated-file
  edits.
- Formatting, check, tests, Clippy, and diff validation all pass.

## Application-tailored weather definitions (`libertas-weather`)

`libertas-weather` is a reusable `no_std` schema library for
application-tailored weather data. It is not a general-purpose weather model.
Keep it free of application entry points, runtime listeners, persistence calls,
and protocol-specific dependencies unless the crate's scope is deliberately
expanded.

- Derive `LibertasAvroEncode`, `LibertasAvroDecode`, and `LibertasExport` on
  every public weather data type.
- Name each schema family after its consuming application or use case, and
  include only the weather inputs that application needs. Do not introduce
  universal location, condition, observation, or forecast types merely to share
  fields between unrelated applications.
- Give independently evolvable public data shapes an explicit version suffix
  such as `V1`. Once published, treat a versioned type's name, field and variant
  order, field types, units, optionality, and meaning as immutable.
- Before the initial `libertas-weather` schema is published, keep every current
  schema and message at `V1` and freely reshape that V1 design. Do not retain
  superseded design-only variants for compatibility. The immutability and
  append-only rules begin when a schema is published.
- Introduce a new versioned type for an incompatible change. Keep older versions
  available; do not rename a versioned type to `Latest` or silently redirect an
  unversioned alias.
- Give each application-specific schema family its own append-only Avro union.
  Add new payload versions at the end, and never reorder, remove, or repurpose
  existing variants.
- Put measurement units in field names and documentation. Use
  `LibertasDateTime` for weather timestamps.
- For sprinkler weather, refresh current conditions every 15 minutes and recent
  history and forecasts every hour. Keep seven days of hourly history and seven
  days of hourly forecast data.
- Make sprinkler history, current conditions, and forecast independently
  optional in runtime snapshots and independently writable persistent-data
  variants. A failed or partial provider response must not erase the last valid
  section.
- Store retrieval and validity timestamps with every cached section. Consumers
  may use stale history or forecasts as degraded inputs, but stale current
  conditions must never be treated as proof that it is safe to water.
- Treat the sprinkler endpoint schema as an arbitrary message contract.
  `GetWeatherV1` is both a one-shot request and a subscription request because
  the endpoint operation is carried outside the protocol value.
  `WeatherRecoveryV1` is its correlated response, and `WeatherIncrementV1`
  carries later subscription data.
- Send exactly one `WeatherRecoveryV1` response for every
  `GetWeatherV1` request or subscription request. The response must include a
  nonzero maximum wait interval. After a successful replay or reset, clients
  restart that timeout after the response and every data report, and retry
  `GetWeatherV1` with their last fully applied cursor if it expires. One-shot
  clients ignore the maximum wait interval. On an error response, the typed
  error and retry delay take precedence.
- Incremental sprinkler-weather subscriptions use an epoch timestamp and
  sequence. The epoch timestamp is the stream generation identifier expressed
  as an ordered `LibertasDateTime`; do not add a separate opaque epoch. Keep it
  unchanged while incrementing the sequence once per atomic change, retain an
  in-memory replay journal for 24 hours, and publish reports with exact
  exclusive-from and inclusive-through cursors. Apply a report atomically only
  when its from-cursor matches the subscriber's stored cursor.
- When no state change occurs before a subscriber's maximum wait interval,
  publish an empty contiguous incremental report as a heartbeat. It keeps the
  same cursor, proves liveness, and restarts the client timeout.
- On a cursor gap or expired cursor, use the resume transaction. Replay a
  contiguous retained range when possible; otherwise return a cached snapshot
  limited to the client's half-open history and forecast recovery ranges.
- Do not persist subscription cursors or replay journals. After a server data
  reset, preserve independently persisted history, current, and forecast
  records, reset only the cursor sequence to zero, and assign the reset cursor
  an epoch timestamp strictly newer than the previous epoch timestamp. Rebuild
  the recovery snapshot from those records and retrieve missing historical
  data from Open-Meteo when available.
- A client recognizes a server reset from the combination of a backward
  sequence and an epoch timestamp newer than its last fully applied cursor. The
  observed sequence need not be zero because changes may occur between the
  server's reset and the client's next response. Accept the accompanying reset
  snapshot atomically. A smaller sequence with the same or an older epoch
  timestamp is stale or out of order; reject it without rolling back local
  weather data and retry the resume transaction.
- Provider refresh failures do not emit clear changes; clear a section only
  after its cached value is proven invalid.
- Add Avro round-trip coverage for every new public data shape and stable
  encoding checks for enum and union discriminants.

## Sprinkler weather endpoint server (`libertas-weather_server`)

`libertas-weather_server` is a `std` Libertas application library that
serves `SprinklerWeatherProtocolV1`. Its application configuration exposes
exactly one endpoint marked with `#[libertas_endpoint_server]`; do not model the
endpoint operation as another protocol field.

- Treat the server's V1 schema and database layout as unpublished design-time
  contracts. Reshape V1 directly when needed; do not keep superseded
  configuration fields, union ordering, or migration code for compatibility.
- Obtain the sprinkler site from `HubProtocol::LocationRsp` on the built-in
  `LIBERTAS_HUB_ENDPOINT`; do not expose latitude or longitude as application
  configuration. Keep `libertas-hub`, `libertas`, and `libertas_macros` on the
  same source revision.
- Subscribe to Hub location at every startup with a nonzero maximum report
  interval. Retry after 60 seconds until the first valid response and whenever
  the Hub fails to report within its requested one-hour maximum interval.
  Handle typed endpoint statuses explicitly with
  `libertas_register_endpoint_status_listener`.
- Persist the last valid Hub location independently under a stable database
  resource. A valid cached location may start provider refreshes while the Hub
  subscription recovers. If no valid cached location exists, leave the HTTP
  worker idle and do not expose weather records that cannot be associated with
  a known site.
- Persist a changed Hub location before clearing history, current conditions,
  and forecast for the old site. Publish one `SectionClearV1` change for each
  section that existed, then request replacement current and hourly data
  immediately. Tag provider commands and results with their location and
  discard any in-flight result for an older site.
- Use the current typed endpoint callback contract. Transaction IDs are always
  present. Send exactly one `WeatherRecoveryV1` response for each
  `OP_ENDPOINT_REQ` or `OP_ENDPOINT_SUB_REQ`, preserving the callback's
  endpoint, transaction ID, and peer. Return
  `LibertasEndpointStatus::InvalidMessage` instead for a decoded protocol value
  that is not `GetWeatherV1`; malformed Avro is rejected by the runtime before
  the callback.
- Add a peer to application subscription state only after a successful
  recovery response. For a rejected subscription, send the typed error response
  and call `libertas_endpoint_remove_subscriber`. Remove peer-specific state on
  `OP_ENDPOINT_PEER_DOWN`. Preserve it on `OP_ENDPOINT_PEER_TIMEOUT`, where peer
  liveness is uncertain.
- Restore history, current conditions, and forecast from their independent
  stable database resources. Validate decoded timestamps, ordering, durations,
  ranges, probabilities, and finite nonnegative measurements before exposing a
  record. A missing or invalid section remains `None` without hiding valid
  sections.
- Never persist the cursor, replay journal, subscriber list, transaction IDs, or
  heartbeat deadlines. Startup creates a current-time epoch timestamp with
  sequence zero while preserving the validated weather snapshot.
- Limit history and forecast recovery requests to their seven-day V1 windows
  and 168 periods per section. Replay only an exact contiguous journal range;
  otherwise return a range-limited reset snapshot.
- Arm heartbeat timers with absolute monotonic expiration ticks. Send each due
  subscriber an empty cursor-preserving `WeatherIncrementV1` before its maximum
  wait interval, then rearm from the actual report time. Do not hold a mutable
  application-state borrow while sending a response or report or updating a
  timer.
- Perform Open-Meteo HTTPS requests only on the dedicated `std` worker thread.
  Reuse one Rustls HTTP client with bounded connect and total timeouts, limited
  redirects, compressed-response support, HTTP status validation, and a capped
  response body. Request Unix timestamps, metric precipitation, and meters per
  second wind units explicitly.
- Communicate through bounded channels. Timer callbacks use `try_send` to enqueue
  refresh commands; the worker uses `try_send` for owned results and calls only
  `libertas_wake_up`. The wake-up callback drains results with `try_recv`.
  Database, timer, logging, endpoint, cursor, journal, and subscriber APIs stay
  on the Libertas application thread.
- Refresh current conditions every 15 minutes and combined history/forecast
  data every hour. Validate a complete provider section before acceptance.
  Persist an accepted section before changing runtime state or publishing its
  incremental report. A provider, internet, HTTP, JSON, or validation failure
  leaves the existing persistent and runtime section unchanged.
- Preserve refresh timing across application restarts. For each valid cached
  section, derive its next due time from `retrieved_at + refresh_interval`.
  Refresh missing, overdue, future-dated, or unschedulable sections immediately;
  otherwise wait only for the remaining interval. Because history and forecast
  share one provider request, schedule it for the earlier section deadline.
  Continue subsequent refreshes from monotonic timer firings.
- Register an application shutdown handler after starting the provider worker.
  Set an atomic stop request and use the bounded command channel to wake the
  worker without blocking the Libertas thread. After any bounded in-flight HTTP
  operation, the worker must discard the result, stop, call
  `libertas_shutdown_complete` as its final Libertas API operation, and return.
- The remaining Libertas API limitation is the lack of a persistence completion
  result when confirmed durability before reporting is required.

## Building HVAC weather definitions (`libertas-weather`)

The `BuildingHvacWeatherV1` family is tailored to whole-house and
whole-building HVAC supervisory control. It supplies outdoor inputs for thermal
load prediction, heat-pump operation, economizer decisions, infiltration
estimation, preheating or precooling, and controlled outdoor-air ventilation.
It is not a general weather display schema and does not replace local sensors
used for freeze, equipment, smoke, or life-safety protection.

- Carry dry-bulb and dew-point temperature, relative humidity, surface pressure,
  wind speed, wind gust, wind direction, precipitation amount and phase, and
  global-horizontal, direct-normal, and diffuse-horizontal solar irradiance.
  Carry solar elevation and solar azimuth calculated from the site coordinates
  and the represented UTC time. HVAC machine-learning features use solar
  elevation directly and the sine and cosine of solar azimuth; do not use raw
  azimuth as an ordinal feature. Derive humidity ratio, moist-air enthalpy, and
  wet-bulb temperature instead of storing redundant values that can disagree.
- Keep modeled outdoor air quality independently optional and independently
  cached from physical weather. V1 includes PM2.5, PM10, ozone, and nitrogen
  dioxide concentrations. Stale or missing model data is never proof that
  outdoor air is safe.
- Refresh current physical weather every 15 minutes and treat it as stale after
  30 minutes. Refresh physical history and forecast hourly; retain 72 hours of
  each. Forecast periods may use 15-minute resolution for the first six hours
  and hourly resolution later. Refresh air quality hourly, treat it as stale
  after two hours, and retain a 48-hour horizon.
- Keep history, current conditions, forecast, and outdoor air quality
  independently optional in runtime snapshots and independently writable in
  persistent data. A provider failure must leave every last valid section
  unchanged.
- Use the same epoch-timestamp-and-sequence reset detection, exact contiguous
  incremental ranges, 24-hour transient replay window, range-limited snapshot
  recovery, and empty heartbeat behavior as the sprinkler family. Neither
  family shares its public types or Avro union because they evolve for different
  consuming applications.
- Keep indoor temperature, indoor humidity, indoor air quality, occupancy,
  equipment state, energy prices, recent energy use, and recently delivered
  heating or cooling out of this weather family. They are HVAC application
  state and persistence inputs.

## Weather-aware sprinkler controller (`libertas-sprinkler`)

`libertas-sprinkler` is a `no_std` Libertas application library that calculates,
exposes, and executes application-tailored irrigation schedules. Treat its V1
schema and database layout as unpublished design-time contracts. Reshape V1
directly when the design changes; do not retain old configuration fields,
protocol variants, sidecar schema files, or migration code for compatibility.

- Configuration contains one shared `SprinklerWeatherProtocolV1` client
  endpoint and one or more `SprinklerZoneV1` values. A zone contains exactly one
  Matter Irrigation System valve, a curated soil type, a curated planting type,
  a measured application rate in millimeters per hour, and one schedule server
  endpoint. Do not restore raw `field_capacity`, sprinkler-head categories,
  notification lists, or a configuration-time watering adjustment.
- Use the Device Type Editor descriptor for a Matter Irrigation System device
  with the Valve Configuration and Control server cluster. Control the valve
  with the generated typed `Open` and `Close` commands, and observe generated
  `CurrentState`, `ValveFault`, and `ValveStateChanged` definitions. Never infer
  delivered water from a requested command duration.
- Subscribe to every distinct valve once with one application-wide bounded
  Matter subscription batch. Treat every observed open interval as irrigation,
  including manual valve operation. Convert actual open time using the zone's
  measured application rate, checkpoint an open interval every minute, merge
  adjacent checkpoints, and persist it before recalculating or reporting the
  schedule. The maximum Matter report interval is 30 seconds; if no valve report
  arrives for 90 seconds, mark its state unavailable and resend the complete
  app subscription outside any active state borrow.
- Expose `SprinklerZoneProtocolV1` on every zone schedule endpoint.
  `GetScheduleV1` is both the one-shot and subscription request because the
  endpoint operation is outside the protocol value. Every accepted request
  returns `ScheduleV1`; a subscription additionally receives `ScheduleV1`
  reports after later calculation, preference, hold-off, weather, valve, or
  water-ledger changes.
- The only runtime watering adjustment is
  `more_or_less_water_normalized`, a finite number from -1.0 through 1.0. Map it
  linearly to 50% through 150% of the calculated replenishment. Persist the
  accepted value before reporting its changed schedule.
- Hold-off periods are runtime schedule constraints, not application
  configuration. Validate nonzero representable half-open intervals, limit the
  replacement list to 64 values, sort it, merge overlapping or touching
  intervals, and shift the complete calculated valve-open slot until it no
  longer overlaps a hold-off. Persist the normalized replacement before
  reporting the new schedule. Omit expired constraints from the exposed
  schedule without reinterpreting their persisted interval.
- `SprinklerZoneScheduleV1` is the user-visible calculated result. Keep its
  calculation time, decision or constraint, optional next watering slot,
  planned water, estimated deficit, seven-day precipitation and observed
  irrigation totals, normalized adjustment, normalized hold-offs, observed
  valve-state availability, valve-open state, and valve fault bitmap
  synchronized. Do not start automatic watering until the first non-null Matter
  current-state report establishes the valve's state.
- Persist every zone independently as `SprinklerDataV1::ZoneMemoryV1`, keyed by
  its valve object. The memory contains the normalized preference, hold-offs,
  a folded water-balance baseline, and a bounded seven-day ledger of historical
  precipitation, historical reference evapotranspiration, and observed
  irrigation. Initialize missing or invalid memory deterministically and
  preserve its effect across restarts.
- Subscribe to sprinkler weather at startup and request seven days of history,
  fresh current conditions, and seven days of forecast. Apply only contiguous
  incremental reports. Resume from the last fully applied runtime cursor after
  timeout or a gap; accept a backward sequence only with a newer epoch timestamp
  as a server reset. If the server returns `CursorAhead`, discard the transient
  client cursor and retry an initial range-limited recovery. Do not persist the
  weather cursor. Track successful stream continuity separately from section
  timestamps: a gap, rejected report, endpoint status, or peer timeout makes
  cached current conditions unsafe for actuation until recovery succeeds,
  without erasing cached history or forecast. If startup has no valid UTC time,
  omit invalid recovery ranges; once UTC becomes available, retry the full
  request every minute while required history is still absent.
- Historical weather drives the water balance, forecast rain and unsafe periods
  shift the plan, and current rain, near-freezing temperature, excessive wind,
  or stale current weather inhibits immediate watering. Stale history or
  forecast may remain degraded planning input, but automatic valve actuation
  requires both available history and fresh safe current conditions.
- Permit at most one automatic zone to be open or command-pending at a time.
  Close a controller-opened valve when current weather becomes unsafe, but do
  not take ownership of or forcibly close a manual valve opening. A manual open
  still blocks another automatic zone and contributes to observed irrigation.
  Keep command-pending state transient, expire it after a bounded interval, and
  wait for observed valve state before accounting irrigation. Re-evaluate
  schedules every minute so hold-off boundaries and command timeouts do not
  inherit the slower weather refresh cadence.
- Test public protocol and persistence Avro round trips and discriminants,
  normalized adjustment behavior, hold-off normalization and schedule shifting,
  weather cursor reset rules, typed Matter command encoding, weather balance,
  restart baselines, and manual/open-valve accounting.
- `libertas_macros` revision `f58b1fd7` adds `libertas_size` to the accepted
  `LibertasExport` helper attributes and to function-argument attribute
  consumption. Keep this SDK support when bounded list schema attributes are
  used.

## Smart building HVAC controller (`libertas-smart_building_hvac`)

`libertas-smart_building_hvac` is a `std` Matter application library for
room-aware supervisory control of a whole house or building on the Libertas Hub
Linux runtime. Treat its V1
configuration, runtime protocol, and database layout as unpublished design-time
contracts. Reshape V1 directly while the design is under review; do not add
legacy fields or migrations.

- Package the CPU-only XGBoost implementation into the application artifact as
  a position-independent static library. Build with CUDA and OpenMP disabled so
  the deployed application has no XGBoost, CUDA, or OpenMP runtime-package
  dependency. Keep XGBoost's unsafe C API behind a safe dependency wrapper;
  application source remains free of unsafe code.
- Own every XGBoost booster on one bounded `std` worker thread. Libertas
  callbacks may only submit owned work through bounded nonblocking channels;
  the worker may call only `libertas_wake_up` until shutdown, then
  `libertas_shutdown_complete` as its final Libertas operation. Never train and
  predict concurrently against one booster.
- Train thermal models from ordered, validated, indexed room samples using a
  time-ordered holdout. Promote a candidate only when it improves on the
  deterministic no-change baseline by the V1 promotion margin. Persist the
  candidate UBJSON model, feature manifest, checksum, training range, and
  validation metrics before activating it. Retain the previous accepted model
  for rollback and use deterministic control whenever no validated model is
  available.
- XGBoost predicts bounded near-term room temperature movement only. The
  deterministic control engine remains authoritative for thermostat limits,
  deadband, user `Off`, command suppression, urgent conditions, and all
  supervisory or life-safety boundaries.

- Keep rooms and thermostats inside one `BuildingHvacBuildingV1` configuration
  argument. Define `rooms` first, then use
  `#[libertas_enum_source("$.rooms")]` on each thermostat room reference so the
  UI presents the room-name header while storing its zero-based list index.
- Give every room one stable Libertas server endpoint for its runtime contract.
  A configured room must be assigned to exactly one Matter thermostat and have
  at least one `BuildingHvacIndoorSensorV1` station. Each indoor station contains
  a required standard Matter Temperature Sensor and may add standard Matter
  Humidity and Air Quality Sensor logical devices from the same physical
  station. An optional local outdoor station follows the same required
  temperature and optional humidity/air-quality shape. Do not assign one
  thermostat, logical sensor device, or room endpoint more than once.
- Discover optional standard concentration-measurement clusters on every indoor
  or outdoor Matter Air Quality Sensor at runtime rather than adding
  configuration flags for PM2.5, carbon dioxide, or individual pollutants.
  Expose only accepted finite nonnegative measurements whose Matter medium is
  air, preserve each sensor-reported unit, bound each list to the ten V1
  measurement kinds, and keep missing or stale measurements explicitly unknown.
  Supervisory air-quality data never replaces certified smoke,
  carbon-monoxide, or other life-safety protection.
- A thermostat may serve multiple rooms. Room setpoints are supervisory comfort
  demands that must later be reconciled into the shared physical thermostat;
  never represent shared equipment as independently actuated room equipment.
- Define every externally visible room runtime transaction as a variant of
  `BuildingHvacRoomProtocolV1`. `RoomDataV1` is both the complete response and
  subscription report and carries writable room intent beside read-only state,
  per-station indoor data, local outdoor station data, statistics, learned
  cross-zone influences, and calculated plan. Include each configured indoor
  station's optional capabilities in its room state and the same building-level
  outdoor station state on each room endpoint so `GetRoomV1` can query
  discovered measurements. Replace writable intent atomically with an expected
  control revision; sensor-only changes do not increment that revision.
- Put `#[libertas_formatted_text]` on the bounded byte-array summaries in room
  data, calculated plans, and rejected writes. Encode each byte array with
  `libertas_formatted_text` as a string-resource identifier followed by the same
  typed printf-style argument array used by Libertas notifications. Do not
  Base64-wrap the bytes, persist these derived summaries, or use them as
  authoritative control data; structured protocol fields remain authoritative.
- Configure one or more unique `LibertasUser` recipients for urgent HVAC
  notifications. These are supervisory temperature, equipment-recovery, and
  data-availability warnings; the application must never label or present them
  as certified fire, smoke, carbon-monoxide, medical, or other life-safety
  alarms. Air Quality Sensor concentration data remains informational even when
  it includes carbon monoxide.
- Confirm freeze risk at or below 5 degrees Celsius and excessive heat at or
  above 35 degrees Celsius for five continuous minutes. Require ten continuous
  minutes at or above 7 degrees Celsius or at or below 32 degrees Celsius,
  respectively, before reporting recovery. Confirm missing trustworthy room
  temperature or thermostat state for ten minutes. Report ineffective heating
  or cooling only for a room still at or below 15 degrees Celsius or at or above
  30 degrees Celsius, respectively, after a one-hour observation with at least
  80 percent fresh temperature and equipment-runtime coverage and less than 0.5
  degrees Celsius movement toward recovery.
- Persist each urgent condition's activation, recovery, last-evidence, and
  last-notification state before notification submission. Send one localized
  notification on activation, no more than one reminder every 30 minutes while
  unchanged, an immediate notification on severity escalation, and one
  informational recovery notification after confirmed hysteresis. A restart
  must resume these timers without replaying an avoidable notification burst.
  Never infer recovery from missing or stale sensor data.
- Expose active urgent conditions as structured read-only room runtime data and
  report their changes to room subscribers. Localized notification and
  `FormattedText` output is derived presentation only. The current Libertas
  notification API can submit typed localized messages but cannot confirm
  delivery, retract or replace a prior notification, or represent user
  acknowledgement; do not claim those guarantees.
- Run `BuildingHvacAnalyticsEngine` before notification or control evaluation.
  Discard future-dated, expired, malformed, or nonfinite Matter readings from
  current state without erasing their independent last-valid persistent
  records. Fuse fresh temperature around the median while excluding readings
  more than 2 degrees Celsius from it; fuse humidity the same way with a
  15-percentage-point distance. Treat an ambiguous two-sensor disagreement as
  unavailable instead of choosing a sensor arbitrarily. Air-quality sections
  remain station-level data and do not affect temperature-control readiness.
- Derive outdoor humidity ratio, moist-air enthalpy, and wet-bulb temperature
  only from fresh current weather. Use one consistent dew-point and surface-
  pressure input set, reject dew point above dry bulb, and use provider relative
  humidity only as a consistency check with a 15-percentage-point tolerance.
  Expose the derived values in room runtime data but do not persist them; the
  independently persisted current-weather section remains authoritative.
- Build room statistics only from ordered, non-overlapping, nonzero condition
  periods. Use duration-weighted means and degree-minutes, preserve explicit
  missing coverage, and reject the entire calculation on invalid or overlapping
  input rather than partially mixing it with accepted periods.
- Use `BuildingHvacControlEngine` to produce at most one setpoint decision per
  physical thermostat before making typed Matter writes. Skip unavailable,
  malformed, wrong-thermostat, or nonfinite room candidates; never override an
  explicit room `Off`. Shift heating upward and cooling downward by at most 1
  degree Celsius for the normalized comfort preference, with the inverse shift
  for savings, and apply learned cross-zone temperature change before
  calculating demand.
- Select the highest enabled heating target and lowest enabled cooling target
  across rooms, clamp both to reported thermostat bounds, and enforce the
  reported deadband. When these targets conflict, preserve the side with the
  larger temperature deviation after weighting degraded data at one half.
  Avoid a Matter write when calculated targets are already applied within 0.05
  degrees Celsius. Calculation types are implementation APIs, not additional
  endpoint runtime schema; user-visible state remains in
  `BuildingHvacRoomProtocolV1`.
- Define every database value as a variant of
  `BuildingHvacPersistentDataV1`. Persist room control, statistics with bounded
  recent condition periods, cross-zone learning state, each room's last accepted
  indoor station sections, each local outdoor temperature, humidity, and
  air-quality section, and each cached weather section independently. A partial
  sensor or provider failure must not erase a different valid section. Key room
  records by the stable room endpoint. Do not persist calculated plans, endpoint
  transactions, subscribers, weather cursors, or transient Matter command
  state.
- Model central-HVAC spillover as a directional matrix from each source
  thermostat-zone to every room served by another thermostat. Learn heating
  temperature rise and cooling temperature drop separately. Accept a learning
  period only when the affected room's thermostat is idle, exactly one other
  source thermostat is active, room and weather coverage are trustworthy, and a
  passive temperature-change estimate can be removed from the observation.
  Learn that passive outdoor coupling separately from periods in which every
  thermostat-zone is inactive.
- Persist decayed online-regression sufficient statistics for every learned
  matrix edge, not only the current coefficient. Halve completed old observation
  weight every 30 days, continue updating after restarts, and persist an
  accepted update before exposing it. Use low-confidence estimates only for
  prediction; never weaken thermostat, equipment, freeze, smoke, or life-safety
  constraints.
- Keep `BuildingHvacWeatherClientV1` as a client endpoint using
  `BuildingHvacWeatherProtocolV1`. The HVAC controller must not perform provider
  HTTP requests. When runtime integration is added, subscribe at startup, use
  exact cursor continuity and reset rules, and treat stale or missing outdoor
  air quality as unknown rather than safe.
- Keep utility price schedules, occupancy, indoor air quality, equipment
  topology, and optimization policy out of this initial room-topology contract
  until their application-specific runtime requirements are designed.
- `libertas_macros` revision `b10d43ab` registers both `libertas_size` and
  `libertas_enum_source` as accepted `LibertasExport` helper attributes. Keep
  all direct and transitive Libertas dependencies aligned to that revision or a
  later revision containing the same support.
- The Rust schema parser must resolve `EnumSource` from each actual application
  argument tree. Do not revalidate a nested reusable type as an independent
  root after its `EnumSource` has been validated in the configured building
  tree, because doing so discards the surrounding `rooms` list.
