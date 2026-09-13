# Plate movement

## Goal

Make plate motion coherent and make the evolution leave a record. The
partition draws plate-like outlines; this is what happens to them afterwards.
Motion is fitted to a smooth global field rather than drawn independently per
plate, the material itself moves and carries its own history, and the plate set
changes during a run.

The measure of success is visual: convergence and divergence organised into
belts that span the sphere, mountain ranges whose width follows how long a
boundary converged, and oceans whose age deepens away from the ridge that made
them.

## Current state

Every stage below has landed. The viewer's default world is 65,536 cells at
sampling seed 7, jitter 0.8, and a run of 60 steps.

- `generate_plate_kinematics(mesh, partition, crust, config)` fits each plate's
  Euler vector to a smooth global flow field over the plate's own cells, then
  scales the fitted direction by `crust_scaled_speed`. That is the whole of the
  speed rule.
- `classify_boundaries` derives per-edge normal and shear speeds from the two
  owners' rotations and classifies each edge.
- `derive_crust_birth_prior` produces the birth field evolution starts from, in
  model time. `derive_seafloor_age` is `elapsed_time - birth`.
- `evolve_plate_ownership` carries the material across steps as particles, one
  per parcel of crust, each holding its birth time, its accumulated
  deformation, and its thickness. It returns the final ownership, the motion it
  ended on, the accumulated deformation, the per-cell birth, and the per-cell
  thickness.
- `derive_base_elevation` reads the mesh, the final seafloor age, the final
  cell crust, and the flow field.
- Cell crust is derived from birth and never stored: `Some` is oceanic, `None`
  is original continental crust. A rifting continental plate therefore grows an
  oceanic margin.
- The plate set a run returns is not the one it was handed. Plate ids are not
  stable across a run, and the final plate count differs from the partition
  stage's in both directions.

What no longer exists, and is not coming back unless something visible asks for
it: a size factor on plate speed, a slab-pull factor and the per-step respeed
that applied it, lateral flow of crustal thickness, and a base-elevation term
that floated a column by the crust under it. Each was measured, each moved
numbers, and none of them changed what the viewer's default world looks like.
The measurements are in this file's history.

## Design

### Flow-field kinematics

A smooth global vector field stands in for mantle convection: three channels of
the fixed-polynomial gradient noise at a low lattice frequency, assembled into
a model-space vector and projected onto the tangent plane. A plate's Euler
vector is the rigid rotation that best fits that field over the plate's cells,
weighted by cell area — minimise the sum of `area · |ω × p − v(p)|²`, whose
normal equations are one symmetric 3-by-3 solve per plate. Fitting neighbouring
plates to one field is what gives boundary regimes a pattern larger than a
plate.

`coherence` blends the fitted axis back toward a hashed random one, so zero
reproduces independent random motion and one is fully field-driven; it defaults
to 0.85. A plate of one or two cells has singular normal equations and no
rotation to fit, so it keeps the hashed axis. Only direction falls back.

`flow_frequency` defaults to 1.0 and is the knob that matters. A flow cell has
to be much larger than a plate for the fit's Euler axis to agree between
neighbours, so raising the frequency costs coherence quickly; halving it buys
alignment but can halve migration.

Speed is the plate's hashed base speed, drawn from
`[minimum_angular_speed, maximum_angular_speed]`, times a crust factor, clamped
to `maximum_angular_speed`. The crust factor interpolates between
`oceanic_speed_factor` at no continental area and `continental_speed_factor` at
nothing but, by the plate's own continental area share: a plate that is half
continent is half as slowed by it. The defaults are 1.5 and 1.0, so continental
crust slows a plate, and the spread between neighbours is part of what makes a
boundary clear the convergence threshold.

There is no lower clamp. `minimum_angular_speed` bounds the hashed draw and
nothing else, so a plate carrying a continent turns more slowly than that.
Forsyth and Uyeda (1975) found no correlation between plate area and speed —
the Pacific is the largest plate and among the fastest — and a negative one
with continental area, which is the correlation the crust factor keeps.

`maximum_angular_speed` is the fastest plate a world can hold. The crust-birth
prior reads it as the speed one hop of its walk is crossed at, and the viewer
reads it to bound its step slider.

