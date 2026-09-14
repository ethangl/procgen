# Windows GPU validation handoff

Validate the real-time pilot on Windows with the NVIDIA RTX 5070 through
wgpu/Vulkan. Read `AGENTS.md`, `docs/realtime-world-gpu-streaming.md`, and
`apps/realtime-pilot/README.md` before starting.

Run these commands from the repository root in PowerShell:

```powershell
cargo test -p procgen-gpu-tests --test voxel_density_agreement -- --nocapture
cargo test -p procgen-gpu-tests --test voxel_mesh_agreement -- --nocapture
cargo test -p procgen-gpu-tests --test voxel_streaming_agreement -- --nocapture --test-threads=1
cargo test -p procgen-gpu-tests --test height_mesh_agreement -- --nocapture --test-threads=1
cargo run -p procgen-realtime-pilot -- --design --explore --design-file planet-design-300km.json --backend gpu --explore-record g5-small-vulkan.csv
cargo run -p procgen-realtime-pilot -- --design --explore --design-file planet-design.json --backend gpu --explore-record g5-large-vulkan.csv
```

Each viewer route runs for 90 seconds, writes a CSV and six adjacent PNG
captures, and exits. Live navigation is disabled during recording.

Inspect test output, screenshots, and timings. Check CPU/GPU agreement,
deterministic revisits, bounded terrain allocations, complete terrain coverage,
and ground contact. Distinguish measured evidence from anything the captures or
telemetry cannot establish. Record frame and update latency, failures, and clean
shutdown for both presets. Include GPU, driver, OS, window resolution, and tested
commit in the results.

Update `docs/realtime-world-gpu-streaming.md` with the Windows results and any
remaining acceptance gaps. Keep raw recordings available locally; do not add
them to Git by default. Do not loosen tolerances merely to pass tests. If a check
fails, preserve its exact output and investigate before changing code. Keep
unrelated terrain tuning and visual polish out of this validation work. Do not
commit, push, or create a branch unless asked.
