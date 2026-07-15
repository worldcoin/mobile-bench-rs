//! Contained artifact roots and atomic run workspaces.
//!
//! [`ApprovedRoot`] validates paths immediately before use. It prevents
//! traversal through pre-existing symlinks, but it does not yet provide the
//! descriptor-relative operations needed to close hostile concurrent-swap
//! races on every supported platform.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use thiserror::Error;

/// A validated artifact identifier that is safe to use as one path component.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtifactId(String);

impl ArtifactId {
    /// Validate and retain a single artifact path component.
    pub fn new(value: impl Into<String>) -> Result<Self, ArtifactPathError> {
        let value = value.into();
        let path = Path::new(&value);
        let is_one_normal_component = {
            let mut components = path.components();
            matches!(components.next(), Some(std::path::Component::Normal(_)))
                && components.next().is_none()
        };
        if value.is_empty()
            || value.contains('/')
            || value.contains('\\')
            || path.is_absolute()
            || !is_one_normal_component
        {
            return Err(ArtifactPathError::InvalidArtifactId { value });
        }
        Ok(Self(value))
    }

    /// Return the validated component.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ArtifactId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

const STAGING_DIRECTORY: &str = ".mobench-staging";
const LATEST_STAGING_DIRECTORY: &str = ".mobench-latest-staging";
const LATEST_LOCK_FILE: &str = ".mobench-latest.lock";
const LATEST_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const LATEST_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(5);
const MAX_ALLOCATION_ATTEMPTS: usize = 1_024;
static WORKSPACE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A unique private directory for building one complete run before publication.
#[derive(Debug)]
pub struct RunWorkspace {
    root: ApprovedRoot,
    staging_path: Option<PathBuf>,
    published_path: PathBuf,
}

/// A completed run directory that has been atomically renamed into visibility.
#[derive(Debug, Clone)]
pub struct PublishedRun {
    root: ApprovedRoot,
    path: PathBuf,
}

#[derive(Debug)]
struct LatestUpdateLock {
    file: File,
}

impl Drop for LatestUpdateLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// One published artifact to copy to a stable convenience path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestArtifact {
    source: ArtifactId,
    destination: ArtifactId,
}

impl LatestArtifact {
    /// Refresh a stable path with a published artifact of the same name.
    pub fn same(id: ArtifactId) -> Self {
        Self {
            source: id.clone(),
            destination: id,
        }
    }

    /// Refresh a stable destination from a differently named published file.
    pub fn new(source: ArtifactId, destination: ArtifactId) -> Self {
        Self {
            source,
            destination,
        }
    }
}

impl RunWorkspace {
    /// Allocate a unique staging directory and reserve a collision-resistant
    /// publication path derived from the validated logical run ID.
    pub fn allocate(
        root: impl AsRef<Path>,
        logical_id: &ArtifactId,
    ) -> Result<Self, ArtifactPathError> {
        let root_path = root.as_ref();
        fs::create_dir_all(root_path).map_err(|source| ArtifactPathError::Io {
            operation: "create artifact root",
            path: root_path.to_path_buf(),
            source,
        })?;
        let root = ApprovedRoot::existing(root_path)?;
        let staging_root = root.prepare_dir(STAGING_DIRECTORY)?;

        for _ in 0..MAX_ALLOCATION_ATTEMPTS {
            let nonce = workspace_nonce();
            let staging_path = staging_root.join(&nonce);
            let published_path = root.path().join(format!("{logical_id}--{nonce}"));
            if fs::symlink_metadata(&published_path).is_ok() {
                continue;
            }

            match fs::create_dir(&staging_path) {
                Ok(()) => {
                    if fs::symlink_metadata(&published_path).is_ok() {
                        let _ = fs::remove_dir(&staging_path);
                        continue;
                    }
                    return Ok(Self {
                        root,
                        staging_path: Some(staging_path),
                        published_path,
                    });
                }
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(ArtifactPathError::Io {
                        operation: "create run staging directory",
                        path: staging_path,
                        source,
                    });
                }
            }
        }

