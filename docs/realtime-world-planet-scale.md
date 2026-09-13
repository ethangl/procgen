# Real-time pilot: physical scale and octree LOD

Status: slice 1 implements physical broad terrain, an octave editor, and bounded
CPU height previews. Slice 2a adds octree addresses and canonical density chunks
with one-meter finest spacing. Camera-driven residency, voxel meshing, and
meter-scale collision remain in the following parts of slice 2. This phase belongs only to the independent real-time pilot.

## Requirements and source evidence

- Give planet radius, elevation, feature wavelength, and local detail independent
  physical dimensions. All new generation lengths are meters; the UI may display
  kilometers. Increasing radius must not also enlarge mountains or the player.
- Target nominal one-meter voxel spacing at the nearest LOD, with coarser distant
  chunks. McKendrick 21:53–22:13 explicitly describes one-meter-cubed voxels and
  32-meter regions (the transcription labels one sentence `21:60`). At
  18:20–18:41 he notes the cube/sphere mapping changes voxel density, so this is
  not a claim that mapped cells are perfect one-meter cubes everywhere.
- Use an octree for chunk subdivision and residency. McKendrick 24:14–25:58
  describes doubled region sizes, reduced distant density, and octree storage.
  At 26:34 onward he also describes cheaper non-voxel distant terrain. We cannot
  assume that six voxel LODs alone can cover a planet at the new scale.
- Separate broad elevation from the local density band. McKendrick 20:26–21:18
  describes a roughly 128-meter band moved by a broad elevation offset. Those
  dimensions are evidence, not an instruction to scale all current constants.
- Begin the broad terrain stack at continental wavelengths, then add finer
  bands. Expose each band's amplitude and shape for user tuning. This is our
  explicit composition of Murray's concepts, not recovered source-game code.

The old radius-4 pilot remains an independent comparison. Its nominal source
spacing would be about 100 meters tangentially and 44 meters radially if assigned
one kilometer per model unit. Its render refinement is not one-meter sampling.

## Slice 1: physical dimensions and octave editor

Run `cargo run -p procgen-realtime-pilot -- --design`. The default editing preset
has seed 42, a 2,000-kilometer reference radius, and 18 bands from 1,048.576 km
to 8 m, halving wavelength each time. The first amplitude is 4,000 m; later
amplitudes decrease by 0.55. The smooth absolute height bound is 12,000 m.
These are provisional tuning values, not accepted visual defaults or an Earth
model. Zero elevation is the reference sphere; there is no water simulation.

`PlanetDesignConfig` owns meter-based dimensions and ordered octave records.
It validates to an immutable CPU field. Each record contains enable state,
wavelength, amplitude, sharpness, warp, slope damping, altitude damping, and ridge
damping. Wavelengths must strictly decrease. The supported editing envelope is
radius 100–8,000 km, wavelength 8 m–16,000 km, and 1–24 octave records.

The field reuses the normalized gradient basis and calibrated sharpness transform
from the relief correction. Unlike the old six-band composition, amplitudes are
explicit meter coefficients; there is no gain or octave-count normalization.
In broad-to-fine order, accumulate unshaped basis derivatives for slope, ridge,
and warp feedback. Altitude damping reads the preceding sum divided by the
configured height bound. Apply each band's amplitude and damping to its shaped
value. Finally bound the total as `h / sqrt(1 + (h / height_limit_m)^2)`.
Amplitude is therefore a coefficient before feedback and bounding, not a promise
that a peak will equal that value. Changing one band can affect later bands
through feedback. Disabled and zero-amplitude bands have no feedback effect.

The viewer has two bounded height previews:

- **Planet:** a welded cube-sphere grid, with physical radius and elevation.
- **Local patch:** a curved tangent patch at editable latitude/longitude, spanning
  128 m to 100 km. Coordinates are relative to its center surface. Curvature is
  evaluated without subtracting two large planet radii.

Both use power-of-two grids, up to 256 quads per side. Geometry is uniformly
scaled for display; elevation is never exaggerated. The grid is a diagnostic
height mesh, not a voxel volume. Globe sample spacing uses a conservative
reference-sphere estimate; patch spacing is nominal tangent spacing. Neither is
an exact bound on distance along steep terrain or on warped noise bandwidth.

