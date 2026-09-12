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

Every measurement in this document down to "Run length" was taken at a viewer
default of 15 evolution steps; that section moves the default to 60, and every
measurement from it on names its own step count in the table caption. Every
measurement down to "Relief decay" was also taken without a sink under
deformation, and every one down to "Crustal thickness" with a collision
stacking crust rather than merging it; each of those sections supersedes the
magnitudes before it.


All five slices — coherent kinematics, displacement-proportional migration,
accumulated deformation, drifting Euler poles, and plate lifecycle — have
landed, and so has the interior relief that follows them; see the last section
for that one.

- `generate_plate_kinematics(mesh, partition, crust, config)` fits each plate's
  Euler vector to a smooth global flow field over the plate's own cells, then
  scales the fitted direction by `crust_scaled_speed`: the plate's hashed base
  speed times a crust factor, clamped to `maximum_angular_speed`. A
  `coherence` fraction blends the axis back toward the hashed random one.
  Plates too small to fit — one or two cells — keep the hashed axis; the speed
  rule is the same for every plate. Nothing on the path uses a libm call, so
  the integer boundary classes the angular velocities decide stay exact.
- That is not the speed a plate turns at. `plate_speed` in `speed.rs` is, and
  it reads the share of a plate's perimeter that is subducting slab, which no
  stage can answer until the crust-birth prior has dated the ocean. Evolution
  applies it: once before step zero and once at the end of every step. See
  "Slab pull" below for the rule, the ordering it forces, and the size factor
  it replaced.
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
  `Some(-hops * hop_duration)` for oceanic cells, `Some(-ridge_less_age)` for
  ridge-less oceanic plates, `None` for continental. `derive_seafloor_age` is `elapsed_time - birth`. See "Time
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
- That carried field decays. Every step multiplies what a parcel holds by
  `1 - step_duration / erosion_time` before the step's boundaries add to it,
  so relief with no boundary under it falls toward zero and relief under one
  rises to where uplift and decay balance. `erosion_time` defaults to thirty
  default steps; infinity turns the sink off, the same convention
  `suture_time` uses. See "Relief decay".
- A parcel also carries its thickness, the original parcels it holds. A
  continent arriving under another continent at a trench merges into it,
  thickness then flows from a column to its thinner same-plate neighbours
  until no pair differs by more than one parcel, and
  `BaseElevationConfig::thickness_uplift` floats the column that results. See
  "Crustal thickness".

- The deformation config moved into `PlateEvolutionConfig`, beside
  `PlateMigrationConfig`, because it is now a substage of a step rather than a
  stage of its own.
 `PlateEvolution` returns the accumulated
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
  edges that are convergent with continental crust on both sides. A pair whose
  front is at least `suture_minimum_shared_length`, converted to an edge count
  against the mesh, grows its collision time by the step; a pair below it
  starts over, the same convention the closing debt uses for an
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
  `suture_time` eight default steps, and `suture_minimum_shared_length` twenty
  default hops.
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
- Every reach a stage walks is a model length on the unit sphere, converted to
  hops once at the top of the stage through `procgen_sphere_mesh::hops`, so a
  belt is as wide in kilometres on a fine mesh as on a coarse one. Renamed with
  their unit: `BaseElevationConfig::margin_width_hops` to `margin_width`,
  `CoarseElevationConfig::smoothing_passes` to `smoothing_radius`,
  `VolcanicArcFieldConfig::inland_offset_cells` to `inland_offset`,
  `HotspotFieldConfig::maximum_trail_cells` to `maximum_trail_length`, and its
  `province_radius_hops` and `province_rim_hops` to `province_radius` and
  `province_rim`. `BoundaryEffect::depth`,
  `ContinentalRiftProfile::decay_depth`, `CratonFieldConfig`'s two distances,
  and `IsostaticAdjustmentConfig::maximum_boundary_distance` keep their names
  and change type. Every default is written as a multiple of
  `default_hop_length`, the width of one cell of the 65,536-cell default mesh,
  so each converts back to the integer it replaced exactly and no pin on that
  mesh moves; a unit test per stage asserts that. `TRANSPORT_REACH_HOPS` and
  `gap_radius` stay in hops, because they describe the raster rather than the
  world; they are the only two that do.
- A stage that counts features states a density per unit area or a fraction of
  the sphere and measures the area it applies to, for the same reason.
  `PlateLifecycleConfig::suture_minimum_shared_edges` became
  `suture_minimum_shared_length` and
  `VolcanicArcFieldConfig::minimum_boundary_edges` became
  `minimum_boundary_length`, each converted to an edge count against the mesh,
  because a boundary's edges are its length in hops to within the mesh's
  irregularity. `SedimentaryBasinFieldConfig::minimum_cell_count` became
  `minimum_area_fraction` of the sphere and the stage sums the component's own
  cell areas. `VolcanicArcFieldConfig::peak_density_divisor` became
  `peak_density` per unit area, and a segment keeps `round(arc area x density)`
  peaks, never fewer than one: a segment long enough to survive the
  minimum-length filter is an arc, and an arc has a volcano on it.
  `CrustBirthPriorConfig::ridge_less_age` became model time, which is what
  every other age in the pipeline is; as a hop count it made a ridge-less
  plate's floor younger on a finer mesh, and its diagnostic summary changed
  unit with it. `growth_roughness` stays a percentage, which is a fraction
  rather than a count and was already resolution independent.
- A run is a duration and the step count follows from it.
  `PlateEvolutionConfig::step_count` became `run_duration`, with the count
  derived as the run over the step and `step_duration` still configured and
  still bounded by `maximum_step_duration`. A finer mesh takes the shorter
  step its transport needs and covers the same history in more of them, where a
  step count would have covered less of it. `MoistureTransportConfig`'s
  `step_count` and `step_seconds` became `simulated_days`, with both derived
  from the mesh: the count goes as the reciprocal of the cell width, because a
  step may not carry moisture past a cell. `PlateEvolutionConfig::with_steps`
  states a run as a product for the fixtures that pin what a given number of
  steps does. See "Resolution independence" below for what the three slices
  bought, measured.
- The oceanic-peak presence draw became a density per unit area: a cell holds a
  peak with probability `density x cell area / default cell area`, so a floor
  carries the same seamounts per square kilometre on any mesh. A cell of
  exactly the mean area draws what it drew and every other cell shifts by its
  own area over the mean. That is the one fingerprint this work moved: over the
  reference fixture's 201 candidates it loses four peaks and gains five.
- Measured on the default world, the three rules that now read real cell area
  change what it holds. That is the point of the change rather than a cost of
  it: a mesh's cells vary around the mean, so no area test can reproduce a cell
  count on one, and the area is the quantity that means the same thing on every
  mesh.
  - Arc peaks go from 2390 to 2259 over 635 segments, 217 of which change by
    one. `div_ceil` was the integer arithmetic that happened to be there and it
    rounded every segment up; once the quantity is peaks per unit area, the
    unbiased estimator is the only one under which the number means the same
    thing at any resolution. `ceil` measures 2530 and puts that per-segment
    bias back, and a default of one peak per 1.9 cells would bury it in a
    constant nobody could explain later. The floor at one peak keeps the only
    part of the old behaviour a reader would notice. If arcs look sparse, that
    is a measured retune of `peak_density` and not this change.
  - Basins go from 41 to 37 of 231 components. The four dropped are three-cell
    components covering less than three mean cells of area, which is what the
    threshold now says.
  - Oceanic peaks go from 1345 to 1326 of 8717 candidates.
  - No test fixture sits near any of those thresholds, so the pins that stand
    are not evidence that the default world is unchanged; these numbers are.
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
`province_radius` of the source, walking only through cells that
are continental and on the source's plate, so it stops at a coast and at a
plate boundary. The radius is a model length, converted to hops against the
mesh once; weight is one within `province_radius - province_rim` and falls
linearly toward zero at the radius: a flat top with a sloped
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

Defaults are `province_fraction` 0.4, `province_radius` five default hops, and
`province_rim` two. Five hops is about 440 km at Earth scale, so a plateau
spans roughly 900 km, whatever the mesh. `plateau_uplift` is 0.06, comparable to the Deccan's
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