        Err(ArtifactPathError::WorkspaceAllocationExhausted {
            logical_id: logical_id.clone(),
        })
    }

    /// Return the private directory where run outputs must be written.
    pub fn staging_path(&self) -> &Path {
        self.staging_path
            .as_deref()
            .expect("unpublished workspace retains its staging path")
    }

    /// Return the unique path where the completed run will be published.
    pub fn published_path(&self) -> &Path {
        &self.published_path
    }

    /// Return the approved output root shared by published and latest artifacts.
    pub fn root(&self) -> &ApprovedRoot {
        &self.root
    }

    /// Publish the complete staging directory without replacing any existing
    /// file, directory, or symbolic link at the destination.
    pub fn publish(
        mut self,
        required_files: &[ArtifactId],
    ) -> Result<PublishedRun, ArtifactPathError> {
        let staging_path = self
            .staging_path
            .as_ref()
            .expect("unpublished workspace retains its staging path")
            .clone();
        let published_path = self.published_path.clone();
        match fs::symlink_metadata(&staging_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ArtifactPathError::SymlinkComponent { path: staging_path });
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(ArtifactPathError::DirectoryComponentNotDirectory {
                    path: staging_path,
                });
            }
            Ok(_) => {}
            Err(source) => {
                return Err(ArtifactPathError::Io {
                    operation: "inspect run staging directory",
                    path: staging_path,
                    source,
                });
            }
        }
        let staging_relative = staging_path.strip_prefix(self.root.path()).map_err(|_| {
            ArtifactPathError::InvalidRelativePath {
                path: staging_path.clone(),
            }
        })?;
        self.root.prepare_dir(staging_relative)?;

        let staging_root = ApprovedRoot::existing(&staging_path)?;
        for required_file in required_files {
            let required_path = staging_root.prepare_file(required_file.as_str())?;
            match fs::symlink_metadata(&required_path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(ArtifactPathError::SymlinkComponent {
                        path: required_path,
                    });
                }
                Ok(metadata) if !metadata.is_file() => {
                    return Err(ArtifactPathError::FileDestinationNotFile {
                        path: required_path,
                    });
                }
                Ok(_) => {}
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                    return Err(ArtifactPathError::RequiredArtifactMissing {
                        path: required_path,
                    });
                }
                Err(source) => {
                    return Err(ArtifactPathError::Io {
                        operation: "inspect required staged artifact",
                        path: required_path,
                        source,
                    });
                }
            }
        }

        match fs::symlink_metadata(&published_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ArtifactPathError::SymlinkComponent {
                    path: published_path,
                });
            }
            Ok(_) => {
                return Err(ArtifactPathError::PublicationDestinationExists {
                    path: published_path,
                });
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ArtifactPathError::Io {
                    operation: "inspect run publication destination",
                    path: published_path,
                    source,
                });
            }
        }

        fs::rename(&staging_path, &published_path).map_err(|source| ArtifactPathError::Io {
            operation: "publish completed run",
            path: published_path.clone(),
            source,
        })?;
        self.staging_path = None;

        Ok(PublishedRun {
            root: self.root.clone(),
            path: published_path,
        })
    }
}

impl Drop for RunWorkspace {
    fn drop(&mut self) {
        if let Some(staging_path) = self.staging_path.as_ref() {
            let _ = fs::remove_dir_all(staging_path);
        }
    }
}

impl PublishedRun {
    /// Return the unique published run directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the approved output root containing this run.
    pub fn root(&self) -> &ApprovedRoot {
        &self.root
    }

