//! Presentation support shared by the workspace's viewer applications.
//!
//! Nothing here generates anything. It holds the parts two viewers would
//! otherwise copy: the orbit controls and the identity colour ramp.

mod camera;
mod palette;

pub use camera::{OrbitCamera, OrbitCameraPlugin, OrbitLimits};
pub use palette::id_color;
