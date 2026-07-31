//! Workspace discovery and compatibility file writes.
//!
//! This is the single CLI-side boundary for repository discovery and ordinary
//! command output. Run publication uses mobench-artifacts' stronger transaction model.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub(crate) fn repo_root() -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("resolving repo root from current directory")?;
    if let Some(root) = find_repo_root(&cwd) {
        return Ok(root);
    }

    let compiled = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    if let Ok(path) = compiled.canonicalize() {
        if let Some(root) = find_repo_root(&path) {
            return Ok(root);
        }
        return Ok(path);
    }

    Ok(cwd)
}

pub(crate) fn find_repo_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|candidate| is_repo_root(candidate))
        .map(|root| root.to_path_buf())
}

pub(crate) fn is_repo_root(candidate: &Path) -> bool {
    candidate.join("bench-mobile").join("Cargo.toml").is_file()
        || candidate
            .join("crates")
            .join("sample-fns")
            .join("Cargo.toml")
            .is_file()
}

pub(crate) fn ensure_can_write(path: &Path, overwrite: bool) -> Result<()> {
    if path.exists() && !overwrite {
        bail!("refusing to overwrite existing file: {:?}", path);
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent directory {:?}", parent))?;
    }
    Ok(())
}

pub(crate) fn write_file(path: &Path, contents: &[u8]) -> Result<()> {
    write_file_no_follow(path, contents).with_context(|| format!("writing file {:?}", path))
}

#[cfg(unix)]
fn write_file_no_follow(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o666)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

#[cfg(not(unix))]
fn write_file_no_follow(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(std::io::Error::other(
                "refusing to follow a symlink output file",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_file_persists_and_replaces_regular_outputs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("report.txt");

        write_file(&path, b"first").expect("first write");
        write_file(&path, b"second").expect("replacement write");

        assert_eq!(fs::read(&path).expect("read output"), b"second");
    }

    #[cfg(unix)]
    #[test]
    fn write_file_rejects_a_symlink_leaf_without_mutating_its_target() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let outside = dir.path().join("outside.txt");
        let output = dir.path().join("report.txt");
        fs::write(&outside, b"untouched").expect("seed outside target");
        symlink(&outside, &output).expect("create output symlink");

        let error = write_file(&output, b"escaped").expect_err("reject symlink output");

        assert!(error.to_string().contains("writing file"));
        assert_eq!(
            fs::read(&outside).expect("read outside target"),
            b"untouched"
        );
    }
}
