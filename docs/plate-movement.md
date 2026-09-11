# Plate movement

## Goal

Make plate motion coherent and make the evolution leave a record. Today the
partition has plate-like outlines, but what happens to them afterwards is
thin: motions are independent random rotations, boundaries only nibble at
convergent edges, and every later stage reads the final boundaries as if the
nine steps had never happened. The changes below keep the rigid-rotation model
and the simultaneous per-step structure, and replace the random inputs and the
stateless derivations with ones that have a past.

The measure of success is visual: convergence and divergence organised into
belts that span the sphere, mountain ranges whose width follows how long a
boundary converged, and oceans whose age deepens away from the ridge that made
them.

## Current state

All five slices — coherent kinematics, displacement-proportional migration,
accumulated deformation, drifting Euler poles, and plate lifecycle — have
landed, and so has the interior relief that follows them; see the last section
for that one.

- `generate_plate_kinematics(mesh, partition, crust, config)` fits each plate's
  Euler vector to a smooth global flow field over the plate's own cells, then
  scales the fitted direction by `plate_speed`: the plate's hashed base speed
  times a crust factor times a slab factor, clamped to
  `maximum_angular_speed`. A `coherence` fraction blends the axis back toward
  the hashed random one. Plates too small to fit — one or two cells — keep the
  hashed axis; the speed rule is the same for every plate. Nothing on the path
  uses a libm call, so the integer boundary classes the angular velocities
  decide stay exact. See "Slab pull" below for the speed rule and for the size
  factor it replaced.
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
- `classify_boundaries` derives per-edge normal and shear speeds from the two
  owners' rotations and classifies each edge. It is correct and stays.
- Evolution carries the material itself across steps, as one particle per
  parcel of crust holding its birth time and its accumulated deformation, plus
  the ownership and the two per-cell fields the cells read off it, the plate
  set with its motion, and how long each continental pair has been colliding.
  A step rotates every particle rigidly with its plate and each cell then
  resolves whatever landed in it. The closing and travel debts, and the
  ownership migration and field advection they paid for, are gone; see
  "Material transport" below for why and for what replaced them.
- Cell crust is derived from birth and never stored: `Some` is oceanic, `None`
  is original continental crust. `CrustClassification::cell_class` is gone, and
  every consumer reads `PlateEvolution::cell_crust`. Plate classes still
  describe plates and still decide volcanic-arc grouping, and a cell's own
  crust decides which of two parcels in it wins. A rifting continental plate therefore grows an oceanic margin.
- `derive_crust_birth_prior` is the old hop-distance algorithm, now producing
  the birth field evolution starts from, in model time:
  `Some(-hops * hop_duration)` for oceanic cells,
  `Some(-ridge_less_age * hop_duration)` for ridge-less oceanic plates, `None`
  for continental. `derive_seafloor_age` is `elapsed_time - birth`. See "Time
  and length units" for what a hop duration is and why both are times.
- `step_duration` defaults to 0.014, the time a plate at the default maximum
  angular speed of 1.0 takes to cross one cell width on the 65,536-cell default
  mesh. The rest of this bullet, and every count in the four that follow it,
  was measured while ownership moved by closing debt and the fields by
  upstream pull. They are kept as the record of what those rules did; the
  material-transport section replaced them. At the viewer's defaults nine steps produce 6473 proposals, 5752
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
  elevation rises from 0.084 to 0.169. It is `0.56` model time today, which is
  those forty default steps; "Time and length units" says why it is a time.
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
  step state.
- Kinematics is per-step state inside the evolving world rather than a fixed
  input. Every step ends, after deform and transport and before the
  next classification, by drifting each plate's rotation vector: the axis
  turns through the fixed angle `axis_drift_rate * sqrt(step_duration)` toward
  a fresh hashed direction perpendicular to it, and the speed is multiplied by
  `1 + s * speed_drift_rate * sqrt(step_duration)` for a hashed `s` in
  `[-1, 1)`.
  Four `signed_f32` draws per plate per step, from a `PLATE_POLE_DRIFT`
  stream on evolution's own new seed, supply the direction and the speed.
  Because the angle per step is fixed and only the direction is hashed, an
  axis takes a random walk on the sphere of directions, whose expected total
  wander over `n` steps is roughly `theta * sqrt(n)`. Both rates are therefore
  per unit *root* time, not per unit time: see "Time and length units" below
  for why, and for the retune that left a default step behaving as it did.
- What drifts is a per-plate *factor*, bounded to `speed_drift_limit` either
  side of one, and the speed a step ends on is `plate_speed` times that
  factor. A random walk against fixed global limits eventually piles every
  plate against one of them; a band around the rule's own answer keeps drift
  the perturbation of it that it is meant to be. Before "Slab pull" the walk
  was of the speed itself, banded around the speed the plate started the run
  with, which is the same band expressed against a speed that no longer
  stands still.
- `axis_drift_rate` defaults to 1.8 and `speed_drift_rate` to 0.9, both per
  unit root time, and `speed_drift_limit` to 0.5. At `DEFAULT_STEP_DURATION`
  the first turns an axis 0.213 radians a step, so `0.213 * sqrt(15)` is about
  forty-seven degrees of expected wander over a default fifteen-step run, and
  the second changes a drift factor by at most about a tenth a step, whose
  expected fifteen-step excursion is about a quarter. A half therefore bounds the tail
  of the speed walk without shaping its bulk: five of the 111 plates at the
  viewer's defaults reach the band edge. The rates stay bounded on purpose:
  larger drift makes boundaries flicker between regimes from step to step and
  blurs the accumulated fields into an average instead of a record.
- `PlateEvolutionInputs.kinematics` is still the initial motion, and
  `PlateEvolution` now returns the motion the run ended on. The viewer's
  `TectonicsWorld` stores that one and does not keep the initial motion at
  all; geology's hotspot trails and the viewer's motion arrows read it.
- The rotation is the tangent half-angle form, which is now
  `Vec3::rotated_toward` in `procgen-core`; the crack walk's private copy is
  gone. Nothing on the drift path calls libm, so the integer boundary classes
  the drifted vectors decide stay exact across machines.
- The defaults of every earlier stage were retuned in the same change, so the
  numbers below are measured at the retuned ones and are not comparable with
  the slice 2 and 3 figures above. Each pair below is the old value then the
  one that stands today. Mesh jitter from 0.5 to 0.8; crack arcs from 40 to
  16, curvature from 2 to 8, and subdivided faces from 0.4 to 0.8, which
  together give 111 plates at sampling seed 7 against 57 from fewer, larger
  crack faces split more often; oceanic speed factor from 1.4 to 1.5 and
  continental from 0.7 to 1.0, so crust class separates speeds less; evolution
  steps from 9 to 15 and the then minimum convergence from 0.3 to 0.5; and the
  convergent profile's depth from 3 to 6, for wider belts.
- At the viewer's retuned defaults, boundary edges after fifteen steps are
  5326 convergent, 5896 divergent, and 5326 transform, against 4842, 5660,
  and 4914 without drift. Migration events go from 9219 over 6878 distinct
  cells to 18,107 over 11,257, and crust-creation events from 5715 to 6369:
  unlike at the previous defaults, drift now moves substantially more,
  because a minimum convergence of 0.5 sits inside the range a drifting speed
  crosses rather than below it. Of the edges that were ever a boundary during
  the run — 30,583 without drift and 46,054 with — those that held more than
  one regime go from 2081 to 17,911. Accumulated deformation spans -0.249 to
  0.500 with a mean of 0.105 over 61,554 affected cells, against -0.287 to
  0.500 over 60,109.
- `full_deformation_time` still defaults to nine default steps while the run
  is now fifteen, so a boundary that holds one regime throughout raises more
  than the full profile and `maximum_magnitude` clamps it. That is a bound
  biting rather than a saturated field: 284 of the 65,536 cells reach it,
  0.4 percent. Raising `full_deformation_time` to the run length would undo
  it, at the cost of thinner belts everywhere.
- The plate set itself now changes during a run. `lifecycle.rs` owns the two
  events, and they happen at the end of a step, after the poles drift and
  before the next classification, so the next step's boundaries are the ones
  the new plate set implies. `EvolvingWorld` therefore owns plate classes and
  the plate count as well as ownership and motion; the classification the run
  was handed describes a plate set the run leaves behind, and
  `PlateEvolution::crust` is the final one every consumer downstream reads.
- Rifting: each step, every plate whose continental area exceeds
  `rift_minimum_area_fraction` of the sphere draws against
  `rift_rate * step_duration` on a `PLATE_RIFT` stream. It was every plate
  wholly continental above that fraction of total area until per-cell crust;
  see the bullet below for what the change did to the default world. A plate that rifts
  walks one arc of `cracks.rs` from a hashed cell of its own, in both
  directions, and the walk stops as soon as it leaves the plate, so the arc
  runs from boundary to boundary and the cells it crossed are the wall. The
  two largest connected components of non-wall cells beside the arc are the
  halves; the wall and any leftover component join the half they share the
  most edges with, which is `cracks.rs`'s own adoption rule and keeps both
  halves connected. Fewer than two components beside the arc means the rift
  separated nothing: the plate is left exactly as it was and the attempt is
  counted as a failed rift. The larger half keeps the plate's id and the
  smaller takes `plate_count`. Both halves are continental, and both keep the
  birth and deformation their cells carried, because a rift is a line in the
  crust rather than new crust: the ridge it becomes makes ocean cell by cell
  through the existing rebirth rule over the steps that follow.
- The halves start from the parent's rotation vector plus and minus
  `rift_opening_speed * normalize(n x m)`, for the wall's plane normal `n` and
  its mean center `m`. That axis is the one whose velocity at the rift is
  normal to the rift — `(n x m) x m = m (n . m) - n (m . m)`, and the wall lies
  in its own plane, so the velocity at `m` is `-n |m|²` — which makes the
  relative motion across the new boundary pure opening. `n` comes off a cross
  product, so its sign is fixed against the side of the plane the first half's
  own rift front sits on. A closed arc is the one degenerate case: its wall's
  mean center is the sphere's own, which leaves the opening direction
  undefined, so a rift is only well posed on a plate an arc can cross rather
  than circle.
- Suturing: each step counts, for every adjacent continental pair, the shared
  edges that are convergent with continental crust on both sides. A pair at or
  above `suture_minimum_shared_edges` grows its collision time by the step; a
  pair below it starts over, the same convention the closing debt uses for an
  edge that stopped converging. At `suture_time` the plate with more area
  absorbs the other: every cell takes the absorber's id, the absorber's
  rotation vector becomes the area-weighted mean of the two, and the absorbed
  id is left owning nothing. The merged plate's drift band restarts around the
  mean, because leaving it around the absorber's original speed would snap the
  merge's own motion straight back.
