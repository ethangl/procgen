# Broad relief calibration

The six-octave pilot produced shallow broad terrain after all four
surface-quality slices. At radius 4 and seed 42, the middle 90% of heights spanned
only 0.057–0.080 model lengths. Most apparent local relief came from cave cuts,
additive density detail, and the material. Improving the mesh did not correct
the height distribution.

## Composition

The original implementation used the conservative gradient-noise bound as the
input contrast scale. That normalized basis has a standard deviation of about
0.135, not a range evenly occupied between -1 and 1. Squaring it reduces contrast
further. `1 - abs(n)` instead produces a large positive offset: the ridge preset's
median broad height was 0.455, with little variation around that raised radius.
Erosion weights and the undamped amplitude envelope further reduce the signal.

The corrected shape blend uses fixed centering and scale factors:

```text
base   = n / 0.135
billow = (n*n - 0.0182) / 0.0226
ridge  = (0.110 - abs(n)) / 0.078
feature = mix(base, sharpness >= 0 ? billow : ridge, abs(sharpness))
```

These are centered versions of Murray's displayed basis, squared, and ridge
shapes, scaled to comparable standard deviations. The same transforms scale
their analytic derivatives. The erosion and perturbation accumulators continue
to use the normalized, unshaped basis derivative in octave coordinates. The
altitude damping signal now accumulates centered contributions, so it responds
to relative relief rather than the ridge shape's constant positive offset.

Six octaves, lacunarity 2, configured wavelengths, gains, erosion strengths,
height scales, density detail, and cave parameters stay fixed. After the octave
sum, the function still divides by the undamped amplitude envelope. A smooth
transfer `x / sqrt(1 + x*x)` then keeps the standardized output within [-1, 1].
Thus `height_scale` remains an absolute elevation bound. The transfer has unit
slope at zero and compresses extremes continuously; it does not clip peaks or
normalize away local erosion damping.

This centering and contrast calibration is a deliberate pilot extension to the
slide fragments. It does not claim to reproduce the original game's function.
There is no per-world histogram, runtime auto-leveling, or camera-dependent
normalization. All points, seeds, region jobs, collision queries, and exports
use the same fixed constants. The new statistics are observational only.

## Reference measurement

The constants are rounded moments of 262,144 normalized `procgen-noise` samples
with key 19. Positions cover `[-32, 32)` on each axis. For sample index `i` and
axis `a`, the coordinate is:

```text
((hash_u32(42, i, a, 0) >> 8) / 16777216.0) * 64 - 32
```

The measured mean was -0.000106, mean absolute value 0.109951, mean square
0.018188, and standard deviation 0.134864. Absolute-value and squared-value
standard deviations were 0.078095 and 0.022582. These measurements describe the
basis, not any generated planet. A regression uses different spatial samples
and three keys to check that all three calibrated shapes remain approximately
centered with comparable spread.

## Height measurement

The CPU sweep now reports `broad_height` and `broad_height_samples` alongside
mesh radius extrema. It samples the broad height function at 65,536 equal-area
Fibonacci directions, before additive density detail and caves. Sampling is
parallel and deterministic for a given backend. The summary contains minimum,
5th percentile, median, 95th percentile, and maximum height above the reference
sphere. Its time is included in the whole-case duration, separate from source
preparation, mesh audit, and walking times.

Seed-42 measurements, with identical preset inputs before and after:

| Preset | Previous p05–p95 | New p05–p95 | Previous median | New median |
| --- | --- | --- | ---: | ---: |
| Hills | -0.020–0.036 | -0.201–0.314 | 0.003 | -0.048 |
| Ridges | 0.406–0.486 | -0.392–0.349 | 0.455 | 0.030 |
| Basins | -0.038–0.040 | -0.246–0.269 | 0.000 | -0.002 |

The middle 90% now spans about 6.6–9.3 times as much altitude. Broad-relief
regressions cover all three presets and seeds 0, 42, and 4,294,967,338. They
require this span to exceed half the configured height scale, the absolute
median to remain below a quarter of that scale, and extrema to stay inside
the configured bound. A flat-height and one/four-worker comparison cover the
measurement's edge case and schedule invariance.

