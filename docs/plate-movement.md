# Plate movement

## Goal

Make plate motion coherent and make the evolution leave a record. Today the
partition has plate-like outlines, but what happens to them afterwards is
thin: motions are independent random rotations, boundaries only nibble at
convergent edges, and every later stage reads the final boundaries as if the
nine steps had never happened. The changes below keep the rigid-rotation model
and the simultaneous per-step structure the GPU pilot mirrors, and replace the
random inputs and the stateless derivations with ones that have a past.

The measure of success is visual: convergence and divergence organised into
belts that span the sphere, mountain ranges whose width follows how long a
boundary converged, and oceans whose age deepens away from the ridge that made
them.

## Current state

Slices 1, 2, 3, and 4 — coherent kinematics, displacement-proportional
migration, accumulated deformation, and drifting Euler poles — have landed.

- `generate_plate_kinematics(mesh, partition, crust, config)` fits each plate's
  Euler vector to a smooth global flow field over the plate's own cells, then
  scales the fitted direction by the plate's hashed base speed times a crust
  factor times the fourth root of the mean plate area over its own. A
  `coherence` fraction blends the axis back toward the hashed random one.
  Plates too small to fit — one or two cells — keep the hashed axis; the speed
  rule is the same for every plate. Nothing on the path uses a libm call, so
  the integer boundary classes the angular velocities decide stay exact.
- The default flow frequency is 1.0. Averaged over the adjacent plate pairs at
  the viewer's defaults, the signed alignment of their rotation axes is 0.34 to
  0.44 across three motion seeds, against 0.03 or less for the independent
  random motion it replaces. Measured while migration was still a binary gate,
  boundary edges after nine steps were 3978 convergent, 4593 divergent, and
  3922 transform against 4792, 5436, and 4529, and 16010 cells changed owner
  against 25246: coherent motion moved fewer cells. Frequency is the knob that
  matters. Halving it to 0.5 reaches 0.52 to
  0.82 alignment but can halve migration; 1.5 falls back to 0.02 to 0.26,
  no better than random for some seeds, because a flow cell has to be much
  larger than a plate for the fit's Euler axis to agree between neighbours.
  Coherence is not the limit: 1.0 measures within 0.02 of 0.85 everywhere.
- `generate_random_plate_kinematics` is the unchanged hashed generator, kept
  public as the raster pilot's interim source until slice 1's own kernel.
- `classify_boundaries` derives per-edge normal and shear speeds from the two
  owners' rotations and classifies each edge. It is correct and stays.
- Evolution carries five things across steps: ownership, a birth step and an
  accumulated deformation per cell, a closing debt per edge, and a travel debt
  per cell. Debts are distances in model units measured against the mesh's one
  cell width, `sqrt(total_area / cell_count)`. A convergent edge at or above
  `minimum_convergence` adds `convergence * step_duration` each step and moves
  one cell across itself once it has closed a whole cell width, carrying the
  remainder forward; every other edge's debt resets to zero. The retreating
  cell takes the advancing plate's id and everything the advancing cell
  carries, so the overriding plate's material covers it.
  `minimum_convergence` is now the speed below which nothing accumulates
  rather than a binary gate.
- Every cell accumulates `|velocity| * step_duration` and, on reaching a cell
  width, pulls what the same-plate neighbour behind it carries. A cell with no
  neighbour behind it is at the plate's trailing edge: if the boundary there is
  a ridge it is reborn as crust made this step, flat because nothing has
  deformed it yet, and otherwise it keeps what it has. Both updates read the
  fields as they stood before the substep.
- Cell crust is derived from birth and never stored: `Some` is oceanic, `None`
  is original continental crust. `CrustClassification::cell_class` is gone, and
  every consumer reads `PlateEvolution::cell_crust`. Plate classes still
  describe plates and still decide migration precedence and volcanic-arc
  grouping. A rifting continental plate therefore grows an oceanic margin.
