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
signed boundary deformation from the final state before composing coarse
elevation. It also derives mantle-hotspot trails and volcanic-arc fields from
the final state without modifying elevation. Volcanic arcs group mixed-crust
convergent boundaries by overriding continental plate, walk a bounded distance
inland, and retain strength-ranked peak candidates. Continental divergent
deformation exposes a configurable negative graben center, weaker negative
flanks, and bounded decay; oceanic ridges remain owned by bathymetry so their
profile is not counted twice. The viewer toggles
topology, plate, crust, seafloor-age, base-elevation, deformation,
composed-elevation, hotspot, volcanic-arc, motion, and final-boundary diagnostics
and reports their aggregate statistics plus stage timings. Plate interiors use
stable per-plate colors. Crust is blue for oceanic, amber for continental, and
white where the classes meet. Seafloor age runs from cyan ridge cells to dark
blue old crust, with continental cells brown. Base and composed elevation use a
deep-water-through-highland color ramp. Deformation runs from blue subsidence
through dark zero to orange uplift. Motion arrows sample each plate's local
tangent velocity field. Static boundaries are red for convergent, blue for
divergent, and yellow for transform. Volcanic arcs use an orange-to-yellow
strength ramp with cross markers for peak candidates.

The viewer is a consumer only. Generation and topology logic belong in reusable
crates, never in this application.
