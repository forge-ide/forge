//! Small text helpers.

/// `name` with its first letter capitalized.
pub fn title(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_capitalizes_the_first_letter() {
        assert_eq!(title("forge"), "Forge");
        assert_eq!(title(""), "");
    }

    #[test]
    fn slugify_lowercases_and_joins_words_with_hyphens() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
    }
}
