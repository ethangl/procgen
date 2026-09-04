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

## Current state

The project is at its initial architecture stage. Do not assume crate names or
shared abstractions are settled until the first few experiments establish them.
