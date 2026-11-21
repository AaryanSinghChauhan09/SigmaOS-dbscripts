//! `.PKGINFO` / `.BUILDINFO` extraction from `.pkg.tar.*` archives.
//!
//! Upstream shells out to `bsdtar -xOf pkg .PKGINFO`; this uses the `tar`,
//! `zstd`, and `flate2` crates to do the same in-process. `.PKGINFO` is a
//! flat `key = value` file; list-valued keys (depend, license, …) repeat.

use std::io::Read;
use std::path::Path;

use crate::DbError;

/// Parsed `.PKGINFO` metadata.
#[derive(Debug, Clone, Default)]
pub struct PkgInfo {
    /// pkgname
    pub pkgname: String,
    /// pkgbase
    pub pkgbase: String,
    /// pkgver (upstream version only, no pkgrel)
    pub pkgver: String,
    /// pkgrel
    pub pkgrel: String,
    /// epoch (0 when absent)
    pub epoch: u64,
    /// pkgdesc
    pub pkgdesc: String,
    /// url
    pub url: String,
    /// builddate (unix timestamp)
    pub builddate: i64,
    /// packager
    pub packager: String,
    /// size (compressed)
    pub size: i64,
    /// isize (installed)
    pub isize: i64,
    /// arch
    pub arch: String,
    /// license list
    pub licenses: Vec<String>,
    /// depend list (`name>=ver` specs)
    pub depends: Vec<String>,
    /// optdepend list
    pub optdepends: Vec<String>,
    /// makedepend list
    pub makedepends: Vec<String>,
    /// checkdepend list
    pub checkdepends: Vec<String>,
    /// conflict list
    pub conflicts: Vec<String>,
    /// replaces list
    pub replaces: Vec<String>,
    /// provides list (`name=version`)
    pub provides: Vec<String>,
    /// group list
    pub groups: Vec<String>,
}

impl PkgInfo {
    /// Full `epoch:pkgver-pkgrel` version string.
    pub fn full_version(&self) -> String {
        let epoch = if self.epoch != 0 {
            format!("{}:", self.epoch)
        } else {
            String::new()
        };
        format!("{}{}-{}", epoch, self.pkgver, self.pkgrel)
    }

    /// Package filename suffix: `<pkgname>-<full_version>-<arch>`.
    pub fn file_stem(&self) -> String {
        format!("{}-{}-{}", self.pkgname, self.full_version(), self.arch)
    }
}

/// Parsed `.BUILDINFO` (build environment record).
#[derive(Debug, Clone, Default)]
pub struct BuildInfo {
    /// buildenv entries (key = value lines from .BUILDINFO).
    pub fields: Vec<(String, String)>,
    /// installed package list (`pkgname-pkgver-pkgrel-arch` lines).
    pub installed: Vec<String>,
}

