# Procgen viewer

Run from the workspace root:

```sh
cargo run -p procgen-viewer
```

Drag the empty viewport with the left mouse button to orbit and use the scroll
wheel to zoom. The control panel regenerates the sphere, tectonic plate
partition, static crust classification, angular velocities, and one simultaneous
migration transition for each configured evolution step, then derives an
independent seafloor hop-age field, ridge-to-deep oceanic base elevation, and
signed boundary deformation from the final state before composing tectonic
elevation. Present-day geology fields are then derived without modifying that
elevation: mantle-hotspot trails follow final-owner plate motion, volcanic arcs
walk inland from final mixed-crust convergent boundaries, and craton strength
ramps across continental land by graph distance from final plate boundaries.
Those fields are composed into a new geological elevation in stable hotspot,
volcanic-arc, craton, then basin order. Cratons flatten toward the configured
continental base and basins flatten toward their original component minima;
sparse seamount and abyssal-hill candidates are not applied to elevation.
Finally, deterministic isostatic support combines current-owner continental
crust, final convergent and divergent boundary proximity, craton strength, and
basin membership, then nudges a separate clamped elevation field toward that
support. Oceanic cells and sedimentary-basin floors remain unchanged.
An independent solar-forcing stage then derives top-of-atmosphere daily-mean
insolation for the selected orbital phase and a bounded-sample annual mean from
the spherical mesh. A second independent stage derives daily and annual
effective radiative-equilibrium temperatures from those fields using explicit
per-cell albedo and uniform emissivity. The viewer selects the Earth and Earthlike
presets by default; neither stage consumes elevation or models atmospheric or
surface dynamics.
A third climate stage uses the final adjusted elevation and sea level to choose
explicit land or ocean surface heat capacity, then solves each cell's isolated
energy balance onto a periodic seasonal cycle. It produces selected-phase,
annual-mean, minimum, maximum, and amplitude temperatures without lateral heat
transport or persistent state.
A fourth climate stage fits local temperature gradients on the spherical mesh,
then derives finite tangent surface winds from explicit planetary radius,
rotation, atmospheric gas constant, linear drag, and bounded terrain steering.
It contains no prescribed latitude bands or coupled feedback. A fifth stage
starts from an empty atmospheric column, evaporates water from exposed ocean
cells according to temperature-dependent capacity, transports it conservatively
over mesh edges with those winds, and derives capacity condensation, background
rainfall, and bounded terrain-ascent orographic precipitation. It reports final
humidity, duration-mean precipitation components, and a water-budget residual
without retaining state between regenerations.
A sixth stage uses the sampled seasonal temperatures, precipitation, and final
land/ocean mask to solve bounded periodic snow and sea-ice reservoirs and an
equilibrium land-ice cover fraction. The bounded solve starts from fixed known
bounds on every regeneration and contains no fixed latitude ice assignment or
internal climate feedback. A bounded orchestration stage reruns the five
downstream climate components to a periodic fixed point using only explicit
per-cell land, ocean, snow, and ice albedo feedback. Configurable RMS
tolerances, under-relaxation, and a hard iteration limit bound the solve, and
every regeneration starts from the same fully covered albedo field with no
persistent climate state.
Continental divergent deformation exposes a configurable negative graben
center, weaker negative flanks, and bounded decay; oceanic ridges remain owned
by bathymetry so their profile is not counted twice.

The viewer selects one filled per-cell surface at a time and independently
toggles edge, marker, and vector overlays. Surface choices include plate,
crust, seafloor-age, base-elevation,
deformation, tectonic-elevation, geological-elevation, isostatic-support,
adjusted-elevation, hotspot, volcanic-arc, craton, basin, insolation, coupled
surface-albedo, daily- and annual-temperature, atmospheric supporting scalars,
wind speed, humidity, precipitation, snow cover, land-ice cover, and sea-ice
cover. Delaunay, Voronoi, and final-boundary diagnostics remain edge overlays;
cell centers remain markers; wind and plate motion remain vectors. The viewer
also reports aggregate statistics and stage timings.
Filled fields use each cell's exact value across its flat Voronoi face rather
than averaging neighboring values onto shared edges.
Plate interiors use stable per-plate colors.
Crust is blue for oceanic, amber for continental, and white where the classes
meet. Seafloor age runs from cyan ridge cells to dark blue old crust, with
continental cells brown. Base, tectonic, and geological elevation use a
deep-water-through-highland color ramp. Deformation runs from blue subsidence
through dark zero to orange uplift. Motion arrows sample each plate's local
tangent velocity field. Static boundaries are red for convergent, blue for
divergent, and yellow for transform. Volcanic arcs use an orange-to-yellow
strength ramp with cross markers for peak candidates. Cratons use a
dark-green-to-pale-gold strength ramp.
Daily-mean insolation runs from dark polar night through blue and cyan to warm
yellow at the current field maximum; the controls expose orbital phase and the
bounded annual sampling count. Temperature uses a fixed kelvin color scale from
dark zero through cold blue, pale freezing-point temperatures, warm yellow, and
hot red. Radiative controls expose emissivity; climate-coupling controls expose
the four surface albedos.
Seasonal thermal controls expose land and ocean heat capacity and orbital
period; the solar-forcing annual sample control is shared by both stages.
Separate layers show the selected phase,
annual mean, annual minimum, annual maximum, and peak-to-trough amplitude;
summary diagnostics include surface-class aggregates and periodic closure.
Planet controls expose radius, rotation, and atmospheric gas constant separately
from the atmospheric-circulation controls for drag, terrain steering, and the
speed cap.
Supporting layers show temperature-gradient magnitude, pressure-gradient
acceleration, signed Coriolis parameter, and applied terrain steering.
Humidity and precipitation layers show atmospheric column water in `kg/m2` and
the total duration-mean precipitation rate in `kg/m2/day`; controls expose the
fixed-step water-budget, capacity, evaporation, rainfall, orographic, terrain
ascent, and transport parameters. The physical terrain scale lives with the
planet controls. Aggregate diagnostics separate condensation
and orographic precipitation and report mass-balance closure.
Cryosphere controls expose temperature thresholds, snow capacities, degree-day
melt factors, sea-ice growth and melt rates, and fixed-point solver bounds.
Diagnostics report annual accumulation/ablation, covered-cell counts, periodic
closure, and snow-mass and sea-ice-cover balance residuals.
Coupling diagnostics report iteration count and RMS convergence residuals;
the component panels own moisture and cryosphere conservation diagnostics.

The viewer is a consumer only. Generation and topology logic belong in reusable
crates, never in this application.
