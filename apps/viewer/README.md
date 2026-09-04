# Procgen viewer

Run from the workspace root:

```sh
cargo run -p procgen-viewer
```

Drag the empty viewport with the left mouse button to orbit and use the scroll
wheel to zoom. The control panel regenerates the sphere from cell count, jitter,
and seed, toggles cell centers and Delaunay/Voronoi topology, and reports stage
timings.

The viewer is a consumer only. Generation and topology logic belong in reusable
crates, never in this application.