- `convergent` 0.4 / depth 6 *(0.2 / 3 since "Crustal thickness")*,
  `collision` 0.5 / 5, `trench` −0.2 / 1. The

  Andean asymmetry is right and the trench is one cell as it should be. Depth
  6 puts every collision belt at about 1,000 km total width, Tibet scale,
  where 4 would be nearer the Andes and Alps; left as the visual choice the
  drift retune made. The symmetric uplift at ocean–ocean convergence is gone;
  see "Island arcs".
- `saturation_speed` 2.0, `full_deformation_time` nine default steps,
  `maximum_magnitude` 0.5. Time-unit questions, left for the time-consistency
  item. Two have moved since: with relief decay under it, 0.5 is the steady
  state of a belt that keeps converging rather than a bound an accumulator
  eventually hits, and "Crustal thickness" halves `convergent` to 0.2 over
  three hops because the plateau it used to paint is now made of material.

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
  yet. *Superseded by "Crustal thickness": the arriving continent now merges
  into the one above it and the column carries the count, so nothing stacks
  and base elevation reads the thickness.*

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
lives beside `TRANSPORT_REACH_HOPS`, because it is that reach restated as a
time and the two have to move together. It returns
`TRANSPORT_REACH_HOPS * cell_width / ((maximum_angular_speed *
(1 + speed_drift_limit) + rift_opening_speed) * radius)`, and
`evolve_plate_ownership` rejects a longer step with
`PlateEvolutionError::StepOutrunsReach`, using the largest `|omega|` its inputs
hold. Two details of this paragraph have since moved: "Slab pull" put the
function in `reach.rs` with the constants it is stated against, and made
evolution pass the kinematics config's own maximum rather than the fastest
plate its inputs hold, because every respeed clamps to that maximum. Beyond that reach a step is not a coarser version of the same run:
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

`speed.rs` owns the rule and `motion.rs` the fit that gives it a direction.
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


### Two stages, because the rule needs an age

The rule reads a subducting fraction, the fraction needs a classification, the
classification needs a motion — and the polarity of an ocean-ocean trench
needs the floor's *age*, which the crust-birth prior supplies and which the
prior cannot supply until it has a classification to find its ridges in. The
ordering is therefore fixed, and the kinematics stage is on the wrong side of
it:

1. `generate_plate_kinematics` fits each axis and scales it by
   `crust_scaled_speed` — the base speed times the crust factor, clamped to
   the ceiling. This is not the speed a plate turns at, and the stage's doc
   says so.
2. The caller classifies that motion.
3. `derive_crust_birth_prior` reads that classification and dates the ocean.
4. `evolve_plate_ownership` begins by respeeding over the prior's real ages
   and that classification — the same `respeed` it runs every step — and
   classifies once more. Step zero then moves a slab-pulled world.

An earlier draft of this slice had the stage do it all, by inventing a birth
field of one age for every oceanic cell so it could reach step 2 itself. That
is fabricated data and it showed: with every floor the same age, every
ocean-ocean trench ranks `Equal`, only 35 of 111 plates at the viewer's
defaults had any slab at all, the mean fraction was 0.003, and every plate
began the run at the trenchless quarter of its base. Giving evolution the
first respeed deletes the invented crust, the stage's second pass, and the
internal classification helper it needed, and it raises those numbers to 100
of 111 plates and a mean of 0.082.

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
- **Respeed** sets `|ω| = plate_speed(base, continental_fraction,
  subducting_fraction, config) × drift_factor` for every plate, with the
  continental fraction from the current cell crust and ownership and the
  subducting fraction from the boundaries the step began with over the current
  ownership. Direction is preserved; only the length is set, and a plate whose
  rotation vector is zero has no direction to keep and stays at rest. It runs
  once before step zero and then between each step's lifecycle and the
  reclassification that ends it.
- **Lifecycle.** A rift's halves inherit the parent's base speed and drift
  factor; the opening is added to the rotation vector as before and the next
  respeed sets the length, so `rift_opening_speed` decides which way a half
  parts and the rule decides how fast. A suture takes the area-weighted mean
  of the two bases and of the two drift factors as well as of the two rotation
  vectors. Compaction remaps both.
- The step order is deform, transport, drift, lifecycle, respeed, reclassify.
- A run of no steps is no longer the identity: it respeeds and reclassifies,
  which is the rule the stage could not apply. That is what its test now says.

### Measured

Both worlds are 65,536 cells: the viewer's defaults at sampling seed 7, jitter
0.8, and 15 steps, and the reference world at sampling seed 9, subdivided
faces 0.2, and 30 steps. "Before" is the pipeline as it stood at "Island
arcs". The start of a run is after its opening respeed, which is what a
zero-step run returns.

| Measure | Defaults, before | Defaults, after | Reference, before | Reference, after |
| --- | --- | --- | --- | --- |
| Start speed minimum | 0.500 | 0.134 | 0.668 | 0.156 |
| Start speed median | 1.000 | 0.404 | 1.000 | 0.437 |
| Start speed maximum | 1.000 | 1.000 | 1.000 | 0.698 |
| Start maximum over minimum | 2.00 | 7.45 | 1.50 | 4.48 |
| Start plates with any slab | — | 100 of 111 | — | 16 of 18 |
| Start mean trench share | — | 0.082 | — | 0.099 |
| End speed minimum | 0.272 | 0.096 | 0.535 | 0.190 |
| End speed median | 0.876 | 0.389 | 1.034 | 0.405 |
| End speed maximum | 1.500 | 0.879 | 1.460 | 0.670 |
| End maximum over minimum | 5.52 | 9.18 | 2.73 | 3.53 |
| Rank correlation, fraction against speed | 0.313 | 0.626 | −0.236 | 0.500 |
| Rank correlation, area against speed | −0.289 | −0.070 | −0.110 | −0.159 |
| Convergent edges | 6,539 | 5,593 | 2,252 | 1,742 |
| Divergent edges | 6,928 | 6,033 | 2,220 | 1,883 |
| Transform edges | 6,109 | 5,070 | 2,101 | 1,617 |
| Land cells at the datum | 16,328 | 16,732 | 17,316 | 17,048 |
| Born particles | 4,851 | 1,418 | 4,036 | 1,681 |
| Subducted particles | 15,404 | 8,127 | 10,490 | 5,848 |
| Rifts | 2 | 3 | 2 | 3 |
| Sutures | 14 | 12 | 7 | 8 |
| Continental particles | 19,365 to 19,365 | 19,365 to 19,365 | 19,358 to 19,358 | 19,358 to 19,358 |
| Final plates | 99 | 102 | 13 | 13 |

The spread is there and it comes from the trenches. The end-of-run rank
correlation between subducting fraction and speed goes from 0.31 to 0.63 at
the defaults and from −0.24 to 0.50 at the reference world — it had the wrong
sign before — while the correlation with plate area falls to −0.07 and −0.16,
which is the size factor going away. The spread widens from 2.0 to 7.5 at the
start and from 5.5 to 9.2 at the end.

Plates are slower on the whole: slab pull is a multiplier below one for every
plate short of saturation, and no plate of either world is half trench. The
median falls by a bit over two at both ends of both runs, subducted particles
by about half, and born particles by about two thirds. Boundary edges fall by
about a sixth, because a slower world migrates less and leaves plates in fewer
pieces. Continental particles are exact either side, as they must be.

Land moves by about two percent in opposite directions at the two worlds — up
404 cells at the defaults and down 268 at the reference — which is deformation
following the boundaries rather than any rule about land.

A sixty-step run at the defaults, the speed and subducting fraction of the two
fastest and two slowest plates at every tenth step:

| Step | Fastest | Slowest |
| --- | --- | --- |
| 0 | 1.000 at 0.16, 0.993 at 0.27 | 0.135 at 0.00, 0.134 at 0.00 |
| 10 | 1.080 at 0.18, 0.824 at 0.25 | 0.112 at 0.00, 0.110 at 0.01 |
| 20 | 0.947 at 0.15, 0.796 at 0.13 | 0.108 at 0.03, 0.095 at 0.02 |
| 30 | 0.918 at 0.14, 0.911 at 0.25 | 0.125 at 0.04, 0.081 at 0.02 |
| 40 | 0.922 at 0.16, 0.865 at 0.16 | 0.158 at 0.09, 0.156 at 0.12 |
| 50 | 1.128 at 0.24, 0.949 at 0.21 | 0.139 at 0.09, 0.131 at 0.06 |
| 60 | 1.168 at 0.27, 0.950 at 0.16 | 0.158 at 0.04, 0.120 at 0.05 |

