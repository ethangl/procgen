# Procgen

This repository centralizes the user's procedural-generation experiments in a
single Rust workspace. Previous experiments may come from other languages or
engines; port concepts deliberately rather than preserving their original
structure.

The first source project is the C# world-generation pipeline at
`~/w/econ/cli` and `~/w/econ/src/WorldGen`. Treat it as a behavioral reference
and rebuild it incrementally; do not attempt a one-shot translation. See
`docs/world-heightmap.md`.

## Architecture

- Keep `procgen-core` dependency-free and limited to backend-neutral value
  types and deterministic pure primitives. It must not become a miscellaneous
  home for algorithms, execution policy, or framework integrations.
- Build small, cohesive crates under `crates/` and compose them into experiments
  and applications.
- "Atomic" means independently understandable, testable, and reusable—not one
  crate per function or algorithm.
- Keep generation crates data-oriented and independent of rendering, UI,
  engines, and export formats.
- Raster stages on the cube-sphere are GPU pipelines. WGSL compute through
  wgpu is their only implementation; they have no CPU path, and none is
  written or proposed. Keep algorithm and data contracts (configs, buffer
  layouts, constants) separate from dispatch code so callers are not coupled
  to a backend.
- Mesh stages and pure primitives (hashing, noise, mapping) keep CPU
  implementations. Where a CPU implementation and a kernel both exist, the CPU
  result is canonical and serves reproducible export. CPU implementations use
  multithreading whenever the workload benefits from it, while avoiding
  parallel overhead for small jobs.
- Pass seeds or RNG state explicitly. Generation should be reproducible.
- Define and test determinism per backend; do not assume floating-point results
  will be bit-identical across backends. Integer results are.
- Prefer concrete APIs first. Extract shared traits only after multiple real
  consumers demonstrate the same boundary.
- Keep domain-specific composition above general primitives: noise should not
  need to know whether it represents terrain, caves, or moisture.

## Working approach

- Support two primary development environments, and every GPU pipeline must
  run on both:
  - macOS on a MacBook Pro, reaching Metal through wgpu.
  - Windows with an NVIDIA RTX 5070, reaching Vulkan through wgpu.
- CUDA is not a backend. Nothing may require an NVIDIA GPU, CUDA toolkit, or
  Windows host. Reconsider CUDA only for a workload that needs something wgpu
  cannot provide, and record the reason in the design doc.
- Where both a CPU implementation and a kernel exist, callers select the
  backend explicitly. There is no automatic fallback, because a raster
  pipeline has nothing to fall back to.
- Port one experiment end to end, extracting reusable pieces as they become
  evident; avoid designing the entire framework in advance.
- Add dependencies narrowly and avoid coupling foundational crates to heavy
  frameworks.
- Include focused tests for determinism, invariants, edge cases, agreement
  between backends within documented tolerances, and run-to-run and schedule
  invariance for GPU pipelines.
- Keep examples or visual tools as consumers of the core crates, not as places
  where generation logic lives.
- Document seeds and parameters for interesting generated results so they can be
  reproduced.

## Code quality

These rules distill the findings that recurred across the code reviews of the
first forty pipeline stages. Each one was raised independently in several
reviews. Treat them as the default bar for new work.

### Ownership and layering

- Every stage output owns a `validate(&self, mesh)` method that checks its own
  shape invariants. Consumers call `input.validate(mesh)?` and then trust the
  data. Do not re-check lengths, ranges, or finiteness that the producing crate
  already guarantees, and never re-validate mesh topology in a consumer.
- Stage error enums wrap upstream errors via `From` (`StageInputError`,
  `GeologyInputError`, `ClimateOutputError`, planet validation). Do not invent
  a parallel error enum, copy Display strings, or add a per-consumer mismatch
  variant. Use one variant per validation rule and interpolate constants into
  messages instead of hardcoding their values.
- Put a helper where the concept lives, not where it was first needed: mesh
  graph traversal and geometry on `SphereMesh`, value-type helpers and
  deterministic mixers in `procgen-core`, crate-wide machinery in the crate's
  `field.rs`, fixtures in `test_support`. The second copy of anything is the
  signal to extract it.
- Values two layers must agree on (the sea-level datum, slider ranges,
  tolerances, stream ids, radii) are owned by the crate that owns the concept,
  as a constant it exports or as a field on the result that carries it. The
  viewer reads them and never restates them as literals.
- The viewer is a pure consumer. Generation sequencing, derived statistics,
  physics-derived bounds, and world validation belong in the crates or on the
  domain results. The viewer must not depend on internal encodings such as
  half-edge layout or flat index arithmetic.

### Data model

- Store one fact and derive the rest. A field that must always equal another
  field, an index, a length, or a config value echoed back should not exist.
  Do not cache a derived quantity on a domain type that goes stale when
  ownership or state changes; compute it from current state or once where the
  result is assembled.
