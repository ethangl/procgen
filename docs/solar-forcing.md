# Solar forcing

The first climate slice is deterministic top-of-atmosphere solar forcing over
the authoritative spherical mesh. `procgen-planet` owns the minimal stellar and
orbital inputs used by the calculation and supplies an Earth preset. The
`procgen-climate` stage contains no Earth-specific constants.

Orbital phase is the fraction of elapsed orbital time since periapsis. The stage
solves Kepler's equation, derives orbital distance and solar declination, then
computes daily-mean insolation for each cell from its Y-up latitude. Polar night
is exactly zero and polar day uses a full-day sunset hour angle. Values are in
watts per square meter and are clamped to the physically available stellar flux.

The annual field is a bounded-cost numerical mean. It samples the orbit at the
configured number of mean-anomaly midpoints, which weights eccentric orbits by
elapsed time rather than true anomaly. The supported sample count is 4 through
4096. Diagnostics report the current distance, flux, declination, polar day and
night counts, and spherical-area-weighted current and annual summaries.

This stage reads only the mesh and planet model. It does not read elevation and
does not model temperature, greenhouse effects, atmosphere, wind, moisture,
ice, ocean transport, feedbacks, coupling, rasterization, or execution backends.
