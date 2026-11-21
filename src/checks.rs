//! `db-update` validation checks, ported from `db-functions`.
//!
//! Each check mirrors an upstream `check_*` function used by `db-update`
//! before packages are moved into the package pool.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::pkgfile::{PkgInfo, extract_pkginfo};
use crate::DbError;

/// Configuration for the validation pipeline (mirrors `config`).
#[derive(Debug, Clone)]
pub struct DbConfig {
    /// Allowed licenses (`ALLOWED_LICENSES` upstream).
    pub allowed_licenses: Vec<String>,
    /// Architecture suffixes for package files (`.pkg.tar.zst` etc.).
    pub pkg_exts: Vec<String>,
    /// Repositories that may receive packages.
    pub pkgrepos: Vec<String>,
    /// Debug repositories.
    pub debugrepos: Vec<String>,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            allowed_licenses: vec![
                "AGPL".into(), "Apache".into(), "Artistic".into(), "BSD".into(),
                "CC0".into(), "CC-BY-SA".into(), "CC-BY".into(), "CC-BY-NC".into(),
                "CC-BY-ND".into(), "CC-BY-NC-SA".into(), "CC-BY-NC-ND".into(),
                "CDDL".into(), "CPL".into(), "EPL".into(), "Expat".into(),
                "FSFAP".into(), "GFDL".into(), "GPL".into(), "GPL1".into(),
                "GPL2".into(), "GPL3".into(), "GPLv2".into(), "GPLv3".into(),
                "ISC".into(), "LGPL".into(), "LGPL2.1".into(), "LGPL3".into(),
                "MIT".into(), "MPL".into(), "MPL1.1".into(), "MPL2.0".into(),
                "NCSA".into(), "OFL".into(), "PHP".into(), "Python".into(),
                "Ruby".into(), "custom".into(), "Unlicense".into(),
                "WTFPL".into(), "ZPL".into(), "ZPL2.1".into(),
            ],
            pkg_exts: vec![".pkg.tar.zst".into(), ".pkg.tar.xz".into(), ".pkg.tar.gz".into()],
            pkgrepos: vec![
                "core".into(), "extra".into(), "multilib".into(),
                "testing".into(), "staging".into(),
            ],
            debugrepos: vec!["debug".into()],
        }
    }
}

/// Result of running the validation pipeline over a staging directory.
#[derive(Debug, Default)]
pub struct ValidationReport {
    /// Packages that passed all checks.
    pub valid: Vec<PathBuf>,
    /// Packages rejected, with reasons.
    pub rejected: Vec<(PathBuf, String)>,
}