- `derive_crust_birth_prior` is the old hop-distance algorithm, now producing
  the birth field evolution starts from: `Some(-hops)` for oceanic cells,
  `Some(-ridge_less_age)` for ridge-less oceanic plates, `None` for continental.
  `derive_seafloor_age` is `step_count - birth`, in steps rather than hops.
- `step_duration` defaults to 0.014, the time a plate at the default maximum
  angular speed of 1.0 takes to cross one cell width on the 65,536-cell default
  mesh. At the viewer's defaults nine steps produce 6473 proposals, 5752
  migration events over 4494 distinct cells, and 1847 crust-creation events,
  against 16010 migration events and no crust creation before. Boundary edges
  after nine steps are 3297 convergent, 3706 divergent, and 3103 transform,
  against 3978, 4593, and 3922: proportional migration moves less and leaves
  smoother boundaries than the binary gate did. Seafloor age spans 1 to 45
  steps with a mean of 15, where the prior alone spanned 0 to 36 hops.
- `BaseElevationConfig::cooling_age` moves from 8 to 40, because the number it
  reads changed unit. It was hops from the final ridge, spanning 0 to 36; it is
  now steps since birth, and every cell that predates the run also ages by the
  whole run, spanning 1 to 45 with a mean of 15. At 8 that put 92.6 percent of
  oceanic cells flat on the deep floor, leaving only crust made during the run
  with any gradient at all. At 40 it is 0.5 percent, and mean oceanic base
  elevation rises from 0.084 to 0.169.
- Deformation is a second per-cell field carried with crust birth. Each step
  computes the profile its own boundaries raise, exactly as the removed
  `derive_boundary_deformation` did over the final ones, scales it by
  `step_duration / full_deformation_time`, adds it to the carried field, and
  clamps to `maximum_magnitude`. The increment comes before migration and
  advection, because uplift happens at the boundary and the material moves
  afterwards. `full_deformation_time` defaults to nine default steps, so the
  viewer's nine-step defaults reproduce the previous magnitudes at a boundary
  that converged the whole run; `maximum_magnitude` defaults to 0.5, the
  largest offset the default profiles can raise, so the clamp does not bite at
  the defaults. At the viewer's defaults the field spans -0.181 to 0.368 with
  a mean of 0.029 over 31,170 affected cells before, and -0.173 to 0.344 with
  a mean of 0.027 over 37,079 affected cells after: nineteen percent more
  cells carry a mark, and the extremes shrink slightly because a boundary
  rarely stays saturated over one cell for a whole run. Land cells move from
  16,872 to 16,808 and tectonic elevation's maximum from 0.900 to 0.934. No
  integer fingerprint downstream moved; the geology pins read synthetic
  elevation fields rather than an evolved one.
- The deformation config moved into `PlateEvolutionConfig`, beside
  `PlateMigrationConfig`, because it is now a substage of a step rather than a
  stage of its own. `PlateEvolution` returns the accumulated
  `BoundaryDeformation`, whose diagnostics sum source-cell events across steps
  and summarize the final field.
- What a step does moved out of `evolution.rs` into `step.rs`, which owns the
  step state and the two fields it carries. `CarriedFields` holds birth and
  deformation together and exposes one `move_onto` that migration and
  advection both call, so the upstream-neighbour search is written once.
- Kinematics is per-step state inside the evolving world rather than a fixed
  input. Every step ends, after deform, migrate, and advect and before the
  next classification, by drifting each plate's rotation vector: the axis
  turns through the fixed angle `axis_drift_rate * step_duration` toward a
  fresh hashed direction perpendicular to it, and the speed is multiplied by
  `1 + s * speed_drift_rate * step_duration` for a hashed `s` in `[-1, 1)`
  and clamped back into the kinematics config's speed bounds. Four
  `signed_f32` draws per plate per step, from a `PLATE_POLE_DRIFT` stream on
  evolution's own new seed, supply the direction and the speed. Because the
  angle per step is fixed and only the direction is hashed, an axis takes a
  random walk on the sphere of directions, whose expected total wander over
  `n` steps is roughly `theta * sqrt(n)`.
