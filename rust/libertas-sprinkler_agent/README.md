# libertas-sprinkler_agent

A `no_std` Libertas application library that calculates and executes
weather-aware irrigation schedules.

Configuration contains:

- one `SprinklerWeatherProtocol` client endpoint shared by all zones;
- one system-wide `Sprinkler Report` server endpoint;
- one to 16 unique Libertas users who receive application reminders, currently
  including system-wide winterization reminders;
- one unique Matter Irrigation System valve per zone; report charts reuse the
  device's normal frontend-resolved name rather than asking for another label;
- a plant type and sprinkler-head type per zone; and
- one server endpoint that exposes an essential regular-user state by default
  and complete advanced state on demand.

The plant profile drives adaptive weather demand and water capacity. The
sprinkler-head profile estimates delivery from observed Matter Valve open time,
without asking the user for soil or application-rate measurements.

With a fresh forecast and known Hub location, the scheduler examines 15-minute
starts between 40% and 65% root-zone deficit. It derives solar elevation from
UTC, latitude, and longitude and scores safe rising-sun candidates by closeness
to the 50% target, reference evapotranspiration, temperature, rain probability,
wind and gusts, and sprinkler-head drift sensitivity. Overhead heads also use
plant-specific foliage-wetness sensitivity and forecast relative humidity:
humid zones move closer to or just after sunrise so leaves can dry, while drip
and bubbler zones have a wider predawn-to-early-morning window. The unconstrained
preferred slot then passes through the hard hold-off decision described below.

A zone at 65% deficit does not wait for another preferred morning; it waters at
the first rain-, freeze-, wind-, valve-, and hold-off-safe opportunity. Missing
location or fresh forecast data preserves the deficit-based offline schedule.
Hold-offs remain absolute no-watering intervals. When delaying an otherwise due
run until after a hold-off would reach 65% deficit, the controller compares safe
15-minute candidates before it. It preempts only after the zone reaches 40%, a
fresh continuous forecast covers the delayed slot, and expected rain cannot
replace at least half the planned water. Otherwise it uses the first legal slot
after the hold-off and recalculates water amount and duration for that actual
start, including any additional delay caused by a longer run overlapping the
next hold-off.

At runtime, the only water-amount input is a 20% through 200% adjuster in 10%
steps; 100% selects the adaptive amount. Users can also replace the zone's
hold-off intervals. The default state and subscription show only the current
watering condition and next watering slot for an active zone; Winterization
contains no fabricated watering slot. An explicit advanced-state request shows
the demand source, calculation time, planned amount, estimated deficit, recent
rain and irrigation, and valve status. A separate subscribable configuration
presents the watering percentage, no-watering periods, and system-wide Active
state together in one end-user view. Each value has its own independent update
action. An accepted update publishes the new configuration before returning a
terminal success acknowledgement; a typed failure leaves the subscribed
configuration installed for correction and retry. Expired no-watering periods
are removed on the next schedule evaluation, persisted, and reported through
the same configuration subscription. The Active state maps to the durable
Active/Winterization interlock and is persisted locally across restarts and
internet outages.

While Active is on, Libertas Notification reminds the configured
users to winterize. Fresh current conditions or a fresh seven-day forecast at
3 °C or below trigger the weather reminder at any latitude. When weather is
unavailable, the cached Hub location provides a seasonal fallback only at 35°
absolute latitude or farther from the equator. The location-only season begins
earlier at higher latitudes and follows the local hemisphere. Reminder state is
persisted before notification submission, repeats no more than once every 30
days, and escalates immediately if fresh freezing-weather evidence follows a
seasonal reminder. Winterization mode suppresses reminders.

