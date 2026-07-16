//! Declared executable provenance and bounded child-process policy.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

#[cfg(unix)]
use std::collections::VecDeque;
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::process::Stdio;
#[cfg(unix)]
use std::time::Instant;

use thiserror::Error;

/// Provenance attached to a child-process executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutableProvenance {
    /// A single-component executable name resolved through `PATH`.
    FixedNamePathSearch,
    /// An executable name or path supplied explicitly by the caller.
    CallerProvided,
}

/// An executable whose selection source is explicit in the type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredExecutable {
    program: PathBuf,
    provenance: ExecutableProvenance,
}

impl DeclaredExecutable {
    /// Declare a fixed program name that Mobench may resolve through `PATH`.
    pub fn path_search(program: impl AsRef<Path>) -> Result<Self, ExecutablePolicyError> {
        let program = program.as_ref();
        let mut components = program.components();
        if program.as_os_str().is_empty()
            || !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
        {
            return Err(ExecutablePolicyError::InvalidPathSearchName(
                program.to_path_buf(),
            ));
        }
        Ok(Self {
            program: program.to_path_buf(),
            provenance: ExecutableProvenance::FixedNamePathSearch,
        })
    }

    /// Accept a caller-supplied program name or path through an explicit seam.
    pub fn explicit_override(program: impl AsRef<Path>) -> Result<Self, ExecutablePolicyError> {
        let program = program.as_ref();
        if program.as_os_str().is_empty() {
            return Err(ExecutablePolicyError::EmptyExplicitOverride);
        }
        Ok(Self {
            program: program.to_path_buf(),
            provenance: ExecutableProvenance::CallerProvided,
        })
    }

    /// Return the selected program path or fixed name.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Return how this executable was selected.
    pub fn provenance(&self) -> ExecutableProvenance {
        self.provenance
    }

    /// Construct a raw command for the declared executable.
    ///
    /// Prefer [`ProcessRunner`] for production subprocesses that need bounded
    /// output, a deadline, and explicit cwd/environment policies.
    pub fn command(&self) -> Command {
        Command::new(&self.program)
    }
}

/// Explicit policy for the child process working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkingDirectoryPolicy {
    /// Inherit the parent process working directory.
    Inherit,
    /// Run from the specified directory.
    Path(PathBuf),
}

/// Explicit policy for the child process standard input stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdinPolicy {
    /// Connect the child to the parent's standard input.
    Inherit,
    /// Connect the child to the platform null device.
    Null,
}

/// Explicit policy for one child output stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStreamPolicy {
    /// Capture the stream with the configured byte and retention bounds.
    Capture,
    /// Connect the child directly to the corresponding parent stream.
    Inherit,
}

/// Which portions of an oversized captured stream are retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureRetention {
    /// Retain the beginning of the stream.
    Head,
    /// Retain the end of the stream.
    Tail,
    /// Split the byte budget between the beginning and end of the stream.
    HeadAndTail,
}

/// Explicit policy for the child process environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentPolicy {
    /// Inherit the parent environment unchanged.
    Inherit,
    /// Inherit the parent environment, then remove and set selected variables.
    InheritWith {
        /// Variables to set after applying removals.
        set: BTreeMap<OsString, OsString>,
        /// Variables to remove from the inherited environment.
        remove: BTreeSet<OsString>,
    },
    /// Clear the parent environment and set only the supplied variables.
    ClearAndSet {
        /// Complete environment visible to the child.
        set: BTreeMap<OsString, OsString>,
    },
}

/// Output and wall-clock bounds for one child process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessLimits {
    /// Maximum wall-clock time before the child is killed and reaped.
    pub timeout: Duration,
    /// Maximum stdout bytes retained in memory.
    pub stdout_max_bytes: usize,
    /// Maximum stderr bytes retained in memory.
    pub stderr_max_bytes: usize,
}

impl ProcessLimits {
    /// Construct explicit process bounds.
    pub const fn new(timeout: Duration, stdout_max_bytes: usize, stderr_max_bytes: usize) -> Self {
        Self {
            timeout,
            stdout_max_bytes,
            stderr_max_bytes,
        }
    }
}

/// Complete specification for one bounded child-process invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    executable: DeclaredExecutable,
    arguments: Vec<OsString>,
    working_directory: WorkingDirectoryPolicy,
    environment: EnvironmentPolicy,
    limits: ProcessLimits,
    stdin: StdinPolicy,
    stdout: OutputStreamPolicy,
    stderr: OutputStreamPolicy,
    stdout_retention: CaptureRetention,
    stderr_retention: CaptureRetention,
    drain_grace: Duration,
}

/// Default time allowed to collect pipe data after process-scope termination.
///
/// Once this grace elapses, capture pipes are closed and the returned stream is
/// marked incomplete. This keeps escaped descendants from extending the
/// process deadline indefinitely.
pub const DEFAULT_DRAIN_GRACE: Duration = Duration::from_millis(250);

impl ProcessSpec {
    /// Construct a process specification with explicit cwd, environment, and
    /// resource limits. Standard input defaults to null; stdout and stderr
    /// default to bounded head-and-tail capture for source compatibility.
    pub fn new(
        executable: DeclaredExecutable,
        arguments: Vec<OsString>,
        working_directory: WorkingDirectoryPolicy,
        environment: EnvironmentPolicy,
        limits: ProcessLimits,
    ) -> Self {
        Self {
            executable,
            arguments,
            working_directory,
            environment,
            limits,
            stdin: StdinPolicy::Null,
            stdout: OutputStreamPolicy::Capture,
            stderr: OutputStreamPolicy::Capture,
            stdout_retention: CaptureRetention::HeadAndTail,
            stderr_retention: CaptureRetention::HeadAndTail,
            drain_grace: DEFAULT_DRAIN_GRACE,
        }
    }

    /// Select the child standard-input policy.
    pub fn with_stdin_policy(mut self, policy: StdinPolicy) -> Self {
        self.stdin = policy;
        self
    }

    /// Select the child standard-output policy.
    pub fn with_stdout_policy(mut self, policy: OutputStreamPolicy) -> Self {
        self.stdout = policy;
        self
    }

    /// Select the child standard-error policy.
    pub fn with_stderr_policy(mut self, policy: OutputStreamPolicy) -> Self {
        self.stderr = policy;
        self
    }

    /// Select retention for captured standard output.
    pub fn with_stdout_retention(mut self, retention: CaptureRetention) -> Self {
        self.stdout_retention = retention;
        self
    }

    /// Select retention for captured standard error.
    pub fn with_stderr_retention(mut self, retention: CaptureRetention) -> Self {
        self.stderr_retention = retention;
        self
    }

    /// Override the bounded post-termination pipe-drain grace.
    pub fn with_drain_grace(mut self, drain_grace: Duration) -> Self {
        self.drain_grace = drain_grace;
        self
    }

    /// Return the declared executable.
    pub fn executable(&self) -> &DeclaredExecutable {
        &self.executable
    }

