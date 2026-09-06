# Moisture transport and precipitation

This climate slice derives a deterministic atmospheric water field from the
selected seasonal temperature, final adjusted elevation, and coarse tangent
wind field. It is a pure generation stage: every run begins with an empty
atmospheric column and retains no history from an earlier run.

The solver uses a configured number of fixed-duration explicit steps. Humidity
is represented as column water in `kg/m2`; multiplication by spherical cell area
gives the conserved quantity used internally. Each step performs four bounded
operations in stable cell and edge order:

1. Exposed ocean cells evaporate toward their local moisture capacity. The
   capacity is an explicitly bounded exponential function of the supplied
   temperature.
2. Wind exports a CFL-bounded fraction of each cell's water mass to downwind
   mesh neighbors. Directional weights are normalized per donor, so every
   exported unit is added exactly once to another cell.
3. Water above the receiving cell's temperature-dependent capacity condenses.
4. Background rainfall removes a bounded fraction of the remaining column.
   On land, positive wind-aligned terrain ascent adds a separately reported
   orographic removal whose per-step fraction has a hard configured maximum.

The output contains final humidity and capacity plus simulated-duration mean
evaporation, total precipitation, capacity condensation, and orographic
precipitation rates. Total precipitation includes condensation, background
rainfall, and the orographic component. `1 kg/m2/day` of liquid water is
numerically equivalent to `1 mm/day`.

Diagnostics include spherical-area-weighted ranges and means, cell counts,
simulated duration, the maximum orographic fraction actually applied, and the
water-budget residual:

```text
evaporation - final atmospheric humidity - precipitation
```

With an initially empty atmosphere, that residual should remain near floating-
point roundoff. Transport itself neither creates nor removes water.

`MoistureTransportConfig::EARTHLIKE` owns convenient values for step duration,
capacity, temperature response, evaporation and rainfall rates, orographic
conversion and bound, and the transport bound. `Planet` owns the physical land
elevation scale used by this and future terrain-aware stages. The algorithm
contains no built-in Earth temperature, humidity, or precipitation constants.

This is only deterministic moisture transport and precipitation. It adds no
rivers, runoff, groundwater, vegetation, evapotranspiration, erosion, ocean
currents, snow or ice, climate feedback, rasterization, persistent state,
parallel execution policy, or acceleration backend.