The `Sprinkler Report` endpoint exposes three independent chart requests so a
client can load every chart in parallel. None asks for a zone. The calculated
available-water chart facets all configured valve devices with their field-
capacity and plant-specific watering and critical reference lines, with
scheduled, skipped, failed, manual, and completed watering decisions shown as
markers; water usage places every zone on one shared time axis and
distinguishes rain, observed irrigation, forecast rain, and scheduled water;
and the weather/ET response aligns provider ET, a combined dual-axis
temperature/humidity panel, and sustained wind and gusts. Modeled ET remains an
internal water-balance input rather than a sparse standalone panel. Each sparse
usage bucket begins one horizontal colored stack in its zone lane. The App emits
numeric `display_start,display_end` synthetic seconds on a linear x scale. The
real Hub-local calendar date in `bucket_starts_on` explicitly owns a generic
span guide: clients merge each date's visible intervals and center its
localized label over every connected span. The synthetic seconds never become
guide labels.
Every response uses a 600-second full stack and a chart-wide maximum water
amount to form one seconds-per-millimeter scale. Rectangle lengths remain
amount-proportional except for bounded whole-second allocation needed to keep
every positive amount nondegenerate. All zones at the same real bucket share a
display anchor; the next anchor follows the prior bucket's greatest rounded
stack extent. A real gap of 0–59 seconds is retained, while a 60-second-or-
larger gap becomes a fixed 30-second display gap. That gap is 5% of a full
stack and leaves room for four one-second positive-contributor floors. A
partial first or last query bucket changes only its exact accumulated amount;
it cannot collapse every segment in the chart. The exact millimeter amount and
real calendar bucket in the tooltip remain authoritative; clients render the
complete supplied coordinates literally and perform no geometry repair or
stacking. The balance markers explain controller decisions while usage shows
their water-accounting effect without a redundant activity timeline.
A zone with no rows in one represented window receives a localized
no-recorded-data text annotation instead of a fabricated event or zero-width
water bar. Zone identity remains the configured `LibertasDevice`, which the
client resolves normally; the report adds neither a duplicate zone name nor
`FormattedText` indirection.

Every report request exposes nullable, timezone-free `starts_on` and `ends_on`
calendar dates through native date pickers; both dates are inclusive. Water
usage interprets those values as Hub-local dates, while balance and weather/ET
retain UTC dates. A client can send both as null immediately instead of showing
a query form. The three requests share one validated request-range type. A
request is valid when either date is blank or when the first date is not later
than the last date. Each response retains its actual effective range and its
current selectable range as hidden transaction context. `RangeMin` and
`RangeMax` constrain both native date pickers to that selectable range, while
`CopyFrom` initializes the next request to its earliest and latest dates.
Initial requests have no prior response, skip `CopyFrom`, and keep both dates
null so the default report can run immediately. The server applies the same
bounds before generating the response. Balance can expose up to 365 dates and
ends on the current UTC date. Usage can expose up to 730 Hub-local dates, and
weather/ET up to 31 UTC dates; both forecast-capable charts end on the last date
of the seven-day forecast horizon. The first date is the earliest retained date
within that chart-specific maximum window. Water usage always uses Hub-local
calendar-day buckets, including across UTC offset changes.
Forecast rain and scheduled water are clipped at the report-generation time,
so a past date can never label a projected input. Available water is a
calculated root-zone balance, not a soil-moisture sensor reading. Water is
reported as depth in millimeters because the configuration has no zone area or
flow meter. If a selected water-balance boundary has no data, the chart carries
forward the nearest earlier daily checkpoint. With no trustworthy earlier
state, it leaves the available-water series empty instead of inventing 0%.

Report weather, watering activities, and daily balance/accounting checkpoints
are retained without an age-based deletion window. Supplying both bounds can
select up to 365 days of water balance, two years of water usage, or 31 days of
weather/ET. If a selected range is longer, the App keeps the final date and
discards the excess earliest dates instead of rejecting the request. Every
request converts its inclusive first and last calendar dates to an internal
half-open UTC timestamp range, preserving exact chart samples while keeping
calendar selection simple. This bounds one response without limiting how old
the requested data may be. Explicit provider corrections can replace or remove
their matching weather records. The controller reconstructs a separate recent
water ledger at startup, without loading the indefinite report archives.

Startup recovery seeks the water-event index at the persisted balance date and
reads successive 64-record pages through the end of the ledger. It also scans
backward for intervals that began before that date but still overlap it; a long
manual run can precede many shorter weather periods. Each timer callback reads
one page and yields using fresh monotonic ticks. There is no fixed 1,024-record
cutoff. All valid unaccounted weather and irrigation events are recovered before
the controller starts. An invalid or incomplete recovery leaves automatic
watering stopped and does not delete unaccounted input. Already-folded records
are removed only after the complete read has been verified.

The 512-event threshold triggers folding of completed UTC days into the saved
balance and daily reports. It is not a hard storage cap: records from a busy
current day remain intact even above that threshold. Daily reports and advanced
balance checkpoints are read back after writing, before covered ledger inputs
are deleted. Startup also rejects an incomplete modeled-weather-gap recovery
instead of substituting an empty history.

