//! Contained artifact roots and atomic run workspaces.
//!
//! [`ApprovedRoot`] validates paths immediately before use. It prevents
//! traversal through pre-existing symlinks, but it does not yet provide the
//! descriptor-relative operations needed to close hostile concurrent-swap
//! races on every supported platform.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// A validated artifact identifier that is safe to use as one path component.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
const STAGING_QUARANTINE_DIRECTORY: &str = ".mobench-staging-quarantine";
const STAGING_LOCK_FILE: &str = ".mobench-staging.lock";
const WORKSPACE_LOCK_FILE: &str = ".mobench-workspace.lock";
const RUN_READER_LOCK_FILE: &str = ".mobench-run-reader.lock";
const RUN_RETENTION_QUARANTINE_DIRECTORY: &str = ".mobench-run-quarantine";
const LATEST_DIRECTORY: &str = ".mobench-latest";
const LATEST_STAGING_DIRECTORY: &str = "staging";
const LATEST_GENERATIONS_DIRECTORY: &str = "generations";
const LATEST_QUARANTINE_DIRECTORY: &str = "quarantine";
const LATEST_CURRENT_FILE: &str = "current";
const LATEST_LOCK_FILE: &str = ".mobench-latest.lock";
const LATEST_READER_LOCK_FILE: &str = ".mobench-reader.lock";
const LATEST_RETENTION_BOUNDARY_FILE: &str = ".mobench-retention-boundary.json";
const RETAIN_LATEST_GENERATIONS: usize = 8;
const RETAIN_PUBLISHED_RUNS: usize = 32;
const RETAIN_QUARANTINE_ENTRIES: usize = 16;
/// File name of the versioned manifest stored in every published run.
pub const RUN_MANIFEST_FILE: &str = "mobench-run-manifest.json";
/// File name of the versioned manifest stored in every latest generation.
pub const LATEST_MANIFEST_FILE: &str = "manifest.json";
const RUN_MANIFEST_VERSION: u32 = 1;
const LATEST_MANIFEST_VERSION: u32 = 1;
const LATEST_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const LATEST_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(5);
const MAX_ALLOCATION_ATTEMPTS: usize = 1_024;
static WORKSPACE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static ACTIVE_WORKSPACES: OnceLock<Mutex<BTreeSet<PathBuf>>> = OnceLock::new();

/// A unique private directory for building one complete run before publication.
#[derive(Debug)]
pub struct RunWorkspace {
    root: ApprovedRoot,
    logical_id: ArtifactId,
    expected_latest_generation: Option<String>,
    workspace_lock: Option<File>,
    staging_path: Option<PathBuf>,
    published_path: PathBuf,
}

/// A completed run directory that has been atomically renamed into visibility.
#[derive(Debug, Clone)]
pub struct PublishedRun {
    root: ApprovedRoot,
    path: PathBuf,
    _reader_lease: Arc<File>,
}

/// One regular file recorded in a durable artifact manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestArtifact {
    /// Slash-separated path relative to the manifest's directory.
    pub relative_path: String,
    /// Exact file length in bytes.
    pub size: u64,
    /// Lower-case hexadecimal SHA-256 digest.
    pub sha256: String,
}

/// Durable identity and integrity record for one published run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunManifest {
    /// Manifest format version. Unknown versions are rejected.
    pub format_version: u32,
    /// Version of the mobench-artifacts producer.
    pub producer_version: String,
    /// Caller-provided logical run identity.
    pub logical_id: String,
    /// Collision-resistant published directory identity.
    pub publication_id: String,
    /// Latest generation observed when this run workspace was allocated.
    /// Refresh uses this as a compare-and-swap token.
    pub expected_latest_generation: Option<String>,
    /// Every regular run artifact except this manifest, sorted by path.
    pub artifacts: Vec<ManifestArtifact>,
}

/// Mapping from a published run artifact into one latest-generation alias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatestManifestArtifact {
    /// Source path relative to the published run.
    pub source_relative_path: String,
    /// Stable public alias and path relative to the generation directory.
    pub destination_relative_path: String,
    /// Exact file length in bytes.
    pub size: u64,
    /// Lower-case hexadecimal SHA-256 digest.
    pub sha256: String,
}

/// Durable identity and integrity record for one immutable latest generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatestManifest {
    /// Manifest format version. Unknown versions are rejected.
    pub format_version: u32,
    /// Version of the mobench-artifacts producer.
    pub producer_version: String,
    /// Identity named by the atomic `current` pointer.
    pub generation: String,
    /// Previously committed generation, when one existed.
    pub predecessor_generation: Option<String>,
    /// Logical identity of the source run.
    pub source_logical_id: String,
    /// Publication identity of the source run.
    pub source_publication_id: String,
    /// Aliases in this generation, sorted by destination path.
    pub artifacts: Vec<LatestManifestArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RetentionBoundary {
    format_version: u32,
    predecessor_generation: String,
}

/// A reader pinned to one immutable latest generation.
///
/// Opening resolves the atomic pointer once and verifies the generation
/// manifest and every listed artifact. Subsequent reads never re-resolve
/// `current`, so a concurrent writer cannot mix generations for this reader.
#[derive(Debug, Clone)]
pub struct LatestSnapshot {
    root: ApprovedRoot,
    generation_path: PathBuf,
    manifest: LatestManifest,
    _reader_lease: Arc<File>,
    _source_run_lease: Arc<File>,
    #[cfg(unix)]
    generation_directory: Arc<File>,
}

#[derive(Debug)]
struct LatestUpdateLock {
    file: File,
}

#[derive(Debug)]
struct StableAliasTransaction {
    path: PathBuf,
    root_path: PathBuf,
    installed: Vec<PathBuf>,
    backups: Vec<(PathBuf, PathBuf)>,
}

#[derive(Debug)]
enum PointerCommitFailure {
    BeforeCommit(ArtifactPathError),
    AfterCommit(ArtifactPathError),
}

impl StableAliasTransaction {
    fn rollback(self, original: ArtifactPathError) -> ArtifactPathError {
        let error =
            latest_update_failure(original, &self.installed, &self.backups, self.path.clone());
        let rollback_complete = !matches!(&error, ArtifactPathError::LatestRollback { .. });
        if rollback_complete {
            let _ = fs::remove_dir_all(&self.path);
        }
        let _ = sync_directory(&self.root_path);
        error
    }

    fn finish(self) {
        let _ = fs::remove_dir_all(self.path);
    }
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
        let expected_latest_generation =
            recover_latest_for_writer(&root)?.map(|snapshot| snapshot.manifest.generation);
        let _staging_manager_lock = acquire_staging_manager_lock(&root)?;
        let staging_root = root.prepare_dir(STAGING_DIRECTORY)?;
        quarantine_abandoned_run_staging(&root, &staging_root)?;

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
                    let workspace_lock_path = staging_path.join(WORKSPACE_LOCK_FILE);
                    let workspace_lock = match OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create_new(true)
                        .open(&workspace_lock_path)
                    {
                        Ok(file) => file,
                        Err(source) => {
                            let _ = fs::remove_dir_all(&staging_path);
                            return Err(ArtifactPathError::Io {
                                operation: "create run-workspace lock",
                                path: workspace_lock_path,
                                source,
                            });
                        }
                    };
                    if let Err(source) = FileExt::lock_exclusive(&workspace_lock) {
                        let _ = fs::remove_dir_all(&staging_path);
                        return Err(ArtifactPathError::Io {
                            operation: "lock run workspace",
                            path: workspace_lock_path,
                            source,
                        });
                    }
                    register_active_workspace(&staging_path);
                    return Ok(Self {
                        root,
                        logical_id: logical_id.clone(),
                        expected_latest_generation,
                        workspace_lock: Some(workspace_lock),
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

        let manifest_path = staging_path.join(RUN_MANIFEST_FILE);
        match fs::symlink_metadata(&manifest_path) {
            Ok(_) => {
                return Err(ArtifactPathError::ReservedManifestPath {
                    path: manifest_path,
                });
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ArtifactPathError::Io {
                    operation: "inspect reserved run manifest path",
                    path: manifest_path,
                    source,
                });
            }
        }

        create_run_reader_lease_file(&staging_path)?;
        let artifacts =
            inspect_artifact_tree(&staging_path, &[WORKSPACE_LOCK_FILE, RUN_READER_LOCK_FILE])?;
        let publication_id = published_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ArtifactPathError::NonUtf8ArtifactPath {
                path: published_path.clone(),
            })?
            .to_owned();
        let manifest = RunManifest {
            format_version: RUN_MANIFEST_VERSION,
            producer_version: env!("CARGO_PKG_VERSION").to_owned(),
            logical_id: self.logical_id.as_str().to_owned(),
            publication_id,
            expected_latest_generation: self.expected_latest_generation.clone(),
            artifacts,
        };
        write_json_file(&manifest_path, &manifest, "write run manifest")?;
        sync_artifact_tree(&staging_path)?;

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

        let _staging_manager_lock = acquire_staging_manager_lock(&self.root)?;
        let workspace_lock_path = staging_path.join(WORKSPACE_LOCK_FILE);
        let workspace_lock = self.workspace_lock.take();
        if let Some(workspace_lock) = workspace_lock.as_ref() {
            let _ = FileExt::unlock(workspace_lock);
        }
        // Close before unlinking so publication also works on platforms that
        // do not allow deleting an open file. The staging-manager lock keeps
        // recovery from racing this short handoff window.
        drop(workspace_lock);
        fs::remove_file(&workspace_lock_path).map_err(|source| ArtifactPathError::Io {
            operation: "remove run-workspace lock before publication",
            path: workspace_lock_path,
            source,
        })?;
        sync_directory(&staging_path)?;

        rename_directory_noreplace(&staging_path, &published_path).map_err(|source| {
            ArtifactPathError::Io {
                operation: "publish completed run",
                path: published_path.clone(),
                source,
            }
        })?;
        record_durability_event("publish_run");
        let published_sync = sync_directory(
            published_path
                .parent()
                .expect("published run always has an artifact-root parent"),
        );
        let staging_sync = sync_directory(
            staging_path
                .parent()
                .expect("staging run always has a staging-root parent"),
        );
        unregister_active_workspace(&staging_path);
        self.staging_path = None;

        if let Err(source) = published_sync.and(staging_sync) {
            return Err(ArtifactPathError::PublicationDurabilityUncertain {
                path: published_path,
                source: Box::new(source),
            });
        }

        let reader_lease = open_shared_lease(
            &published_path.join(RUN_READER_LOCK_FILE),
            "open published-run reader lease",
            "lock published-run reader lease",
        )
        .map_err(
            |source| ArtifactPathError::PublicationPostCommitMaintenance {
                path: published_path.clone(),
                source: Box::new(source),
            },
        )?;
        let published = PublishedRun {
            root: self.root.clone(),
            path: published_path,
            _reader_lease: reader_lease,
        };
        prune_published_runs(&self.root).map_err(|source| {
            ArtifactPathError::PublicationPostCommitMaintenance {
                path: published.path.clone(),
                source: Box::new(source),
            }
        })?;
        Ok(published)
    }
}

impl Drop for RunWorkspace {
    fn drop(&mut self) {
        if let Some(staging_path) = self.staging_path.as_ref() {
            if let Ok(_manager_lock) = acquire_staging_manager_lock(&self.root) {
                let _ = fs::remove_dir_all(staging_path);
            }
            unregister_active_workspace(staging_path);
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

    /// Read and verify this run's versioned manifest and artifact digests.
    pub fn manifest(&self) -> Result<RunManifest, ArtifactPathError> {
        validate_run_manifest(&self.path)
    }

    /// Commit an immutable latest generation and refresh legacy stable copies.
    ///
    /// The generation becomes authoritative through one atomic `current`
    /// pointer rename. Root-level stable copies are retained for compatibility;
    /// because that legacy file set cannot be swapped atomically, the next
    /// refresh first repairs it from the already committed generation.
    pub fn refresh_latest(&self, artifacts: &[LatestArtifact]) -> Result<(), ArtifactPathError> {
        let published_root = ApprovedRoot::existing(&self.path)?;
        let mut prepared: Vec<(PathBuf, PathBuf, ArtifactId, ArtifactId)> =
            Vec::with_capacity(artifacts.len());
        for artifact in artifacts {
            if prepared
                .iter()
                .any(|(_, _, _, destination)| destination == &artifact.destination)
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
            prepared.push((
                source,
                destination,
                artifact.source.clone(),
                artifact.destination.clone(),
            ));
        }
        let run_manifest = self.manifest()?;
        maybe_mutate_latest_source_after_validation();

        let latest_root_path = self.root.prepare_dir(LATEST_DIRECTORY)?;
        let latest_root = ApprovedRoot::existing(&latest_root_path)?;
        latest_root.prepare_dir(LATEST_STAGING_DIRECTORY)?;
        latest_root.prepare_dir(LATEST_GENERATIONS_DIRECTORY)?;
        latest_root.prepare_dir(LATEST_QUARANTINE_DIRECTORY)?;
        // Allocation and lease acquisition are one short serialized operation.
        // Without this fence, another writer could observe the directory in the
        // interval before its lease exists and quarantine an active staging tree.
        let (generation, staging_path, staging_lease) = {
            let _latest_update_lock = acquire_latest_update_lock(&self.root)?;
            let (generation, staging_path) = allocate_latest_generation(&latest_root)?;
            let lease_result = (|| {
                create_reader_lease_file(&staging_path)?;
                let staging_lease_path = staging_path.join(LATEST_READER_LOCK_FILE);
                let staging_lease = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&staging_lease_path)
                    .map_err(|source| ArtifactPathError::Io {
                        operation: "open active latest-generation staging lease",
                        path: staging_lease_path.clone(),
                        source,
                    })?;
                FileExt::lock_exclusive(&staging_lease).map_err(|source| {
                    ArtifactPathError::Io {
                        operation: "lock active latest-generation staging lease",
                        path: staging_lease_path,
                        source,
                    }
                })?;
                Ok(staging_lease)
            })();
            match lease_result {
                Ok(staging_lease) => (generation, staging_path, staging_lease),
                Err(error) => {
                    let _ = quarantine_latest_path(&latest_root, &staging_path);
                    return Err(error);
                }
            }
        };
        let mut staging_lease = Some(staging_lease);

        // Copy, hash, and sync potentially large artifacts before entering the
        // serialized commit section. The staging lease prevents another
        // writer's recovery pass from quarantining this active generation.
        let preparation_result = (|| {
            let mut manifest_artifacts = Vec::with_capacity(prepared.len());
            for (source, _, source_id, destination_id) in &prepared {
                let staged = staging_path.join(destination_id.as_str());
                copy_new_synced(source, &staged, "stage latest generation artifact")?;
                let (size, sha256) = digest_file(&staged)?;
                let expected = run_manifest
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.relative_path == source_id.as_str())
                    .ok_or_else(|| ArtifactPathError::ManifestIntegrity {
                        path: self.path.join(RUN_MANIFEST_FILE),
                        detail: format!(
                            "latest source `{source_id}` is not recorded in the run manifest"
                        ),
                    })?;
                if expected.size != size || expected.sha256 != sha256 {
                    return Err(ArtifactPathError::ManifestIntegrity {
                        path: source.clone(),
                        detail: format!(
                            "latest source `{source_id}` changed after run-manifest validation"
                        ),
                    });
                }
                manifest_artifacts.push(LatestManifestArtifact {
                    source_relative_path: source_id.as_str().to_owned(),
                    destination_relative_path: destination_id.as_str().to_owned(),
                    size,
                    sha256,
                });
            }
            manifest_artifacts.sort_by(|left, right| {
                left.destination_relative_path
                    .cmp(&right.destination_relative_path)
            });
            sync_artifact_tree(&staging_path)?;
            Ok(manifest_artifacts)
        })();
        let manifest_artifacts = match preparation_result {
            Ok(manifest_artifacts) => manifest_artifacts,
            Err(error) => {
                if let Some(lease) = staging_lease.take() {
                    let _ = FileExt::unlock(&lease);
                    drop(lease);
                }
                if let Err(cleanup) = quarantine_latest_path(&latest_root, &staging_path) {
                    return Err(ArtifactPathError::LatestStagingCleanup {
                        original: Box::new(error),
                        cleanup: Box::new(cleanup),
                        staging_path,
                    });
                }
                return Err(error);
            }
        };

        let generation_path = latest_root
            .path()
            .join(LATEST_GENERATIONS_DIRECTORY)
            .join(&generation);
        let mut generation_committed = false;
        let generation_result = (|| {
            let _latest_update_lock = acquire_latest_update_lock(&self.root)?;
            let predecessor = recover_latest_state_locked(&self.root, &latest_root)?;
            let observed_generation = predecessor
                .as_ref()
                .map(|snapshot| snapshot.manifest.generation.clone());
            if run_manifest.expected_latest_generation != observed_generation {
                return Err(ArtifactPathError::StaleLatestGeneration {
                    expected: run_manifest.expected_latest_generation.clone(),
                    observed: observed_generation,
                });
            }

            let manifest = LatestManifest {
                format_version: LATEST_MANIFEST_VERSION,
                producer_version: env!("CARGO_PKG_VERSION").to_owned(),
                generation: generation.clone(),
                predecessor_generation: predecessor
                    .as_ref()
                    .map(|snapshot| snapshot.manifest.generation.clone()),
                source_logical_id: run_manifest.logical_id.clone(),
                source_publication_id: run_manifest.publication_id.clone(),
                artifacts: manifest_artifacts.clone(),
            };
            write_json_file(
                &staging_path.join(LATEST_MANIFEST_FILE),
                &manifest,
                "write latest-generation manifest",
            )?;
            sync_directory(&staging_path)?;

            fs::rename(&staging_path, &generation_path).map_err(|source| {
                ArtifactPathError::Io {
                    operation: "publish latest generation",
                    path: generation_path.clone(),
                    source,
                }
            })?;
            record_durability_event("publish_generation");
            sync_directory(&latest_root.path().join(LATEST_STAGING_DIRECTORY))?;
            sync_directory(&latest_root.path().join(LATEST_GENERATIONS_DIRECTORY))?;
            if let Some(lease) = staging_lease.take() {
                let _ = FileExt::unlock(&lease);
                drop(lease);
            }

            let generation_manifest = validate_latest_manifest(&generation_path, &generation)?;
            let candidate = make_latest_snapshot(
                self.root.clone(),
                generation_path.clone(),
                generation_manifest,
            )?;
            let alias_transaction =
                install_stable_aliases_transactional(&self.root, &latest_root, &candidate)?;
            match commit_current_pointer(&latest_root, &generation) {
                Ok(()) => {
                    generation_committed = true;
                    alias_transaction.finish();
                }
                Err(PointerCommitFailure::BeforeCommit(pointer_error)) => {
                    return Err(alias_transaction.rollback(pointer_error));
                }
                Err(PointerCommitFailure::AfterCommit(durability_error)) => {
                    generation_committed = true;
                    alias_transaction.finish();
                    return Err(ArtifactPathError::LatestCommitDurabilityUncertain {
                        generation: generation.clone(),
                        source: Box::new(durability_error),
                    });
                }
            }
            prune_latest_generations(&latest_root, candidate.manifest()).map_err(|source| {
                ArtifactPathError::LatestRetentionAfterCommit {
                    generation: generation.clone(),
                    source: Box::new(source),
                }
            })?;
            Ok(())
        })();

        if generation_result.is_err() {
            if let Some(lease) = staging_lease.take() {
                let _ = FileExt::unlock(&lease);
                drop(lease);
            }
            if staging_path.exists() {
                quarantine_latest_path(&latest_root, &staging_path)?;
            } else if generation_path.exists() && !generation_committed {
                quarantine_latest_path(&latest_root, &generation_path)?;
            }
        }
        generation_result
    }
}

