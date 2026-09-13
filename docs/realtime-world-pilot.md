# Real-time procedural world pilot

Status: slice 1 is implemented. Slices 2–5 remain proposals.

## Purpose

Build a small exploration prototype that generates a spherical world as the
camera moves. Its terrain must have varied shapes, caves, and overhangs. The
same place must remain the same after travel, eviction from memory, and reload.
Generation must fit within bounded memory and execution budgets.

The pilot tests two ideas together:

1. A world can be a set of deterministic, local functions. Generate only the
   data needed to render and interact with the current surroundings.
2. Terrain variety comes from control over shape, scale, and the interaction
   between noise layers. More octaves alone are not sufficient.

The design sources are [McKendrick's transcript](gdc-mckendrick.txt) and
[Murray's transcript and appended slide extracts](gdc-murray.txt). This is a separate pilot, with no
dependency on the project's other world-generation designs. The transcripts
describe a shipped system, but do not supply its complete implementation.
All decisions below are pilot proposals unless identified as talk evidence.

The surface-extraction section also uses two supplied technical references:
[Ronen Tzur's Contouring Implicit Surfaces](https://www.oocities.org/tzukkers/isosurf/isosurfaces.html)
and [the simplified dual-contouring implementation by ejpbruel](https://github.com/ejpbruel/ejpbruel.github.io).
These explain implementation choices; they do not establish which choices
McKendrick's shipped system used.

## What the talks establish

| Concept                                                                                          | Source                                                  | Consequence for the pilot                                                         |
| ------------------------------------------------------------------------------------------------ | ------------------------------------------------------- | --------------------------------------------------------------------------------- |
| Runtime generation permits only local knowledge of the world.                                    | McKendrick 07:19–08:40, 34:40–35:22; Murray 30:40–32:06 | A density query must not depend on generated neighboring regions.                 |
| Simulate on a sphere; use a cube projection to store terrain data.                               | McKendrick 10:50–21:35                                  | Separate world positions from storage addresses. Evaluate shapes in world space.  |
| A broad elevation field offsets a limited voxel band.                                            | McKendrick 20:03–21:35                                  | Separate mountain-scale displacement from local volumetric detail.                |
| Coarse coverage arrives before detailed terrain.                                                 | McKendrick 23:58–30:56                                  | Schedule visible coverage first and keep it until a replacement is ready.         |
| Generators have inspectable intermediate data.                                                   | McKendrick 31:36–33:14                                  | Save resolved world parameters as well as the seed.                               |
| Plain noise and realistic terrain statistics did not provide the desired exploration experience. | Murray 19:14–25:51                                      | Judge terrain at walking and flight speeds, as well as in overhead images.        |
| Derivatives, domain warping, shape blending, and octave emphasis provide local control.          | Murray 27:08–40:09                                      | Build a bounded terrain function with controls that have visible effects.         |
| Surface fields and 3D density work together.                                                     | McKendrick 35:28–39:30; Murray 40:28–42:37              | Use surface fields for broad structure and volume fields for caves and overhangs. |
| Collision, navigation, and population can cost more than terrain generation.                     | McKendrick 45:49–49:18                                  | Measure the cost of making terrain usable, not only the cost of sampling noise.   |
| Fixed locations and broad automated sampling are both needed.                                    | McKendrick 52:04–53:43; Murray 44:22–45:22              | Keep repeatable performance routes and a separate search for unusual worlds.      |

## Pilot scope

The target experience is one planet with continuous travel from its surface
to a distant view and back. A user can inspect terrain, move on a collision
surface, fly, change generation parameters, and return to a saved location.
Sparse rocks or plants and one type of landmark test placement and streaming.

The pilot does not include a galaxy, interplanetary travel, multiplayer,
creature AI, navigation meshes, resource economies, or persistent terrain
edits. These are separate extensions after terrain, streaming, and collision
work together. Rivers in this pilot are terrain forms or water-filled channels;
the generator does not establish drainage networks or simulate water flow.

Start with a CPU reference and a small consumer application. Parallelize
independent generation jobs. Add GPU generation only after measurements identify
a useful stage to move. Any compute path uses wgpu on Metal and Vulkan, with
explicit backend selection and no automatic fallback. The CPU result remains
the reference. This platform choice is a repository constraint, not a claim
about the talks.

Do not select a new engine or design a reusable framework as part of this
document. Keep field evaluation, region generation, and scheduling separate
from rendering. Establish concrete modules before extracting shared APIs.

## World identity and local evaluation

The generation hierarchy is:

```text
world seed + generator version
    -> resolved planet parameters
    -> surface controls at a direction
    -> density and material at a position
    -> region samples
    -> render mesh, collision, and placement
```

Planet parameters describe radius, the elevation envelope, sea level, terrain
shape controls, and placement rules. They are small enough to resolve before
local generation starts. Save this data so a failure can be reproduced without
rerunning the earlier parameter selection.

A field query uses only its explicit position, resolved parameters, and stable
seed streams. It must not use camera position, job order, cache contents, or a
shared mutable random sequence. Separate streams identify terrain, caves, and
placement so a change to one does not consume another's random values.

Local evaluation does not prohibit all nearby samples. Meshing can sample a
fixed border, and a query can evaluate a bounded set of nearby candidates.
Neither operation may recursively request generated neighboring regions.
The cost of a query must have a known limit.

Cache keys include the generator version, resolved parameter identity, region
address, resolution, and backend where results differ. Editing parameters
creates a new generation revision. Results from older jobs cannot enter the
current world. A seed alone is not a promise that a future generator produces
the same terrain.

## Coordinates and terrain storage

Use planet-centered positions for world identity and a local origin for
rendering and physics. Radial direction defines up. Camera and movement code
must not assume a fixed world Y axis. Origin changes must preserve field
queries, landmark identities, and collision positions.

The precision scheme must be selected with the target radius and smallest
feature size. Do not upload large absolute positions and expect fine detail
to survive conversion to floats. Host-side world addressing can use integer
coordinates with a local offset; a mirrored kernel receives bounded local
coordinates and the stable address needed by the field evaluator.

Store terrain in regions on six cube faces, with a radial coordinate relative
to a broad elevation field. For planet-centered position `p`, define:

```text
direction = normalize(p)
altitude = length(p) - planet_radius
band_position = altitude - broad_elevation(direction)
```

The cube face and its surface coordinates address storage. They do not define
independent noise domains. Shared face boundaries need a canonical ownership
rule, matching sample positions, and explicit neighbor transforms. Test face
edges and corners from the start.

The broad elevation field allows tall mountains without allocating a voxel
volume from the planet center to the highest peak. The voxel band holds local
surface variation and caves. Its depth and height are model limits: terrain
outside this envelope is not representable by this storage scheme.

Require density to be solid at the band's lower boundary and empty at its
upper boundary. Bound detail and cave operations accordingly. A preset that
cannot satisfy this contract fails validation; do not clip its terrain at
runtime. Water is a separate spherical surface at sea level, so it can appear
above a low terrain band.

McKendrick gives approximately 128 m for the band and 32 m for a nearest-detail
region at 20:26–22:39. These are reference measurements from that system, not
pilot defaults. Choose dimensions from measured memory, generation cost, and
the desired cave size.

## Terrain shape function

### Surface controls

Evaluate broad elevation and shape controls over the sphere before filling
each radial column. Reuse those values within the column. This follows the
surface-first organization described at McKendrick 35:28–37:19.

Controls should describe what the user sees:

| Control                                  | Intended effect                                               |
| ---------------------------------------- | ------------------------------------------------------------- |
| Feature wavelength and amplitude         | Set the spacing and height of broad terrain forms.            |
| Shape blend                              | Blend base noise toward squared noise or ridges.              |
| Warp amplitude and wavelength            | Bend features and reduce visibly regular patterns.            |
| Slope detail                             | Suppress contributions using accumulated derivatives.         |
| Altitude detail                          | Adjust amplitude using the accumulated noise value.           |
| Ridge detail                             | Adjust amplitude using a separate derivative accumulator.     |
| Octave emphasis                          | Control amplitude progression through gain and amplification. |
| Cave size, depth, and occurrence density | Control local underground space.                              |

Introduce controls only with a working visual example. Begin with a few named
presets, such as rounded hills, ridged mountains, and broad basins. Use smooth,
low-frequency fields to vary selected controls within a planet. Independent
random parameters at every sample would destroy spatial structure.

### Noise composition and derivatives

Murray calls the combined approach “Uber noise.” The appended slides supply
its signature and several update equations. Use these as the reference for
the pilot's shape experiment. They do not supply a complete function body.

The signature at **37:24** takes a 3D position, octave count, feature
perturbation, sharpness, feature amplification, altitude erosion, ridge erosion,
slope erosion, lacunarity, and gain. It returns a `float`; the shown signature
does not expose the final analytical derivative described in the talk.

The equations below use short aliases for the slide identifiers and normalize
obvious extraction errors such as `1fFeatureNoise` and `1.Of`. They preserve
the displayed operations. Each block is a separate slide fragment, not a
complete octave loop to concatenate and execute.

**Sharpness, 37:55.** Let `n` be the feature noise before shaping and `s` be
sharpness. Compute both targets before either blend:

```text
ridged = 1 - abs(n)
billow = n * n
feature = lerp(n, billow, max(0, s))
feature = lerp(feature, ridged, abs(min(0, s)))
```

For `s` between -1 and 1, negative values select ridges, positive values
select squared noise, and zero preserves the base noise. This interval is the
pilot's proposed blend range; the signature does not declare valid ranges.
The square is significant: the earlier spoken explanation uses absolute
value for billow, but the displayed implementation uses `n * n`.

**Slope erosion, 38:11.** With `d` standing for the slide's `lDerivative`:

```text
slope_sum += d * slope_erosion
slope_weight = 1 / (1 + dot(slope_sum, slope_sum))
sum += amplitude * feature * slope_weight
```

This weight suppresses a contribution as the accumulated vector's magnitude
increases. It is not a geometric slope measurement from neighboring voxels.
The source does not show whether `d` is taken before or after feature shaping,
or which frequency and warp derivative factors it includes.

**Altitude damping, 38:25.** The later slide shows the contribution using a
damped amplitude, followed by amplitude updates:

```text
sum += damped_amplitude * feature * slope_weight
amplitude *= lerp(current_gain,
                  current_gain * smoothstep(0, 1, sum), altitude_erosion)
damped_amplitude = amplitude *
    (1 - ridge_erosion / (1 + dot(ridge_sum, ridge_sum)))
```

Treat this contribution as a refinement of the slope-only fragment, not a
second addition. Altitude damping depends on the updated accumulated noise
`sum`, not directly on radial altitude or the derivative. Ridge damping is a
separate operation using `ridge_sum`. Its accumulation rule is not shown.
The initial amplitudes and the full loop order still need to be specified.

**Domain perturbation, 39:00**, uses accumulated derivatives to offset the
sample position:

```text
octave_position = position * frequency + perturb_sum
perturb_sum += d * perturb_features
```

The displayed position uses the accumulator before this update. The frequency
update and the computation of `d` between these steps are not supplied.

**Feature amplification, 39:11**, shows:

```text
current_gain = gain + amplify_features
```

This establishes an additive adjustment to the gain used above. It does not
establish an independent weight for every octave or how the first octave is
initialized. Measure each control's effect before exposing it in the pilot UI.

### Derivative contract

Return a value and its spatial gradient where an analytical derivative is
supported. Each operation must account for its effect on that gradient:
frequency scaling, shape transforms, spatially varying weights, and the
Jacobian of a domain warp. Define behavior at ridge cusps. A base-noise
derivative is not automatically the derivative of the composed terrain.

Derivative-driven feedback needs particular care. If an octave's weight
depends on the previous gradient, an exact derivative of that weight can
require second derivatives. Settle this contract before calling the combined
output analytical: either implement the necessary derivatives or identify
the feedback quantity as a shaping signal and compute final surface normals
separately. The slides use derivatives as shaping inputs, but do not show
the final derivative calculation. Treat the shown accumulators as shaping
signals until the composed derivative has been derived and checked. Keep
them separate from the density gradients used by dual contouring.

Slope and altitude shaping imitate the appearance of erosion. They do not
transport material or establish a drainage history. Likewise, warped channels
are not proof of connected rivers. This distinction limits what downstream
features can infer from the terrain.

### Volumetric detail

Use the surface elevation as a base solid boundary. Add bounded 3D structure
for cliff variation and overhangs, then subtract cave shapes. State one density
sign convention throughout: positive is solid, negative is empty, and zero is
the extracted surface.

Attenuate additive detail with altitude so the upper band becomes empty.
This controls the envelope; it does not prove that every solid component is
attached to the ground. Floating fragments are an explicit visual test.

For the first cave experiment, use deterministic feature candidates that
contain both an underground shape and an entrance shape tied to the same
surface query. This is a proposed bounded solution to the entrance problem
Murray raises at 42:14–42:37. It promises neither a connected cave network nor
an entrance for every void made by arbitrary noise.

Density also supplies material weights. Keep the number of render material
contributions small and explicit. Begin with rock and soil, triplanar mapping,
and a separate water surface. Detailed texture synthesis is not needed to
evaluate terrain shape.

## Regions, meshes, and levels of detail

Generate region samples with a fixed border for surface extraction and normals.
Derive border width from the extractor's needs. Density queries at a shared
position must agree regardless of which region requests them.

Use dual contouring as the initial extraction candidate because the talk
identifies sharp-feature preservation as its reason for moving away from
marching cubes (McKendrick 39:55–40:56). First test it on a small volume with
sharp shapes, cave mouths, and nearly flat gradients. Vertex constraints and
degenerate cases are part of that test, not assumed solved by the transcript.

### Dual-contouring experiment

McKendrick's term at 40:51 is **mass points**, confirmed against the video by
the user. Tzur defines a mass point as the average of a cell's surface-edge
intersections. His implementation translates these intersections around that
point before solving for the vertex, then restores the translation. Dual
contouring uses intersection positions and normals to fit a vertex per active
cell, and connects cell vertices around shared sign-changing edges.
See [Tzur's explanation](https://www.oocities.org/tzukkers/isosurf/isosurfaces.html).

The [reference demo's index.js](https://github.com/ejpbruel/ejpbruel.github.io/blob/master/index.js)
shows the steps separately: interpolate edge crossings, estimate normals by
central differences, center the solve on the intersection centroid, solve by
singular value decomposition, and connect vertices on a uniform grid. Its
solver discards small singular values. These are useful reference steps;
the fixed numerical thresholds are not pilot defaults.

For intersection positions `p_i` and unit normals `n_i`, use the quadratic
error function (QEF):

```text
E(x) = sum_i (dot(n_i, x - p_i))^2
m = average(p_i)
x = m + y
```

Solve for `y` using the translated points `p_i - m`. When the least-squares
solution is not unique, choose its minimum-length displacement from `m`.
This gives a defined choice for planar and edge-like data.

Centering does not impose a cell-bound constraint. Tzur's text claims an
in-cell guarantee for underdetermined systems, but the translation alone does
not establish it; the demo's `generateVertex` also adds the centroid back
without a bounds test. The pilot must test out-of-cell solutions and select
an explicit constrained solve or documented placement rule before adopting
the extractor. Do not silently clamp an unconstrained result.

The experiment must also establish:

- A bounded edge-root search and a consistent rule for exactly zero samples.
  Linear interpolation is an approximation for nonlinear density fields.
- Normals of the final composed density, expressed in the same coordinate
  space as the fitted positions. Surface-control gradients alone are not
  sufficient after cave subtraction and volumetric detail.
- Stable results for planar data, nearly parallel normals, and sharp corners;
  an explicit treatment of zero gradients and unresolved thin features.
- Sign-dependent triangle winding and a single owner for each shared polygon.
  With positive-solid density, outward normals oppose its gradient.
- Separate tests for region borders, mixed resolutions, and cube-face joins.
  The uniform-grid demo does not supply the pilot's streaming boundary method.

A density field need not be a signed distance field to extract its zero
surface. Do not use density magnitude as a safe distance for collision or
ray stepping without a separate bound. Neither mass-point placement nor
sharp-feature preservation guarantees correct topology below sample spacing.

### Coverage and transitions

Increase region coverage as detail decreases. Keep a cheap whole-planet
representation available outside the active region set. It must sample the
same broad terrain fields so descent preserves recognizable landmarks.
Fine caves may disappear at distance; mountains must not become new mountains.

Treat transitions as two separate problems:

- Spatial joins: equal-resolution neighbors, different-resolution neighbors,
  and cube-face boundaries must form acceptable visible surfaces.
- Replacement: a resident coarse region remains until all required finer
  coverage is ready. Upload completion alone does not retire its parent.

McKendrick describes region overlap to hide seams at 22:24–22:39 and
24:27–24:41. Test overlap as an early visual experiment; do not assume it
guarantees closed geometry. Select a boundary construction after the mixed
resolution test. Collision must use a defined surface without competing
overlapping contacts. This decision gates moving-surface tests.

Use a short dithered transition when new render coverage is ready. Prefer a
distance-controlled transition when generation stays far enough ahead; a
late result needs a readiness-controlled transition. Record late arrivals
instead of treating fading as a fix for insufficient throughput.

## Runtime pipeline and budgets

Each region follows a dependency pipeline:

```text
request -> surface controls -> density/material samples -> extraction
    -> render upload
    -> collision preparation
    -> placement resolution
```

The last three outputs have separate readiness states. Background jobs build
data; bounded main-thread work installs render resources, collision, and
instances. Rendering does not wait for decorative population. Ground movement
requires collision coverage along the intended motion.

Prioritize missing coarse coverage and nearby collision, then visible detail,
then population. Add motion direction and time until a region is needed to
the priority calculation. Keep coverage behind the camera so a rapid turn
does not expose an empty world. Stable region addresses break priority ties.

Bound queued jobs, active jobs, sample memory, mesh memory, upload bytes, and
installation time. Reserve capacity for coarse coverage and collision before
accepting detail work. Use worst-case output limits as well as average timings;
a complex density field can produce much more mesh data than a smooth one.
Eviction must account for buffers still referenced by jobs or GPU commands.

Cancel obsolete work at stage boundaries. Check revision and residency again
before installation. Memory accounting includes temporary data and old/new
meshes during transitions, not only the steady resident set.

If the camera outruns generation, retain coarse coverage and reduce detail
demand. If ground movement reaches missing collision, stop movement at the
covered boundary. A deliberate teleport can enter an explicit loading state.
These conditions are visible in diagnostics and count as failures on a route
that claims to support uninterrupted travel at its stated speed.

Use 60 frames per second as a provisional target, giving about 16.7 ms per
frame. It is a test objective, not a measured result. The first streaming
slice must record the planet radius, nearest sample spacing, viewport size,
walking and flight speeds, memory cap, and exact hardware. Set numerical
budgets for generation and installation from that baseline before expanding
the prototype. Report frame-time percentiles and stalls as well as averages.

## Placement and interaction

Generate placement candidates from stable spatial addresses and seed streams.
Use noise for clustered decoration and a bounded offset grid for landmarks
whose spacing matters. This follows McKendrick 46:24–49:18.

A candidate's identity and horizontal location do not change with terrain
detail. Surface queries determine altitude, slope, and material eligibility.
Use the same acceptance rule for distant landmark queries and local creation;
a marker must not promise a building that disappears when its region arrives.

Distinguish a nearby candidate from the nearest valid landmark. Rejecting
underwater or steep candidates can make a search much larger. The pilot uses
a bounded search radius and can return no result; it does not promise a
global nearest-landmark query.

When a render surface becomes more detailed, decoration may interpolate to
its refined contact position. Collision transitions require their own
continuity test. A visual fade does not make a sudden support-height change
safe for a grounded controller.

## Evaluation and delivery slices

Slice 1 lives in [apps/realtime-pilot](../apps/realtime-pilot/README.md). It
provides a CPU surface function, bounded density queries, three presets, and a
64³ volume inspector with movable cross-sections. The app also has a headless
capture command. Its generation library uses the existing hash and gradient
noise primitives; it has no dependency on the other world-generation stages.

The [implementation notes](../apps/realtime-pilot/README.md#field-decisions)
record the pilot's choices for the missing octave-loop details. The inspection
box is Cartesian and uses local model lengths. It is not spherical storage or
a surface mesher. Density feedback vectors are shaping signals; only the
basis and sharpness transform currently claim analytical gradients.

Keep two test sets: fixed seeds and routes for comparison, and a changing
sample of seeds to find unexpected terrain and expensive cases. Save failing
parameters, location, camera route, backend, generator version, timing, and
screenshots. Add useful discoveries to the fixed set.

| Slice                     | Deliverable                                                                              | Exit evidence                                                                                                                          |
| ------------------------- | ---------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| 1. Local terrain function | CPU surface and density queries; a small inspectable volume; a few terrain presets.      | Repeatable queries in shuffled order; finite fields; visible control effects; gradient checks where derivatives are claimed.           |
| 2. Spherical regions      | Cube-face storage, elevation band, one-resolution meshes, and a planet overview.         | Matching boundary samples; correct face edges and corners; no band clipping; stable positions after origin changes.                    |
| 3. Streaming and detail   | Bounded scheduler, coarse-to-fine replacement, mixed-resolution joins, and cancellation. | Recorded descent, rapid turns, and high-speed routes; bounded peak memory; no exposed holes; no obsolete results installed.            |
| 4. Usable terrain         | Nearby collision, grounded movement, sparse decoration, and landmark queries.            | Stable contact through replacement; consistent placement after eviction; measured total runtime cost on both target platforms.         |
| 5. Variety and stress     | Preset comparison, parameter inspection, replay captures, and automated seed sampling.   | Distinct terrain at ground and flight scales; stable fixed-route results; reported expensive and invalid cases with reproduction data. |

Do not start a slice until the previous one has a usable result and its open
correctness issues are recorded. A GPU experiment belongs after profiling
identifies a bottleneck; it is not a condition for completing these slices.
If added, test run-to-run and schedule invariance, integer decisions, and
documented CPU/GPU float tolerances on Metal and Vulkan. Do not pin float bits.

The pilot succeeds when a user can descend, explore, leave, and revisit a
recognizable place at the declared travel speeds within the recorded frame
and memory budgets. Shape controls must produce useful differences without
breaking locality, the terrain envelope, or usable collision. Screenshots
alone cannot establish success.

## Source gaps and decisions still to make

The appended slides resolve the signature, sharpness transform, slope weight,
altitude and ridge amplitude formulas, domain perturbation, and gain adjustment.
The architecture and these fragments are grounded in the supplied sources;
the complete noise function and surface extractor still require design work.

- **Murray 37:24 and 38:11–39:00:** the extracts omit base-noise evaluation,
  initial accumulator and amplitude values, the ridge accumulator update,
  frequency progression, and final output normalization. Specify these as
  pilot choices unless more source material supplies them.
- **Murray 37:02–37:08 and 37:24:** the talk asks for an analytical derivative,
  but the extracted signature returns only a scalar. Derivative propagation
  through the complete composition is not shown.
- **Murray 39:29–40:09:** “lower octaves” does not establish frequency ordering.
  The slide at 39:11 supplies a gain adjustment, not the octave loop. Use
  explicit frequency and amplitude progression rather than inferring an
  index convention or per-octave weights from that phrase.

The remaining pilot decisions are experimental: precision at the selected
planet scale, exact noise and derivative composition, voxel-band dimensions,
mesh boundary construction, and measured runtime budgets. Resolve each in
the slice that first exercises it. Do not turn missing transcript detail into
an undocumented claim about the original implementation.
