//! SigmaOS dbscripts component.
//!
//! Rust implementation of `archlinux/dbscripts`: the tooling that turns
//! staged `.pkg.tar.*` files into pacman repository databases (`repo.db`)
//! and validates packages before publication (`db-update` checks).
//!
//! Ported semantics:
//! - [`pkgfile::extract_pkginfo`] — read `.PKGINFO`/`.BUILDINFO` from
//!   `.pkg.tar.zst|xz|gz` archives (upstream `bsdtar -xOf`).
//! - [`repo_db`] — generate `repo.db` tar archives in pacman's sync-db
//!   `desc` format; `repo_add`/`repo_remove` mirror `repo-add`/`repo-remove`.
//! - [`checks`] — the `db-update` validation pipeline: pkgfile consistency,
//!   symlink checks, license allowlist, split-package completeness,
//!   already-in-repo checks.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod checks;
pub mod error;
pub mod pkgfile;
pub mod repo_db;

pub use error::DbError;
pub use pkgfile::{BuildInfo, PkgInfo, extract_buildinfo, extract_pkginfo};
pub use checks::{DbConfig, ValidationReport, check_license, check_not_in_repo, check_pkgfile, check_splitpkgs, validate_staging};
pub use repo_db::{RepoDb, repo_add, repo_remove, write_repo_db};
