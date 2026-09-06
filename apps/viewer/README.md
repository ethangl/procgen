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
uniform albedo and emissivity. The viewer selects the Earth and Earthlike
presets by default; neither stage consumes elevation or models atmospheric or
surface dynamics.
Continental divergent deformation exposes a configurable negative graben
center, weaker negative flanks, and bounded decay; oceanic ridges remain owned
by bathymetry so their profile is not counted twice.

The viewer toggles topology, plate, crust, seafloor-age, base-elevation,
deformation, tectonic-elevation, geological-elevation, isostatic-support,
adjusted-elevation, hotspot, volcanic-arc, craton, basin, insolation, daily- and
annual-temperature, motion, and final-boundary diagnostics and reports aggregate
statistics plus stage timings. Plate interiors use stable per-plate colors.
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
hot red. Its controls expose the uniform albedo and emissivity.

The viewer is a consumer only. Generation and topology logic belong in reusable
crates, never in this application.
