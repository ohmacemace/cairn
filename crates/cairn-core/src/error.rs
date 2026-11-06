use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("toml encode error: {0}")]
    TomlEncode(#[from] toml::ser::Error),

    #[error("git error: {0}")]
    Git(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("store error: {0}")]
    Store(String),

    #[error("adapter error: {0}")]
    Adapter(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid path {0}: {1}")]
    InvalidPath(PathBuf, String),
}

pub type Result<T> = std::result::Result<T, Error>;
