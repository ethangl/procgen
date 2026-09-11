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
- `classify_boundaries` derives per-edge normal and shear speeds from the two
  owners' rotations and classifies each edge. It is correct and stays.
- Evolution carries the material itself across steps, as one particle per
  parcel of crust holding its birth step and its accumulated deformation, plus
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
  the birth field evolution starts from: `Some(-hops)` for oceanic cells,
  `Some(-ridge_less_age)` for ridge-less oceanic plates, `None` for continental.
  `derive_seafloor_age` is `step_count - birth`, in steps rather than hops.
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
  step state.
- Kinematics is per-step state inside the evolving world rather than a fixed
  input. Every step ends, after deform and transport and before the
  next classification, by drifting each plate's rotation vector: the axis
  turns through the fixed angle `axis_drift_rate * step_duration` toward a
  fresh hashed direction perpendicular to it, and the speed is multiplied by
  `1 + s * speed_drift_rate * step_duration` for a hashed `s` in `[-1, 1)`.
  Four `signed_f32` draws per plate per step, from a `PLATE_POLE_DRIFT`
  stream on evolution's own new seed, supply the direction and the speed.
  Because the angle per step is fixed and only the direction is hashed, an
  axis takes a random walk on the sphere of directions, whose expected total
  wander over `n` steps is roughly `theta * sqrt(n)`.
- The drifted speed is bounded to `speed_drift_limit` either side of the
  speed the plate started the run with, not to the kinematics config's global
  angular-speed range. A random walk against fixed global limits eventually
  piles every plate against one of them; a band around the fitted speed keeps
  drift the perturbation of the flow field's answer that it is meant to be,
  and evolution needs nothing from the kinematics stage but its result.
- `axis_drift_rate` defaults to 15.0 and `speed_drift_rate` to 7.5, both per
  unit time, and `speed_drift_limit` to 0.5. At `DEFAULT_STEP_DURATION` the
  first turns an axis 0.21 radians a step, so `0.21 * sqrt(15)` is about
  forty-seven degrees of expected wander over a default fifteen-step run, and
  the second changes a speed by at most about a tenth a step, whose expected
  fifteen-step excursion is about a quarter. A half therefore bounds the tail
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
`signed_f32` as the crack walk already uses. Birth steps and accumulated displacement are
integers or exact multiples of the step length. Accumulated deformation is a
float field and is tested for run-to-run equality and invariants only, never
pinned across machines.

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
- Fit to a field rather than simulate forces. Slab pull and ridge push would
  need a mantle model; a smooth field with crust and size factors gets the
  large-scale pattern at a fraction of the complexity.
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
  drift retune made. The symmetric uplift at ocean–ocean convergence belongs
  to the island-arc item.
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
- Ocean–ocean polarity: the older floor subducts, which is the island-arc
  item's first half.
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