    /// Return the literal argv entries passed to the executable.
    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    /// Return the working-directory policy.
    pub fn working_directory(&self) -> &WorkingDirectoryPolicy {
        &self.working_directory
    }

    /// Return the environment policy.
    pub fn environment(&self) -> &EnvironmentPolicy {
        &self.environment
    }

    /// Return the process resource limits.
    pub fn limits(&self) -> ProcessLimits {
        self.limits
    }

    /// Return the child standard-input policy.
    pub fn stdin_policy(&self) -> StdinPolicy {
        self.stdin
    }

    /// Return the child standard-output policy.
    pub fn stdout_policy(&self) -> OutputStreamPolicy {
        self.stdout
    }

    /// Return the child standard-error policy.
    pub fn stderr_policy(&self) -> OutputStreamPolicy {
        self.stderr
    }

    /// Return standard-output capture retention.
    pub fn stdout_retention(&self) -> CaptureRetention {
        self.stdout_retention
    }

    /// Return standard-error capture retention.
    pub fn stderr_retention(&self) -> CaptureRetention {
        self.stderr_retention
    }

    /// Return the bounded post-termination pipe-drain grace.
    pub fn drain_grace(&self) -> Duration {
        self.drain_grace
    }
}

/// Bounded output captured from one child stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedStream {
    /// Whether this stream was configured for capture.
    pub captured: bool,
    /// Retained bytes according to `retention`, never larger than the cap.
    pub bytes: Vec<u8>,
    /// Whether bytes were omitted from the retained capture.
    pub truncated: bool,
    /// Whether capture ended before EOF because the drain grace elapsed.
    pub incomplete: bool,
    /// Bytes observed before EOF or forced pipe closure.
    pub total_bytes: u64,
    /// Retention policy used for this stream.
    pub retention: CaptureRetention,
}

impl CapturedStream {
    /// True when the bytes cannot be treated as the complete child stream.
    pub const fn is_partial(&self) -> bool {
        self.truncated || self.incomplete
    }

    #[cfg(unix)]
    fn inherited(retention: CaptureRetention) -> Self {
        Self {
            captured: false,
            bytes: Vec::new(),
            truncated: false,
            incomplete: false,
            total_bytes: 0,
            retention,
        }
    }
}

/// Structured result from a child that was successfully spawned and reaped.
#[derive(Debug)]
pub struct ProcessOutcome {
    /// Program name/path selected by the caller.
    pub program: PathBuf,
    /// Provenance of the executable selection.
    pub provenance: ExecutableProvenance,
    /// Reaped child exit status, including the post-kill status on timeout.
    pub status: ExitStatus,
    /// True when the deadline elapsed and the runner killed the child.
    pub timed_out: bool,
    /// True when cooperative cancellation killed the child process scope.
    pub cancelled: bool,
    /// Wall-clock duration including output drain and child reap.
    pub duration: Duration,
    /// Bounded stdout capture.
    pub stdout: CapturedStream,
    /// Bounded stderr capture.
    pub stderr: CapturedStream,
}

impl ProcessOutcome {
    /// Convert a captured outcome into the standard library's complete output
    /// shape.
    ///
    /// Machine-readable callers must use this instead of parsing retained
    /// bytes directly: inherited, truncated, or incompletely drained streams
    /// are rejected rather than being mistaken for a complete document.
    pub fn into_complete_output(self) -> Result<std::process::Output, IncompleteOutputError> {
        validate_complete_stream(&self.stdout, ProcessStream::Stdout)?;
        validate_complete_stream(&self.stderr, ProcessStream::Stderr)?;
        Ok(std::process::Output {
            status: self.status,
            stdout: self.stdout.bytes,
            stderr: self.stderr.bytes,
        })
    }
}

/// A captured child stream was not a complete machine-readable value.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{stream} was not captured completely ({reason})")]
pub struct IncompleteOutputError {
    /// Stream that did not contain a complete value.
    pub stream: ProcessStream,
    /// Why the stream cannot be parsed as complete output.
    pub reason: IncompleteOutputReason,
}

/// Why captured output cannot be treated as complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncompleteOutputReason {
    /// The stream was connected directly to the parent.
    NotCaptured,
    /// The configured byte budget omitted part of the stream.
    Truncated,
    /// The drain grace elapsed before the stream reached EOF.
    Incomplete,
}

impl std::fmt::Display for IncompleteOutputReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCaptured => formatter.write_str("stream was inherited"),
            Self::Truncated => formatter.write_str("capture byte limit was exceeded"),
            Self::Incomplete => formatter.write_str("capture ended before EOF"),
        }
    }
}

fn validate_complete_stream(
    stream: &CapturedStream,
    kind: ProcessStream,
) -> Result<(), IncompleteOutputError> {
    let reason = if !stream.captured {
        Some(IncompleteOutputReason::NotCaptured)
    } else if stream.incomplete {
        Some(IncompleteOutputReason::Incomplete)
    } else if stream.truncated {
        Some(IncompleteOutputReason::Truncated)
    } else {
        None
    };
    match reason {
        Some(reason) => Err(IncompleteOutputError {
            stream: kind,
            reason,
        }),
        None => Ok(()),
    }
}

/// Identifies one captured child stream in structured errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStream {
    /// Child standard output.
    Stdout,
    /// Child standard error.
    Stderr,
}

impl std::fmt::Display for ProcessStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stdout => formatter.write_str("stdout"),
            Self::Stderr => formatter.write_str("stderr"),
        }
    }
}

/// Structured failure while starting, supervising, or capturing a child.
#[derive(Debug, Error)]
pub enum ProcessRunError {
    /// Cooperative cancellation was already requested before spawn.
    #[error("process launch for {program} was cancelled before spawn")]
    CancelledBeforeSpawn {
        /// Declared program name/path.
        program: PathBuf,
    },
    /// The child could not be started.
    #[error("failed to spawn {program} ({provenance:?}): {source}")]
    Spawn {
        /// Declared program name/path.
        program: PathBuf,
        /// Executable-selection provenance.
        provenance: ExecutableProvenance,
        /// Operating-system error.
        #[source]
        source: io::Error,
    },
    /// A configured pipe was unexpectedly unavailable.
    #[error("{stream} pipe was unavailable for {program}")]
    MissingPipe {
        /// Declared program name/path.
        program: PathBuf,
        /// Missing stream.
        stream: ProcessStream,
    },
    /// A capture pipe could not be switched to nonblocking mode.
    #[error("failed to configure {stream} capture for {program}: {source}")]
    ConfigurePipe {
        /// Declared program name/path.
        program: PathBuf,
        /// Stream that could not be configured.
        stream: ProcessStream,
        /// Operating-system error.
        #[source]
        source: io::Error,
    },
    /// The child status could not be polled.
    #[error("failed to poll {program}: {source}")]
    Poll {
        /// Declared program name/path.
        program: PathBuf,
        /// Operating-system error.
        #[source]
        source: io::Error,
    },
    /// The child process scope could not be terminated during cleanup.
    #[error("failed to terminate process scope for {program}: {source}")]
    Terminate {
        /// Declared program name/path.
        program: PathBuf,
        /// Operating-system error.
        #[source]
        source: io::Error,
    },
    /// The child could not be reaped.
    #[error("failed to reap {program}: {source}")]
    Reap {
        /// Declared program name/path.
        program: PathBuf,
        /// Operating-system error.
        #[source]
        source: io::Error,
    },
    /// Draining a child stream failed.
    #[error("failed to drain {stream} for {program}: {source}")]
    Capture {
        /// Declared program name/path.
        program: PathBuf,
        /// Stream that failed.
        stream: ProcessStream,
        /// Operating-system error.
        #[source]
        source: io::Error,
    },
    /// The direct child did not become reapable within the cleanup grace.
    #[error("cleanup deadline elapsed before {program} could be reaped")]
    CleanupDeadline {
        /// Declared program name/path.
        program: PathBuf,
    },
    /// The platform cannot provide the process-tree deadline guarantee.
    #[error("{feature} is unsupported on this platform")]
    UnsupportedPlatformGuarantee {
        /// Guarantee that is unavailable.
        feature: &'static str,
    },
}