### Particle transport

A step rotates every particle rigidly with its plate and each cell then
resolves whatever landed in it. A particle is a unit position, a plate, a birth
time, a deformation, a thickness, and the cell and Delaunay triangle its last
location walk found. The projection every consumer reads — ownership, birth,
deformation, thickness — is what each cell sampled from the particle that won
it, so the fields move with the material by construction rather than by a rule
that copies them between cells.

- **Move.** Each particle turns about its plate's rotation vector by
  `|ω| · step_duration`. The position splits into the part along the axis,
  which the rotation leaves alone, and the part across it, which is turned
  toward `axis × position` — perpendicular to it and of its own length, which
  is what makes the turn preserve that length. Every particle of one plate goes
  through the same linear map, so a plate's material is rigid by construction
  rather than by tolerance.
- **Locate.** `SphereMesh::locate_delaunay(position, hint)` with the particle's
  previous triangle as the hint, so a step costs a short walk per particle.
- **Resolve.** Each cell takes the particle that wins it, by one lexicographic
  order: continental over oceanic; then, among continental, the cell's previous
  owner over a newcomer; then, among oceanic, the younger; then the nearer to
  the cell's centre; then the lower particle index. It is written as one total
  order rather than a pairwise rule because the pairwise rule is not
  transitive — three particles on two plates can cycle, and the winner would
  then depend on the order they were compared in.
- **Subduct.** A losing particle is removed when it is oceanic, belongs to a
  different plate than the winner, and the cell has an edge to a cell of the
  loser's plate that the current classification calls convergent. That is the
  trench and nothing else. Every other loser survives, stacked: two parcels of
  one plate are lattice noise that spreads back out next step, continental
  material is never destroyed, and a parcel that crossed a transform or a ridge
  is not being subducted.
- **Sample.** A cell no particle reached takes the nearest particle within
  `gap_radius` cell widths of its centre, preferring particles of its previous
  owner. This is a raster fill, not a material event.
- **Make floor.** No particle within `gap_radius`: one is created at the cell
  centre, oceanic, born this step, flat, on the cell's previous plate. This is
  the ridge, and it is the only place a run creates material.

`relabel_particles(from, to, scope)` is the one loop the lifecycle needs: a
suture takes the `WholePlate` of what it absorbs, and a rift takes only the
material in `OwnCells`, so the halves part carrying their crust and the arc
between them opens a real gap. A particle of a third plate stacked in one of
those cells is not the parent's to give away and keeps the plate it belongs to.

### Boundary deformation and relief decay

Each step computes the profile its own boundaries raise, scales it by
`step_duration / full_deformation_time`, and adds it to what each particle in
the cell carries, stacked ones included: material under a collision is being
deformed too. The increment comes before the material moves, because uplift
happens at the boundary.

What a parcel carries also decays. `accumulate` multiplies it by
`1 - step_duration / erosion_time` before that step's boundaries add to it, so
relief with no boundary under it falls toward zero and relief under one rises
to where uplift and decay balance. The kept fraction is the linear
`1 - dt/tau` rather than `exp(-dt/tau)`, which costs a multiply and a divide
instead of a libm call on a path a kernel would mirror; the two differ to
second order in `dt/tau`, about one part in a thousand at the defaults.

`erosion_time` defaults to thirty default steps. At 88 km cells the erosion
that matters is the denudation of a whole orogen rather than hillslope
diffusion or river incision, both of which live far below one cell. Ahnert's
1970 relation makes that rate proportional to mean relief, and proportional
loss is exponential decay; crust under a mountain rebounds isostatically as the
top is stripped, so surface elevation falls about six times more slowly than
rock leaves. An effective e-folding of 40 to 50 Myr is what leaves the
Appalachians standing, and at one default step per 1.5 Myr that is about thirty
steps. Infinity turns the sink off, the same convention `suture_time` uses.

`maximum_magnitude` defaults to 0.5 and is the steady state of a belt that
keeps converging rather than an accumulator overflowing: uplift and decay
balance well above it, so an active belt reaches the clamp and holds there
while a belt whose boundary moved on decays away from it.

