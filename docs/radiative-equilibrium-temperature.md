# Radiative-equilibrium temperature

The second climate slice is a deterministic, static radiative-equilibrium
response to `SolarForcing`. Each cell uses its explicit input albedo `a` and a
configured longwave emissivity `epsilon`, balancing absorbed radiation against
Stefan-Boltzmann emission:

```text
T = ((1 - a) Q / (epsilon sigma))^(1/4)
```

`Q` is either the cell's daily-mean or annual-mean top-of-atmosphere insolation,
`sigma` is the SI Stefan-Boltzmann constant, and the resulting effective
temperature is in kelvin. Zero insolation produces exactly 0 K. Albedo must be
between zero and one; emissivity must be greater than zero and at most one.

The daily field is the instantaneous equilibrium response to the selected
orbital phase's daily-mean forcing. The annual field is equilibrium with the
annual-mean forcing. It is deliberately not an average of daily temperatures:
there is no heat capacity or time integration in this stage. Diagnostics report
spherical-area-weighted means and extrema for both fields.

`RadiativeEquilibriumConfig::EARTHLIKE` supplies a convenient emissivity of 1.0.
Albedo is always a required per-cell input; callers that want a uniform value
construct a uniform field. The stage does not read elevation or crust and does not
model an atmosphere, greenhouse effects, lapse rates, heat transport or capacity,
land-ocean differences, wind, moisture, ice, feedbacks, iterative coupling,
rasterization, or acceleration backends.

The separate time-dependent consumer is documented in
`docs/seasonal-thermal-response.md`; the orchestration boundary is documented
in `docs/climate-coupling.md`.