    /// Refresh stable convenience files from this completed published run.
    pub fn refresh_latest(&self, artifacts: &[LatestArtifact]) -> Result<(), ArtifactPathError> {
        let published_root = ApprovedRoot::existing(&self.path)?;
        let mut prepared = Vec::with_capacity(artifacts.len());
        for artifact in artifacts {
            if prepared
                .iter()
                .any(|(_, _, destination): &(PathBuf, PathBuf, ArtifactId)| {
                    destination == &artifact.destination
                })
            {
                return Err(ArtifactPathError::DuplicateLatestDestination {
                    id: artifact.destination.clone(),
                });
            }
            let source = published_root.prepare_file(artifact.source.as_str())?;
            match fs::symlink_metadata(&source) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(ArtifactPathError::SymlinkComponent { path: source });
                }
                Ok(metadata) if !metadata.is_file() => {
                    return Err(ArtifactPathError::FileDestinationNotFile { path: source });
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(ArtifactPathError::RequiredArtifactMissing { path: source });
                }
                Err(source_error) => {
                    return Err(ArtifactPathError::Io {
                        operation: "inspect published latest source",
                        path: source,
                        source: source_error,
                    });
                }
            }
            let destination = self.root.prepare_file(artifact.destination.as_str())?;
            prepared.push((source, destination, artifact.destination.clone()));
        }

        // The individual renames below are atomic, but the set is not. Serialize
        // writers so two complete runs cannot leave a mixed final alias set.
        // Readers that ignore this lock may still observe a transition between
        // aliases; a crash-proof multi-file pointer swap is a later hardening step.
        let _latest_update_lock = acquire_latest_update_lock(&self.root)?;
        let transaction_path = allocate_latest_transaction(&self.root)?;
        let update_result = (|| {
            let transaction_root = ApprovedRoot::existing(&transaction_path)?;
            let new_root = transaction_root.prepare_dir("new")?;
            let backup_root = transaction_root.prepare_dir("backup")?;

            for (source, _, destination_id) in &prepared {
                let staged = new_root.join(destination_id.as_str());
                fs::copy(source, &staged).map_err(|source| ArtifactPathError::Io {
                    operation: "stage latest artifact",
                    path: staged,
                    source,
                })?;
            }

            for (_, destination, _) in &prepared {
                self.root
                    .prepare_file(destination.strip_prefix(self.root.path()).map_err(|_| {
                        ArtifactPathError::InvalidRelativePath {
                            path: destination.clone(),
                        }
                    })?)?;
            }

            let mut backups = Vec::new();
            for (_, destination, destination_id) in &prepared {
                match fs::symlink_metadata(destination) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        let error = ArtifactPathError::SymlinkComponent {
                            path: destination.clone(),
                        };
                        return Err(latest_update_failure(error, &[], &backups));
                    }
                    Ok(metadata) if !metadata.is_file() => {
                        let error = ArtifactPathError::FileDestinationNotFile {
                            path: destination.clone(),
                        };
                        return Err(latest_update_failure(error, &[], &backups));
                    }
                    Ok(_) => {
                        let backup = backup_root.join(destination_id.as_str());
                        if let Err(source) = fs::rename(destination, &backup) {
                            let error = ArtifactPathError::Io {
                                operation: "back up latest artifact",
                                path: destination.clone(),
                                source,
                            };
                            return Err(latest_update_failure(error, &[], &backups));
                        }
                        backups.push((destination.clone(), backup));
                    }
                    Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                    Err(source) => {
                        let error = ArtifactPathError::Io {
                            operation: "inspect latest artifact before replacement",
                            path: destination.clone(),
                            source,
                        };
                        return Err(latest_update_failure(error, &[], &backups));
                    }
                }
            }

            let mut installed = Vec::new();
            for (_, destination, destination_id) in &prepared {
                let staged = new_root.join(destination_id.as_str());
                if let Err(source) = fs::rename(&staged, destination) {
                    let error = ArtifactPathError::Io {
                        operation: "install latest artifact",
                        path: destination.clone(),
                        source,
                    };
                    return Err(latest_update_failure(error, &installed, &backups));
                }
                installed.push(destination.clone());
            }
            Ok(())
        })();
        let _ = fs::remove_dir_all(&transaction_path);
        update_result
    }
}

fn acquire_latest_update_lock(root: &ApprovedRoot) -> Result<LatestUpdateLock, ArtifactPathError> {
    let path = root.prepare_file(LATEST_LOCK_FILE)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| ArtifactPathError::Io {
            operation: "open latest-artifact lock",
            path: path.clone(),
            source,
        })?;
    let started = Instant::now();

    loop {
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(LatestUpdateLock { file }),
            Err(source)
                if source.kind() == std::io::ErrorKind::WouldBlock
                    && started.elapsed() < LATEST_LOCK_TIMEOUT =>
            {
                std::thread::sleep(LATEST_LOCK_POLL_INTERVAL);
            }
            Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {
                return Err(ArtifactPathError::LatestLockTimeout { path });
            }
            Err(source) => {
                return Err(ArtifactPathError::Io {
                    operation: "lock latest artifacts",
                    path,
                    source,
                });
            }
        }
    }
}

fn allocate_latest_transaction(root: &ApprovedRoot) -> Result<PathBuf, ArtifactPathError> {
    let staging_root = root.prepare_dir(LATEST_STAGING_DIRECTORY)?;
    for _ in 0..MAX_ALLOCATION_ATTEMPTS {
        let path = staging_root.join(workspace_nonce());
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(ArtifactPathError::Io {
                    operation: "create latest staging transaction",
                    path,
                    source,
                });
            }
        }
    }
    Err(ArtifactPathError::LatestTransactionAllocationExhausted)
}

