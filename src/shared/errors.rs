use thiserror::Error;

/// Application result with user-facing error variants.
pub type AppResult<T> = Result<T, AppError>;
pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug, Error)]
/// Configuration loading or validation error.
pub enum ConfigError {
    #[error("missing configuration value: {0}")]
    /// A required field is absent.
    Missing(&'static str),
    #[error("invalid configuration value: {0}")]
    /// A field violates its value rules.
    Invalid(String),
    #[error("configuration parse error: {0}")]
    /// TOML syntax or type parsing failed.
    Parse(#[from] toml::de::Error),
    #[error("invalid endpoint ID: {0}")]
    /// The value is not a valid iroh endpoint ID.
    InvalidNodeId(#[source] iroh::KeyParsingError),
    #[error("unknown host alias: {0}")]
    /// The requested access alias is not configured.
    UnknownHost(String),
}

#[derive(Debug, Error)]
/// Command-line argument validation error.
pub enum ArgumentError {
    #[error("invalid argument: {0}")]
    /// An argument violates its value rules.
    Invalid(String),
}

#[derive(Debug, Error)]
/// Top-level application error.
pub enum AppError {
    #[error("configuration error: {0}")]
    /// Configuration failed to load or validate.
    Config(#[from] ConfigError),
    #[error("I/O error: {0}")]
    /// A filesystem or stream I/O operation failed.
    Io(#[from] std::io::Error),
    #[error("iroh error: {0}")]
    /// An iroh operation failed.
    Iroh(#[source] BoxError),
    #[error("SSH error: {0}")]
    /// An SSH operation failed.
    Ssh(#[source] BoxError),
    #[error("service error: {0}")]
    /// A service operation failed.
    Service(#[source] BoxError),
    #[error("argument error: {0}")]
    /// A command-line argument was invalid.
    InvalidArgument(#[from] ArgumentError),
}

pub(crate) fn iroh_error<E>(error: E) -> AppError
where
    E: std::error::Error + Send + Sync + 'static,
{
    AppError::Iroh(Box::new(error))
}

pub(crate) fn timeout_error(message: &'static str) -> AppError {
    std::io::Error::new(std::io::ErrorKind::TimedOut, message).into()
}
