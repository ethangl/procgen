# Real-time world pilot

A separate application for [the pilot design](../../docs/realtime-world-pilot.md).
It uses `procgen-core`, the CPU gradient-noise primitive from `procgen-noise`, and
cube-face geometry from `procgen-cubesphere`, and does not touch the existing world
pipeline or viewer.

The target shape is GPU height tiles as the far field and a GPU voxel world as the near
field, with CPU code kept as the reference oracle the shaders are tested against. The
viewer implements both halves: height tiles everywhere and a camera-centered band of
mixed-LOD voxel chunks over them.

## GPU orbit and descent viewer

```sh
cargo run -p procgen-realtime-pilot
```

The viewer loads the repository's `planet-design-300km.json` with its saved seed,
300 km radius, and complete octave stack. `--design-file PATH` overrides that file.
`--seed U64` selects the starter preset instead.

Use **Design** for radius, height bound, seed, sea level, file actions, and the
**Volume** 3D detail term, and **Octaves** for enable state, wavelength, amplitude,
sharpness, warp, and all damping values. Volume has an enabled checkbox, wavelength and
amplitude in meters, a sharpness slider, and the fade height above the surface, shipped
enabled at 24 m, 6 m, 0.5, and 12 m as starting values, not a measured preset. Valid
terrain edits apply 350 ms after the last
edit, and rapid edits coalesce. An edit, including an invalid draft, invalidates older
unpublished results. The last complete design stays visible while its replacement builds
on a worker; height meshes and field queries switch to the accepted design together.
Edits preserve the camera and its orientation, except that enabled altitude
protection raises it if the new terrain would cover it.

**Load…** opens a path dialog, validates the file, and makes it the current design.
**Save controls** overwrites that current file; its path is read-only in the panel,
so use **Save as…** to choose a different destination. Only a successful Load or
Save as changes the current file. **Copy JSON** copies the same complete design.
Controls never save automatically. `--write-design PATH` writes the initial config
and sets the viewer's save path. Camera, coloring, and control-panel state are not
generation data and are not saved.

The viewer renders a spherical ocean with depth-based shorelines, shallow-water
absorption, sky reflection, a sun highlight, and underwater tint. **Show ocean** and
**Sea level (m)** update immediately without terrain generation. Save and Load
include ocean settings in a separate top-level `ocean` object; files without one
default to enabled at zero meters. Water has no collision and no waves. See
[ocean rendering](../../docs/realtime-world-oceans.md) for limits and tests.

## Orbit and flight

Use **Descend continuously** to travel from orbit, or **Go to ground** for a direct
shortcut to five meters above terrain. **Fly** enables W/A/S/D movement, right drag
looks around, and E/Q move radially up and down. Scroll changes orbit distance or
flight speed. In Orbit, left drag rotates around the current screen axes, including
across the poles. Flight speed increases smoothly with terrain clearance above the
64 m ground band; the panel shows speed in m/s and the scroll multiplier.

**Keep camera 5 m above terrain** is enabled by default: it samples the canonical height
field at the camera direction and raises a low camera radially, and disabling it allows
inspection below terrain. It checks the endpoint only, so fast lateral flight can cross a
ridge, and it provides no walking, swept collision, gravity, or water contact. Camera
preferences are not saved in design files.

The GPU draws up to 384 height tiles with 64 by 64 quads each, with a one-meter
refinement target, stitched edges, and 250 ms replacement fades. Filtering follows
the selected tile layout and retains fine distant bands. Unchanged tiles reuse
their GPU buffers during movement; changed neighbors rebuild only affected
identities. See [height tile reuse](../../docs/realtime-world-height-reuse.md) for
the filtering rule, shared normals, and validation. Height generation keeps at most
two batches of 32 tiles in flight.

The viewer also draws a camera-centered voxel band over the height tiles: the
balanced octree selection for 384 requested leaves, with every chunk coarser than
16 m cells dropped, so cell spacing grows with distance from the camera and stops at
the cap. There is no altitude gate; the band empties by itself a few kilometers
above the design's height envelope and fills in again on descent. It advances one
closed replacement group at a time, refining live rather than waiting for the whole
target, and reserves 1,536 GPU mesh slots of 2,781,000 bytes inside a 5 GiB budget.
Its edge dissolves into the height tiles over the span of its coarsest chunk, at least
16 m, and the height surface underneath drops by that chunk's cell spacing plus the
volume term's amplitude, at least 1 m; that bias rule is provisional. **Show local
voxels** turns the layer off; it is on by default. Previous height, current height, and
local surfaces need three RGBA16F/Depth32F layers: 36 bytes per pixel, 197.8 MiB at
2880 × 2000.

