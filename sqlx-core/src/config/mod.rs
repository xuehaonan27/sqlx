//! (Exported for documentation only) Guide and reference for `sqlx.toml` files.
//!
//! To use, create a `sqlx.toml` file in your crate root (the same directory as your `Cargo.toml`).
//! The configuration in a `sqlx.toml` configures SQLx *only* for the current crate.
//!
//! See the [`Config`] type and its fields for individual configuration options.
//!
//! See the [reference][`_reference`] for the full `sqlx.toml` file.

use std::error::Error;
use std::fmt::Debug;
use std::io;
use std::path::{Path, PathBuf};

// `std::sync::OnceLock` doesn't have a stable `.get_or_try_init()`
// because it's blocked on a stable `Try` trait.
use once_cell::sync::OnceCell;

/// Configuration shared by multiple components.
///
/// See [`common::Config`] for details.
pub mod common;

/// Configuration for the `query!()` family of macros.
///
/// See [`macros::Config`] for details.
pub mod macros;

/// Configuration for migrations when executed using `sqlx::migrate!()` or through `sqlx-cli`.
///
/// See [`migrate::Config`] for details.
pub mod migrate;

/// Reference for `sqlx.toml` files
///
/// Source: `sqlx-core/src/config/reference.toml`
///
/// ```toml
#[doc = include_str!("reference.toml")]
/// ```
pub mod _reference {}

#[cfg(all(test, feature = "sqlx-toml"))]
mod tests;

/// The parsed structure of a `sqlx.toml` file.
#[derive(Debug, Default)]
#[cfg_attr(
    feature = "sqlx-toml",
    derive(serde::Deserialize),
    serde(default, rename_all = "kebab-case")
)]
pub struct Config {
    /// Configuration shared by multiple components.
    ///
    /// See [`common::Config`] for details.
    pub common: common::Config,

    /// Configuration for the `query!()` family of macros.
    ///
    /// See [`macros::Config`] for details.
    pub macros: macros::Config,

    /// Configuration for migrations when executed using `sqlx::migrate!()` or through `sqlx-cli`.
    ///
    /// See [`migrate::Config`] for details.
    pub migrate: migrate::Config,
}

/// Error returned from various methods of [`Config`].
#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    /// The loading method expected `CARGO_MANIFEST_DIR` to be set and it wasn't.
    ///
    /// This is necessary to locate the root of the crate currently being compiled.
    ///
    /// See [the "Environment Variables" page of the Cargo Book][cargo-env] for details.
    ///
    /// [cargo-env]: https://doc.rust-lang.org/cargo/reference/environment-variables.html#environment-variables-cargo-sets-for-crates
    #[error("environment variable `CARGO_MANIFEST_DIR` must be set and valid")]
    Env(
        #[from]
        #[source]
        std::env::VarError,
    ),

    /// No configuration file was found. Not necessarily fatal.
    #[error("config file {path:?} not found")]
    NotFound {
        path: PathBuf,
    },

    /// An I/O error occurred while attempting to read the config file at `path`.
    ///
    /// If the error is [`io::ErrorKind::NotFound`], [`Self::NotFound`] is returned instead.
    #[error("error reading config file {path:?}")]
    Io {
        path: PathBuf,
        #[source]
        error: io::Error,
    },

    /// An error in the TOML was encountered while parsing the config file at `path`.
    ///
    /// The error gives line numbers and context when printed with `Display`/`ToString`.
    /// 
    /// Only returned if the `sqlx-toml` feature is enabled.
    #[error("error parsing config file {path:?}")]
    Parse {
        path: PathBuf,
        /// Type-erased [`toml::de::Error`].
        #[source]
        error: Box<dyn Error + Send + Sync + 'static>,
    },

    /// A `sqlx.toml` file was found or specified, but the `sqlx-toml` feature is not enabled.
    #[error("SQLx found config file at {path:?} but the `sqlx-toml` feature was not enabled")]
    ParseDisabled {
        path: PathBuf
    },
}