- `axis_drift_rate` defaults to 8.0 and `speed_drift_rate` to 3.5, both per
  unit time. At `DEFAULT_STEP_DURATION` the first turns an axis 0.112 radians
  a step, so `0.112 * sqrt(9)` is about twenty degrees of expected wander over
  a default nine-step run, and the second changes a speed by at most about
  five percent a step. Both are modest on purpose: larger drift makes
  boundaries flicker between regimes from step to step and blurs the
  accumulated fields into an average instead of a record.
- `PlateEvolutionInputs.kinematics` is still the initial motion, and
  `PlateEvolution` now returns the motion the run ended on. The viewer's
  `TectonicsWorld` stores that one and does not keep the initial motion at
  all; geology's hotspot trails and the viewer's motion arrows read it.
  Evolution also reads the kinematics config, for the speed bounds the drift
  clamps into.
- The rotation is the tangent half-angle form, which is now
  `Vec3::rotated_toward` in `procgen-core`; the crack walk's private copy is
  gone. Nothing on the drift path calls libm, so the integer boundary classes
  the drifted vectors decide stay exact across machines.
- At the viewer's defaults, boundary edges after nine steps are 3302
  convergent, 3576 divergent, and 3132 transform, against 3297, 3706, and 3103
  without drift: the totals barely move, which is the point — drift changes
  which edges hold which regime rather than how many of each there are.
  Migration events go from 5752 over 4494 distinct cells to 5666 over 4387,
  and crust-creation events from 1847 to 1902. Of the edges that were ever a
  boundary during the run — 21,133 without drift and 20,965 with — those that
  held more than one regime go from 1370 to 5813, a little over four times as
  many. Accumulated deformation spans -0.168 to 0.334 with a mean of 0.029
  over 37,731 affected cells, against -0.173 to 0.344 over 37,079.
- Carried risk: the viewer's small-mesh fixtures each use a seed at which
  climate coupling reaches its fixed point, and every slice that moves terrain
  moves which seeds those are. One changed here, two in slice 3, three in
  slice 2, four in slice 1. The fix
  belongs in the fixture — a mesh coarse enough to be fast but not so coarse
  that coupling is marginal — rather than in each slice's seed list.

## Design

Five changes, in the order they should land. Each is one slice with its own
prompt, review, and merge. Every one is a per-plate reduction or a per-cell
gather over the current step, so the fixed-point argument in the compute
shader pilot keeps holding, and the raster pipeline can mirror each in turn.

### 1. Coherent kinematics from a flow field

A smooth global vector field stands in for mantle convection: three channels of
the fixed-polynomial gradient noise at a low lattice frequency, projected onto
the tangent plane at each cell. A plate's Euler vector is the rigid rotation
that best fits the field over the plate's cells, weighted by cell area:
minimise the sum of `area · |ω × p − v(p)|²`, which is a 3-by-3 linear solve
per plate.

The fitted speed is then scaled by crust and size. Oceanic plates move faster
than continental ones, and large plates slower than small, both by configured
factors, and the result is clamped to the existing minimum and maximum
angular speed. Crust and size are what give neighbours contrast: with a
smooth field alone, adjacent plates move almost together and few boundaries
clear the migration threshold.

A `coherence` fraction blends the fitted rotation with the hashed random one,
so zero reproduces today's motion and one is fully field-driven. The default
is high but not one. The field frequency, the crust and size factors, and
coherence are config; the field's seed derives from the kinematics seed.

The signature changes: kinematics now reads the mesh, the partition, and the
crust classification. The raster pilot keeps calling the random generator
until its own slice adds the per-plate reduction on the GPU; that is the one
place two generators coexist, and it is temporary.

### 2. Displacement-proportional migration and real seafloor age

Migration accumulates a displacement per boundary edge, closing speed times
the step length, and moves a cell only when the accumulated displacement
crosses a cell width, carrying the remainder forward. Fast boundaries migrate
most steps, slow ones rarely, and the threshold becomes a speed below which
nothing accumulates rather than a binary gate.

