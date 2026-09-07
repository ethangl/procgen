# Terrain detail refinement

## Goal

Produce deterministic elevation below the coarse mesh, from roughly 88 km per
cell down to the nominal 32 m product in `docs/world-heightmap.md`, without
ever materializing a global fine raster. The same height function must serve
two consumers: the viewer, which needs detail on demand as the camera moves,
and export, which needs tiles reproducible on any backend. Nothing in this
stage changes a generation field; fine detail is a refinement layered on the
authoritative coarse result.

The C# reference added detail after equirectangular rasterization with wrapped
blur, global and coastal detail, and sharpening passes. This design replaces
that raster stage with an on-demand function on the sphere, consistent with the
porting guidance not to preserve the reference decomposition.

## Current state

The viewer draws one triangle fan per Voronoi cell and radially displaces fan
centers from sea level by final adjusted elevation, with shared corner heights
averaged across their three incident cells. Relief exaggeration and lighting
are display controls. The orbit camera stops at 1.25 sphere radii, about
1,600 km above the surface at Earth scale, so nothing yet consumes detail
below the mesh. Generated worlds are cached as snapshots keyed on generator
build identity, so an expensive coarse run is paid once per build and settings.

There is no noise primitive, no sphere projection, and no tiling anywhere in
the workspace. Those are the pieces this stage adds.

## Two kinds of data

Everything below the mesh falls into one of two categories, and the category
decides where it lives.

Local data is a pure function of a unit direction and a seed. Noise, control
lookups, and stamps with known centers are local. Local data is computed where
it is needed and never stored; a tile is a cache of function values, not a
product.

Non-local data depends on neighbors or on a global solve. Hydraulic erosion,
drainage, river networks, and sediment transport are non-local. Non-local data
is computed once at a fixed resolution and stored. This stage introduces none
of it; when it arrives, the nominal 512 m base product is its home, and its
outputs re-enter the local function as stamps and control fields.

The coarse mesh remains authoritative for everything discretized. Land versus
ocean for climate, plate ownership, basin membership, and which tile a sample
belongs to are decided on the mesh or on integer tile coordinates, never from
noise.

## Control bake

The mesh is a Voronoi partition, so per-cell fields are piecewise constant and
would render as 88 km terraces. The detail function needs smooth, cheap,
seamless access to those fields from a shader. The bake provides it.

Once per generated world, project the control fields onto an equi-angular
cube-sphere with six square faces. For each texel, a `procgen-sphere-mesh`
point-location query finds the Delaunay triangle containing its direction, and
the three incident cell values are interpolated barycentrically. This yields a
continuous field rather than nearest-cell steps. An optional blur of about one
cell radius removes the remaining creases at triangle edges. Sampling then
uses hardware seamless cube filtering on the GPU and an identical bilinear
filter on the CPU.

Baked channels are the final adjusted elevation plus derived detail controls:

| Channel           | Source                            | Role in the detail function |
| ----------------- | --------------------------------- | --------------------------- |
| Base elevation    | Adjusted elevation                | Additive base               |
| Amplitude         | Cratons, arcs, boundaries, basins | Scales all octaves          |
| Ridge weight      | Convergent boundaries, arcs       | Blends fbm toward ridged    |
| Roughness         | Craton strength, basins           | Gain per octave             |
| Abyssal amplitude | Seafloor age                      | Ocean-floor hill height     |
| Stamp inputs      | Hotspots, volcanic peaks          | Cone centers and strengths  |

The composition of raw fields into controls lives in `procgen-terrain` with
its own tests; the bake is a projection of its output performed by
`procgen-cubesphere`. Both are deterministic given the snapshot and are cached
alongside it.

Face resolution is derived from the mesh, not fixed: the smallest power of two
that gives at least four texels per mesh cell along a face edge. At 65,536
cells that is 512 texels per face edge, about 20 km per texel, and an RGBA16
face set is about 13 MB. At 16,384 cells it drops to 256, about 39 km and
3 MB.

## Detail function

Height at a unit direction `d`:

```text
h(d) = base(d) + amplitude(d) * noise(d; ridge(d), roughness(d)) + stamps(d)
```

Noise is evaluated in three dimensions on `d` itself, never in face
coordinates, so faces are seamless by construction and no face-boundary logic
exists in the function. Octave zero has a wavelength of about two mesh cells
so the noise begins where the mesh stops carrying information. With lacunarity
2, eleven octaves reach a 76 m wavelength, twice the finest vertex spacing.

