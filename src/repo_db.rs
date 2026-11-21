//! Repo database generation — port of `repo-add`/`repo-remove`/`db-update`.
//!
//! A pacman sync db (`repo.db`) is an *uncompressed* tar archive containing
//! one directory per package (`<pkgname>-<fullver>-<arch>/`) with a `desc`
//! file in `%FIELD%` format — the same format `SigmaOS-repro-status` and
//! `SigmaOS-rebuild-order` already parse.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use crate::pkgfile::PkgInfo;
use crate::DbError;

/// A repo database: packages keyed by `<pkgname>-<fullver>-<arch>`.
#[derive(Debug, Default)]
pub struct RepoDb {
    /// Packages keyed by their sync-db directory name.
    pub packages: BTreeMap<String, PkgInfo>,
}

impl RepoDb {
    /// Create an empty db.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a package (replaces existing entry with same key).
    pub fn add(&mut self, info: PkgInfo) {
        self.packages.insert(info.file_stem(), info);
    }

    /// Remove a package by file stem or pkgname.
    pub fn remove(&mut self, name: &str) {
        self.packages.retain(|k, v| k != name && v.pkgname != name);
    }

    /// Load an existing `repo.db` tar archive.
    pub fn load(path: &Path) -> Result<Self, DbError> {
        let data = std::fs::read(path)
            .map_err(|e| DbError::InvalidPkgFile(format!("{}: {e}", path.display())))?;
        let mut archive = tar::Archive::new(data.as_slice());
        let mut db = RepoDb::new();
        for entry in archive
            .entries()
            .map_err(|e| DbError::Tar(e.to_string()))?
        {
            let mut entry = entry.map_err(|e| DbError::Tar(e.to_string()))?;
            let p = entry
                .path()
                .map_err(|e| DbError::Tar(e.to_string()))?
                .to_path_buf();
            let s = p.to_string_lossy().replace('\\', "/");
            // Expect "<dir>/desc"
            if !s.ends_with("/desc") {
                continue;
            }
            let mut text = String::new();
            std::io::Read::read_to_string(&mut entry, &mut text)
                .map_err(|e| DbError::Tar(e.to_string()))?;
            if let Some(info) = parse_db_desc(&text) {
                db.packages.insert(info.file_stem(), info);
            }
        }
        Ok(db)
    }
}

/// Parse a sync-db `desc` file back into [`PkgInfo`].
fn parse_db_desc(text: &str) -> Option<PkgInfo> {
    let mut fields: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.trim_end();
        if line.len() > 2 && line.starts_with('%') && line.ends_with('%') {
            let name = line.trim_matches('%').to_string();
            fields.entry(name.clone()).or_default();
            current = Some(name);
        } else if let Some(f) = &current {
            fields.get_mut(f).unwrap().push(line.trim().to_string());
        }
    }
    let get = |k: &str| -> Vec<String> {
        fields
            .get(k)
            .map(|v| v.iter().filter(|s| !s.is_empty()).cloned().collect())
            .unwrap_or_default()
    };
    let first = |k: &str| -> String { get(k).into_iter().next().unwrap_or_default() };
    let version = first("VERSION"); // full epoch:pkgver-pkgrel
    if version.is_empty() {
        return None;
    }
    // Split version into components.
    let (epoch, rest) = match version.split_once(':') {
        Some((e, r)) => (e.parse().unwrap_or(0), r),
        None => (0, version.as_str()),
    };
    let (pkgver, pkgrel) = match rest.rsplit_once('-') {
        Some((v, r)) => (v.to_string(), r.to_string()),
        None => (rest.to_string(), String::new()),
    };
    Some(PkgInfo {
        pkgname: first("NAME"),
        pkgbase: first("BASE"),
        pkgver,
        pkgrel,
        epoch,
        pkgdesc: first("DESC"),
        url: first("URL"),
        arch: first("ARCH"),
        builddate: first("BUILDDATE").parse().unwrap_or(0),
        packager: first("PACKAGER"),
        size: first("SIZE").parse().unwrap_or(0),
        isize: first("ISIZE").parse().unwrap_or(0),
        licenses: get("LICENSE"),
        depends: get("DEPENDS"),
        optdepends: get("OPTDEPENDS"),
        conflicts: get("CONFLICTS"),
        replaces: get("REPLACES"),
        provides: get("PROVIDES"),
        groups: get("GROUPS"),
        makedepends: vec![],
        checkdepends: vec![],
    })
}

