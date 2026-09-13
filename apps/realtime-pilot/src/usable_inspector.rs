//! Inspector consumption of walking, collision and stable population products.
use bevy::prelude::*;
use procgen_core::Vec3 as Point;
use procgen_realtime_pilot::{
    LANDMARK_SEARCH_REACH, MAX_WALK_SECONDS, PlacementId, PlacementKind, UsableTerrain, WALK_SPEED,
    Walker,
};
use std::collections::BTreeMap;

#[derive(Resource, Default)]
pub struct UsableState {
    pub terrain: Option<UsableTerrain>,
    pub walker: Option<Walker>,
    pub status: String,
    pub walking_ms: f64,
    pub population_ms: f64,
    pub clamped_frames: usize,
    pub collision_failures: usize,
    instances: BTreeMap<PlacementId, Entity>,
    meshes: Option<[Handle<Mesh>; 2]>,
    materials: Option<[Handle<StandardMaterial>; 2]>,
}
pub fn point(p: Vec3) -> Point {
    Point::new(p.x, p.y, p.z)
}
pub fn vector(p: Point) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}
impl UsableState {
    pub fn up(&self) -> Vec3 {
        self.walker.as_ref().map_or(Vec3::Y, |w| vector(w.up()))
    }
    pub fn toggle(&mut self, position: Vec3) -> Option<Vec3> {
        if self.walker.take().is_some() {
            self.status = "Flight mode".into();
            return None;
        }
        let Some(terrain) = &self.terrain else {
            self.status = "Waiting for collision preparation".into();
            return None;
        };
        match Walker::land(&terrain.queries, point(position)) {
            Ok(walker) => {
                let eye = vector(walker.eye());
                self.walker = Some(walker);
                self.status = "Walking".into();
                Some(eye)
            }
            Err(e) => {
                self.status = e.to_string();
                None
            }
        }
    }
    pub fn walk(&mut self, input: Vec3, seconds: f32) -> Option<Vec3> {
        let start = std::time::Instant::now();
        let walker = self.walker.as_mut()?;
        let terrain = self.terrain.as_ref().expect("walker has terrain");
        if seconds > MAX_WALK_SECONDS {
            self.clamped_frames += 1;
        }
        if let Err(e) = walker.advance(
            &terrain.queries,
            point(input),
            seconds.min(MAX_WALK_SECONDS),
        ) {
            self.collision_failures += 1;
            self.status = e.to_string();
        } else {
            self.status = format!(
                "{} · {} nearby triangles",
                if walker.grounded() {
                    "Grounded"
                } else {
                    "Falling"
                },
                walker.triangle_count()
            );
        }
        self.walking_ms = start.elapsed().as_secs_f64() * 1000.0;
        Some(vector(walker.eye()))
    }
    pub fn clearance(&self) -> Option<f32> {
        let walker = self.walker.as_ref()?;
        let terrain = self.terrain.as_ref()?;
        walker.clearance(&terrain.queries)
    }
    pub fn recorded_walk(&mut self, seconds: f32, dt: f32) -> Result<(Vec3, Vec3), String> {
        if self.walker.is_none() {
            let terrain = self.terrain.as_ref().ok_or("waiting for collision")?;
            let walker =
                procgen_realtime_pilot::route_walker(terrain).map_err(|e| e.to_string())?;
            self.walker = Some(walker);
        }
        let input = vector(procgen_realtime_pilot::walking_input(seconds));
        let position = self.walk(input, dt).expect("route walker");
        let up = self.up();
        let tangent = (Vec3::X - up * up.x).normalize();
        let forward = if (12.0..18.0).contains(&seconds) {
            let angle = (seconds - 12.0) * std::f32::consts::TAU / 0.3;
            (tangent * angle.cos() + up * angle.sin()).normalize()
        } else {
            let sign = if seconds < 18.0 { 1.0 } else { -1.0 };
            (tangent * sign - up * 0.2).normalize()
        };
        Ok((position, forward))
    }
    pub fn description(&self, position: Vec3) -> String {
        let landmark = self
            .terrain
            .as_ref()
            .and_then(|t| t.population.landmark(point(position)));
        let marker = match landmark {
            Some(p) => format!(
                "Landmark {:?} {}/{} · {:.2} away",
                p.id.face,
                p.id.x,
                p.id.y,
                (p.position - point(position)).length()
            ),
            None => format!("No landmark within {LANDMARK_SEARCH_REACH} model lengths"),
        };
        format!(
            "G: land / return to flight\nWalk speed: {WALK_SPEED}\n{}\n{} instances · {:.2} ms\n{} delayed frames · {} collision failures\n{marker}",
            self.status,
            self.instances.len(),
            self.walking_ms + self.population_ms,
            self.clamped_frames,
            self.collision_failures
        )
    }
}
pub fn population(
    mut state: ResMut<UsableState>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    camera: Res<crate::stream_inspector::CameraState>,
) {
    let start = std::time::Instant::now();
    let Some(terrain) = &state.terrain else {
        return;
    };
    let desired: BTreeMap<_, _> = terrain
        .population
        .nearby(point(camera.position))
        .map(|p| (p.id, *p))
        .collect();
    let removed: Vec<_> = state
        .instances
        .keys()
        .filter(|id| !desired.contains_key(id))
        .copied()
        .collect();
    for id in removed {
        commands
            .entity(state.instances.remove(&id).expect("resident instance"))
            .despawn();
    }
    if state.meshes.is_none() {
        state.meshes = Some([
            meshes.add(population_mesh(
                Sphere::new(1.0).mesh().ico(1).expect("one subdivision"),
            )),
            meshes.add(population_mesh(Cuboid::new(0.45, 1.0, 0.45).into())),
        ]);
        state.materials = Some([
            materials.add(Color::srgb(0.22, 0.18, 0.14)),
            materials.add(Color::srgb(0.9, 0.2, 0.1)),
        ]);
    }
    // Population is optional. Bound main-thread installation to eight instances.
    for (id, p) in desired
        .into_iter()
        .filter(|(id, _)| !state.instances.contains_key(id))
        .take(8)
        .collect::<Vec<_>>()
    {
        let index = match id.kind {
            PlacementKind::Rock => 0,
            PlacementKind::Landmark => 1,
        };
        let normal = vector(p.normal);
        let transform = Transform {
            translation: vector(p.position) + normal * (p.scale * 0.5),
            rotation: Quat::from_rotation_arc(Vec3::Y, normal),
            scale: Vec3::splat(p.scale),
        };
        let entity = commands
            .spawn((
                Mesh3d(state.meshes.as_ref().expect("population meshes")[index].clone()),
                MeshMaterial3d(
                    state.materials.as_ref().expect("population materials")[index].clone(),
                ),
                transform,
                crate::stream_render::full_visibility(),
            ))
            .id();
        state.instances.insert(id, entity);
    }
    state.population_ms = start.elapsed().as_secs_f64() * 1000.0;
}

// Match the terrain vertex layout and shader variant already used by the overview.
// Population has no textures; unused UVs would require another pipeline at landing.
fn population_mesh(mut mesh: Mesh) -> Mesh {
    mesh.remove_attribute(Mesh::ATTRIBUTE_UV_0);
    mesh.remove_attribute(Mesh::ATTRIBUTE_TANGENT);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        vec![[1.0_f32; 4]; mesh.count_vertices()],
    );
    mesh
}