**Material** is the default coloring, chosen per fragment from slope and altitude: soil
below 28 degrees blending to rock above 42; snow from 55 percent of the height limit and
full by 65, held off faces steeper than 35 degrees and gone by 50, so cliffs stay rock
through the snow line; and sand within 4 m of sea level fading out by 8 m, where the
ocean is on and the face is gentler than the rock threshold. The colors and angles are
starting values, not measured ones, and lighting is unchanged. **Height**, **Neutral**,
**LOD**, and **Normals** remain as inspection modes: Height uses a fixed ramp from minus
to plus the configured height limit above the reference radius, and under LOD each band
chunk reads its own level, so the spacing rings separate from the height tiles
underneath. See [the flight viewer](../../docs/realtime-world-flight-viewer.md) for
architecture and limits, and
[terrain draw visibility](../../docs/realtime-world-visibility.md) for radial tile bounds.

## Headless captures

The CPU library and the preview audit run without the engine dependencies:

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- --check
```

`--check` reports the build fingerprint, seed, radius, preview geometry counts and
spacing, resolved octave weights, and a full-resolution height distribution as JSON.
`--patch-span METERS` audits a ground patch instead of the whole planet, `--solo
INDEX` audits one octave band, and `--preview-quads 16|32|64|128|256` sets the
resolution. These select headless audits, not visual previews.

Record the fixed 90-second orbit/descent/flight route:

```sh
cargo run -p procgen-realtime-pilot -- \
  --explore-record /tmp/procgen-flight-route.csv
