# Real-time pilot oceans

The GPU orbit/descent viewer renders a smooth spherical ocean at reference
radius plus sea level. Run `cargo run -p procgen-realtime-pilot`. The Design tab
has **Show ocean** and **Sea level (m)** controls. These take effect on the next
frame, without terrain generation or camera movement. Terrain defaults,
one-meter local voxels, distant detail, coloring, and navigation are unchanged.

## Saved settings

The version-1 design document has an optional top-level `ocean` object, separate
from the `design` generation parameters:

```json
"ocean": { "enabled": true, "sea_level_m": 0.0 }
```

Existing files load with water enabled at zero meters. Sea level accepts
-20,000 to 20,000 meters. **Save controls** writes terrain and ocean settings to
the current file; **Save as…** changes the target only after success. **Load…**,
**Copy JSON**, `--design-file`, and `--write-design` carry both sets of settings.
Editing controls never writes a file automatically. Loading only a different
sea level does not invalidate pending terrain work.

## Rendering

The existing opaque terrain compositor intersects each camera ray with the
water sphere. It compares water and terrain depth, so height tiles and local
voxels both cut shorelines. Water resolves within each terrain snapshot before
crossfading, so retiring terrain cannot leave an old shoreline. There is no
water mesh, LOD, density sampling, readback, or extra render target.

The host computes camera altitude from integer meters and the sub-meter offset.
It uploads `c = altitude * (2 * ocean_radius + altitude)` for the sphere equation;
the shader uses stable quadratic roots. This avoids subtracting large squared
radii in f32. Rays and depths use the current viewport and camera-relative
projection. All shader arithmetic is f32 and uses the same WGSL on Metal/Vulkan.

Water uses depth absorption, a simple reflected sky, a sun highlight, and
Fresnel reflection. An underwater camera sees distance-based attenuation and
a tinted exit surface. These are visual approximations, not fluid simulation.

## Limits

- The water is flat at its spherical datum. Waves, foam, refraction, reflected
  terrain, tides, and atmosphere are not included.
- Every exposed basin below sea level fills, including disconnected inland
  depressions. There is no drainage or ocean-connectivity model.
- Water does not affect collision. Walking uses the terrain, including the
  seafloor; flight can pass through water. Swimming and buoyancy are not included.
- Shorelines depend on the displayed terrain mesh resolution. Subpixel coast
  edges have no dedicated antialiasing, and missing terrain coverage cannot
  supply a seabed depth.
- `--backend cpu` and headless terrain audits preserve ocean settings but do not
  render water. Other experimental viewers are unchanged.
- Metal is tested here. The shader tests select Vulkan on Windows, but this
  slice still needs a native Windows run.

## Validation

```sh
cargo test -p procgen-realtime-pilot --bin procgen-realtime-pilot
cargo test -p procgen-realtime-pilot --no-default-features --bin procgen-realtime-pilot
cargo test -p procgen-gpu-tests --test ocean_rendering --test surface_composition -- --nocapture
cargo clippy -p procgen-realtime-pilot --all-targets -- -D warnings
cargo run -p procgen-realtime-pilot -- --explore-record /tmp/procgen-ocean-route.csv
```

Tests cover old file loading, ocean validation and explicit saving, sea-level
edits during pending terrain generation, stable text focus, centimeter-scale
camera height at radius extremes, sphere intersections against a geometric
reference, dry terrain occlusion, shallow/deep water, underwater depth, and
shoreline replacement through the real compositor.

On 2026-09-14, the 90-second native Metal route with the current saved 300 km
preset recorded 10,477 frames. All 1,738 walking frames reported `Advanced`,
collision coverage, and one-meter finest spacing. Six screenshots were saved
as `/tmp/procgen-ocean-route.1.png` through `.6.png`; the orbit image shows
terrain-defined coastlines. The route landed on dry terrain and does not by
itself validate underwater navigation. Frame pacing and Windows validation
remain separate from these correctness checks.

Default native launch was also checked. Interactive sea-level/save/reload and
waterline travel were not completed because the user took control of the open
viewer; the automated editor and GPU tests cover their underlying state and
rendering behavior. The viewer was left open for manual inspection.