The separation is there from step zero, which is what the opening respeed
buys: the two fastest plates carry trench shares of 0.16 and 0.27 and the two
slowest carry none at all, and the fast end is seven times the slow end. The
same run before the change had its fastest plates pinned at 1.500, the drift
band's ceiling over a speed the run could not otherwise change, with trench
shares of 0.00 to 0.21 — no relation at all.

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
  ceiling, and that reserve is now dead. A rift adds the opening to its
  halves' rotation vectors, but the respeed ending the same step sets their
  lengths back inside the ceiling, and a run respeeds the motion it was handed
  before its first step as well, so no transport reads a speed carrying it.
  Dropping the term would loosen the bound, which changes what step durations
  are legal and how far the viewer's slider reaches, so it is its own change.

### Pins

Every ownership, birth, seafloor-age, and base-elevation pin moved: speeds
changed for every plate. Each was re-pinned once. The reference run's
aggregates moved with them — owner changes 396 to 149, subducted particles 195
to 112, born particles 29 to 5, rifts 3 to 2 with two that separated nothing,
sutures 4 to 7 — which is a slower world overriding less and colliding for
longer. The crust-birth prior's own aggregates moved too, because it reads a
classification of the crust-scaled motion rather than of the old
crust-and-size one: every plate of the reference world now holds a ridge where
one used to hold none.

The volcanic-arc reference fingerprint did not move. That fixture fits its own
plate motion, so it would have moved under the draft that made the stage apply
the slab factor; with the stage stopping at the crust factor its boundaries
classify as they did. The geology and climate pins that read synthetic fields
did not move. One viewer cache fixture had to change seed: climate coupling
does not reach a fixed point on every 32-cell world, and seed 30 stopped
converging under the new tectonics.

### Files

`motion.rs`, `evolution.rs`, `lifecycle.rs`, and `transport.rs` each crossed a
thousand lines, so each gave up a concept it was only housing: `speed.rs`
takes the speed rule, its factors, and the spread summary; `evolution_error.rs`
takes the run's error enum; `rifting.rs` takes the half of the plate lifecycle
that creates a plate, leaving suturing and compaction behind; and `reach.rs`
takes the one ring count and the two bounds derived from it, which evolution
and the viewer both read.

"Crustal thickness" splits four more the same way. `thickness.rs` takes every
rule that changes what a column holds — accretion, the compaction merge, and
the lateral flow — and stands beside `rifting.rs` as the second module that
carries material through an event rather than across the mesh.
`boundary_sources.rs` takes the table of which profile each boundary class
raises, leaving `deformation.rs` the propagation and the carried field, and
`evolution_diagnostics.rs` takes the totals a run keeps, beside
`evolution_config.rs` and `evolution_error.rs`.


## Run length

Every default so far has been a short run — nine steps, then fifteen — and
nothing said whether that is a world or the first act of one. This section
runs the same two worlds over five lengths and one per-step series to find
out. Three things reach a steady state; one does
not, and it is the reason a default cannot simply be made long.

Both worlds are 65,536 cells. "Defaults" is sampling seed 7 and jitter 0.8.
"Reference" is the same with sampling seed 9 and subdivided faces 0.2, the
world of a dozen large plates used for judging continental interior relief.
Each row is a complete run of that length from the same inputs, not a snapshot
taken during one long run. Times are the tectonics phase alone, over a mesh
already built, in `--release` on an M1 Max.

Two columns are new. "Covered" is continental particles that no cell reads,
because another parcel of continent shares the cell with them; continental
material outranks every parcel of ocean floor, so the cells that hold some are
exactly the continental cells that material accounts for, and the covered
count is the gap between the material a run conserves and the raster it can
show. "Foreign" is the subset of those lying under a cell their own plate does
not own, which is what a collision stacked rather than what one plate's own
material crowded together. "Deepest stack" is the largest column of foreign
material over any one cell at any step, which the run already kept.

### At the viewer's defaults

Sampling seed 7, jitter 0.8, 65,536 cells.

| steps | continental cells | land at 0.5 | plates | rifts / sutures | born / subducted | cells at clamp | covered / foreign | deepest stack | deformation mean | ocean age p10 / p50 / p90 | speed min / median / max | tectonics phase |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 15 | 19,588 | 16,732 | 102 | 3 / 12 | 1,418 / 8,127 | 0 | 6,725 / 628 | 7 | 0.030 | 0.21 / 0.25 / 0.32 | 0.096 / 0.389 / 0.879 | 0.6 s |
| 30 | 19,196 | 16,845 | 85 | 5 / 31 | 5,094 / 17,246 | 0 | 7,059 / 316 | 7 | 0.053 | 0.10 / 0.45 / 0.53 | 0.081 / 0.446 / 0.918 | 1.2 s |
| 60 | 18,903 | 17,870 | 77 | 10 / 44 | 14,741 / 35,091 | 866 | 7,168 / 61 | 7 | 0.080 | 0.08 / 0.45 / 0.92 | 0.120 / 0.410 / 1.168 | 2.2 s |
| 120 | 18,364 | 18,139 | 84 | 28 / 53 | 37,019 / 62,642 | 4,260 | 7,572 / 157 | 7 | 0.109 | 0.07 / 0.41 / 1.68 | 0.094 / 0.337 / 1.066 | 4.3 s |
| 240 | 17,573 | 17,800 | 76 | 52 / 77 | 86,287 / 113,580 | 9,971 | 7,995 / 92 | 10 | 0.142 | 0.07 / 0.36 / 1.23 | 0.072 / 0.380 / 1.038 | 8.6 s |

Continental particles are 19,365 at every length, exactly.

### At the reference world

Sampling seed 9, subdivided faces 0.2, 65,536 cells.

| steps | continental cells | land at 0.5 | plates | rifts / sutures | born / subducted | cells at clamp | covered / foreign | deepest stack | deformation mean | ocean age p10 / p50 / p90 | speed min / median / max | tectonics phase |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 15 | 19,772 | 16,864 | 16 | 2 / 4 | 504 / 2,914 | 0 | 6,722 / 158 | 7 | 0.013 | 0.22 / 0.35 / 0.57 | 0.145 / 0.429 / 0.580 | 0.5 s |
| 30 | 19,638 | 17,048 | 13 | 3 / 8 | 1,681 / 5,848 | 0 | 6,748 / 50 | 7 | 0.023 | 0.42 / 0.56 / 0.78 | 0.190 / 0.405 / 0.670 | 1.0 s |
| 60 | 19,548 | 17,345 | 13 | 5 / 10 | 5,174 / 14,115 | 632 | 6,745 / 50 | 7 | 0.038 | 0.20 / 0.95 / 1.20 | 0.184 / 0.406 / 0.663 | 1.9 s |
| 120 | 19,586 | 18,157 | 18 | 16 / 16 | 12,475 / 27,280 | 2,116 | 6,863 / 79 | 8 | 0.059 | 0.17 / 1.71 / 2.03 | 0.214 / 0.366 / 0.914 | 3.9 s |
| 240 | 18,811 | 17,826 | 11 | 25 / 28 | 27,497 / 46,863 | 3,252 | 7,148 / 94 | 8 | 0.077 | 0.18 / 1.82 / 3.65 | 0.220 / 0.454 / 0.846 | 7.6 s |

Continental particles are 19,358 at every length, exactly. The two worlds
agree on every shape above: the same settling, the same unbounded clamp share,
the same slow loss of continental cells. The reference world reaches about
half the defaults' clamp share, because a dozen plates hold fewer boundaries
to raise it, and its ocean grows much older, because it holds fewer trenches
to consume it.

### Where each quantity settles

At the defaults, every tenth step of a 240-step run.