fn remove_installed_latest(installed: &[PathBuf]) -> Result<(), ArtifactPathError> {
    let mut first_error = None;
    for destination in installed.iter().rev() {
        match fs::remove_file(destination) {
            Ok(()) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                first_error.get_or_insert_with(|| ArtifactPathError::Io {
                    operation: "roll back installed latest artifact",
                    path: destination.clone(),
                    source,
                });
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn restore_latest_backups(backups: &[(PathBuf, PathBuf)]) -> Result<(), ArtifactPathError> {
    let mut first_error = None;
    for (destination, backup) in backups.iter().rev() {
        if let Err(source) = fs::rename(backup, destination) {
            first_error.get_or_insert_with(|| ArtifactPathError::Io {
                operation: "restore previous latest artifact",
                path: destination.clone(),
                source,
            });
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn latest_update_failure(
    original: ArtifactPathError,
    installed: &[PathBuf],
    backups: &[(PathBuf, PathBuf)],
) -> ArtifactPathError {
    let remove_error = remove_installed_latest(installed).err();
    let restore_error = restore_latest_backups(backups).err();
    let rollback = remove_error.or(restore_error);

    match rollback {
        Some(rollback) => ArtifactPathError::LatestRollback {
            original: Box::new(original),
            rollback: Box::new(rollback),
        },
        None => original,
    }
}

fn workspace_nonce() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = WORKSPACE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{timestamp:x}-{:x}-{sequence:x}", std::process::id())
}

/// An existing directory approved as the boundary for artifact writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedRoot {
    path: PathBuf,
}

impl ApprovedRoot {
    /// Approve an existing, non-symlink directory and retain its canonical path.
    pub fn existing(path: impl AsRef<Path>) -> Result<Self, ArtifactPathError> {
        let path = path.as_ref();
        let metadata = fs::symlink_metadata(path).map_err(|source| ArtifactPathError::Io {
            operation: "inspect approved root",
            path: path.to_path_buf(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ArtifactPathError::SymlinkComponent {
                path: path.to_path_buf(),
            });
        }
        if !metadata.is_dir() {
            return Err(ArtifactPathError::RootNotDirectory(path.to_path_buf()));
        }

        let path = fs::canonicalize(path).map_err(|source| ArtifactPathError::Io {
            operation: "canonicalize approved root",
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self { path })
    }

    /// Return the canonical approved root.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Resolve a relative directory beneath this root without creating it.
    ///
    /// Existing components must be real directories, never symbolic links.
    /// Once a missing component is reached there cannot be a deeper existing
    /// component, so the remaining validated path can be projected safely.
    pub fn project_dir(&self, relative: impl AsRef<Path>) -> Result<PathBuf, ArtifactPathError> {
        let relative = relative.as_ref();
        validate_relative_path(relative)?;

        let mut current = self.path.clone();
        for component in relative.components() {
            let std::path::Component::Normal(name) = component else {
                unreachable!("relative path was validated before directory projection")
            };
            current.push(name);

            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(ArtifactPathError::SymlinkComponent {
                        path: current.clone(),
                    });
                }
                Ok(metadata) if !metadata.is_dir() => {
                    return Err(ArtifactPathError::DirectoryComponentNotDirectory {
                        path: current.clone(),
                    });
                }
                Ok(_) => {}
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(self.path.join(relative));
                }
                Err(source) => {
                    return Err(ArtifactPathError::Io {
                        operation: "inspect projected artifact directory",
                        path: current,
                        source,
                    });
                }
            }
        }

        Ok(current)
    }

    /// Prepare a relative directory, rejecting traversal and symlink components.
    pub fn prepare_dir(&self, relative: impl AsRef<Path>) -> Result<PathBuf, ArtifactPathError> {
        let relative = relative.as_ref();
        validate_relative_path(relative)?;
        prepare_directory_components(&self.path, relative)
    }

    /// Prepare a relative file's parent directories and validate its destination.
    ///
    /// This does not create or truncate the file. An existing destination must
    /// be a regular file and must not be a symlink.
    pub fn prepare_file(&self, relative: impl AsRef<Path>) -> Result<PathBuf, ArtifactPathError> {
        let relative = relative.as_ref();
        validate_relative_path(relative)?;
        if let Some(parent) = relative
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            prepare_directory_components(&self.path, parent)?;
        }
        let path = self.path.join(relative);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(ArtifactPathError::SymlinkComponent { path })
            }
            Ok(metadata) if !metadata.is_file() => {
                Err(ArtifactPathError::FileDestinationNotFile { path })
            }
            Ok(_) => Ok(path),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(path),
            Err(source) => Err(ArtifactPathError::Io {
                operation: "inspect artifact file",
                path,
                source,
            }),
        }
    }
}

