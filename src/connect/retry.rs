use std::{
    ffi::{OsStr, OsString},
    future::Future,
    time::Duration,
};

use tokio::process::Command;

use crate::shared::{AppError, AppResult};

pub(super) const MIN_RETRY_DELAY: Duration = Duration::from_millis(100);
pub(super) const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);
pub(super) const MAX_RETRIES: u32 = 64;

/// Reconnect behavior for an access session.
pub struct RetryPolicy {
    /// Enable reconnect attempts after a failed SSH process.
    pub enabled: bool,
    /// Delay between attempts, clamped to a safe minimum.
    pub max_delay: Duration,
    /// Maximum number of reconnect attempts.
    pub max_retries: u32,
}

/// One OpenSSH session attempt.
pub(crate) trait SessionRunner {
    async fn run(&self, executable: &OsStr, argv: &[OsString]) -> AppResult<i32>;
}

pub(crate) struct OpenSshRunner;

impl SessionRunner for OpenSshRunner {
    async fn run(&self, executable: &OsStr, argv: &[OsString]) -> AppResult<i32> {
        let mut child = Command::new(executable)
            .args(argv)
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| ssh_error("spawn", error))?;
        tokio::select! {
            status = child.wait() => {
                status
                    .map(|status| status.code().unwrap_or(130))
                    .map_err(|error| ssh_error("wait for", error))
            }
            _ = tokio::signal::ctrl_c() => {
                child.kill().await.map_err(|error| ssh_error("stop", error))?;
                Ok(130)
            }
        }
    }
}

pub(crate) async fn run_ssh<R>(
    runner: &R,
    executable: &OsStr,
    argv: &[OsString],
    retry: &RetryPolicy,
) -> AppResult<i32>
where
    R: SessionRunner,
{
    run_ssh_with_cancel(runner, executable, argv, retry, tokio::signal::ctrl_c()).await
}

pub(crate) async fn run_ssh_with_cancel<R, C>(
    runner: &R,
    executable: &OsStr,
    argv: &[OsString],
    retry: &RetryPolicy,
    cancel: C,
) -> AppResult<i32>
where
    R: SessionRunner,
    C: Future<Output = std::io::Result<()>> + Send,
{
    let mut attempt = 0_u32;
    tokio::pin!(cancel);

    loop {
        let code = runner.run(executable, argv).await?;
        if !should_retry(retry.enabled, Some(code)) || attempt >= retry_limit(retry) {
            return Ok(code);
        }

        attempt = attempt.saturating_add(1);
        let delay = retry_delay(retry);
        eprintln!(
            "sshp: SSH disconnected; reconnect attempt {attempt} in {}s",
            delay.as_secs_f32()
        );
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = &mut cancel => return Ok(130),
        }
    }
}

pub(super) fn should_retry(reconnect: bool, exit_code: Option<i32>) -> bool {
    // OpenSSH uses 255 for connection failures, but does not distinguish them
    // from authentication failures or remote exit code 255.
    reconnect && exit_code == Some(255)
}

pub(super) fn retry_delay(policy: &RetryPolicy) -> Duration {
    policy.max_delay.max(MIN_RETRY_DELAY).min(MAX_RETRY_DELAY)
}

pub(super) fn retry_limit(policy: &RetryPolicy) -> u32 {
    policy.max_retries.min(MAX_RETRIES)
}

fn ssh_error(operation: &'static str, error: std::io::Error) -> AppError {
    AppError::Ssh(Box::new(std::io::Error::new(
        error.kind(),
        format!("failed to {operation} ssh: {error}"),
    )))
}
