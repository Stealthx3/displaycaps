// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-cli/src/archive.rs; the USB
// descriptors exemption is dropped and a round-trip test is added.
//! Capture tarballs: writing, and safe extraction into a fixture directory.

use std::fs;
use std::io;
use std::path::{Component, Path};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

/// Writes `dir` into `tarball` under the top-level folder `base`, keeping symlinks as symlinks.
pub fn write(dir: &Path, base: &str, tarball: &Path) -> io::Result<()> {
    let f = fs::File::create(tarball)?;
    let mut b = tar::Builder::new(GzEncoder::new(f, Compression::default()));
    b.follow_symlinks(false);
    b.mode(tar::HeaderMode::Deterministic);
    b.append_dir_all(base, dir)?;
    b.into_inner()?.finish()?;
    Ok(())
}

/// Extracts a capture tarball into `dest` (which must not exist), dropping the top-level folder.
/// Refuses absolute paths, `..`, hard links, device entries and symlinks that leave the tree.
pub fn extract(tarball: &Path, dest: &Path) -> io::Result<usize> {
    if dest.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} exists", dest.display()),
        ));
    }
    let mut ar = tar::Archive::new(GzDecoder::new(fs::File::open(tarball)?));
    let mut n = 0;
    for e in ar.entries()? {
        let mut e = e?;
        let path = e.path()?.into_owned();
        let mut comps = path.components();
        comps.next(); // top-level folder
        let rel: std::path::PathBuf = comps.collect();
        if rel.as_os_str().is_empty() {
            continue;
        }
        if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err(bad(format!("unsafe path {}", path.display())));
        }
        let out = dest.join(&rel);
        match e.header().entry_type() {
            tar::EntryType::Directory => fs::create_dir_all(&out)?,
            tar::EntryType::Regular => {
                if let Some(p) = out.parent() {
                    fs::create_dir_all(p)?;
                }
                e.unpack(&out)?;
                n += 1;
            }
            tar::EntryType::Symlink => {
                let target = e
                    .link_name()?
                    .ok_or_else(|| bad("symlink without target".into()))?
                    .into_owned();
                if target.is_absolute() {
                    return Err(bad(format!("absolute symlink {}", rel.display())));
                }
                // Resolve lexically against the link's directory; it must stay inside `dest`.
                let mut depth: i64 = rel.components().count() as i64 - 1;
                for c in target.components() {
                    match c {
                        Component::ParentDir => depth -= 1,
                        Component::Normal(_) => depth += 1,
                        Component::CurDir => {}
                        _ => return Err(bad(format!("odd symlink {}", rel.display()))),
                    }
                    if depth < 0 {
                        return Err(bad(format!(
                            "symlink leaves the capture: {}",
                            rel.display()
                        )));
                    }
                }
                if let Some(p) = out.parent() {
                    fs::create_dir_all(p)?;
                }
                std::os::unix::fs::symlink(&target, &out)?;
            }
            other => {
                return Err(bad(format!(
                    "unsupported entry {other:?} at {}",
                    rel.display()
                )));
            }
        }
    }
    Ok(n)
}

fn bad(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Scans every regular file under `dir` for addresses that should have been redacted.
pub fn check_redaction(dir: &Path) -> io::Result<Vec<String>> {
    let mut out = vec![];
    walk(dir, &mut |p| {
        if p.file_name().is_some_and(|n| n == "edid") {
            return;
        }
        if let Ok(b) = fs::read(p)
            && let Ok(t) = std::str::from_utf8(&b)
        {
            for l in crate::redact::leaks(t) {
                out.push(format!(
                    "{}: {l}",
                    p.strip_prefix(dir).unwrap_or(p).display()
                ));
            }
        }
    })?;
    Ok(out)
}

/// Re-applies the redaction rules to an extracted fixture (pseudo addresses are kept), e.g. after
/// a rule is tightened. Returns how many files changed.
pub fn redact_fixture(dir: &Path) -> io::Result<usize> {
    let r = crate::redact::Redactor::for_redacted_fixture();
    let mut changed = 0;
    let mut err = None;
    walk(dir, &mut |p| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "edid" || name == "manifest.json" {
            return;
        }
        let Ok(text) = fs::read_to_string(p) else {
            return;
        };
        let new = if name.ends_with(".json") {
            match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(mut v) => {
                    r.json(&mut v);
                    let mut s = serde_json::to_string_pretty(&v).unwrap_or_default();
                    s.push('\n');
                    s
                }
                Err(_) => r.text(&text),
            }
        } else {
            r.text(&text)
        };
        if new != text {
            match fs::write(p, new) {
                Ok(()) => changed += 1,
                Err(e) => err = Some(e),
            }
        }
    })?;
    err.map_or(Ok(changed), Err)
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) -> io::Result<()> {
    for e in fs::read_dir(dir)? {
        let e = e?;
        let ft = e.file_type()?;
        if ft.is_dir() {
            walk(&e.path(), f)?;
        } else if ft.is_file() {
            f(&e.path());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("displaycaps-cli-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn write_extract_round_trip_keeps_files_and_relative_symlinks() {
        let t = temp("archive");
        let src = t.join("src");
        fs::create_dir_all(src.join("root/sys")).unwrap();
        fs::write(src.join("manifest.json"), "{}").unwrap();
        fs::write(
            src.join("root/sys/status"),
            "connected
",
        )
        .unwrap();
        std::os::unix::fs::symlink("status", src.join("root/sys/link")).unwrap();
        let tarball = t.join("c.tar.gz");
        write(&src, "capture", &tarball).unwrap();
        let dest = t.join("out");
        assert_eq!(extract(&tarball, &dest).unwrap(), 2);
        assert_eq!(
            fs::read_to_string(dest.join("root/sys/status")).unwrap(),
            "connected
"
        );
        assert_eq!(
            fs::read_link(dest.join("root/sys/link")).unwrap(),
            std::path::Path::new("status")
        );
        // The destination must not exist.
        assert!(extract(&tarball, &dest).is_err());
        let _ = fs::remove_dir_all(&t);
    }

    #[test]
    fn extract_refuses_symlinks_that_leave_the_capture() {
        let t = temp("escape");
        let src = t.join("src");
        fs::create_dir_all(&src).unwrap();
        std::os::unix::fs::symlink("../../etc/passwd", src.join("evil")).unwrap();
        let tarball = t.join("c.tar.gz");
        write(&src, "capture", &tarball).unwrap();
        assert!(extract(&tarball, &t.join("out")).is_err());
        let _ = fs::remove_dir_all(&t);
    }

    #[test]
    fn redaction_check_and_re_redaction() {
        let t = temp("redact");
        fs::write(
            t.join("a.txt"),
            "host 198.51.100.4 mac 00:00:5e:00:53:13
",
        )
        .unwrap();
        fs::write(t.join("edid"), "198.51.100.4").unwrap();
        let leaks = check_redaction(&t).unwrap();
        assert_eq!(leaks.len(), 2, "{leaks:?}");
        assert!(leaks.iter().all(|l| l.starts_with("a.txt")));
        assert_eq!(redact_fixture(&t).unwrap(), 1);
        assert!(check_redaction(&t).unwrap().is_empty());
        let _ = fs::remove_dir_all(&t);
    }
}
