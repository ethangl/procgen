# Real-time world pilot

Slices 1–3 of [the pilot design](../../docs/realtime-world-pilot.md). This is a
separate application. It uses `procgen-core` and the CPU gradient-noise primitive
from `procgen-noise`, plus cube-face geometry from `procgen-cubesphere`.
It does not use the existing world pipeline or viewer.

## Run

```sh
cargo run -p procgen-realtime-pilot -- --seed 42
```

Select a preset, edit its controls, and press **Generate**. Sampling runs on a
worker and uses parallel columns. The inspector keeps showing the previous
volume until the result arrives. A notice identifies unapplied controls.
Move the three slice sliders to inspect the sampled volume without regenerating.

The four panels show base height, an XY density cut, an XZ density cut, and a
ZY density cut. Brown is solid; blue is air. Light pixels are near zero density.
The height panel excludes caves and 3D detail. Colors saturate outside their
display range; the stored values are unchanged.

The diagnostic contains 64 samples per axis, including both ends of a local
box from -1 to +1. These are **local model lengths**, not planet meters. Y is
up only in this Cartesian experiment. The box is a view into the field and
can crop terrain at extreme settings; it is not the spherical voxel
band described below. Surface queries support X/Z and density queries support XYZ within
plus or minus 8 model lengths. Invalid inputs return typed errors.

Three presets cover rounded hills, ridged mountains, and broad basins. Use
seed 42 for the initial comparison. The full seed and resolved parameters are
printed after each interactive generation.

## Headless captures