| steps | plates | cells at clamp | continental cells | land at 0.5 |
| --- | --- | --- | --- | --- |
| 0 | 111 | 0 | 19,365 | 16,432 |
| 10 | 102 | 0 | 19,886 | 16,819 |
| 20 | 93 | 0 | 19,366 | 16,603 |
| 30 | 85 | 0 | 19,196 | 16,845 |
| 40 | 81 | 76 | 19,102 | 17,183 |
| 50 | 82 | 402 | 19,077 | 17,628 |
| 60 | 77 | 866 | 18,903 | 17,870 |
| 70 | 79 | 1,331 | 18,597 | 17,705 |
| 80 | 77 | 1,799 | 18,475 | 17,724 |
| 90 | 79 | 2,435 | 18,493 | 17,917 |
| 100 | 80 | 3,091 | 18,354 | 17,930 |
| 110 | 79 | 3,713 | 18,359 | 18,057 |
| 120 | 84 | 4,260 | 18,364 | 18,139 |
| 130 | 82 | 4,587 | 18,468 | 18,262 |
| 140 | 82 | 5,158 | 18,382 | 18,291 |
| 150 | 83 | 5,747 | 18,404 | 18,346 |
| 160 | 82 | 6,347 | 18,263 | 18,271 |
| 170 | 86 | 6,984 | 18,412 | 18,502 |
| 180 | 86 | 7,637 | 18,407 | 18,500 |
| 190 | 84 | 8,073 | 18,469 | 18,588 |
| 200 | 83 | 8,554 | 18,155 | 18,234 |
| 210 | 80 | 9,018 | 18,030 | 18,159 |
| 220 | 76 | 9,436 | 17,789 | 17,904 |
| 230 | 75 | 9,712 | 17,660 | 17,851 |
| 240 | 76 | 9,971 | 17,573 | 17,800 |

- **Plates settle by 60 steps.** The partition makes 111; the run falls to 102
  by step 10 and 85 by step 30, and then holds between 75 and 86 for the
  remaining 180 steps. Rifting and suturing hold it there against each other:
  rifts run at a steady 0.17 to 0.23 a step at every length, while sutures fall
  from about one a step to about a third as the pairs available to merge run
  out. The count never runs away in either direction.
- **Land settles by 60 steps**, near 18,000 cells, and wanders between 17,700
  and 18,600 for the rest of the run. It starts lower — 16,432 at step zero and
  16,732 at step 15 — because deformation has not yet raised the belts.
- **Ocean age reaches its own distribution by 30 steps.** The prior's ages are
  consumed and the median holds near 0.4 model time from step 30 to step 240.
  One default step is one cell width of travel for the fastest plate, about
  88 km at the length scale the "Profile retune" section works in, which at a
  fast-plate speed of about 6 cm a year is roughly 1.5 Myr; on that reading the
  median floor is about 48 Myr against Earth's 60, and `cooling_age` at 0.56
  model time — about 60 Myr — sits inside the distribution as it should.
- **Cells at the clamp do not settle.** They are 0 out to step 30, first appear
  at step 40, and then climb by about 50 cells a step for the remaining 200
  steps with no sign of turning over: 866 at step 60, 4,260 at 120, and 9,971
  at 240, which is 15 percent of the world. The field's mean climbs with them,
  0.030 to 0.142. This is the finding of the slice.
- **Continental cells fall slowly**, 19,588 to 17,573 over 240 steps, while the
  particle count is exact. See the third item below for where they go.

### Cost

Cost is linear in the step count, about 36 ms a step on the whole tectonics
phase at this mesh. The viewer regenerates all three phases, and the other two
do not read the step count:

| steps | mesh | tectonics | geology | climate | whole pipeline |
| --- | --- | --- | --- | --- | --- |
| 15 | 0.12 s | 0.64 s | 0.08 s | 1.12 s | 1.96 s |
| 45 | 0.12 s | 1.74 s | 0.09 s | 1.13 s | 3.09 s |
| 60 | 0.12 s | 2.28 s | 0.09 s | 1.12 s | 3.61 s |

Climate costs more than tectonics at 15 steps and half as much at 60. The
whole pipeline goes from 2.0 to 3.6 seconds, which is not enough to argue for
a shorter default.

### The default is 60 steps

`TectonicsSettings::default()` takes `step_count` from 15 to 60. That is the
shortest run at which the age distribution, the plate count, and land have all
reached the steady states they hold out to 240 steps, and at which the clamp
still touches about one cell in eighty rather than one in seven. At
`DEFAULT_STEP_DURATION` it is 0.84 model time, about 90 Myr on the reading
above: the time an ocean takes to open.

The crate default of 5 in `PlateEvolutionConfig` is a library default the
fixtures scale from and does not change. No library pin and no viewer fixture
reads the viewer default — `test_support` sets its own step count of 4 — so
nothing was re-pinned for this. The viewer's step slider already reached 256.

### What a long run exposes

In priority order, each with the number that shows it.

1. **Deformation needs a sink.** *Resolved; see "Relief decay".* The clamp
   share grew without bound, because uplift was added every step and nothing
   removed it: 0 cells at 30 steps, 866 at 60, 4,260 at 120, and 9,971 at 240,
   climbing about 50 a step with no turnover. Every belt that held its regime
   long enough became a flat-topped plateau at `maximum_magnitude`, and the
   later history of a long run was written as a saturated field rather than as
   a record. Relief decay is the sink: the same runs now hold 19, 33, and 59
   cells at the clamp, with no trend after step 50.

2. **Ocean floor gets too old.** The median settles, but the old tail does not.
   The defaults' p90 goes 0.32, 0.53, 0.92, 1.68, 1.23 — 34 to 180 Myr — so by
   120 steps the oldest tenth of the floor is as old as the oldest floor on
   Earth. The reference world is far worse: its median reaches 1.82 and its p90
   3.65, about 195 and 390 Myr, because a dozen large plates hold too little
   trench to consume what their ridges make. Nothing in the model retires old
   floor except a trench it happens to meet.
3. **Continental material crowds, and nothing spreads it back out.** The
   particle count is exact at every length, but the cells holding continental
   material fall from 12,640 at 15 steps to 11,370 at 240, and the continental
   raster with them, 19,588 to 17,573. That is ten percent of the continents in
   240 steps, and it splits into 1,270 more covered particles and 745 fewer
   empty cells that read a continental neighbour. Almost none of it is one
   plate overriding another: the foreign count is 628 at 15 steps and 92 at
   240, and it falls rather than rises, because stacked material usually meets
   a trench or belongs to a plate that is itself absorbed. Most of the crowding
   arrives in the first ten steps — 1,525 covered particles after one step and
   6,603 after ten — which makes it a property of rotating a point set across
   an irregular Voronoi lattice rather than of run length. What run length adds
   is the slow creep of about five particles a step on top. The thickness slice
   that reads the collision stack is where both belong. *Partly resolved by
   "Crustal thickness": the foreign count falls to zero and the covered count
   falls with run length instead of rising, 5,626 at 15 steps to 4,100 at 240
   where it used to reach 7,995. The continental raster falls faster rather
   than slower, because merging leaves fewer parcels to fill cells with, so
   the crowding this item is really about is still open.*

4. **The drift band shapes a long run rather than bounding it.**
   `speed_drift_limit` of 0.5 was set against a fifteen-step walk whose
   unclamped excursion is expected to be about a quarter. The expectation goes
   as `0.061 * sqrt(n)`, so it is about a half at 60 steps and about 0.95 at
   240: the clamp is what holds the speed distribution together on a long run,
   not a bound on its tail. The distribution does not in fact run away — the
   median plate speed holds near 0.4 at every length at both worlds — which is
   the clamp doing that work. Whoever next touches the speed rule should decide whether
   that is wanted.

### Not changed here, and why

`DEFAULT_STEP_DURATION`, `cooling_age`, `suture_time`,
`full_deformation_time`, `rift_rate`, and `maximum_magnitude` all stay. Each is
a model-time quantity that already means the same thing at any run length, and
the clamp share is the symptom of a missing sink rather than of a wrong clamp.
Erosion and unstacking are the next two slices; this one exists to size them.

## Resolution independence

Three slices replaced every config field that was a hop, a cell, an edge, or a
step with the quantity it stood for: a model length on the unit sphere, a
density per unit area, a fraction of the sphere, or model time. A stage
converts once against its own mesh. The point is that the same settings
describe the same world at any cell count, and this section is what that is
worth measured rather than argued.

### What was renamed

Lengths, converted through `procgen_sphere_mesh::hops`:

| Was | Is |
| --- | --- |
| `BaseElevationConfig::margin_width_hops` | `margin_width` |
| `CoarseElevationConfig::smoothing_passes` | `smoothing_radius` |
| `VolcanicArcFieldConfig::inland_offset_cells` | `inland_offset` |
| `VolcanicArcFieldConfig::minimum_boundary_edges` | `minimum_boundary_length` |
| `HotspotFieldConfig::maximum_trail_cells` | `maximum_trail_length` |
| `HotspotFieldConfig::province_radius_hops` | `province_radius` |
| `HotspotFieldConfig::province_rim_hops` | `province_rim` |
| `PlateLifecycleConfig::suture_minimum_shared_edges` | `suture_minimum_shared_length` |

