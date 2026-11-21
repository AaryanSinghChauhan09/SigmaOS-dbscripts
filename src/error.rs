//! Error types for the dbscripts component.

/// Errors that can occur during repo db generation or package validation.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// The package file is not a valid `.pkg.tar.*` archive or lacks `.PKGINFO`.
    #[error("invalid package file: {0}")]
    InvalidPkgFile(String),

    /// A required field was missing or malformed in `.PKGINFO`.
    #[error("bad .PKGINFO: {0}")]
    BadPkgInfo(String),

    /// A validation check failed.
    #[error("check failed: {0}")]
    CheckFailed(String),

    /// I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Tar archive error.
    #[error("tar error: {0}")]
    Tar(String),
}
