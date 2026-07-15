//! Declared executable provenance and bounded child-process policy.

use std::path::{Component, Path, PathBuf};
use std::process::Command;

use thiserror::Error;

/// Provenance attached to a child-process executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutableProvenance {
    /// A fixed executable name declared by Mobench and resolved through `PATH`.
    BuiltInPathSearch,
    /// A path passed through an explicit trusted caller seam.
    ExplicitOverride,
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
            provenance: ExecutableProvenance::BuiltInPathSearch,
        })
    }

    /// Accept a program path only through an explicit trusted caller seam.
    pub fn explicit_override(program: impl AsRef<Path>) -> Result<Self, ExecutablePolicyError> {
        let program = program.as_ref();
        if program.as_os_str().is_empty() {
            return Err(ExecutablePolicyError::EmptyExplicitOverride);
        }
        Ok(Self {
            program: program.to_path_buf(),
            provenance: ExecutableProvenance::ExplicitOverride,
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

    /// Construct a command for the declared executable.
    pub fn command(&self) -> Command {
        Command::new(&self.program)
    }
}

/// Executable-selection policy failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExecutablePolicyError {
    #[error("built-in executable must be one fixed PATH-search name, got {0}")]
    InvalidPathSearchName(PathBuf),
    #[error("explicit executable override cannot be empty")]
    EmptyExplicitOverride,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_path_search_accepts_one_fixed_name() {
        let executable = DeclaredExecutable::path_search("python3").expect("declare executable");

        assert_eq!(executable.program(), Path::new("python3"));
        assert_eq!(
            executable.provenance(),
            ExecutableProvenance::BuiltInPathSearch
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
    fn explicit_override_records_trusted_provenance() {
        let executable =
            DeclaredExecutable::explicit_override("./tools/python").expect("declare override");

        assert_eq!(executable.program(), Path::new("./tools/python"));
        assert_eq!(
            executable.provenance(),
            ExecutableProvenance::ExplicitOverride
        );
    }
}
