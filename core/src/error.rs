use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("store is missing at {0}")]
    StoreMissing(PathBuf),
    #[error("store version mismatch: expected {expected}, found {found}")]
    StoreVersionMismatch { expected: u32, found: u32 },
    #[error("fingerprint config mismatch: expected {expected}, found {found}")]
    ConfigMismatch { expected: String, found: String },
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("invalid dump {path}:{line}: {message}")]
    InvalidDump {
        path: PathBuf,
        line: usize,
        message: String,
    },
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid store: {0}")]
    InvalidStore(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn wire_kind(&self) -> &'static str {
        match self {
            Self::StoreMissing(_) => "store_missing",
            Self::StoreVersionMismatch { .. } => "store_version_mismatch",
            Self::ConfigMismatch { .. } => "config_mismatch",
            Self::BadRequest(_) | Self::InvalidDump { .. } => "bad_request",
            Self::Unsupported(_) => "unsupported",
            Self::Io { .. } | Self::InvalidStore(_) | Self::Internal(_) => "internal",
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
