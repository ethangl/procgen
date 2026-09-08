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
- Design parallelizable work for GPU acceleration whenever practical, with CUDA
  as a primary backend. Keep algorithm/data contracts separate from execution
  backends so callers are not coupled to CUDA-specific types.
- Provide a CPU path for unsupported hardware, development, and verification.
  CPU implementations should use multithreading whenever the workload benefits
  from it, while avoiding parallel overhead for small jobs.
- Pass seeds or RNG state explicitly. Generation should be reproducible.
- Define and test determinism per backend; do not assume floating-point results
  will be bit-identical across CPU and GPU implementations.
- Prefer concrete APIs first. Extract shared traits only after multiple real
  consumers demonstrate the same boundary.
- Keep domain-specific composition above general primitives: noise should not
  need to know whether it represents terrain, caves, or moisture.

## Working approach

- Support two primary development environments:
  - macOS on a MacBook Pro, where the multithreaded CPU backend must work.
  - WSL on Windows with an NVIDIA RTX 5070, where CUDA is the primary
    accelerated backend.
- CUDA must remain optional at build and runtime. Stable capabilities must not
  require an NVIDIA GPU, CUDA toolkit, or Windows host.
- Prefer an `auto` execution mode that selects an available accelerated backend
  and otherwise falls back to CPU; also allow explicit backend selection for
  testing and benchmarking.
- Consider cross-platform GPU compute (including Metal-compatible approaches)
  when an algorithm benefits from it, but do not compromise the CUDA or CPU
  implementation merely to force one universal backend.
- Port one experiment end to end, extracting reusable pieces as they become
  evident; avoid designing the entire framework in advance.
- Add dependencies narrowly and avoid coupling foundational crates to heavy
  frameworks.
- Include focused tests for determinism, invariants, edge cases, and agreement
  between compute backends within documented tolerances.
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
- Constants two layers must agree on (sea level, slider ranges, tolerances,
  stream ids, radii) are exported by the crate that owns the concept. The
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

- The CPU implementation is the canonical reference. Settle expression order,
  seed narrowing, and kernel structure before a GPU mirror pins them; every
  kernel line is written again per backend, so compactness pays several times.
- Avoid 64-bit integers and `f64` in any path a WGSL or CUDA kernel must
  mirror. Narrow seeds once on the host through one named public function.
- Test vector tables and agreement tolerances are public constants in the
  library crate, not literals in a test. Record the measured divergence,
  adapter, and date beside each tolerance and set it at ten times the measured
  maximum.

### Tests and docs

- Test invariants and pinned fingerprints, not definitions. An assertion that
  recomputes the function under test with the same code, or checks something
  the type system already guarantees, is noise. Shared fixtures go in
  `test_support`.
- When a fingerprint changes, the commit message says why. Behavior changes,
  default retunes, and visible side effects never ride inside a refactor
  commit.
- Update doc comments, READMEs, and design docs in the same change as the code
  they describe. Record non-obvious decisions in one sentence where the next
  reader will look.

## Current state

The coarse pipeline described in `docs/world-heightmap.md` is in place on a
65,536-cell default mesh: Fibonacci sampling, spherical Delaunay/Voronoi
topology, tectonics through tectonic elevation, geology through isostatic
adjustment, and climate (solar forcing, radiative equilibrium, seasonal thermal
response, circulation, moisture transport, cryosphere, and bounded coupling).
The viewer consumes every stage, caches generated worlds keyed on the generator
build identity, and draws a displaced fan mesh with relief and lighting
controls.

Terrain-detail refinement (`docs/terrain-detail-refinement.md`) is in
progress. Slices 1 through 14 have landed: the 32-bit hash, `procgen-noise`
with its WGSL mirror and agreement tests, Delaunay point location, cube-sphere
mapping and tile addressing, and terrain-control composition with CPU
control-face baking cached in generated-world snapshots, plus the canonical CPU
terrain-height function, coastline domain warp, canonical CPU and WGSL tiles,
and complete viewer LOD through level 12 with spacing-derived octave fading,
skirts, relative tile origins, resident GPU slots, and bounded tile-generation
scheduling. Next is deterministic CPU tile export.

Crate boundaries and the per-stage conventions in "Code quality" are
established. New stages should follow the sibling shapes rather than introduce
new ones. No CUDA backend exists yet; the WGSL noise and terrain mirrors are
exercised by `procgen-gpu-tests` through wgpu, and the cross-backend tolerances
remain provisional until CUDA calibration.