impl ProcessRunError {
    /// Return the operating-system error kind when spawning failed.
    pub fn spawn_error_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Spawn { source, .. } => Some(source.kind()),
            Self::CancelledBeforeSpawn { .. }
            | Self::MissingPipe { .. }
            | Self::ConfigurePipe { .. }
            | Self::Poll { .. }
            | Self::Terminate { .. }
            | Self::Reap { .. }
            | Self::Capture { .. }
            | Self::CleanupDeadline { .. }
            | Self::UnsupportedPlatformGuarantee { .. } => None,
        }
    }
}

/// Synchronous bounded child-process runner.
///
/// On Unix, each child is placed in a new process group. Capture pipes are
/// nonblocking and are closed after a bounded drain grace, so even a descendant
/// that escapes the group cannot hold [`ProcessRunner::run`] open indefinitely.
/// Platforms without this implementation fail before spawning with
/// [`ProcessRunError::UnsupportedPlatformGuarantee`].
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessRunner;

/// Thread-safe cooperative process cancellation signal.
#[derive(Debug, Clone, Default)]
pub struct ProcessCancellation {
    requested: Arc<AtomicBool>,
}

impl ProcessCancellation {
    /// Request cancellation of current and subsequent runs using this token.
    pub fn cancel(&self) {
        self.requested.store(true, Ordering::SeqCst);
    }

    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }
}

static GLOBAL_CANCELLATION: OnceLock<ProcessCancellation> = OnceLock::new();

/// Return the process-wide token used by [`ProcessRunner::run`].
///
/// CLI entrypoints can connect an OS interruption handler to this token so
/// subprocesses launched through the CLI and SDK share one cancellation scope.
pub fn global_cancellation_token() -> ProcessCancellation {
    GLOBAL_CANCELLATION
        .get_or_init(ProcessCancellation::default)
        .clone()
}

impl ProcessRunner {
    /// Run the specified child, concurrently drain stdout/stderr, enforce the
    /// deadline, and return only after the child has been reaped.
    pub fn run(spec: &ProcessSpec) -> Result<ProcessOutcome, ProcessRunError> {
        Self::run_cancellable(spec, &global_cancellation_token())
    }

    /// Run with an explicit cooperative cancellation token.
    pub fn run_cancellable(
        spec: &ProcessSpec,
        cancellation: &ProcessCancellation,
    ) -> Result<ProcessOutcome, ProcessRunError> {
        if cancellation.is_cancelled() {
            return Err(ProcessRunError::CancelledBeforeSpawn {
                program: spec.executable.program().to_path_buf(),
            });
        }
        run_platform(spec, cancellation)
    }
}

#[cfg(not(unix))]
fn run_platform(
    _spec: &ProcessSpec,
    _cancellation: &ProcessCancellation,
) -> Result<ProcessOutcome, ProcessRunError> {
    Err(ProcessRunError::UnsupportedPlatformGuarantee {
        feature: "bounded process-tree supervision",
    })
}

#[cfg(unix)]
fn run_platform(
    spec: &ProcessSpec,
    cancellation: &ProcessCancellation,
) -> Result<ProcessOutcome, ProcessRunError> {
    const MAX_POLL_INTERVAL: Duration = Duration::from_millis(10);

    let program = spec.executable.program().to_path_buf();
    let provenance = spec.executable.provenance();
    let started = Instant::now();
    let mut command = spec.executable.command();
    command.args(&spec.arguments);
    apply_working_directory_policy(&mut command, &spec.working_directory);
    apply_environment_policy(&mut command, &spec.environment);
    configure_process_group(&mut command);
    apply_stdio_policy(&mut command, spec);

    let child = command.spawn().map_err(|source| ProcessRunError::Spawn {
        program: program.clone(),
        provenance,
        source,
    })?;
    let mut scope = ChildScope::new(child);

    let mut stdout = match spec.stdout {
        OutputStreamPolicy::Capture => {
            let pipe = scope
                .child
                .stdout
                .take()
                .ok_or_else(|| ProcessRunError::MissingPipe {
                    program: program.clone(),
                    stream: ProcessStream::Stdout,
                })?;
            Some(ActiveCapture::new(
                pipe,
                spec.limits.stdout_max_bytes,
                spec.stdout_retention,
                &program,
                ProcessStream::Stdout,
            )?)
        }
        OutputStreamPolicy::Inherit => None,
    };
    let mut stderr = match spec.stderr {
        OutputStreamPolicy::Capture => {
            let pipe = scope
                .child
                .stderr
                .take()
                .ok_or_else(|| ProcessRunError::MissingPipe {
                    program: program.clone(),
                    stream: ProcessStream::Stderr,
                })?;
            Some(ActiveCapture::new(
                pipe,
                spec.limits.stderr_max_bytes,
                spec.stderr_retention,
                &program,
                ProcessStream::Stderr,
            )?)
        }
        OutputStreamPolicy::Inherit => None,
    };

    let mut status = None;
    let mut timed_out = false;
    let mut cancelled = false;
    let mut cleanup_started = None;

    loop {
        drain_optional_capture(&mut stdout, &program, ProcessStream::Stdout)?;
        drain_optional_capture(&mut stderr, &program, ProcessStream::Stderr)?;

        if status.is_none()
            && scope.has_exited().map_err(|source| ProcessRunError::Poll {
                program: program.clone(),
                source,
            })?
        {
            // Keep the direct child waitable until the process group has been
            // terminated. The unreaped child pins its PID/PGID, preventing a
            // successful short-lived command from racing an unrelated process
            // group that later reuses the numeric id.
            scope
                .terminate(true)
                .map_err(|source| ProcessRunError::Terminate {
                    program: program.clone(),
                    source,
                })?;
            status = Some(scope.reap().map_err(|source| ProcessRunError::Reap {
                program: program.clone(),
                source,
            })?);
        }

        if status.is_some() && cleanup_started.is_none() {
            cleanup_started = Some(Instant::now());
        } else if status.is_none() && cleanup_started.is_none() && cancellation.is_cancelled() {
            cancelled = true;
            scope
                .terminate(false)
                .map_err(|source| ProcessRunError::Terminate {
                    program: program.clone(),
                    source,
                })?;
            cleanup_started = Some(Instant::now());
        } else if status.is_none()
            && cleanup_started.is_none()
            && started.elapsed() >= spec.limits.timeout
        {
            timed_out = true;
            scope
                .terminate(false)
                .map_err(|source| ProcessRunError::Terminate {
                    program: program.clone(),
                    source,
                })?;
            cleanup_started = Some(Instant::now());
        }

        let captures_complete = capture_complete(&stdout) && capture_complete(&stderr);
        if let Some(exit_status) = status
            && captures_complete
        {
            return Ok(build_outcome(
                &program,
                OutcomeState {
                    provenance,
                    status: exit_status,
                    timed_out,
                    cancelled,
                    started,
                },
                (stdout, stderr),
                spec,
                false,
            ));
        }

        if let Some(cleanup_started) = cleanup_started
            && cleanup_started.elapsed() >= spec.drain_grace
        {
            let Some(exit_status) = status else {
                return Err(ProcessRunError::CleanupDeadline { program });
            };
            return Ok(build_outcome(
                &program,
                OutcomeState {
                    provenance,
                    status: exit_status,
                    timed_out,
                    cancelled,
                    started,
                },
                (stdout, stderr),
                spec,
                true,
            ));
        }

        let wait = next_poll_interval(started, cleanup_started, spec, MAX_POLL_INTERVAL);
        wait_for_pipe_activity(&stdout, &stderr, wait, &program)?;
    }
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // A zero process-group id makes the child the leader of a fresh group.
    command.process_group(0);
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy)]
struct ProcessGroup {
    id: libc::pid_t,
}