The first basis is gradient noise on the cubic lattice with a quintic fade and
analytic derivatives. It has the fewest float operations before the lattice
floor, so the fewest points where backends can diverge, and it is the easiest
to mirror across Rust, WGSL, and CUDA. It ships as a plain function; a shared
basis interface is extracted only if a second implementation proves the
boundary. Its grid artifacts are mostly hidden by the ridge and roughness
controls. Derivatives give tile normals without finite differences and enable
derivative-damped accumulation, which suppresses high octaves on steep slopes
and reads as erosion rather than static. Ridged multifractal handles mountain
belts. Plain fbm, ridged, and derivative-damped accumulation ship together,
because the baked ridge and roughness channels presuppose them.

Coastlines are the one place additive noise misbehaves: it fragments the shore
into lakes and islets and makes the fine ocean mask disagree with the mask
climate used. Near sea level, warp the sampling direction instead of adding
height, and taper amplitude toward zero at the coast. The coarse ocean mask
stays authoritative; the fine coastline deviates by a bounded distance. Domain
warping is deferred to the first tile slice, when coastlines first become
visible below the mesh.

## Determinism

Cross-backend agreement follows the workspace rule: integer results are
bit-exact, float results agree within a documented tolerance. The CPU
implementation is the canonical result. With integer hashing, no
transcendental functions, and Rust's default of no FMA contraction, it is
bit-exact across x86 and ARM, so an export that must be reproducible anywhere
runs on the CPU. GPU paths exist for speed and must agree with the CPU within
named value and derivative-angle tolerances.

- Lattice hashing is integer-only. Workspace-facing noise APIs retain the
  existing `u64` seed convention, while WGSL has no 64-bit integers.
  `procgen-noise` therefore owns one named conversion that hashes both 32-bit
  halves of the seed into a `u32` lattice key. This is deliberately a
  many-to-one fold: all seed bits influence the result, but noise has a 32-bit
  field-key namespace and distinct `u64` seeds are not guaranteed to select
  distinct fields. `procgen-noise` folds the seed once on the host; WGSL and
  CUDA receive that key rather than reimplementing or repeating the conversion.
  The lattice wrapper combines the key with three `i32` coordinates,
  reinterpreting each signed coordinate's bits as `u32`. Backend tests reproduce
  the core hash vectors and selected lattice gradients bit-exactly in a compute
  dispatch.
- The float path uses add, multiply, floor, and lerp only. No transcendental
  functions inside noise, no fast-math flags, and FMA contraction either
  disabled or applied identically on every backend.
- Every backend derives the sample direction from integer face, level, and
  tile coordinates in f32 using the same expression order, so coordinate
  quantization is shared rather than a source of disagreement.
- The tolerances are set at ten times the measured maximum divergence over a
  large sample on both Metal and CUDA, and the measurements are recorded beside
  the constants. The provisional value tolerance is an absolute height
  difference of 1e-5 in normalized units, about 10 cm at the Earth preset,
  with normals within 1e-3 radians. Expected drift is dominated by the finest
  octave's sensitivity to direction rounding and should land well under that.

Two invariants get tests independent of backend:

- Height at a direction is identical whichever tile or level computed it,
  except for the octaves the coarser level dropped.
- Adjacent tiles at the same level produce bit-identical shared edge vertices,
  so cracks can only come from level mismatch.

## Level of detail

Each cube face carries a quadtree. A node is a tile of 65 by 65 vertices, 64
quads per side, plus a one-quad skirt. Vertex spacing at Earth radius, nominal
at face center:

| Level | Vertex spacing | Reference                  |
| ----- | -------------- | -------------------------- |
| 0     | 156 km         |                            |
| 1     | 78 km          | Current mesh cell          |
| 4     | 9.8 km         |                            |
| 8     | 611 m          | Nominal 512 m base product |
| 12    | 38 m           | Nominal 32 m product       |

The octave cutoff is the level of detail. A tile evaluates only octaves whose
wavelength is at least twice its vertex spacing, and the last octave fades in
with the split factor so refinement does not pop. Skirts hide cracks between
neighboring levels; index stitching can replace them later. Nodes split and
merge on projected size with a per-frame budget for new tiles. Tile positions
are stored relative to the tile origin so f32 precision holds at close range.

The whole planet at level 12 is about 100 million tiles and 4e11 samples. It
is never materialized; a view holds on the order of a thousand tiles. One tile
at eleven octaves is about 46,000 basis evaluations, microseconds on a GPU and
a few milliseconds per CPU core. A thousand resident tiles with height and
packed normal occupy 35 to 70 MB.

## Execution backends