- Keep mutable state, generation provenance, and transition records in
  separate types. A type must not promise an invariant a later stage cannot
  keep.
- Name the shape. Replace a `bool` threaded through several functions with an
  enum, tuples and positional arrays with a named struct, long parameter lists
  with an `*Inputs` struct, and an `Option` used as a mode flag with an
  explicit branch at the call site. `allow(clippy::too_many_arguments)` marks
  a missing type.
- Parameter types are `*Config` with public fields. If validation must run
  once ahead of a hot loop, keep the public config as the data contract and
  return an opaque validated handle.
- No speculative surface. A config knob, field, method, `Default` impl, or
  public helper with no consumer in the same change is deleted, not kept for a
  planned stage. Diagnostics that exist only for a UI counter are computed from
  the result, not by adding bookkeeping to the hot loop. Rendering preferences
  never live in generation data.

### Control flow and invariants

- A guard that cannot fire misstates the invariant. Delete it, demote it to
  `debug_assert!`, or make the precondition explicit in the type or doc
  comment. Treat silent fallbacks (`.max(0.0)`, `unwrap_or`, a "just in case"
  orientation flip) as bugs that hide whether the algorithm produced the right
  answer.
- Do not bolt a special case into a general dispatch. If a match arm encodes
  policy, extend the table with a named, tunable entry instead.
- Never add production code, rounding steps, or loosened tolerances to make a
  test pass. If a test demands an exact match the algorithm does not
  guarantee, fix the test.
- Pick one failure convention per crate. Mesh queries assert on caller
  contract violations; pipeline stages return typed errors. Do not mix
  `panic!` and `Result` for the same class of misuse.