```

This writes frame and stage timings plus six adjacent PNG captures, then exits. The route
uses the default coloring, so the captures are material-colored. PNG encoding runs off
the main thread; clean exit waits for images to save. The CSV reports height generation,
buffer and target bytes, camera position, clearance and protection, local voxel counts,
bytes and spacings, and the stage timings; the
[flight viewer](../../docs/realtime-world-flight-viewer.md) lists every column in header
order and defines the ones that need it. The panel shows the same band counts, both
spacings, the leaves the world is settling on against the target band, resident and
retiring bytes, and stage timings. Live navigation is disabled during recorded runs.

`--visibility-record FILE.csv` records fixed down and horizon views at 5 m and 500 m
clearance over 35 seconds, with four screenshots.

## GPU generation work

The GPU voxel generators and their audits are complete and tested, and the viewer
draws their output as its near field over the height tiles. The
[GPU streaming plan](../../docs/realtime-world-gpu-streaming.md) records what each
stage supplies: density, uniform-chunk meshing, 2:1 transitions with bounded
incremental residency, and filtered height tiles. These audits run on Metal on macOS
and Vulkan on Windows and Linux:

```sh
cargo test -p procgen-gpu-tests --test voxel_density_agreement -- --nocapture
cargo test -p procgen-gpu-tests --test voxel_mesh_agreement -- --nocapture
cargo test -p procgen-gpu-tests --test voxel_streaming_agreement -- --nocapture --test-threads=1
cargo test -p procgen-gpu-tests --test height_mesh_agreement -- --nocapture --test-threads=1
cargo test -p procgen-gpu-tests --test surface_composition -- --nocapture
```

They check real GPU chunk batches, shared halo and parent samples, repeated runs, and CPU
agreement using the saved preset and radius/control extremes. A compatible GPU is
required; shader validation alone can run without one by filtering to
`voxel_density_wgsl_validates_without_a_device`. The meshing audit checks topology, exact
shared boundaries, replay order, capacity failures, and saved-terrain agreement with CPU
extraction, feeding GPU density straight into extraction; the mesher is marching
tetrahedra, and `qef.rs` seeds a dual-contour mesher and has no caller. The streaming
audit adds mixed-LOD seams, publication, slot reuse, cancellation, retirement, overflow,
memory limits, deferred submission, draw leases through retirement, and a fixed route
through the saved terrain. Visual samples, vertices, indices, and draw counts stay on the
GPU. The `gpu` feature owns the wgpu implementation, the inspector enables it, and the
audits use the same encoder as the residency owner.

## Field decisions

The equations in Murray's slides are the reference. Missing parts are completed as
follows for this experiment:

- Use cubic gradient noise divided by its exported conservative bound, so the basis
  and its derivative share one normalization. Each octave is then centered and
  scaled by fixed reference moments; the safety bound is not treated as the useful
  contrast range.
- An explicit octave list replaces a fixed lacunarity. Each band carries its own
  wavelength in meters, amplitude in meters, sharpness, warp, and three damping
  controls. Wavelengths must strictly decrease. One key is shared across octaves.
- Use the unshaped basis derivative in octave coordinates for slope, ridge, and
  warp signals. Ridge accumulation mirrors slope accumulation, scaled by its own
  control. These are pilot choices.
- Apply the slope update before adding the damped contribution, then update
  altitude and ridge damping and the warp offset.
- Nonzero sample spacing smoothly removes wavelengths from four down to two samples
  per wavelength, filtering the feedback signals as well as height, so an unresolved
  octave cannot alter later bands or re-enter through domain warp.
- Divide the accumulated relief by the height limit and apply `x / sqrt(1 + x*x)`
  to bound it without hard clipping and without normalizing away position-dependent
  damping.
- Claim analytical derivatives only for the normalized basis and the sharpness
  transform. The composed surface returns a scalar; its feedback vectors are not
  final gradients.
- Add one bounded 3D term to the voxel density, so the near field can carry cliffs and
  undercuts a radial height cannot express: `d + amplitude * bound(shape(noise(p /
  wavelength), sharpness)) * fade(d)`, with `d = height - altitude` and `p` the position
  in world meters. `fade` is one below the surface, smoothsteps to zero over `fade_m`
  above it, and is zero past that, so nothing the term adds floats higher than `fade_m`.
  `bound` is the design's own `x / sqrt(1 + x*x)`, so the term stays strictly inside
  +/- `amplitude_m` whatever the sharpness, which is what the render bias below relies
  on; [the flight viewer](../../docs/realtime-world-flight-viewer.md) carries the
  measured argument for bounding it. Its key `0x564F_4C55` is separate from the height
  field's `0x5355_5246`, so enabling it cannot move the height surface.
- Apply no spacing filter to that term, unlike the octaves: coincident halo and parent
  samples must stay identical across LODs, which a spacing-dependent term would break,
  seaming chunk levels. Coarse chunks alias it instead; bound its wavelength from below,
  at 4 m and never finer than `radius / 2^18`, so `p / wavelength` stays inside the f32
  range that resolves a noise cell to a sixteenth of its width.
- Push the height surface below the band by the coarsest chunk's cell spacing plus that
  amplitude, because the voxel surface can now sit below the height surface as well as
  above it and would otherwise show through every undercut. The soft bound makes that a
  budget rather than a guess at a tail. Height tiles still evaluate height only, and no
  fragment discard is added: the band's box can contain surface the band does not cover.

The [relief calibration report](../../docs/realtime-world-relief.md) describes the
fixed shape moments; the seed is folded once by the noise crate. See
[physical scale and octree LOD](../../docs/realtime-world-planet-scale.md) for the
voxel address space and density contract.

## Validation and boundaries

```sh
cargo build -p procgen-realtime-pilot --no-default-features
cargo build -p procgen-realtime-pilot --no-default-features --features gpu
cargo test -p procgen-realtime-pilot
cargo clippy -p procgen-realtime-pilot -p procgen-gpu-tests --all-targets --all-features
cargo fmt -p procgen-realtime-pilot -- --check
```

Three feature tiers build independently: none is the CPU library and the headless
audit, `gpu` adds the wgpu generators, and `inspector` adds the windowed viewer.

Tests cover shuffled queries, independent thread schedules, direct and batched
agreement, field envelopes, finite values at control extremes, analytical gradient
checks, invalid inputs, shared cube-face addresses, height-tile stitching, voxel
addressing, band selection and its spacing cap, transition meshes, and quantized
initial fingerprints. Exact float bits are not pinned. Precision across large
distances is open. No planet-scale performance claim follows from these experiments.
