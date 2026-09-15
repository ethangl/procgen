//! Build identity stamped into headless audit output so a JSON report can be
//! traced back to the exact sources that produced it. `build.rs` hashes every
//! source file of this crate and its generation dependencies into PILOT_BUILD_ID.
pub const TOOLCHAIN: &str = env!("PILOT_RUSTC");
pub const BUILD_ID: &str = env!("PILOT_BUILD_ID");
