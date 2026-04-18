//! Provider spec parser for `forged --provider <spec>` and `FORGE_PROVIDER` env.
//!
//! Grammar: `<kind>` or `<kind>:<rest>`. The first colon separates kind from
//! rest — Ollama model values themselves contain colons (`qwen2.5:0.5b`), so
//! the parser does not split on subsequent colons.

use anyhow::{anyhow, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderKind {
    Mock,
    Ollama { model: String },
}

pub fn parse_provider_spec(spec: &str) -> Result<ProviderKind> {
    if spec.is_empty() {
        return Err(anyhow!("provider spec is empty"));
    }
    let (kind, rest) = match spec.split_once(':') {
        Some((k, r)) => (k, Some(r)),
        None => (spec, None),
    };
    match kind {
        "mock" => Ok(ProviderKind::Mock),
        "ollama" => {
            let model = rest
                .filter(|r| !r.is_empty())
                .ok_or_else(|| anyhow!("ollama spec requires a model: ollama:<model>"))?;
            Ok(ProviderKind::Ollama {
                model: model.to_string(),
            })
        }
        other => Err(anyhow!(
            "unknown provider kind: {other:?} (supported: mock, ollama)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_mock() {
        assert_eq!(parse_provider_spec("mock").unwrap(), ProviderKind::Mock);
    }

    #[test]
    fn parses_ollama_with_simple_model() {
        assert_eq!(
            parse_provider_spec("ollama:llama3").unwrap(),
            ProviderKind::Ollama {
                model: "llama3".to_string()
            }
        );
    }

    #[test]
    fn parses_ollama_with_colon_in_model() {
        assert_eq!(
            parse_provider_spec("ollama:qwen2.5:0.5b").unwrap(),
            ProviderKind::Ollama {
                model: "qwen2.5:0.5b".to_string()
            }
        );
    }

    #[test]
    fn rejects_empty_spec() {
        assert!(parse_provider_spec("").is_err());
    }

    #[test]
    fn rejects_unknown_kind() {
        let err = parse_provider_spec("anthropic:claude").unwrap_err();
        assert!(err.to_string().contains("unknown"));
    }

    #[test]
    fn rejects_ollama_without_model() {
        let err = parse_provider_spec("ollama:").unwrap_err();
        assert!(err.to_string().contains("model"));
    }

    #[test]
    fn rejects_ollama_bare_no_colon() {
        // `ollama` without `:<model>` is ambiguous — reject rather than guess.
        let err = parse_provider_spec("ollama").unwrap_err();
        assert!(err.to_string().contains("model"));
    }
}