The profiles are per boundary class, each an offset and a depth. `convergent`
is the collision mountain, offset 0.4 over six default hops, belt and all.
`transform` raises relief per unit of residual convergence rather than per unit
of shear, because pure lateral slip builds nothing and what a transform makes
comes from the small normal component at a bend. `trench` and `island_arc` take
the two sides of an ocean-ocean convergence.

### The drift walk

Kinematics is per-step state, not a fixed input. Without drift every step would
classify the same relative motion at a boundary that has not moved, and the
accumulated fields would record a scaled copy of the final state.

Each step, after deform and transport and before the lifecycle, every plate's
rotation vector takes one step: the axis turns through the fixed angle
`axis_drift_rate * sqrt(step_duration)` toward a fresh hashed direction
perpendicular to it, and the speed is multiplied by
`1 + s * speed_drift_rate * sqrt(step_duration)` for a hashed `s` in `[-1, 1)`.
Four `signed_f32` draws per plate per step supply both.

Because the angle per step is fixed and only the direction is hashed, an axis
takes a random walk on the sphere of directions, whose expected total wander
over `n` steps is roughly `theta * sqrt(n)`. Both rates are therefore per unit
*root* time: a rate against `sqrt(step_duration)` leaves a run's total wander
where it is when the run is sliced more finely, where a rate against the step
itself would halve the variance every time the step halved.

The speed walk is clamped to `speed_drift_limit` either side of the speed the
plate began with. A random walk against fixed global limits eventually piles
every plate against one of them; a band around where the plate started keeps
drift the perturbation of the fit that it is meant to be. "Began" is the run's
start for a plate the partition made, and the step it came into being for one a
rift or a suture made.

`axis_drift_rate` defaults to 1.8 and `speed_drift_rate` to 0.9, both per unit
root time, and `speed_drift_limit` to 0.5. The rates stay modest on purpose:
larger drift makes boundaries flicker between regimes from step to step and
blurs the accumulated fields into an average instead of a record.

The turn is the half-angle tangent form, so it costs only add, multiply, and
divide. Half the intended angle stands in for its tangent, which is the same
number to third order.

### Rifting and suturing

Both happen at the end of a step, after the drift, so the next step's
boundaries are the ones the new plate set implies.

**Rifting.** Each step, every plate whose continental area exceeds
`rift_minimum_area_fraction` of the sphere draws against
`rift_rate * step_duration`. A plate that rifts walks one arc of `cracks.rs`
from a hashed cell of its own, in both directions, stopping as soon as it
leaves the plate, so the arc runs from boundary to boundary and the cells it
crossed are the wall. The two largest connected components of non-wall cells
beside the arc are the halves; the wall and any leftover component join the
half they share the most edges with. Fewer than two components means the rift
separated nothing: the plate is left exactly as it was and the attempt is
counted as a failed rift. The larger half keeps the plate's id.

Both halves are continental and both keep the birth and deformation their cells
carried, because a rift is a line in the crust rather than new crust: the ridge
it becomes makes ocean cell by cell through the rebirth rule over the steps
that follow. The halves start from the parent's rotation vector plus and minus
`rift_opening_speed * normalize(n × m)`, for the wall's plane normal `n` and
its mean center `m`. That is the axis whose velocity at the rift is normal to
the rift, which makes the relative motion across the new boundary pure opening.
A closed arc is the one degenerate case: its wall's mean center is the sphere's
own, which leaves the opening direction undefined, so a rift is only well posed
on a plate an arc can cross rather than circle.

Eligibility is a plate's *continental* area, not its extent. The largest any
plate holds at the viewer's defaults is 0.0127 of the sphere, so the default of
0.012 makes four plates eligible at step zero where 0.014 and above make none.
`rift_rate` defaults to 3.0 per unit time and is swept against that minimum
rather than alone, because the two together decide the count. A half that a
rift leaves is usually too small to be eligible again, which is what stops the
count running away.

**Suturing.** Each step counts, for every adjacent continental pair, the shared
edges that are convergent with continental crust on both sides. A pair whose
front is at least `suture_minimum_shared_length` grows its collision time by
the step; a pair below it starts over. At `suture_time` the plate with more
area absorbs the other: every cell takes the absorber's id and the absorber's
rotation vector becomes the area-weighted mean of the two. The merged plate's
drift band restarts around that mean, because leaving it around the absorber's
original speed would snap the merge's own motion straight back.