The CPU library and capture command can run without the engine dependencies:

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --seed 42 --capture /tmp/procgen-pilot
```

This writes one PPM image and one text record per preset. Panel order is height
XZ, density XY, density XZ, density ZY, starting at the top left. Each text
record includes the seed, grid, resolved config, and sampling time. Captures
are local diagnostics; they do not implement terrain-tile export.

## Field decisions

The equations in Murray's slides are the reference. Missing parts are completed
as follows for this experiment:

- Use cubic gradient noise divided by its exported conservative bound. The
  basis and its derivative therefore share the same normalization.
- Use six octaves, frequency `1 / wavelength`, lacunarity 2, and one key across
  octaves. Begin with unit amplitude and damped amplitude; all sums begin at zero.
- Use the unshaped basis derivative in octave coordinates for slope, ridge,
  and perturbation signals. Ridge accumulation mirrors slope accumulation,
  scaled by its own erosion control. These are pilot choices.
- Apply the slope update before adding the damped contribution. Then update
  altitude amplitude and ridge damping, perturbation, and frequency.
- Store the resolved gain once. The displayed `gain + amplification` is one
  value here; the supplied fragments do not establish separate effects.
- Divide the sum by the undamped geometric amplitude envelope. This gives a
  conservative [-1, 1] bound without normalizing away position-dependent damping.
- Claim analytical derivatives only for the normalized basis and sharpness
  transform. The composed surface and density return scalar values. Their
  feedback vectors are not final gradients.

The surface is the shape function times the height scale. A separate 3D noise
field perturbs the solid boundary. Cave candidates use a 0.5-unit horizontal
grid, a 0.11-unit chamber radius, and a 0.2-unit depth below their own surface
query. Each chamber connects to an inclined capsule that reaches above the
local additive-detail envelope. Only the surrounding nine grid candidates can
affect a column. Cave density is an expected count per square model length.
Chambers can overlap or open through nearby slopes; no global cave network is
promised.

Density magnitude is truncated to 0.125 away from the zero surface. This
preserves the solid boundary and limits cave influence, so omitted distant
candidates cannot change values when a query crosses a candidate-grid boundary.

The seed is folded once by the noise crate. Surface, detail, and caves use
separate named hash domains. A per-column model reuses surface and cave queries
across vertical samples. The direct query path and volume sampler use the same
column evaluator, so cache state and job order cannot change the field.

## Validation and boundaries

```sh
cargo test -p procgen-realtime-pilot --no-default-features
cargo clippy -p procgen-realtime-pilot --all-targets -- -D warnings
cargo fmt -p procgen-realtime-pilot -- --check
```

Tests cover shuffled queries, independent thread schedules, direct/batched
agreement, field envelopes, finite values at control extremes, cave entrances,
agreement with a wider cave search, control effects, analytical gradient checks, invalid inputs, and quantized
initial fingerprints. Exact float bits are not pinned.

Slice 3 adds mixed-detail render meshes and streaming below. Collision, GPU
generation, and precision across large distances remain for later slices. No
planet-scale performance claim follows from these bounded CPU experiments.

## Spherical regions (slice 2)

```sh
cargo run -p procgen-realtime-pilot -- --planet --seed 42
cargo run -p procgen-realtime-pilot --no-default-features -- --planet --check --seed 42
```

The native inspector shows the complete planet. Drag to orbit, scroll to zoom,
and use **Generate** to apply field, sphere, or resolution changes. The previous
mesh remains visible during generation. Switch to **Broad elevation overview**
for the inexpensive height mesh, **Color polygon owners** to inspect the six
face regions, and **Shift render origin** to compare rebased rendering without
changing field queries. The overview and detail mesh use the same broad field;
the overview omits caves and additive density detail.

The initial experiment uses radius 4, height scale 0.5, detail scale 0.04,
rounded-hill noise, and seed 42. Each face has 64 by 64 tangential cells and
16 radial cells. The band extends 0.4 below and 0.3 above broad elevation.
Radial spacing is 0.04375; reference-sphere tangential spacing near a face center
is approximately 0.1. These are model lengths. The default viewport is
1280 by 900 logical pixels. There is one region per face, all resident at the
same resolution. The overview uses 16 by 16 quads per face.

### Field and storage

Broad elevation evaluates the slice-1 shape function at `radius * direction`.
The band radius is `radius + broad_elevation + offset`. Detail evaluates the
3D noise at that planet-centered position. No noise input contains a face ID.
The spherical cave experiment uses candidates from a global 0.5-unit 3D grid:
retain candidates within 0.25 of the reference sphere and project them onto it.
Activation probability is density times 0.5 squared. This is an approximate
surface density, not a promise of a uniform point process.

Cave chambers and vertical shafts follow broad elevation. Their horizontal
metric is reference-sphere chord distance; their vertical metric is band offset.
This is a deliberate spherical adaptation of the local experiment, whose shafts
are inclined. The chamber radius and depth are shared with slice 1. Candidate
projection moves a point by at most 0.25; chamber influence plus density
truncation reaches 0.235. Thus only the surrounding 27 candidate cells can
matter. A wider-search test checks this bound.

Configuration validation requires the lower band to exceed both the additive
detail envelope and the cave depth plus radius, the upper band to exceed the
detail envelope, and the inner radius to remain positive. The upper extent is at most one
reference radius to bound this experiment's query domain. Invalid bands fail;
they are not widened or clipped. The sampled output checks solid lower and
empty upper endpoints as well.

Signed integer cube coordinates identify shared face-grid vertices. All faces
request one canonical sample at a shared edge or corner. Parallel sampling and
cell solves preserve address order. The current sampler assembles the complete
shell; it does not yet generate or evict individual regions. Shared samples
supply the contourer's complete one-cell stencil. Independent region generation
will need the same canonical border cells and polygon-ownership rule.

### Contouring choices and limits

The extractor contours a **trilinear reconstruction of sampled final density**,
not the exact unsampled nonlinear field. Linear interpolation is therefore an
exact, bounded edge root for this reconstruction. Exactly zero samples are
solid. Normals come from the analytical gradient of this reconstructed final
density in normalized cell coordinates, where the QEF is also fitted. They are
not gradients of the broad height field alone.

The QEF uses 12 fixed Jacobi sweeps on the 3 by 3 normal matrix. It discards
eigenvalues at or below 1e-5 of the largest one and selects minimum displacement
from the intersection mass point in unconstrained directions. If the fitted
point is outside the storage cell, the placement rule explicitly uses the mass
point. It does not clamp coordinates. Zero gradients contribute to the mass
point but add no plane constraint. The accepted point maps back to world space
through the cell's trilinear position map. This constrains storage coordinates;
it does not minimize a world-distance QEF on the curved cell.

Each sign-changing grid edge emits one polygon. Cell-face adjacency orders its
ring; the density sign determines winding. Ordinary rings have four cells;
radial edges at cube corners have three. The lowest incident face address owns
the polygon. A triangle fan splits quads. Region color identifies this owner,
so border polygons can extend slightly into the adjacent face.

**One vertex per active cell does not resolve all surface topology.** Seed 42
at the initial resolution has no open edges, unbalanced edge winding, or
zero-area triangles, but has 467 edges with four incident triangles. Those
edges join unresolved sheets. The inspector reports this count. This is not a
manifold mesh suitable for collision. Increasing resolution can reveal smaller
features; it is not a general topology guarantee. Topology-preserving extraction
needs a separate follow-up before any consumer requires manifold output.

The pure sphere fixture verifies closed two-triangle edge incidence and outward
winding at all seams and corners. Other tests cover planar and sharp QEF data,
near-parallel normals, zero gradients, exact-zero samples, out-of-cell fits,
band rejection, direct/sample agreement, and one-worker/four-worker equality.
The render-origin test reconstructs positions within 0.000002 model lengths
for tested offsets up to 16; it does not claim astronomical f32 precision.

Seed 42 produces 417,826 density samples, 53,056 contour vertices, 106,486
triangles, and a 3,072-triangle overview. An optimized development build on the
macOS test host generated these CPU products in about 70–90 ms. This excludes
topology inspection, render upload, and frame time. Metal was used for native visual inspection;
this slice has no generation kernel or Vulkan agreement claim.

## Streaming and detail (slice 3)

```sh
cargo run -p procgen-realtime-pilot -- --stream --seed 42
cargo run -p procgen-realtime-pilot -- --stream --seed 42 \
  --record /tmp/procgen-stream/seed42.csv