#[cfg(unix)]
impl ProcessGroup {
    fn for_child(child: &std::process::Child) -> Self {
        Self {
            id: child.id() as libc::pid_t,
        }
    }

    fn terminate(self, child: &mut std::process::Child, direct_exited: bool) -> io::Result<()> {
        // SAFETY: `id` is the positive pid returned by `Child::id`; negating it
        // addresses only the process group created for this child.
        let result = unsafe { libc::kill(-self.id, libc::SIGKILL) };
        let group_error = (result != 0).then(io::Error::last_os_error);

        if !direct_exited {
            match child.kill() {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::InvalidInput => {}
                Err(error) => return Err(error),
            }
        }

        match group_error {
            // Darwin reports EPERM when the process group contains only the
            // unreaped zombie leader. If a live same-uid descendant remained
            // in the group, it would still be signalable and killpg would
            // succeed. The waitable leader continues to pin the numeric PGID.
            Some(error) if direct_exited && error.kind() == io::ErrorKind::PermissionDenied => {
                Ok(())
            }
            Some(error) if error.raw_os_error() != Some(libc::ESRCH) => Err(error),
            Some(_) | None => Ok(()),
        }
    }
}

#[cfg(unix)]
struct ChildScope {
    child: std::process::Child,
    process_group: ProcessGroup,
    terminated: bool,
    status: Option<ExitStatus>,
}

#[cfg(unix)]
impl ChildScope {
    fn new(child: std::process::Child) -> Self {
        let process_group = ProcessGroup::for_child(&child);
        Self {
            child,
            process_group,
            terminated: false,
            status: None,
        }
    }