impl LatestSnapshot {
    /// Resolve and verify the committed generation without writing or locking.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, ArtifactPathError> {
        let root = ApprovedRoot::existing(root)?;
        load_current_snapshot(&root, false)?.ok_or_else(|| {
            ArtifactPathError::LatestSnapshotUnavailable {
                path: root.path().join(LATEST_DIRECTORY).join(LATEST_CURRENT_FILE),
            }
        })
    }

    /// Repair legacy root-level aliases from the committed generation.
    ///
    /// This is idempotent and is also run automatically before every writer
    /// creates a new generation. The returned reader remains pinned to the
    /// generation used for recovery.
    pub fn recover_stable_aliases(root: impl AsRef<Path>) -> Result<Self, ArtifactPathError> {
        let root = ApprovedRoot::existing(root)?;
        let snapshot = recover_latest_for_writer(&root)?.ok_or_else(|| {
            ArtifactPathError::LatestSnapshotUnavailable {
                path: root.path().join(LATEST_DIRECTORY).join(LATEST_CURRENT_FILE),
            }
        })?;
        Ok(snapshot)
    }

    /// Return the pinned generation identity.
    pub fn generation(&self) -> &str {
        &self.manifest.generation
    }

    /// Return the immutable generation directory.
    pub fn path(&self) -> &Path {
        &self.generation_path
    }

    /// Return the approved artifact root containing this snapshot.
    pub fn root(&self) -> &ApprovedRoot {
        &self.root
    }

    /// Return the verified versioned manifest.
    pub fn manifest(&self) -> &LatestManifest {
        &self.manifest
    }

    /// Open one alias listed by this pinned generation.
    ///
    /// Callers that need integrity verification should prefer
    /// [`LatestSnapshot::read_artifact`], which verifies the exact bytes read
    /// against the pinned manifest.
    pub fn open_artifact(&self, id: &ArtifactId) -> Result<File, ArtifactPathError> {
        if !self
            .manifest
            .artifacts
            .iter()
            .any(|artifact| artifact.destination_relative_path == id.as_str())
        {
            return Err(ArtifactPathError::LatestArtifactNotInSnapshot { id: id.clone() });
        }
        let path = self.generation_path.join(id.as_str());
        open_snapshot_artifact(self, id).map_err(|source| ArtifactPathError::Io {
            operation: "open latest snapshot artifact",
            path,
            source,
        })
    }

    /// Read one alias from this pinned generation.
    pub fn read_artifact(&self, id: &ArtifactId) -> Result<Vec<u8>, ArtifactPathError> {
        let expected = self
            .manifest
            .artifacts
            .iter()
            .find(|artifact| artifact.destination_relative_path == id.as_str())
            .ok_or_else(|| ArtifactPathError::LatestArtifactNotInSnapshot { id: id.clone() })?;
        let mut file = self.open_artifact(id)?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)
            .map_err(|source| ArtifactPathError::Io {
                operation: "read latest snapshot artifact",
                path: self.generation_path.join(id.as_str()),
                source,
            })?;
        let actual_digest = sha256_bytes(&contents);
        if contents.len() as u64 != expected.size || actual_digest != expected.sha256 {
            return Err(ArtifactPathError::CorruptGeneration {
                path: self.generation_path.join(id.as_str()),
                detail: "artifact bytes changed after snapshot validation".to_owned(),
            });
        }
        Ok(contents)
    }
}

fn acquire_latest_update_lock(root: &ApprovedRoot) -> Result<LatestUpdateLock, ArtifactPathError> {
    acquire_named_lock(
        root,
        LATEST_LOCK_FILE,
        "open latest-artifact lock",
        "lock latest artifacts",
    )
}

fn acquire_staging_manager_lock(
    root: &ApprovedRoot,
) -> Result<LatestUpdateLock, ArtifactPathError> {
    acquire_named_lock(
        root,
        STAGING_LOCK_FILE,
        "open run-staging lock",
        "lock run staging",
    )
}

fn acquire_named_lock(
    root: &ApprovedRoot,
    name: &str,
    open_operation: &'static str,
    lock_operation: &'static str,
) -> Result<LatestUpdateLock, ArtifactPathError> {
    let path = root.prepare_file(name)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| ArtifactPathError::Io {
            operation: open_operation,
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
                    operation: lock_operation,
                    path,
                    source,
                });
            }
        }
    }
}

fn quarantine_abandoned_run_staging(
    root: &ApprovedRoot,
    staging_root: &Path,
) -> Result<(), ArtifactPathError> {
    let quarantine_root = root.prepare_dir(STAGING_QUARANTINE_DIRECTORY)?;
    let entries = fs::read_dir(staging_root).map_err(|source| ArtifactPathError::Io {
        operation: "list run staging for recovery",
        path: staging_root.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| ArtifactPathError::Io {
            operation: "inspect run staging entry",
            path: staging_root.to_path_buf(),
            source,
        })?;
        let staging_path = entry.path();
        if is_active_workspace(&staging_path) {
            continue;
        }
        let metadata =
            fs::symlink_metadata(&staging_path).map_err(|source| ArtifactPathError::Io {
                operation: "inspect run staging workspace",
                path: staging_path.clone(),
                source,
            })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            quarantine_run_staging_path(staging_root, &quarantine_root, &staging_path)?;
            continue;
        }

        let workspace_lock_path = staging_path.join(WORKSPACE_LOCK_FILE);
        match fs::symlink_metadata(&workspace_lock_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                quarantine_run_staging_path(staging_root, &quarantine_root, &staging_path)?;
                continue;
            }
            Ok(_) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ArtifactPathError::Io {
                    operation: "inspect run-workspace lock",
                    path: workspace_lock_path,
                    source,
                });
            }
        }
        let workspace_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&workspace_lock_path)
            .map_err(|source| ArtifactPathError::Io {
                operation: "open staged run-workspace lock",
                path: workspace_lock_path.clone(),
                source,
            })?;
        match FileExt::try_lock_exclusive(&workspace_lock) {
            Ok(()) => {
                quarantine_run_staging_path(staging_root, &quarantine_root, &staging_path)?;
            }
            Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(source) => {
                return Err(ArtifactPathError::Io {
                    operation: "lock staged run workspace for recovery",
                    path: workspace_lock_path,
                    source,
                });
            }
        }
    }
    prune_quarantine_entries(&quarantine_root, RETAIN_QUARANTINE_ENTRIES)
}

fn quarantine_run_staging_path(
    staging_root: &Path,
    quarantine_root: &Path,
    staging_path: &Path,
) -> Result<(), ArtifactPathError> {
    let name = staging_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unreadable");
    let quarantine_path = quarantine_root.join(format!("{name}--{}", workspace_nonce()));
    fs::rename(staging_path, &quarantine_path).map_err(|source| ArtifactPathError::Io {
        operation: "quarantine abandoned run workspace",
        path: staging_path.to_path_buf(),
        source,
    })?;
    sync_directory(staging_root)?;
    sync_directory(quarantine_root)
}

fn allocate_latest_generation(
    latest_root: &ApprovedRoot,
) -> Result<(String, PathBuf), ArtifactPathError> {
    let staging_root = latest_root.prepare_dir(LATEST_STAGING_DIRECTORY)?;
    for _ in 0..MAX_ALLOCATION_ATTEMPTS {
        let generation = format!("generation-{}", workspace_nonce());
        let path = staging_root.join(&generation);
        match fs::create_dir(&path) {
            Ok(()) => return Ok((generation, path)),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(ArtifactPathError::Io {
                    operation: "create latest staging generation",
                    path,
                    source,
                });
            }
        }
    }
    Err(ArtifactPathError::LatestTransactionAllocationExhausted)
}

fn quarantine_abandoned_latest_staging(
    latest_root: &ApprovedRoot,
) -> Result<(), ArtifactPathError> {
    let staging = latest_root.prepare_dir(LATEST_STAGING_DIRECTORY)?;
    let entries = fs::read_dir(&staging).map_err(|source| ArtifactPathError::Io {
        operation: "list abandoned latest staging",
        path: staging.clone(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| ArtifactPathError::Io {
            operation: "inspect abandoned latest staging entry",
            path: staging.clone(),
            source,
        })?;
        let staging_path = entry.path();
        let metadata =
            fs::symlink_metadata(&staging_path).map_err(|source| ArtifactPathError::Io {
                operation: "inspect abandoned latest staging entry",
                path: staging_path.clone(),
                source,
            })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            quarantine_latest_path(latest_root, &staging_path)?;
            continue;
        }

        let lease_path = staging_path.join(LATEST_READER_LOCK_FILE);
        let lease_metadata = match fs::symlink_metadata(&lease_path) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                quarantine_latest_path(latest_root, &staging_path)?;
                continue;
            }
            Err(source) => {
                return Err(ArtifactPathError::Io {
                    operation: "inspect latest-generation staging lease",
                    path: lease_path,
                    source,
                });
            }
        };
        if lease_metadata.file_type().is_symlink() || !lease_metadata.is_file() {
            quarantine_latest_path(latest_root, &staging_path)?;
            continue;
        }
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lease_path)
            .map_err(|source| ArtifactPathError::Io {
                operation: "open latest-generation staging lease for recovery",
                path: lease_path.clone(),
                source,
            })?;
        match FileExt::try_lock_exclusive(&lease) {
            Ok(()) => quarantine_latest_path(latest_root, &staging_path)?,
            Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(source) => {
                return Err(ArtifactPathError::Io {
                    operation: "lock latest-generation staging lease for recovery",
                    path: lease_path,
                    source,
                });
            }
        }
    }
    Ok(())
}

fn quarantine_latest_path(
    latest_root: &ApprovedRoot,
    source_path: &Path,
) -> Result<(), ArtifactPathError> {
    let quarantine = latest_root.prepare_dir(LATEST_QUARANTINE_DIRECTORY)?;
    let source_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unreadable");
    let destination = quarantine.join(format!("{source_name}--{}", workspace_nonce()));
    fs::rename(source_path, &destination).map_err(|source| ArtifactPathError::Io {
        operation: "quarantine abandoned latest staging",
        path: source_path.to_path_buf(),
        source,
    })?;
    sync_directory(
        source_path
            .parent()
            .expect("latest staging entry always has a parent"),
    )?;
    sync_directory(&quarantine)?;
    prune_quarantine_entries(&quarantine, RETAIN_QUARANTINE_ENTRIES)
}

