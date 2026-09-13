//! File-owning seed sweep. Evaluation remains in the CPU library.
use crate::replay::{Case, Pose, Replay};
use procgen_realtime_pilot::{
    BUILD_ID, PLANET_PRESETS, PREPARATION_LIMIT_MS, RouteKind, Scenario, WALK_STEP_LIMIT_MS,
    evaluate,
};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Write,
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub enum SweepSelection {
    Sample { count: usize, seed: Option<u64> },
    Case(Case),
}
struct NamedCase {
    name: String,
    case: Case,
}
pub fn run(directory: &Path, selection: SweepSelection) -> Result<(), Box<dyn std::error::Error>> {
    let (cases, sample_seed) = match selection {
        SweepSelection::Case(case) => (
            vec![NamedCase {
                name: "case".into(),
                case,
            }],
            None,
        ),
        SweepSelection::Sample { count, seed } => {
            if count > 32 {
                return Err("--samples must be 0..=32 fresh seeds per preset".into());
            }
            let seed = match seed {
                Some(seed) => seed,
                None => SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as u64,
            };
            let root = procgen_noise::fold_seed_u64_to_u32(seed);
            let mut seeds = BTreeSet::from([0_u64, 42, 42 + (1_u64 << 32)]);
            for i in 0..count as u32 {
                seeds.insert(
                    u64::from(procgen_core::hash_u32(root, i, 0, 0))
                        | (u64::from(procgen_core::hash_u32(root, i, 1, 0)) << 32),
                );
            }
            let cases = PLANET_PRESETS
                .iter()
                .flat_map(|preset| {
                    seeds.iter().map(move |&seed| NamedCase {
                        name: format!("{}-{seed}", preset.id),
                        case: Case::new(Scenario {
                            seed,
                            planet: preset.planet,
                            route: RouteKind::Walk,
                        }),
                    })
                })
                .collect::<Vec<_>>();
            (cases, Some(seed))
        }
    };
    if directory.exists() && fs::read_dir(directory)?.next().is_some() {
        return Err("sweep output directory must be empty".into());
    }
    fs::create_dir_all(directory)?;
    let count = cases.len();
    let seeds: BTreeSet<_> = cases.iter().map(|c| c.case.scenario.seed).collect();
    fs::write(
        directory.join("suite.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"version":1,"build":BUILD_ID,"platform":std::env::consts::OS,"toolchain":procgen_realtime_pilot::TOOLCHAIN,"backend":"canonical CPU","sample_seed":sample_seed,"seeds":seeds,"preparation_limit_ms":PREPARATION_LIMIT_MS,"walk_step_limit_ms":WALK_STEP_LIMIT_MS}),
        )?,
    )?;
    let mut rows = File::create(directory.join("results.jsonl"))?;
    let mut failed = 0;
    let mut expensive = 0;
    println!("build={BUILD_ID} sample_seed={sample_seed:?} cases={count}");
    for NamedCase { name, case } in cases {
        case.save(&directory.join(format!("{name}.case.json")))?;
        let start = Instant::now();
        let mut row = serde_json::json!({"case":name,"scenario":case.scenario,"build":BUILD_ID,"backend":"canonical CPU"});
        match evaluate(case.scenario) {
            Ok(r) => {
                let slow = r.preparation_ms > PREPARATION_LIMIT_MS
                    || r.walk_step_max_ms > WALK_STEP_LIMIT_MS;
                if slow {
                    expensive += 1;
                }
                row["status"] = serde_json::json!(if slow { "expensive" } else { "passed" });
                row["metrics"] = serde_json::json!({"preparation_ms":r.preparation_ms,"mesh_checks_ms":r.mesh_checks_ms,"walk_ms":r.walk_ms,"walk_step_max_ms":r.walk_step_max_ms,"triangles":r.triangles,"nonmanifold_edges":r.topology.nonmanifold_edges,"nonmanifold_vertices":r.topology.nonmanifold_vertices,"radius_range":r.radius_range,"mesh_fingerprint":format!("{:016x}",r.mesh_fingerprint),"route_fingerprint":format!("{:016x}",r.route_fingerprint),"placements":r.placements,"min_clearance":r.min_clearance,"walk_displacement":r.walk_displacement,"rest_drift":r.rest_drift,"source_bytes":r.source_bytes,"collision_index_bytes":r.collision_index_bytes});
            }
            Err(e) => {
                failed += 1;
                row["status"] = serde_json::json!("failed");
                row["stage"] = serde_json::json!(e.stage);
                row["error"] = serde_json::json!(e.error.to_string());
                row["tick"] = serde_json::json!(e.tick);
                if let Some(position) = e.location {
                    row["location"] = serde_json::json!([position.x, position.y, position.z]);
                    // Static inspection view at the failure location. This is a
                    // camera reproduction, not a replay of failed physics steps.
                    let up = position.normalized();
                    let axis = if up.x.abs() < 0.8 {
                        procgen_core::Vec3::X
                    } else {
                        procgen_core::Vec3::Y
                    };
                    let forward = (axis - up * axis.dot(up) - up * 0.2).normalized();
                    let view = procgen_realtime_pilot::StreamView { position, forward };
                    Replay {
                        case: case.clone(),
                        poses: vec![Pose::new(0.0, view), Pose::new(36.0, view)],
                    }
                    .save(&directory.join(format!("{name}.failure.replay.json")))?;
                }
            }
        }
        row["total_ms"] = serde_json::json!(start.elapsed().as_secs_f64() * 1000.0);
        writeln!(rows, "{}", serde_json::to_string(&row)?)?;
        rows.flush()?;
        println!(
            "{name}: {} ({:.0} ms)",
            row["status"],
            row["total_ms"].as_f64().expect("elapsed")
        );
    }
    fs::write(
        directory.join("summary.txt"),
        format!(
            "build={BUILD_ID}\nsample_seed={sample_seed:?}\ncases={count}\nfailed={failed}\nexpensive={expensive}\nWindows/Vulkan is not exercised by this CPU sweep.\n",
        ),
    )?;
    if failed > 0 {
        return Err(format!(
            "{failed} cases failed; saved inputs and results in {}",
            directory.display()
        )
        .into());
    }
    Ok(())
}
