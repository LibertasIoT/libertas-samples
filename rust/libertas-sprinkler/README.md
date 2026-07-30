# libertas-sprinkler

A `no_std` Libertas application library that calculates and executes
weather-aware irrigation schedules.

Configuration contains:

- one `SprinklerWeatherProtocolV1` client endpoint shared by all zones;
- one Matter Irrigation System valve per zone;
- a soil and planting profile;
- the zone's measured application rate in millimeters per hour; and
- one server endpoint that exposes the zone's calculated schedule.

The application deliberately has no raw `field_capacity` or sprinkler-head
configuration. Soil and planting profiles derive root-zone water capacity, and
the measured application rate converts observed Matter Valve open time into
applied water.

At runtime, users can set one normalized “more or less water” value from `-1.0`
through `1.0` and replace the zone's hold-off intervals. The published schedule
shows the next watering slot, planned amount, estimated deficit, recent rain and
irrigation, constraints, and valve status.

Each zone persists a seven-day water ledger. Weather history supplies
precipitation and evapotranspiration; Matter Valve state subscriptions measure
both automatic and manual watering. Open valves are checkpointed every minute
so restart loss is bounded.
