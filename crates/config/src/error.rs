use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Could not read config file at '{path}': {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },

    #[error("Could not parse config file: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("Validation error: {0}")]
    Validation(String),
}