Unresolved noise fades smoothly between four and two samples per wavelength.
The fade multiplies height and derivative feedback; fully filtered bands cost no
noise query. The editor reports each band's applied preview weight. A globe
cannot show meter-scale detail: switch to a small patch to inspect those bands.
The full-resolution elevation percentiles are sampled separately over 65,536
approximately equal-area directions, so they describe the selected field over
the planet, not the visible patch or filtered mesh extrema.

**Solo** evaluates a band alone, including a disabled band selected explicitly;
it does not isolate its marginal effect within the interacting stack. **Combined
terrain** restores the complete stack. Auto apply waits 350 ms after edits and
permits one worker at a time. Changes during a job coalesce into the next request.
The previous preview remains visible while the existing status line reports progress.
With auto apply off, that line reports pending edits. Editable controls use stable
IDs so status and validation changes cannot interrupt text entry.

**Save controls**, **Load**, and **Copy JSON** exchange a strict version-1 file
containing seed, radius, height bound, and every octave value. Save writes the
editable controls, including edits not yet displayed. Camera, solo selection,
patch position, preview resolution, and coloring are viewer preferences and are
not saved as generation data. Loading validates the entire design. With auto
apply off, press Generate to display it.

Headless examples:

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --check --write-design /tmp/planet-design.json
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --design-file /tmp/planet-design.json --check --patch-span 128 --solo 17
cargo run -p procgen-realtime-pilot -- \
  --design --design-file /tmp/planet-design.json
```

The last band has an 8-meter wavelength, rather than a one-meter wavelength:
a one-meter sampling target does not imply one-meter features can be resolved.
The preview uses the existing f32 CPU noise primitive in its bounded editing
domain. Slice 2a adds integer meter addressing and a stable radial-altitude
calculation for density samples. The noise primitive remains unchanged; its
precision and any future GPU mirror still need agreement checks before ground
collision depends on them.

## Slice 2: bounded octree density generation

Replace whole-shell residency with addressed chunks. Start with nominal 32³-meter
nearest regions and one-meter finest voxel spacing. Define power-of-two spatial
addressing, shared samples and halos, world/chunk transforms, and a local origin
representation before implementing traversal. Test shared-face, edge, corner,
and parent/child agreement, including at the maximum supported planet radius.

Then implement a bounded working set driven by the camera, local density-band
generation over broad elevation, cancellation, eviction, and coarse/fine mesh
boundaries. Keep near collision tied to the canonical density/surface, independent
of render replacement. Settle the CPU expression order and coordinate contract
before adding any WGSL mirror. Do not retain a planet-wide fine source or use
radius scaling as a substitute for addressing.

Acceptance: nearest active samples meet the meter-scale contract; distant chunks
coarsen; budgeted residency stays bounded during travel; revisiting an evicted
location recreates the same field and surface; shared boundaries remain valid.
CPU remains canonical. GPU work, when introduced, must support Metal and Vulkan.

### Slice 2a: integer addresses and density chunks (implemented)

Voxel storage now uses one planet-centered **Cartesian octree**, rather than the
six surface-face charts used by the coarse preview. This is a deliberate pilot
choice: regular world-space cells give exact meter edges, share face/edge/corner
samples without chart transforms, and keep storage independent of terrain shape.
The terrain function still evaluates a sphere. Interior/empty octree nodes will
be culled by the working-set stage; the root does not allocate its descendants.
The cube-sphere mapping remains useful for the cheap height overview.

The root spans 16,777,216 meters, centered on the origin. It encloses the current
maximum radius (8,000 km) and relief bound. Root LOD is 19; LOD zero is finest.
Each subdivision produces eight children with half the physical span and spacing.
Every chunk has 32 cells per axis. Thus LOD 0 spans 32 m with 1 m spacing, LOD 1
spans 64 m with 2 m spacing, and so on. Chunk indices are unsigned integers from
the root minimum; world sample positions are signed integer meters. Half-open
chunk ownership handles negative coordinates and zero without ambiguous cells.

Each volume includes 33 endpoint samples plus one halo sample on each side:
35³ = 42,875 f32 densities, or **171,500 bytes of density payload**. Halo samples
are addressed before any float conversion. Neighboring chunks and parent/child
chunks therefore query the same world position at coincident nodes. Relative
positions subtract integer origins before conversion to render coordinates.
Integer addressing has a pinned fingerprint across LODs and negative positions.

`sample_voxel_chunk` is a bounded, parallel CPU job. It samples the existing saved
design's **complete** octave stack, independent of chunk LOD. Density is broad
surface elevation minus radial altitude, clamped to +/-4 m, positive for solid.
It is not a signed-distance function. The sampler can skip noise outside the
configured global height envelope, where the clamped sign is already known.
This establishes height-derived density; local caves and overhangs are not yet
part of the physical design contract.

Radial altitude uses `(dot(p,p) - radius²) / (length(p) + radius)`, with compensated
f32 products and sums in the numerator. This avoids cancellation between rounded
planet radii. Tests compare it with an independent f64 oracle at axis, oblique,
and corner directions, radii through 8,000 km, and +/-20 km offsets, within 2 mm.
There is no f64 or 64-bit integer requirement in the sampling implementation.
The existing f32 noise function and its precision limits remain unchanged.

Full-stack density keeps identical values at coincident samples across LODs.
The preview's octave fading is not silently applied to collision/source density.
Filtering for rendered coarse meshes and transitions must be resolved explicitly
when those meshes are implemented.

Run a bounded headless audit with a saved design:

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --design-file /tmp/planet-design.json --check-chunks
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --check-chunks --chunk-lod 1 --chunk-point 2000197,0,0
```