fn commit_current_pointer(
    latest_root: &ApprovedRoot,
    generation: &str,
) -> Result<(), PointerCommitFailure> {
    ArtifactId::new(generation.to_owned()).map_err(PointerCommitFailure::BeforeCommit)?;
    let temporary = latest_root
        .path()
        .join(format!(".current-{}", workspace_nonce()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|source| {
            PointerCommitFailure::BeforeCommit(ArtifactPathError::Io {
                operation: "create latest pointer candidate",
                path: temporary.clone(),
                source,
            })
        })?;
    file.write_all(generation.as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .map_err(|source| {
            PointerCommitFailure::BeforeCommit(ArtifactPathError::Io {
                operation: "write latest pointer candidate",
                path: temporary.clone(),
                source,
            })
        })?;
    file.sync_all().map_err(|source| {
        PointerCommitFailure::BeforeCommit(ArtifactPathError::Io {
            operation: "sync latest pointer candidate",
            path: temporary.clone(),
            source,
        })
    })?;
    record_durability_event("sync_pointer_file");

    let current = latest_root.path().join(LATEST_CURRENT_FILE);
    match fs::symlink_metadata(&current) {
        Ok(metadata) if metadata.is_dir() => {
            let _ = fs::remove_file(&temporary);
            return Err(PointerCommitFailure::BeforeCommit(
                ArtifactPathError::FileDestinationNotFile { path: current },
            ));
        }
        Ok(_) => {}
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            let _ = fs::remove_file(&temporary);
            return Err(PointerCommitFailure::BeforeCommit(ArtifactPathError::Io {
                operation: "inspect current latest pointer",
                path: current,
                source,
            }));
        }
    }
    replace_file_atomically(&temporary, &current).map_err(|source| {
        PointerCommitFailure::BeforeCommit(ArtifactPathError::Io {
            operation: "commit latest pointer",
            path: current,
            source,
        })
    })?;
    record_durability_event("commit_pointer");
    sync_directory(latest_root.path()).map_err(PointerCommitFailure::AfterCommit)
}

fn recover_latest_for_writer(
    root: &ApprovedRoot,
) -> Result<Option<LatestSnapshot>, ArtifactPathError> {
    let _lock = acquire_latest_update_lock(root)?;
    let latest_path = root.prepare_dir(LATEST_DIRECTORY)?;
    let latest_root = ApprovedRoot::existing(latest_path)?;
    latest_root.prepare_dir(LATEST_STAGING_DIRECTORY)?;
    latest_root.prepare_dir(LATEST_GENERATIONS_DIRECTORY)?;
    latest_root.prepare_dir(LATEST_QUARANTINE_DIRECTORY)?;
    recover_latest_state_locked(root, &latest_root)
}

fn recover_latest_state_locked(
    root: &ApprovedRoot,
    latest_root: &ApprovedRoot,
) -> Result<Option<LatestSnapshot>, ArtifactPathError> {
    let (snapshot, needs_alias_refresh) = match load_current_snapshot(root, true) {
        Ok(Some(snapshot)) => {
            quarantine_generations_outside_committed_chain(latest_root, &snapshot.manifest)?;
            (Some(snapshot), true)
        }
        Ok(None) => (
            recover_current_from_generation_chain(root, latest_root)?,
            false,
        ),
        Err(error) if is_recoverable_pointer_error(&error) => (
            recover_current_from_generation_chain(root, latest_root)?,
            false,
        ),
        Err(error) => return Err(error),
    };
    if let Some(snapshot) = snapshot.as_ref() {
        reject_producer_downgrade(&snapshot.manifest.producer_version)?;
        if needs_alias_refresh {
            install_stable_aliases_transactional(root, latest_root, snapshot)?.finish();
        }
    }
    quarantine_abandoned_latest_staging(latest_root)?;
    Ok(snapshot)
}

fn quarantine_generations_outside_committed_chain(
    latest_root: &ApprovedRoot,
    committed: &LatestManifest,
) -> Result<(), ArtifactPathError> {
    let mut committed_chain = BTreeSet::new();
    let mut cursor = committed.clone();
    loop {
        committed_chain.insert(cursor.generation.clone());
        let Some(predecessor) = cursor.predecessor_generation.as_ref() else {
            break;
        };
        let cursor_path = latest_root
            .path()
            .join(LATEST_GENERATIONS_DIRECTORY)
            .join(&cursor.generation);
        if load_retention_boundary(&cursor_path, &cursor)?.is_some() {
            break;
        }
        let path = latest_root
            .path()
            .join(LATEST_GENERATIONS_DIRECTORY)
            .join(predecessor);
        cursor = validate_latest_manifest(&path, predecessor)?;
    }

    let generations = latest_root.path().join(LATEST_GENERATIONS_DIRECTORY);
    let entries = fs::read_dir(&generations).map_err(|source| ArtifactPathError::Io {
        operation: "list uncommitted latest generations",
        path: generations,
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| ArtifactPathError::Io {
            operation: "inspect uncommitted latest generation",
            path: latest_root.path().join(LATEST_GENERATIONS_DIRECTORY),
            source,
        })?;
        let path = entry.path();
        let generation = entry
            .file_name()
            .to_str()
            .ok_or_else(|| ArtifactPathError::NonUtf8ArtifactPath { path: path.clone() })?
            .to_owned();
        if committed_chain.contains(&generation) {
            continue;
        }
        // The current pointer already anchors and validates the committed
        // chain. Everything else is uncommitted recovery material, so move it
        // without parsing attacker-controlled or crash-torn contents.
        quarantine_latest_path(latest_root, &path)?;
    }
    Ok(())
}

fn is_recoverable_pointer_error(error: &ArtifactPathError) -> bool {
    fn is_current(path: &Path) -> bool {
        path.file_name().and_then(|name| name.to_str()) == Some(LATEST_CURRENT_FILE)
    }

    match error {
        ArtifactPathError::InvalidLatestPointer { .. } => true,
        ArtifactPathError::LatestSnapshotUnavailable { path } => is_current(path),
        ArtifactPathError::SymlinkComponent { path }
        | ArtifactPathError::FileDestinationNotFile { path } => is_current(path),
        ArtifactPathError::Io {
            operation, path, ..
        } => operation.contains("latest snapshot pointer") && is_current(path),
        _ => false,
    }
}

fn recover_current_from_generation_chain(
    root: &ApprovedRoot,
    latest_root: &ApprovedRoot,
) -> Result<Option<LatestSnapshot>, ArtifactPathError> {
    let candidate = select_unique_generation_tip(root, latest_root)?;
    // A directory at `current` blocks replacement even when there is no
    // generation to recover. Isolate it before returning or committing.
    quarantine_directory_pointer_for_recovery(latest_root)?;
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    reject_producer_downgrade(&candidate.manifest.producer_version)?;
    let alias_transaction = install_stable_aliases_transactional(root, latest_root, &candidate)?;
    match commit_current_pointer(latest_root, candidate.generation()) {
        Ok(()) => alias_transaction.finish(),
        Err(PointerCommitFailure::BeforeCommit(error)) => {
            return Err(alias_transaction.rollback(error));
        }
        Err(PointerCommitFailure::AfterCommit(error)) => {
            alias_transaction.finish();
            return Err(ArtifactPathError::LatestCommitDurabilityUncertain {
                generation: candidate.generation().to_owned(),
                source: Box::new(error),
            });
        }
    }
    Ok(Some(candidate))
}

fn quarantine_directory_pointer_for_recovery(
    latest_root: &ApprovedRoot,
) -> Result<(), ArtifactPathError> {
    let current = latest_root.path().join(LATEST_CURRENT_FILE);
    let metadata = match fs::symlink_metadata(&current) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(ArtifactPathError::Io {
                operation: "inspect corrupt latest pointer for recovery",
                path: current,
                source,
            });
        }
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(());
    }
    let quarantine = latest_root.prepare_dir(LATEST_QUARANTINE_DIRECTORY)?;
    let destination = quarantine.join(format!("current-corrupt--{}", workspace_nonce()));
    fs::rename(&current, &destination).map_err(|source| ArtifactPathError::Io {
        operation: "quarantine corrupt latest pointer directory",
        path: current,
        source,
    })?;
    sync_directory(latest_root.path())?;
    sync_directory(&quarantine)
}

fn select_unique_generation_tip(
    root: &ApprovedRoot,
    latest_root: &ApprovedRoot,
) -> Result<Option<LatestSnapshot>, ArtifactPathError> {
    let generations_path = latest_root.path().join(LATEST_GENERATIONS_DIRECTORY);
    let mut entries = fs::read_dir(&generations_path)
        .map_err(|source| ArtifactPathError::Io {
            operation: "list latest generations for recovery",
            path: generations_path.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ArtifactPathError::Io {
            operation: "inspect latest generation for recovery",
            path: generations_path.clone(),
            source,
        })?;
    entries.sort_by_key(|entry| entry.file_name());

    let mut manifests = BTreeMap::new();
    let mut retention_boundaries = BTreeSet::new();
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|source| ArtifactPathError::Io {
            operation: "inspect latest generation for recovery",
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ArtifactPathError::SymlinkComponent { path });
        }
        if !metadata.is_dir() {
            return Err(ArtifactPathError::DirectoryComponentNotDirectory { path });
        }
        let generation = entry
            .file_name()
            .to_str()
            .ok_or_else(|| ArtifactPathError::NonUtf8ArtifactPath { path: path.clone() })?
            .to_owned();
        ArtifactId::new(generation.clone())?;
        let manifest = validate_latest_manifest(&path, &generation)?;
        if load_retention_boundary(&path, &manifest)?.is_some() {
            retention_boundaries.insert(generation.clone());
        }
        manifests.insert(generation, (path, manifest));
    }
    if manifests.is_empty() {
        return Ok(None);
    }

    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut predecessors = BTreeSet::new();
    for (generation, (_, manifest)) in &manifests {
        if retention_boundaries.contains(generation) {
            continue;
        }
        if let Some(predecessor) = manifest.predecessor_generation.as_ref() {
            let Some((_, predecessor_manifest)) = manifests.get(predecessor) else {
                return Err(ArtifactPathError::BrokenGenerationChain {
                    generation: generation.clone(),
                    predecessor: predecessor.clone(),
                });
            };
            reject_chain_downgrade(predecessor_manifest, manifest)?;
            children
                .entry(predecessor.clone())
                .or_default()
                .push(generation.clone());
            predecessors.insert(predecessor.clone());
        }
    }
    for (predecessor, children) in &children {
        if children.len() > 1 {
            return Err(ArtifactPathError::AmbiguousGenerationFork {
                predecessor: predecessor.clone(),
                children: children.clone(),
            });
        }
    }
    validate_generation_map_acyclic(&manifests, &retention_boundaries)?;

    let tips: Vec<_> = manifests
        .keys()
        .filter(|generation| !predecessors.contains(*generation))
        .cloned()
        .collect();
    if tips.len() != 1 {
        return Err(ArtifactPathError::AmbiguousGenerationTips { tips });
    }
    let tip = &tips[0];
    let (generation_path, manifest) = manifests
        .remove(tip)
        .expect("selected generation tip came from the manifest map");
    Ok(Some(make_latest_snapshot(
        root.clone(),
        generation_path,
        manifest,
    )?))
}

fn validate_generation_map_acyclic(
    manifests: &BTreeMap<String, (PathBuf, LatestManifest)>,
    retention_boundaries: &BTreeSet<String>,
) -> Result<(), ArtifactPathError> {
    let mut complete = BTreeSet::new();
    for generation in manifests.keys() {
        let mut chain = BTreeSet::new();
        let mut cursor = generation.as_str();
        while !complete.contains(cursor) {
            if !chain.insert(cursor.to_owned()) {
                return Err(ArtifactPathError::GenerationChainCycle {
                    generation: cursor.to_owned(),
                });
            }
            if retention_boundaries.contains(cursor) {
                break;
            }
            let Some(predecessor) = manifests
                .get(cursor)
                .and_then(|(_, manifest)| manifest.predecessor_generation.as_deref())
            else {
                break;
            };
            cursor = predecessor;
        }
        complete.extend(chain);
    }
    Ok(())
}

fn load_current_snapshot(
    root: &ApprovedRoot,
    allow_missing: bool,
) -> Result<Option<LatestSnapshot>, ArtifactPathError> {
    let latest_path = root.path().join(LATEST_DIRECTORY);
    match fs::symlink_metadata(&latest_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(ArtifactPathError::SymlinkComponent { path: latest_path });
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(ArtifactPathError::DirectoryComponentNotDirectory { path: latest_path });
        }
        Ok(_) => {}
        Err(source) if source.kind() == std::io::ErrorKind::NotFound && allow_missing => {
            return Ok(None);
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(ArtifactPathError::LatestSnapshotUnavailable { path: latest_path });
        }
        Err(source) => {
            return Err(ArtifactPathError::Io {
                operation: "inspect latest snapshot root",
                path: latest_path,
                source,
            });
        }
    }
    let latest_root = ApprovedRoot::existing(&latest_path)?;
    let current = latest_root.path().join(LATEST_CURRENT_FILE);
    let metadata = match fs::symlink_metadata(&current) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(ArtifactPathError::SymlinkComponent { path: current });
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(ArtifactPathError::FileDestinationNotFile { path: current });
        }
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound && allow_missing => {
            return Ok(None);
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(ArtifactPathError::LatestSnapshotUnavailable { path: current });
        }
        Err(source) => {
            return Err(ArtifactPathError::Io {
                operation: "inspect latest snapshot pointer",
                path: current,
                source,
            });
        }
    };
    if metadata.len() > 512 {
        return Err(ArtifactPathError::InvalidLatestPointer { path: current });
    }
    let generation = fs::read_to_string(&current)
        .map_err(|source| ArtifactPathError::Io {
            operation: "read latest snapshot pointer",
            path: current.clone(),
            source,
        })?
        .trim()
        .to_owned();
    ArtifactId::new(generation.clone()).map_err(|_| ArtifactPathError::InvalidLatestPointer {
        path: current.clone(),
    })?;

    let generation_path = latest_root
        .path()
        .join(LATEST_GENERATIONS_DIRECTORY)
        .join(&generation);
    match fs::symlink_metadata(&generation_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(ArtifactPathError::SymlinkComponent {
                path: generation_path,
            });
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(ArtifactPathError::DirectoryComponentNotDirectory {
                path: generation_path,
            });
        }
        Ok(_) => {}
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(ArtifactPathError::InvalidLatestPointer { path: current });
        }
        Err(source) => {
            return Err(ArtifactPathError::Io {
                operation: "inspect latest generation",
                path: generation_path,
                source,
            });
        }
    }
    let manifest = validate_latest_manifest(&generation_path, &generation)?;
    validate_predecessor_chain(&latest_root, &manifest)?;
    Ok(Some(make_latest_snapshot(
        root.clone(),
        generation_path,
        manifest,
    )?))
}

fn make_latest_snapshot(
    root: ApprovedRoot,
    generation_path: PathBuf,
    manifest: LatestManifest,
) -> Result<LatestSnapshot, ArtifactPathError> {
    let lease_path = generation_path.join(LATEST_READER_LOCK_FILE);
    let reader_lease = Arc::new(OpenOptions::new().read(true).open(&lease_path).map_err(
        |source| ArtifactPathError::Io {
            operation: "open latest-generation reader lease",
            path: lease_path.clone(),
            source,
        },
    )?);
    FileExt::lock_shared(reader_lease.as_ref()).map_err(|source| ArtifactPathError::Io {
        operation: "lock latest-generation reader lease",
        path: lease_path,
        source,
    })?;
    let source_run_lease = open_shared_lease(
        &root
            .path()
            .join(&manifest.source_publication_id)
            .join(RUN_READER_LOCK_FILE),
        "open latest source-run reader lease",
        "lock latest source-run reader lease",
    )?;
    #[cfg(unix)]
    let generation_directory = Arc::new(open_directory_no_follow(&generation_path).map_err(
        |source| ArtifactPathError::Io {
            operation: "pin latest generation directory",
            path: generation_path.clone(),
            source,
        },
    )?);
    Ok(LatestSnapshot {
        root,
        generation_path,
        manifest,
        _reader_lease: reader_lease,
        _source_run_lease: source_run_lease,
        #[cfg(unix)]
        generation_directory,
    })
}