`BoundaryEffect::depth`, `ContinentalRiftProfile::decay_depth`,
`CratonFieldConfig`'s two distances, and
`IsostaticAdjustmentConfig::maximum_boundary_distance` keep their names and
changed type.

Areas and densities: `SedimentaryBasinFieldConfig::minimum_cell_count` became
`minimum_area_fraction`, `VolcanicArcFieldConfig::peak_density_divisor` became
`peak_density` per unit area, and the oceanic-peak presence draw scales with a
cell's own area.

Times: `CrustBirthPriorConfig::ridge_less_age` became model time.
`PlateEvolutionConfig::step_count` became `run_duration`, with the count
derived as the run over the step. `MoistureTransportConfig`'s `step_count` and
`step_seconds` became `simulated_days`, with the schedule derived from the
mesh.

`TRANSPORT_REACH_HOPS` and `MaterialTransportConfig::gap_radius` stay in hops,
because they describe the raster rather than the world.

### Measured

The viewer's defaults at sampling seed 7, at three cell counts. The step
duration is clamped to what transport can see, which is what the viewer's
slider does when the cell count rises; at 262,144 that clamp is what raises the
derived step count.

| | 16,384 | 65,536 | 262,144 |
| --- | --- | --- | --- |
| land fraction | 0.297 | 0.273 | 0.261 |
| plates | 86 | 77 | 80 |
| ocean age p10 / p50 / p90 | 0.098 / 0.588 / 0.923 | 0.084 / 0.448 / 0.923 | 0.061 / 0.416 / 0.902 |
| deformation clamp share | 2.13% | 1.32% | 1.36% |
| collision belt | 529 km | 441 km | 441 km |
| island arc belt | 176 km | 176 km | 176 km |
| rift belt | 353 km | 265 km | 265 km |
| evolution steps | 60 | 60 | 111 |
| sutures | 41 | 44 | 49 |
| arc peaks | 1691 | 2259 | 1140 |
| seamounts | 62 | 57 | 22 |
| abyssal hills | 912 | 1269 | 1884 |
| basins | 68 | 37 | 20 |
| flood basalt provinces | 2 | 2 | 0 |
| moisture steps | 60 | 120 | 240 |
| moisture range | 529 km | 265 km | 132 km |
| whole pipeline | 1.0 s | 4.2 s | 32.5 s |

Moisture range is the distance inland at which ocean-sourced precipitation
falls to a tenth of its value one cell from the coast.

A mesh makes its own world, so a count that reads the boundary network varies
between meshes for reasons that are not resolution. The control is the same
table at sampling seeds 7, 11, and 23:

| | 16,384 | 65,536 |
| --- | --- | --- |
| land fraction | 0.297, 0.307, 0.278 | 0.273, 0.272, 0.270 |
| plates | 86, 95, 73 | 77, 68, 91 |
| arc peaks | 1691, 1713, 1635 | 2259, 2247, 2364 |
| seamounts | 62, 43, 49 | 57, 57, 44 |
| abyssal hills | 912, 1100, 887 | 1269, 1253, 1432 |

### What this bought, and what it did not

Independent, within what an irregular mesh allows:

- Belt widths. A collision belt is 441 km at 65,536 and at 262,144, an island
  arc 176 km at all three. At 16,384 the collision belt is 529 km because five
  default hops is two and a half cells there and a belt is a whole number of
  cell rings: the mesh cannot draw the width it was asked for, and rounds up.
  This is the quantisation the conversion admits to, not a scaling error.
- The evolution run. All three cover the same 0.84 of model time; 262,144
  takes 111 steps rather than 60 because its transport needs a shorter one.
  Before this work it would have run 60 steps and covered half the history.
- The moisture run. 60, 120, and 240 steps over the same thirty days.
- Deformation clamp share, ocean age p90, plate count, and suture count agree
  to within the seed spread.
- Seamount count: 43 to 62 at 16,384 against 44 to 57 at 65,536, which is one
  range.

Not independent, and why:

- Moisture range halves with the cell width, 529 to 265 to 132 km. Those are
  exactly three cells at every resolution, reached after 60, 120, and 240
  steps. The step count is not the limit: if the distance a step carries
  moisture were what stopped it, twice the steps would carry it twice as many
  cells and the kilometres would hold. Something removes a fixed fraction per
  step rather than per second, so twice the steps removes twice as much over
  the same thirty days. The two leads are the config's two per-step caps,
  `maximum_orographic_fraction_per_step` at 0.35 and
  `maximum_transport_fraction_per_step` at 0.5; neither is shown to be the
  cause here. Stating the run as days fixed how long the weather runs, not how
  far it reaches, and the fix is a climate slice. This measurement is what
  sizes it: the number to move is three cells, and it should be a distance.
- Arc peaks: 1691, 2259, 1140, against a seed spread of under five percent
  within a resolution, so this is the mesh. Two causes. At 16,384 every arc
  cell is a peak — the peak count equals the arc cell count exactly — because
  a density of one per two default cells asks for more peaks than a coarse
  mesh has cells to put them in, and a per-cell field saturates. And the arc
  belt itself covers 0.103 of the sphere at 16,384 against 0.068 at 65,536,
  because the belt is a whole number of cell rings around a boundary network
  that is itself a different length.
- Basin count falls by about half per fourfold cell count. The area threshold
  is a fixed area, but what it measures is a connected component, and a finer
  mesh resolves low-lying land into more and smaller pieces. Connectivity is
  not something an area threshold can make resolution independent.
- Land fraction drifts from 0.297 to 0.261. The seed spread at 16,384 is 0.029
  and the gap to 65,536 is 0.02, so part of this is the world; the tight
  clustering at 65,536 says part of it is not.
- Province count is two, two, and zero, which is a hashed draw over twenty
  hotspots. Small-number noise rather than a trend.

## Relief decay

"Run length" found one quantity that does not settle: cells pinned at
`maximum_magnitude` climbed about fifty a step forever, because uplift was
added every step and nothing removed it. This slice gives deformation the sink
it never had. Every step, what a parcel of crust carries is multiplied by
`1 - step_duration / erosion_time` before that step's boundaries add to it.

### Why decay, and where the time constant comes from

At 88 km cells the erosion that matters is not hillslope diffusion or river
incision — both live far below one cell — but the denudation of a whole
orogen. Ahnert's 1970 relation makes that rate proportional to mean relief,
and proportional loss is exponential decay. Relief with no uplift under it
falls as `exp(-t / tau)`; relief under a boundary rises to where uplift and
denudation balance and stays there. That is the shape of every real orogen:
Tibet and the Andes stand at their steady state while convergence lasts, the
Appalachians and the Urals have decayed for 300 Myr and are low but not gone.

Ahnert's coefficient gives an e-folding near 7 Myr for surface relief eroding
freely, but crust under a mountain rebounds isostatically as the top is
stripped, so surface elevation falls about six times more slowly than rock
leaves. An effective e-folding of 40 to 50 Myr is what leaves the Appalachians
standing. At one default step per 1.5 Myr that is about thirty steps, so
`erosion_time` defaults to `30 * DEFAULT_STEP_DURATION`, 0.42 model time.

Checked against the source it sits under: a saturated convergent boundary adds
`0.4 * 0.014 / 0.126 = 0.044` a step, and the decay takes `D / 30` a step.
They balance at `D = 1.33`, well above the 0.5 clamp, so an active belt still
reaches the clamp and holds there. That is now what the clamp means — the
steady state of a belt that keeps converging — rather than an accumulator
overflowing.

The kept fraction is the linear `1 - dt/tau` rather than `exp(-dt/tau)`, which
costs a multiply and a divide instead of libm on a path a kernel would mirror.
The two differ to second order in `dt/tau`, about one part in a thousand at the
defaults, so a run at half the step does not decay to the same bits. The
still-world halving test is unaffected: a still world raises no deformation to
decay.

### Measured

Both worlds are 65,536 cells, at `DEFAULT_STEP_DURATION`. "Defaults" is
sampling seed 7 and jitter 0.8; "Reference" is sampling seed 9 with subdivided
faces 0.2. "Sink off" is `erosion_time` infinite, which is the world this
slice replaced. "Relic" cells carry more than 0.1 of deformation and lie
further than the convergent profile's own depth from any current convergent
edge: belts standing where no boundary is. Release on an M1 Max.

