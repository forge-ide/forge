//! Scripted ACP agent that replays YAML steps for tests.

fn banner() -> String {
    format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

fn main() {
    println!("{}", banner());
}

#[cfg(test)]
mod tests {
    #[test]
    fn banner_names_binary() {
        assert!(super::banner().starts_with("forge-fake-agent "));
    }
}