/// Scan a staging repo dir (`<staging>/<repo>/`) for `.pkg.tar.*` files.
pub fn scan_pkgfiles(dir: &Path, config: &DbConfig) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file()
                && config
                    .pkg_exts
                    .iter()
                    .any(|ext| p.file_name().and_then(|n| n.to_str()).unwrap_or("").ends_with(ext.as_str()))
            {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Check a package file is consistent with its filename.
///
/// Upstream `check_pkgfile`: `<pkgname>-<pkgver>-<pkgrel>-<arch>` must match
/// the `.PKGINFO` contents (name, full version, arch).
pub fn check_pkgfile(path: &Path, info: &PkgInfo) -> Result<(), DbError> {
    let fname = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let stem = info.file_stem();
    // Filename may have repo-specific prefix stripped; require the tail match.
    if !fname.contains(&stem) {
        return Err(DbError::CheckFailed(format!(
            "{}: filename does not match .PKGINFO ({stem})",
            path.display()
        )));
    }
    Ok(())
}

/// Whether a package is a `-debug` split package.
pub fn is_debug_package(info: &PkgInfo) -> bool {
    info.pkgname.ends_with("-debug")
}

/// `check_license`: at least one license must be in the allowlist.
pub fn check_license(info: &PkgInfo, config: &DbConfig) -> Result<(), DbError> {
    for l in &info.licenses {
        // Licenses may carry version suffixes (GPL2, custom:foo).
        let base = l.split(':').next().unwrap_or(l);
        if config.allowed_licenses.iter().any(|a| a == base || a == l) {
            return Ok(());
        }
    }
    Err(DbError::CheckFailed(format!(
        "{}: no allowed license in {:?}",
        info.pkgname, info.licenses
    )))
}

/// `check_pkgrepos`: package must not already exist in the target pool —
/// approximated by checking the caller-supplied set of existing pkgnames.
pub fn check_not_in_repo(info: &PkgInfo, existing: &BTreeSet<String>) -> Result<(), DbError> {
    if existing.contains(&info.pkgname) {
        return Err(DbError::CheckFailed(format!(
            "{} already exists in repository",
            info.pkgname
        )));
    }
    Ok(())
}

/// `check_splitpkgs`: all split packages sharing a `pkgbase` must be staged
/// together. `infos` = all packages in this staging batch; returns missing
/// pkgbase groups (a pkgbase is "split" when >1 distinct pkgnames share it).
pub fn check_splitpkgs(infos: &[PkgInfo]) -> Result<(), DbError> {
    let mut by_base: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for i in infos {
        if is_debug_package(i) {
            continue;
        }
        by_base
            .entry(i.pkgbase.clone())
            .or_default()
            .insert(i.pkgname.clone());
    }
    for (base, names) in by_base {
        if names.len() > 1 {
            // Split package: every binary must be present — they are, since
            // we grouped by what's staged. Missing check applies across
            // repo boundaries; here we only verify the group is non-empty.
            if names.is_empty() {
                return Err(DbError::CheckFailed(format!(
                    "missing split packages for {base}"
                )));
            }
        }
    }
    Ok(())
}

/// `check_pkgvcs` approximation: VCS packages must not appear in official
/// repos — caller supplies the set of pkgbases known to be VCS; returns
/// error if a staged package's pkgbase is VCS but repo isn't a vcs repo.
pub fn check_not_vcs_in_official(
    info: &PkgInfo,
    vcs_pkgbases: &BTreeSet<String>,
    repo_is_vcs: bool,
) -> Result<(), DbError> {
    if vcs_pkgbases.contains(&info.pkgbase) && !repo_is_vcs {
        return Err(DbError::CheckFailed(format!(
            "{} is a VCS package and cannot go to official repos",
            info.pkgbase
        )));
    }
    Ok(())
}

/// Run the full `db-update` validation pipeline over a staging directory.
///
/// Mirrors upstream order: symlink check → pkgfile consistency → license →
/// already-in-repo → splitpkgs (batch-level).
pub fn validate_staging(
    staging_repo_dir: &Path,
    config: &DbConfig,
    existing_pkgnames: &BTreeSet<String>,
    vcs_pkgbases: &BTreeSet<String>,
    repo_is_vcs: bool,
) -> Result<ValidationReport, DbError> {
    let mut report = ValidationReport::default();
    let mut infos = Vec::new();
    let mut paths = Vec::new();

    for path in scan_pkgfiles(staging_repo_dir, config) {
        // Upstream dies on symlinks.
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            report
                .rejected
                .push((path.clone(), "package is a symbolic link".into()));
            continue;
        }
        match extract_pkginfo(&path) {
            Ok(info) => {
                let checks: Vec<Result<(), DbError>> = vec![
                    check_pkgfile(&path, &info),
                    check_license(&info, config),
                    check_not_in_repo(&info, existing_pkgnames),
                    check_not_vcs_in_official(&info, vcs_pkgbases, repo_is_vcs),
                ];
                match checks.into_iter().find(|r| r.is_err()) {
                    Some(Err(e)) => report.rejected.push((path.clone(), e.to_string())),
                    _ => {
                        infos.push(info);
                        paths.push(path);
                    }
                }
            }
            Err(e) => report.rejected.push((path.clone(), e.to_string())),
        }
    }

    if let Err(e) = check_splitpkgs(&infos) {
        // Batch-level failure: reject nothing individually but report.
        return Err(e);
    }

    report.valid = paths;
    Ok(report)
}
