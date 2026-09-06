# Solar forcing

The first climate slice is deterministic top-of-atmosphere solar forcing over
the authoritative spherical mesh. `procgen-planet` owns the minimal stellar and
orbital inputs used by the calculation and supplies an Earth preset. The
`procgen-climate` stage contains no Earth-specific constants.

Orbital phase is elapsed orbital time in orbits since periapsis and wraps
periodically. The stage solves Kepler's equation, derives orbital distance and solar declination, then
computes daily-mean insolation for each cell from its Y-up latitude. Polar night
is exactly zero and polar day uses a full-day sunset hour angle. Values are in
watts per square meter and are clamped to the physically available stellar flux.

The annual field is a bounded-cost numerical mean. It samples the orbit at the
configured number of mean-anomaly midpoints, which weights eccentric orbits by
elapsed time rather than true anomaly. The supported sample count is 4 through
4096. Diagnostics report the effective wrapped phase, current distance, flux,
declination, polar day and night counts, and spherical-area-weighted current and
annual summaries.

This forcing stage reads only the mesh and planet model. It does not read
elevation and does not itself model temperature or any atmospheric response.
The independent static radiative-equilibrium consumer is documented in
`docs/radiative-equilibrium-temperature.md`.
