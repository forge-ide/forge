//! Project, workspace, and agent-profile config with layering.

/// This crate's package name.
pub const NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    #[test]
    fn name_matches_package() {
        assert_eq!(super::NAME, "forge-config");
    }
}
