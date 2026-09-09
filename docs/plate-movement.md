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

Slice 1, coherent kinematics, has landed.

- `generate_plate_kinematics(mesh, partition, crust, config)` fits each plate's
  Euler vector to a smooth global flow field over the plate's own cells, scales
  the fitted direction by a speed derived from the plate's hashed base speed,
  its crust class, and its area, and blends the axis back toward the hashed
  random one by the configured `coherence`. Plates too small to fit — one or
  two cells — keep the hashed random rotation. At the viewer's defaults the
  fifty-seven plates give 3923 convergent, 4064 divergent, and 3655 transform
  edges after nine steps, against 4792, 5436, and 4529 for the old independent
  motion; coherent motion moves fewer cells across boundaries, 16120 against
  25246.

  The default flow frequency of 1.5 does not yet organise those edges into
  belts. Averaged over the adjacent plate pairs, the signed alignment of their
  rotation axes is 0.02 to 0.26 across three motion seeds, against 0.03 for
  independent motion — no better than random for the shipped seed. The fit's
  Euler axis is sensitive to where in a flow cell a plate sits, so a cell has
  to be much larger than a plate. Frequency 1.0 measures 0.34 to 0.46 and 0.5
  measures 0.52 to 0.85, both with migration still in the fifteen-thousand
  range. Retuning the default down is the obvious next step and needs no code.
  Coherence itself is not the limit: 1.0 measures within 0.02 of 0.85
  everywhere, because a fifteen-percent random admixture only tilts the axis.

  `generate_random_plate_kinematics` is the unchanged hashed generator, kept
  public as the raster pilot's interim source until slice 1's own kernel.
- `classify_boundaries` derives per-edge normal and shear speeds from the two
  owners' rotations and classifies each edge. It is correct and stays.
- `migrate_plates_once` moves one cell per convergent edge whose closing speed
  exceeds a threshold; continental overrides oceanic, then the faster side
  wins. Divergent edges do nothing. Over the default nine steps about four
  percent of cells change owner. Interiors never move.
- `derive_seafloor_age` is hop distance from the final ridges within the final
  owner. `derive_boundary_deformation` is a profile around the final boundary
  scaled by final strength. Neither sees earlier steps; the world-heightmap
  doc records this as "none of these stages accumulates state during
  evolution".

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
the two development machines. Birth steps and accumulated displacement are
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

The size factor's configurable exponent is the one transcendental on the path
that boundary classes are decided from, which the determinism note above did
not anticipate. It costs nothing yet: kinematics is a float output that is
never pinned, and the raster pilot still quantizes its own random rotations
before upload. Slice 1's raster kernel has to settle the exponent as a fixed
sequence of square roots, or take the same `powf` on the host and quantize.

### Displacement migration and birth steps.

Per-edge accumulated displacement, divergent opening, per-cell birth step,
seafloor age from birth, base elevation reading true age.

### Accumulated deformation.

Per-step deformation increments into a persistent field; the final-boundary
derivation removed.

### Drifting poles.

Bounded per-step change of each plate's rotation vector.

### Lifecycle.

Rifting along new cracks and suturing of converged continental pairs.