    fn has_exited(&self) -> io::Result<bool> {
        // `waitid(..., WNOWAIT)` observes the terminal state without reaping
        // the child. Holding the zombie until the process group is terminated
        // pins the numeric PID/PGID and removes the reuse race around killpg.
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        // SAFETY: `info` points to writable siginfo storage and the positive
        // pid belongs to the live `Child` handle. WNOWAIT preserves waitability.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.process_group.id as libc::id_t,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result != 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                return Ok(false);
            }
            return Err(error);
        }
        // SAFETY: waitid initialized `info` on success, including the no-state
        // case where si_pid is zero.
        Ok(unsafe { info.assume_init().si_pid() } == self.process_group.id)
    }

    fn reap(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.status {
            return Ok(status);
        }
        let status = self.child.wait()?;
        self.status = Some(status);
        Ok(status)
    }

    fn terminate(&mut self, direct_exited: bool) -> io::Result<()> {
        if !self.terminated {
            self.process_group
                .terminate(&mut self.child, direct_exited)?;
            self.terminated = true;
        }
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for ChildScope {
    fn drop(&mut self) {
        if !self.terminated {
            let _ = self
                .process_group
                .terminate(&mut self.child, self.status.is_some());
        }
        if self.status.is_none() {
            let _ = self.child.kill();
            // SIGKILL is not catchable. A blocking wait here is therefore
            // bounded by kernel task teardown and guarantees that error paths
            // do not leak a zombie direct child.
            let _ = self.child.wait();
        }
    }
}

#[cfg(unix)]
fn apply_working_directory_policy(command: &mut Command, policy: &WorkingDirectoryPolicy) {
    match policy {
        WorkingDirectoryPolicy::Inherit => {}
        WorkingDirectoryPolicy::Path(path) => {
            command.current_dir(path);
        }
    }
}

#[cfg(unix)]
fn apply_environment_policy(command: &mut Command, policy: &EnvironmentPolicy) {
    match policy {
        EnvironmentPolicy::Inherit => {}
        EnvironmentPolicy::InheritWith { set, remove } => {
            for key in remove {
                command.env_remove(key);
            }
            command.envs(set);
        }
        EnvironmentPolicy::ClearAndSet { set } => {
            command.env_clear().envs(set);
        }
    }
}

#[cfg(unix)]
fn apply_stdio_policy(command: &mut Command, spec: &ProcessSpec) {
    command.stdin(match spec.stdin {
        StdinPolicy::Inherit => Stdio::inherit(),
        StdinPolicy::Null => Stdio::null(),
    });
    command.stdout(match spec.stdout {
        OutputStreamPolicy::Capture => Stdio::piped(),
        OutputStreamPolicy::Inherit => Stdio::inherit(),
    });
    command.stderr(match spec.stderr {
        OutputStreamPolicy::Capture => Stdio::piped(),
        OutputStreamPolicy::Inherit => Stdio::inherit(),
    });
}

#[cfg(unix)]
struct ActiveCapture<R> {
    reader: R,
    accumulator: CaptureAccumulator,
    eof: bool,
}

#[cfg(unix)]
impl<R: Read + AsRawFd> ActiveCapture<R> {
    fn new(
        reader: R,
        max_bytes: usize,
        retention: CaptureRetention,
        program: &Path,
        stream: ProcessStream,
    ) -> Result<Self, ProcessRunError> {
        set_nonblocking(reader.as_raw_fd()).map_err(|source| ProcessRunError::ConfigurePipe {
            program: program.to_path_buf(),
            stream,
            source,
        })?;
        Ok(Self {
            reader,
            accumulator: CaptureAccumulator::new(max_bytes, retention),
            eof: false,
        })
    }

    fn drain_available(&mut self) -> io::Result<()> {
        const DRAIN_QUANTUM_BYTES: usize = 256 * 1024;

        let mut buffer = [0_u8; 8 * 1024];
        let mut drained = 0_usize;
        while drained < DRAIN_QUANTUM_BYTES {
            match self.reader.read(&mut buffer) {
                Ok(0) => {
                    self.eof = true;
                    return Ok(());
                }
                Ok(read) => {
                    self.accumulator.record(&buffer[..read]);
                    drained = drained.saturating_add(read);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn finish(self, force_incomplete: bool) -> CapturedStream {
        self.accumulator.finish(force_incomplete && !self.eof)
    }
}

#[cfg(unix)]
fn drain_optional_capture<R: Read + AsRawFd>(
    capture: &mut Option<ActiveCapture<R>>,
    program: &Path,
    stream: ProcessStream,
) -> Result<(), ProcessRunError> {
    if let Some(capture) = capture {
        capture
            .drain_available()
            .map_err(|source| ProcessRunError::Capture {
                program: program.to_path_buf(),
                stream,
                source,
            })?;
    }
    Ok(())
}

#[cfg(unix)]
fn capture_complete<R>(capture: &Option<ActiveCapture<R>>) -> bool {
    capture.as_ref().is_none_or(|capture| capture.eof)
}

#[cfg(unix)]
struct OutcomeState {
    provenance: ExecutableProvenance,
    status: ExitStatus,
    timed_out: bool,
    cancelled: bool,
    started: Instant,
}

#[cfg(unix)]
fn build_outcome(
    program: &Path,
    state: OutcomeState,
    captures: (
        Option<ActiveCapture<std::process::ChildStdout>>,
        Option<ActiveCapture<std::process::ChildStderr>>,
    ),
    spec: &ProcessSpec,
    force_incomplete: bool,
) -> ProcessOutcome {
    let (stdout, stderr) = captures;
    ProcessOutcome {
        program: program.to_path_buf(),
        provenance: state.provenance,
        status: state.status,
        timed_out: state.timed_out,
        cancelled: state.cancelled,
        duration: state.started.elapsed(),
        stdout: stdout.map_or_else(
            || CapturedStream::inherited(spec.stdout_retention),
            |capture| capture.finish(force_incomplete),
        ),
        stderr: stderr.map_or_else(
            || CapturedStream::inherited(spec.stderr_retention),
            |capture| capture.finish(force_incomplete),
        ),
    }
}

#[cfg(unix)]
fn next_poll_interval(
    started: Instant,
    cleanup_started: Option<Instant>,
    spec: &ProcessSpec,
    maximum: Duration,
) -> Duration {
    match cleanup_started {
        Some(started) => maximum.min(spec.drain_grace.saturating_sub(started.elapsed())),
        None => maximum.min(spec.limits.timeout.saturating_sub(started.elapsed())),
    }
}

#[cfg(unix)]
fn wait_for_pipe_activity<Stdout: AsRawFd, Stderr: AsRawFd>(
    stdout: &Option<ActiveCapture<Stdout>>,
    stderr: &Option<ActiveCapture<Stderr>>,
    timeout: Duration,
    program: &Path,
) -> Result<(), ProcessRunError> {
    let mut descriptors = Vec::with_capacity(2);
    if let Some(capture) = stdout
        && !capture.eof
    {
        descriptors.push(libc::pollfd {
            fd: capture.reader.as_raw_fd(),
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        });
    }
    if let Some(capture) = stderr
        && !capture.eof
    {
        descriptors.push(libc::pollfd {
            fd: capture.reader.as_raw_fd(),
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        });
    }

    let timeout_millis = timeout
        .as_nanos()
        .saturating_add(999_999)
        .saturating_div(1_000_000)
        .min(i32::MAX as u128) as libc::c_int;
    if descriptors.is_empty() {
        std::thread::sleep(timeout);
        return Ok(());
    }

    // SAFETY: `descriptors` is a valid mutable pollfd array for the duration of
    // this call, and its length is passed unchanged.
    let result = unsafe {
        libc::poll(
            descriptors.as_mut_ptr(),
            descriptors.len() as libc::nfds_t,
            timeout_millis,
        )
    };
    if result >= 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::Interrupted {
        Ok(())
    } else {
        Err(ProcessRunError::Poll {
            program: program.to_path_buf(),
            source: error,
        })
    }
}

#[cfg(unix)]
fn set_nonblocking(fd: std::os::fd::RawFd) -> io::Result<()> {
    // SAFETY: `fd` is owned by a live child pipe. `F_GETFL` does not mutate
    // memory and `F_SETFL` updates only flags on that descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
struct CaptureAccumulator {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    max_bytes: usize,
    total_bytes: u64,
    retention: CaptureRetention,
}

#[cfg(unix)]
impl CaptureAccumulator {
    fn new(max_bytes: usize, retention: CaptureRetention) -> Self {
        Self {
            head: Vec::with_capacity(max_bytes.min(8 * 1024)),
            tail: VecDeque::new(),
            max_bytes,
            total_bytes: 0,
            retention,
        }
    }

    fn record(&mut self, bytes: &[u8]) {
        self.total_bytes = self.total_bytes.saturating_add(bytes.len() as u64);
        match self.retention {
            CaptureRetention::Head => {
                let retained = self
                    .max_bytes
                    .saturating_sub(self.head.len())
                    .min(bytes.len());
                self.head.extend_from_slice(&bytes[..retained]);
            }
            CaptureRetention::Tail => retain_tail(&mut self.tail, bytes, self.max_bytes),
            CaptureRetention::HeadAndTail => {
                let head_max = self.max_bytes.saturating_add(1) / 2;
                let head_retained = head_max.saturating_sub(self.head.len()).min(bytes.len());
                self.head.extend_from_slice(&bytes[..head_retained]);
                retain_tail(
                    &mut self.tail,
                    &bytes[head_retained..],
                    self.max_bytes.saturating_sub(head_max),
                );
            }
        }
    }

    fn finish(self, incomplete: bool) -> CapturedStream {
        let mut bytes = self.head;
        bytes.extend(self.tail);
        CapturedStream {
            captured: true,
            truncated: incomplete || self.total_bytes > bytes.len() as u64,
            incomplete,
            total_bytes: self.total_bytes,
            retention: self.retention,
            bytes,
        }
    }
}

#[cfg(unix)]
fn retain_tail(tail: &mut VecDeque<u8>, bytes: &[u8], maximum: usize) {
    if maximum == 0 {
        return;
    }
    if bytes.len() >= maximum {
        tail.clear();
        tail.extend(&bytes[bytes.len() - maximum..]);
        return;
    }
    let overflow = tail
        .len()
        .saturating_add(bytes.len())
        .saturating_sub(maximum);
    tail.drain(..overflow);
    tail.extend(bytes);
}

/// Executable-selection policy failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExecutablePolicyError {
    #[error("executable must be one fixed PATH-search name, got {0}")]
    InvalidPathSearchName(PathBuf),
    #[error("explicit executable override cannot be empty")]
    EmptyExplicitOverride,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    fn write_executable_script(directory: &Path, name: &str, body: &str) -> PathBuf {
        let path = directory.join(name);
        fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).expect("write test script");
        let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("make test script executable");
        path
    }

    #[cfg(unix)]
    fn test_spec(
        executable: DeclaredExecutable,
        arguments: Vec<OsString>,
        timeout: Duration,
        stdout_max_bytes: usize,
        stderr_max_bytes: usize,
    ) -> ProcessSpec {
        ProcessSpec::new(
            executable,
            arguments,
            WorkingDirectoryPolicy::Inherit,
            EnvironmentPolicy::Inherit,
            ProcessLimits::new(timeout, stdout_max_bytes, stderr_max_bytes),
        )
    }

    #[test]
    fn built_in_path_search_accepts_one_fixed_name() {
        let executable = DeclaredExecutable::path_search("python3").expect("declare executable");

        assert_eq!(executable.program(), Path::new("python3"));
        assert_eq!(
            executable.provenance(),
            ExecutableProvenance::FixedNamePathSearch
        );
    }

    #[test]
    fn built_in_path_search_rejects_paths_and_empty_names() {
        for invalid in ["", "./python", "tools/python", "../python", "/bin/python"] {
            assert!(matches!(
                DeclaredExecutable::path_search(invalid),
                Err(ExecutablePolicyError::InvalidPathSearchName(_))
            ));
        }
    }

    #[test]
    fn explicit_override_records_caller_provenance() {
        let executable =
            DeclaredExecutable::explicit_override("./tools/python").expect("declare override");

        assert_eq!(executable.program(), Path::new("./tools/python"));
        assert_eq!(
            executable.provenance(),
            ExecutableProvenance::CallerProvided
        );
    }

    #[cfg(unix)]
    #[test]
    fn runner_passes_metacharacters_as_literal_argv() {
        let root = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(root.path(), "literal argv", "printf '%s' \"$1\"");
        let marker = root.path().join("shell-expanded");
        let payload = format!("literal;$(touch {}) `false` * ?", marker.display());
        let spec = test_spec(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            vec![payload.clone().into()],
            Duration::from_secs(5),
            4 * 1024,
            4 * 1024,
        );

        let outcome = ProcessRunner::run(&spec).expect("run literal argv script");

        assert!(outcome.status.success());
        assert_eq!(outcome.stdout.bytes, payload.as_bytes());
        assert!(!marker.exists(), "argv was interpreted by a shell");
    }

    #[cfg(unix)]
    #[test]
    fn runner_applies_working_directory_and_clear_environment() {
        let root = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(
            root.path(),
            "cwd-env",
            "printf '%s|%s|%s' \"$MOBENCH_VALUE\" \"${HOME-unset}\" \"$(pwd)\"",
        );
        let mut set = BTreeMap::new();
        set.insert(OsString::from("MOBENCH_VALUE"), OsString::from("visible"));
        let spec = ProcessSpec::new(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            Vec::new(),
            WorkingDirectoryPolicy::Path(root.path().to_path_buf()),
            EnvironmentPolicy::ClearAndSet { set },
            ProcessLimits::new(Duration::from_secs(5), 4096, 4096),
        );

        let outcome = ProcessRunner::run(&spec).expect("run with cwd and clear environment");
        let stdout = String::from_utf8_lossy(&outcome.stdout.bytes);
        let mut fields = stdout.split('|');
        assert_eq!(fields.next(), Some("visible"));
        assert_eq!(fields.next(), Some("unset"));
        let reported_cwd = PathBuf::from(fields.next().expect("reported cwd"));
        assert_eq!(
            reported_cwd.canonicalize().expect("canonical reported cwd"),
            root.path().canonicalize().expect("canonical expected cwd")
        );
        assert!(!outcome.stdout.is_partial());
    }

    #[cfg(unix)]
    #[test]
    fn runner_applies_inherit_with_set_and_remove() {
        let root = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(
            root.path(),
            "inherit-with",
            "printf '%s|%s' \"$MOBENCH_PROCESS_VISIBLE\" \"${HOME-unset}\"",
        );
        let mut set = BTreeMap::new();
        set.insert(
            OsString::from("MOBENCH_PROCESS_VISIBLE"),
            OsString::from("yes"),
        );
        let mut remove = BTreeSet::new();
        remove.insert(OsString::from("HOME"));
        let spec = ProcessSpec::new(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            Vec::new(),
            WorkingDirectoryPolicy::Inherit,
            EnvironmentPolicy::InheritWith { set, remove },
            ProcessLimits::new(Duration::from_secs(5), 4096, 4096),
        );

        let outcome = ProcessRunner::run(&spec).expect("run with edited environment");
        assert_eq!(outcome.stdout.bytes, b"yes|unset");
    }

    #[cfg(unix)]
    #[test]
    fn runner_uses_null_stdin_and_supports_inherited_output() {
        let root = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(
            root.path(),
            "stdin-null",
            "if IFS= read -r line; then exit 9; else printf eof >&2; fi",
        );
        let spec = test_spec(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            Vec::new(),
            Duration::from_secs(5),
            1024,
            1024,
        )
        .with_stdin_policy(StdinPolicy::Null)
        .with_stdout_policy(OutputStreamPolicy::Inherit);

        let outcome = ProcessRunner::run(&spec).expect("run with null stdin");
        assert!(outcome.status.success());
        assert!(!outcome.stdout.captured);
        assert_eq!(outcome.stderr.bytes, b"eof");
    }

    #[cfg(unix)]
    #[test]
    fn runner_zero_caps_drain_and_report_truncation() {
        let root = tempfile::tempdir().expect("tempdir");
        let script =
            write_executable_script(root.path(), "zero-caps", "printf stdout; printf stderr >&2");
        let spec = test_spec(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            Vec::new(),
            Duration::from_secs(5),
            0,
            0,
        );

        let outcome = ProcessRunner::run(&spec).expect("drain zero-cap streams");
        assert!(outcome.stdout.bytes.is_empty());
        assert!(outcome.stderr.bytes.is_empty());
        assert_eq!(outcome.stdout.total_bytes, 6);
        assert_eq!(outcome.stderr.total_bytes, 6);
        assert!(outcome.stdout.truncated);
        assert!(outcome.stderr.truncated);
        assert!(!outcome.stdout.incomplete);
        assert!(!outcome.stderr.incomplete);
    }

    #[cfg(unix)]
    #[test]
    fn runner_retains_head_and_tail_for_machine_detectable_truncation() {
        let root = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(root.path(), "head-tail", "printf 0123456789");
        let spec = test_spec(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            Vec::new(),
            Duration::from_secs(5),
            6,
            0,
        );

        let outcome = ProcessRunner::run(&spec).expect("capture head and tail");
        assert_eq!(outcome.stdout.bytes, b"012789");
        assert_eq!(outcome.stdout.total_bytes, 10);
        assert_eq!(outcome.stdout.retention, CaptureRetention::HeadAndTail);
        assert!(outcome.stdout.is_partial());
        assert!(!outcome.stdout.incomplete);
    }

    #[cfg(unix)]
    #[test]
    fn runner_caps_dual_streams_without_deadlock() {
        let root = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(
            root.path(),
            "dual-stream",
            "dd if=/dev/zero bs=4096 count=64 2>/dev/null\n\
             dd if=/dev/zero bs=4096 count=64 1>&2 2>/dev/null",
        );
        let spec = test_spec(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            Vec::new(),
            Duration::from_secs(5),
            1024,
            2048,
        );

        let outcome = ProcessRunner::run(&spec).expect("drain both streams");

        assert!(outcome.status.success());
        assert_eq!(outcome.stdout.bytes.len(), 1024);
        assert_eq!(outcome.stderr.bytes.len(), 2048);
        assert_eq!(outcome.stdout.total_bytes, 64 * 4096);
        assert_eq!(outcome.stderr.total_bytes, 64 * 4096);
        assert!(outcome.stdout.truncated);
        assert!(outcome.stderr.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn runner_reports_truncation_per_stream() {
        let root = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(
            root.path(),
            "one-truncated-stream",
            "printf 'short'\n\
             dd if=/dev/zero bs=4096 count=2 1>&2 2>/dev/null",
        );
        let spec = test_spec(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            Vec::new(),
            Duration::from_secs(5),
            1024,
            512,
        );

        let outcome = ProcessRunner::run(&spec).expect("capture streams");

        assert_eq!(outcome.stdout.bytes, b"short");
        assert!(!outcome.stdout.truncated);
        assert_eq!(outcome.stderr.bytes.len(), 512);
        assert!(outcome.stderr.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn complete_machine_output_rejects_truncated_and_inherited_streams() {
        let root = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(root.path(), "machine-output", "printf 0123456789");
        let truncated = test_spec(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            Vec::new(),
            Duration::from_secs(5),
            4,
            1024,
        );

        let error = ProcessRunner::run(&truncated)
            .expect("run truncated output script")
            .into_complete_output()
            .expect_err("truncated machine output must be rejected");
        assert_eq!(error.stream, ProcessStream::Stdout);
        assert_eq!(error.reason, IncompleteOutputReason::Truncated);

        let inherited = test_spec(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            Vec::new(),
            Duration::from_secs(5),
            1024,
            1024,
        )
        .with_stdout_policy(OutputStreamPolicy::Inherit);
        let error = ProcessRunner::run(&inherited)
            .expect("run inherited output script")
            .into_complete_output()
            .expect_err("inherited machine output must be rejected");
        assert_eq!(error.stream, ProcessStream::Stdout);
        assert_eq!(error.reason, IncompleteOutputReason::NotCaptured);
    }

    #[cfg(unix)]
    #[test]
    fn complete_machine_output_preserves_status_and_bytes() {
        let root = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(
            root.path(),
            "complete-output",
            "printf stdout; printf stderr >&2; exit 7",
        );
        let spec = test_spec(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            Vec::new(),
            Duration::from_secs(5),
            1024,
            1024,
        );

        let output = ProcessRunner::run(&spec)
            .expect("run complete output script")
            .into_complete_output()
            .expect("convert complete output");
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout, b"stdout");
        assert_eq!(output.stderr, b"stderr");
    }

    #[cfg(unix)]
    #[test]
    fn runner_times_out_kills_and_reaps_child() {
        let root = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(
            root.path(),
            "hang-with-descendant",
            "sleep 30 &\nwhile :; do :; done",
        );
        let spec = test_spec(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            Vec::new(),
            Duration::from_millis(250),
            1024,
            1024,
        );

        let outcome = ProcessRunner::run(&spec).expect("time out hanging child");

        assert!(outcome.timed_out);
        assert!(!outcome.cancelled);
        assert!(!outcome.status.success());
        assert!(
            outcome.duration < Duration::from_secs(5),
            "deadline enforcement took {:?}",
            outcome.duration
        );
    }

    #[cfg(unix)]
    #[test]
    fn cooperative_cancellation_kills_and_reaps_the_process_scope() {
        let root = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(
            root.path(),
            "cancel-with-descendant",
            "sleep 30 &\nwhile :; do :; done",
        );
        let spec = test_spec(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            Vec::new(),
            Duration::from_secs(30),
            1024,
            1024,
        );
        let cancellation = ProcessCancellation::default();
        let request = cancellation.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            request.cancel();
        });

        let outcome = ProcessRunner::run_cancellable(&spec, &cancellation)
            .expect("cooperatively cancel child");
        canceller.join().expect("join cancellation request");

        assert!(outcome.cancelled);
        assert!(!outcome.timed_out);
        assert!(!outcome.status.success());
        assert!(outcome.duration < Duration::from_secs(2));
    }

    #[test]
    fn cancellation_requested_before_run_fails_before_spawn() {
        let cancellation = ProcessCancellation::default();
        cancellation.cancel();
        let spec = ProcessSpec::new(
            DeclaredExecutable::path_search("definitely-not-executed").expect("declare program"),
            Vec::new(),
            WorkingDirectoryPolicy::Inherit,
            EnvironmentPolicy::Inherit,
            ProcessLimits::new(Duration::from_secs(1), 1024, 1024),
        );

        assert!(matches!(
            ProcessRunner::run_cancellable(&spec, &cancellation),
            Err(ProcessRunError::CancelledBeforeSpawn { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn continuous_dual_stream_output_cannot_starve_deadline() {
        let root = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(
            root.path(),
            "continuous-output",
            "while :; do printf 0123456789; printf 9876543210 >&2; done",
        );
        let spec = test_spec(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            Vec::new(),
            Duration::from_millis(150),
            64,
            64,
        );

        let outcome = ProcessRunner::run(&spec).expect("bound continuous output");
        assert!(outcome.timed_out);
        assert!(outcome.duration < Duration::from_secs(2));
        assert!(outcome.stdout.bytes.len() <= 64);
        assert!(outcome.stderr.bytes.len() <= 64);
    }

    #[cfg(unix)]
    #[test]
    fn runner_bounds_escaped_session_descendant_pipe_drain() {
        const MODE: &str = "MOBENCH_PROCESS_ESCAPED_DESCENDANT_MODE";
        const PID_PATH: &str = "MOBENCH_PROCESS_ESCAPED_DESCENDANT_PID_PATH";
        const READY_PATH: &str = "MOBENCH_PROCESS_ESCAPED_DESCENDANT_READY_PATH";
        const TEST_NAME: &str = "tests::runner_bounds_escaped_session_descendant_pipe_drain";

        match std::env::var(MODE).as_deref() {
            Ok("child") => {
                // SAFETY: this subprocess exists only to verify that a new
                // session can retain inherited capture descriptors.
                assert_ne!(unsafe { libc::setsid() }, -1, "setsid failed");
                fs::write(std::env::var_os(READY_PATH).expect("ready path"), b"ready")
                    .expect("write ready marker");
                std::thread::sleep(Duration::from_secs(10));
                return;
            }
            Ok("parent") => {
                let child = Command::new(std::env::current_exe().expect("current test exe"))
                    .arg(TEST_NAME)
                    .arg("--exact")
                    .arg("--nocapture")
                    .env(MODE, "child")
                    .env(
                        READY_PATH,
                        std::env::var_os(READY_PATH).expect("ready path"),
                    )
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit())
                    .spawn()
                    .expect("spawn escaped descendant");
                fs::write(
                    std::env::var_os(PID_PATH).expect("pid path"),
                    child.id().to_string(),
                )
                .expect("write escaped pid");
                let ready_path = PathBuf::from(
                    std::env::var_os(READY_PATH).expect("ready path should remain set"),
                );
                let wait_started = Instant::now();
                while !ready_path.exists() && wait_started.elapsed() < Duration::from_secs(2) {
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(ready_path.exists(), "escaped child did not become ready");
                drop(child);
                return;
            }
            _ => {}
        }

        struct KillEscapedChild(libc::pid_t);
        impl Drop for KillEscapedChild {
            fn drop(&mut self) {
                // SAFETY: the pid was written by the dedicated child process.
                let _ = unsafe { libc::kill(self.0, libc::SIGKILL) };
            }
        }

        let root = tempfile::tempdir().expect("tempdir");
        let pid_path = root.path().join("escaped.pid");
        let ready_path = root.path().join("escaped.ready");
        let mut set = BTreeMap::new();
        set.insert(OsString::from(MODE), OsString::from("parent"));
        set.insert(OsString::from(PID_PATH), pid_path.clone().into_os_string());
        set.insert(OsString::from(READY_PATH), ready_path.into_os_string());
        let spec = ProcessSpec::new(
            DeclaredExecutable::explicit_override(
                std::env::current_exe().expect("current test executable"),
            )
            .expect("declare test executable"),
            vec![TEST_NAME.into(), "--exact".into(), "--nocapture".into()],
            WorkingDirectoryPolicy::Inherit,
            EnvironmentPolicy::InheritWith {
                set,
                remove: BTreeSet::new(),
            },
            ProcessLimits::new(Duration::from_secs(3), 4096, 4096),
        )
        .with_drain_grace(Duration::from_millis(150));

        let outcome = ProcessRunner::run(&spec);
        let escaped_pid = fs::read_to_string(&pid_path)
            .expect("read escaped pid")
            .parse::<libc::pid_t>()
            .expect("parse escaped pid");
        let _cleanup = KillEscapedChild(escaped_pid);
        let outcome = outcome.expect("return despite escaped pipe holder");

        assert!(outcome.status.success());
        assert!(!outcome.timed_out);
        assert!(outcome.duration < Duration::from_secs(2));
        assert!(outcome.stdout.incomplete);
        assert!(outcome.stderr.incomplete);
        assert!(outcome.stdout.is_partial());
        assert!(outcome.stderr.is_partial());
    }

    #[cfg(unix)]
    #[test]
    fn runner_cleans_pipe_holding_descendant_after_parent_exit() {
        let root = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(
            root.path(),
            "exiting-parent",
            "sleep 30 &\nprintf parent-exited",
        );
        let spec = test_spec(
            DeclaredExecutable::explicit_override(&script).expect("declare script"),
            Vec::new(),
            Duration::from_secs(2),
            1024,
            1024,
        );

        let outcome = ProcessRunner::run(&spec).expect("clean up pipe-holding descendant");

        assert!(outcome.status.success());
        assert!(!outcome.timed_out);
        assert_eq!(outcome.stdout.bytes, b"parent-exited");
        assert!(
            outcome.duration < Duration::from_secs(2),
            "descendant held capture pipes for {:?}",
            outcome.duration
        );
    }

    #[cfg(unix)]
    #[test]
    fn runner_preserves_executable_provenance() {
        let path_search = test_spec(
            DeclaredExecutable::path_search("sh").expect("declare PATH search"),
            vec!["-c".into(), "printf path-search".into()],
            Duration::from_secs(5),
            1024,
            1024,
        );
        let explicit = test_spec(
            DeclaredExecutable::explicit_override("/bin/sh").expect("declare explicit shell"),
            vec!["-c".into(), "printf explicit".into()],
            Duration::from_secs(5),
            1024,
            1024,
        );

        let path_outcome = ProcessRunner::run(&path_search).expect("run PATH search");
        let explicit_outcome = ProcessRunner::run(&explicit).expect("run explicit executable");

        assert_eq!(
            path_outcome.provenance,
            ExecutableProvenance::FixedNamePathSearch
        );
        assert_eq!(
            explicit_outcome.provenance,
            ExecutableProvenance::CallerProvided
        );
        assert_eq!(path_outcome.stdout.bytes, b"path-search");
        assert_eq!(explicit_outcome.stdout.bytes, b"explicit");
    }

    #[cfg(unix)]
    #[test]
    fn ambient_executable_selection_does_not_replace_declared_program() {
        let root = tempfile::tempdir().expect("tempdir");
        let marker = root.path().join("ambient-ran");
        let declared = write_executable_script(root.path(), "declared", "printf declared");
        let ambient = write_executable_script(
            root.path(),
            "ambient",
            &format!("printf ambient > {}", marker.display()),
        );
        let mut set = BTreeMap::new();
        set.insert(
            OsString::from("MOBENCH_PROCESS_EXECUTABLE"),
            ambient.into_os_string(),
        );
        let spec = ProcessSpec::new(
            DeclaredExecutable::explicit_override(&declared).expect("declare executable"),
            Vec::new(),
            WorkingDirectoryPolicy::Inherit,
            EnvironmentPolicy::InheritWith {
                set,
                remove: BTreeSet::new(),
            },
            ProcessLimits::new(Duration::from_secs(5), 1024, 1024),
        );

        let outcome = ProcessRunner::run(&spec).expect("run declared executable");

        assert_eq!(outcome.stdout.bytes, b"declared");
        assert!(!marker.exists(), "ambient executable was invoked");
    }

    #[cfg(not(unix))]
    #[test]
    fn unsupported_platform_fails_before_spawning() {
        let spec = ProcessSpec::new(
            DeclaredExecutable::path_search("definitely-not-executed").expect("declare program"),
            Vec::new(),
            WorkingDirectoryPolicy::Inherit,
            EnvironmentPolicy::Inherit,
            ProcessLimits::new(Duration::from_secs(1), 1024, 1024),
        );

        assert!(matches!(
            ProcessRunner::run(&spec),
            Err(ProcessRunError::UnsupportedPlatformGuarantee { .. })
        ));
    }
}
