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
mod scalar_field;

pub use fingerprint::{fingerprint, quantized_fingerprint};
pub use hash32::{HASH_U32_TEST_VECTORS, hash_u32};
pub use math::Vec3;
pub use random::RandomStream;
pub use scalar_field::ScalarFieldSample3;

/// WGSL mirror of the canonical four-word 32-bit hash.
///
/// The source is backend-neutral shader text and introduces no dependency.
/// Every crate that hashes in WGSL composes this source rather than
/// restating the constants.
pub const HASH_WGSL_SOURCE: &str = include_str!("../wgsl/hash.wgsl");
