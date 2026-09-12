//! Throwaway measurement module for the relief-decay slice. Deleted with the
//! slice; it lives here because the two settings profiles are in the bin crate.

use crate::model::{GenerationTimings, TectonicsSettings, TectonicsWorld, build_mesh};
use procgen_sphere::FibonacciConfig;
use procgen_sphere_mesh::{SphereMesh, hops};
use procgen_tectonics::{
    BoundaryClass, BoundaryDeformationConfig, DEFAULT_STEP_DURATION, PlatePartitionConfig,
};

const CELL_COUNT: usize = 65_536;

fn defaults_world() -> TectonicsSettings {
    TectonicsSettings::default()
}

fn reference_world() -> TectonicsSettings {
    let default = TectonicsSettings::default();
    TectonicsSettings {
        fibonacci: FibonacciConfig {
            seed: 9,
            ..default.fibonacci
        },
        plates: PlatePartitionConfig {
            subdivided_fraction: 0.2,
            ..default.plates
        },
        ..default
    }
}

fn run(base: &TectonicsSettings, steps: usize, erosion_time: f32) -> TectonicsWorld {
    let mut settings = base.clone();
    settings.fibonacci.count = CELL_COUNT;
    settings.evolution.run_duration = steps as f32 * DEFAULT_STEP_DURATION;
    settings.evolution.step_duration = DEFAULT_STEP_DURATION;
    settings.evolution.deformation = BoundaryDeformationConfig {
        erosion_time,
        ..settings.evolution.deformation
    };
    let mut timings = GenerationTimings::default();
    let mesh = build_mesh(settings.fibonacci, &mut timings).unwrap();
    TectonicsWorld::generate(mesh, settings, timings).unwrap()
}

/// Hops from the nearest cell touching a currently convergent edge, saturating
/// at `limit`.
fn convergent_distance(mesh: &SphereMesh, world: &TectonicsWorld, limit: usize) -> Vec<usize> {
    let mut distance = vec![usize::MAX; mesh.cell_count()];
    let mut frontier = Vec::new();
    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        if world.boundaries.edge_classes[edge_index] == BoundaryClass::Convergent {
            for cell in edge.cells {
                if distance[cell] != 0 {
                    distance[cell] = 0;
                    frontier.push(cell);
                }
            }
        }
    }
    for step in 1..=limit {
        let mut next = Vec::new();
        for cell in frontier.drain(..) {
            for corner in mesh.cell_corners(cell) {
                if distance[corner.neighbor] == usize::MAX {
                    distance[corner.neighbor] = step;
                    next.push(corner.neighbor);
                }
            }
        }
        frontier = next;
    }
    distance
}

struct Row {
    clamped: usize,
    mean: f32,
    maximum: f32,
    land: usize,
    relic_count: usize,
    relic_maximum: f32,
}

fn measure(world: &TectonicsWorld) -> Row {
    let mesh = &world.voronoi;
    let config = world.config.evolution.deformation;
    let field = &world.deformation.cell_deformation;
    let reach = hops(mesh.cell_count(), config.convergent.depth);
    let distance = convergent_distance(mesh, world, reach);
    let mut relic_count = 0;
    let mut relic_maximum: f32 = 0.0;
    for (cell, &value) in field.iter().enumerate() {
        if value.abs() > 0.1 && distance[cell] > reach {
            relic_count += 1;
            relic_maximum = relic_maximum.max(value.abs());
        }
    }
    Row {
        clamped: field
            .iter()
            .filter(|value| value.abs() >= config.maximum_magnitude)
            .count(),
        mean: world.deformation.diagnostics.summary.mean,
        maximum: world.deformation.diagnostics.summary.maximum,
        land: world
            .elevation
            .cell_elevations
            .iter()
            .filter(|&&value| value > world.elevation.sea_level)
            .count(),
        relic_count,
        relic_maximum,
    }
}

fn table(name: &str, base: &TectonicsSettings, lengths: &[usize]) {
    println!("\n## {name}");
    println!("| steps | sink | clamp | mean | max | land | relic cells | relic max |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for &steps in lengths {
        for (label, erosion) in [
            ("off", f32::INFINITY),
            ("on", BoundaryDeformationConfig::default().erosion_time),
        ] {
            let row = measure(&run(base, steps, erosion));
            println!(
                "| {steps} | {label} | {} | {:.4} | {:.3} | {} | {} | {:.3} |",
                row.clamped, row.mean, row.maximum, row.land, row.relic_count, row.relic_maximum
            );
        }
    }
}

#[test]
#[ignore]
fn measure_relief_decay() {
    let lengths = [15, 30, 60, 120, 240];
    table("Viewer defaults", &defaults_world(), &lengths);
    table("Reference world", &reference_world(), &lengths);

    println!("\n## Per-step series at the defaults, sink on");
    println!("| steps | clamp | mean | max | land | relic cells | relic max |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    let erosion = BoundaryDeformationConfig::default().erosion_time;
    for steps in (0..=240).step_by(10) {
        let row = measure(&run(&defaults_world(), steps, erosion));
        println!(
            "| {steps} | {} | {:.4} | {:.3} | {} | {} | {:.3} |",
            row.clamped, row.mean, row.maximum, row.land, row.relic_count, row.relic_maximum
        );
    }
}