impl ConfigError {
    /// Create a [`ConfigError`] from a [`std::io::Error`].
    /// 
    /// Maps to either `NotFound` or `Io`.
    pub fn from_io(path: PathBuf, error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::NotFound {
            Self::NotFound { path }
        } else {
            Self::Io { path, error }
        }
    }
    
    /// If this error means the file was not found, return the path that was attempted.
    pub fn not_found_path(&self) -> Option<&Path> {
        if let Self::NotFound { path } = self {
            Some(path)
        } else {
            None
        }
    }
}

static CACHE: OnceCell<Config> = OnceCell::new();

/// Internal methods for loading a `Config`.
#[allow(clippy::result_large_err)]
impl Config {
    /// Get the cached config, or attempt to read `$CARGO_MANIFEST_DIR/sqlx.toml`.
    ///
    /// On success, the config is cached in a `static` and returned by future calls.
    ///
    /// Returns `Config::default()` if the file does not exist.
    ///
    /// ### Panics
    /// If the file exists but an unrecoverable error was encountered while parsing it.
    pub fn from_crate() -> &'static Self {
        Self::try_from_crate().unwrap_or_else(|e| {
            match e {
                ConfigError::NotFound { path } => {
                    // Non-fatal
                    tracing::debug!("Not reading config, file {path:?} not found");
                    CACHE.get_or_init(Config::default)
                }
                // FATAL ERRORS BELOW:
                // In the case of migrations,
                // we can't proceed with defaults as they may be completely wrong.
                e @ ConfigError::ParseDisabled { .. } => {
                    // Only returned if the file exists but the feature is not enabled.
                    panic!("{e}")
                }
                e => {
                    panic!("failed to read sqlx config: {e}")
                }
            }
        })
    }

    /// Get the cached config, or to read `$CARGO_MANIFEST_DIR/sqlx.toml`.
    ///
    /// On success, the config is cached in a `static` and returned by future calls.
    ///
    /// Errors if `CARGO_MANIFEST_DIR` is not set, or if the config file could not be read.
    pub fn try_from_crate() -> Result<&'static Self, ConfigError> {
        Self::try_get_with(|| {
            let mut path = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
            path.push("sqlx.toml");
            Ok(path)
        })
    }

    /// Get the cached config, or attempt to read `sqlx.toml` from the current working directory.
    ///
    /// On success, the config is cached in a `static` and returned by future calls.
    ///
    /// Errors if the config file does not exist, or could not be read.
    pub fn try_from_current_dir() -> Result<&'static Self, ConfigError> {
        Self::try_get_with(|| Ok("sqlx.toml".into()))
    }

    /// Get the cached config, or attempt to read it from the path returned by the closure.
    ///
    /// On success, the config is cached in a `static` and returned by future calls.
    ///
    /// Errors if the config file does not exist, or could not be read.
    pub fn try_get_with(
        make_path: impl FnOnce() -> Result<PathBuf, ConfigError>,
    ) -> Result<&'static Self, ConfigError> {
        CACHE.get_or_try_init(|| {
            let path = make_path()?;
            Self::read_from(path)
        })
    }

    #[cfg(feature = "sqlx-toml")]
    fn read_from(path: PathBuf) -> Result<Self, ConfigError> {
        // The `toml` crate doesn't provide an incremental reader.
        let toml_s = match std::fs::read_to_string(&path) {
            Ok(toml) => toml,
            Err(error) => {
                return Err(ConfigError::from_io(path, error));
            }
        };

        // TODO: parse and lint TOML structure before deserializing
        // Motivation: https://github.com/toml-rs/toml/issues/761
        tracing::debug!("read config TOML from {path:?}:\n{toml_s}");

        toml::from_str(&toml_s).map_err(|error| ConfigError::Parse { path, error: Box::new(error) })
    }
    
    #[cfg(not(feature = "sqlx-toml"))]
    fn read_from(path: PathBuf) -> Result<Self, ConfigError> {
        match path.try_exists() {
            Ok(true) => Err(ConfigError::ParseDisabled { path }),
            Ok(false) => Err(ConfigError::NotFound { path }),
            Err(e) => Err(ConfigError::from_io(path, e))
        }
    }
}
