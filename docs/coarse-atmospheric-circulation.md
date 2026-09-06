# Coarse atmospheric circulation

This climate slice derives a deterministic, phase-resolved near-surface wind
field on the authoritative spherical mesh. It consumes the selected seasonal
temperature and final adjusted elevation without modifying either input or
feeding any result back into earlier climate or geology stages.

For each cell, a least-squares fit over its mesh neighbors estimates the local
tangent temperature gradient in kelvin per radian. The ideal-gas thermal
pressure-gradient approximation is

```text
a = R_specific grad(T) / planet_radius
```

and a steady linear surface balance is solved exactly:

```text
drag u + f (normal cross u) = a
f = 2 rotation_rate sin(latitude)
```

This produces direct gradient-following flow on a non-rotating planet and
Coriolis deflection whose sign and magnitude emerge continuously from the
supplied rotation and cell latitude. There are no hard-coded 30-degree or
60-degree circulation bands and no stochastic curl field.

Terrain steering is deliberately bounded. The stage estimates the final
elevation gradient with the same local fit and removes only the configured
fraction, from zero to one, of an upslope wind component. It never adds energy,
changes a downslope component, or performs terrain-scale flow simulation. A
configured maximum wind speed is a final numerical and modeling safety bound.
Every output vector is reprojected onto its cell tangent plane.

`Planet::EARTH` owns the convenient Earth-like radius, sidereal rotation
period, and dry-air specific gas constant. `AtmosphericCirculationConfig::EARTHLIKE`
owns the Earth-like surface-drag, terrain-steering, and maximum-speed choices.
They are caller-selected preset values, not solver constants:

- radius: `6,371,000 m`
- sidereal rotation period: `86,164.0905 s`
- atmospheric specific gas constant: `287.05 J/kg/K`
- linear surface drag: `1.5e-5 s^-1`
- terrain steering: `0.65`
- maximum wind speed: `100 m/s`

Outputs include tangent wind vectors and scalar wind speed, temperature-gradient
magnitude, pressure-gradient acceleration, signed Coriolis parameter, and the
fractional speed reduction from terrain steering. Aggregate diagnostics report
spherical-area-weighted summaries plus calm, terrain-steered, and speed-capped
cell counts and maximum tangency error.

This is coarse diagnostic circulation, not full atmospheric dynamics. It adds
no moisture, precipitation, ocean currents, ice, coupled climate feedback,
vertical structure, mass continuity solve, full CFD, rasterization, parallel
execution policy, or acceleration backend.
