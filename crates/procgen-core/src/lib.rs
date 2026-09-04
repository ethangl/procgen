//! Dependency-free primitives shared by procedural-generation crates.
//!
//! This crate is intentionally narrow: backend-neutral value types and
//! deterministic pure operations belong here; algorithms and execution policy
//! do not.

mod math;
mod random;

pub use math::Vec3;
pub use random::RandomStream;