The viewer generates tiles with a WGSL compute shader through Bevy's wgpu
renderer, which reaches Metal on macOS and Vulkan on Windows. CUDA is not used
on this path; interop between CUDA and wgpu is not worth its complexity for
work this cheap. CUDA remains the primary accelerated backend for offline tile
export, alongside the mandatory multithreaded CPU implementation used for
verification and unsupported hardware.

The noise kernel is small enough to hand-mirror in Rust, WGSL, and CUDA.
Shared test vectors, not shared source, are what guarantee agreement.
Single-source compilation from Rust to SPIR-V and PTX is worth evaluating
later, not a dependency of this stage.

## Storage

Noise is never written to disk. The control bake is stored with the world
snapshot and invalidated by the same build identity. Later non-local products
at the 512 m level will need tiled storage of their own; that is out of scope
here.

## Crate layout

- `procgen-core` gains a generic four-word 32-bit hash beside `RandomStream`.
  It is a pure primitive.
- `procgen-noise` is new: basis functions, fbm variants, the CPU
  implementation, the signed lattice-coordinate wrapper, and the WGSL source
  as a checked-in asset. It carries no domain knowledge.
- `procgen-gpu-tests` is a test-only integration crate added with the first GPU
  mirror. It owns wgpu dispatch agreement tests and has no production API.
- `procgen-sphere-mesh` gains Delaunay point location. It is a mesh query.
- `procgen-cubesphere` is new: the equi-angular mapping, tile addressing, and
  rasterization of mesh fields into faces. The heightmap plan already names
  projection as its own boundary, and both the viewer and export consume it.
- `procgen-terrain` is new: control composition and the height function that
  consumes the controls. They are one contract, and this is the layer where
  noise becomes terrain.

The viewer consumes these crates and owns no generation logic. GPU agreement
also stays outside the viewer, so application structure does not own generation
correctness.

## Decisions

Settled before implementation:

- Tiles are 65 by 65 vertices, 64 quads per side, with a one-quad skirt.
- Control-face resolution is derived from the mesh at four texels per cell,
  not fixed.
- The first basis is cubic-lattice gradient noise with analytic derivatives.
  The basis ships in slice 2; plain fbm, ridged, and derivative-damped variants
  ship in slice 3; domain warp ships in slice 10.
- The CPU path is canonical. GPU agreement uses named value and
  derivative-angle tolerances set at ten times measured divergence,
  provisionally 1e-5 normalized and 1e-3 radians.
- `procgen-core` and `procgen-sphere-mesh` gain primitives; `procgen-noise`,
  `procgen-cubesphere`, and `procgen-terrain` are new.

## Non-goals

This stage adds no erosion, hydrology, rivers, sediment, biomes, surface
materials or texturing, vegetation, atmospheric scattering, equirectangular
export, or the 32 m export product itself. It does not modify any generation
field, does not feed back into climate or geology, and does not introduce CUDA
into the viewer.

## Slices

### `procgen-core` hash and `procgen-noise`: gradient basis with derivatives, fbm, ridged, and derivative-damped accumulation, CPU implementation, WGSL source, and agreement tests.

1. ~~Add the generic four-word 32-bit hash and provisional test vectors to procgen-core.~~
2. ~~Create procgen-noise with the CPU gradient basis and analytic derivatives only.~~
3. ~~Add CPU fbm, ridged, and derivative-damped accumulation.~~
4. ~~Add the WGSL mirror and a procgen-gpu-tests agreement dispatch—without viewer integration.~~

### `procgen-sphere-mesh` point location, `procgen-cubesphere` mapping and bake, and `procgen-terrain` control composition, cached with the snapshot.

5. ~~Add Delaunay point location to procgen-sphere-mesh.~~
6. Add cube-sphere mapping, integer tile addressing, and seam tests to procgen-cubesphere.
7. Add terrain-control composition to procgen-terrain.
8. Add CPU control-face baking and deterministic bilinear sampling.
9. Cache the control bake with generated-world snapshots.

### Single-level tile generation in the viewer at a fixed level, replacing the fan mesh below a zoom threshold, with coastline domain warping.

10. Add the backend-neutral height function and coastline domain warp.
11. Add fixed-level CPU tile generation as the canonical reference.
12. Add fixed-level WGSL viewer tiles below a zoom threshold.

### Quadtree split and merge, skirts, octave fading, and a closer camera limit.

13. Add quadtree selection and bounded tile-generation scheduling.
14. Add skirts, relative tile origins, octave fading, and closer camera behavior.

### CPU and CUDA tile export sharing the viewer's function, with the tolerances measured and recorded.

15. Add deterministic CPU tile export.
16. Add CUDA export and measure the final cross-backend tolerance.

Each slice should explicitly exclude later consumers and backends.