The generation behavior changes for existing cases: case parameters and file
formats remain valid, but the same seed now gives different heights, meshes,
contacts, and accepted placements. Recording build IDs identify the new
generator. The density, triangle-identity, and placement-ID fingerprints were
updated for this intentional relief change; their tolerances were not loosened.

## Checks exposed by steeper terrain

The broader seed sweep found four degenerate refined triangles for hills seed
3,924,050,149,737,654,367. The extracted base mesh was valid. On very thin
triangles, an `f32` midpoint subdivision can collapse even after rejecting field
projection. Refinement now checks linear child winding before projection and
removes unrepresentable splits from the shared edge set. Neighboring triangles
use the same remaining split pattern; original geometry is retained. A thin
tetrahedron regression covers midpoint rounding, closed joins, and preservation
of the original vertices. The unchanged sphere refinement test still requires
its full subdivision and measured geometric improvement.

The original rest audit also treated every idle-input phase as stationary
support. Hills seed 0 was still airborne when input stopped at 12 seconds; it
landed five steps later and then stayed fixed. Other routes stop on slopes too
steep to be walkable. The controller correctly keeps gravity active in both
situations. The audit now checks drift only during continuous grounded contact.
It separately settles a walker for one second at the route's landing site, then
requires one second of grounded, stationary support there. This probe does not
alter the route's initial state. The unchanged 0.001 drift limit applies to both
checks; contact clearance is still checked throughout the whole route.
`grounded_rest_steps` records how much stationary support was checked.

## Validation record

Build `f0ad988920672ac6`, tested on 2026-09-13 on Apple M1 Max / Metal:

- All 41 unit tests pass. Clippy passes with warnings denied, with the inspector
  and without default features; formatting and diff checks pass.
- The CPU sweep covers all three presets with nine seeds each: the fixed 0, 42,
  and 4,294,967,338 plus six seeds selected with sample seed 42. All 27 pass fine
  and mixed-LOD topology, placement recreation, contact clearance, and stationary
  support. Minimum clearance is 0.035011 and maximum grounded rest drift is zero.
  Each case checks 120–1,560 grounded idle steps.
- All 27 remain classified expensive because preparation takes 2.09–4.26
  seconds, above the existing 1-second triage threshold. Some checks ran alongside
  compilation and native captures, so these timings do not establish frame pacing.

The final sweep is saved in `/tmp/procgen-relief/final-sweep`. Reproduce it with:

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --sweep /tmp/procgen-relief-check --samples 6 --sample-seed 42
```

Native ridges-42 comparison uses the same held camera poses as surface-quality
slice 4, with `--surface-detail plain`. Peaks, valleys, and the planet silhouette
change visibly without material detail. Hills-42 flight also uses the plain
material. The greater relief exposes the existing large and narrow triangles;
this calibration does not increase source sampling resolution or provide
geographically distinct terrain regions.

The ridge comparison completed 3,812 frames, with a peak managed reservation of
257,735,720 bytes and a largest upload of 510,120 bytes. Hills flight saved all
2,928 frames and screenshots, peaking at 213,509,668 bytes with a largest upload
of 516,048 bytes. Both retained all six faces, exercised all three LODs, and
installed no obsolete tickets. Their saved frame and pose counts agree, and
their reservations stay inside the unchanged 256 MiB and 512 KiB limits.

The hills process hung after saving its complete recording, during Bevy world
cleanup in the macOS event-loop exit path. A process sample is retained as
`/tmp/procgen-relief/hills-exit-sample.txt`; the capture process was terminated.
This is an unresolved shutdown issue, not a successful process exit. The
native artifacts are in `/tmp/procgen-relief/captures`. Windows/Vulkan rendering
validation remains open.

The separate textured ridges-42 walking route completed and exited successfully
after 4,188 frames, with minimum clearance 0.035013 and no collision failures,
missing faces, or obsolete installations. Peak reservation was 213,743,408 bytes
and the largest upload was 510,120 bytes. Its ground views often face nearby
steep, dark terrain; the fixed-camera plain comparisons are the clearer visual
evidence of the changed broad relief.
