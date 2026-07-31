//! CLI-side adapter from command plans into `mobench-process` policy.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Output;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use mobench_process::{
    DeclaredExecutable, EnvironmentPolicy, OutputStreamPolicy, ProcessCancellation, ProcessLimits,
    ProcessOutcome, ProcessRunner, ProcessSpec, StdinPolicy, WorkingDirectoryPolicy,
};

pub(crate) const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub(crate) const TOOL_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;

/// One declared external tool invocation with explicit resource policies.
#[derive(Debug, Clone)]
pub(crate) struct ToolCommand {
    executable: DeclaredExecutable,
    arguments: Vec<OsString>,
    working_directory: WorkingDirectoryPolicy,
    environment: EnvironmentPolicy,
    timeout: Duration,
    stdin: StdinPolicy,
    stdout: OutputStreamPolicy,
    stderr: OutputStreamPolicy,
}

impl ToolCommand {
    pub(crate) fn path_search(name: &'static str) -> Self {
        Self::new(
            DeclaredExecutable::path_search(name)
                .expect("fixed tool name must be one PATH-search component"),
        )
    }

    pub(crate) fn explicit(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let executable = DeclaredExecutable::explicit_override(path)
            .with_context(|| format!("declaring external tool {}", path.display()))?;
        Ok(Self::new(executable))
    }

    fn new(executable: DeclaredExecutable) -> Self {
        Self {
            executable,
            arguments: Vec::new(),
            working_directory: WorkingDirectoryPolicy::Inherit,
            environment: EnvironmentPolicy::Inherit,
            timeout: DEFAULT_TOOL_TIMEOUT,
            stdin: StdinPolicy::Null,
            stdout: OutputStreamPolicy::Capture,
            stderr: OutputStreamPolicy::Capture,
        }
    }

    pub(crate) fn arg(&mut self, argument: impl AsRef<OsStr>) -> &mut Self {
        self.arguments.push(argument.as_ref().to_os_string());
        self
    }

    pub(crate) fn args<I, S>(&mut self, arguments: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.arguments.extend(
            arguments
                .into_iter()
                .map(|argument| argument.as_ref().to_os_string()),
        );
        self
    }

    pub(crate) fn current_dir(&mut self, path: impl AsRef<Path>) -> &mut Self {
        self.working_directory = WorkingDirectoryPolicy::Path(path.as_ref().to_path_buf());
        self
    }

    pub(crate) fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        let key = key.as_ref().to_os_string();
        let (set, remove) = self.inherited_environment_edits();
        remove.remove(&key);
        set.insert(key, value.as_ref().to_os_string());
        self
    }

    pub(crate) fn timeout(&mut self, timeout: Duration) -> &mut Self {
        self.timeout = timeout;
        self
    }

    pub(crate) fn stdin(&mut self, policy: StdinPolicy) -> &mut Self {
        self.stdin = policy;
        self
    }

    pub(crate) fn inherit_output(&mut self) -> &mut Self {
        self.stdout = OutputStreamPolicy::Inherit;
        self.stderr = OutputStreamPolicy::Inherit;
        self
    }

    pub(crate) fn run(&self) -> Result<ProcessOutcome> {
        let outcome = ProcessRunner::run(&self.spec())
            .with_context(|| format!("running external tool {}", self.program().display()))?;
        if outcome.cancelled {
            bail!("external tool {} was interrupted", self.program().display());
        }
        Ok(outcome)
    }

    pub(crate) fn run_cancellable(
        &self,
        cancellation: &ProcessCancellation,
    ) -> Result<ProcessOutcome> {
        let outcome = ProcessRunner::run_cancellable(&self.spec(), cancellation)
            .with_context(|| format!("running external tool {}", self.program().display()))?;
        if outcome.cancelled {
            bail!("external tool {} was interrupted", self.program().display());
        }
        Ok(outcome)
    }

    pub(crate) fn output(&self) -> Result<Output> {
        self.run()?.into_complete_output().with_context(|| {
            format!(
                "capturing complete output from {}",
                self.program().display()
            )
        })
    }

    pub(crate) fn output_cancellable(&self, cancellation: &ProcessCancellation) -> Result<Output> {
        self.run_cancellable(cancellation)?
            .into_complete_output()
            .with_context(|| {
                format!(
                    "capturing complete output from {}",
                    self.program().display()
                )
            })
    }

    pub(crate) fn program(&self) -> &Path {
        self.executable.program()
    }

    fn spec(&self) -> ProcessSpec {
        ProcessSpec::new(
            self.executable.clone(),
            self.arguments.clone(),
            self.working_directory.clone(),
            self.environment.clone(),
            ProcessLimits::new(self.timeout, TOOL_OUTPUT_LIMIT, TOOL_OUTPUT_LIMIT),
        )
        .with_stdin_policy(self.stdin)
        .with_stdout_policy(self.stdout)
        .with_stderr_policy(self.stderr)
    }

    fn inherited_environment_edits(
        &mut self,
    ) -> (&mut BTreeMap<OsString, OsString>, &mut BTreeSet<OsString>) {
        if matches!(self.environment, EnvironmentPolicy::Inherit) {
            self.environment = EnvironmentPolicy::InheritWith {
                set: BTreeMap::new(),
                remove: BTreeSet::new(),
            };
        }
        match &mut self.environment {
            EnvironmentPolicy::InheritWith { set, remove } => (set, remove),
            EnvironmentPolicy::Inherit | EnvironmentPolicy::ClearAndSet { .. } => {
                unreachable!("CLI adapter only creates inherited environments")
            }
        }
    }
}