impl BuildInfo {
    /// Look up a field value.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// Extract and parse `.PKGINFO` from a `.pkg.tar.*` file.
pub fn extract_pkginfo(path: &Path) -> Result<PkgInfo, DbError> {
    let text = extract_file(path, ".PKGINFO")?;
    parse_pkginfo(&text)
}

/// Extract and parse `.BUILDINFO` from a `.pkg.tar.*` file.
pub fn extract_buildinfo(path: &Path) -> Result<BuildInfo, DbError> {
    match extract_file(path, ".BUILDINFO") {
        Ok(text) => Ok(parse_buildinfo(&text)),
        Err(DbError::InvalidPkgFile(_)) => Ok(BuildInfo::default()),
        Err(e) => Err(e),
    }
}

/// Parse `.PKGINFO` text into [`PkgInfo`].
pub fn parse_pkginfo(text: &str) -> Result<PkgInfo, DbError> {
    let mut info = PkgInfo::default();
    let mut cur_key: Option<String> = None;
    let mut cur_vals: Vec<String> = Vec::new();

    let mut flush = |info: &mut PkgInfo, key: &str, vals: &[String]| {
        let first = vals.first().cloned().unwrap_or_default();
        match key {
            "pkgname" => info.pkgname = first,
            "pkgbase" => info.pkgbase = first,
            "pkgver" => info.pkgver = first,
            "pkgrel" => info.pkgrel = first,
            "epoch" => info.epoch = first.parse().unwrap_or(0),
            "pkgdesc" => info.pkgdesc = first,
            "url" => info.url = first,
            "builddate" => info.builddate = first.parse().unwrap_or(0),
            "packager" => info.packager = first,
            "size" => info.size = first.parse().unwrap_or(0),
            "isize" => info.isize = first.parse().unwrap_or(0),
            "arch" => info.arch = first,
            "license" => info.licenses = vals.to_vec(),
            "depend" => info.depends = vals.to_vec(),
            "optdepend" => info.optdepends = vals.to_vec(),
            "makedepend" => info.makedepends = vals.to_vec(),
            "checkdepend" => info.checkdepends = vals.to_vec(),
            "conflict" => info.conflicts = vals.to_vec(),
            "replaces" => info.replaces = vals.to_vec(),
            "provides" => info.provides = vals.to_vec(),
            "group" => info.groups = vals.to_vec(),
            _ => {}
        }
    };

    for line in text.lines() {
        if let Some((k, v)) = line.split_once(" = ") {
            if let Some(prev) = cur_key.take() {
                flush(&mut info, &prev, &cur_vals);
                cur_vals.clear();
            }
            cur_key = Some(k.to_string());
            cur_vals.push(v.to_string());
        } else if cur_key.is_some() {
            // Continuation lines belong to the current key's value list
            // (.PKGINFO repeats `key = value` instead, but be tolerant).
            cur_vals.push(line.trim().to_string());
        }
    }
    if let Some(prev) = cur_key.take() {
        flush(&mut info, &prev, &cur_vals);
    }

    if info.pkgname.is_empty() || info.pkgver.is_empty() {
        return Err(DbError::BadPkgInfo("missing pkgname/pkgver".into()));
    }
    Ok(info)
}

/// Parse `.BUILDINFO` text. Format: `%FUNCTION%` style headers are not used;
/// it's `key = value` lines plus a final list of installed packages after
/// the `installed` key.
pub fn parse_buildinfo(text: &str) -> BuildInfo {
    let mut bi = BuildInfo::default();
    let mut in_installed = false;
    for line in text.lines() {
        let line = line.trim_end();
        if line == "installed" {
            in_installed = true;
            continue;
        }
        if in_installed {
            if !line.trim().is_empty() {
                bi.installed.push(line.trim().to_string());
            }
            continue;
        }
        if let Some((k, v)) = line.split_once(" = ") {
            bi.fields.push((k.to_string(), v.to_string()));
        }
    }
    bi
}

/// Decompress package archive bytes based on extension/magic.
fn decompress(data: &[u8], path: &Path) -> Result<Vec<u8>, DbError> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let out = if name.ends_with(".zst")
        || data.starts_with(&[0x28, 0xB5, 0x2F, 0xFD])
    {
        zstd::stream::decode_all(data)
            .map_err(|e| DbError::InvalidPkgFile(format!("zstd: {e}")))?
            .to_vec()
    } else if name.ends_with(".xz") {
        // xz decompression via xz2 not a dep; try `xz` crate-free fallback:
        // most Arch pkgs are .zst, so report unsupported for .xz.
        return Err(DbError::InvalidPkgFile(format!(
            "{}: xz compression not supported (use .pkg.tar.zst)",
            path.display()
        )));
    } else if name.ends_with(".gz") || data.starts_with(&[0x1F, 0x8B]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(data)
            .read_to_end(&mut out)
            .map_err(|e| DbError::InvalidPkgFile(format!("gzip: {e}")))?;
        out
    } else {
        data.to_vec()
    };
    Ok(out)
}

