//! Dependency-free primitives shared by procedural-generation crates.
//!
//! This crate is intentionally narrow: backend-neutral value types and
//! deterministic pure operations belong here; algorithms and execution policy
//! do not.

mod fingerprint;
mod hash32;
mod math;
mod random;
pub mod random_streams;

pub use fingerprint::fingerprint;
pub use hash32::{HASH_U32_TEST_VECTORS, hash_u32};
pub use math::Vec3;
pub use random::RandomStream;