| steps | sink | cells at clamp | mean | maximum | land at 0.5 | relic cells | largest relic |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 15 | off | 0 | 0.0301 | 0.310 | 16,732 | 1 | 0.112 |
| 15 | on | 0 | 0.0241 | 0.245 | 16,525 | 0 | — |
| 30 | off | 0 | 0.0532 | 0.483 | 16,845 | 595 | 0.262 |
| 30 | on | 0 | 0.0351 | 0.335 | 16,263 | 131 | 0.157 |
| 60 | off | 866 | 0.0804 | 0.500 | 17,870 | 3,486 | 0.500 |
| 60 | on | 19 | 0.0405 | 0.500 | 16,684 | 916 | 0.343 |
| 120 | off | 4,260 | 0.1093 | 0.500 | 18,139 | 4,270 | 0.500 |
| 120 | on | 33 | 0.0439 | 0.500 | 16,294 | 174 | 0.268 |
| 240 | off | 9,971 | 0.1425 | 0.500 | 17,800 | 6,478 | 0.500 |
| 240 | on | 59 | 0.0511 | 0.500 | 15,948 | 716 | 0.373 |

At the reference world, sampling seed 9 and subdivided faces 0.2:

| steps | sink | cells at clamp | mean | maximum | land at 0.5 | relic cells | largest relic |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 15 | off | 0 | 0.0133 | 0.210 | 16,864 | 47 | 0.116 |
| 15 | on | 0 | 0.0105 | 0.172 | 16,766 | 0 | — |
| 30 | off | 0 | 0.0229 | 0.495 | 17,048 | 1,036 | 0.227 |
| 30 | on | 0 | 0.0144 | 0.352 | 16,772 | 192 | 0.147 |
| 60 | off | 632 | 0.0381 | 0.500 | 17,345 | 2,022 | 0.500 |
| 60 | on | 1 | 0.0180 | 0.500 | 16,570 | 143 | 0.199 |
| 120 | off | 2,116 | 0.0586 | 0.500 | 18,157 | 4,515 | 0.500 |
| 120 | on | 3 | 0.0212 | 0.500 | 16,957 | 813 | 0.298 |
| 240 | off | 3,252 | 0.0770 | 0.500 | 17,826 | 8,800 | 0.500 |
| 240 | on | 1 | 0.0167 | 0.500 | 15,781 | 872 | 0.305 |

Every tenth step of a 240-step run at the defaults, with the sink on:

| steps | cells at clamp | mean | maximum | land at 0.5 | relic cells | largest relic |
| --- | --- | --- | --- | --- | --- | --- |
| 0 | 0 | 0.0000 | 0.000 | 16,432 | 0 | — |
| 10 | 0 | 0.0183 | 0.191 | 16,732 | 0 | — |
| 20 | 0 | 0.0289 | 0.263 | 16,304 | 42 | 0.138 |
| 30 | 0 | 0.0351 | 0.335 | 16,263 | 131 | 0.157 |
| 40 | 0 | 0.0392 | 0.430 | 16,408 | 370 | 0.233 |
| 50 | 1 | 0.0406 | 0.500 | 16,579 | 321 | 0.228 |
| 60 | 19 | 0.0405 | 0.500 | 16,684 | 916 | 0.343 |
| 70 | 30 | 0.0410 | 0.500 | 16,405 | 304 | 0.211 |
| 80 | 25 | 0.0414 | 0.500 | 16,281 | 798 | 0.301 |
| 90 | 0 | 0.0423 | 0.498 | 16,287 | 510 | 0.223 |
| 100 | 7 | 0.0423 | 0.500 | 16,167 | 230 | 0.238 |
| 110 | 45 | 0.0434 | 0.500 | 16,279 | 186 | 0.301 |
| 120 | 33 | 0.0439 | 0.500 | 16,294 | 174 | 0.268 |
| 130 | 21 | 0.0433 | 0.500 | 16,363 | 446 | 0.265 |
| 140 | 28 | 0.0433 | 0.500 | 16,310 | 304 | 0.285 |
| 150 | 67 | 0.0450 | 0.500 | 16,390 | 221 | 0.187 |
| 160 | 54 | 0.0472 | 0.500 | 16,189 | 366 | 0.332 |
| 170 | 26 | 0.0483 | 0.500 | 16,378 | 151 | 0.272 |
| 180 | 41 | 0.0500 | 0.500 | 16,280 | 182 | 0.287 |
| 190 | 43 | 0.0510 | 0.500 | 16,504 | 130 | 0.191 |
| 200 | 30 | 0.0521 | 0.500 | 16,285 | 208 | 0.322 |
| 210 | 26 | 0.0532 | 0.500 | 16,377 | 374 | 0.292 |
| 220 | 81 | 0.0527 | 0.500 | 16,150 | 918 | 0.285 |
| 230 | 58 | 0.0521 | 0.500 | 16,024 | 456 | 0.337 |
| 240 | 59 | 0.0511 | 0.500 | 15,948 | 716 | 0.373 |

### What settled

- **The clamp share settles by about fifty steps and holds.** The first cell
  reaches it at step 50 and the count then oscillates between 0 and 81 for the
  remaining 190 steps with no trend: 19 at 60, 33 at 120, 59 at 240. It was
  866, 4,260, and 9,971, climbing about fifty a step. This is the finding the
  slice was for.
- **Relic belts are fewer and much lower.** At 60 steps they fall from 3,486
  cells to 916 and the largest from the clamp itself to 0.343; at 240, from
  6,478 to 716 and 0.500 to 0.373. With the sink off a relic belt was as high
  as an active one, which is the thing that made a long run unreadable. With
  it on, no relic reaches the clamp at any length at either world.
- **The mean still creeps, slowly.** 0.0183 at ten steps, 0.0405 at sixty,
  0.0511 at 240. A third of the sink-off mean at 240 and far flatter, but not
  flat: the height of a belt is bounded now, its area is not.
- **Land falls by about 1,200 cells at sixty steps** — 17,870 to 16,684 — and
  stops climbing with run length. Lower belts clear the 0.5 datum in fewer
  cells; this is the visible price of the sink and it is the intended one.

### Pins

None moved. Ownership, crust birth, the plate set, and every count a run keeps
read the crust and the motion rather than the relief on them, so the whole
tectonics suite stood unchanged;
`turning_the_sink_off_changes_the_deformation_field_alone` in `evolution.rs`
is the assertion that says so, comparing whole results rather than named
fields. The deformation field itself was never pinned, because a float
fingerprint pins the toolchain rather than the algorithm.

### Not this slice

No slope-driven transport, no sediment deposition, no isostatic rebound as a
separate term, and no reading of the collision stack: the time constant folds
rebound in and the thickness slice unfolds it. No change to the profile
offsets, `full_deformation_time`, or `maximum_magnitude`. `erosion_time` is
the one knob here: lower it and belts wear down faster and land falls further;
raise it and the clamp share starts climbing again, reaching the sink-off
behaviour at infinity.

## Crustal thickness

A continental parcel that arrives under another continent at a trench stops
being a stacked ghost and becomes thickness: the two merge into one column
holding both, and base elevation floats that column as a plateau. Continental
material is still exactly conserved, as a sum of thickness rather than a count
of particles.

### What it replaces

Three findings pointed at the same missing rule. Continental parcels that lost
a cell to another plate's continent survived under it and kept moving with
their own plate, and nothing ever read them. The collision plateau was paint:
the `convergent` profile raised the same offset whether a collision had just
begun or had underthrust for sixty steps. And the deformation mean kept
creeping after relief decay, because deformation was the only record of
shortening and it could only spread.

On Earth the Tibetan plateau is where Indian crust has been thrust under Asian
crust. Its extent is the underthrust distance and its height is Airy isostasy
on the doubled crust — a root of extra crust floats a surface about a fifth as
high. Thickness gives the model both, out of material it already conserves.

### The rule

`Particle` gains `thickness`, the number of original parcels it holds. In
`pick_winners`, a loser that is continental, belongs to another plate, and
whose cell has a convergent edge to that plate within the reach transport
already searches is **accreted**: the winner takes its thickness and the loser
is removed. That is the trench rule's own test, factored out and shared, so
the two cannot drift apart. The winner is always the incumbent's continent —
continent outranks floor, and among two continents the cell's own plate keeps
it — so the arriving continent goes under and the boundary stays where the
suture will form, which is what India did.