fn open_shared_lease(
    path: &Path,
    open_operation: &'static str,
    lock_operation: &'static str,
) -> Result<Arc<File>, ArtifactPathError> {
    let lease = Arc::new(OpenOptions::new().read(true).open(path).map_err(|source| {
        ArtifactPathError::Io {
            operation: open_operation,
            path: path.to_path_buf(),
            source,
        }
    })?);
    FileExt::lock_shared(lease.as_ref()).map_err(|source| ArtifactPathError::Io {
        operation: lock_operation,
        path: path.to_path_buf(),
        source,
    })?;
    Ok(lease)
}

fn create_run_reader_lease_file(directory: &Path) -> Result<(), ArtifactPathError> {
    create_lease_file(
        directory,
        RUN_READER_LOCK_FILE,
        "create published-run reader lease",
    )
}

fn create_reader_lease_file(directory: &Path) -> Result<(), ArtifactPathError> {
    create_lease_file(
        directory,
        LATEST_READER_LOCK_FILE,
        "create latest-generation reader lease",
    )
}

fn create_lease_file(
    directory: &Path,
    name: &str,
    operation: &'static str,
) -> Result<(), ArtifactPathError> {
    let path = directory.join(name);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|source| ArtifactPathError::Io {
            operation,
            path: path.clone(),
            source,
        })?;
    file.sync_all().map_err(|source| ArtifactPathError::Io {
        operation: "sync latest-generation reader lease",
        path,
        source,
    })
}

fn prune_latest_generations(
    latest_root: &ApprovedRoot,
    current: &LatestManifest,
) -> Result<(), ArtifactPathError> {
    let generations_root = latest_root.path().join(LATEST_GENERATIONS_DIRECTORY);
    let mut chain = Vec::new();
    let mut cursor = current.clone();
    loop {
        let path = generations_root.join(&cursor.generation);
        let boundary = load_retention_boundary(&path, &cursor)?;
        chain.push((path, cursor.clone()));
        if boundary.is_some() {
            break;
        }
        let Some(predecessor) = cursor.predecessor_generation.as_ref() else {
            break;
        };
        let predecessor_path = generations_root.join(predecessor);
        cursor = validate_latest_manifest(&predecessor_path, predecessor)?;
    }
    if chain.len() <= RETAIN_LATEST_GENERATIONS {
        return Ok(());
    }

    // Retention is lease-aware and all-or-nothing. Keeping the complete tail
    // temporarily is preferable to stranding a leased generation outside the
    // retained chain.
    let mut leases = Vec::new();
    for (path, _) in &chain[RETAIN_LATEST_GENERATIONS..] {
        let lease_path = path.join(LATEST_READER_LOCK_FILE);
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lease_path)
            .map_err(|source| ArtifactPathError::Io {
                operation: "open latest-generation retention lease",
                path: lease_path.clone(),
                source,
            })?;
        match FileExt::try_lock_exclusive(&lease) {
            Ok(()) => leases.push(lease),
            Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(source) => {
                return Err(ArtifactPathError::Io {
                    operation: "lock latest generation for retention",
                    path: lease_path,
                    source,
                });
            }
        }
    }

    let (oldest_retained_path, oldest_retained) = &chain[RETAIN_LATEST_GENERATIONS - 1];
    let predecessor = oldest_retained
        .predecessor_generation
        .clone()
        .expect("a prunable chain has a predecessor after the retention boundary");
    let boundary = RetentionBoundary {
        format_version: 1,
        predecessor_generation: predecessor,
    };
    let boundary_path = oldest_retained_path.join(LATEST_RETENTION_BOUNDARY_FILE);
    let boundary_temp =
        oldest_retained_path.join(format!(".retention-boundary-{}", workspace_nonce()));
    write_json_file(
        &boundary_temp,
        &boundary,
        "write latest-generation retention boundary",
    )?;
    replace_file_atomically(&boundary_temp, &boundary_path).map_err(|source| {
        ArtifactPathError::Io {
            operation: "commit latest-generation retention boundary",
            path: boundary_path.clone(),
            source,
        }
    })?;
    sync_directory(oldest_retained_path)?;

    let quarantine = latest_root.prepare_dir(LATEST_QUARANTINE_DIRECTORY)?;
    let mut retired = Vec::new();
    for (path, _) in &chain[RETAIN_LATEST_GENERATIONS..] {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("generation");
        let destination = quarantine.join(format!("retired-{name}--{}", workspace_nonce()));
        fs::rename(path, &destination).map_err(|source| ArtifactPathError::Io {
            operation: "retire latest generation",
            path: path.clone(),
            source,
        })?;
        retired.push(destination);
    }
    sync_directory(&generations_root)?;
    sync_directory(&quarantine)?;
    drop(leases);

    for path in retired {
        fs::remove_dir_all(&path).map_err(|source| ArtifactPathError::Io {
            operation: "delete retired latest generation",
            path,
            source,
        })?;
    }
    sync_directory(&quarantine)
}

fn prune_published_runs(root: &ApprovedRoot) -> Result<(), ArtifactPathError> {
    let mut runs = Vec::new();
    for entry in fs::read_dir(root.path()).map_err(|source| ArtifactPathError::Io {
        operation: "list published runs for retention",
        path: root.path().to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| ArtifactPathError::Io {
            operation: "inspect published run for retention",
            path: root.path().to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(|source| ArtifactPathError::Io {
            operation: "inspect published run type for retention",
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        if !path.join(RUN_MANIFEST_FILE).is_file() || !path.join(RUN_READER_LOCK_FILE).is_file() {
            continue;
        }
        // Never delete a directory merely because its name resembles a run.
        // Full manifest verification proves Mobench owns the complete tree.
        if validate_run_manifest(&path).is_err() {
            continue;
        }
        let order_key = name
            .rsplit_once("--")
            .map(|(_, nonce)| nonce.to_owned())
            .unwrap_or_else(|| name.clone());
        runs.push((order_key, path));
    }
    runs.sort_by(|left, right| right.0.cmp(&left.0));
    if runs.len() <= RETAIN_PUBLISHED_RUNS {
        return Ok(());
    }

    let quarantine = root.prepare_dir(RUN_RETENTION_QUARANTINE_DIRECTORY)?;
    for (_, path) in &runs[RETAIN_PUBLISHED_RUNS..] {
        let lease_path = path.join(RUN_READER_LOCK_FILE);
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lease_path)
            .map_err(|source| ArtifactPathError::Io {
                operation: "open published-run retention lease",
                path: lease_path.clone(),
                source,
            })?;
        match FileExt::try_lock_exclusive(&lease) {
            Ok(()) => {}
            Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(source) => {
                return Err(ArtifactPathError::Io {
                    operation: "lock published run for retention",
                    path: lease_path,
                    source,
                });
            }
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("run");
        let retired = quarantine.join(format!("retired-{name}--{}", workspace_nonce()));
        fs::rename(path, &retired).map_err(|source| ArtifactPathError::Io {
            operation: "retire published run",
            path: path.clone(),
            source,
        })?;
        drop(lease);
        fs::remove_dir_all(&retired).map_err(|source| ArtifactPathError::Io {
            operation: "delete retired published run",
            path: retired,
            source,
        })?;
    }
    sync_directory(root.path())?;
    sync_directory(&quarantine)?;
    prune_quarantine_entries(&quarantine, RETAIN_QUARANTINE_ENTRIES)
}

fn prune_quarantine_entries(directory: &Path, retain: usize) -> Result<(), ArtifactPathError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|source| ArtifactPathError::Io {
            operation: "list artifact quarantine for retention",
            path: directory.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ArtifactPathError::Io {
            operation: "inspect artifact quarantine for retention",
            path: directory.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.file_name()));
    for entry in entries.into_iter().skip(retain) {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|source| ArtifactPathError::Io {
            operation: "inspect quarantined artifact for deletion",
            path: path.clone(),
            source,
        })?;
        let result = if metadata.file_type().is_symlink() || metadata.is_file() {
            fs::remove_file(&path)
        } else if metadata.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        result.map_err(|source| ArtifactPathError::Io {
            operation: "delete expired quarantined artifact",
            path,
            source,
        })?;
    }
    sync_directory(directory)
}

fn validate_predecessor_chain(
    latest_root: &ApprovedRoot,
    tip: &LatestManifest,
) -> Result<(), ArtifactPathError> {
    let mut visited = BTreeSet::new();
    let mut child = tip.clone();
    loop {
        if !visited.insert(child.generation.clone()) {
            return Err(ArtifactPathError::GenerationChainCycle {
                generation: child.generation,
            });
        }
        let Some(predecessor) = child.predecessor_generation.as_ref() else {
            return Ok(());
        };
        let child_path = latest_root
            .path()
            .join(LATEST_GENERATIONS_DIRECTORY)
            .join(&child.generation);
        if load_retention_boundary(&child_path, &child)?.is_some() {
            return Ok(());
        }
        let predecessor_path = latest_root
            .path()
            .join(LATEST_GENERATIONS_DIRECTORY)
            .join(predecessor);
        let predecessor_manifest = match fs::symlink_metadata(&predecessor_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ArtifactPathError::SymlinkComponent {
                    path: predecessor_path,
                });
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(ArtifactPathError::DirectoryComponentNotDirectory {
                    path: predecessor_path,
                });
            }
            Ok(_) => validate_latest_manifest(&predecessor_path, predecessor)?,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Err(ArtifactPathError::BrokenGenerationChain {
                    generation: child.generation,
                    predecessor: predecessor.clone(),
                });
            }
            Err(source) => {
                return Err(ArtifactPathError::Io {
                    operation: "inspect predecessor generation",
                    path: predecessor_path,
                    source,
                });
            }
        };
        reject_chain_downgrade(&predecessor_manifest, &child)?;
        child = predecessor_manifest;
    }
}

fn load_retention_boundary(
    generation_path: &Path,
    manifest: &LatestManifest,
) -> Result<Option<RetentionBoundary>, ArtifactPathError> {
    let path = generation_path.join(LATEST_RETENTION_BOUNDARY_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ArtifactPathError::Io {
                operation: "inspect latest-generation retention boundary",
                path,
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(ArtifactPathError::SymlinkComponent { path });
    }
    if !metadata.is_file() {
        return Err(ArtifactPathError::FileDestinationNotFile { path });
    }
    let body = fs::read(&path).map_err(|source| ArtifactPathError::Io {
        operation: "read latest-generation retention boundary",
        path: path.clone(),
        source,
    })?;
    let boundary: RetentionBoundary =
        serde_json::from_slice(&body).map_err(|source| ArtifactPathError::ManifestJson {
            path: path.clone(),
            source,
        })?;
    if boundary.format_version != 1
        || manifest.predecessor_generation.as_deref()
            != Some(boundary.predecessor_generation.as_str())
    {
        return Err(ArtifactPathError::CorruptGeneration {
            path,
            detail: "retention boundary does not match the manifest predecessor".to_owned(),
        });
    }
    Ok(Some(boundary))
}

fn reject_chain_downgrade(
    predecessor: &LatestManifest,
    child: &LatestManifest,
) -> Result<(), ArtifactPathError> {
    if parse_producer_version(&predecessor.producer_version)?
        > parse_producer_version(&child.producer_version)?
    {
        return Err(ArtifactPathError::ProducerDowngrade {
            existing: predecessor.producer_version.clone(),
            attempted: child.producer_version.clone(),
        });
    }
    Ok(())
}

fn reject_producer_downgrade(existing: &str) -> Result<(), ArtifactPathError> {
    let existing_version = parse_producer_version(existing)?;
    let producer_version = parse_producer_version(env!("CARGO_PKG_VERSION"))?;
    if existing_version > producer_version {
        return Err(ArtifactPathError::ProducerDowngrade {
            existing: existing.to_owned(),
            attempted: env!("CARGO_PKG_VERSION").to_owned(),
        });
    }
    Ok(())
}

fn install_stable_aliases_transactional(
    root: &ApprovedRoot,
    latest_root: &ApprovedRoot,
    snapshot: &LatestSnapshot,
) -> Result<StableAliasTransaction, ArtifactPathError> {
    let current_aliases: BTreeSet<ArtifactId> = match load_current_snapshot(root, true) {
        Ok(Some(current)) => current
            .manifest
            .artifacts
            .iter()
            .map(|artifact| ArtifactId::new(artifact.destination_relative_path.clone()))
            .collect::<Result<_, _>>()?,
        Ok(None) => BTreeSet::new(),
        Err(error) if is_recoverable_pointer_error(&error) => BTreeSet::new(),
        Err(error) => return Err(error),
    };
    let next_aliases: BTreeSet<ArtifactId> = snapshot
        .manifest
        .artifacts
        .iter()
        .map(|artifact| ArtifactId::new(artifact.destination_relative_path.clone()))
        .collect::<Result<_, _>>()?;

    let staging_root = latest_root.prepare_dir(LATEST_STAGING_DIRECTORY)?;
    let transaction_path = staging_root.join(format!("aliases-{}", workspace_nonce()));
    fs::create_dir(&transaction_path).map_err(|source| ArtifactPathError::Io {
        operation: "create stable-alias transaction",
        path: transaction_path.clone(),
        source,
    })?;
    let transaction_root = ApprovedRoot::existing(&transaction_path)?;
    let new_root = transaction_root.prepare_dir("new")?;
    let backup_root = transaction_root.prepare_dir("backup")?;

    let mut prepared = Vec::with_capacity(snapshot.manifest.artifacts.len());
    for artifact in &snapshot.manifest.artifacts {
        let id = ArtifactId::new(artifact.destination_relative_path.clone())?;
        let source = snapshot.generation_path.join(id.as_str());
        let destination = root.prepare_file(id.as_str())?;
        let staged = new_root.join(id.as_str());
        if let Err(error) = copy_new_synced(&source, &staged, "stage stable latest alias") {
            let _ = fs::remove_dir_all(&transaction_path);
            return Err(error);
        }
        prepared.push((staged, destination, id));
    }
    sync_artifact_tree(&transaction_path)?;

    let mut transaction = StableAliasTransaction {
        path: transaction_path,
        root_path: root.path().to_path_buf(),
        installed: Vec::new(),
        backups: Vec::new(),
    };

    // Aliases owned by the previous committed generation but omitted by the
    // candidate must disappear in the same rollback-capable transaction. Their
    // backups restore the old alias set if the pointer does not commit.
    for id in current_aliases.difference(&next_aliases) {
        let destination = root.prepare_file(id.as_str())?;
        match fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let original = ArtifactPathError::SymlinkComponent { path: destination };
                return Err(transaction.rollback(original));
            }
            Ok(metadata) if !metadata.is_file() => {
                let original = ArtifactPathError::FileDestinationNotFile { path: destination };
                return Err(transaction.rollback(original));
            }
            Ok(_) => {
                let backup = backup_root.join(id.as_str());
                if let Err(source) = fs::rename(&destination, &backup) {
                    let original = ArtifactPathError::Io {
                        operation: "retire obsolete stable latest alias",
                        path: destination,
                        source,
                    };
                    return Err(transaction.rollback(original));
                }
                transaction.backups.push((destination, backup));
                if let Err(error) =
                    sync_directory(root.path()).and_then(|()| sync_directory(&backup_root))
                {
                    return Err(transaction.rollback(error));
                }
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                let original = ArtifactPathError::Io {
                    operation: "inspect obsolete stable latest alias",
                    path: destination,
                    source,
                };
                return Err(transaction.rollback(original));
            }
        }
    }

    for (staged, destination, id) in prepared {
        match fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let original = ArtifactPathError::SymlinkComponent { path: destination };
                return Err(transaction.rollback(original));
            }
            Ok(metadata) if !metadata.is_file() => {
                let original = ArtifactPathError::FileDestinationNotFile { path: destination };
                return Err(transaction.rollback(original));
            }
            Ok(_) => {
                let backup = backup_root.join(id.as_str());
                if let Err(source) = fs::rename(&destination, &backup) {
                    let original = ArtifactPathError::Io {
                        operation: "back up stable latest alias",
                        path: destination,
                        source,
                    };
                    return Err(transaction.rollback(original));
                }
                transaction.backups.push((destination.clone(), backup));
                if let Err(error) =
                    sync_directory(root.path()).and_then(|()| sync_directory(&backup_root))
                {
                    return Err(transaction.rollback(error));
                }
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                let original = ArtifactPathError::Io {
                    operation: "inspect stable latest alias",
                    path: destination,
                    source,
                };
                return Err(transaction.rollback(original));
            }
        }

        if let Err(original) = maybe_fail_alias_install(transaction.installed.len(), &destination) {
            return Err(transaction.rollback(original));
        }
        if let Err(source) = fs::rename(&staged, &destination) {
            let original = ArtifactPathError::Io {
                operation: "install stable latest alias",
                path: destination,
                source,
            };
            return Err(transaction.rollback(original));
        }
        transaction.installed.push(destination);
        if let Err(error) = sync_directory(root.path()) {
            return Err(transaction.rollback(error));
        }
    }
    record_durability_event("install_aliases");
    Ok(transaction)
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
    recovery_path: PathBuf,
) -> ArtifactPathError {
    let remove_error = remove_installed_latest(installed).err();
    let restore_error = restore_latest_backups(backups).err();
    let rollback = remove_error.or(restore_error);

    match rollback {
        Some(rollback) => ArtifactPathError::LatestRollback {
            original: Box::new(original),
            rollback: Box::new(rollback),
            recovery_path,
        },
        None => original,
    }
}

