# Deterministic cryosphere

This slice derives seasonal snowfall and melt, seasonal snow cover, perennial
land-ice cover, and seasonal sea-ice cover from fields already produced by the
pipeline. Its only inputs are the sampled annual temperature cycle, the current
precipitation climatology, final adjusted elevation, and the existing sea-level
land predicate. Ice is never assigned from latitude.

The seasonal thermal stage exposes cell-major midpoint temperatures for its
uniform annual intervals. The cryosphere advances one interval per sample. A
shared orbital helper selects the interval containing the requested phase; the
cryosphere reports the reservoir after that complete interval rather than
interpolating between two reservoir states. The current
precipitation rate is held constant through that cycle; temperature determines
whether it falls as snow. On land, snowfall fills a bounded seasonal snow
reservoir, positive degree days melt available snow, and persistent overflow is
reported as land-ice accumulation. Land-ice cover is the bounded area fraction
whose warm-season ablation can balance that accumulation; accumulation beyond
full-cover ablation remains visible as positive mass balance. This produces no
glacier thickness or flow state.

On ocean cells, negative degree days grow fractional sea-ice cover and positive
degree days melt it. Both operations are bounded by the remaining open-water or
ice fraction. The solver brackets each bounded reservoir between empty and full,
then applies the same annual forcing in a bounded fixed-point refinement. It
fails rather than returning an unconverged cycle. Every solve starts from those
fixed bounds; nothing persists between generation runs.

Outputs at the selected orbital phase include snowfall and land-snow melt rates
plus snow, land-ice, and sea-ice cover fractions. Aggregate diagnostics include
annual snowfall, snow melt, land-ice accumulation and ablation, sea-ice growth
and melt, land/ocean and covered-cell counts, refinements used, periodic closure,
and mass-balance diagnostics:

```text
snowfall - snow melt - perennial accumulation - change in snow storage
land-ice accumulation - land-ice ablation
sea-ice growth - sea-ice melt - change in sea-ice cover
```

`CryosphereConfig::EARTHLIKE` owns the freezing thresholds, snow capacities,
degree-day factors, sea-ice fractional rates, convergence tolerance, and solver
iteration limit. These are visible caller-selected parameters, not hidden properties of
the solver.

This stage does not add glacier flow, calving, isostasy, erosion, ocean
currents, ice-albedo or any other climate feedback, biomes, rasterization,
persistent state, parallel execution policy, or acceleration backends.