- Prefer the standard library and existing helpers over hand-rolled versions
  (`BinaryHeap`, `iter::successors`, tuple comparison for lexicographic
  tie-breaks, the crate's `validate_range`). Do not mutate counters inside
  `map` closures.

### Stage structure

- A stage function reads as its pipeline: validate inputs, build a per-cell
  model of time-invariant quantities, run a pure per-cell solve, assemble the
  output. Keep the per-cell solve free of bookkeeping so it can become a
  parallel map. Collect a `Vec` of row structs and project columns once rather
  than threading parallel vectors through an accumulator.
- Precompute what the hot loop does not need to recompute. Do not walk the
  same orbit, graph, or edge list twice to recover data the first pass had.
- Mirror sibling stages in shape: same config, result, error, and validation
  layout. When a change adds a module and leaves the crate half-migrated,
  finish the migration in the same change.
- A raster stage is a sequence of dispatches over cell or border-edge buffers.
  Host code packs configs and sequences dispatches; it holds no per-cell
  logic. Relaxations are frontier-based with indirect dispatch, and no stage
  reads back mid-pipeline. Outputs stay resident and a settings change reruns
  from the first dirty stage.

### Module and crate layout

- `lib.rs` is a facade of module declarations and re-exports. One module per
  stage, tests inline as `#[cfg(test)] mod tests` in the module they cover,
  shared fixtures in `test_support`.
- Do not draw a crate boundary for a single consumer. Start as a module and
  lift it when the second consumer appears. Dependency edges point from domain
  crates toward geometry and core, never the reverse.
- Modules import downward from one shared home, never sideways in both
  directions. Use explicit imports and the narrowest visibility that compiles.
- Split files by domain well before 1000 lines. When adding an entry means
  editing several parallel tables (enum, list, label, width, builder),
  collapse them into one record per entry and derive ordering from declaration
  order with a compile-time assertion.

### Naming

- One word per concept across a crate. `triangle` and `face`, or `seed` and
  `key`, or three names for one field must not coexist.
- Names must not lie: a `divergent` knob that only affects rifts, a `stride`
  that is a ratio, a `multiplier` bounded at one, or an `Input` variant that
  means the opposite of its sibling's.

### Cross-backend determinism

- Where a CPU implementation exists, it is the canonical reference. Settle
  expression order, seed narrowing, and kernel structure before a kernel pins
  them; kernel lines are expensive to change, so compactness pays.
- GPU-only stages are deterministic by construction. Every integer result is
  the fixed point of a lexicographic minimum or a per-cell gather in fixed
  neighbor order, so it is bit-identical run to run, across dispatch
  schedules, and across backends. Integer diagnostics use atomic counters;
  float diagnostics use fixed-order reductions, never float atomics.
- No transcendental function sits on a path that decides an integer. Evaluate
  such functions with a fixed polynomial using add and multiply only,
  identical in Rust and WGSL. Quantize host-computed inputs to a fixed grid
  before upload.
- Avoid 64-bit integers and `f64` in any kernel or any path a kernel mirrors.
  Narrow seeds once on the host through one named public function.
- Pin integer fingerprints exactly, and never pin float bits: libm and codegen
  differ across machines, so a hash over `to_bits()` pins the toolchain rather
  than the algorithm. Where a float agreement test needs a tolerance, use a
  round number with a one-line reason for where it sits, and calibrate it
  against measurements only if it fires.
- GPU pipeline tests assert run-to-run and schedule invariance across
  workgroup and frontier chunk sizes, and structural invariants read back at
  full resolution. Expected values at small resolution may be computed in test
  code; that is scaffolding, not a CPU path.

### Tests and docs

- Test invariants and pinned integer fingerprints, not definitions. An
  assertion that recomputes the function under test with the same code, or
  checks something the type system already guarantees, is noise. Shared
  fixtures go in `test_support`.
- When a fingerprint changes, the commit message says why. Behavior changes,
  default retunes, and visible side effects never ride inside a refactor
  commit.
- Update doc comments, READMEs, and design docs in the same change as the code
  they describe. Record non-obvious decisions in one sentence where the next
  reader will look.

## Current state

The coarse pipeline described in `docs/world-heightmap.md` is in place on a
65,536-cell default mesh: Fibonacci sampling, spherical Delaunay/Voronoi
topology, tectonics through tectonic elevation, whose evolution changes the
plate set as continents rift and suture, geology through isostatic adjustment,
and climate (solar forcing, radiative equilibrium, seasonal thermal response,
circulation, moisture transport, cryosphere, and bounded coupling). Sea level
is a configured datum every elevation field carries, tectonic and geological
alike, so land is elevation above the field's own datum rather than above a
constant and no reader pairs a bare vector with a sea level from elsewhere,
and crust belongs to cells rather than plates: continental nuclei grow to a
target area across plate boundaries, plates have no crust class, and about
nineteen coast edges in twenty lie inside a plate as a passive margin.
Continental margins, the shelf that floods and drains with sea level, are the
next step in that story.
The viewer consumes every stage, caches complete generated worlds keyed on the
generator build identity, and draws a displaced fan mesh with relief and
lighting controls. It runs tectonics, geology, and climate as separately
generated phases: one phase at a time in the sidebar, each generated on its own
over the upstream results already in memory.

Terrain-detail refinement (`docs/terrain-detail-refinement.md`) is in
progress. Slices 1 through 14 have landed: the 32-bit hash, `procgen-noise`
with its WGSL mirror and agreement tests, Delaunay point location, cube-sphere
mapping and tile addressing, and terrain-control composition with CPU
control-face baking cached in generated-world snapshots, plus the canonical CPU
terrain-height function, coastline domain warp, canonical CPU and WGSL tiles,
and complete viewer LOD through level 12 with spacing-derived octave fading,
skirts, relative tile origins, resident GPU slots, and bounded tile-generation
scheduling. Slices 15 and 16, tile export, follow the pilot's evaluation.

The active work is the compute-shader tectonics pilot
(`docs/compute-shader-tectonics-pilot.md`): a GPU-only tectonics pipeline on a
1024-texel-per-face cube-sphere raster in `procgen-raster-tectonics` and
`apps/raster-viewer`, the intended successor to the viewer. Slices 1 through 3
have landed: cube-sphere raster addressing with chamfer links, canonical
border-edge ids, texel solid angles, texel-center directions, and the
polynomial tangent in Rust and WGSL; the plate seed and growth kernels with
their frontier relaxation; crust classification, boundary classification, and
the simultaneous migration step the evolution repeats; and the pilot
application rendering six face grids coloured by plate, crust, or boundary
class at a selectable face resolution, with stage timings and readback
diagnostics. Bathymetry, relief, and interactivity follow. The current viewer
is frozen while the pilot runs. The pilot's evaluation decides whether geology
and climate follow, and the future of the Voronoi path.

The pilot's kernels sit at `wgpu`'s default limit of eight storage buffers per
stage, so new per-cell state consolidates into an existing buffer rather than
adding a binding. Fused multiply-add contraction is enabled on Metal through
`wgpu` and cannot be turned off from there, so integers that float comparisons
decide are expected to agree across backends only within what slice 5
measures.

`procgen-core` owns the WGSL mirror of the four-word hash; `procgen-noise`,
`procgen-terrain`, and `procgen-raster-tectonics` compose it rather than
restating it. `procgen-viewer-support` owns what both applications need to
present a world: the orbit controls and the identity colour ramp.

Crate boundaries and the per-stage conventions in "Code quality" are
established. New stages should follow the sibling shapes rather than introduce
new ones. There is no CUDA backend and none is planned. The WGSL noise and
terrain mirrors are exercised by `procgen-gpu-tests` through wgpu; their
tolerances were set on Metal, and one can be widened if Vulkan trips it.
