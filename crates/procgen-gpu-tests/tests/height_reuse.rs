//! Real selected layouts must retain exact shared geometry and shading on device.
use procgen_core::Vec3;
use procgen_cubesphere::MAX_TILE_LEVEL;
use procgen_gpu_tests::{readback, request_device_for_backends};
use procgen_realtime_pilot::*;
use std::collections::BTreeMap;

fn sample_key(tile: HeightTile, x: u32, y: u32) -> [i32; 3] {
    let a = tile.address();
    let scale = (HEIGHT_QUADS << MAX_TILE_LEVEL) as i32;
    let u = ((a.x() * HEIGHT_QUADS + x) << (MAX_TILE_LEVEL - a.level())) as i32 * 2 - scale;
    let v = ((a.y() * HEIGHT_QUADS + y) << (MAX_TILE_LEVEL - a.level())) as i32 * 2 - scale;
    let f = a.face().frame();
    let p = f.normal * scale as f32 + f.u_axis * u as f32 + f.v_axis * v as f32;
    [p.x as i32, p.y as i32, p.z as i32]
}
#[test]
fn selected_mixed_lod_layouts_share_gpu_positions_and_normals() {
    let backend = if cfg!(target_os = "macos") {
        wgpu::Backends::METAL
    } else {
        wgpu::Backends::VULKAN
    };
    let (_, device, queue) = request_device_for_backends("reusable heights", backend).unwrap();
    let doc: serde_json::Value =
        serde_json::from_str(include_str!("../../../planet-design-300km.json")).unwrap();
    let config: PlanetDesignConfig = serde_json::from_value(doc["design"].clone()).unwrap();
    let field = config.validate().unwrap();
    let mesher = HeightGpuMesher::new(&device, &field);
    for direction in [
        Vec3::new(1., 1., 0.).normalized(),
        Vec3::new(1., 1., 1.).normalized(),
        Vec3::new(1., 0.7, -0.3).normalized(),
    ] {
        let p = direction * (config.radius_m + field.elevation_m(direction, 0.).unwrap() + 5.);
        let tiles = select_height_coverage(
            &field,
            VoxelPosition {
                x_m: p.x as i32,
                y_m: p.y as i32,
                z_m: p.z as i32,
            },
        );
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("audit complete height layout"),
            size: tiles.len() as u64 * HEIGHT_TILE_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        for (batch_index, batch) in tiles.chunks(HEIGHT_GPU_BATCH_TILES).enumerate() {
            let mut encoder = device.create_command_encoder(&Default::default());
            let buffers = mesher.encode_batch(&device, &mut encoder, batch);
            for (i, buffer) in buffers.iter().enumerate() {
                encoder.copy_buffer_to_buffer(
                    buffer,
                    0,
                    &output,
                    (batch_index * HEIGHT_GPU_BATCH_TILES + i) as u64 * HEIGHT_TILE_BYTES,
                    HEIGHT_TILE_BYTES,
                );
            }
            queue.submit([encoder.finish()]);
        }
        let vertices = readback::<HeightVertex>(
            &device,
            &queue,
            &output,
            tiles.len() * HEIGHT_VERTEX_COUNT as usize,
        );
        let mut shared = BTreeMap::new();
        let mut matches = 0;
        for (tile, vertices) in tiles
            .iter()
            .zip(vertices.chunks(HEIGHT_VERTEX_COUNT as usize))
        {
            // Even edge samples survive stitching on both fine and coarse tiles.
            for i in (0..=HEIGHT_QUADS).step_by(2) {
                for [x, y] in [[0, i], [HEIGHT_QUADS, i], [i, 0], [i, HEIGHT_QUADS]] {
                    let v = vertices[(y * HEIGHT_SIDE + x) as usize];
                    let surface = (
                        [v.anchor[0], v.anchor[1], v.anchor[2]],
                        [v.offset[0], v.offset[1], v.offset[2]],
                        v.normal,
                    );
                    if let Some(previous) = shared.insert(sample_key(*tile, x, y), surface) {
                        assert_eq!(previous, surface, "GPU shared surface {tile:?} at {x},{y}");
                        matches += 1;
                    }
                }
            }
        }
        assert!(matches > 10_000, "exercise the complete mixed-level cover");
    }
}
