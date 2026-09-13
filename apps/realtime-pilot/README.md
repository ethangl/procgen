# Real-time world pilot: local fields

Slice 1 of [the pilot design](../../docs/realtime-world-pilot.md). This is a
separate application. It uses `procgen-core` and the CPU gradient-noise primitive
from `procgen-noise`; it does not use the existing world pipeline or viewer.

## Run

```sh
cargo run -p procgen-realtime-pilot -- --seed 42
```

Select a preset, edit its controls, and press **Generate**. Sampling runs on a
worker and uses parallel columns. The inspector keeps showing the previous
volume until the result arrives. A notice identifies unapplied controls.
Move the three slice sliders to inspect the sampled volume without regenerating.

The four panels show base height, an XY density cut, an XZ density cut, and a
ZY density cut. Brown is solid; blue is air. Light pixels are near zero density.
The height panel excludes caves and 3D detail. Colors saturate outside their
display range; the stored values are unchanged.

The diagnostic contains 64 samples per axis, including both ends of a local
box from -1 to +1. These are **local model lengths**, not planet meters. Y is
up only in this Cartesian experiment. The box is a view into the field and
can crop terrain at extreme settings; it is not the future spherical voxel
band. Surface queries support X/Z and density queries support XYZ within
plus or minus 8 model lengths. Invalid inputs return typed errors.

Three presets cover rounded hills, ridged mountains, and broad basins. Use
seed 42 for the initial comparison. The full seed and resolved parameters are
printed after each interactive generation.

## Headless captures

The CPU library and capture command can run without the engine dependencies:

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --seed 42 --capture /tmp/procgen-pilot
```

This writes one PPM image and one text record per preset. Panel order is height
XZ, density XY, density XZ, density ZY, starting at the top left. Each text
record includes the seed, grid, resolved config, and sampling time. Captures
are local diagnostics; they do not implement terrain-tile export.

## Field decisions

The equations in Murray's slides are the reference. Missing parts are completed
as follows for this experiment:

- Use cubic gradient noise divided by its exported conservative bound. The
  basis and its derivative therefore share the same normalization.
- Use six octaves, frequency `1 / wavelength`, lacunarity 2, and one key across
  octaves. Begin with unit amplitude and damped amplitude; all sums begin at zero.
- Use the unshaped basis derivative in octave coordinates for slope, ridge,
  and perturbation signals. Ridge accumulation mirrors slope accumulation,
  scaled by its own erosion control. These are pilot choices.
- Apply the slope update before adding the damped contribution. Then update
  altitude amplitude and ridge damping, perturbation, and frequency.
- Store the resolved gain once. The displayed `gain + amplification` is one
  value here; the supplied fragments do not establish separate effects.
- Divide the sum by the undamped geometric amplitude envelope. This gives a
  conservative [-1, 1] bound without normalizing away position-dependent damping.
- Claim analytical derivatives only for the normalized basis and sharpness
  transform. The composed surface and density return scalar values. Their
  feedback vectors are not final gradients.

The surface is the shape function times the height scale. A separate 3D noise
field perturbs the solid boundary. Cave candidates use a 0.5-unit horizontal
grid, a 0.11-unit chamber radius, and a 0.2-unit depth below their own surface
query. Each chamber connects to an inclined capsule that reaches above the
local additive-detail envelope. Only the surrounding nine grid candidates can
affect a column. Cave density is an expected count per square model length.
Chambers can overlap or open through nearby slopes; no global cave network is
promised.

Density magnitude is truncated to 0.125 away from the zero surface. This
preserves the solid boundary and limits cave influence, so omitted distant
candidates cannot change values when a query crosses a candidate-grid boundary.

The seed is folded once by the noise crate. Surface, detail, and caves use
separate named hash domains. A per-column model reuses surface and cave queries
across vertical samples. The direct query path and volume sampler use the same
column evaluator, so cache state and job order cannot change the field.

## Validation and boundaries

```sh
cargo test -p procgen-realtime-pilot --no-default-features
cargo clippy -p procgen-realtime-pilot --all-targets -- -D warnings
cargo fmt -p procgen-realtime-pilot -- --check
```

Tests cover shuffled queries, independent thread schedules, direct/batched
agreement, field envelopes, finite values at control extremes, cave entrances,
agreement with a wider cave search, control effects, analytical gradient checks, invalid inputs, and quantized
initial fingerprints. Exact float bits are not pinned.

Spherical addressing, precision across large distances, dual contouring,
mixed-resolution meshes, streaming, collision, and GPU generation remain for
later slices. No frame-time or planet-scale performance claim follows from
this small volume experiment.
