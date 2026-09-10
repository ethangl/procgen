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
  the slice 2 and 3 figures above. Mesh jitter 0.5 to 0.8; crack arcs 40 to
  16, curvature 2 to 8, and subdivided faces 0.4 to 0.8, which together give
  111 plates against 57 from fewer, larger crack faces split more often;
  oceanic speed factor 1.4 to 1.5 and continental 0.7 to 1.0, so crust class
  separates speeds less; evolution steps 9 to 15 and minimum convergence 0.3
  to 0.5; and the convergent profile's depth 3 to 6, for wider belts.
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
- Rifting: each step, every continental plate above
  `rift_minimum_area_fraction` of the sphere draws against
  `rift_rate * step_duration` on a `PLATE_RIFT` stream. A plate that rifts
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
  migration empties as well as the ones suturing does — migration could
  already wipe out a plate before this slice — so plate ids are no longer
  stable across a run, and a run's output plate count differs from the
  partition stage's in both directions.
- `PlateLifecycleConfig` sits under `PlateEvolutionConfig` beside
  `PlateMigrationConfig` and `PoleDriftConfig`. A `rift_rate` of zero and a
  `suture_time` of infinity each disable their event; `test_support`'s
  `NO_LIFECYCLE` is both, as `NO_POLE_DRIFT` is for drift.
- Defaults: `rift_rate` 4.5 per unit time, `rift_minimum_area_fraction` 0.04,
  `rift_curvature` 8.0 (the partition's own), `rift_opening_speed` 0.33,
  `suture_time` eight default steps, and `suture_minimum_shared_edges` 20.
  They were calibrated by running the viewer's defaults over nine, fifteen,
  and thirty steps and sweeping the two knobs that matter. The area fraction
  is one of them: the largest continental plate at the viewer's defaults
  covers 0.044 of the sphere, so 0.05 makes nobody eligible and 0.04 makes one
  or two. The rift rate saturates above three or so, because both halves of a
  rift fall below the minimum area and cannot rift again. The shared-edge
  count is the other: at eight the viewer's defaults suture six times over
  fifteen steps, at sixteen three times, and at twenty once. The opening speed
  is a third of the default maximum angular speed, which clears the default
  minimum convergence of 0.5 across the rift on its own.
- At the viewer's defaults a fifteen-step run produces 1 rift, 0 failed rifts,
  1 suture, and 111 final plates, from the 111 the partition made: the rift
  and the suture cancel. Boundary edges after the run are 5303 convergent,
  5869 divergent, and 5269 transform, against 5326, 5896, and 5326 with the
  events disabled, and crust-creation events 6426 against 6369, over 18,190
  migration events against 18,107. A nine-step run gives 1 rift and 1 suture;
  thirty steps give 1 rift, 1 failed rift, and 3 sutures. The events are a
  small perturbation of the aggregate at these defaults, which is the point:
  they change the plate set's history rather than its statistics.
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

Two things came out differently from the intent above. Plate ids stop being
stable across a run: compaction has to renumber, and migration could already
empty a plate on its own, so the rule that every id owns a cell forces the
renumbering on every run rather than only on one that sutured. And a rift arc
is only well posed on a plate an arc can cross rather than circle, because a
closed arc's wall has its mean center at the sphere's own, which leaves the
opening direction undefined.

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

Slice 5 adds the arc walk, connected components, integer edge tallies, and
area sums, all of which are exact, plus one plane fit and one cross product
that use add, multiply, and square root alone. The floats that decide an
integer there are area comparisons, which every stage that weights by area
already depends on. Nothing on the lifecycle path calls libm, so the plate ids
it produces and the boundary classes the new rotation vectors decide stay
bit-identical across the two development machines.

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
drown a continent on its own: `0.65 - 0.03 - 0.05` is 0.57, above the 0.5 of
`SEA_LEVEL`. That arithmetic is nominal rather than a bound, because the
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

Carried risk. The 65,536-cell mesh has about 180 cells whose Voronoi corner
ring is locally inverted — 69 of them with no jitter at all, so it is the f32
circumcenter of a near-degenerate Fibonacci-lattice triangle rather than the
jitter. No boundary integral over an inverted ring can be right, and those
cells are where the divergence field's peak of 4.6 RMS comes from: on a
4096-cell mesh, which has none, the worst cell of a rigid-rotation field
measures 0.04 against an exact zero. The visible effect is a few dozen isolated
cells carrying up to 0.14 of dynamic topography instead of the 0.03 the field
around them carries. Fixing it belongs in the triangulation, not here.

Erosion is the next stage, and it is the reason this one exists: it needs
slopes to move material down, and until now a plate interior had none.
