# Seasonal thermal response

The third climate slice adds deterministic seasonal thermal inertia to the
phase-resolved solar forcing and radiative-equilibrium temperature stages. It
solves one independent local surface energy balance per spherical cell:

```text
C dT/dt = epsilon sigma (T_eq(t)^4 - T(t)^4)
```

`T_eq(t)` is the effective radiative-equilibrium temperature derived from the
cell's daily-mean insolation at orbital phase `t`. `epsilon` and `sigma` are the
same explicit emissivity and Stefan-Boltzmann constant used by the radiative
stage. `C` is an explicit effective surface heat capacity in J/m2/K. Final
elevation strictly above the pipeline's `SEA_LEVEL` selects the land capacity;
elevation at or below it selects the ocean capacity. This is the shared tectonic
elevation predicate used elsewhere in the pipeline. Sea level is not
configurable in the current pipeline. The stage does not inspect tectonic crust
classes.

The orbit uses the solar-forcing stage's single bounded count of 4 to 4096
uniform elapsed-time intervals; seasonal response has no second sampling
setting that can disagree. Forcing is sampled at interval midpoints. Longwave
emission is integrated with an implicit step whose unique solution is bounded
between the previous temperature and the current radiative target. This makes
large time steps and small positive capacities stable without inventing
temperature overshoot. A capacity of exactly zero bypasses integration and
follows phase-resolved radiative equilibrium exactly.

Positive-capacity cells solve their initial temperature as a fixed point of one
complete orbit. Consequently every generation starts directly on the periodic
steady seasonal cycle and retains no state from an earlier run. A bounded
Newton solve uses at most 48 refinements per cell and reports both its maximum
iteration count and the maximum one-orbit closure error. Failure to reach the
documented `1e-7 K` closure tolerance is an error rather than a partially
converged output.

The output contains temperature at the selected orbital phase and per-cell
annual mean, minimum, maximum, and peak-to-trough amplitude. Diagnostics report
spherical-area-weighted summaries for all five fields, land and ocean cell
counts, selected-phase area-weighted means for each available surface class,
and periodic-convergence measurements.

`SeasonalThermalConfig::EARTHLIKE` is only a convenient caller-selected preset:

- land heat capacity: `5e7 J/m2/K`
- ocean heat capacity: `4e8 J/m2/K`
- orbital period: `365.256363004 days`

The default solar-forcing config supplies the shared `96` orbital samples.

Heat capacity may be exactly zero or is bounded to `[1, 1e12] J/m2/K`; orbital
period is bounded to `[0.01, 1e6]` days. These are execution and numerical-safety
bounds, not claims that every extreme is a realistic surface.

There is deliberately no greenhouse atmosphere, elevation lapse rate, lateral
heat transport, wind, moisture, current, ice, phase change, coupled feedback,
persistent inter-run state, rasterization, parallel execution policy, or
acceleration backend in this stage.