/// Extract a named file's text content from a `.pkg.tar.*` archive.
fn extract_file(path: &Path, wanted: &str) -> Result<String, DbError> {
    let data = std::fs::read(path)
        .map_err(|e| DbError::InvalidPkgFile(format!("{}: {e}", path.display())))?;
    let tar_bytes = decompress(&data, path)?;
    let mut archive = tar::Archive::new(tar_bytes.as_slice());
    for entry in archive
        .entries()
        .map_err(|e| DbError::Tar(e.to_string()))?
    {
        let mut entry = entry.map_err(|e| DbError::Tar(e.to_string()))?;
        let entry_path = entry
            .path()
            .map_err(|e| DbError::Tar(e.to_string()))?
            .to_path_buf();
        // Entries may be "./.PKGINFO" or ".PKGINFO".
        let normalized = entry_path.to_string_lossy().trim_start_matches("./").to_string();
        if normalized == wanted {
            let mut text = String::new();
            entry
                .read_to_string(&mut text)
                .map_err(|e| DbError::Tar(e.to_string()))?;
            return Ok(text);
        }
    }
    Err(DbError::InvalidPkgFile(format!(
        "{}: {wanted} not found",
        path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_pkg(dir: &Path, name: &str, pkginfo: &str) -> std::path::PathBuf {
        let mut tar_data = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_data);
            let mut h = tar::Header::new_gnu();
            h.set_size(pkginfo.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, ".PKGINFO", pkginfo.as_bytes()).unwrap();
            let bi = "builddate = 1700000000\npackager = TU\ninstalled\nfoo-1-1-x86_64\n";
            let mut h2 = tar::Header::new_gnu();
            h2.set_size(bi.len() as u64);
            h2.set_mode(0o644);
            h2.set_cksum();
            b.append_data(&mut h2, ".BUILDINFO", bi.as_bytes()).unwrap();
            b.finish().unwrap();
        }
        // zstd-compress
        let compressed = zstd::stream::encode_all(tar_data.as_slice(), 3).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, compressed).unwrap();
        path
    }

    #[test]
    fn test_extract_pkginfo_zst() {
        let dir = tempfile::tempdir().unwrap();
        let pkginfo = "pkgname = foo\npkgbase = foo\npkgver = 1.2.3\npkgrel = 4\nepoch = 2\npkgdesc = Test pkg\nurl = https://example.com\nbuilddate = 1700000000\npackager = TU\nsize = 1000\nisize = 2000\narch = x86_64\nlicense = MIT\ndepend = bar>=1.0\nprovides = foo-lib=1.2.3\n";
        let path = make_pkg(dir.path(), "foo-2:1.2.3-4-x86_64.pkg.tar.zst", pkginfo);
        let info = extract_pkginfo(&path).unwrap();
        assert_eq!(info.pkgname, "foo");
        assert_eq!(info.epoch, 2);
        assert_eq!(info.full_version(), "2:1.2.3-4");
        assert_eq!(info.depends, vec!["bar>=1.0"]);
        assert_eq!(info.provides, vec!["foo-lib=1.2.3"]);
        assert_eq!(info.licenses, vec!["MIT"]);
    }

    #[test]
    fn test_extract_buildinfo() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_pkg(
            dir.path(),
            "foo-1-1-x86_64.pkg.tar.zst",
            "pkgname = foo\npkgver = 1\npkgrel = 1\narch = x86_64\n",
        );
        let bi = extract_buildinfo(&path).unwrap();
        assert_eq!(bi.get("packager"), Some("TU"));
        assert_eq!(bi.installed, vec!["foo-1-1-x86_64"]);
    }

    #[test]
    fn test_missing_pkginfo_errors() {
        let dir = tempfile::tempdir().unwrap();
        // tar without .PKGINFO
        let mut tar_data = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_data);
            b.finish().unwrap();
        }
        let compressed = zstd::stream::encode_all(tar_data.as_slice(), 3).unwrap();
        let path = dir.path().join("empty.pkg.tar.zst");
        std::fs::write(&path, compressed).unwrap();
        assert!(extract_pkginfo(&path).is_err());
    }
}
