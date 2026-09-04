# Procgen viewer

Run from the workspace root:

```sh
cargo run -p procgen-viewer
```

Drag the empty viewport with the left mouse button to orbit and use the scroll
wheel to zoom. The control panel regenerates the sphere and tectonic plate
partition and angular velocities, toggles topology, plate, motion, and boundary
diagnostics, and reports stage timings. Plate interiors use stable per-plate
colors. Motion arrows sample each plate's local tangent velocity field. Static
boundaries are red for convergent, blue for divergent, and yellow for transform.

The viewer is a consumer only. Generation and topology logic belong in reusable
crates, never in this application.