Divergent boundaries open. When accumulated separation crosses a cell width,
the cells on both sides of the ridge are re-created as new oceanic crust with
the current step as their birth step, the ridge staying between them. Seafloor
age becomes time since birth for cells born during evolution, with cells
present from the start given a configured initial age. `derive_seafloor_age`
reads the birth field instead of walking hops, and base elevation's cooling
curve reads true age.

### 3. Accumulated deformation

Uplift and subsidence are added per step at the boundaries current in that
step, into a per-cell field that persists across steps, with the existing
profiles as the per-step increment scaled by strength and step length. Belts
widen where a boundary converged for many steps, sutures remain where a
boundary used to be, and a boundary that changed regime leaves both marks.
The final-boundary derivation goes away; deformation is the accumulated field.
Smoothing and clamping in tectonic elevation are unchanged.

### 4. Drifting Euler poles

Each plate's rotation vector takes a small deterministic step per evolution
step, a hashed rotation of the axis and a hashed change of speed, both bounded
by config. Regimes then change during the run, which is what gives slices 2
and 3 something to record. Without this, accumulated history is a scaled copy
of the final state.

### 5. Plate lifecycle

A large continental plate rifts along a new crack arc that starts and ends on
its boundary, reusing the crack walk. Two continental plates whose shared
boundary has converged for long enough suture into one. Plate count then
changes during evolution, so plate ids, crust classes, and kinematics have to
be carried through a merge and a split. This is the most involved slice and
the least certain, and it lands last.

## Determinism

Kinematics is a float result and always was; the raster pilot already
quantizes angular velocities to a power-of-two grid before upload. Slice 1
keeps that: the fit uses add, multiply, divide, and square root, and its
output is quantized once on the host by the existing function. Boundary
classes and migration decisions are integers derived from those quantized
floats through comparisons, so they stay bit-identical run to run and across
the two development machines. One libm call remains in reach of the mesh
path's angular velocities, and it predates this slice:
`RandomStream::unit_vector` takes a sine and a cosine, and the hashed axis is
still a fifteen-percent share of every plate's direction at the default
coherence. If a re-pinned integer fingerprint ever splits between the two
machines, that is the first thing to replace, with a normalized triple of
`signed_f32` as the crack walk already uses. Birth steps and accumulated displacement are
integers or exact multiples of the step length. Accumulated deformation is a
float field and is tested for run-to-run equality and invariants only, never
pinned across machines.

## Decisions

- Keep rigid rotations. Plates are rigid to first order, and the Euler-vector
  model gives boundary classification for free.
- Fit to a field rather than simulate forces. Slab pull and ridge push would
  need a mantle model; a smooth field with crust and size factors gets the
  large-scale pattern at a fraction of the complexity.
- Keep evolution as simultaneous steps. Everything here is a Jacobi update,
  which is what keeps the GPU mirror well-defined.
- Birth step, not age, is the stored fact; age is derived from the current
  step. Store one fact and derive the rest.
- The mesh path is canonical. The raster pilot follows each slice with its own
  kernel, and the raster viewer keeps working on the previous slice until it
  does.

## Non-goals

- No mantle convection model, no force balance, no plate-boundary forces.
- No continuous advection of cell contents. Cells change owner or are re-born;
  they do not move.
- No history retained per step in the output beyond the accumulated fields.
  The viewer shows the end state and aggregate diagnostics, as today.
- No change to the partition stage.

## Slices

### Coherent kinematics. Landed.

Flow field, per-plate fit, crust and size factors, coherence blend, config
and viewer controls, docs. Kinematics signature takes the mesh, partition, and
crust. Raster pilot untouched.

### Displacement migration and birth steps. Landed.

Per-edge accumulated displacement, divergent opening, per-cell birth step,
seafloor age from birth, base elevation reading true age.

### Accumulated deformation. Landed.

Per-step deformation increments into a persistent field; the final-boundary
derivation removed.

### Drifting poles. Landed.

Bounded per-step change of each plate's rotation vector.

### Lifecycle.

Rifting along new cracks and suturing of converged continental pairs.
