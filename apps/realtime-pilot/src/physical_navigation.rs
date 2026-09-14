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
    if !pointer_blocked && state.mode == Navigation::Orbit && buttons.pressed(MouseButton::Left) {
        let rotation = Quat::from_rotation_y(-mouse.delta.x * 0.003)
            * Quat::from_axis_angle(state.rotation * Vec3::X, -mouse.delta.y * 0.003);
        let radius = state.field.config().radius_m
            + state.eye.altitude_m(state.field.config().radius_m) as f32;
        let p = rotation * up * radius;
        state.eye = MeterPosition::new(
            VoxelPosition {
                x_m: p.x.round() as i32,
                y_m: p.y.round() as i32,
                z_m: p.z.round() as i32,
            },
            procgen_core::Vec3::ZERO,
        );
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