```

Drag to look, use WASD and Q/E to fly, and hold Shift for fast flight. Colors
identify mesh detail: blue is coarse, green is medium, and gold is fine.
The recording command runs a 36-second descent, rapid turns, fast flight,
and retreat, then exits. It writes frame data and a summary. Add `--screenshots` to write
one PNG per phase beside the CSV in a separate visual run; image readback
adds frame stalls. Recording begins after initial coarse GPU coverage is
ready. The route has no grounded movement or collision.

This slice keeps the bounded CPU contour source resident and streams **render
meshes**, not density pages. The source uses the slice-2 64-by-64 face grid and
16 radial cells. This is a deliberate scale limit for the six-region pilot;
it is not a solution for storing an arbitrarily large planet. Density data is
released after extraction. The cheap overview covers the planet during source
preparation and remains until all six initial coarse regions are ready.

Coarse and medium meshes cluster tangential cell vertices in blocks of four
and two respectively. Fine meshes retain the original vertices. Clustering
keeps radial layers separate, selects the mean position of the source vertices,
and removes triangles collapsed to fewer than three distinct vertices. A cluster
that reverses or flattens a surviving triangle retains its fine vertices; this
check repeats until neighboring changes preserve orientation.
Every face retains a four-cell fine collar. The collar's sample identities,
positions, and normals are identical at every detail level. Mixed-detail joins
therefore use the same polygons as slice 2. This costs more border geometry
than an adaptive transition mesh, but needs no skirts or overlapping seam
surfaces. Reduction inherits the extractor's nonmanifold limitation.

The renderer-independent scheduler admits two region jobs at most. Its queue
has at most one current request per face. Serial tickets reject late results,
including a request that changes from fine to medium and back to fine.
Workers check cancellation before work, during reduction, and during triangle assembly. A cancelled
worker retains its memory reservation until its result is consumed. Fine meshes
are replaced with coarse meshes and released when the camera leaves; every
face retains resident coverage while its replacement is built.

Uploads are split into at most 1,024 triangles per piece. Each frame admits
at most 512 KiB of conservatively counted upload data and stops admitting pieces
after 2 ms of installation work. One piece can cross that time target, so the
recording includes measured installation time. A region becomes ready only
after every piece appears in Bevy's render-world mesh assets. A 0.15-second
complementary ordered-dither transition then replaces the old region. Its
memory reservation remains until the old render assets disappear and the GPU
queue confirms completion of previously submitted work. Further
replacement of that face waits for retirement.

Managed source, job, staging, resident, and retiring-product reservations are
capped at 128 MiB. Job reservations cover map scratch and worst-case region
output from the actual source triangle count. Render accounting allows both
CPU and GPU copies at 132 bytes per source triangle; indexed chunks normally
use less. Initial fixed-grid source preparation has a separate conservative
1 GiB working reservation and runs once with no concurrent region jobs. These
are application buffer budgets, not a cap on Bevy, the driver, thread stacks,
window buffers, or diagnostic screenshot readback. Process peak RSS is measured
separately in the route report.

The baseline remains radius 4 with about 0.1 tangential sample spacing near
face centers and 0.04375 radial spacing. Manual flight speeds are 0.5 and 4
model lengths per second. The recorded descent moves at 0.7125. The turn phase ends with two seconds of
deliberate 0.12-second revolutions to outrun uploads and test cancellation. Fast flight
circles at radius 5.5 and speed 4. The viewport is 1280 by 900 logical pixels.
Detail uses altitude and viewing direction, with distance hysteresis. Coarse
coverage remains behind the camera. Late detail retains that coarse coverage;
waiting frames and cancelled/rejected work are recorded rather than hidden.

Tests weld mixed-detail face products by their canonical identities and check
for open edges, unbalanced winding, and reversed surviving triangles. They also check readiness of every
upload piece, rejected obsolete results, request reversal, memory admission,
and delayed retirement. GPU visual and timing evidence is recorded separately;
headless scheduler tests do not establish GPU performance.
