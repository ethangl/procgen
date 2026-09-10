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
rather than something the settings ask for.

The measure of success is visual and mechanical at once: passive margins
inside plates, continental shelves that appear when sea level rises a little
and vanish when it falls, and a single sea-level slider that behaves like the
real one.

## Current state

- `classify_crust` hashes plates into an order and flips them to oceanic
  until `target_ocean_fraction` of the sphere's area is oceanic. Every cell of
  a plate starts with its plate's class. Earth is not like this: the South
  American plate is half continent and half Atlantic floor, and the join is a
  passive margin, not a plate edge.
- Since the displacement-migration slice, a cell's crust is read from its
  birth step: `Some` is oceanic, `None` is original continental crust. That is
  already a per-cell fact, and a rifting continent already grows an oceanic
  margin. The per-plate assignment is now the odd one out, and it survives in
  five readers: the kinematics speed factor, rift eligibility, the suture
  test's plate-class check, migration precedence, and the crust-birth prior's
  choice of which cells are oceanic at step zero.
- Base elevation gives every continental cell `continental_base` plus the
  interior relief terms, so a continent is a plateau with a cliff at its edge.
  There is no shelf: a coast is a one-cell step from about 0.65 to the
  cooling curve.
- Sea level is a configured datum carried by every elevation field. The
  borrowed `ElevationField` view pairs a stage's normalized elevations with the
  datum they were composed against, geological elevation and isostatic
  adjustment carry it alongside the tectonic field, and climate's inputs and
  the viewer's surface take the field rather than a vector and a loose `f32`.
  That was the first slice of this work.
- The raster pilot classifies crust per plate on the GPU and is unaffected by
  anything here.

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

### 2. Per-cell continental crust

Replace the per-plate classification with a per-cell one, produced before
kinematics and independent of plate boundaries.

**Nuclei.** `nucleus_count` continental nuclei (default around 8, the order
of Earth's cratonic assemblies) are seeded farthest-first from a hashed first
cell, using the same farthest-first helper the partition uses. They ignore
plates entirely.

**Growth.** The nuclei grow by the partition's shortest-arrival growth with
per-edge integer costs and the configured roughness, with no face mask, until
the continental area reaches `continental_fraction` of the sphere (default
0.3, matching the current default ocean fraction of 0.7). Growth stops at the
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

### 3. Continental margins

A continent's edge is a shelf, not a cliff. Base elevation lowers
`continental_base` toward the ocean over the outermost `margin_width_hops`
continental cells (default 2), reaching `margin_edge_elevation` at the coast
(default about 0.45, so the outermost cell sits just below the default sea
level and the next one just above it). Hop distance from the nearest oceanic
cell comes from `multi_source_distances` seeded on every oceanic cell,
restricted to nothing, since a shelf may span a plate boundary. The taper is
linear in hops and exact.

With the datum at 0.5 the shelf's outer cell is flooded and the coast sits one
cell inland of the crust boundary. Raising sea level to 0.55 floods the second
cell and the low interiors dynamic topography made; lowering it to 0.45
exposes the shelf so the coast and the crust boundary coincide. That is the
behaviour the sea-level slider was for.

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
- The raster pilot keeps its per-plate crust kernel until its own slice. The
  mesh path is canonical, and the two will disagree about crust until then,
  which the pilot's doc records.

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

### Per-cell continental crust.

Nuclei, growth to a continental area, `cell_classes`, the derived plate
fraction, the five readers rewritten, volcanic-arc grouping, ocean fraction as
a diagnostic, viewer controls and summary, pins re-set once.

### Continental margins.

The base-elevation taper over the outermost continental cells, its two
config values, and the measured coast movement at sea levels 0.45, 0.50, and
0.55 on the reference world (sampling seed 9, subdivided faces 0.2, 30 steps).
