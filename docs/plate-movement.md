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

Slices 1 and 2, coherent kinematics and displacement-proportional migration,
have landed.

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
- Evolution carries four things across steps: ownership, a birth step per cell,
  a closing debt per edge, and a travel debt per cell. Debts are distances in
  model units measured against the mesh's one cell width,
  `sqrt(total_area / cell_count)`. A convergent edge at or above
  `minimum_convergence` adds `convergence * step_duration` each step and moves
  one cell across itself once it has closed a whole cell width, carrying the
  remainder forward; every other edge's debt resets to zero. The retreating
  cell takes the advancing plate's id and the advancing cell's birth, so the
  overriding plate's material covers it. `minimum_convergence` is now the speed
  below which nothing accumulates rather than a binary gate.
- Every cell accumulates `|velocity| * step_duration` and, on reaching a cell
  width, pulls the birth of the same-plate neighbour behind it. A cell with no
  neighbour behind it is at the plate's trailing edge: if the boundary there is
  a ridge it is reborn as crust made this step, and otherwise it keeps what it
  has. Both updates read the field as it stood before the substep.
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
- `derive_boundary_deformation` is still a profile around the final boundary
  scaled by final strength, and still sees no earlier step. Slice 3 replaces it.
- Carried risk: the viewer's 32-cell cache fixtures each use a seed at which
  climate coupling reaches its fixed point, and every slice that moves terrain
  moves which seeds those are. Three changed here, four in slice 1. The fix
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

### Accumulated deformation.

Per-step deformation increments into a persistent field; the final-boundary
derivation removed.

### Drifting poles.

Bounded per-step change of each plate's rotation vector.

### Lifecycle.

Rifting along new cracks and suturing of converged continental pairs.
