//! JSON-RPC method and event types for the Forge daemon API.
//!
//! These Rust types are the source of truth for the wire format; `just gen` writes the
//! TypeScript the UI uses from them.

#[cfg(feature = "client")]
pub mod client;
pub mod error;
pub mod events;
pub mod ids;
pub mod methods;
pub mod model;
pub mod rpc;
mod timestamp;
pub mod typescript;

pub use error::{ApiError, ErrorData, ErrorKind};
pub use ids::*;
pub use timestamp::Timestamp;

/// API version (semver). Adding a field or method is a minor bump; removing or changing one is major.
pub const API_VERSION: &str = "0.2.0";

/// The major component of a semver string, if it has one.
pub fn major_version(version: &str) -> Option<u64> {
    version.split('.').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn api_version_has_a_major() {
        assert_eq!(super::major_version(super::API_VERSION), Some(0));
        assert_eq!(super::major_version("x.1"), None);
    }
}