/// Render a [`PkgInfo`] as a sync-db `desc` file body.
fn render_desc(info: &PkgInfo) -> String {
    let mut out = String::new();
    let mut field = |name: &str, values: &[String]| {
        out.push_str(&format!("%{name}%\n"));
        for v in values {
            if !v.is_empty() {
                out.push_str(v);
                out.push('\n');
            }
        }
    };
    let one = |v: &str| -> Vec<String> { vec![v.to_string()] };
    field("NAME", &one(&info.pkgname));
    field("BASE", &one(&info.pkgbase));
    field("VERSION", &one(&info.full_version()));
    field("DESC", &one(&info.pkgdesc));
    field("URL", &one(&info.url));
    field("ARCH", &one(&info.arch));
    field("BUILDDATE", &[info.builddate.to_string()]);
    field("PACKAGER", &one(&info.packager));
    field("SIZE", &[info.size.to_string()]);
    field("ISIZE", &[info.isize.to_string()]);
    if !info.licenses.is_empty() {
        field("LICENSE", &info.licenses);
    }
    if !info.replaces.is_empty() {
        field("REPLACES", &info.replaces);
    }
    if !info.groups.is_empty() {
        field("GROUPS", &info.groups);
    }
    if !info.depends.is_empty() {
        field("DEPENDS", &info.depends);
    }
    if !info.optdepends.is_empty() {
        field("OPTDEPENDS", &info.optdepends);
    }
    if !info.conflicts.is_empty() {
        field("CONFLICTS", &info.conflicts);
    }
    if !info.provides.is_empty() {
        field("PROVIDES", &info.provides);
    }
    out
}

/// Write the db as an uncompressed tar archive to `out`.
pub fn write_repo_db(db: &RepoDb, out: &Path) -> Result<(), DbError> {
    let file = std::fs::File::create(out)?;
    let mut builder = tar::Builder::new(file);
    for (stem, info) in &db.packages {
        let desc = render_desc(info);
        let mut header = tar::Header::new_gnu();
        header.set_size(desc.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                format!("{stem}/desc"),
                desc.as_bytes(),
            )
            .map_err(|e| DbError::Tar(e.to_string()))?;
    }
    builder.finish().map_err(|e| DbError::Tar(e.to_string()))?;
    Ok(())
}

/// `repo-add` semantics: add packages to an existing db file (or create new).
pub fn repo_add(db_path: &Path, infos: &[PkgInfo]) -> Result<(), DbError> {
    let mut db = if db_path.exists() {
        RepoDb::load(db_path)?
    } else {
        RepoDb::new()
    };
    for info in infos {
        db.add(info.clone());
    }
    write_repo_db(&db, db_path)
}

/// `repo-remove` semantics: remove packages by name/stem from a db file.
pub fn repo_remove(db_path: &Path, names: &[String]) -> Result<(), DbError> {
    if !db_path.exists() {
        return Err(DbError::CheckFailed(format!(
            "no database found at {}",
            db_path.display()
        )));
    }
    let mut db = RepoDb::load(db_path)?;
    for n in names {
        db.remove(n);
    }
    write_repo_db(&db, db_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str, ver: &str, rel: &str) -> PkgInfo {
        PkgInfo {
            pkgname: name.into(),
            pkgbase: name.into(),
            pkgver: ver.into(),
            pkgrel: rel.into(),
            arch: "x86_64".into(),
            pkgdesc: "desc".into(),
            url: "https://x".into(),
            builddate: 1700000000,
            packager: "TU".into(),
            size: 100,
            isize: 200,
            licenses: vec!["MIT".into()],
            depends: vec!["glibc".into()],
            ..Default::default()
        }
    }

    #[test]
    fn test_repo_db_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("extra.db");
        let infos = vec![info("foo", "1.0", "1"), info("bar", "2.0", "3")];
        repo_add(&db_path, &infos).unwrap();
        let db = RepoDb::load(&db_path).unwrap();
        assert_eq!(db.packages.len(), 2);
        let foo = db.packages.get("foo-1.0-1-x86_64").unwrap();
        assert_eq!(foo.pkgname, "foo");
        assert_eq!(foo.depends, vec!["glibc"]);
        assert_eq!(foo.licenses, vec!["MIT"]);
    }

    #[test]
    fn test_repo_remove() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("extra.db");
        repo_add(&db_path, &[info("foo", "1.0", "1"), info("bar", "1.0", "1")]).unwrap();
        repo_remove(&db_path, &["foo".to_string()]).unwrap();
        let db = RepoDb::load(&db_path).unwrap();
        assert_eq!(db.packages.len(), 1);
        assert!(db.packages.contains_key("bar-1.0-1-x86_64"));
    }

    #[test]
    fn test_repo_add_replaces_same_version() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("extra.db");
        repo_add(&db_path, &[info("foo", "1.0", "1")]).unwrap();
        let mut updated = info("foo", "1.0", "1");
        updated.pkgdesc = "new desc".into();
        repo_add(&db_path, &[updated]).unwrap();
        let db = RepoDb::load(&db_path).unwrap();
        assert_eq!(db.packages["foo-1.0-1-x86_64"].pkgdesc, "new desc");
    }

    #[test]
    fn test_db_is_plain_tar() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("core.db");
        repo_add(&db_path, &[info("linux", "6.1", "1")]).unwrap();
        let data = std::fs::read(&db_path).unwrap();
        // ustar magic at offset 257
        assert_eq!(&data[257..262], b"ustar");
    }

    #[test]
    fn test_write_repo_db_produces_desc_entries() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.db");
        let mut db = RepoDb::new();
        db.add(info("vim", "9.0", "1"));
        write_repo_db(&db, &out).unwrap();
        let loaded = RepoDb::load(&out).unwrap();
        assert!(loaded.packages.contains_key("vim-9.0-1-x86_64"));
    }
}
