use std::process::ExitStatus;

use tokio::process::Child;

#[cfg(unix)]
pub(crate) struct ForwardedSignals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

#[cfg(unix)]
pub(crate) fn install_forwarded_signals() -> Result<ForwardedSignals, std::io::Error> {
    use tokio::signal::unix::{SignalKind, signal};

    Ok(ForwardedSignals {
        interrupt: signal(SignalKind::interrupt())?,
        terminate: signal(SignalKind::terminate())?,
    })
}

#[cfg(unix)]
pub(crate) async fn wait_with_forwarded_signals(
    child: &mut Child,
    mut signals: ForwardedSignals,
) -> Result<ExitStatus, std::io::Error> {
    let child_id = child.id();
    loop {
        tokio::select! {
            status = child.wait() => return status,
            received = signals.interrupt.recv() => {
                if received.is_some() {
                    forward(child_id, "-INT").await;
                }
            }
            received = signals.terminate.recv() => {
                if received.is_some() {
                    forward(child_id, "-TERM").await;
                }
            }
        }
    }
}

#[cfg(unix)]
async fn forward(child_id: Option<u32>, signal: &str) {
    let Some(child_id) = child_id else {
        return;
    };
    let _ = tokio::process::Command::new("kill")
        .arg(signal)
        .arg(child_id.to_string())
        .status()
        .await;
    let _ = tokio::process::Command::new("pkill")
        .arg(signal)
        .arg("-P")
        .arg(child_id.to_string())
        .status()
        .await;
}

#[cfg(not(unix))]
pub(crate) struct ForwardedSignals;

#[cfg(not(unix))]
pub(crate) fn install_forwarded_signals() -> Result<ForwardedSignals, std::io::Error> {
    Ok(ForwardedSignals)
}

#[cfg(not(unix))]
pub(crate) async fn wait_with_forwarded_signals(
    child: &mut Child,
    _signals: ForwardedSignals,
) -> Result<ExitStatus, std::io::Error> {
    child.wait().await
}