Compaction accretes rather than drops. A plate that owns no cell still holds
parcels; its floor is dropped as before and its continent merges into the
column its cell reads, so the thickness a run conserves survives its own
compaction.

`BaseElevationConfig::thickness_uplift` floats a continental cell by
`thickness_uplift * (thickness - 1)`, after the margin taper and inside the
same clamp as every other term.

### Measured

65,536 cells at `DEFAULT_STEP_DURATION`, release on an M1 Max. "Before" is
`thickness_uplift` zero with the old `convergent` profile, which is the world
this slice replaces; "thick only" adds the uplift; "thick+retune" also halves
and narrows `convergent`. Plateau columns describe the largest connected run
of thickened cells and how far its surface stands above the continent around
it.

Viewer defaults, sampling seed 7 and jitter 0.8:

| steps | variant | accreted | thickened | max thick | covered | foreign | cont. cells | land | def. mean | elev. clamp | plateau cells | plateau height |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 15 | before | 1,638 | 1,108 | 13 | 5,626 | 1 | 19,391 | 16,124 | 0.0239 | 0 | 19 | 0.030 |
| 15 | thick only | 1,638 | 1,108 | 13 | 5,626 | 1 | 19,391 | 16,288 | 0.0239 | 17 | 19 | 0.214 |
| 15 | thick+retune | 1,638 | 1,108 | 13 | 5,626 | 1 | 19,391 | 15,885 | 0.0091 | 5 | 19 | 0.214 |
| 60 | before | 3,469 | 1,469 | 52 | 5,162 | 0 | 17,499 | 15,145 | 0.0421 | 4 | 34 | −0.051 |
| 60 | thick only | 3,469 | 1,469 | 52 | 5,162 | 0 | 17,499 | 15,399 | 0.0421 | 260 | 34 | 0.054 |
| 60 | thick+retune | 3,469 | 1,469 | 52 | 5,162 | 0 | 17,499 | 15,112 | 0.0327 | 211 | 34 | 0.055 |
| 120 | before | 4,742 | 1,596 | 70 | 4,713 | 0 | 16,296 | 14,631 | 0.0457 | 58 | 41 | −0.037 |
| 120 | thick+retune | 4,742 | 1,596 | 70 | 4,713 | 0 | 16,296 | 14,770 | 0.0396 | 363 | 41 | 0.141 |
| 240 | before | 6,951 | 1,809 | 115 | 4,100 | 0 | 13,941 | 12,416 | 0.0430 | 22 | 50 | −0.032 |
| 240 | thick+retune | 6,951 | 1,809 | 115 | 4,100 | 0 | 13,941 | 12,600 | 0.0371 | 345 | 50 | 0.224 |

Reference world, sampling seed 9 and subdivided faces 0.2:

| steps | variant | accreted | thickened | max thick | covered | foreign | cont. cells | land | def. mean | elev. clamp | plateau cells | plateau height |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 15 | before | 707 | 382 | 13 | 6,257 | 0 | 19,537 | 16,548 | 0.0107 | 0 | 17 | 0.010 |
| 15 | thick+retune | 707 | 382 | 13 | 6,257 | 0 | 19,537 | 16,386 | 0.0039 | 0 | 17 | 0.184 |
| 60 | before | 1,219 | 459 | 33 | 6,041 | 0 | 18,936 | 15,971 | 0.0169 | 29 | 22 | 0.022 |
| 60 | thick+retune | 1,219 | 459 | 33 | 6,041 | 0 | 18,936 | 15,951 | 0.0129 | 74 | 22 | 0.257 |
| 120 | before | 2,176 | 706 | 81 | 5,739 | 0 | 18,215 | 15,579 | 0.0240 | 0 | 32 | 0.021 |
| 120 | thick+retune | 2,176 | 706 | 81 | 5,739 | 0 | 18,215 | 15,537 | 0.0194 | 87 | 32 | 0.214 |
| 240 | before | 4,127 | 1,192 | 81 | 5,013 | 0 | 16,622 | 14,378 | 0.0329 | 74 | 27 | 0.002 |
| 240 | thick+retune | 4,127 | 1,192 | 81 | 5,013 | 0 | 16,622 | 14,473 | 0.0290 | 164 | 27 | 0.159 |

Thickness over the thickened cells of the viewer's defaults:

| steps | thickened | p50 | p90 | p99 | max | over 5 | over 10 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 15 | 1,108 | 3 | 5 | 9 | 13 | 89 | 3 |
| 60 | 1,469 | 3 | 9 | 22 | 52 | 327 | 104 |
| 120 | 1,596 | 3 | 10 | 30 | 70 | 432 | 157 |
| 240 | 1,809 | 4 | 13 | 48 | 115 | 585 | 258 |

### What it fixed

- **Stacked continent is gone.** Foreign continental particles — parcels lying
  under a cell their own plate does not own — fall to 1 at fifteen steps and 0
  at every length beyond, at both worlds, from 628 and 61 before. The whole of
  that stacking was continental, which is why `collided_cell_count` and
  `maximum_collision_stack` now read zero on the reference fixture: what is
  left for them to count is a parcel that crossed a transform or a ridge by
  lattice jitter, and neither world has one.
- **The covered count falls with run length instead of rising.** 5,626 at
  fifteen steps to 4,100 at 240, where before it climbed 6,725 to 7,995.
- **The deformation mean stops creeping** and turns over: 0.0239, 0.0421,
  0.0457, 0.0430. The retune lowers it further, to 0.0327 at sixty steps.
- **A collision now has a plateau.** Before, the largest thickened region
  stood 0.05 *below* the continent around it at sixty steps and 0.03 below at
  240, because the convergent profile painted the rim as hard as the middle.
  After, it stands 0.14 to 0.26 above at most lengths.

### The finding that needed a second rule

Accretion alone left thickness where it landed. The deepest column went 13,
52, 70, 115 parcels at 15, 60, 120 and 240 steps — linear in run length, with
no bound — while the largest connected run of thickened cells held at 19, 34,
41, 50. The model made thick cells, not a Tibet, and no value of
`thickness_uplift` turned that into a plateau: swept at sixty steps, the
largest plateau stood 0.032 high at an uplift of 0.05 and 0.063 at 0.3, while
the cells clamped at 1.0 climbed 98, 211, 394. Raising the knob bought no
height and bought saturated cells linearly, because the plateau's own cells
were thin and the deep columns were isolated.

Real thick crust flows. Tibet is flat because its lower crust spreads under
its own weight, and a plateau's extent is set by that spreading as much as by
the underthrust that fed it. Thickness had the source; it needed the spreading.

### The flow rule, and why this one

One pass over the cells between picking the step's winners and projecting
them, reading the thicknesses the winners hold before the pass and writing
after it, so the update is simultaneous like every other substep.

Every cell whose winner is continental asks its thickest same-plate
continental neighbour for one parcel, if that neighbour stands at least two
parcels above it; equally thick neighbours lose the tie to the lower cell id.
A column that receives more requests than two serves the two lowest cell ids
and refuses the rest. Continental only, same plate only, and only the columns
cells actually read — so a plateau cannot spread into ocean, cannot cross a
plate boundary until a suture has made the two plates one, and cannot reach a
covered parcel.

The gap of two and the cap of two are what make the pass converge, and the
argument is short enough to state. Let `Q` be the sum of squared thickness
over continental parcels. Every cell asks at most one neighbour, so a column
receives at most one parcel and gives at most two, and every transfer runs
from a column at `t` to one at `s <= t - 2`. Writing `T` for the transfers a
pass makes,

```text
dQ = sum (s - g + r)^2 - s^2
   = 2 sum_transfers (s_receiver - s_giver) + sum (r - g)^2
  <= -4T + (T + 2T)  =  -T
```

so a pass that moves anything strictly lowers `Q`. `Q` is a non-negative
integer, so the passes reach a fixed point, and at that point no cell has a
same-plate continental neighbour two or more parcels thicker than itself: the
crust is a plateau with a one-parcel rim. The cap is what keeps the middle
term small — a column granting `k` requests moves `Q` by at most `k(k - 3)`,
which is negative at one and two grants, zero at three, and positive from
four, so an uncapped column emptying into thin neighbours could grow `Q`
instead. A gap of one would let two columns differing by one swap the same
parcel for ever, because a pass that reads before it writes has no order to
break that tie with. Neither number is a knob; both are the proof.