fn validate_run_manifest(run_path: &Path) -> Result<RunManifest, ArtifactPathError> {
    let manifest_path = run_path.join(RUN_MANIFEST_FILE);
    let manifest: RunManifest = read_json_file(&manifest_path, "read run manifest")?;
    if manifest.format_version != RUN_MANIFEST_VERSION {
        return Err(ArtifactPathError::UnknownManifestVersion {
            path: manifest_path,
            found: manifest.format_version,
            supported: RUN_MANIFEST_VERSION,
        });
    }
    parse_producer_version(&manifest.producer_version)?;
    ArtifactId::new(manifest.logical_id.clone())?;
    ArtifactId::new(manifest.publication_id.clone())?;
    if let Some(expected) = manifest.expected_latest_generation.as_ref() {
        ArtifactId::new(expected.clone())?;
    }
    let expected_publication = run_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ArtifactPathError::NonUtf8ArtifactPath {
            path: run_path.to_path_buf(),
        })?;
    if manifest.publication_id != expected_publication {
        return Err(ArtifactPathError::ManifestIntegrity {
            path: manifest_path,
            detail: "publication identity does not match its directory".to_owned(),
        });
    }

    let actual = inspect_artifact_tree(run_path, &[RUN_MANIFEST_FILE, RUN_READER_LOCK_FILE])?;
    if manifest.artifacts != actual {
        return Err(ArtifactPathError::ManifestIntegrity {
            path: manifest_path,
            detail: "run artifact paths, sizes, or digests do not match".to_owned(),
        });
    }
    Ok(manifest)
}

fn validate_latest_manifest(
    generation_path: &Path,
    expected_generation: &str,
) -> Result<LatestManifest, ArtifactPathError> {
    let manifest_path = generation_path.join(LATEST_MANIFEST_FILE);
    let manifest: LatestManifest = read_json_file(&manifest_path, "read latest manifest")?;
    if manifest.format_version != LATEST_MANIFEST_VERSION {
        return Err(ArtifactPathError::UnknownManifestVersion {
            path: manifest_path,
            found: manifest.format_version,
            supported: LATEST_MANIFEST_VERSION,
        });
    }
    parse_producer_version(&manifest.producer_version)?;
    ArtifactId::new(manifest.generation.clone())?;
    ArtifactId::new(manifest.source_logical_id.clone())?;
    ArtifactId::new(manifest.source_publication_id.clone())?;
    if let Some(predecessor) = manifest.predecessor_generation.as_ref() {
        ArtifactId::new(predecessor.clone())?;
    }
    if manifest.generation != expected_generation {
        return Err(ArtifactPathError::CorruptGeneration {
            path: generation_path.to_path_buf(),
            detail: "manifest generation does not match current pointer".to_owned(),
        });
    }

    let actual = inspect_artifact_tree(
        generation_path,
        &[
            LATEST_MANIFEST_FILE,
            LATEST_READER_LOCK_FILE,
            LATEST_RETENTION_BOUNDARY_FILE,
        ],
    )?;
    let mut expected = Vec::with_capacity(manifest.artifacts.len());
    for artifact in &manifest.artifacts {
        ArtifactId::new(artifact.source_relative_path.clone())?;
        ArtifactId::new(artifact.destination_relative_path.clone())?;
        expected.push(ManifestArtifact {
            relative_path: artifact.destination_relative_path.clone(),
            size: artifact.size,
            sha256: artifact.sha256.clone(),
        });
    }
    expected.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    if expected != actual {
        return Err(ArtifactPathError::CorruptGeneration {
            path: generation_path.to_path_buf(),
            detail: "generation artifact paths, sizes, or digests do not match".to_owned(),
        });
    }
    Ok(manifest)
}

fn parse_producer_version(version: &str) -> Result<Version, ArtifactPathError> {
    Version::parse(version).map_err(|source| ArtifactPathError::InvalidProducerVersion {
        version: version.to_owned(),
        source,
    })
}

fn read_json_file<T: for<'de> Deserialize<'de>>(
    path: &Path,
    operation: &'static str,
) -> Result<T, ArtifactPathError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| ArtifactPathError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ArtifactPathError::SymlinkComponent {
            path: path.to_path_buf(),
        });
    }
    if !metadata.is_file() {
        return Err(ArtifactPathError::FileDestinationNotFile {
            path: path.to_path_buf(),
        });
    }
    let file = File::open(path).map_err(|source| ArtifactPathError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_reader(BufReader::new(file)).map_err(|source| {
        ArtifactPathError::ManifestJson {
            path: path.to_path_buf(),
            source,
        }
    })
}

fn write_json_file<T: Serialize>(
    path: &Path,
    value: &T,
    operation: &'static str,
) -> Result<(), ArtifactPathError> {
    let mut encoded =
        serde_json::to_vec_pretty(value).map_err(|source| ArtifactPathError::ManifestJson {
            path: path.to_path_buf(),
            source,
        })?;
    encoded.push(b'\n');
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| ArtifactPathError::Io {
            operation,
            path: path.to_path_buf(),
            source,
        })?;
    file.write_all(&encoded)
        .map_err(|source| ArtifactPathError::Io {
            operation,
            path: path.to_path_buf(),
            source,
        })?;
    file.sync_all().map_err(|source| ArtifactPathError::Io {
        operation: "sync manifest",
        path: path.to_path_buf(),
        source,
    })?;
    record_durability_event("sync_manifest");
    Ok(())
}

fn inspect_artifact_tree(
    root: &Path,
    excluded_relative_paths: &[&str],
) -> Result<Vec<ManifestArtifact>, ArtifactPathError> {
    let mut artifacts = Vec::new();
    inspect_artifact_tree_recursive(root, root, excluded_relative_paths, &mut artifacts)?;
    artifacts.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(artifacts)
}

fn inspect_artifact_tree_recursive(
    root: &Path,
    directory: &Path,
    excluded_relative_paths: &[&str],
    artifacts: &mut Vec<ManifestArtifact>,
) -> Result<(), ArtifactPathError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|source| ArtifactPathError::Io {
            operation: "list artifact directory",
            path: directory.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ArtifactPathError::Io {
            operation: "inspect artifact directory entry",
            path: directory.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|source| ArtifactPathError::Io {
            operation: "inspect artifact tree entry",
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ArtifactPathError::SymlinkComponent { path });
        }
        if metadata.is_dir() {
            inspect_artifact_tree_recursive(root, &path, excluded_relative_paths, artifacts)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(ArtifactPathError::FileDestinationNotFile { path });
        }
        reject_hardlinked_regular_file(&metadata, &path)?;
        let relative_path = manifest_relative_path(root, &path)?;
        if excluded_relative_paths.contains(&relative_path.as_str()) {
            continue;
        }
        let (size, sha256) = digest_file(&path)?;
        artifacts.push(ManifestArtifact {
            relative_path,
            size,
            sha256,
        });
    }
    Ok(())
}

fn manifest_relative_path(root: &Path, path: &Path) -> Result<String, ArtifactPathError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| ArtifactPathError::InvalidRelativePath {
            path: path.to_path_buf(),
        })?;
    let mut components = Vec::new();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(ArtifactPathError::InvalidRelativePath {
                path: relative.to_path_buf(),
            });
        };
        components.push(component.to_str().ok_or_else(|| {
            ArtifactPathError::NonUtf8ArtifactPath {
                path: path.to_path_buf(),
            }
        })?);
    }
    Ok(components.join("/"))
}

fn digest_file(path: &Path) -> Result<(u64, String), ArtifactPathError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| ArtifactPathError::Io {
        operation: "inspect artifact for digest",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ArtifactPathError::SymlinkComponent {
            path: path.to_path_buf(),
        });
    }
    if !metadata.is_file() {
        return Err(ArtifactPathError::FileDestinationNotFile {
            path: path.to_path_buf(),
        });
    }
    reject_hardlinked_regular_file(&metadata, path)?;
    let mut file = File::open(path).map_err(|source| ArtifactPathError::Io {
        operation: "open artifact for digest",
        path: path.to_path_buf(),
        source,
    })?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| ArtifactPathError::Io {
                operation: "read artifact for digest",
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok((metadata.len(), format!("{:x}", digest.finalize())))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

#[cfg(unix)]
fn open_directory_no_follow(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

#[cfg(unix)]
fn open_snapshot_artifact(snapshot: &LatestSnapshot, id: &ArtifactId) -> std::io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};

    let name = CString::new(id.as_str())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "name contains NUL"))?;
    // SAFETY: `generation_directory` pins a real directory descriptor and the
    // validated artifact ID is exactly one child component. O_NOFOLLOW rejects
    // a raced symlink at the leaf.
    let fd = unsafe {
        libc::openat(
            snapshot.generation_directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `openat` returned a new owned descriptor.
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "snapshot artifact is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(not(unix))]
fn open_snapshot_artifact(snapshot: &LatestSnapshot, id: &ArtifactId) -> std::io::Result<File> {
    File::open(snapshot.generation_path.join(id.as_str()))
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn rename_directory_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "source contains NUL")
    })?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "destination contains NUL")
    })?;
    // SAFETY: both C strings remain alive for the duration of the call. The
    // platform flag provides an atomic no-replace directory rename.
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_directory_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "source contains NUL")
    })?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "destination contains NUL")
    })?;
    // SAFETY: the arguments are valid C strings and AT_FDCWD selects their
    // absolute paths. RENAME_NOREPLACE makes collision handling atomic.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android",
    windows
)))]
fn rename_directory_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    // These targets retain the preflight check but cannot promise an atomic
    // no-replace rename through Rust's portable API.
    fs::rename(source, destination)
}

#[cfg(windows)]
fn rename_directory_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // Without REPLACE_EXISTING, this is an atomic no-replace move. The
    // write-through flag requests durable completion before returning.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if result != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn replace_file_atomically(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: both UTF-16 buffers are NUL-terminated and alive for the call.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(windows))]
fn replace_file_atomically(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

fn copy_new_synced(
    source: &Path,
    destination: &Path,
    operation: &'static str,
) -> Result<(), ArtifactPathError> {
    let source_metadata =
        fs::symlink_metadata(source).map_err(|source_error| ArtifactPathError::Io {
            operation,
            path: source.to_path_buf(),
            source: source_error,
        })?;
    if source_metadata.file_type().is_symlink() {
        return Err(ArtifactPathError::SymlinkComponent {
            path: source.to_path_buf(),
        });
    }
    if !source_metadata.is_file() {
        return Err(ArtifactPathError::FileDestinationNotFile {
            path: source.to_path_buf(),
        });
    }
    reject_hardlinked_regular_file(&source_metadata, source)?;
    let mut source_file = File::open(source).map_err(|source_error| ArtifactPathError::Io {
        operation,
        path: source.to_path_buf(),
        source: source_error,
    })?;
    let mut destination_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|source_error| ArtifactPathError::Io {
            operation,
            path: destination.to_path_buf(),
            source: source_error,
        })?;
    std::io::copy(&mut source_file, &mut destination_file).map_err(|source_error| {
        ArtifactPathError::Io {
            operation,
            path: destination.to_path_buf(),
            source: source_error,
        }
    })?;
    destination_file
        .sync_all()
        .map_err(|source_error| ArtifactPathError::Io {
            operation: "sync copied artifact",
            path: destination.to_path_buf(),
            source: source_error,
        })?;
    record_durability_event("sync_file");
    Ok(())
}

fn sync_artifact_tree(root: &Path) -> Result<(), ArtifactPathError> {
    sync_artifact_tree_recursive(root)
}

