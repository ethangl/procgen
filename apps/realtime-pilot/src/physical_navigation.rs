//! Native input and movement with retained asynchronous collision coverage.
use super::{Inspector, MAX_ORBIT_CLEARANCE_RADII, Navigation};
use crate::physical_record::MotionOutcome;
use crate::physical_render::core;
use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    prelude::*,
};
use bevy_egui::EguiContexts;
use procgen_realtime_pilot::{MeterPosition, PhysicalWalker, VoxelPosition};
use std::time::Instant;

pub(super) fn movement(
    mut state: NonSendMut<Inspector>,
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mouse: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut contexts: EguiContexts,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let keyboard_blocked = state.record.is_some() || ctx.wants_keyboard_input();
    let pointer_blocked = state.record.is_some() || ctx.wants_pointer_input();
    let dt = time.delta_secs().min(0.05);
    state.motion = MotionOutcome::Idle;
    state.frame_ms = state.frame_ms * 0.95 + time.delta_secs() * 1000.0 * 0.05;
    state.peak_frame_ms = state.peak_frame_ms.max(time.delta_secs() * 1000.0);
    let up = state.up();
    if !pointer_blocked && buttons.pressed(MouseButton::Right) {
        state.rotation = Quat::from_axis_angle(up, -mouse.delta.x * 0.003)
            * state.rotation
            * Quat::from_rotation_x(-mouse.delta.y * 0.003);
    }
    if !pointer_blocked
        && state.mode == Navigation::Orbit
        && buttons.pressed(MouseButton::Left)
        && mouse.delta != Vec2::ZERO
    {
        let state = &mut *state;
        drag_orbit(&mut state.eye, &mut state.rotation, mouse.delta);
        state.face_ground();
    }
    let scroll_lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y * 0.02,
    };
    if !pointer_blocked && scroll_lines != 0.0 {
        if state.mode == Navigation::Orbit {
            let clearance = (state.clearance_estimate() * (-scroll_lines * 0.08).exp()).clamp(
                5.0,
                state.field.config().radius_m * MAX_ORBIT_CLEARANCE_RADII,
            );
            state.set_radial(clearance);
            state.face_ground();
        } else {
            state.speed_factor =
                (state.speed_factor * (scroll_lines * 0.08).exp()).clamp(0.25, 8.0);
        }
    }
    if state.descent {
        let clearance = state.clearance_estimate();
        if clearance > 6.0 {
            state.set_radial((clearance * (-dt * 0.8).exp()).max(5.0));
        } else {
            state.descent = false;
            state.mode = Navigation::Fly;
            state.face_horizon();
        }
    }
    let key = |k| {
        if !keyboard_blocked && keys.pressed(k) {
            1.0
        } else {
            0.0
        }
    };
    let mut input = Vec3::new(
        key(KeyCode::KeyD) - key(KeyCode::KeyA),
        key(KeyCode::KeyE) - key(KeyCode::KeyQ),
        key(KeyCode::KeyS) - key(KeyCode::KeyW),
    );
    if state.record.as_ref().is_some_and(|r| r.moving()) {
        input.z = -1.0;
    }
    let mut forecast = state.eye;
    if state.mode == Navigation::Fly && input != Vec3::ZERO {
        state.descent = false;
        let clearance = state.clearance_estimate();
        let speed = if clearance < 64.0 {
            8.0
        } else if state.eye.altitude_m(state.field.config().radius_m)
            < state.field.config().height_limit_m as f64 + 100.0
        {
            50.0
        } else {
            (clearance * 0.6).clamp(8.0, 2_000_000.0)
        };
        let velocity = core(
            (state.rotation * Vec3::new(input.x, 0.0, input.z) + up * input.y).normalize_or_zero()
                * speed
                * state.speed_factor,
        );
        let delta = velocity * dt;
        forecast = state.eye.translated(velocity);
        if clearance < 32.0 {
            if let Some(collision) = state.collision.patch() {
                match collision.sweep(state.eye.relative_to(collision.origin_m()), delta, 0.2) {
                    Ok(sweep) => {
                        state.eye = MeterPosition::new(collision.origin_m(), sweep.position_m);
                        state.motion = MotionOutcome::Advanced;
                        state.status = "Flying with nearby collision.".into();
                    }
                    Err(e) => {
                        state.motion = MotionOutcome::from_error(&e);
                        state.status = format!("Flight paused: {e}");
                    }
                }
            } else {
                state.status = "Flight paused for nearby collision.".into();
                state.motion = MotionOutcome::MissingCoverage;
            }
        } else {
            let next = state.eye.translated(delta);
            if next.altitude_m(state.field.config().radius_m)
                <= (state.field.config().radius_m * MAX_ORBIT_CLEARANCE_RADII
                    + state.field.config().height_limit_m) as f64
            {
                state.eye = next;
                state.status = "Flying.".into();
            } else {
                state.status = "Flight reached the exploration boundary. Move toward the planet or press Orbit.".into();
            }
        }
    }
    let direction = core(state.rotation * Vec3::new(input.x, 0.0, input.z));
    let position = state.collision_position();
    if let Some(walker) = &state.walker {
        forecast = walker.collision_forecast(direction);
    }
    if (state.walker.is_some() || state.landing || state.clearance_estimate() < 32.0)
        && !state.collision_busy
        && let Some(request) = state.collision.request(position, forecast)
        && state
            .jobs
            .collision
            .try_send(crate::physical_jobs::CollisionRequest {
                field: std::sync::Arc::clone(&state.field),
                position: request,
            })
            .is_ok()
    {
        state.collision_busy = true;
    }
    if state.landing
        && let Some(collision) = state.collision.patch()
    {
        match PhysicalWalker::land(collision, state.eye) {
            Ok(walker) => {
                state.eye = walker.eye();
                state.walker = Some(walker);
                state.mode = Navigation::Walk;
                state.landing = false;
                state.face_horizon();
                state.status = "Walking on one-meter collision terrain.".into();
            }
            Err(e) => {
                state.motion = MotionOutcome::from_error(&e);
                state.status = format!("Landing: {e}");
            }
        }
    }
    if state.mode == Navigation::Walk {
        let started = Instant::now();
        let direction = core(state.rotation * Vec3::new(input.x, 0.0, input.z));
        // Move the walker out briefly to borrow the independent patch.
        if let Some(mut walker) = state.walker.take() {
            if let Some(collision) = state.collision.patch() {
                match walker.advance(collision, direction, dt) {
                    Ok(()) => {
                        state.motion = MotionOutcome::Advanced;
                        state.status = if walker.grounded() {
                            "Walking on one-meter collision terrain."
                        } else {
                            "Falling with collision coverage."
                        }
                        .into();
                    }
                    Err(e) => {
                        state.motion = MotionOutcome::from_error(&e);
                        state.status = format!("Walking paused: {e}");
                    }
                }
                state.eye = walker.eye();
            } else {
                state.motion = MotionOutcome::MissingCoverage;
            }
            state.walker = Some(walker);
        }
        state.query_ms = started.elapsed().as_secs_f32() * 1000.0;
    }
    if state.last_query.elapsed().as_millis() > 200 {
        state.last_query = Instant::now();
        state.ground_clearance = state.collision.patch().and_then(|c| {
            let p = state.eye.relative_to(c.origin_m());
            let up = state.eye.direction();
            c.ray(p, p - up * 24.0)
                .ok()
                .flatten()
                .map(|hit| (p - hit.position_m).length())
        });
    }
}

