//! Presentation support shared by the workspace's viewer applications.
//!
//! Nothing here generates anything. It holds the parts two viewers would
//! otherwise copy: the orbit controls and the identity colour ramp.

mod camera;
mod palette;

pub use camera::{Orbit, OrbitCamera, OrbitCameraPlugin, OrbitLimits};
pub use palette::{ID_HUE_STEP_DEGREES, ID_LIGHTNESS, ID_SATURATION, id_color};