- The run ends by compacting: every plate id owning no cell is removed and
  ownership, the rotation vectors, and the plate classes are remapped to
  `0..live_count` in id order. `PlatePartition::validate` now requires that
  every id below `plate_count` owns a cell. Compaction removes the ids
  transport empties as well as the ones suturing does — the ownership move of
  the day could already wipe out a plate before this slice — so plate ids are no longer
  stable across a run, and a run's output plate count differs from the
  partition stage's in both directions.
- `PlateLifecycleConfig` sits under `PlateEvolutionConfig` beside
  `MaterialTransportConfig` and `PoleDriftConfig`. A `rift_rate` of zero and a
  `suture_time` of infinity each disable their event; `test_support`'s
  `NO_LIFECYCLE` is both, as `NO_POLE_DRIFT` is for drift.
- Defaults: `rift_rate` 4.5 per unit time, `rift_minimum_area_fraction` 0.04
  at the time and 0.012 since the profile retune below,
  `rift_curvature` 8.0 (the partition's own), `rift_opening_speed` 0.33,
  `suture_time` eight default steps, and `suture_minimum_shared_edges` 20.
  They were calibrated by running the viewer's defaults over nine, fifteen,
  and thirty steps and sweeping the two knobs that matter. The area fraction
  is one of them: the largest continental plate covered 0.044 of the sphere
  when whole plates were continental, so 0.05 made nobody eligible and 0.04
  made one or two. The rift rate saturates above three or so, because both
  halves of a rift fall below the minimum area and cannot rift again. The
  shared-edge count is the other: at eight the viewer's defaults suture six
  times over fifteen steps, at sixteen three times, and at twenty once. The
  opening speed is a third of the default maximum angular speed, which clears
  the default minimum convergence of 0.5 across the rift on its own.
- That area calibration no longer describes the default world. Per-cell crust
  made rift eligibility a plate's *continental* area rather than its total,
  and the largest continental area any plate holds at the viewer's defaults is
  0.0127 of the sphere against a minimum of 0.04, so nothing is eligible and
  the default world does not rift at any step count. The measurements below
  are re-measured at that state; `docs/land-and-ocean.md` records the change
  and the numbers behind it. Lowering the minimum to about 0.01 would restore
  rifting at the defaults, which is its own change with its own pins. The
  reference world of eighteen larger plates still rifts, twice over thirty
  steps. That change is the profile retune at the end of this document: the
  minimum is 0.012, the rift rate 3.0, and the default world rifts once over
  fifteen steps and twice over thirty, so the two bullets that follow describe
  a state the retune left behind.
- At the viewer's defaults a fifteen-step run produces 0 rifts, 0 failed
  rifts, 3 sutures, and 108 final plates, from the 111 the partition made:
  three merges and nothing migration emptied, since the run with the events
  disabled ends on all 111. Boundary edges after the run are 5678 convergent,
  6408 divergent, and 5654 transform, against 5806, 6497, and 5738 with the
  events disabled, and crust-creation events 6107 against 6116, over 17,555
  migration events against 17,588. A nine-step run gives 0 rifts and 2
  sutures, ending on 109 plates; thirty steps give 0 rifts, 0 failed rifts,
  and 3 sutures, ending on 106 because migration empties two more. Suturing is
  a small perturbation of the aggregate at these defaults, which is the point:
  it changes the plate set's history rather than its statistics.
- Integer pins that moved. The reference fixture's run now rifts once and
  sutures three times, so its ownership and birth fingerprints, and the base
  elevation and seafloor age fingerprints derived from them, were all
  re-pinned, along with the proposal, migration, and crust-creation counts.
  The ownership fingerprint of a run with neither drift nor lifecycle moved
  too, for a separate reason: compaction now removes the ids migration
  emptied, which renumbers every plate above them. That run's birth
  fingerprint is unchanged and is still the value pinned before this slice,
  which is what says the cells this slice moves and the crust it makes are
  otherwise the same.
- `Cracks::walk_arc` in `cracks.rs` is now the reusable single-arc walk: it
  takes the start cell and a rule for which cells the arc may enter, and
  returns the cells it crossed in walk order. The partition passes "not
  another arc's wall" and records the wall itself; the rift passes "owned by
  the plate that is splitting". `adopt_wall_cells` became
  `adopt_unassigned_cells` with an eligibility predicate, so the rift can run
  the same adoption confined to one plate. Neither generalisation changed the
  walk's output: the partition fingerprints pinned before this slice pass
  unchanged, which is the test that says so.
- Carried risk: the viewer's small-mesh fixtures each use a seed at which
  climate coupling reaches its fixed point, and every slice that moves terrain
  moves which seeds those are. Five changed here, two in slice 3, three in
  slice 2, four in slice 1. The fix
  belongs in the fixture — a mesh coarse enough to be fast but not so coarse
  that coupling is marginal — rather than in each slice's seed list. One more
  changed here.

## Design

Five changes, in the order they should land. Each is one slice with its own
prompt, review, and merge. Every one is a per-plate reduction or a per-cell
gather over the current step.

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
crust classification.

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

Two things came out differently from the intent above. Plate ids stop being
stable across a run: compaction has to renumber, and migration could already
empty a plate on its own, so the rule that every id owns a cell forces the
renumbering on every run rather than only on one that sutured. And a rift arc
is only well posed on a plate an arc can cross rather than circle, because a
closed arc's wall has its mean center at the sphere's own, which leaves the
opening direction undefined.

## Determinism

Kinematics is a float result and always was, and angular velocities are
quantized to a power-of-two grid. Slice 1 keeps that: the fit uses add,
multiply, divide, and square root, and its output is quantized once on the host
by the existing function. Boundary
classes and the ownership a cell resolves to are integers derived from those
quantized floats through comparisons, so they stay bit-identical run to run
and across the two development machines. One libm call remains in reach of the mesh
path's angular velocities, and it predates this slice:
`RandomStream::unit_vector` takes a sine and a cosine, and the hashed axis is
still a fifteen-percent share of every plate's direction at the default
coherence. If a re-pinned integer fingerprint ever splits between the two
machines, that is the first thing to replace, with a normalized triple of
`signed_f32` as the crack walk already uses. Birth times are exact multiples of
the step length or of the prior's hop duration, which is one divide over a
cell width and a speed. Accumulated deformation is a
float field and is tested for run-to-run equality and invariants only, never
pinned across machines.

A run's meaning does not depend on how the run is sliced, and one case of that
is exact. Take a world whose every rotation vector is zero, with drift and the
lifecycle off, and run it at `(step_duration, n)` and at `(step_duration / 2,
2n)`. Halving a float and doubling a count are both exact, so the two runs
cover the same elapsed time to the bit; nothing moves, so every cell keeps its
own particle; and crust birth, seafloor age, and base elevation come out
bit-identical.
`slicing_a_still_run_twice_as_finely_changes_nothing` is that statement, over
the reference world with its plates frozen; the prior it ages is untouched by
the freezing, because the prior reads the configured plate speed rather than
the fitted plates. A moving world cannot make the claim — see "Time and length
units" for what it can claim instead.

A float field that does carry a fingerprint is pinned through
`quantized_fingerprint`, which hashes each value's step on a 1/1024 grid
rather than its bits. Scaling by a power of two is exact, so the grid step is
an integer fact about the value, and the step is four orders of magnitude
coarser than the last bit of an `f32` near one. Base elevation, isostatic
support, and composed geological elevation are pinned this way; each one is
built from add, multiply, divide, and square root alone, so nothing on those
paths can cross a grid boundary between machines. Pinning `to_bits()` instead
pinned the toolchain, because one differing last bit changes the whole hash.

Slice 5 adds the arc walk, connected components, integer edge tallies, and
area sums, all of which are exact, plus one plane fit and one cross product
that use add, multiply, and square root alone. The floats that decide an
integer there are area comparisons, which every stage that weights by area
already depends on. Nothing on the lifecycle path calls libm, so the plate ids
it produces and the boundary classes the new rotation vectors decide stay
bit-identical across the two development machines.

Material transport adds no libm call either. A particle's rotation is add,
multiply, and divide in the same half-angle tangent form the pole drift uses;
its location is dot and cross products in f64 on unit directions, which is
what the hull already does; and a cell's resolution is comparisons with
integer tie-breaks. Cell owners and births are therefore integers decided by
float comparisons, the same class of result as the boundary classes, exact on
one machine and expected to agree between the two. Particle positions
themselves are a float field and are never pinned.

## Decisions

- Keep rigid rotations. Plates are rigid to first order, and the Euler-vector
  model gives boundary classification for free.
- Fit to a field rather than simulate forces. A real force balance would need
  a mantle model; a smooth field with speed factors gets the large-scale
  pattern at a fraction of the complexity. The slab factor "Slab pull" adds is
  a multiplier on a hashed base, not a force.
- Keep evolution as simultaneous steps. Everything here is a Jacobi update:
  every cell resolves against the ownership the step began with, and the new
  ownership is written only once every cell has been decided. That is what
  keeps a GPU mirror well-defined, and it holds for material transport too.
- Birth step, not age, is the stored fact; age is derived from the current
  step. Store one fact and derive the rest.

## Non-goals

- No mantle convection model, no force balance, no plate-boundary forces.
- No moving cells. The mesh is fixed: cells change owner and read what the
  material lying in them carries. The material does move, as particles under
  each plate's own rotation; see "Material transport".
- No history retained per step in the output beyond the accumulated fields.
  The viewer shows the end state and aggregate diagnostics, as today.
- No change to the partition stage.

## Slices

### Coherent kinematics. Landed.

Flow field, per-plate fit, crust and size factors, coherence blend, config
and viewer controls, docs. Kinematics signature takes the mesh, partition, and
crust.

### Displacement migration and birth steps. Landed.

Per-edge accumulated displacement, divergent opening, per-cell birth step,
seafloor age from birth, base elevation reading true age.

### Accumulated deformation. Landed.

Per-step deformation increments into a persistent field; the final-boundary
derivation removed.

### Drifting poles. Landed.

Bounded per-step change of each plate's rotation vector.

### Lifecycle. Landed.

Rifting along new cracks, suturing of converged continental pairs, plate-set
compaction, config and viewer controls, docs.

## Interior relief

A sixth slice, after the five above and small beside them. Nothing in the five
acts on a plate interior: boundary deformation reaches three to five cells from
a boundary, the cratons of the geology phase flatten toward a constant, and
base elevation gave every continental cell one number. A plate wider than about
ten cells was flat by construction. Two low-frequency fields are added to base
elevation, neither replacing anything.

- **Dynamic topography.** Where mantle flow converges the surface sags and
  where it diverges it swells. Slice 1's flow field already stands in for
  mantle flow, so the term is its divergence over the mesh, negated, divided by
  a fixed scale, times `dynamic_topography_amplitude`. It applies to oceanic
  and continental crust alike. The divergence is the discrete divergence
  theorem over each cell's Voronoi polygon, on `SphereMesh` beside
  `cell_gradients`: the field at each edge's midpoint direction dotted with the
  outward edge normal, times the edge's chord, summed over the cell's corners
  and divided by its area.
- **Continental basement.** Crust thickness varies between old shields and
  younger provinces. Three octaves of the fixed-polynomial gradient noise at
  `basement_frequency * 2^k`, amplitudes halving, normalised by the amplitude
  sum and scaled by `basement_amplitude`, on continental cells only. Its keys
  come from a new `PLATE_BASEMENT` stream on base elevation's own new seed.

`derive_base_elevation` therefore reads the mesh, the final seafloor age, the
final cell crust, and the flow field. `flow_field_keys` and `flow_velocity`
became the public `FlowField`, built once from `PlateKinematicsConfig`, so the
kinematics fit and base elevation sample one field from one config; the fit's
output did not change, which its pinned fingerprints prove.

Defaults are `dynamic_topography_amplitude` 0.03, `basement_amplitude` 0.05,
and `basement_frequency` 3.0. A lattice feature spans about two lattice cells,
so the basement's longest wavelength is two thirds of a model unit — a tenth of
a great circle, or some 48 cells of the default mesh — and its shortest, two
octaves up, about a dozen. The amplitudes are bounded so interior relief cannot
drown a continent on its own: `0.65 - 0.03 - 0.05` is 0.57, above the default
sea level of 0.5. That arithmetic is nominal rather than a bound, because the
dynamic term is normalised by the divergence field's RMS and not its peak.
Measured, the lowest continental base elevation is 0.516 at the viewer's
defaults and 0.532 at the reference world, so it holds with a thinner margin
than 0.57 suggests. A coast still moves only through the oceanic side, which
the dynamic term alone touches: land cells go from 17,957 to 18,096 at the
viewer's defaults and from 17,927 to 18,079 at the reference world.

`DIVERGENCE_SCALE` is 1.2, the RMS of the divergence field measured once at the
viewer's defaults, where it spans -3.38 to 5.53 with an RMS of 1.206. Dividing
by the RMS makes the amplitude the swell a typical divergence raises. The peak
is 4.6 times the RMS, so the term reaches several times its amplitude at the
most divergent cells, which is why base elevation now clamps to `[0, 1]` as the
composition stage does. Only the deep floor at 0.08 can reach that clamp: no
cell of the 65,536 does at the viewer's defaults, and 55 do at the reference
world, the lowest unclamped value being -0.022.

Measured at the viewer's defaults — sampling seed 7, jitter 0.8, 15 steps — and
at the reference world of sampling seed 9, subdivided faces 0.2, and 30 steps:

| | viewer defaults | reference world |
| --- | --- | --- |
| dynamic topography | -0.138 to 0.084 | -0.125 to 0.082 |
| basement, continental cells | -0.026 to 0.027 | -0.024 to 0.025 |
| base elevation | 0.014 to 0.744, mean 0.297 | 0.000 to 0.733, mean 0.247 |
| base elevation before | 0.088 to 0.650, mean 0.297 | 0.080 to 0.650, mean 0.247 |
| tectonic elevation | 0.071 to 1.000, mean 0.401 | 0.000 to 1.000, mean 0.335 |
| continental interior, five hops in | 0.465 to 1.000 | 0.399 to 1.000 |
| the same before | 0.491 to 1.000 | 0.371 to 1.000 |

The basement reaches about half its amplitude because three octaves rarely
align. The dynamic term is the larger of the two everywhere, and the term that
gives the deep ocean the gradient it never had.

The geology phase's craton flattening now reads
`base_elevation.cell_elevations[cell]` instead of the tectonics config's
`continental_base`, so a shield keeps the dynamic topography and basement its
own interior carries rather than levelling to one number. Isostasy's
`continental_support` stays a constant: it is the support continental crust has
away from every other effect, not the elevation a particular cell would stand
at.

No integer fingerprint moved. The only fingerprint over base elevation is now
taken with both amplitudes zero, which is what says the two terms are the whole
of the change, and every geology and climate pin downstream reads a synthetic
elevation field rather than an evolved one.

Retired risk. The 65,536-cell mesh had about 180 cells whose Voronoi corner
ring was locally inverted, and they were where the divergence field's peak of
4.6 RMS came from. The circumcenter was not the cause. Two of the three causes
were in the hull: its visibility predicate read a plane distance as the
difference of two f32 quantities of magnitude one, which cannot resolve the
sign for a near-cocircular quadruple of points and left the triangulation
locally non-Delaunay at 84 edges; and it read the f32 points themselves, whose
6e-8 radial error a hull takes for a weight, which tilts the plane between two
cells a few ten-thousandths apart far enough to leave one of them outside its
own cell. The circumcenter mattered only once those two were fixed, because a
triangle spanning such a pair has a normal that swings by a few percent of a
cell width per f32 step. `SphericalDelaunay` now decides all three in f64 on
unit directions, no cell of the mesh has an inverted ring at any count, jitter,
or seed tried, and a rigid rotation's divergence peaks at 0.024 there against
the 4096-cell mesh's 0.040. `tests/topology.rs` asserts the ring invariant at
the count and jitter the viewer runs at.

### Continental flood basalt provinces

A second interior source, in geology rather than base elevation. The hotspot
stage already traces each plume's trail of up to eight cells opposite its
plate's motion, so every hotspot was a point feature a few cells long. On Earth
a plume under a continent does something else as well: it floods a broad,
flat-topped plateau hundreds of kilometres across in a few million years, as
the Deccan Traps and the Columbia River basalts did. That plateau is one of the
few sources of relief a continental interior has away from any boundary, and
the pipeline had nothing like it.

A hotspot is a candidate when its source cell's crust is continental, read from
the evolution's `CellCrust`, which the hotspot stage now takes. Each candidate
rolls one hashed draw from a new `HOTSPOT_PROVINCE` stream on the hotspot seed
against `province_fraction`. A province is every cell within
`province_radius_hops` mesh hops of the source, walking only through cells that
are continental and on the source's plate, so it stops at a coast and at a
plate boundary. Weight is one within `province_radius_hops - province_rim_hops`
hops and falls linearly toward zero at the radius: a flat top with a sloped
rim, which is the shape of a flood basalt pile at this scale. The radius is
where that fall reaches zero, so the ring at it is outside the province and
every cell of one carries a positive weight. Weights are rationals
of two integers and hop distances are integers, so the cells and the profile
are bit-identical across machines. `HotspotField` carries them as
`cell_plateau`, the maximum province weight over every province covering a
cell, resolving overlaps by maximum as the intensity field does. Composition
adds `plateau_uplift` times that weight in the same pass as hotspot uplift,
before craton flattening, so a craton partly flattens an old province — which
is acceptable, since a real one erodes too.

Defaults are `province_fraction` 0.4, `province_radius_hops` 5, and
`province_rim_hops` 2. Five hops is about 440 km at Earth scale, so a plateau
spans roughly 900 km. `plateau_uplift` is 0.06, comparable to the Deccan's
height above the Indian shield in normalised units.

The fraction is 0.4 rather than the third a "few plumes in a few tens of
millions of years" reading suggests, because the candidate pool is smaller than
it looks. Twenty hotspots put only four sources on continental crust at the
viewer's defaults and three at the reference world. A quarter of the sphere is
continental, so five is the expectation and three or four an ordinary draw.
At 0.3 none of those seven draws fires: the lowest are 0.378 and 0.340, so both
worlds get their first province just above 0.34 and neither has one at 0.3.
Raising the fraction to 0.4 buys one province at the viewer's defaults and two
at the reference world, which is the "a couple per world" this is aiming at,
without making a province the normal fate of a continental plume.

| | viewer defaults | reference world |
| --- | --- | --- |
| continental sources, of twenty | 4 | 3 |
| provinces | 1 | 2 |
| plateau cells | 79 | 120 |
| cells whose elevation moved | 79 | 120 |
| the same, continental interior | 23 | 120 |
| largest uplift applied | 0.060 | 0.030 |
| provinces at fraction 1.0 | 4 | 3 |
| plateau cells at fraction 1.0 | 192 | 190 |
| continental interior, five hops in | 0.421 to 0.888 | 0.399 to 0.900 |
| the same before | 0.421 to 0.888 | 0.399 to 0.900 |
| land cells | 18,539 | 18,157 |
| land cells before | 18,539 | 18,157 |

The continental interior here is every continental cell at least five hops from
a plate boundary, 6,650 cells at the viewer's defaults and 12,628 at the
reference world, measured on geological elevation. Its range does not move: a
province is a couple of hundred cells and its 0.06 never reaches either extreme
of six thousand, and no coast moves either, because a province stops at one.
What moves is inside the range. The reference world's largest uplift is 0.030,
exactly half the amplitude, because both of its provinces sit on a
full-strength craton and `craton_flattening` is 0.5 — the flattening the model
accepts. Every plateau cell moves at the default, which is what says no cell of
a province is dead weight; only at fraction 1.0 do the counts part, where four
of the viewer's 192 plateau cells already stood at one and clamp.

Zero fraction reproduces the field exactly, which a test asserts against the
pre-slice fingerprint. No integer pin moved: every geology and climate pin
downstream reads a synthetic elevation field or a fixture too small to hold a
province.

Erosion is the next stage, and it is the reason this one exists: it needs
slopes to move material down, and until now a plate interior had none.

## Sea level

Sea level was the constant `SEA_LEVEL`, 0.5, and it is now a datum
`CoarseElevationConfig` carries and the composed `CoarseElevation` carries
onward, defaulting to the same 0.5. Every reader either holds the field and
asks `CoarseElevation::is_land`, or is handed the datum explicitly:
`is_land(elevation, sea_level)`, the climate stages' `sea_level` input, the
terrain height and tile inputs, and the terrain shader's parameter buffer. The
elevation field is not interpretable without it, the same way a `SphereMesh` is
not interpretable without its radius, which is why it travels with the field
rather than being read back out of a config.

The composition arithmetic does not change, so at the default the whole
pipeline is bit-identical and no fingerprint moved. What the knob does is move
the coast: at the reference world of sampling seed 9, subdivided faces 0.2, and
30 steps, tectonic elevation holds 19,311 land cells at a datum of 0.45, 18,150
at 0.50, and 17,249 at 0.55 out of 65,536. Raising the datum can only flood and
lowering it can only expose, which a test asserts at those three values.

That knob floods and exposes whatever the elevation field happens to put near
the datum, which is a margin only where the field already has one. Continental
crust is still classified per plate, so a continent is a plate-shaped patch of
the configured continental base and its coast is wherever interior relief and
deformation carry that patch across the datum. Replacing per-plate crust
classification with per-cell continental nuclei is the next step, and it is
what gives a margin its own shape for this datum to cut.

## Profile retune

The deformation constants were ported verbatim from the C# reference's
`ElevationOps.cs` when the default mesh went to 32,768 cells, and only one of
them has been re-decided since: the drift retune took the convergent profile's
depth from 3 to 6 for wider belts, and the mesh has doubled again to 65,536
under all of them. Two of the rest are wrong in sign or kind rather than in
scale, and two other defaults changed unit or eligibility under later slices
without being revisited. Five changes, all defaults and one rule, with the
vertical scale read as the land mapping in `procgen-planet` gives it:
normalized 1.0 is 10,000 m above the 0.5 datum, so 0.05 of normalized
elevation is about a kilometre, and a hop is one cell width, about 88 km at
Earth radius on the 65,536-cell mesh.

### Transform relief follows the residual normal component

`boundary_source` gave every transform edge the transform profile scaled by
its *shear*, so pure lateral slip raised the same central offset as a
convergent boundary and spread it over a 500 km belt. Eleven percent of the
default world's land was built that way: zeroing the transform offset against
the old defaults dropped land from 15,379 cells to 13,656 and moved 13,022
cells by more than 0.05.

Earth's transform boundaries make almost no relief. Oceanic fracture zones are
sub-cell scarps, and continental transforms build ranges and basins only at
bends, where the small normal component of the motion is compressive
(Transverse Ranges) or extensional (Dead Sea, Salton Trough) — relief one cell
wide at this resolution.

`source_scale` now scales a `Transform` edge by its *signed residual
convergence* over `saturation_speed`, clamped to ±1 and not clamped below
zero; convergent and divergent edges still scale by their own strength, which
is a magnitude. Pure shear raises nothing, a transpressive bend raises, and a
transtensional one subsides. `retain_stronger_source` compares magnitudes, so
a negative source already competes correctly, and
`PropagationProfile::from(BoundaryEffect)` carries the sign into the flank, so
a negative offset produces the pull-apart basin shape rather than a graben
with raised edges. The transform profile keeps `offset: 0.4` and moves to
`depth: 1`: at the same closing speed a bend raises what a convergent boundary
would, over one cell instead of six, and at a typical transform obliquity —
residual convergence 0.1 to 0.3 — it raises 0.02 to 0.06, half a kilometre to
a kilometre.

Against the new defaults, transform boundaries still build relief on 30,774
cells at the defaults and 25,135 at the reference world, but only 149 and 179
of those move by more than 0.05: land changes by 20 cells at the defaults and
22 at the reference world, and mean tectonic elevation by 0.002. That is the
intended reading — transforms are visible in the field and negligible in the
coastline.

### Continental rifts get raised shoulders

`ContinentalRiftProfile` defaulted to centre −0.4, flank −0.1, decay depth 4,
and `is_valid` required `center < flank < 0`: a 700 km depression with no
edges. Suppressing it raised land from 15,379 cells to 16,521, so the old
profile drowned about 1,100 continental cells along divergent boundaries.

Rift shoulders stand high on Earth. The East African Rift's floor is about a
kilometre below shoulders that rise one to two kilometres above the plateau
behind them, and the Red Sea, the Rhine graben, and the Basin and Range flanks
are the same shape: a valley one cell wide at this resolution with shoulders
one to two.

The defaults are now centre −0.2, flank +0.08, decay depth 3 — a one-cell
graben as deep as the trench profile, shoulders about 1.6 km at saturation
that fade to zero two hops out. `is_valid` became `center < 0 && flank >
center`, so a positive flank is legal and the old all-negative graben still
is; the `InvalidRiftProfile` message says so, and the validation test's
positive-flank case flipped from invalid to valid with a flank below the
centre taking its place. Suppressing the new profile raises land from 14,916
to 15,268: it costs 352 land cells where the old one cost 1,142.

**The opened ocean gets shoulders on both sides.** At the defaults, 2,096
continental cells touch ocean that was born during the run. With every other
profile zeroed so the rift profile is alone in the field, 522 of them carry
positive deformation, the largest is 0.0616 against a flank of 0.08, and none
exceeds `continental_base + flank` — the shoulders are bounded by the profile
that raised them. In the full field 1,928 are raised and 578 exceed that
bound, because a cell beside run-born ocean can also sit in a convergent belt,
where `maximum_magnitude` rather than the rift profile is what bounds it.

### Abyssal-hill age window was left in hops

`OceanicPeakFieldConfig::maximum_young_age` was 4 and its doc comment still
said "hop age". Seafloor age has been steps since birth since the
displacement-migration slice, which is why `cooling_age` was retuned 8 to 40;
this one was missed. At the defaults it made 3,497 of 48,490 oceanic cells
eligible, seven percent, against ages that run p10 6, median 16, p90 23,
maximum 40.

Abyssal hills form at the ridge and are the most common landform on the
planet; they fade under sediment over tens of millions of years, not within
the youngest few percent of the floor. `maximum_young_age` is now 10, a
quarter of `cooling_age`, and the doc comment says steps. At the defaults
10,691 of 48,465 oceanic cells are eligible and 1,818 abyssal-hill peaks are
placed, against 3,497 and 647 before; at the reference world 3,614 of 48,172
and 621 hills, against 1,092 and 212. Seamount counts barely move — 44 to 43
and 56 to 58 — since they read hotspot intensity rather than age, and shift
only where a cell carrying both claims now goes to the hill instead.

### Rift eligibility never fired at the defaults

Since per-cell crust, eligibility is a plate's *continental* area rather than
its extent, and the largest any plate holds at the viewer's defaults is 0.0127
of the sphere against a `rift_minimum_area_fraction` of 0.04 — so the default
world rifted at no step count, and the comment above the default still quoted
the pre-per-cell figure of 0.044.

The minimum is now 0.012. Swept at the defaults over fifteen and thirty steps,
at the rift rate of 4.5 that stood at the time:

| Minimum | Eligible at step zero | Rifts over 15 | Rifts over 30 |
| ------- | --------------------- | ------------- | ------------- |
| 0.008   | 10                    | 9             | 12 (1 failed) |
| 0.010   | 5                     | 4             | 8             |
| 0.012   | 4                     | 3             | 4             |
| 0.014   | 0                     | 1             | 3             |
| 0.016   | 0                     | 1             | 1             |
| 0.040   | 0                     | 0             | 0             |

The area fraction alone cannot land on one or two rifts over fifteen steps.
0.01 gives four, and the values that do give one or two — 0.014 and above —
reach it with *nothing* eligible at step zero: their rifts happen only after a
suture has grown a plate past the threshold, which is the same accidental
regime 0.04 was in. 0.012 is the value taken for eligibility: four plates
eligible at step zero against a largest continental area of 0.0127 and a
fourth largest of 0.0122, and not every eligible plate rifting.

### The rift rate is swept against that minimum, not alone

The two knobs decide the count together, so fixing eligibility and leaving the
rate at 4.5 leaves the defaults at three rifts over fifteen steps. Swept at
the new minimum, where the same four plates are eligible throughout:

| `rift_rate` | Seed 7, 15 steps | Seed 7, 30 steps | Seeds 9 / 11 / 13, 15 steps |
| ----------- | ---------------- | ---------------- | --------------------------- |
| 1.50        | 0                | 0                | 1 / 0 / 0                   |
| 2.25        | 0                | 0                | 1 / 0 / 1                   |
| 3.00        | 1                | 2                | 1 / 0 / 1                   |
| 4.50        | 3                | 4                | 1 / 0 / 3                   |

`rift_rate` is now 3.0, a chance of one in twenty-four a step per eligible
plate: one rift over fifteen steps and two over thirty, which is the density
this knob was always meant to give, and it scales with run length rather than
saturating. The response is lumpy rather than smooth — 2.25 gives none where
3.0 gives one — because the draw is a hashed value per plate per step, not an
expectation over many plates, so there is one value here rather than a range.
Seed 11 is a 34-plate world whose single attempt fails to separate anything at
every rate. The reference fixture's own rate is the default scaled by its step
duration, and the two draws that pass at 4.5 still pass at 3.0, so this change
moves no pin.

### Measured

Both worlds are 65,536 cells at the viewer's defaults: sampling seed 7, jitter
0.8, and 15 evolution steps for the first, and the reference world's sampling
seed 9, subdivided faces 0.2, and 30 steps for the second. "Before" is the
pre-retune pipeline and "after" is what all four changes give together. Land
and elevation are on tectonic elevation, where the viewer reports them.

| Measure                    | Defaults, before | Defaults, after | Reference, before | Reference, after |
| -------------------------- | ---------------- | --------------- | ----------------- | ---------------- |
| Land cells at 0.5          | 15,379           | 14,916          | 17,311            | 15,537           |
| Tectonic minimum           | 0.0683           | 0.0683          | 0.0003            | 0.0003           |
| Tectonic maximum           | 1.0000           | 1.0000          | 1.0000            | 1.0000           |
| Tectonic mean              | 0.3653           | 0.3447          | 0.3289            | 0.2977           |
| Deformation minimum        | −0.3716          | −0.1891         | −0.3997           | −0.2156          |
| Deformation maximum        | 0.5000           | 0.5000          | 0.5000            | 0.5000           |
| Deformation mean           | 0.0992           | 0.0787          | 0.0921            | 0.0666           |
| Affected cells             | 61,935           | 62,038          | 44,273            | 43,393           |
| Rifts / failed / sutures   | 0 / 0 / 3        | 1 / 0 / 2       | 2 / 0 / 2         | 3 / 0 / 2        |
| Final plates               | 108              | 110             | 18                | 18               |
| Oceanic cells              | 48,490           | 48,465          | 47,425            | 48,172           |
| Peak-eligible cells        | 3,497            | 10,691          | 1,092             | 3,614            |
| Peaks, seamount / hill     | 44 / 647         | 43 / 1,818      | 56 / 212          | 58 / 621         |
| Cells whose elevation moved| —                | 63,582          | —                 | 52,274           |
| Cells moved by over 0.05   | —                | 11,807          | —                 | 22,049           |

Land at the datum falls 3 percent at the defaults and 10 percent at the
reference world. The defaults move little because the two changes that act on
land pull against each other: transform boundaries stop building eleven
percent of it, and the rift graben stops drowning about eleven hundred
continental cells, leaving one new rift as the net loss. The reference world
falls much further because it is the world where rifting was already happening
and now happens half again as often, each rift opening ocean through a
continent.

Deformation's negative tail halves at both worlds — −0.19 against −0.37 at the
defaults — which is the rift centre moving from −0.4 to −0.2 and the pull-apart
side of a transform never reaching the old shear-scaled magnitudes. The
positive extreme stays at `maximum_magnitude`, but far fewer cells reach it at
the defaults: 121 of 65,536, against the 284 the pre-retune defaults recorded,
because the clamp was mostly being fed by transform belts holding one regime
for a whole run. The reference world, at twice the run length, reaches it on
835.

### Not changed, and why

- `convergent` 0.4 / depth 6, `collision` 0.5 / 5, `trench` −0.2 / 1. The
  Andean asymmetry is right and the trench is one cell as it should be. Depth
  6 puts every collision belt at about 1,000 km total width, Tibet scale,
  where 4 would be nearer the Andes and Alps; left as the visual choice the
  drift retune made. The symmetric uplift at ocean–ocean convergence is gone;
  see "Island arcs".
- `saturation_speed` 2.0, `full_deformation_time` nine default steps,
  `maximum_magnitude` 0.5. Time-unit questions, left for the time-consistency
  item.
- Hop-based geology radii: cratons 3 + 3, isostasy 5, arc inland 2, hotspot
  trail 8, province radius 5, basin minimum 3, margin width 3. All were tuned
  at 125 km cells and now cover seventy percent of that distance, but each
  still lands inside the Earth range for what it represents — arc–trench gap
  100 to 250 km, cratons hundreds of km from boundaries, shelves 80 to 300 km.
- Vertical constants: continental base 0.65, ridge 0.30, deep floor 0.08, and
  the geology uplifts. Under the 0.05-per-km land mapping the deep floor is
  8.4 km and the continental base 3 km, both stretched about 1.5× against
  Earth, but they are stretched together and every downstream stage reads
  them. A vertical-datum calibration is its own decision, probably alongside
  erosion.
- Plate count, speeds, migration threshold, drift, suture.

### Pins

Deformation is a float field tested for run-to-run equality and invariants and
never pinned across machines, and base elevation's fingerprint is taken with
interior relief off and does not read deformation, so the first two changes
move no integer pin on their own. The rift minimum gives the reference fixture
a rift it did not have — one split and one arc that separated nothing, against
none of either — which moves that run's ownership and birth fingerprints and
the base elevation and seafloor age fingerprints derived from them, along with
its proposal, migration, and crust-creation counts and its final plate count.
The rift rate moves nothing: that fixture's rate is the default scaled by its
own step duration, and both draws that passed at 4.5 still pass at 3.0.
The abyssal-hill window moves the oceanic-peak fingerprint, whose synthetic
fixture holds ages in 0..8 and so gains cells in 5..=10. Each was re-pinned
once. The ownership and birth fingerprints of a run with neither drift nor
lifecycle are unchanged, which is what says nothing outside the lifecycle
moved a cell.

## Material transport

One slice, after the profile retune. It replaces the two transport substeps of
an evolution step — ownership migration by closing debt and field advection by
travel debt — with one. Material lives in particles that rotate rigidly with
their plate, and cells sample them. The output of a run keeps its shape, so
nothing downstream of `PlateEvolution` changed.

### Why particles, not patched rules

Neither of the two rules conserved anything, and the loss was structural
rather than a tuning error. Measured at the viewer's defaults by comparing the
initial continental mask to the final one read from birth steps, the
continental area fraction went 0.297 at step zero, 0.260 at fifteen steps,
0.261 at thirty, 0.334 at sixty, and 0.447 at a hundred and twenty: a fifteen
percent loss that turned into a fifty percent gain. Attribution over the
fifteen-step run, per substep: migration +2,568 cells of continental copying
onto the oceanic cell it overrode, advection −4,862, of which the rebirth of
continental trailing cells was only 666.

The rest of the advection loss was the upstream pull itself. The pull map at
step zero had 21,191 cells pulled by two or more downstream cells and 22,243
pulled by none, so a third of the material was duplicated and a third dropped
every time a plate moved one cell. A single plate under a pure rotation with
no boundaries at all lost 5.5 percent of a continental cap in forty steps.

A cell-to-cell scheme cannot fix that. A per-step permutation of cells along
the flow does not exist on this mesh: a deferred-acceptance matching of every
cell to a downstream neighbour leaves 9,186 of 65,536 unmatched under a pure
rotation and forms no cycles, so a chain scheme that moves only complete
chains stalls the whole plate. Conservation needs the material to be points,
not cells.

### What a step does

`transport.rs` owns the particle, the config, the substep, and every write to
a particle's plate that a lifecycle event implies; `migration.rs` is gone. A particle is a unit position, a plate, a birth step,
a deformation, and the cell and Delaunay triangle its last location walk
found. `EvolvingWorld` holds the particles and, as the projection every
consumer reads, the ownership, birth, and deformation each cell sampled from
the particle that won it. `CarriedFields`, `edge_closing`, and `cell_travel`
are gone with the rules they served.

A step is deform, transport, drift, lifecycle, reclassify — the same slots as
before with migrate and advect merged into one.

- **Deform.** The per-cell boundary profile as before, added to every particle
  in the cell, stacked ones included, and clamped to `maximum_magnitude` per
  particle. `accumulate_boundary_deformation` became
  `boundary_deformation_increment`, which returns the scaled per-cell
  increment and leaves the accumulation to its caller.
- **Move.** Each particle turns about its plate's rotation vector by
  `|ω| · step_duration`. The position splits into the part along the axis,
  which the rotation leaves alone, and the part across it, which
  `Vec3::rotated_toward` turns toward `axis × position` — a vector
  perpendicular to it and of its own length, which is what makes the turn
  preserve that length. This is the same libm-free half-angle tangent form the
  pole drift uses, and the realized angle is short by four parts in ten
  thousand at the default step. Every particle of one plate goes through the
  same linear map, so a plate's material is rigid by construction rather than
  by tolerance. The design for this slice prescribed rotating the whole
  position toward the normalized velocity instead; that form ignores the sine
  of the angle from the axis, over-rotates everything off the plate's equator,
  and is not rigid, so the conservation test it asks for would not have
  passed.
- **Locate.** `SphereMesh::locate_delaunay(position, hint)` with the
  particle's previous triangle as the hint, so a step costs a short walk per
  particle. The particle's cell is the located triangle's corner cell with the
  greatest dot product with the position.
- **Resolve.** Each cell takes the particle that wins it, by one lexicographic
  order: continental over oceanic; then, among continental, the cell's
  previous owner over a newcomer; then, among oceanic, the younger; then the
  nearer to the cell's centre; then the lower particle index. The
  incumbent-then-age split is what the design asked for, written as one total
  order rather than a pairwise rule, because the pairwise rule is not
  transitive — three particles on two plates can cycle, and the winner would
  then depend on the order they were compared in.
- **Subduct.** A losing particle is removed when it is oceanic, belongs to a
  different plate than the winner, and the cell has an edge to a cell of the
  loser's plate that the current classification calls convergent. That is the
  trench and nothing else. Every other loser survives, stacked: two parcels of
  one plate are lattice noise that spreads back out next step, continental
  material is never destroyed, and a parcel that crossed a transform or a
  ridge is not being subducted.
- **Sample.** A cell no particle reached takes the nearest particle within
  `gap_radius` cell widths of its centre, preferring particles of its previous
  owner, searching the occupants of its one- and two-hop rings. This is a
  raster fill, not a material event: the cell reads what the particle carries
  and no particle is created or moved.
- **Make floor.** No particle within `gap_radius`: one is created at the cell
  centre, oceanic, born this step, flat, on the cell's previous plate. This is
  the ridge, and it is the only place a run creates material. A particle made
  here is invisible to the rest of the substep, so two adjacent gap cells both
  make floor rather than one sampling the other's — the same Jacobi rule every
  other substep follows.

The lifecycle carries material too, through two methods on the evolving world
that `transport.rs` owns, so `lifecycle.rs` never touches the particles
itself. `relabel_particles(from, to, scope)` is the one loop all of it needs:
a suture takes the `WholePlate` of what it absorbs, and a rift takes only the
material in `OwnCells` — the cells the new half now owns — so the halves part
carrying their crust and the arc between them opens a real gap. The design
said "every particle in a cell of the new half"; a particle of a third plate
stacked there is not the parent's to give away and keeps the plate it belongs
to. `remap_particle_plates` is the second, for compaction: it drops the
particles of a plate the run left owning no cell at all, which only compaction
can do, because that material is stacked under other plates' cells and nothing
moves it again.

### `gap_radius`

`MaterialTransportConfig::gap_radius` defaults to 1.5 cell widths and is the
only knob the slice adds; `PlateMigrationConfig::minimum_convergence` and its
viewer control are gone.

Cell areas vary, so a rigid rotation on its own leaves a third of the cells
empty and a quarter doubled at any moment — that is jitter, not transport, and
none of it may be read as a gap. Measured on the default mesh after rigid
rotations of 1, 7, and 50 cell widths, no empty cell's nearest particle was
further than 1.5 cell widths: zero false gaps at all three distances, where a
radius of 1.0 would have made 2, 672, and 949 cells of false floor. The real
gaps are much wider: after five steps of the viewer's actual plate motion with
no resampling, 756 cells exceeded 1.5 widths and 103 had no particle within
two hops at all.

The radius has a ceiling as well as a default. The search reads the one- and
two-hop rings, which reach about two cell widths, so a larger radius is not a
wider search but silent false floor: the cell would accept material it never
looks at. `MAX_GAP_RADIUS` is that ceiling, `evolve_plate_ownership` rejects a
radius above it, and the viewer's slider reads the constant rather than
restating it.

### What comes out for free

- Boundaries move with the plates. Two plates moving together carry their
  shared boundary along; a plate moving away from a stationary one opens ocean
  behind it at its own speed.
- Subduction rate equals relative convergence: the overriding plate's arrivals
  find the subducting plate's cells vacated at that plate's speed and occupied
  otherwise.
- Ocean–ocean polarity: the older floor subducts. "Island arcs" lifts that
  rule out of `occupant_order` into `material_order` and gives deformation and
  the volcanic arcs the same answer.
- Continental collision stacks particles rather than destroying them. The
  depth of the stack is a thickness signal for a later slice; nothing reads it
  yet.
- A rift opens an ocean by moving the halves apart, not by turning the wall
  into floor.

### Measured

At the viewer's defaults — sampling seed 7, jitter 0.8, 15 steps — and at the
reference world of sampling seed 9, subdivided faces 0.2, and 30 steps:

| | viewer defaults | reference world |
| --- | --- | --- |
| continental particles, start to end | 19,365 to 19,365 | 19,358 to 19,358 |
| continental cells, start to end | 19,365 to 18,252 | 19,358 to 18,936 |
| land at the 0.5 datum | 17,010 (0.260) | 18,167 (0.277) |
| born particles | 4,437 | 3,804 |
| subducted particles | 12,068 | 8,625 |
| owner changes | 42,722 | 32,446 |
| collided-cell events / deepest collision | 20,896 / 8 | 13,972 / 8 |
| sampled-cell events | 372,326 | 722,692 |
| rifts / failed rifts / sutures | 2 / 0 / 14 | 2 / 1 / 6 |
| final plates | 99 | 14 |
| oceanic age p10 / median / p90 | 5 / 17 / 23 | 13 / 38 / 55 |
| evolution, per step | 52 ms | 41 ms |

The continental particle count is exact at both, which is the whole claim.

A collided cell is one holding material of more than one plate after
subduction, and the depth is the foreign material plus the parcel that won the
cell. Counting every cell with two parcels in it instead gives 232,726 and 9
at the defaults, nine tenths of which is the same cell-area variance that
leaves a third of the cells empty — a count of the lattice, not of collisions.

The continental *cell* series at the viewer's defaults, which replaces the one
this section opened with: 0.279 at fifteen steps, 0.272 at thirty, 0.258 at
sixty, 0.244 at a hundred and twenty, from 0.295 at step zero, which is what
the configured target of 0.297 grew to. The target was
a flat line and this is a slow decline of five points over a hundred and
twenty steps, against a swing from 0.260 to 0.447 before. The decline is the
collision stack: continental material that ends a run under another
continent's cell occupies no cell of its own. It is the thickness signal
arriving as a cell-count deficit, and a later slice that reads the stack is
where it belongs.

Cost is 52 ms a step at 65,536 cells against a whole tectonics phase of
0.89 s. Of that, about 13 ms is the move — the location walks — and about
14 ms the resolution, of which the empty-cell ring search is most. The ring
buffer is reused across cells; allocating it per empty cell cost 6 ms a step
on its own.

### Risks, measured

- **Continental ghosting.** Two continents that collide stack for up to
  `suture_time` before the lifecycle merges them, and the losing plate's
  particles keep moving through the winner's cells until then. At the viewer's
  defaults 908 of 19,365 continental particles, 4.7 percent, end a run in a
  cell owned by another plate. That is a few hundred kilometres of
  underthrust, which is the real thing, so the blocking rule the design held
  in reserve is not needed.
- **Boundary flicker.** An empty boundary cell sampled from a neighbour can
  change owner step to step, and incumbent preference is the mitigation. There
  are 42,722 owner changes at the defaults, against 17,588 migration events
  measured at the same settings with the lifecycle disabled — not the same
  quantity, but the same order. The reason not to expect speckle in the
  deformation field is structural rather than measured: the field rides the
  particles, so a cell that flickers between two plates reads two parcels the
  same boundary has been raising. Looking at the two worlds side by side is
  still outstanding.

### Pins that moved

Every integer fingerprint downstream of a run: ownership and birth on the
reference fixture with and without drift and lifecycle, seafloor age, and base
elevation, along with the reference run's aggregate counts and its final plate
count. Each was re-pinned once. The geology and climate pins read synthetic
elevation fields rather than an evolved one and did not move, as expected.

## Time and length units

Every configured quantity that describes a rate or an age now means the same
thing whatever the step duration and whatever the mesh. Four things did not,
and one of them was a live defect in the particle transport.

### Drift rates are per unit root time

Pole drift is a random walk, and a random walk's spread grows with the root of
the time it takes. Turning an axis by `axis_drift_rate * step_duration` each
step therefore gave a total wander over a fixed time `T` of
`rate * sqrt(T * step_duration)`: halving the step halved the variance. The
`PoleDriftConfig` doc claimed the opposite — that a shorter step only changes
how many steps a given wander takes — and that claim was wrong.

Both rates are now per unit root time: `half_tangent` is
`0.5 * axis_drift_rate * sqrt(step_duration)` and the speed span is
`speed_drift_rate * sqrt(step_duration)`. `sqrt` is IEEE-exact and was already
on the path, so no libm call is added. The defaults were retuned so that a
default step behaves as it did: `15 * 0.014` and `7.5 * 0.014` become
`1.8 * sqrt(0.014)` and `0.9 * sqrt(0.014)`, which turn an axis 0.213 radians a
step against 0.21 and scale a speed by at most 0.106 against 0.105. The
viewer's two drift slider ranges were stated as a per-step maximum over
`DEFAULT_STEP_DURATION`; they are now that maximum over its root.

`half_axis_turn` exposes the per-step angle, so the test that checks how it
scales reads it rather than recovering it from a rotated vector. The scaling is
exact: `sqrt(1/4)` is 1/2 in IEEE, so the turn at `step_duration / 4` is
exactly half the turn at `step_duration`, for any rate.

### Birth and age are model time

`cell_birth` was `Option<i32>` steps and the prior wrote `-hops` into it as if
one hop were one step. `SeafloorAge` was steps. `cooling_age` (40),
`maximum_young_age` (10), and `abyssal_age_saturation` (8) were integers whose
meaning changed with the step and with the mesh. The one coincidence that made
"hop" and "step" line up was `DEFAULT_STEP_DURATION` itself, which is defined
as the time a plate at maximum speed takes to cross one cell of the
65,536-cell mesh — true at the defaults and nowhere else.

Birth is now `Option<f32>` model time, on the particle and in `cell_birth`. A
particle born at step `k` gets `k * step_duration`; `None` is still original
continental crust, so `is_continental` and `CellCrust` are unchanged. The prior
writes `-hops * hop_duration`, where the hop duration is
`mean_cell_width(mesh) / (maximum_angular_speed * radius)` — exactly what
`DEFAULT_STEP_DURATION` was defined as, so at the defaults the prior's births
are the values they were, and on any other mesh or at any other step they are
what they were meant to be. `derive_crust_birth_prior` takes
`PlateKinematicsConfig` for that one number. `ridge_less_age` stays a hop
count, because that is what it is, and it is multiplied by the same hop
duration.

The speed is the configured maximum rather than the fastest fitted plate. Every
fitted speed is clamped to that maximum, so it is the fastest plate a world can
hold and not an approximation of one; reading it means the prior does not
depend on the fit at all, a world whose fit came out slow does not re-date its
whole ocean, and there is no motionless case to fall back from. The maximum was
validated finite, non-negative, and above the minimum, which let a configured
maximum of exactly zero through; it is validated positive now, because a world
in which no plate can move leaves every stage that measures a length against a
plate speed with nothing to divide by.

The config is the one input to the prior that is not a stage output, so the
prior holds it to the kinematics stage's own rules rather than taking it on
trust: `motion::validate_config` is `pub(crate)` and the prior calls it, so a
raw config with a zero maximum is an error at the door instead of a NaN in
every ridge cell. That gives the prior a stage error enum of its own,
`CrustBirthPriorError`, wrapping `PlateKinematicsError` and `StageInputError`
the way the sibling stages wrap theirs. It had returned `StageInputError`
directly, which no config rule can travel through.

`PlateEvolution` carries `elapsed_time`, and `derive_seafloor_age(mesh,
evolution)` has dropped the `elapsed_steps` argument a caller had to keep in
agreement with the run. `BaseElevationConfig::cooling_age` is `0.56` model
time, `OceanicPeakFieldConfig::maximum_young_age` is `0.14`, and
`TerrainControlConfig::abyssal_age_saturation` is `0.112` — the same forty,
ten, and eight default steps they were. The oceanic-peak hill strength is now
`1 - age / maximum_young_age` for `0 < age <= maximum_young_age`, the
continuous form of the integer ramp it replaced.

### The trench rule reaches as far as a step travels

`subducts` looked for a convergent edge from the landing cell to a cell the
loser's plate owned, one hop out. Measured at the viewer's defaults, the
fastest plate moves 1.01 cell widths per step at the start, 1.52 once drift has
taken it to the band edge, and 1.85 with a rift opening added. A particle that
landed two cells deep found no such edge and stacked instead of subducting: at
the end of a fifteen-step run 1,245 oceanic particles sat in cells another
plate owned, where the rule intends only transform and divergent jitter to
survive there.

`TRANSPORT_REACH_HOPS` is now the one fact the empty-cell search, the gap
radius bound, and the trench rule are all stated against. `subducts` asks
whether the loser is oceanic, of another plate than the winner, and whether
within that reach there is a convergent edge whose far cell the loser's plate
owned before the step; it reuses the ring the empty-cell search builds rather
than walking a second one.

Measured before and after: at the viewer's defaults stranded oceanic particles
fall from 1,245 to 0 and subducted particles rise from 12,068 to 15,404; on the
reference fixture they fall from 29 to 0 and rise from 127 to 195. The rise
exceeds the fall because a particle subducted on the step it arrives is not
there to stack on any later step: owner changes fall from 42,722 to 37,860 and
collided cells from 20,896 to 9,608 at the viewer's defaults.

### The step is bounded

`maximum_step_duration(maximum_angular_speed, radius, cell_width, config)`
lives in `transport.rs` beside `TRANSPORT_REACH_HOPS`, because it is that reach
restated as a time and the two have to move together. It returns
`TRANSPORT_REACH_HOPS * cell_width / ((maximum_angular_speed *
(1 + speed_drift_limit) + rift_opening_speed) * radius)`, and
`evolve_plate_ownership` rejects a longer step with
`PlateEvolutionError::StepOutrunsReach`, using the largest `|omega|` its inputs
hold. Beyond that reach a step is not a coarser version of the same run:
material jumps trenches without subducting, gaps open that no search can fill,
and deformation is painted at boundary positions the plates left partway
through the step.

At the viewer's defaults the bound is `2 * 0.0138 / 1.83 = 0.01513`, so `0.014`
passes with eight percent of the reach to spare. The viewer's step slider takes
its top from the same function, evaluated with the kinematics config's
`maximum_angular_speed` because the plates are not fitted until the phase runs;
it used to stop at a fixed ten times the default, which moved ten cells a step.
The test-support step for small meshes already scales by root cell count and
stays under the bound, and
`the_test_meshes_keep_their_step_inside_the_transport_reach` asserts it.

### What this does not fix

A moving world at `(step_duration / 2, 2n)` is not the world at
`(step_duration, n)`. Cells resolve twice as often, so which particle wins a
contested cell and when a gap opens can differ, and the half-angle tangent
differs in the fourth decimal between one turn and two half turns. What is
identical is the meaning of every configured quantity; the still-world test in
"Determinism" is the exact form of that claim.

Measured at the viewer's defaults, `(0.014, 15)` against `(0.007, 30)`:
continental cells 18,466 against 18,429, land 16,887 against 16,594, born
particles 4,851 against 4,887, subducted particles 15,404 against 16,199, and
stranded oceanic particles 0 against 2. Continental particles are 19,365 at
both, exactly, by construction. The oldest seafloor age is 0.54234 at both, to
the bit: it belongs to crust the prior dated, which no slicing touches.

`rift_rate * step_duration` is a per-step probability of a Poisson process and
already scales correctly for small products. `suture_time` and
`full_deformation_time` are already model time. None of these changed.

### Pins that moved

Ownership on the reference fixture, from the drift retune and the trench reach.
Every birth and age pin, from the type change, re-pinned through
`quantized_fingerprint` over the time field: the 1/1024 grid resolves a step of
0.014 and a hop of 0.0138 comfortably. Base elevation and the oceanic-peak
fingerprint moved from their age input; the oceanic-peak fixture builds a
synthetic `SeafloorAge` and changed unit with it. The geology and climate pins
that read synthetic elevation fields did not move.

Nothing moved from taking the hop duration off the configured maximum speed
rather than the fitted plates: the fit clamps to that maximum, and both the
reference world and the viewer's own hold a plate at it.

Two test fixtures were retuned rather than re-pinned. The reference
base-elevation config now scales `cooling_age` to the 512-cell mesh the way
`reference_evolution_config` already scales the run that feeds it: a hop there
is eleven times the default mesh's, so without it four fifths of the reference
ocean would sit flat on the deep floor and the curve those pins hold would be a
constant. Two viewer fixture seeds moved to ones whose climate coupling
converges on the new elevation field, which the fixture comment already says is
per cell count.

## Island arcs

Ocean-ocean convergence looked like nothing on Earth: `boundary_source` gave
both sides of the edge the symmetric `convergent` belt, a 1,000 km swell on
each side of the trench and no trench, and `collect_boundary_data` skipped
every convergent edge whose two cells shared a class, so no arc could exist
over ocean floor. The Marianas, the Aleutians, Tonga, and the Lesser Antilles
are the opposite shape: a trench on the older plate, a narrow volcanic arc one
or two cells behind it on the younger plate, and islands where that arc breaks
the surface.

### One precedence rule, stated once

`material_order` in `crust.rs` says which of two parcels of crust covers the
other: continental over oceanic, and among oceanic the younger over the older.
`Equal` is two continents, or two floors of one age, and means no polarity.
The arguments are birth times as `CellCrust` stores them, so `None` is
continental crust nothing re-made and outranks any ocean floor;
`CellCrust::order` is the same rule over two cells.

Material transport already settled the polarity — `occupant_order` ranked
continental over oceanic and the younger floor over the older — so this
extracts what transport was doing rather than deciding anything new.
`occupant_order` is now `material_order`, then the incumbent term, then
nearness, then index, which is the same total order it had; the proof is that
no ownership or birth pin moved. Deformation and the volcanic arcs read the
same function, so the three stages cannot disagree about who is on top.

### Ocean-ocean deformation takes a side

`boundary_source` reads the order over the two cells' crust rather than their
classes. For a convergent edge:

| pair | side | profile |
| --- | --- | --- |
| continental over oceanic | continental | `collision` |
| | oceanic | `trench` |
| oceanic over oceanic | younger | `island_arc` |
| | older | `trench` |
| `Equal` | both | `convergent` |

`convergent` is thereby the no-polarity profile: the continental collision
belt, and the tie. `trench` is the older oceanic side of any convergent
boundary that has a polarity, whether a continent or a younger floor stands
over it. Divergent and transform edges do not read the order: a rift is a
property of the crust on the side, and a transform's relief is its residual
normal component whatever lies across it.

`island_arc` defaults to `offset: 0.4, depth: 2`. An arc is narrow — a
volcanic front 100 to 200 km behind the trench — so two hops, not the six a
collision belt spreads over. The offset matches `convergent` so that a floor
at 0.08 to 0.30 reaches the 0.5 datum once the boundary has held for the whole
of `full_deformation_time` and the volcanic uplift lands on top. That is what
makes an arc an island chain rather than a submarine ridge, and it is the
number to retune if arcs stay drowned.

### Arcs on an oceanic overriding plate

`collect_boundary_data` claims the overriding cell of every convergent edge,
which is the one `material_order` ranks greater. `Equal` skips the edge, which
keeps continental collisions arc-free as they are on Earth and skips two
floors of one age. The mixed-crust case resolves exactly as it did.

Grouping and the inland walk stay on one crust class as well as one plate, so
a segment has one `ArcKind`: a continental arc still stops at the coast, and
an island arc stops where the overriding plate's floor meets its own continent
or another plate. `VolcanicArcDiagnostics` counts island segments and their
arc cells, and the viewer's arc summary shows both.

Composition does not change. `volcanic_arc_uplift` applies to every arc cell
whatever its kind, terrain controls read the arc field and not the kind, and
cratons and basins read continental land and ignore islands, which is right.
An island arc cell above the datum is oceanic land: it gets no margin taper,
so it stands as a cliff, which is what a volcanic island does.

### Measured

Both worlds are 65,536 cells: the viewer's defaults at sampling seed 7, jitter
0.8, and 15 steps, and the reference world at sampling seed 9, subdivided
faces 0.2, and 30 steps. "Before" is the pipeline as it stood at "Time and
length units".

| Measure | Defaults, before | Defaults, after | Reference, before | Reference, after |
| --- | --- | --- | --- | --- |
| Ocean-ocean convergent edges, polarised | 3,978 | 3,978 | 1,640 | 1,640 |
| Ocean-ocean convergent edges, tied | 536 | 536 | 53 | 53 |
| Arc segments, continental | 115 | 115 | 35 | 35 |
| Arc segments, island | 0 | 434 | 0 | 173 |
| Island arc cells | 0 | 2,623 | 0 | 1,247 |
| Island arc cells above the datum | 0 | 159 | 0 | 103 |
| Land cells at 0.5 | 16,887 | 16,328 | 17,975 | 17,316 |
| Tectonic minimum | 0.0692 | 0.0632 | 0.0000 | 0.0000 |
| Tectonic maximum | 0.9488 | 0.9480 | 1.0000 | 1.0000 |
| Tectonic mean | 0.3635 | 0.3406 | 0.3160 | 0.2887 |
| Deformation minimum | −0.0618 | −0.0699 | −0.0404 | −0.1439 |
| Deformation maximum | 0.5000 | 0.5000 | 0.5000 | 0.5000 |
| Deformation mean | 0.0805 | 0.0575 | 0.0675 | 0.0402 |
| Affected cells | 61,342 | 59,512 | 39,913 | 35,581 |
| Oceanic cells | 47,070 | 47,070 | 46,446 | 46,446 |
| Final plates | 99 | 99 | 13 | 13 |

Islands exist. 159 island arc cells stand above the datum at the defaults and
103 at the reference world, out of 2,623 and 1,247 arc cells: a sparse chain
of volcanic islands over a mostly submarine ridge, which is the shape of a
real arc. The island arc offset is left where it is, and no island-specific
uplift is added.

Ownership does not move. Oceanic cells and final plates are identical at both
worlds, which is the check on the `occupant_order` rewrite, and the
continental arc segment count is identical too, which is the check that the
mixed-crust case resolves as it did.

Nine tenths of the ocean-ocean convergence at both worlds has a polarity —
3,978 edges against 536 at the defaults — so almost every ocean-ocean
boundary now has a side. The 434 island segments at the defaults are short,
about six arc cells each, because a segment ends wherever the overriding
plate's floor ends, and the default world's ocean is cut into 99 plates.

Land falls by about 3.5 percent at both worlds, and mean tectonic elevation by
0.023 and 0.027. That is the symmetric swell going away: an ocean-ocean
boundary used to raise a 0.4 belt six hops into *both* plates and now raises
0.4 over two hops into one and −0.2 over one hop into the other. Deformation's
mean falls with it and its affected cells fall by 1,830 and 4,332, because the
arc reaches a third as far as the belt it replaced. The negative tail deepens
at the reference world — −0.14 against −0.04 — which is the trench appearing
where a swell used to be, on a world whose long run gives its trenches time to
saturate.

### Pins

None moved. Deformation is never pinned. The volcanic arc reference
fingerprint stands: that fixture's ocean is one age everywhere, so it has no
ocean-ocean convergence with a polarity and builds no island arc. Ownership
and birth pins stand, which is the check on the one precedence rule. The
geology elevation and isostasy pins read synthetic fields and do not move.

## Slab pull

`plate_speed` was `hashed base × crust factor × size factor`, clamped to
`[minimum_angular_speed, maximum_angular_speed]`, where the size factor was
the fourth root of the mean plate area over the plate's own. Two things were
wrong with it.

The size factor is not supported by the data. Forsyth and Uyeda (1975) found
no correlation between plate area and speed; the Pacific is the largest plate
and among the fastest. What does correlate is negative with continental area,
which the crust factor already has, and strong and positive with the fraction
of the plate's perimeter that is subducting slab: Nazca and Cocos, half
trench, move at 8 to 10 cm/yr where Eurasia, Antarctica, and Africa, with no
slab to speak of, move at 1 to 2.

And the spread was too narrow. Base speeds span a factor of two and the lower
clamp at 0.5 held every plate above half the maximum. Earth's span a factor of
ten. Slab pull is what produces that spread, and it is also the one feedback
the model can add cheaply: a plate that starts subducting speeds up, which is
the Farallon story, and a plate that loses its trench slows down.

### The speed rule, stated once

`motion.rs` exports

```
pub fn plate_speed(
    base: f32,                 // the plate's hashed base speed
    continental_fraction: f64, // area share, as `plate_continental_fraction` gives it
    subducting_fraction: f64,  // perimeter share, below
    config: PlateKinematicsConfig,
) -> f32
```

which is `base × crust_speed_factor × slab_speed_factor`, clamped only from
above, to `maximum_angular_speed`. There is no lower clamp: a plate with no
slab and a continent on it is slow, and that is the point.
`minimum_angular_speed` and `maximum_angular_speed` stay the range the hashed
base is drawn from, and the maximum stays the ceiling the step bound reads;
the minimum is no longer a floor on the result.

`slab_speed_factor(fraction)` is
`trenchless + (1 − trenchless) × min(1, fraction / saturation)`, with two new
config fields: `trenchless_speed_factor`, default 0.25, the share of its base
a plate with no slab keeps; and `slab_saturation_fraction`, default 0.4, the
perimeter share at which slab pull is fully felt, which is about the Pacific's
trench share. Both are validated finite and in `(0, 1]`. Together with the
crust factor and the hashed base this allows a spread of about twelve —
`0.25 × 1.0 × 0.5` against `1.0 × 1.5 × 1.0` before the ceiling — where
Earth's is ten. The measurements below say what a real world reaches, which is
less, because no plate of these worlds is half trench.

### The subducting fraction

`subducting_fractions(mesh, partition, crust, boundaries)` in `boundaries.rs`
counts, per plate and over the edges whose two cells it owns one of and
another plate owns the other, how many are `Convergent` with `material_order`
ranking the plate's own cell `Less` — its floor is the slab — and divides by
the count of all of them. Two integer counts per plate in one pass over the
edges, so the fraction is exact. `Equal` edges are not slab: two floors of one
age, or two continents, pull neither plate. A plate with no boundary edge, which
compaction guarantees does not exist, would read zero.

`boundaries` need not be the classification of the current ownership. Evolution
passes the boundaries its step began with over the ownership transport has
since moved, which is the Jacobi convention every other substep keeps.

### The kinematics stage uses it once

`generate_plate_kinematics` fits directions as before, draws the hashed base,
and then needs a classification to know the fractions — which needs speeds. It
makes one extra pass: speeds at `base × crust factor`, classify, take the
fractions, then the final speeds through `plate_speed`. Only the lengths change
between the two passes, so the boundary edges the fractions counted are the
same edges either way, though a few of them change regime; the classification
a run starts from is the caller's, taken from the motion the stage returns, so
the birth prior and evolution's first step both see the slab-pulled world.

Before step zero no ocean floor has an age — the birth prior runs *after* this
stage — so an ocean-ocean convergence ranks `Equal` and only a continent
standing over floor is slab. At the viewer's defaults that leaves 35 of 111
plates with a nonzero fraction and a mean of 0.003, so almost every plate
starts near the trenchless quarter of its base. Evolution's own per-step
recomputation reads the aged birth field and sees the rest, which is why the
measured spread widens over a run rather than starting wide.

### Evolution recomputes speed every step

Speed is per-step state derived from the world, and drift is a multiplier on
it rather than a walk of the speed itself.

- `PlateKinematics` gains `base_speeds`, the hashed draw each plate's speed is
  a multiple of, so evolution does not re-derive it. `EvolvingWorld` keeps
  `drift_factors` in place of `starting_speeds`; both start at one.
- `PlateEvolutionInputs` gains `kinematics_config`, the config the motion was
  fitted under. `evolve_plate_ownership` validates it with the kinematics
  stage's own `validate_config`, and `maximum_step_duration` reads its maximum
  in place of the fastest fitted plate, so the viewer and evolution now bound
  the step by the same number.
- **Drift.** The axis turn is unchanged. The speed walk becomes
  `drift_factor *= 1 + s × speed_drift_rate × sqrt(step_duration)`, clamped to
  `[1 − speed_drift_limit, 1 + speed_drift_limit]`.
- **Respeed**, a new substep between lifecycle and reclassification, sets
  `|ω| = plate_speed(base, continental_fraction, subducting_fraction, config)
  × drift_factor` for every plate, with the continental fraction from the
  current cell crust and ownership and the subducting fraction from the
  boundaries the step began with over the current ownership. Direction is
  preserved; only the length is set, and a plate whose rotation vector is zero
  has no direction to keep and stays at rest.
- **Lifecycle.** A rift's halves inherit the parent's base speed and drift
  factor; the opening is added to the rotation vector as before and the next
  respeed sets the length, so `rift_opening_speed` decides which way a half
  parts and the rule decides how fast. A suture takes the area-weighted mean
  of the two bases and of the two drift factors as well as of the two rotation
  vectors. Compaction remaps both.
- The step order is deform, transport, drift, lifecycle, respeed, reclassify.

### Measured

Both worlds are 65,536 cells: the viewer's defaults at sampling seed 7, jitter
0.8, and 15 steps, and the reference world at sampling seed 9, subdivided
faces 0.2, and 30 steps. "Before" is the pipeline as it stood at "Island
arcs".

| Measure | Defaults, before | Defaults, after | Reference, before | Reference, after |
| --- | --- | --- | --- | --- |
| Start speed minimum | 0.500 | 0.134 | 0.668 | 0.162 |
| Start speed median | 1.000 | 0.247 | 1.000 | 0.249 |
| Start speed maximum | 1.000 | 0.381 | 1.000 | 0.386 |
| Start maximum over minimum | 2.00 | 2.84 | 1.50 | 2.38 |
| End speed minimum | 0.272 | 0.108 | 0.535 | 0.183 |
| End speed median | 0.876 | 0.388 | 1.034 | 0.379 |
| End speed maximum | 1.500 | 0.844 | 1.460 | 0.695 |
| End maximum over minimum | 5.52 | 7.79 | 2.73 | 3.79 |
| Rank correlation, fraction against speed | 0.313 | 0.629 | −0.236 | 0.745 |
| Rank correlation, area against speed | −0.289 | −0.137 | −0.110 | −0.155 |
| Convergent edges | 6,539 | 5,525 | 2,252 | 1,401 |
| Divergent edges | 6,928 | 5,978 | 2,220 | 1,627 |
| Transform edges | 6,109 | 4,983 | 2,101 | 1,235 |
| Land cells at the datum | 16,328 | 16,694 | 17,316 | 16,920 |
| Born particles | 4,851 | 1,339 | 4,036 | 1,519 |
| Subducted particles | 15,404 | 8,019 | 10,490 | 5,193 |
| Rifts | 2 | 3 | 2 | 3 |
| Sutures | 14 | 14 | 7 | 10 |
| Continental particles | 19,365 to 19,365 | 19,365 to 19,365 | 19,358 to 19,358 | 19,358 to 19,358 |
| Final plates | 99 | 100 | 13 | 11 |

The spread is there and it comes from the trenches. The end-of-run rank
correlation between subducting fraction and speed goes from 0.31 to 0.63 at
the defaults and from −0.24 to 0.75 at the reference world — it had the wrong
sign before — while the correlation with plate area falls to −0.14 and −0.16,
which is the size factor going away. The end-of-run spread widens from 5.5 to
7.8 and from 2.7 to 3.8.

Every plate is slower. The start-of-run median falls by four, which is the
trenchless factor applied to a world whose ocean has no ages yet, and the
end-of-run median by a bit over two. Subducted particles fall by about half
and born particles by about two thirds at both worlds, because a slower world
opens fewer gaps and overrides fewer cells. Boundary edges fall by about a
sixth at the defaults and a third at the reference world, for the same reason:
less migration leaves plates in fewer pieces. Continental particles are exact
either side, as they must be.

Land moves by about two percent in opposite directions at the two worlds —
up 366 cells at the defaults and down 396 at the reference — which is
deformation following the boundaries rather than any rule about land.

A sixty-step run at the defaults, the speed and subducting fraction of the two
fastest and two slowest plates at every tenth step:

| Step | Fastest | Slowest |
| --- | --- | --- |
| 0 | 0.381 at 0.09, 0.374 at 0.11 | 0.135 at 0.06, 0.134 at 0.00 |
| 10 | 1.023 at 0.17, 0.802 at 0.16 | 0.128 at 0.02, 0.112 at 0.00 |
| 20 | 0.950 at 0.15, 0.844 at 0.12 | 0.122 at 0.04, 0.103 at 0.02 |
| 30 | 1.092 at 0.28, 0.993 at 0.14 | 0.088 at 0.01, 0.077 at 0.01 |
| 40 | 0.940 at 0.17, 0.860 at 0.15 | 0.143 at 0.07, 0.133 at 0.07 |
| 50 | 0.956 at 0.15, 0.909 at 0.17 | 0.131 at 0.08, 0.120 at 0.03 |
| 60 | 0.971 at 0.21, 0.897 at 0.14 | 0.136 at 0.10, 0.116 at 0.02 |

The fastest plates carry two to ten times the trench share of the slowest at
every sample, and the fast end is eight times the slow end from step ten on,
against a factor of three at step zero. The same run before the change had
its fastest plates pinned at 1.500, the drift band's ceiling over a speed the
run could not otherwise change, with trench shares of 0.00 to 0.21 — no
relation at all.

### What this deliberately does not do

- No plate-boundary force balance, no ridge push, no basal drag. Slab pull is
  a multiplier on a hashed base, not a force. The fit to the flow field still
  sets direction.
- No response time. A plate's speed follows its trench share within one step.
  Real plates respond over a few million years, which is one to three steps;
  if the measurements show speeds flickering with the fraction, a relaxation
  is the follow-up, not part of this slice.
- No change to the crust factor, the coherence blend, the flow field, or
  `maximum_angular_speed`.
- `maximum_step_duration` still reserves `rift_opening_speed` on top of the
  ceiling. The respeed now clamps a rift's halves back inside the ceiling
  within the same step, so that reserve is only spent by step zero's transport,
  which reads the motion the caller supplied. Removing it would loosen the
  bound and is a separate change.

### Pins

Every ownership, birth, seafloor-age, and base-elevation pin moved: speeds
changed for every plate. Each was re-pinned once. The reference run's
aggregates moved with them — owner changes 396 to 134, subducted particles 195
to 101, born particles 29 to 1, plates 26 to 24, rifts 3 to 2 with two that
separated nothing, sutures 4 to 9 — all of which is a slower world overriding
less and colliding for longer.

The volcanic-arc reference fingerprint moved too, against the expectation that
it would not: that fixture fits its own plate motion before building its
boundaries, so the speed rule reaches it. Its arcs are still all continental
and its island counts still zero. The geology and climate pins that read
synthetic fields did not move. One viewer cache fixture had to change seed:
climate coupling does not reach a fixed point on every 32-cell world, and seed
30 stopped converging under the new tectonics.