/// Rotate the position and view together about the current screen axes.
fn drag_orbit(eye: &mut MeterPosition, orientation: &mut Quat, delta: Vec2) {
    let local = Quat::from_scaled_axis(Vec3::new(-delta.y, -delta.x, 0.0) * 0.003);
    let orbit = (*orientation * local * orientation.conjugate()).normalize();
    // This is host camera positioning only. Retain fractional meters while
    // rotating a large planet-relative position, as MeterPosition does in flight.
    let anchor = eye.anchor();
    let offset = eye.relative_to(anchor);
    let position = bevy::math::DVec3::new(
        anchor.x_m as f64 + offset.x as f64,
        anchor.y_m as f64 + offset.y as f64,
        anchor.z_m as f64 + offset.z as f64,
    );
    let next = orbit.as_dquat().normalize() * position;
    let anchor = VoxelPosition {
        x_m: next.x.round() as i32,
        y_m: next.y.round() as i32,
        z_m: next.z.round() as i32,
    };
    *eye = MeterPosition::new(
        anchor,
        procgen_core::Vec3::new(
            (next.x - anchor.x_m as f64) as f32,
            (next.y - anchor.y_m as f64) as f32,
            (next.z - anchor.z_m as f64) as f32,
        ),
    );
    *orientation = (orbit * *orientation).normalize();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_render::vector;

    fn view(radius: i32) -> (MeterPosition, Quat) {
        let eye = MeterPosition::new(
            VoxelPosition {
                x_m: radius,
                y_m: 0,
                z_m: 0,
            },
            procgen_core::Vec3::X * 0.25,
        );
        let orientation = Transform::IDENTITY.looking_to(-Vec3::X, Vec3::Y).rotation;
        (eye, orientation)
    }

    #[test]
    fn horizontal_drags_follow_screen_up_after_crossing_a_pole() {
        let (mut eye, mut orientation) = view(900_000);
        for vertical in [600.0, -250.0, 800.0, -1100.0] {
            drag_orbit(&mut eye, &mut orientation, Vec2::new(0.0, vertical));
            let right = orientation * Vec3::X;
            let up = orientation * Vec3::Y;
            let before = vector(eye.direction());
            drag_orbit(&mut eye, &mut orientation, Vec2::new(100.0, 0.0));
            let after = vector(eye.direction());
            // Horizontal drag moves along screen right, with no vertical motion or roll.
            assert!((after.dot(up) - before.dot(up)).abs() < 0.000001);
            assert!(after.dot(right) < -0.25);
            assert!((orientation * Vec3::Y - up).length() < 0.000001);
            assert!((orientation * Vec3::NEG_Z + after).length() < 0.000001);
        }
    }

    #[test]
    fn repeated_drags_keep_orbit_distance_and_planet_centered() {
        let (mut eye, mut orientation) = view(8_000_000);
        for i in 0..2000 {
            let delta = if i % 2 == 0 {
                Vec2::new(35.0, 70.0)
            } else {
                Vec2::new(-20.0, 15.0)
            };
            drag_orbit(&mut eye, &mut orientation, delta);
            // Sub-meter host positioning should not turn rotation into zoom.
            assert!((eye.altitude_m(8_000_000.0) - 0.25).abs() < 0.001);
            assert!((orientation * Vec3::NEG_Z + vector(eye.direction())).length() < 0.00001);
        }
    }
}