Historical weather now includes temperature, relative humidity, sustained wind,
and gusts in addition to precipitation and reference ET. The weather agent
obtains these from the same provider hourly response and validates them before
publishing or persisting the period. This begins honest wind history when the
new schema is deployed; unavailable periods remain gaps instead of being
inferred.

The report also retains accepted 15-minute current-condition samples so the
exact wind, gust, freeze, or rain evidence behind a controller decision remains
visible after hourly history catches up. Completed hourly periods remain the
sole rain/ET accounting source to avoid double-counting overlapping current
samples. Daily checkpoints label provider coverage and persist any recent-
weather, location/season, or conservative ET used to model a gap.

Before issuing a timed Matter `Open` command, the controller durably records a
`CommandPending` activity in its single current-state record, with the planned
start, duration, and water depth.
That reservation is not counted as delivered irrigation. The first observed
open creates a durable restart checkpoint; unobserved failed commands cannot
manufacture delivered water. Planned and accounted duration remain separate.
Each observed run has one indexed activity
record updated throughout its lifecycle. Future plans, pending command attempts,
and superseded calculations use the current-state record without creating new
archive rows. No-water failures and skips retain one representative per zone,
UTC planned day, outcome, and reason, regardless of origin: the earliest attempt's
identity and planned fields remain, while `updated_at` advances to the latest identical
failure. Any observed watering fields prevent this filtering, so a failed run
that actually opened the valve remains a distinct record. Failed valve commands
wait at least 60 seconds before another attempt, using the existing periodic
evaluation. A manually opened valve is never commandeered or closed by the
controller: it blocks other automatic watering, is checkpointed while open, and
is finalized when observed closed. Every newly observed close starts a
10-second controller-wide delay before another automatic open decision.

The SDK's asynchronous shutdown handler blocks new controller work, checkpoints
one zone per timer callback, verifies its database writes, and then calls
`libertas_shutdown_complete`. An accepted explicit-duration `Open` continues
on the valve. Shutdown saves the expected full timed interval in the ledger,
with an authoritative single-record checkpoint under
`SPRINKLER_WATERING_RESTART_V1` that distinguishes its confirmed prefix from its
expected finish. This checkpoint is also saved on the first open observation
and during normal accounting, so a sudden power loss needs no shutdown callback.

On restart, the controller restores the confirmed prefix and waits for a fresh
valve state before allowing another automatic action. Open confirms continuity
of the same run: the restart gap is included once, and accounting resumes from
that callback without sending another `Open` or resetting the valve's timer.
Closed shortens the run to the earlier of the callback time and expected timed
finish, preserving previously confirmed water. Early closure is recorded as
`StoppedEarly`. The corrected activity and ledger segment are verified before
clearing the restart checkpoint. An interrupted reconciliation replays the saved
correction, preserving the same activity and ledger index.

These rules cover task upgrades and brief outages that restart both the Hub and
valve. A state callback has no historical stop timestamp or run identifier:
watering during downtime is inferred, with uncertainty up to the gap between
the last checkpoint and first callback. If the valve loses power and closes,
this can count a few outage minutes as watering. Manual runs remain externally
owned and have no assumed timed finish.

The temporary V1/V2 activity archive repair has been removed. Startup now begins
directly with the permanent ledger recovery before enabling the controller.
Permanent archive filtering and the command retry delay remain in effect.
The historical `SprinklerData::ActivityArchiveCleanupV1` variant and its database
resource labels remain for stored-data compatibility; startup no longer reads or
writes cleanup markers.

Internet weather is an enhancement rather than a watering dependency. The
controller always projects demand from the best available source: at least one
day of persisted local reference evapotranspiration, otherwise a cached Hub
location and seasonal latitude estimate, and finally a conservative built-in
5 mm/day reference rate. Missing or stale weather therefore produces an
offline estimated schedule and does not prevent automatic watering. Fresh rain,
freezing, or excessive-wind observations still delay a run for safety. There is
no blanket calendar-based winter cutoff: the seasonal estimate reduces winter
demand, while actual fresh freezing conditions provide the safety cutoff.
When a zone is first configured, its balance starts fully replenished at the
current time because weather history cannot reveal unobserved prior irrigation;
the controller does not manufacture a dry-week catch-up run.
