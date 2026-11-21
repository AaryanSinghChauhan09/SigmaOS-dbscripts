//! Integration tests for sigmaos-dbscripts.

use std::collections::BTreeSet;
use std::path::Path;

use dbscripts::{
    DbConfig, PkgInfo, RepoDb, check_license, check_not_in_repo, check_pkgfile,
    check_splitpkgs, extract_pkginfo, repo_add, repo_remove, validate_staging,
};

fn info(name: &str, base: &str, ver: &str, rel: &str, arch: &str) -> PkgInfo {
    PkgInfo {
        pkgname: name.into(),
        pkgbase: base.into(),
        pkgver: ver.into(),
        pkgrel: rel.into(),
        arch: arch.into(),
        pkgdesc: "d".into(),
        licenses: vec!["MIT".into()],
        ..Default::default()
    }
}

fn make_pkg(dir: &Path, stem: &str, pkginfo: &str) -> std::path::PathBuf {
    let mut tar_data = Vec::new();
    {
        let mut b = tar::Builder::new(&mut tar_data);
        let mut h = tar::Header::new_gnu();
        h.set_size(pkginfo.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, ".PKGINFO", pkginfo.as_bytes()).unwrap();
        b.finish().unwrap();
    }
    let compressed = zstd::stream::encode_all(tar_data.as_slice(), 3).unwrap();
    let path = dir.join(format!("{stem}.pkg.tar.zst"));
    std::fs::write(&path, compressed).unwrap();
    path
}

#[test]
fn test_full_db_update_flow() {
    let dir = tempfile::tempdir().unwrap();
    let staging = dir.path().join("extra");
    std::fs::create_dir_all(&staging).unwrap();
    let pkginfo = "pkgname = foo\npkgbase = foo\npkgver = 1.0\npkgrel = 1\narch = x86_64\npkgdesc = Foo\nlicense = MIT\n";
    make_pkg(&staging, "foo-1.0-1-x86_64", pkginfo);

    let config = DbConfig::default();
    let report = validate_staging(&staging, &config, &BTreeSet::new(), &BTreeSet::new(), false)
        .unwrap();
    assert_eq!(report.valid.len(), 1);
    assert!(report.rejected.is_empty());

    // Build repo db from validated packages.
    let mut db = RepoDb::new();
    for p in &report.valid {
        db.add(extract_pkginfo(p).unwrap());
    }
    let db_path = dir.path().join("extra.db");
    dbscripts::write_repo_db(&db, &db_path).unwrap();
    let loaded = RepoDb::load(&db_path).unwrap();
    assert!(loaded.packages.contains_key("foo-1.0-1-x86_64"));
}

#[test]
fn test_rejects_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let staging = dir.path().join("extra");
    std::fs::create_dir_all(&staging).unwrap();
    let target = dir.path().join("real.pkg.tar.zst");
    std::fs::write(&target, b"not a pkg").unwrap();
    std::os::unix::fs::symlink(&target, staging.join("foo-1-1-x86_64.pkg.tar.zst")).unwrap();
    let report = validate_staging(&staging, &DbConfig::default(), &BTreeSet::new(), &BTreeSet::new(), false)
        .unwrap();
    assert_eq!(report.valid.len(), 0);
    assert_eq!(report.rejected.len(), 1);
    assert!(report.rejected[0].1.contains("symbolic link"));
}

#[test]
fn test_check_pkgfile_filename_mismatch() {
    let i = info("foo", "foo", "1.0", "1", "x86_64");
    let path = Path::new("/tmp/wrongname-9.9-9-x86_64.pkg.tar.zst");
    assert!(check_pkgfile(path, &i).is_err());
}

#[test]
fn test_license_check() {
    let mut i = info("foo", "foo", "1.0", "1", "x86_64");
    i.licenses = vec!["Proprietary".into()];
    assert!(check_license(&i, &DbConfig::default()).is_err());
    i.licenses = vec!["MIT".into()];
    assert!(check_license(&i, &DbConfig::default()).is_ok());
}

#[test]
fn test_check_not_in_repo() {
    let i = info("foo", "foo", "1.0", "1", "x86_64");
    let mut existing = BTreeSet::new();
    existing.insert("foo".to_string());
    assert!(check_not_in_repo(&i, &existing).is_err());
    existing.clear();
    assert!(check_not_in_repo(&i, &existing).is_ok());
}

#[test]
fn test_splitpkgs_grouping() {
    let infos = vec![
        info("foo", "foo", "1.0", "1", "x86_64"),
        info("foo-lib", "foo", "1.0", "1", "x86_64"),
        info("bar", "bar", "1.0", "1", "x86_64"),
    ];
    assert!(check_splitpkgs(&infos).is_ok());
}

#[test]
fn test_repo_add_remove_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("extra.db");
    repo_add(&db, &[info("a", "a", "1", "1", "x86_64"), info("b", "b", "1", "1", "any")]).unwrap();
    let loaded = RepoDb::load(&db).unwrap();
    assert_eq!(loaded.packages.len(), 2);
    repo_remove(&db, &["a".to_string()]).unwrap();
    let loaded = RepoDb::load(&db).unwrap();
    assert_eq!(loaded.packages.len(), 1);
    assert!(loaded.packages.contains_key("b-1-1-any"));
}