#[cfg(unix)]
fn reject_hardlinked_regular_file(
    metadata: &fs::Metadata,
    path: &Path,
) -> Result<(), ArtifactPathError> {
    use std::os::unix::fs::MetadataExt;

    if metadata.nlink() > 1 {
        return Err(ArtifactPathError::HardLinkedArtifact {
            path: path.to_path_buf(),
            links: metadata.nlink(),
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn reject_hardlinked_regular_file(
    _metadata: &fs::Metadata,
    _path: &Path,
) -> Result<(), ArtifactPathError> {
    Ok(())
}

fn sync_artifact_tree_recursive(directory: &Path) -> Result<(), ArtifactPathError> {
    let entries = fs::read_dir(directory)
        .map_err(|source| ArtifactPathError::Io {
            operation: "list artifact tree for sync",
            path: directory.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ArtifactPathError::Io {
            operation: "inspect artifact tree entry for sync",
            path: directory.to_path_buf(),
            source,
        })?;
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|source| ArtifactPathError::Io {
            operation: "inspect artifact tree entry for sync",
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ArtifactPathError::SymlinkComponent { path });
        }
        if metadata.is_dir() {
            sync_artifact_tree_recursive(&path)?;
        } else if metadata.is_file() {
            File::open(&path)
                .and_then(|file| file.sync_all())
                .map_err(|source| ArtifactPathError::Io {
                    operation: "sync artifact file",
                    path: path.clone(),
                    source,
                })?;
            record_durability_event("sync_file");
        } else {
            return Err(ArtifactPathError::FileDestinationNotFile { path });
        }
    }
    sync_directory(directory)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), ArtifactPathError> {
    maybe_fail_directory_sync(path)?;
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| ArtifactPathError::Io {
            operation: "sync artifact directory",
            path: path.to_path_buf(),
            source,
        })?;
    record_durability_event("sync_directory");
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), ArtifactPathError> {
    maybe_fail_directory_sync(_path)?;
    // Rust's portable filesystem API does not expose a directory handle that
    // can be flushed on every non-Unix target. File contents are still synced;
    // power-loss durability of directory entries is therefore weaker there.
    record_durability_event("directory_sync_unavailable");
    Ok(())
}

#[cfg(test)]
thread_local! {
    static DURABILITY_EVENTS: std::cell::RefCell<Vec<&'static str>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    static FAIL_ALIAS_INSTALL_AFTER: std::cell::Cell<Option<usize>> = const {
        std::cell::Cell::new(None)
    };
    static MUTATE_LATEST_SOURCE_AFTER_VALIDATION: std::cell::RefCell<Option<(PathBuf, Vec<u8>)>> =
        const { std::cell::RefCell::new(None) };
    static FAIL_DIRECTORY_SYNC_AFTER_EVENT: std::cell::Cell<Option<&'static str>> =
        const { std::cell::Cell::new(None) };
    static FAIL_NEXT_DIRECTORY_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn record_durability_event(event: &'static str) {
    DURABILITY_EVENTS.with(|events| events.borrow_mut().push(event));
    FAIL_DIRECTORY_SYNC_AFTER_EVENT.with(|target| {
        if target.get() == Some(event) {
            target.set(None);
            FAIL_NEXT_DIRECTORY_SYNC.with(|fail| fail.set(true));
        }
    });
}

#[cfg(not(test))]
fn record_durability_event(_event: &'static str) {}

#[cfg(test)]
fn maybe_fail_directory_sync(path: &Path) -> Result<(), ArtifactPathError> {
    let fail = FAIL_NEXT_DIRECTORY_SYNC.with(|fail| fail.replace(false));
    if fail {
        return Err(ArtifactPathError::Io {
            operation: "sync artifact directory",
            path: path.to_path_buf(),
            source: std::io::Error::other("injected post-commit directory sync failure"),
        });
    }
    Ok(())
}

#[cfg(not(test))]
fn maybe_fail_directory_sync(_path: &Path) -> Result<(), ArtifactPathError> {
    Ok(())
}

#[cfg(test)]
fn maybe_fail_alias_install(installed: usize, path: &Path) -> Result<(), ArtifactPathError> {
    let should_fail = FAIL_ALIAS_INSTALL_AFTER.with(|fail_after| {
        let should_fail = fail_after.get() == Some(installed);
        if should_fail {
            fail_after.set(None);
        }
        should_fail
    });
    if should_fail {
        return Err(ArtifactPathError::Io {
            operation: "install stable latest alias",
            path: path.to_path_buf(),
            source: std::io::Error::other("injected stable-alias failure"),
        });
    }
    Ok(())
}

#[cfg(test)]
fn maybe_mutate_latest_source_after_validation() {
    MUTATE_LATEST_SOURCE_AFTER_VALIDATION.with(|mutation| {
        if let Some((path, contents)) = mutation.borrow_mut().take() {
            fs::write(path, contents).expect("apply injected post-validation source mutation");
        }
    });
}

#[cfg(not(test))]
fn maybe_mutate_latest_source_after_validation() {}

#[cfg(not(test))]
fn maybe_fail_alias_install(_installed: usize, _path: &Path) -> Result<(), ArtifactPathError> {
    Ok(())
}

fn active_workspaces() -> &'static Mutex<BTreeSet<PathBuf>> {
    ACTIVE_WORKSPACES.get_or_init(|| Mutex::new(BTreeSet::new()))
}

fn register_active_workspace(path: &Path) {
    active_workspaces()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(path.to_path_buf());
}

fn unregister_active_workspace(path: &Path) {
    active_workspaces()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(path);
}

fn is_active_workspace(path: &Path) -> bool {
    active_workspaces()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(path)
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
    #[error("staged run already contains the reserved manifest path: {path}")]
    ReservedManifestPath { path: PathBuf },
    #[error("required staged artifact is missing: {path}")]
    RequiredArtifactMissing { path: PathBuf },
    #[error("latest artifact destination was requested more than once: {id}")]
    DuplicateLatestDestination { id: ArtifactId },
    #[error("could not allocate a unique latest-artifact transaction")]
    LatestTransactionAllocationExhausted,
    #[error("timed out waiting for the latest-artifact update lock at {path}")]
    LatestLockTimeout { path: PathBuf },
    #[error("no committed latest snapshot is available at {path}")]
    LatestSnapshotUnavailable { path: PathBuf },
    #[error("latest pointer is not one valid generation identity: {path}")]
    InvalidLatestPointer { path: PathBuf },
    #[error("latest snapshot does not contain alias {id}")]
    LatestArtifactNotInSnapshot { id: ArtifactId },
    #[error("latest generation is corrupt at {path}: {detail}")]
    CorruptGeneration { path: PathBuf, detail: String },
    #[error("latest-generation chain is broken at {generation}: missing {predecessor}")]
    BrokenGenerationChain {
        generation: String,
        predecessor: String,
    },
    #[error("latest-generation chain contains a cycle at {generation}")]
    GenerationChainCycle { generation: String },
    #[error("latest-generation chain forks after {predecessor} into {children:?}")]
    AmbiguousGenerationFork {
        predecessor: String,
        children: Vec<String>,
    },
    #[error("latest-generation recovery has ambiguous tips: {tips:?}")]
    AmbiguousGenerationTips { tips: Vec<String> },
    #[error(
        "published run has stale latest-generation token: expected {expected:?}, observed {observed:?}"
    )]
    StaleLatestGeneration {
        expected: Option<String>,
        observed: Option<String>,
    },
    #[error("refusing latest-generation producer downgrade from {existing} to {attempted}")]
    ProducerDowngrade { existing: String, attempted: String },
    #[error("invalid artifact producer version {version:?}: {source}")]
    InvalidProducerVersion {
        version: String,
        #[source]
        source: semver::Error,
    },
    #[error("unsupported manifest version {found} at {path}; supported version is {supported}")]
    UnknownManifestVersion {
        path: PathBuf,
        found: u32,
        supported: u32,
    },
    #[error("artifact manifest integrity check failed at {path}: {detail}")]
    ManifestIntegrity { path: PathBuf, detail: String },
    #[error("could not encode or decode artifact manifest at {path}: {source}")]
    ManifestJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "latest-artifact update failed ({original}) and rollback was incomplete ({rollback}); recovery material retained at {recovery_path}"
    )]
    LatestRollback {
        original: Box<ArtifactPathError>,
        rollback: Box<ArtifactPathError>,
        /// Transaction directory retained with any unrestored backups.
        recovery_path: PathBuf,
    },
    #[error(
        "latest-generation preparation failed ({original}) and staging cleanup failed ({cleanup}) at {staging_path}"
    )]
    LatestStagingCleanup {
        original: Box<ArtifactPathError>,
        cleanup: Box<ArtifactPathError>,
        staging_path: PathBuf,
    },
    #[error("run is committed at {path}, but directory durability is uncertain: {source}")]
    PublicationDurabilityUncertain {
        path: PathBuf,
        #[source]
        source: Box<ArtifactPathError>,
    },
    #[error("run is committed at {path}, but post-commit retention maintenance failed: {source}")]
    PublicationPostCommitMaintenance {
        path: PathBuf,
        #[source]
        source: Box<ArtifactPathError>,
    },
    #[error(
        "latest generation {generation} is committed, but pointer durability is uncertain: {source}"
    )]
    LatestCommitDurabilityUncertain {
        generation: String,
        #[source]
        source: Box<ArtifactPathError>,
    },
    #[error("latest generation {generation} is committed, but retention failed: {source}")]
    LatestRetentionAfterCommit {
        generation: String,
        #[source]
        source: Box<ArtifactPathError>,
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
    #[error("artifact file has {links} hard links and is not publication-private: {path}")]
    HardLinkedArtifact { path: PathBuf, links: u64 },
    #[error("artifact path is not valid UTF-8 and cannot be recorded in a manifest: {path:?}")]
    NonUtf8ArtifactPath { path: PathBuf },
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
    fn latest_refresh_removes_aliases_omitted_by_the_next_generation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = publish_pair(temp.path(), "first-run", "first");
        refresh_pair(&first).expect("refresh initial aliases");
        assert!(temp.path().join("summary.md").is_file());

        let logical_id = ArtifactId::new("profile-only-run").expect("logical ID");
        let workspace = RunWorkspace::allocate(temp.path(), &logical_id).expect("workspace");
        fs::write(workspace.staging_path().join("profile.json"), "second").expect("write profile");
        let profile = ArtifactId::new("profile.json").expect("profile ID");
        let published = workspace
            .publish(std::slice::from_ref(&profile))
            .expect("publish profile-only run");
        published
            .refresh_latest(&[LatestArtifact::same(profile)])
            .expect("refresh profile-only aliases");

        assert_eq!(
            fs::read_to_string(temp.path().join("profile.json")).expect("profile alias"),
            "second"
        );
        assert!(
            !temp.path().join("summary.md").exists(),
            "obsolete alias survived the generation change"
        );
    }

    #[test]
    fn latest_generation_retention_keeps_a_bounded_valid_chain() {
        let temp = tempfile::tempdir().expect("tempdir");
        for index in 0..(RETAIN_LATEST_GENERATIONS + 4) {
            let published = publish_pair(
                temp.path(),
                &format!("retained-run-{index}"),
                &format!("receipt-{index}"),
            );
            refresh_pair(&published).expect("refresh retained generation");
        }

        let generations = temp
            .path()
            .join(LATEST_DIRECTORY)
            .join(LATEST_GENERATIONS_DIRECTORY);
        assert_eq!(
            fs::read_dir(&generations)
                .expect("list retained generations")
                .count(),
            RETAIN_LATEST_GENERATIONS
        );
        let snapshot = LatestSnapshot::open(temp.path()).expect("open retained latest snapshot");
        assert_eq!(
            snapshot
                .read_artifact(&ArtifactId::new("profile.json").expect("profile ID"))
                .expect("read retained latest artifact"),
            format!("receipt-{}", RETAIN_LATEST_GENERATIONS + 3).as_bytes()
        );
        validate_predecessor_chain(
            &ApprovedRoot::existing(temp.path().join(LATEST_DIRECTORY)).expect("open latest root"),
            snapshot.manifest(),
        )
        .expect("retained predecessor chain remains valid");
    }

    #[test]
    fn latest_generation_retention_defers_while_an_old_reader_is_leased() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = publish_pair(temp.path(), "leased-run-0", "receipt-0");
        refresh_pair(&first).expect("refresh first generation");
        let old_reader = LatestSnapshot::open(temp.path()).expect("lease first generation");

        for index in 1..=RETAIN_LATEST_GENERATIONS {
            let published = publish_pair(
                temp.path(),
                &format!("leased-run-{index}"),
                &format!("receipt-{index}"),
            );
            refresh_pair(&published).expect("refresh while old reader is leased");
        }
        let generations = temp
            .path()
            .join(LATEST_DIRECTORY)
            .join(LATEST_GENERATIONS_DIRECTORY);
        assert_eq!(
            fs::read_dir(&generations)
                .expect("list deferred generations")
                .count(),
            RETAIN_LATEST_GENERATIONS + 1
        );
        assert_eq!(
            old_reader
                .read_artifact(&ArtifactId::new("profile.json").expect("profile ID"))
                .expect("read leased generation"),
            b"receipt-0"
        );
        drop(old_reader);

        let final_run = publish_pair(temp.path(), "leased-run-final", "receipt-final");
        refresh_pair(&final_run).expect("refresh after releasing old reader");
        assert_eq!(
            fs::read_dir(&generations)
                .expect("list pruned generations")
                .count(),
            RETAIN_LATEST_GENERATIONS
        );
    }

    #[test]
    fn published_run_retention_is_bounded_and_defers_leased_runs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let leased = publish_pair(temp.path(), "published-run-0", "receipt-0");
        for index in 1..=RETAIN_PUBLISHED_RUNS {
            drop(publish_pair(
                temp.path(),
                &format!("published-run-{index}"),
                &format!("receipt-{index}"),
            ));
        }

        let count_runs = || {
            fs::read_dir(temp.path())
                .expect("list artifact root")
                .filter_map(Result::ok)
                .filter(|entry| entry.path().join(RUN_MANIFEST_FILE).is_file())
                .count()
        };
        assert_eq!(count_runs(), RETAIN_PUBLISHED_RUNS + 1);
        assert_eq!(
            fs::read_to_string(leased.path().join("profile.json")).expect("read leased run"),
            "receipt-0"
        );
        drop(leased);

        drop(publish_pair(
            temp.path(),
            "published-run-final",
            "receipt-final",
        ));
        assert_eq!(count_runs(), RETAIN_PUBLISHED_RUNS);
    }

    #[test]
    fn abandoned_workspace_quarantine_is_bounded() {
        let temp = tempfile::tempdir().expect("tempdir");
        let staging = temp.path().join(STAGING_DIRECTORY);
        fs::create_dir_all(&staging).expect("create staging root");
        for index in 0..(RETAIN_QUARANTINE_ENTRIES + 5) {
            let crashed = staging.join(format!("crashed-{index}"));
            fs::create_dir(&crashed).expect("create crashed workspace");
            fs::write(crashed.join(WORKSPACE_LOCK_FILE), "").expect("write released lock");
        }

        let workspace = RunWorkspace::allocate(
            temp.path(),
            &ArtifactId::new("quarantine-writer").expect("logical ID"),
        )
        .expect("recover abandoned workspaces");
        assert_eq!(
            fs::read_dir(temp.path().join(STAGING_QUARANTINE_DIRECTORY))
                .expect("list bounded quarantine")
                .count(),
            RETAIN_QUARANTINE_ENTRIES
        );
        drop(workspace);
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

        let mut committed = 0;
        let mut rejected_as_stale = 0;
        for handle in handles {
            match handle.join().expect("latest refresh thread") {
                Ok(()) => committed += 1,
                Err(ArtifactPathError::StaleLatestGeneration { .. }) => rejected_as_stale += 1,
                Err(error) => panic!("unexpected latest refresh error: {error}"),
            }
        }
        assert_eq!(committed, 1, "exactly one shared CAS token may commit");
        assert_eq!(rejected_as_stale, WRITERS - 1);

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
            temp.path().join("retained-rollback"),
        );

        assert!(matches!(error, ArtifactPathError::LatestRollback { .. }));
        assert_eq!(
            fs::read_to_string(&restorable_destination).expect("read restored backup"),
            "old restored"
        );
        assert!(blocked_backup.is_file());
    }

    #[test]
    fn incomplete_transaction_rollback_retains_backup_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let transaction_path = temp.path().join("alias-transaction");
        let backup_root = transaction_path.join("backup");
        fs::create_dir_all(&backup_root).expect("create transaction backup root");
        let destination = temp.path().join("profile.json");
        fs::create_dir(&destination).expect("create blocking destination directory");
        let backup = backup_root.join("profile.json");
        fs::write(&backup, "previous").expect("write backup");
        let transaction = StableAliasTransaction {
            path: transaction_path.clone(),
            root_path: temp.path().to_path_buf(),
            installed: vec![destination.clone()],
            backups: vec![(destination, backup.clone())],
        };
        let original = ArtifactPathError::Io {
            operation: "install stable latest alias",
            path: temp.path().join("profile.json"),
            source: std::io::Error::other("injected install failure"),
        };

        let error = transaction.rollback(original);
        assert!(matches!(
            error,
            ArtifactPathError::LatestRollback {
                ref recovery_path,
                ..
            } if recovery_path == &transaction_path
        ));
        assert!(transaction_path.is_dir());
        assert_eq!(
            fs::read_to_string(backup).expect("read retained backup"),
            "previous"
        );
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

    fn publish_pair(root: &Path, logical: &str, receipt: &str) -> PublishedRun {
        let logical_id = ArtifactId::new(logical).expect("valid logical ID");
        let workspace = RunWorkspace::allocate(root, &logical_id).expect("allocate workspace");
        fs::write(workspace.staging_path().join("profile.json"), receipt).expect("write profile");
        fs::write(workspace.staging_path().join("summary.md"), receipt).expect("write summary");
        workspace
            .publish(&[
                ArtifactId::new("profile.json").expect("profile ID"),
                ArtifactId::new("summary.md").expect("summary ID"),
            ])
            .expect("publish pair")
    }

    fn refresh_pair(published: &PublishedRun) -> Result<(), ArtifactPathError> {
        published.refresh_latest(&[
            LatestArtifact::same(ArtifactId::new("profile.json").expect("profile ID")),
            LatestArtifact::same(ArtifactId::new("summary.md").expect("summary ID")),
        ])
    }

    fn durability_events() -> Vec<&'static str> {
        DURABILITY_EVENTS.with(|events| std::mem::take(&mut *events.borrow_mut()))
    }

    fn rewrite_latest_manifest(generation_path: &Path, update: impl FnOnce(&mut LatestManifest)) {
        let manifest_path = generation_path.join(LATEST_MANIFEST_FILE);
        let mut manifest: LatestManifest =
            read_json_file(&manifest_path, "read test manifest").expect("read manifest");
        update(&mut manifest);
        fs::remove_file(&manifest_path).expect("remove old manifest");
        write_json_file(&manifest_path, &manifest, "rewrite test manifest")
            .expect("rewrite manifest");
    }

    fn clone_generation(
        source: &LatestSnapshot,
        generation: &str,
        predecessor: Option<&str>,
    ) -> PathBuf {
        let generations = source
            .path()
            .parent()
            .expect("generation parent")
            .to_path_buf();
        let destination = generations.join(generation);
        fs::create_dir(&destination).expect("create cloned generation");
        for artifact in &source.manifest().artifacts {
            fs::copy(
                source.path().join(&artifact.destination_relative_path),
                destination.join(&artifact.destination_relative_path),
            )
            .expect("copy generation artifact");
        }
        create_reader_lease_file(&destination).expect("create cloned generation lease");
        let mut manifest = source.manifest().clone();
        manifest.generation = generation.to_owned();
        manifest.predecessor_generation = predecessor.map(str::to_owned);
        write_json_file(
            &destination.join(LATEST_MANIFEST_FILE),
            &manifest,
            "write cloned generation manifest",
        )
        .expect("write cloned manifest");
        destination
    }

    #[test]
    fn writer_startup_quarantines_an_unlocked_crash_workspace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let staging = temp.path().join(STAGING_DIRECTORY);
        let crashed = staging.join("crashed-workspace");
        fs::create_dir_all(&crashed).expect("create crashed workspace");
        fs::write(crashed.join(WORKSPACE_LOCK_FILE), "").expect("create released lock");
        fs::write(crashed.join("partial.data"), "partial").expect("write partial artifact");

        let workspace = RunWorkspace::allocate(
            temp.path(),
            &ArtifactId::new("recovery-writer").expect("logical ID"),
        )
        .expect("allocate after crash");
        assert!(!crashed.exists());
        let quarantine = temp.path().join(STAGING_QUARANTINE_DIRECTORY);
        assert!(
            fs::read_dir(quarantine)
                .expect("read run quarantine")
                .any(|entry| entry
                    .expect("quarantine entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with("crashed-workspace--"))
        );
        drop(workspace);
    }

    #[test]
    fn writer_startup_never_quarantines_an_active_workspace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = RunWorkspace::allocate(
            temp.path(),
            &ArtifactId::new("active-first").expect("logical ID"),
        )
        .expect("first workspace");
        let first_path = first.staging_path().to_path_buf();

        let second = RunWorkspace::allocate(
            temp.path(),
            &ArtifactId::new("active-second").expect("logical ID"),
        )
        .expect("second workspace");
        assert!(first_path.is_dir(), "active workspace was moved");
        assert!(first_path.join(WORKSPACE_LOCK_FILE).is_file());
        drop(second);
        drop(first);
    }

    #[test]
    fn run_manifest_records_deterministic_recursive_digests_and_identity() {
        let temp = tempfile::tempdir().expect("tempdir");
        let logical_id = ArtifactId::new("android-sample").expect("logical ID");
        let workspace = RunWorkspace::allocate(temp.path(), &logical_id).expect("workspace");
        fs::write(workspace.staging_path().join("profile.json"), "hello").expect("write profile");
        fs::create_dir(workspace.staging_path().join("nested")).expect("create nested");
        fs::write(workspace.staging_path().join("nested/notes.txt"), "notes").expect("write notes");
        let published = workspace
            .publish(&[ArtifactId::new("profile.json").expect("profile ID")])
            .expect("publish");

        let manifest = published.manifest().expect("verified manifest");
        assert_eq!(manifest.format_version, RUN_MANIFEST_VERSION);
        assert_eq!(manifest.producer_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(manifest.logical_id, "android-sample");
        assert_eq!(
            manifest.publication_id,
            published
                .path()
                .file_name()
                .expect("publication name")
                .to_string_lossy()
        );
        assert_eq!(
            manifest
                .artifacts
                .iter()
                .map(|artifact| artifact.relative_path.as_str())
                .collect::<Vec<_>>(),
            ["nested/notes.txt", "profile.json"]
        );
        let profile = manifest
            .artifacts
            .iter()
            .find(|artifact| artifact.relative_path == "profile.json")
            .expect("profile manifest entry");
        assert_eq!(profile.size, 5);
        assert_eq!(
            profile.sha256,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn publication_syncs_files_and_directories_before_the_atomic_rename() {
        let temp = tempfile::tempdir().expect("tempdir");
        let logical_id = ArtifactId::new("ordered-run").expect("logical ID");
        let workspace = RunWorkspace::allocate(temp.path(), &logical_id).expect("workspace");
        fs::write(workspace.staging_path().join("profile.json"), "complete")
            .expect("write profile");
        durability_events();

        workspace
            .publish(&[ArtifactId::new("profile.json").expect("profile ID")])
            .expect("publish");
        let events = durability_events();
        let publication = events
            .iter()
            .position(|event| *event == "publish_run")
            .expect("publication event");
        assert!(events[..publication].contains(&"sync_manifest"));
        assert!(events[..publication].contains(&"sync_file"));
        assert!(events[..publication].contains(&"sync_directory"));
        assert!(events[publication + 1..].contains(&"sync_directory"));
    }

    #[test]
    fn post_rename_sync_failure_reports_committed_run_explicitly() {
        let temp = tempfile::tempdir().expect("tempdir");
        let logical_id = ArtifactId::new("uncertain-run").expect("logical ID");
        let workspace = RunWorkspace::allocate(temp.path(), &logical_id).expect("workspace");
        fs::write(workspace.staging_path().join("profile.json"), "complete")
            .expect("write profile");
        let expected_path = workspace.published_path().to_path_buf();
        FAIL_DIRECTORY_SYNC_AFTER_EVENT.with(|target| target.set(Some("publish_run")));

        let error = workspace
            .publish(&[ArtifactId::new("profile.json").expect("profile ID")])
            .expect_err("surface post-commit durability uncertainty");
        assert!(matches!(
            error,
            ArtifactPathError::PublicationDurabilityUncertain { ref path, .. }
                if path == &expected_path
        ));
        assert!(expected_path.join(RUN_MANIFEST_FILE).is_file());
        validate_run_manifest(&expected_path).expect("committed run remains valid");
    }

    #[test]
    fn latest_commit_orders_generation_sync_before_the_single_pointer_swap() {
        let temp = tempfile::tempdir().expect("tempdir");
        let published = publish_pair(temp.path(), "ordered-latest", "receipt");
        durability_events();

        refresh_pair(&published).expect("refresh latest");
        let events = durability_events();
        let generation = events
            .iter()
            .position(|event| *event == "publish_generation")
            .expect("generation publication event");
        let pointer_sync = events
            .iter()
            .position(|event| *event == "sync_pointer_file")
            .expect("pointer sync event");
        let pointer_commit = events
            .iter()
            .position(|event| *event == "commit_pointer")
            .expect("pointer commit event");
        assert!(events[..generation].contains(&"sync_manifest"));
        assert!(events[generation + 1..pointer_sync].contains(&"sync_directory"));
        let aliases = events
            .iter()
            .position(|event| *event == "install_aliases")
            .expect("alias install event");
        assert!(generation < pointer_sync);
        assert!(
            aliases < pointer_sync,
            "aliases must precede pointer commit"
        );
        assert!(pointer_sync < pointer_commit);
        assert!(events[pointer_commit + 1..].contains(&"sync_directory"));
    }

    #[test]
    fn post_pointer_sync_failure_reports_committed_generation_explicitly() {
        let temp = tempfile::tempdir().expect("tempdir");
        let published = publish_pair(temp.path(), "uncertain-latest", "receipt");
        FAIL_DIRECTORY_SYNC_AFTER_EVENT.with(|target| target.set(Some("commit_pointer")));

        let error = refresh_pair(&published).expect_err("surface pointer durability uncertainty");
        let generation = match error {
            ArtifactPathError::LatestCommitDurabilityUncertain { generation, .. } => generation,
            other => panic!("unexpected error: {other}"),
        };
        let snapshot = LatestSnapshot::open(temp.path()).expect("committed snapshot remains open");
        assert_eq!(snapshot.generation(), generation);
        assert_eq!(
            snapshot
                .read_artifact(&ArtifactId::new("profile.json").expect("profile ID"))
                .expect("read committed artifact"),
            b"receipt"
        );
    }

    #[test]
    fn latest_snapshot_pins_one_generation_across_concurrent_refreshes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = publish_pair(temp.path(), "first-run", "first");
        refresh_pair(&first).expect("first refresh");
        let pinned = LatestSnapshot::open(temp.path()).expect("first snapshot");

        let second = publish_pair(temp.path(), "second-run", "second");
        refresh_pair(&second).expect("second refresh");
        let current = LatestSnapshot::open(temp.path()).expect("second snapshot");

        assert_ne!(pinned.generation(), current.generation());
        for id in ["profile.json", "summary.md"] {
            let id = ArtifactId::new(id).expect("artifact ID");
            assert_eq!(pinned.read_artifact(&id).expect("pinned read"), b"first");
            assert_eq!(current.read_artifact(&id).expect("current read"), b"second");
        }
    }

    #[cfg(unix)]
    #[test]
    fn latest_snapshot_open_succeeds_with_a_read_only_artifact_root() {
        use std::os::unix::fs::PermissionsExt;

        fn set_tree_mode(path: &Path, directory_mode: u32, file_mode: u32) {
            let metadata = fs::symlink_metadata(path).expect("tree metadata");
            if metadata.is_dir() {
                for entry in fs::read_dir(path).expect("read tree") {
                    set_tree_mode(
                        &entry.expect("tree entry").path(),
                        directory_mode,
                        file_mode,
                    );
                }
                fs::set_permissions(path, fs::Permissions::from_mode(directory_mode))
                    .expect("set directory mode");
            } else {
                fs::set_permissions(path, fs::Permissions::from_mode(file_mode))
                    .expect("set file mode");
            }
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let published = publish_pair(temp.path(), "read-only", "receipt");
        refresh_pair(&published).expect("refresh latest");
        set_tree_mode(temp.path(), 0o555, 0o444);

        let opened = LatestSnapshot::open(temp.path()).expect("read-only snapshot open");
        assert_eq!(
            opened
                .read_artifact(&ArtifactId::new("profile.json").expect("profile ID"))
                .expect("read snapshot artifact"),
            b"receipt"
        );

        set_tree_mode(temp.path(), 0o755, 0o644);
    }

    #[test]
    fn writer_allocation_recovers_mixed_legacy_aliases() {
        let temp = tempfile::tempdir().expect("tempdir");
        let published = publish_pair(temp.path(), "recover-run", "committed");
        refresh_pair(&published).expect("refresh latest");
        fs::write(temp.path().join("profile.json"), "interrupted-new")
            .expect("simulate interrupted alias install");
        fs::write(temp.path().join("summary.md"), "committed").expect("retain old alias");

        LatestSnapshot::open(temp.path()).expect("read-only open");
        assert_eq!(
            fs::read_to_string(temp.path().join("profile.json")).expect("unrecovered profile"),
            "interrupted-new",
            "reader open must not mutate compatibility aliases"
        );

        let recovery_probe = RunWorkspace::allocate(
            temp.path(),
            &ArtifactId::new("recovery-probe").expect("probe ID"),
        )
        .expect("writer startup recovers aliases");
        drop(recovery_probe);
        let recovered = LatestSnapshot::open(temp.path()).expect("open recovered snapshot");
        assert_eq!(
            fs::read(temp.path().join("profile.json")).expect("profile alias"),
            b"committed"
        );
        assert_eq!(
            fs::read(temp.path().join("summary.md")).expect("summary alias"),
            b"committed"
        );
        assert_eq!(
            recovered
                .read_artifact(&ArtifactId::new("profile.json").expect("profile ID"))
                .expect("read recovered snapshot"),
            b"committed"
        );
    }

    #[test]
    fn missing_current_recovers_the_unique_highest_chain_tip() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = publish_pair(temp.path(), "first-chain-run", "first");
        refresh_pair(&first).expect("first refresh");
        let second = publish_pair(temp.path(), "second-chain-run", "second");
        refresh_pair(&second).expect("second refresh");
        let expected = LatestSnapshot::open(temp.path()).expect("tip snapshot");
        let expected_generation = expected.generation().to_owned();
        let current = temp.path().join(LATEST_DIRECTORY).join(LATEST_CURRENT_FILE);
        fs::remove_file(&current).expect("remove current pointer");
        fs::write(temp.path().join("profile.json"), "interrupted")
            .expect("tamper compatibility alias");

        assert!(matches!(
            LatestSnapshot::open(temp.path()),
            Err(ArtifactPathError::LatestSnapshotUnavailable { .. })
        ));
        let recovered =
            LatestSnapshot::recover_stable_aliases(temp.path()).expect("recover missing pointer");
        assert_eq!(recovered.generation(), expected_generation);
        assert_eq!(
            fs::read_to_string(temp.path().join("profile.json")).expect("recovered profile"),
            "second"
        );
    }

    #[test]
    fn corrupt_current_recovers_the_unique_valid_chain_tip() {
        let temp = tempfile::tempdir().expect("tempdir");
        let published = publish_pair(temp.path(), "corrupt-pointer-run", "receipt");
        refresh_pair(&published).expect("refresh latest");
        let expected = LatestSnapshot::open(temp.path())
            .expect("snapshot")
            .generation()
            .to_owned();
        let current = temp.path().join(LATEST_DIRECTORY).join(LATEST_CURRENT_FILE);
        fs::write(&current, "../../invalid\n").expect("corrupt pointer");

        assert!(matches!(
            LatestSnapshot::open(temp.path()),
            Err(ArtifactPathError::InvalidLatestPointer { .. })
        ));
        let recovered =
            LatestSnapshot::recover_stable_aliases(temp.path()).expect("recover corrupt pointer");
        assert_eq!(recovered.generation(), expected);
        assert_eq!(
            fs::read_to_string(current)
                .expect("read repaired pointer")
                .trim(),
            expected
        );
    }

    #[test]
    fn missing_current_fails_closed_on_an_ambiguous_generation_fork() {
        let temp = tempfile::tempdir().expect("tempdir");
        let published = publish_pair(temp.path(), "fork-root-run", "root");
        refresh_pair(&published).expect("refresh root");
        let root_snapshot = LatestSnapshot::open(temp.path()).expect("root snapshot");
        let root_generation = root_snapshot.generation().to_owned();
        clone_generation(&root_snapshot, "fork-child-a", Some(&root_generation));
        clone_generation(&root_snapshot, "fork-child-b", Some(&root_generation));
        fs::remove_file(temp.path().join(LATEST_DIRECTORY).join(LATEST_CURRENT_FILE))
            .expect("remove current pointer");

        assert!(matches!(
            LatestSnapshot::recover_stable_aliases(temp.path()),
            Err(ArtifactPathError::AmbiguousGenerationFork { .. })
        ));
    }

    #[test]
    fn predecessor_cycle_is_rejected_by_open_and_recovery() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = publish_pair(temp.path(), "cycle-first", "first");
        refresh_pair(&first).expect("first refresh");
        let first_snapshot = LatestSnapshot::open(temp.path()).expect("first snapshot");
        let first_generation = first_snapshot.generation().to_owned();
        let second = publish_pair(temp.path(), "cycle-second", "second");
        refresh_pair(&second).expect("second refresh");
        let second_snapshot = LatestSnapshot::open(temp.path()).expect("second snapshot");
        let second_generation = second_snapshot.generation().to_owned();
        rewrite_latest_manifest(&first_snapshot.generation_path, |manifest| {
            manifest.predecessor_generation = Some(second_generation.clone());
        });

        assert!(matches!(
            LatestSnapshot::open(temp.path()),
            Err(ArtifactPathError::GenerationChainCycle { .. })
        ));
        fs::remove_file(temp.path().join(LATEST_DIRECTORY).join(LATEST_CURRENT_FILE))
            .expect("remove current pointer");
        assert!(matches!(
            LatestSnapshot::recover_stable_aliases(temp.path()),
            Err(ArtifactPathError::GenerationChainCycle { .. })
        ));
        assert_ne!(first_generation, second_generation);
    }

    #[test]
    fn broken_predecessor_chain_fails_closed_during_recovery() {
        let temp = tempfile::tempdir().expect("tempdir");
        let published = publish_pair(temp.path(), "broken-chain", "receipt");
        refresh_pair(&published).expect("refresh latest");
        let snapshot = LatestSnapshot::open(temp.path()).expect("snapshot");
        rewrite_latest_manifest(snapshot.path(), |manifest| {
            manifest.predecessor_generation = Some("missing-predecessor".to_owned());
        });
        fs::remove_file(temp.path().join(LATEST_DIRECTORY).join(LATEST_CURRENT_FILE))
            .expect("remove current pointer");

        assert!(matches!(
            LatestSnapshot::recover_stable_aliases(temp.path()),
            Err(ArtifactPathError::BrokenGenerationChain { .. })
        ));
    }

    #[test]
    fn alias_install_failure_rolls_back_before_pointer_commit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = publish_pair(temp.path(), "first-run", "first");
        refresh_pair(&first).expect("first refresh");
        let before = LatestSnapshot::open(temp.path()).expect("snapshot before failure");
        let before_generation = before.generation().to_owned();
        let second = publish_pair(temp.path(), "second-run", "second");

        FAIL_ALIAS_INSTALL_AFTER.with(|fail_after| fail_after.set(Some(1)));
        let error = refresh_pair(&second).expect_err("injected alias failure");
        assert!(matches!(error, ArtifactPathError::Io { .. }));

        let after = LatestSnapshot::open(temp.path()).expect("snapshot after rollback");
        assert_eq!(after.generation(), before_generation);
        assert_eq!(
            fs::read_to_string(temp.path().join("profile.json")).expect("profile alias"),
            "first"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("summary.md")).expect("summary alias"),
            "first"
        );
    }

    #[test]
    fn stale_allocated_run_cannot_replace_a_newer_generation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stale = publish_pair(temp.path(), "stale-run", "stale");
        let winner = publish_pair(temp.path(), "winner-run", "winner");
        refresh_pair(&winner).expect("winner refresh");
        let winner_generation = LatestSnapshot::open(temp.path())
            .expect("winner snapshot")
            .generation()
            .to_owned();

        let error = refresh_pair(&stale).expect_err("reject stale CAS token");
        assert!(matches!(
            error,
            ArtifactPathError::StaleLatestGeneration {
                expected: None,
                observed: Some(_)
            }
        ));
        let current = LatestSnapshot::open(temp.path()).expect("current snapshot");
        assert_eq!(current.generation(), winner_generation);
        assert_eq!(
            current
                .read_artifact(&ArtifactId::new("profile.json").expect("profile ID"))
                .expect("read winner"),
            b"winner"
        );
    }

    #[test]
    fn abandoned_latest_staging_is_quarantined_before_the_next_commit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = publish_pair(temp.path(), "first-run", "first");
        refresh_pair(&first).expect("first refresh");
        let staging = temp
            .path()
            .join(LATEST_DIRECTORY)
            .join(LATEST_STAGING_DIRECTORY);
        let abandoned = staging.join("abandoned");
        fs::create_dir(&abandoned).expect("create abandoned staging");
        fs::write(abandoned.join("partial"), "partial").expect("write partial staging");

        let second = publish_pair(temp.path(), "second-run", "second");
        refresh_pair(&second).expect("second refresh");
        assert!(!abandoned.exists());
        let quarantine = temp
            .path()
            .join(LATEST_DIRECTORY)
            .join(LATEST_QUARANTINE_DIRECTORY);
        assert!(
            fs::read_dir(quarantine)
                .expect("read quarantine")
                .any(|entry| entry
                    .expect("quarantine entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with("abandoned--"))
        );
    }

    #[test]
    fn active_latest_staging_lease_defers_recovery_quarantine() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = ApprovedRoot::existing(temp.path()).expect("approved root");
        let latest_path = root
            .prepare_dir(LATEST_DIRECTORY)
            .expect("latest directory");
        let latest_root = ApprovedRoot::existing(latest_path).expect("approved latest root");
        latest_root
            .prepare_dir(LATEST_QUARANTINE_DIRECTORY)
            .expect("quarantine directory");
        let (_, staging_path) = allocate_latest_generation(&latest_root).expect("staging path");
        create_reader_lease_file(&staging_path).expect("staging lease file");
        let lease_path = staging_path.join(LATEST_READER_LOCK_FILE);
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lease_path)
            .expect("open staging lease");
        FileExt::lock_exclusive(&lease).expect("lock active staging lease");

        quarantine_abandoned_latest_staging(&latest_root).expect("active staging recovery pass");
        assert!(staging_path.exists(), "active staging was quarantined");

        FileExt::unlock(&lease).expect("unlock staging lease");
        drop(lease);
        quarantine_abandoned_latest_staging(&latest_root).expect("abandoned staging recovery pass");
        assert!(!staging_path.exists(), "abandoned staging was retained");
    }

    #[test]
    fn newer_latest_producer_version_rejects_a_downgrade() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = publish_pair(temp.path(), "first-run", "first");
        refresh_pair(&first).expect("first refresh");
        let second = publish_pair(temp.path(), "second-run", "second");
        let snapshot = LatestSnapshot::open(temp.path()).expect("snapshot");
        let manifest_path = snapshot.path().join(LATEST_MANIFEST_FILE);
        let mut manifest: LatestManifest =
            read_json_file(&manifest_path, "read test manifest").expect("manifest");
        manifest.producer_version = "999.0.0".to_owned();
        fs::remove_file(&manifest_path).expect("remove old manifest");
        write_json_file(&manifest_path, &manifest, "rewrite test manifest")
            .expect("write newer producer manifest");

        let error = refresh_pair(&second).expect_err("reject producer downgrade");
        assert!(matches!(error, ArtifactPathError::ProducerDowngrade { .. }));
        assert_eq!(
            fs::read_to_string(temp.path().join("profile.json")).expect("stable profile"),
            "first"
        );
    }

    #[test]
    fn unknown_latest_manifest_version_fails_closed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let published = publish_pair(temp.path(), "unknown-version", "receipt");
        refresh_pair(&published).expect("refresh latest");
        let snapshot = LatestSnapshot::open(temp.path()).expect("snapshot");
        let manifest_path = snapshot.path().join(LATEST_MANIFEST_FILE);
        let mut manifest: LatestManifest =
            read_json_file(&manifest_path, "read test manifest").expect("manifest");
        manifest.format_version = LATEST_MANIFEST_VERSION + 1;
        fs::remove_file(&manifest_path).expect("remove old manifest");
        write_json_file(&manifest_path, &manifest, "rewrite test manifest")
            .expect("write unknown manifest");

        assert!(matches!(
            LatestSnapshot::open(temp.path()),
            Err(ArtifactPathError::UnknownManifestVersion { .. })
        ));
    }

    #[test]
    fn corrupt_latest_generation_fails_digest_validation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let published = publish_pair(temp.path(), "corrupt-generation", "receipt");
        refresh_pair(&published).expect("refresh latest");
        let snapshot = LatestSnapshot::open(temp.path()).expect("snapshot");
        fs::write(snapshot.path().join("profile.json"), "tampered").expect("tamper artifact");

        assert!(matches!(
            LatestSnapshot::open(temp.path()),
            Err(ArtifactPathError::CorruptGeneration { .. })
        ));
    }

    #[test]
    fn pinned_snapshot_rejects_bytes_changed_after_open() {
        let temp = tempfile::tempdir().expect("tempdir");
        let published = publish_pair(temp.path(), "mutated-after-open", "original");
        refresh_pair(&published).expect("refresh latest");
        let snapshot = LatestSnapshot::open(temp.path()).expect("open pinned snapshot");
        fs::write(snapshot.path().join("profile.json"), "changed").expect("mutate artifact");

        assert!(matches!(
            snapshot.read_artifact(&ArtifactId::new("profile.json").expect("profile ID")),
            Err(ArtifactPathError::CorruptGeneration { .. })
        ));
    }

    #[test]
    fn latest_refresh_rejects_source_changed_after_run_validation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let published = publish_pair(temp.path(), "mutated-source", "original");
        MUTATE_LATEST_SOURCE_AFTER_VALIDATION.with(|mutation| {
            mutation.replace(Some((
                published.path().join("profile.json"),
                b"changed after validation".to_vec(),
            )));
        });

        let error = refresh_pair(&published).expect_err("reject changed source");
        assert!(matches!(error, ArtifactPathError::ManifestIntegrity { .. }));
        assert!(matches!(
            LatestSnapshot::open(temp.path()),
            Err(ArtifactPathError::LatestSnapshotUnavailable { .. })
        ));
    }

    #[test]
    fn invalid_current_directory_does_not_block_first_commit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let latest = temp.path().join(LATEST_DIRECTORY);
        fs::create_dir_all(latest.join(LATEST_CURRENT_FILE)).expect("seed invalid pointer dir");
        let published = publish_pair(temp.path(), "first-after-invalid-pointer", "receipt");

        refresh_pair(&published).expect("commit after quarantining invalid pointer");
        let snapshot = LatestSnapshot::open(temp.path()).expect("open committed snapshot");
        assert_eq!(
            snapshot
                .read_artifact(&ArtifactId::new("profile.json").expect("profile ID"))
                .expect("read committed artifact"),
            b"receipt"
        );
        assert!(
            fs::read_dir(latest.join(LATEST_QUARANTINE_DIRECTORY))
                .expect("read quarantine")
                .any(|entry| entry
                    .expect("quarantine entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with("current-corrupt--"))
        );
    }

    #[test]
    fn corrupt_off_chain_generation_is_quarantined_without_blocking_writer() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = publish_pair(temp.path(), "anchored", "first");
        refresh_pair(&first).expect("anchor current generation");
        let orphan = temp
            .path()
            .join(LATEST_DIRECTORY)
            .join(LATEST_GENERATIONS_DIRECTORY)
            .join("generation-corrupt-orphan");
        fs::create_dir(&orphan).expect("create corrupt orphan");
        fs::write(orphan.join(LATEST_MANIFEST_FILE), b"not-json").expect("write corrupt orphan");

        let second = publish_pair(temp.path(), "after-orphan", "second");
        refresh_pair(&second).expect("refresh despite corrupt orphan");
        assert!(!orphan.exists());
        let snapshot = LatestSnapshot::open(temp.path()).expect("open latest snapshot");
        assert_eq!(
            snapshot
                .read_artifact(&ArtifactId::new("profile.json").expect("profile ID"))
                .expect("read latest artifact"),
            b"second"
        );
    }

    #[cfg(unix)]
    #[test]
    fn recursive_publication_validation_rejects_nested_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::NamedTempFile::new().expect("outside file");
        let logical_id = ArtifactId::new("nested-symlink").expect("logical ID");
        let workspace = RunWorkspace::allocate(temp.path(), &logical_id).expect("workspace");
        fs::write(workspace.staging_path().join("profile.json"), "complete")
            .expect("write profile");
        fs::create_dir(workspace.staging_path().join("nested")).expect("nested directory");
        symlink(
            outside.path(),
            workspace.staging_path().join("nested/linked"),
        )
        .expect("nested symlink");

        assert!(matches!(
            workspace.publish(&[ArtifactId::new("profile.json").expect("profile ID")]),
            Err(ArtifactPathError::SymlinkComponent { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn publication_rejects_artifacts_hardlinked_outside_the_workspace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        let logical_id = ArtifactId::new("hardlinked-artifact").expect("logical ID");
        let workspace = RunWorkspace::allocate(temp.path(), &logical_id).expect("workspace");
        let profile = workspace.staging_path().join("profile.json");
        fs::write(&profile, "complete").expect("write profile");
        fs::hard_link(&profile, outside.path().join("alias.json")).expect("create hard link");

        assert!(matches!(
            workspace.publish(&[ArtifactId::new("profile.json").expect("profile ID")]),
            Err(ArtifactPathError::HardLinkedArtifact { .. })
        ));
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "linux",
        target_os = "android"
    ))]
    #[test]
    fn platform_publication_rename_never_replaces_a_raced_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir(&source).expect("create source");
        fs::create_dir(&destination).expect("create raced destination");
        fs::write(source.join("source.txt"), "source").expect("write source");
        fs::write(destination.join("destination.txt"), "destination").expect("write destination");

        let error = rename_directory_noreplace(&source, &destination)
            .expect_err("no-replace rename must reject collision");
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::Other
        ));
        assert!(source.join("source.txt").is_file());
        assert_eq!(
            fs::read_to_string(destination.join("destination.txt"))
                .expect("read original destination"),
            "destination"
        );
    }

    #[cfg(unix)]
    #[test]
    fn latest_reader_rejects_a_symlink_pointer() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::NamedTempFile::new().expect("outside pointer");
        fs::write(outside.path(), "generation-attacker").expect("write outside pointer");
        let published = publish_pair(temp.path(), "symlink-pointer", "receipt");
        refresh_pair(&published).expect("refresh latest");
        let current = temp.path().join(LATEST_DIRECTORY).join(LATEST_CURRENT_FILE);
        fs::remove_file(&current).expect("remove current pointer");
        symlink(outside.path(), &current).expect("install pointer symlink");

        assert!(matches!(
            LatestSnapshot::open(temp.path()),
            Err(ArtifactPathError::SymlinkComponent { .. })
        ));
    }
}
