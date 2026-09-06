# Bounded climate coupling

`procgen-climate::derive_coupled_climate` is the orchestration boundary around
the existing radiative-equilibrium, seasonal-thermal, atmospheric-circulation,
moisture-transport, and cryosphere stages. Each component remains available as
an independent pure function. The orchestrator adds only one feedback: the
cryosphere's selected snow, land-ice, and sea-ice cover updates an explicit
per-cell surface albedo field for the next radiative and seasonal solve.

Bare cells use configured land or ocean albedo. Land ice blends bare land
toward ice albedo, then snow blends that result toward snow albedo. Sea ice
blends ocean toward ice albedo. The deterministic solve starts from fully
covered land and ocean albedos to select the cold fixed-point branch, applies a
configurable under-relaxed albedo update, and stops when area-weighted RMS
albedo, selected-temperature, precipitation, and cover changes all meet their
configured tolerances. A hard iteration limit returns an error. Every call
reconstructs its working fields and retains no state between generation runs.

The result contains the ordinary output of every component plus the converged
albedo field. Diagnostics report iteration count, the four convergence
residuals, radiative-equilibrium closure (subject to stored `f32` rounding), and
the existing moisture, snow, land-ice, and sea-ice conservation residuals.

This slice adds no cloud, greenhouse, ocean-heat-transport, carbon, vegetation,
glacier-flow, erosion, rasterization, or acceleration model.