Without a point, the command chooses the +X surface and quantizes that probe to
integer meters once on the host. Explicit points use integer `x,y,z` meters.
The JSON reports the root, selected address and children, sample spacing, payload
size, density range, and neighboring/parent agreement. It compares 3,675 face and
halo samples with an X neighbor and 4,913 coincident samples with the parent.
At most two density payloads remain live: the neighbor is released before the
parent is sampled. The 343,000-byte paired payload bound excludes the shared
field, thread pool, allocator overhead, and process memory. The root has neither
a neighbor nor parent, so those comparison counts are zero.

Validation: all 56 tests pass (53 library, three headless binary), including eight
new address/density/audit tests. Both Clippy configurations, build, and formatting
checks pass. Headless audits cover the default saved preset, radii 100 km and
8,000 km, seeds 0/42/u64::MAX, negative/oblique positions, LODs 0/1/8/19, and
invalid inputs. Every checked coincident density matches exactly on the CPU;
the largest paired density payload is 343,000 bytes. No GPU density mirror or
new rendered mesh is claimed by these checks.

### Slice 2b: camera-driven residency (next)

Select octree leaves around the camera, retain cheap distant coverage, and cull
interior/empty regions conservatively. Add bounded scheduling, cancellation,
eviction, and stale-result rejection. Verify repeated travel and different job
orders against the canonical chunk addresses and density samples from 2a.

### Slice 2c: voxel meshes and nearby collision

Extract chunk surfaces, settle coarse/fine transitions and density filtering,
and connect canonical nearby collision. Validate shared boundaries before
replacing the fixed source used by the earlier streaming experiment. The final
acceptance criteria for slice 2 still apply; 2a alone does not satisfy them.

## Slice 3: integrated planet-to-ground viewer

Connect the editable design to octree terrain, nearby collision, and cheap distant
coverage. Give the player and camera physical dimensions and speeds. Show meters
or kilometers for altitude and distinguish reference altitude from ground clearance.
Filter geometric and material detail to their actual resolution. Preserve the
neutral inspection views and saved-preset workflow.

Acceptance: continuous orbit-to-ground travel, stable walking, no exposed LOD
seams, deterministic revisits, and measured memory, upload, generation, and frame
budgets on macOS/Metal and Windows/Vulkan. This does not add climate, tectonics,
biomes, or dependencies on the older world pipeline.

## Slice 1 validation

The 45 library tests and three binary tests pass with inspector features disabled.
Coverage includes physical scale invariance, independent octave controls,
filtered/disabled feedback, bounded extremes, both halves of the seed, schedule
invariance, a closed globe, local curvature, and versioned preset round trips.
The original pilot's field fingerprints and terrain/streaming tests still pass.

Headless seed-42 checks generated the 196,608-triangle globe and a
32,768-triangle, 128-meter patch. The globe's reference spacing is 31.25 km;
bands 0–3 are fully present, band 4 nearly faded out, and finer bands absent.
The patch at 128 quads has nominal one-meter height-sample spacing, and solo
band 17 (8-meter wavelength) is fully present. This confirms preview filtering,
not completion of the voxel sampling or collision contract.

Native macOS/Metal inspection exercised planet and patch views, per-band disable,
automatic regeneration, Save, and Load. The scene has its own viewport so the
sidebar does not cover terrain. Clipboard support is enabled through the existing
eGUI integration for numeric/path paste and JSON export. Windows/Vulkan visual
validation remains open.
