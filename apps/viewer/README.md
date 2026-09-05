# Procgen viewer

Run from the workspace root:

```sh
cargo run -p procgen-viewer
```

Drag the empty viewport with the left mouse button to orbit and use the scroll
wheel to zoom. The control panel regenerates the sphere, tectonic plate
partition, static crust classification, angular velocities, and one simultaneous
migration transition for each configured evolution step, then derives coarse
elevation from the final state. It toggles topology, plate, crust, elevation,
motion, and final-boundary diagnostics and reports aggregate evolution and
elevation statistics plus stage timings. Plate interiors use stable per-plate
colors. Crust is blue for oceanic, amber for continental, and white where the
classes meet. Elevation uses a deep-water through highland color ramp. Motion
arrows sample each plate's local tangent velocity field. Static boundaries are
red for convergent, blue for divergent, and yellow for transform.

The viewer is a consumer only. Generation and topology logic belong in reusable
crates, never in this application.
