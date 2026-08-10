# libertas-sprinkler_agent

A `no_std` Libertas application library that calculates and executes
weather-aware irrigation schedules.

Configuration contains:

- one `SprinklerWeatherProtocolV1` client endpoint shared by all zones;
- one to 16 unique Libertas users who receive application reminders, currently
  including system-wide winterization reminders;
- one Matter Irrigation System valve per zone;
- a plant type and sprinkler-head type per zone; and
- one server endpoint that exposes the zone's complete current state.

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
hold-off intervals. The published Active state contains all current zone data:
the adjuster first, followed by the next watering slot, planned amount,
estimated deficit, recent rain and irrigation, constraints, and valve status.
Expired hold-offs are removed on the next schedule evaluation, persisted, and
published to zone subscribers even if the rest of the schedule is unchanged.
The top-level state is an Active/Winterization union. Active contains the
complete current zone data; Winterization contains no fabricated watering slot.
One system-wide Watering mode control selects Active or Winterization and is
persisted locally across restarts and internet outages.

While Watering mode is Active, Libertas Notification reminds the configured
users to winterize. Fresh current conditions or a fresh seven-day forecast at
3 °C or below trigger the weather reminder at any latitude. When weather is
unavailable, the cached Hub location provides a seasonal fallback only at 35°
absolute latitude or farther from the equator. The location-only season begins
earlier at higher latitudes and follows the local hemisphere. Reminder state is
persisted before notification submission, repeats no more than once every 30
days, and escalates immediately if fresh freezing-weather evidence follows a
seasonal reminder. Winterization mode suppresses reminders.

Each weather period and observed irrigation interval is persisted as a separate
indexed water event. Startup loads and validates those records to reconstruct a
bounded seven-day ledger dynamically. Weather corrections and minute valve
checkpoints update only their matching indexed record rather than rewriting the
full ledger. Every irrigation interval records the zone's watering percentage;
changing that setting while a valve is open first checkpoints the old value and
starts a separate adjacent interval for the new value.

Internet weather is an enhancement rather than a watering dependency. The
controller always projects demand from the best available source: at least one
day of persisted local reference evapotranspiration, otherwise a cached Hub
location and seasonal latitude estimate, and finally a conservative built-in
5 mm/day reference rate. Missing or stale weather therefore produces an
offline estimated schedule and does not prevent automatic watering. Fresh rain,
freezing, or excessive-wind observations still delay a run for safety. There is
no blanket calendar-based winter cutoff: the seasonal estimate reduces winter
demand, while actual fresh freezing conditions provide the safety cutoff.