**Compaction.** The run ends by removing every plate id owning no cell and
remapping ownership and the rotation vectors to `0..live_count` in id order.
Transport can empty a plate on its own, so this runs on every run rather than
only on one that sutured.

### Accretion and crustal thickness

A parcel of continent carries the original parcels it holds. Two rules change
that number.

A continent arriving under another continent at a trench merges into it: one
column of crust where there were two, standing where the incumbent stood. The
test is the trench rule's on continental material — a parcel of another plate's
continent, landing inside a plate that is converging on it. India goes under
Asia and the pair is one crust from then on. Compaction merges the continent of
a plate that has run out of cells into the column above it, which conserves the
same material by a different route.

Both exist to stop material disappearing, and what a run conserves is the sum
of thickness over continental parcels — an integer, and exact. Nothing reads
that number as an elevation. `cell_thickness` is a record of where continents
have collided, which the viewer draws as an overlay, and the mountains a
collision makes are what the `convergent` profile paints.

### Island arcs

`material_order` says which of two parcels of crust covers the other:
continental over oceanic, and among oceanic the younger over the older. `Equal`
is two continents, or two floors of one age, and means no polarity. Transport,
boundary deformation, and the volcanic arcs all read it, so the three cannot
disagree about who is on top.

Ocean-ocean convergence therefore takes a side: a trench on the older plate and
a narrow volcanic arc one or two cells behind it on the younger, which is the
shape of the Marianas, the Aleutians, Tonga, and the Lesser Antilles. An arc is
narrow — a volcanic front 100 to 200 km behind the trench — so its default
depth is two default hops rather than the six a collision belt spreads over.

### Interior relief

Boundary deformation reaches three to five cells from a boundary, so a plate
wider than about ten cells would be flat by construction. Two low-frequency
fields go into base elevation.

**Dynamic topography.** Where mantle flow converges the surface sags and where
it diverges it swells, so the term is the flow field's divergence over the
mesh, negated, divided by a fixed scale, times `dynamic_topography_amplitude`.
It applies to oceanic and continental crust alike. The divergence is the
discrete divergence theorem over each cell's Voronoi polygon. The fit and base
elevation sample one `FlowField` built from one config, so plate motion and the
sag under it describe the same flow.

**Continental basement.** Three octaves of the fixed-polynomial gradient noise
at `basement_frequency * 2^k`, amplitudes halving, on continental cells only,
scaled by `basement_amplitude`.

Defaults are 0.03, 0.05, and a frequency of 3.0, bounded so interior relief
cannot drown a continent on its own. The dynamic term is the larger of the two
and is what gives the deep ocean a gradient it otherwise has none of; because
it is normalised by the divergence field's RMS rather than its peak, it reaches
several times its amplitude at the most divergent cells, which is why base
elevation clamps to `[0, 1]` as the composition stage does.

### Sea level

Sea level is a datum `CoarseElevationConfig` carries and the composed
`CoarseElevation` carries onward, defaulting to 0.5. Every reader either holds
the field and asks `CoarseElevation::is_land`, or is handed the datum
explicitly. The elevation field is not interpretable without it, the same way a
`SphereMesh` is not interpretable without its radius, which is why it travels
with the field rather than being read back out of a config.

Raising the datum can only flood and lowering it can only expose. What it cuts
is whatever the field already puts near the datum: base elevation tapers the
continental base to a shelf edge over the outermost few cells, so the margin it
cuts across is a shelf rather than a cliff.

### Lengths, times, and the run

Every reach a stage walks is a model length on the unit sphere, converted to
hops once at the top of the stage through `procgen_sphere_mesh::hops`, so a
belt is as wide in kilometres on a fine mesh as on a coarse one. Every such
default is written as a multiple of `default_hop_length`, the width of one cell
of the 65,536-cell default mesh, so each converts back to the integer it
replaced exactly; a unit test per stage asserts that. `TRANSPORT_REACH_HOPS`
and `gap_radius` stay in hops, because they describe the raster rather than the
world; they are the only two that do.

