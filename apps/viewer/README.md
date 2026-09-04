# Procgen viewer

Run from the workspace root:

```sh
cargo run -p procgen-viewer
```

Drag the empty viewport with the left mouse button to orbit and use the scroll
wheel to zoom. The control panel regenerates the sphere and tectonic plate
partition, toggles topology and plate diagnostics, and reports stage timings.
Plate interiors use stable per-plate colors and plate boundaries are white.

The viewer is a consumer only. Generation and topology logic belong in reusable
crates, never in this application.
