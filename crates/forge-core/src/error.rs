#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    #[error(transparent)]
    Other(#[from] anyhow::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ForgeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forge_error_wraps_anyhow() {
        let anyhow_err = anyhow::anyhow!("something went wrong");
        let err: ForgeError = anyhow_err.into();
        assert!(matches!(err, ForgeError::Other(_)));
    }

    #[test]
    fn result_alias_is_forge_error() {
        let ok: Result<i32> = Ok(42);
        assert!(ok.is_ok());

        let err: Result<i32> = Err(anyhow::anyhow!("fail").into());
        assert!(err.is_err());
    }
}
