use std::process::Stdio;

use tokio::process::Command;

use crate::{ProcessSpec, install_forwarded_signals, wait_with_forwarded_signals};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessOutcome {
    pub exit_code: i32,
}

pub async fn run_attached(spec: &ProcessSpec) -> Result<ProcessOutcome, ProcessError> {
    // Register handlers before the child exists so an immediate Ctrl-C cannot
    // race process creation and terminate Context Bridge without forwarding.
    let signals = install_forwarded_signals().map_err(|source| ProcessError::Signal {
        executable: spec.executable.clone(),
        source,
    })?;
    let mut command = Command::new(&spec.executable);
    command
        .args(&spec.args)
        .envs(&spec.environment)
        .current_dir(&spec.current_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(false);
    let mut child = command.spawn().map_err(|source| ProcessError::Launch {
        executable: spec.executable.clone(),
        source,
    })?;
    let status = wait_with_forwarded_signals(&mut child, signals)
        .await
        .map_err(|source| ProcessError::Wait {
            executable: spec.executable.clone(),
            source,
        })?;
    Ok(ProcessOutcome {
        exit_code: exit_code(status),
    })
}

#[cfg(unix)]
fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .unwrap_or(1)
}

#[cfg(not(unix))]
fn exit_code(status: std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("could not install signal forwarding for `{executable}`")]
    Signal {
        executable: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not launch `{executable}`")]
    Launch {
        executable: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not wait for `{executable}`")]
    Wait {
        executable: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}