/// Failure to approve a root or safely prepare one of its descendants.
#[derive(Debug, Error)]
pub enum ArtifactPathError {
    #[error("artifact identifier must be one non-empty relative path component: {value:?}")]
    InvalidArtifactId { value: String },
    #[error("could not allocate a unique workspace for logical run ID {logical_id}")]
    WorkspaceAllocationExhausted { logical_id: ArtifactId },
    #[error("artifact publication destination already exists: {path}")]
    PublicationDestinationExists { path: PathBuf },
    #[error("required staged artifact is missing: {path}")]
    RequiredArtifactMissing { path: PathBuf },
    #[error("latest artifact destination was requested more than once: {id}")]
    DuplicateLatestDestination { id: ArtifactId },
    #[error("could not allocate a unique latest-artifact transaction")]
    LatestTransactionAllocationExhausted,
    #[error("timed out waiting for the latest-artifact update lock at {path}")]
    LatestLockTimeout { path: PathBuf },
    #[error("latest-artifact update failed ({original}) and rollback was incomplete ({rollback})")]
    LatestRollback {
        original: Box<ArtifactPathError>,
        rollback: Box<ArtifactPathError>,
    },
    #[error("approved artifact root is not a directory: {0}")]
    RootNotDirectory(PathBuf),
    #[error(
        "artifact child path must be a non-empty relative path without parent traversal: {path}"
    )]
    InvalidRelativePath { path: PathBuf },
    #[error("artifact path component is a symbolic link: {path}")]
    SymlinkComponent { path: PathBuf },
    #[error("artifact directory component is not a directory: {path}")]
    DirectoryComponentNotDirectory { path: PathBuf },
    #[error("artifact file destination is not a regular file: {path}")]
    FileDestinationNotFile { path: PathBuf },
    #[error("failed to {operation} at {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn validate_relative_path(path: &Path) -> Result<(), ArtifactPathError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(ArtifactPathError::InvalidRelativePath {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn prepare_directory_components(
    root: &Path,
    relative: &Path,
) -> Result<PathBuf, ArtifactPathError> {
    let mut current = root.to_path_buf();

    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            unreachable!("relative path was validated before directory preparation")
        };
        current.push(name);

        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ArtifactPathError::SymlinkComponent {
                    path: current.clone(),
                });
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(ArtifactPathError::DirectoryComponentNotDirectory {
                    path: current.clone(),
                });
            }
            Ok(_) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                match fs::create_dir(&current) {
                    Ok(()) => {}
                    Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                        match fs::symlink_metadata(&current) {
                            Ok(metadata) if metadata.file_type().is_symlink() => {
                                return Err(ArtifactPathError::SymlinkComponent {
                                    path: current.clone(),
                                });
                            }
                            Ok(metadata) if !metadata.is_dir() => {
                                return Err(ArtifactPathError::DirectoryComponentNotDirectory {
                                    path: current.clone(),
                                });
                            }
                            Ok(_) => {}
                            Err(source) => {
                                return Err(ArtifactPathError::Io {
                                    operation: "inspect raced artifact directory",
                                    path: current.clone(),
                                    source,
                                });
                            }
                        }
                    }
                    Err(source) => {
                        return Err(ArtifactPathError::Io {
                            operation: "create artifact directory",
                            path: current.clone(),
                            source,
                        });
                    }
                }
            }
            Err(source) => {
                return Err(ArtifactPathError::Io {
                    operation: "inspect artifact directory",
                    path: current.clone(),
                    source,
                });
            }
        }
    }

    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_id_rejects_an_empty_component() {
        assert!(matches!(
            ArtifactId::new(""),
            Err(ArtifactPathError::InvalidArtifactId { .. })
        ));
    }

    #[test]
    fn artifact_id_rejects_values_that_are_not_one_normal_path_component() {
        for invalid in [
            ".",
            "..",
            "/absolute",
            "nested/child",
            r"nested\child",
            r"C:\absolute",
        ] {
            assert!(
                matches!(
                    ArtifactId::new(invalid),
                    Err(ArtifactPathError::InvalidArtifactId { .. })
                ),
                "accepted invalid artifact ID {invalid:?}"
            );
        }
    }

    #[test]
    fn repeated_run_workspace_allocations_are_isolated() {
        let temp = tempfile::tempdir().expect("tempdir");
        let logical_id = ArtifactId::new("android-sample").expect("valid logical ID");

        let first = RunWorkspace::allocate(temp.path(), &logical_id).expect("first workspace");
        let second = RunWorkspace::allocate(temp.path(), &logical_id).expect("second workspace");

        assert_ne!(first.staging_path(), second.staging_path());
        assert_ne!(first.published_path(), second.published_path());
        assert!(first.staging_path().is_dir());
        assert!(second.staging_path().is_dir());
        assert!(!first.published_path().exists());
        assert!(!second.published_path().exists());
    }

    #[test]
    fn concurrent_run_workspace_allocations_are_isolated() {
        use std::collections::BTreeSet;
        use std::sync::{Arc, Barrier};

        let temp = tempfile::tempdir().expect("tempdir");
        let root = Arc::new(temp.path().to_path_buf());
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let root = Arc::clone(&root);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let logical_id = ArtifactId::new("android-sample").expect("valid logical ID");
                    barrier.wait();
                    let workspace =
                        RunWorkspace::allocate(root.as_ref(), &logical_id).expect("workspace");
                    (
                        workspace.staging_path().to_path_buf(),
                        workspace.published_path().to_path_buf(),
                    )
                })
            })
            .collect();

        let paths: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().expect("allocation thread"))
            .collect();
        let staging_paths: BTreeSet<_> = paths.iter().map(|(path, _)| path).collect();
        let published_paths: BTreeSet<_> = paths.iter().map(|(_, path)| path).collect();

        assert_eq!(staging_paths.len(), paths.len());
        assert_eq!(published_paths.len(), paths.len());
    }

    #[test]
    fn repeated_published_runs_do_not_inherit_stale_artifacts() {
        let temp = tempfile::tempdir().expect("tempdir");
        let logical_id = ArtifactId::new("android-sample").expect("valid logical ID");
        let required = [
            ArtifactId::new("profile.json").expect("valid profile ID"),
            ArtifactId::new("summary.md").expect("valid summary ID"),
        ];
        let first = RunWorkspace::allocate(temp.path(), &logical_id).expect("first workspace");
        fs::write(first.staging_path().join("profile.json"), "first profile")
            .expect("write first profile");
        fs::write(first.staging_path().join("summary.md"), "first summary")
            .expect("write first summary");
        fs::write(first.staging_path().join("stale.data"), "stale").expect("write stale artifact");
        let first = first.publish(&required).expect("publish first run");

        let second = RunWorkspace::allocate(temp.path(), &logical_id).expect("second workspace");
        fs::write(second.staging_path().join("profile.json"), "second profile")
            .expect("write second profile");
        fs::write(second.staging_path().join("summary.md"), "second summary")
            .expect("write second summary");
        let second = second.publish(&required).expect("publish second run");

        assert_ne!(first.path(), second.path());
        assert!(first.path().join("stale.data").is_file());
        assert!(!second.path().join("stale.data").exists());
    }

    #[cfg(unix)]
    #[test]
    fn run_workspace_rejects_a_symlink_publication_collision() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        let logical_id = ArtifactId::new("android-sample").expect("valid logical ID");
        let workspace =
            RunWorkspace::allocate(temp.path(), &logical_id).expect("allocate workspace");
        fs::write(workspace.staging_path().join("profile.json"), "complete")
            .expect("write staged manifest");
        symlink(outside.path(), workspace.published_path()).expect("create collision symlink");

        let required = [ArtifactId::new("profile.json").expect("valid required file")];
        let error = workspace
            .publish(&required)
            .expect_err("reject symlink collision");

        assert!(matches!(error, ArtifactPathError::SymlinkComponent { .. }));
        assert!(outside.path().is_dir());
        assert!(!outside.path().join("profile.json").exists());
    }

    #[test]
    fn incomplete_run_workspace_does_not_replace_latest_artifacts() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("profile.json"), "old profile").expect("seed latest profile");
        fs::write(temp.path().join("summary.md"), "old summary").expect("seed latest summary");
        let logical_id = ArtifactId::new("android-sample").expect("valid logical ID");
        let workspace =
            RunWorkspace::allocate(temp.path(), &logical_id).expect("allocate workspace");
        let published_path = workspace.published_path().to_path_buf();
        fs::write(workspace.staging_path().join("profile.json"), "new profile")
            .expect("write staged profile");
        let required = [
            ArtifactId::new("profile.json").expect("valid profile ID"),
            ArtifactId::new("summary.md").expect("valid summary ID"),
        ];

        let error = workspace
            .publish(&required)
            .expect_err("reject incomplete run");

        assert!(matches!(
            error,
            ArtifactPathError::RequiredArtifactMissing { .. }
        ));
        assert!(!published_path.exists());
        assert_eq!(
            fs::read_to_string(temp.path().join("profile.json")).expect("read latest profile"),
            "old profile"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("summary.md")).expect("read latest summary"),
            "old summary"
        );
    }

    #[test]
    fn published_run_refreshes_stable_latest_artifacts() {
        let temp = tempfile::tempdir().expect("tempdir");
        let logical_id = ArtifactId::new("android-sample").expect("valid logical ID");
        let workspace =
            RunWorkspace::allocate(temp.path(), &logical_id).expect("allocate workspace");
        fs::write(workspace.staging_path().join("profile.json"), "new profile")
            .expect("write staged profile");
        fs::write(workspace.staging_path().join("summary.md"), "new summary")
            .expect("write staged summary");
        let required = [
            ArtifactId::new("profile.json").expect("valid profile ID"),
            ArtifactId::new("summary.md").expect("valid summary ID"),
        ];

        let published = workspace.publish(&required).expect("publish complete run");
        published
            .refresh_latest(&[
                LatestArtifact::same(required[0].clone()),
                LatestArtifact::same(required[1].clone()),
            ])
            .expect("refresh latest artifacts");

        assert!(published.path().join("profile.json").is_file());
        assert!(published.path().join("summary.md").is_file());
        assert_eq!(
            fs::read_to_string(temp.path().join("profile.json")).expect("read latest profile"),
            "new profile"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("summary.md")).expect("read latest summary"),
            "new summary"
        );
    }

    #[test]
    fn concurrent_latest_refreshes_leave_one_complete_alias_pair() {
        use std::sync::{Arc, Barrier};

        const WRITERS: usize = 8;
        let temp = tempfile::tempdir().expect("tempdir");
        let required = [
            ArtifactId::new("profile.json").expect("valid profile ID"),
            ArtifactId::new("summary.md").expect("valid summary ID"),
        ];
        let mut published = Vec::new();
        for index in 0..WRITERS {
            let logical_id = ArtifactId::new(format!("run-{index}")).expect("valid logical ID");
            let workspace =
                RunWorkspace::allocate(temp.path(), &logical_id).expect("allocate workspace");
            let receipt = format!("run-{index}");
            fs::write(workspace.staging_path().join("profile.json"), &receipt)
                .expect("write staged profile");
            fs::write(workspace.staging_path().join("summary.md"), &receipt)
                .expect("write staged summary");
            published.push(workspace.publish(&required).expect("publish complete run"));
        }

        let barrier = Arc::new(Barrier::new(WRITERS));
        let handles: Vec<_> = published
            .into_iter()
            .map(|published| {
                let barrier = Arc::clone(&barrier);
                let required = required.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    published.refresh_latest(&[
                        LatestArtifact::same(required[0].clone()),
                        LatestArtifact::same(required[1].clone()),
                    ])
                })
            })
            .collect();

        for handle in handles {
            handle
                .join()
                .expect("latest refresh thread")
                .expect("serialize latest refresh");
        }

        let profile =
            fs::read_to_string(temp.path().join("profile.json")).expect("read latest profile");
        let summary =
            fs::read_to_string(temp.path().join("summary.md")).expect("read latest summary");
        assert_eq!(profile, summary, "latest aliases came from different runs");
        assert!(profile.starts_with("run-"));
    }

    #[test]
    fn rollback_restores_other_backups_after_installed_file_removal_fails() {
        let temp = tempfile::tempdir().expect("tempdir");
        let blocked_destination = temp.path().join("blocked");
        let restorable_destination = temp.path().join("restorable");
        let blocked_backup = temp.path().join("blocked.backup");
        let restorable_backup = temp.path().join("restorable.backup");
        fs::create_dir(&blocked_destination).expect("create removal failure directory");
        fs::write(&restorable_destination, "new").expect("write installed file");
        fs::write(&blocked_backup, "old blocked").expect("write blocked backup");
        fs::write(&restorable_backup, "old restored").expect("write restorable backup");
        let original = ArtifactPathError::Io {
            operation: "install latest artifact",
            path: restorable_destination.clone(),
            source: std::io::Error::other("simulated install failure"),
        };

        let error = latest_update_failure(
            original,
            &[restorable_destination.clone(), blocked_destination.clone()],
            &[
                (blocked_destination.clone(), blocked_backup.clone()),
                (restorable_destination.clone(), restorable_backup),
            ],
        );

        assert!(matches!(error, ArtifactPathError::LatestRollback { .. }));
        assert_eq!(
            fs::read_to_string(&restorable_destination).expect("read restored backup"),
            "old restored"
        );
        assert!(blocked_backup.is_file());
    }

    #[test]
    fn latest_refresh_prepares_every_source_before_replacing_destinations() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("profile.json"), "old profile").expect("seed latest profile");
        fs::write(temp.path().join("summary.md"), "old summary").expect("seed latest summary");
        let logical_id = ArtifactId::new("android-sample").expect("valid logical ID");
        let workspace =
            RunWorkspace::allocate(temp.path(), &logical_id).expect("allocate workspace");
        fs::write(workspace.staging_path().join("profile.json"), "new profile")
            .expect("write staged profile");
        fs::write(workspace.staging_path().join("summary.md"), "new summary")
            .expect("write staged summary");
        let required = [
            ArtifactId::new("profile.json").expect("valid profile ID"),
            ArtifactId::new("summary.md").expect("valid summary ID"),
        ];
        let published = workspace.publish(&required).expect("publish complete run");
        fs::remove_file(published.path().join("summary.md"))
            .expect("remove second published source");

        let error = published
            .refresh_latest(&[
                LatestArtifact::same(required[0].clone()),
                LatestArtifact::same(required[1].clone()),
            ])
            .expect_err("reject incomplete latest source set");

        assert!(matches!(
            error,
            ArtifactPathError::RequiredArtifactMissing { .. }
        ));
        assert_eq!(
            fs::read_to_string(temp.path().join("profile.json")).expect("read latest profile"),
            "old profile"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("summary.md")).expect("read latest summary"),
            "old summary"
        );
    }

    #[test]
    fn approved_root_prepares_nested_directory_beneath_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = ApprovedRoot::existing(temp.path()).expect("approve root");

        let prepared = root
            .prepare_dir("plots/nested")
            .expect("prepare nested directory");

        assert_eq!(prepared, root.path().join("plots/nested"));
        assert!(prepared.is_dir());
    }

    #[test]
    fn approved_root_projects_a_missing_directory_without_creating_it() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = ApprovedRoot::existing(temp.path()).expect("approve root");

        let projected = root
            .project_dir("target/mobench")
            .expect("project output directory");

        assert_eq!(projected, root.path().join("target/mobench"));
        assert!(!projected.exists());
        assert!(!root.path().join("target").exists());
    }

    #[test]
    fn approved_root_rejects_paths_that_are_not_relative_descendants() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = ApprovedRoot::existing(temp.path()).expect("approve root");

        for invalid in [Path::new("../escape"), Path::new("/absolute/escape")] {
            assert!(matches!(
                root.prepare_dir(invalid),
                Err(ArtifactPathError::InvalidRelativePath { .. })
            ));
        }

        assert!(!root.path().join("../escape").exists());
    }

    #[test]
    fn approved_root_projection_rejects_paths_that_are_not_relative_descendants() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = ApprovedRoot::existing(temp.path()).expect("approve root");

        for invalid in [Path::new("../escape"), Path::new("/absolute/escape")] {
            assert!(matches!(
                root.project_dir(invalid),
                Err(ArtifactPathError::InvalidRelativePath { .. })
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn approved_root_rejects_symlink_directory_components_without_following_them() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        symlink(outside.path(), temp.path().join("linked")).expect("create symlink");
        let root = ApprovedRoot::existing(temp.path()).expect("approve root");

        assert!(matches!(
            root.prepare_dir("linked/nested"),
            Err(ArtifactPathError::SymlinkComponent { .. })
        ));
        assert!(!outside.path().join("nested").exists());
    }

    #[cfg(unix)]
    #[test]
    fn approved_root_projection_rejects_symlink_directory_components_without_writing() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        symlink(outside.path(), temp.path().join("linked")).expect("create symlink");
        let root = ApprovedRoot::existing(temp.path()).expect("approve root");

        assert!(matches!(
            root.project_dir("linked/nested"),
            Err(ArtifactPathError::SymlinkComponent { .. })
        ));
        assert!(!outside.path().join("nested").exists());
    }

    #[test]
    fn approved_root_prepares_file_parent_without_creating_the_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = ApprovedRoot::existing(temp.path()).expect("approve root");

        let prepared = root
            .prepare_file("plots/nested/chart.svg")
            .expect("prepare artifact file");

        assert_eq!(prepared, root.path().join("plots/nested/chart.svg"));
        assert!(prepared.parent().expect("parent").is_dir());
        assert!(!prepared.exists());
    }

    #[cfg(unix)]
    #[test]
    fn approved_root_rejects_preexisting_symlink_file_without_following_it() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let root = ApprovedRoot::existing(temp.path()).expect("approve root");
        let plots = root.prepare_dir("plots").expect("prepare plots");
        let outside = tempfile::NamedTempFile::new().expect("outside file");
        fs::write(outside.path(), "unchanged").expect("seed outside file");
        symlink(outside.path(), plots.join("chart.svg")).expect("create symlink");

        assert!(matches!(
            root.prepare_file("plots/chart.svg"),
            Err(ArtifactPathError::SymlinkComponent { .. })
        ));
        assert_eq!(
            fs::read_to_string(outside.path()).expect("read outside file"),
            "unchanged"
        );
    }

    #[cfg(unix)]
    #[test]
    fn approved_root_rejects_a_symlink_as_the_root() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().expect("parent tempdir");
        let actual_root = tempfile::tempdir().expect("actual root");
        let linked_root = parent.path().join("linked-root");
        symlink(actual_root.path(), &linked_root).expect("create root symlink");

        assert!(matches!(
            ApprovedRoot::existing(&linked_root),
            Err(ArtifactPathError::SymlinkComponent { path }) if path == linked_root
        ));
    }
}