A stage that counts features states a density per unit area or a fraction of
the sphere and measures the area it applies to, for the same reason. Birth and
age are model time, not step counts, and a run is a duration with the step
count derived from it: a finer mesh takes the shorter step its transport needs
and covers the same history in more of them, where a step count would have
covered less of it.

`step_duration` defaults to 0.014, the time a plate at the default maximum
angular speed takes to cross one cell width on the default mesh, and is bounded
above by `maximum_step_duration`: past it a step carries material further than
the trench rule looks.

The viewer's default run is 60 steps, 0.84 model time, about 90 Myr — the time
an ocean takes to open. It is the shortest run at which the age distribution,
the plate count, and land have all reached the steady states they hold out to
240 steps, and at which the deformation clamp still touches about one cell in
eighty rather than one in seven. The crate default in `PlateEvolutionConfig` is
a library default the fixtures scale from and is much shorter.

## Determinism

Kinematics is a float result and angular velocities are quantized to a
power-of-two grid. The fit uses add, multiply, divide, and square root, and its
output is quantized once on the host. Boundary classes and the ownership a cell
resolves to are integers derived from those quantized floats through
comparisons, so they stay bit-identical run to run and across the two
development machines.

One libm call remains in reach of the angular velocities:
`RandomStream::unit_vector` takes a sine and a cosine, and the hashed axis is a
fifteen-percent share of every plate's direction at the default coherence. If a
re-pinned integer fingerprint ever splits between the two machines, that is the
first thing to replace, with a normalized triple of `signed_f32` as the crack
walk already uses.

Nothing else on these paths calls libm. The drift's turn, a particle's
rotation, and the speed rule are add, multiply, divide, and `min`; a particle's
location is dot and cross products in f64 on unit directions; a cell's
resolution is comparisons with integer tie-breaks; the lifecycle adds an arc
walk, connected components, integer edge tallies, and area sums, all exact,
plus one plane fit and one cross product.

A run's meaning does not depend on how the run is sliced, and one case of that
is exact. Take a world whose every rotation vector is zero, with drift and the
lifecycle off, and run it at `(step_duration, n)` and at
`(step_duration / 2, 2n)`. Halving a float and doubling a count are both exact,
so the two runs cover the same elapsed time to the bit; nothing moves, so every
cell keeps its own particle; and crust birth, seafloor age, and base elevation
come out bit-identical. `slicing_a_still_run_twice_as_finely_changes_nothing`
is that statement. A moving world cannot make the claim, and the linear decay
factor is one reason why.

Accumulated deformation and particle positions are float fields, tested for
run-to-run equality and invariants only, never pinned across machines. A float
field that does carry a fingerprint is pinned through `quantized_fingerprint`,
which hashes each value's step on a 1/1024 grid rather than its bits: scaling
by a power of two is exact, so the grid step is an integer fact about the
value, and the step is four orders of magnitude coarser than the last bit of an
`f32` near one. Pinning `to_bits()` instead would pin the toolchain, because
one differing last bit changes the whole hash.

## Decisions

- Keep rigid rotations. Plates are rigid to first order, and the Euler-vector
  model gives boundary classification for free.
- Fit to a field rather than simulate forces. A real force balance would need a
  mantle model; a smooth field with a crust factor gets the large-scale pattern
  at a fraction of the complexity.
- Keep evolution as simultaneous steps. Everything is a Jacobi update: every
  cell resolves against the ownership the step began with, and the new
  ownership is written only once every cell has been decided. That is what
  keeps a GPU mirror well-defined.
- Birth time, not age, is the stored fact; age is derived against the run's
  elapsed time. Store one fact and derive the rest.
- A slice lands only with a before-and-after screenshot of the viewer's default
  world showing a difference you can see. A slice whose only evidence is a
  table is a measurement, and a measurement is not a change.

## Non-goals

- No mantle convection model, no force balance, no plate-boundary forces, no
  ridge push, no basal drag.
- No moving cells. The mesh is fixed: cells change owner and read what the
  material lying in them carries. The material does move, as particles under
  each plate's own rotation.
- No history retained per step in the output beyond the accumulated fields. The
  viewer shows the end state and aggregate diagnostics.
- No change to the partition stage.
