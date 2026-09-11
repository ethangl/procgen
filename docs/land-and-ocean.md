# Land and ocean

## Goal

Make the coastline a consequence rather than a decision. Today the crust
stage marks whole plates continental or oceanic until a target ocean area is
met, base elevation puts every continental cell at one height, and the coast
is wherever that height crosses a sea-level datum. The datum became a
configured value in the first step of this work; this document is about the
second: crust that belongs to cells rather than plates, so that a plate can
carry a continent and an ocean, a continent's edge is a shelf that floods and
drains with sea level, and the ocean fraction is something the world has
rather than something the settings ask for. All three slices have landed.

The measure of success is visual and mechanical at once: passive margins
inside plates, continental shelves that appear when sea level rises a little
and vanish when it falls, and a single sea-level slider that behaves like the
real one.

## Current state

- `classify_crust` grows `nucleus_count` continental nuclei to
  `continental_fraction` of the sphere and calls every cell they reached
  continental. Plates have no crust class: `plate_classes`, `plate_count`, and
  `ocean_fraction` are gone, and the readers that want a plate-level number
  take the derived `plate_continental_fraction`. That was the second slice of
  this work; the measurements are below.
- Since the displacement-migration slice, a cell's crust during and after
  evolution is read from its birth step: `Some` is oceanic, `None` is original
  continental crust. The classification is now purely the initial condition
  the birth prior reads, and evolution takes no classification at all.
- Base elevation tapers `continental_base` down to `margin_edge_elevation`
  over the outermost `margin_width_hops` continental cells before it adds the
  interior relief terms, so a continent ends in a shelf that the sea-level
  datum floods and drains one hop at a time. That was the third slice of this
  work; the measurements are below.
- Sea level is a configured datum carried by every elevation field. The
  borrowed `ElevationField` view pairs a stage's normalized elevations with the
  datum they were composed against, geological elevation and isostatic
  adjustment carry it alongside the tectonic field, and climate's inputs and
  the viewer's surface take the field rather than a vector and a loose `f32`.
  That was the first slice of this work.

## Design

Three slices, in the order they should land. The first is preparation that
the second needs and that removes plumbing the first sea-level step left
behind. The second is the change itself. The third is what makes it visible.

### 1. Elevation fields carry their datum (landed)

`CoarseElevation` carried `sea_level`. Geology's `GeologicalElevation` and
`IsostaticAdjustment` produced elevation fields against the same datum but as
bare `Vec<f32>`, so climate took `final_elevation: &[f32]` and a separate
`sea_level: f32` on four input structs.

`procgen-tectonics` now owns `ElevationField<'a>`, a borrowed
`cell_elevations` and `sea_level` with `validate(mesh)`, `is_land(cell)`, and
`land_cell_count()`, in the shape of the existing `CellCrust` view. Every
stage that produces normalized elevation owns the datum and lends the view
through `field()`: `CoarseElevation`, `GeologicalElevation`, and
`IsostaticAdjustment`. Geology's craton and basin stages take the view instead
of the whole tectonic result, climate's four input structs take it in place of
their slice and scalar, and `Surface::at(field, cell)` replaces
`Surface::from_elevation(value, datum)`. Atmospheric circulation keeps a bare
slice, because it reads elevation gradients and never asks where the ocean is.
The viewer's `surface_elevations()` returns the field, so its surface mesh,
palettes, and terrain-tile datum come from the field they draw rather than
from tectonics by hand. Nothing changed numerically; the fingerprints prove
it.

### 2. Per-cell continental crust (landed)

Replace the per-plate classification with a per-cell one, produced before
kinematics and independent of plate boundaries.