The rate that follows is two parcels a step out of a column, so a 115-parcel
column takes about sixty steps to spread and a three-parcel stack takes one.
At one step per 1.5 Myr that is the right order for lower crustal flow.

### Measured with the flow

65,536 cells at `DEFAULT_STEP_DURATION` and `thickness_uplift` 0.1, release on
an M1 Max. "Deepest" is the deepest column in parcels; "plateau" is the
largest connected run of thickened cells and how far its surface stands above
the continent around it.

Viewer defaults:

| steps | accreted | transfers | thickened | deepest | cont. cells | land | elev. clamp | plateau cells | plateau height |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 15 | 1,638 | 1,355 | 1,913 | 7 | 19,391 | 15,913 | 2 | 104 | 0.094 |
| 60 | 3,469 | 5,280 | 3,362 | 28 | 17,499 | 15,201 | 116 | 417 | 0.121 |
| 120 | 4,742 | 9,327 | 4,131 | 16 | 16,296 | 14,853 | 409 | 594 | 0.098 |
| 240 | 6,951 | 17,865 | 5,353 | 56 | 13,941 | 12,827 | 291 | 889 | 0.122 |

Reference world:

| steps | accreted | transfers | thickened | deepest | cont. cells | land | elev. clamp | plateau cells | plateau height |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 15 | 707 | 574 | 714 | 13 | 19,537 | 16,403 | 0 | 111 | 0.118 |
| 60 | 1,219 | 1,753 | 1,224 | 14 | 18,936 | 15,995 | 81 | 202 | 0.105 |
| 120 | 2,176 | 3,669 | 2,004 | 48 | 18,215 | 15,599 | 66 | 226 | 0.079 |
| 240 | 4,127 | 8,720 | 3,699 | 25 | 16,622 | 14,671 | 152 | 371 | 0.083 |

Thickness over the thickened cells of the viewer's defaults, against the same
distribution before the flow:

| steps | thickened | p50 | p90 | p99 | max | p90 before | p99 before | max before |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 15 | 1,913 | 2 | 2 | 5 | 7 | 5 | 9 | 13 |
| 60 | 3,362 | 2 | 3 | 5 | 28 | 9 | 22 | 52 |
| 120 | 4,131 | 2 | 3 | 6 | 16 | 10 | 30 | 70 |
| 240 | 5,353 | 2 | 4 | 7 | 56 | 13 | 48 | 115 |

### What the flow settled

- **The plateau grows with the run.** The largest connected thickened region
  goes 104, 417, 594, 889 cells where it held at 19, 34, 41, 50, and the
  thickened cells with it, 1,913 to 5,353 against 1,108 to 1,809. A collision
  now spreads across a region rather than into a cell.
- **The distribution flattened.** The ninetieth percentile of thickness is 2
  to 4 parcels where it was 5 to 13, and the ninety-ninth is 5 to 7 where it
  was 9 to 48. The typical thickened cell holds two parcels, which is the
  doubled crust the physics describes.
- **The deepest column stopped growing with run length.** It goes 7, 28, 16,
  56 at the defaults and 13, 14, 48, 25 at the reference world: it wanders
  rather than climbing, where before it rose 13, 52, 70, 115 monotonically.

### What the flow did not settle

- **The deepest column is not in single digits.** The rule drains two parcels
  a step from any one column, and an actively fed collision accretes faster
  than that, so the handful of columns sitting under a closing boundary
  outrun the drain while the run lasts. What changed is that they no longer
  grow without bound and no longer set the shape of the field: 56 parcels at
  240 steps against a ninety-ninth percentile of 7.
- **The plateau is lower than the slice asked for.** It stands 0.079 to 0.122
  above the continent around it, against the 0.15 to 0.3 a real plateau
  wants. The cause is the flattening: `thickness_uplift` is per parcel and the
  typical thickened cell now holds two rather than the three that set the
  default, so the typical plateau cell stands one term up instead of two.
- **The clamp share still rises with the term on.** Swept at the defaults:

  | steps | uplift | land | elev. clamp | plateau height |
  | --- | --- | --- | --- | --- |
  | 60 | 0 | 14,783 | 3 | 0.042 |
  | 60 | 0.1 | 15,201 | 116 | 0.121 |
  | 60 | 0.15 | 15,283 | 329 | 0.152 |
  | 60 | 0.2 | 15,328 | 561 | 0.180 |
  | 240 | 0 | 12,318 | 22 | 0.005 |
  | 240 | 0.1 | 12,827 | 291 | 0.122 |
  | 240 | 0.15 | 12,919 | 833 | 0.175 |
  | 240 | 0.2 | 12,954 | 1,321 | 0.213 |

  An uplift of 0.15 reaches the bottom of the band and roughly triples the
  clamped cells; 0.1 keeps them near a hundred and falls short of it. The flow
  narrowed the gap — 116 clamped cells at sixty steps against 211 before it —
  but did not close it. `thickness_uplift` stays at 0.1 and the choice is
  recorded here rather than made silently.

### Continental area is now a sediment-budget number

Continental cells fall 19,391 to 13,941 over 240 steps at the defaults, and
the flow does not change that: it moves thickness between cells without
changing which cells are continental. Area times thickness is what a run
conserves, so as crust thickens its area must fall, and nothing yet thins it
back. That is the sediment budget's to answer, and until it lands the falling
continental raster is the arithmetic working rather than a defect.

### The defaults these measurements chose

`thickness_uplift` is **0.1**, not the 0.2 that Airy isostasy on a doubled
crust alone would ask for. The term is per parcel and the median thickened
cell holds three, so 0.1 puts the typical collision 0.2 above the continent
around it, which is the 3-to-5 km plateau the physics describes. At 0.2 that
same typical cell reaches the clamp at 1.0, and the clamped count roughly
doubles at every run length.

`convergent` goes from offset 0.4 over six hops to **0.2 over three**. With the
plateau made of material, the profile is the fold-and-thrust front at the
suture rather than the plateau behind it, and leaving it as it was would count
the same crust twice. The retune lowers the deformation mean by about a fifth,
leaves the deformation clamp where it was or slightly below, and does not
change plateau height, which is what says the plateau is now coming from
thickness rather than from paint.

### Pins

Ownership and birth both moved, and the reason is worth stating because it is
not the obvious one. Within a single step accretion only removes a parcel that
had already lost its cell, so that step's owners and births are untouched.
Across steps it is not neutral: the parcel is gone from every later step, so a
cell it would have won in step three is won by something else and the run makes
its floor in other places. `owner_change_count` on the reference fixture falls
149 to 147, `sampled_cell_count` rises 696 to 716, and the ownership, birth,
age, and base-elevation fingerprints all move once.

The flow moved one more: `thickened_cell_count` on the reference fixture rises
27 to 34, which is a column reaching its neighbours and is the whole point of
the pass. `thickness_transfer_count` is new and pinned at 7. Nothing else
moved, because the flow changes what a column holds and never which parcel
won a cell.


The viewer's complete-world fixtures moved mesh rather than pins. A complete
world runs the climate phase, whose coupling is a fixed-point iteration that a
mesh below a couple of hundred cells does not reliably reach: swept over
fourteen seeds, 7 of 14 converge at 32 cells, 4 at 64, and 7 at 128, with and
without the thickness term alike, while every one of them converges at 192 and
above and the 65,536-cell default converges in a single iteration. Any change
that moves elevation re-rolls which seeds fall badly, which is what broke nine
tests here and one in the previous slice. Those fixtures now run at 1,024 cells
and up.

### Not this slice

No thickness decay or isostatic rebound: thickened crust stays thick, so
nothing removes a plateau once its collision ends. No crustal thinning at
rifts. No same-plate shortening — one plate's own parcels crowding behind a
blocked front still stack and are still uncounted. No change to the geology
isostasy stage, whose convergent support bonus now overlaps this term. The
flow rate is not configurable: two parcels a step is the cap the convergence
argument needs, and moving it means redoing that argument at a wider gap.

The next slice is the sediment budget. It answers the three things left open
here: the continental area that falls because area times thickness is
conserved, the plateau that never wears down once its boundary moves on, and
the handful of actively fed columns that still outrun a two-parcel drain.

