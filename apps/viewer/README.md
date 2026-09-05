# Procgen viewer

Run from the workspace root:

```sh
cargo run -p procgen-viewer
```

Drag the empty viewport with the left mouse button to orbit and use the scroll
wheel to zoom. The control panel regenerates the sphere, tectonic plate
partition, static crust classification, angular velocities, and one simultaneous
migration step; toggles topology, plate, crust, motion, boundary, and migration
diagnostics; and reports stage timings. Plate interiors use stable per-plate
colors. Crust is blue for oceanic,
amber for continental, and white where the classes meet. Motion arrows sample
each plate's local tangent velocity field. Static boundaries are red for
convergent, blue for divergent, and yellow for transform. Migrated cell outlines
are magenta with the winning source boundary highlighted in yellow.

The viewer is a consumer only. Generation and topology logic belong in reusable
crates, never in this application.