**Nuclei.** `nucleus_count` continental nuclei (default around 8, the order
of Earth's cratonic assemblies) are seeded farthest-first from a hashed first
cell, using the same farthest-first helper the partition uses. They ignore
plates entirely.

**Growth.** The nuclei grow by the partition's shortest-arrival growth with
per-edge integer costs and the configured roughness, with no face mask, until
the continental area reaches `continental_fraction` of the sphere (0.3 when
this slice landed, matching the ocean fraction of 0.7 it replaced; slice 3
retuned it to 0.347 to pay for the shelf it floods). Growth stops at the
first settled cell that carries the total past the target, so the achieved
area overshoots by at most one cell. Cells the growth never reached are
oceanic. This gives blobby continents with rough coasts whose edges fall
wherever they fall relative to plate boundaries, which is what produces
passive margins.

**Output.** `CrustClassification` becomes `cell_classes: Vec<CrustClass>`
with `validate(mesh)`, `class(cell)`, and a derived
`plate_continental_fraction(partition, mesh) -> Vec<f64>` for the readers that
need a plate-level number. Plate classes are gone. The `CellCrust` view that
reads crust from birth stays as the answer during and after evolution; the new
classification is what the birth prior reads to decide which cells are
oceanic at step zero.

**Readers.** Each of the five per-plate readers becomes what it meant:

- Kinematics speed factor: interpolate between the oceanic and continental
  factors by the plate's continental area fraction.
- Rift eligibility: the plate's continental area, not its total area, exceeds
  the minimum fraction. The arc still walks the whole plate.
- Suture: the shared edges' cells are continental on both sides, which the
  test already checks; the plate-class check is deleted.
- Migration precedence: the two edge cells' crust, continental over oceanic.
- Crust-birth prior: continental cells are `None`; oceanic cells take the hop
  distance to the initial ridges as today.

Volcanic arcs group by the overriding plate where the overriding cell is
continental, which is the cell-crust reading of "overriding continental
plate". Cratons, basins, hotspots, and deformation already read cell crust.

**Ocean fraction becomes a diagnostic.** The crust stage reports the achieved
continental area. Land fraction at the current sea level is reported by the
elevation field, where it already is.

### 3. Continental margins (landed)

A continent's edge is a shelf, not a cliff. Base elevation lowers
`continental_base` toward the ocean over the outermost `margin_width_hops`
continental cells (three as shipped, not the two this design sketched),
reaching `margin_edge_elevation` at the coast (0.46, not the 0.45 sketched
here). Hop distance from the nearest oceanic cell comes from
`multi_source_distances` seeded on every oceanic cell, restricted to nothing,
since a shelf may span a plate boundary. The taper is linear in hops and
exact.

Three hops at an edge of 0.46 put the margin cells at 0.46, 0.5233 and
0.5867 against a `continental_base` of 0.65. With the datum at 0.5 the
shelf's outer cell is flooded and the coast sits one cell inland of the crust
boundary. Raising sea level to 0.55 floods the second cell as well, and the
low interiors dynamic topography made; lowering it to 0.45 exposes the whole
shelf so the coast and the crust boundary coincide. That is the behaviour the
sea-level slider was for. `is_land` is strict, so a cell exactly at the datum
is ocean.

Interior relief, deformation, and geology apply on top as they do now; a
shelf under a convergent boundary still gets its collision profile.

## Determinism

Nuclei seeding, growth, hop distances, and area sums are integer or exact,
so the continental mask is bit-identical across machines and is pinned by
fingerprint. The margin taper is a rational of two integers. Kinematics stays
a float output, never pinned; every integer downstream of it is decided by
comparisons on values the previous slices already made exact. Fingerprints of
ownership, birth, seafloor age, and the geology fields will change at the new
defaults, once, when slice 2 lands.

## Decisions

- Crust type stays. It is a material property that drives which side
  overrides, what a boundary builds, where new ocean floor appears, and what
  can rift or suture. Erosion will need it too. What goes is assigning it by
  plate.
- Grow nuclei rather than threshold a noise field. Growth gives exact control
  of the continental area, reuses the partition's engine and roughness, and
  produces the same rough-edged blobs the plates have, so the two structures
  look like they belong to one world.
- Continents ignore plate boundaries on purpose. A continent that ends inside
  a plate is a passive margin, which is most of Earth's coastline. Continents
  that happen to end at a boundary are active margins. Both arise without a
  rule.
- The margin is a base-elevation term, not a geology stage. It is what a
  continental edge is at rest, before anything acts on it, and base elevation
  is where that lives.

## Non-goals

- No isostatic model of crust thickness. Thickness is two classes and a
  margin taper, not a continuous field. If a continuous field is ever wanted,
  it replaces the class, not the other way round.
- No ocean volume or water budget. Sea level is a datum the user sets.
- No change to the partition, the flow field, or evolution's step structure.
- No erosion. Shelves and coasts are where erosion will act, which is why this
  lands first.

## Slices

### Elevation fields carry their datum. (landed)

Geology's elevation outputs gained the datum and the land queries; climate's
inputs take the fields and dropped their `sea_level` scalars. No numeric
change.

### Per-cell continental crust. (landed)

Nuclei, growth to a continental area, `cell_classes`, the derived plate
fraction, the five readers rewritten, volcanic-arc grouping, ocean fraction as
a diagnostic, viewer controls and summary, pins re-set once.

The growth engine gained a second entry point rather than a mode: the
partition still calls `PlateGrowth::grow`, and crust calls `grow_to_area`,
which settles cells one at a time until their total area passes the target.
The engine's face restriction became a `GrowthBounds` enum, `WithinFaces` for
the partition and `WholeSphere` for crust, and its cost stream became the
caller's, so a `CRUST_GROWTH_COST` stream keeps the two cost fields
independent even at the same seed. The partition's fingerprint is unchanged,
which is what proves the engine did not move.

Two consequences beyond the plan. The volcanic arc's inland walk had relied on
the overriding plate being wholly continental; it now stops at that plate's
own coast as well as at its boundary, or an arc would sit on ocean floor.
And rift eligibility on continental area alone is a real tightening: at the
viewer's default 111-plate partition the largest continental area any plate
holds is 1.3% of the sphere, well under `rift_minimum_area_fraction` of 0.04,
so the default world no longer rifts at all, while the coarser reference world
below (eighteen plates) still rifts once. Lowering that default to about 0.01
would restore it; a default retune is its own change with its own
measurements.

**Measured.** Both worlds are the viewer's defaults at 65,536 cells: 15
evolution steps for the first, and the reference world's sampling seed 9,
subdivided faces 0.2, and 30 steps for the second. Both use the viewer's
`continental_fraction` of 0.25 rather than the crate default of 0.3, which is
the same 0.05 the viewer took off the crate's ocean fraction before, so the
table isolates the change of algorithm from a change of target area. "Plate"
is the per-plate classification this slice replaced, run through the same
pipeline.

| Measure                       | Defaults, plate | Defaults, cell | Reference, plate | Reference, cell |
| ----------------------------- | --------------- | -------------- | ---------------- | --------------- |
| Plates                        | 111             | 111            | 18               | 18              |
| Continental area fraction     | 0.2501          | 0.2500         | 0.2419           | 0.2500          |
| Continental components        | 111 plates      | 8              | 18 plates        | 8               |
| Coast edges at step zero      | 4,292           | 4,998          | 1,912            | 4,830           |
| Coast edges inside a plate    | 0%              | 93.6%          | 0%               | 97.7%           |
| Land cells at sea level 0.5   | 18,829          | 15,421         | 18,934           | 15,699          |
| Rifts / sutures               | 2 / 0           | 0 / 3          | 1 / 1            | 1 / 1           |

The passive-margin fraction is the number the design promised: nineteen coast
edges in twenty lie inside a plate rather than on a plate boundary, where
per-plate crust could not put a single one there. Land at sea level 0.5 falls
by about eighteen percent, because the achieved area is a share of the sphere
and the grown continents take in more small cells than whole plates did.
Slice 3's margin taper lowers it again; that retune belongs to slice 3.

### Continental margins. (landed)

`BaseElevationConfig` gained `margin_width_hops` (3) and
`margin_edge_elevation` (0.46). `derive_base_elevation` seeds
`multi_source_distances` on every oceanic cell of the final `CellCrust`,
restricted by nothing, and a continental cell at hop `h` in `1..=width` takes
`edge + (base - edge) * (h - 1) / width` in place of `continental_base`
before the dynamic topography and basement terms are added, so a shelf
carries the same interior relief as the plateau behind it. Because the
distance is measured against the final cell crust rather than the initial
classification, the ocean a rift opened during the run gets shelves on both
of its sides like any other coast, which a test asserts. Width zero restores
the pre-slice cliff exactly, including its fingerprint. The interior relief
terms moved to their own `interior_relief` module in the same change to keep
`base_elevation.rs` under a thousand lines.

`BaseElevationDiagnostics` gained `margin_cell_count` and a `margin_depth`
summary of how far below `continental_base` the taper put each margin cell.
The viewer gained a margin width drag value and a margin edge slider in the
base elevation section, and both diagnostics in its summary.

**The land the shelf floods was paid for.** Flooding the outer shelf cell
removes a ring of land around every continent, so `continental_fraction` was
retuned by the 0.047 that restores the land count: the crate default from 0.3
to 0.347 and the viewer's setting from 0.25 to 0.297, keeping the same 0.05
the viewer has always taken off the crate.

**Measured.** Both worlds are 65,536 cells at the viewer's defaults: sampling
seed 7 and 15 evolution steps for the first, and the reference world's
sampling seed 9, subdivided faces 0.2, and 30 steps for the second. "Before"
is the pre-slice pipeline — no taper, `continental_fraction` 0.25 — and
"after" is what shipped. Land counts are on tectonic elevation, which is
where the viewer reports them.

| Measure                      | Defaults, before | Defaults, after | Reference, before | Reference, after |
| ---------------------------- | ---------------- | --------------- | ----------------- | ---------------- |
| `continental_fraction`       | 0.25             | 0.297           | 0.25              | 0.297            |
| Continental cells            | 14,316           | 17,046          | 14,192            | 18,111           |
| Margin cells                 | 0                | 12,417          | 0                 | 10,491           |
| Land cells at sea level 0.45 | 16,692           | 17,790          | 16,900            | 19,371           |
| Land cells at sea level 0.50 | 15,421           | 15,379          | 15,699            | 17,311           |
| Land cells at sea level 0.55 | 14,484           | 12,956          | 14,443            | 15,220           |
| Rifts / sutures              | 0 / 3            | 0 / 3           | 1 / 1             | 2 / 2            |

The retune was set at the viewer's defaults, where land at the datum lands
within 0.3 percent of where it stood: 15,379 against 15,421. The reference
world is 10 percent over its own before-figure, because a target area is a
share of the sphere and eighteen large plates hold their extra continent
differently from a hundred small ones; the retune restores one world exactly
and the other approximately, which is what one knob can do.

The datum now does visible work. Across the three datums land moves by 4,834
cells at the defaults and 4,151 at the reference world, against 2,208 and
2,457 before — and before, almost all of that movement was on the oceanic
side, because a continent was a cliff.

**Every coast edge's landward cell is a margin cell** where the taper alone
decides the coast: on base elevation with interior relief off, 8,156 of 8,156
coast edges at the defaults and 6,784 of 6,784 at the reference world. With
interior relief on it is 8,373 of 8,467 and 7,170 of 7,170; the ninety-four
exceptions are inland lows the basement dug below the datum. On tectonic
elevation, after deformation and smoothing, it is 9,477 of 11,229 and 8,024
of 10,483, because a trench floods land the taper never reached and an
orogenic belt pushes a coast out to sea. The margin depth spans 0.0633 to
0.19 at both worlds, the full drop from `continental_base` to the shelf edge.

**Integer pins re-set.** The `continental_fraction` retune moves everything
downstream of the crust mask, once: the crust classification fingerprint, the
birth prior's diagnostics and fingerprint, seafloor age, ownership and birth
after a run and after a run without drift or lifecycle, one migration step,
and geology's craton, volcanic-arc, and hotspot-province fingerprints. The
reference run's aggregates moved with them — 261 proposals over 225 migrated
cells against 254 over 218, one suture rather than two, 28 final plates
rather than 24 — and the hotspot fixture now draws three provinces over 22
cells rather than two over 13. Base elevation's own fingerprint did not move:
it is pinned at width zero and the pre-slice `continental_fraction`, which is
the test that proves the taper is the only change to the field.
