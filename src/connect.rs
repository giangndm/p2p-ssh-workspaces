//! Access/connect mode.

use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

use tokio::io::{AsyncRead, AsyncWrite, stdin, stdout};

use crate::shared::{
    AppResult, ArgumentError, HostAlias, HostConfig, IrohEndpoint, NodeId, PeerTransport, SshHost,
    StartupCommand, StreamParts, bridge,
};

#[path = "connect/retry.rs"]
mod retry;
pub use retry::RetryPolicy;
use retry::{OpenSshRunner, SessionRunner, run_ssh};

/// OpenSSH invocation data.
pub struct SshCommand {
    /// SSH host name passed to OpenSSH.
    pub host: SshHost,
    /// Optional remote startup command.
    pub startup: Option<StartupCommand>,
    /// Extra OpenSSH arguments.
    pub ssh_args: Vec<OsString>,
    /// Hidden stdio proxy command.
    pub proxy: ProxyCommand,
}

impl SshCommand {
    /// Build an OpenSSH command description for one configured host.
    pub fn new(
        alias: HostAlias,
        host: HostConfig,
        ssh_args: Vec<OsString>,
        proxy_executable: PathBuf,
    ) -> Self {
        Self {
            host: host.ssh_host,
            startup: host.startup,
            ssh_args,
            proxy: ProxyCommand {
                executable: proxy_executable,
                args: vec![
                    OsString::from("connect"),
                    OsString::from("--stdio"),
                    OsString::from(alias.as_ref()),
                ],
            },
        }
    }

    /// Return the argv passed to OpenSSH.
    pub fn argv(&self) -> AppResult<Vec<OsString>> {
        let mut args = self.ssh_args.clone();
        args.push(OsString::from("-o"));
        args.push(OsString::from(format!(
            "ProxyCommand={}",
            self.proxy.shell_command()?
        )));
        args.push(OsString::from(self.host.as_ref()));
        if let Some(startup) = &self.startup {
            args.push(OsString::from(shell_command(startup)?));
        }
        Ok(args)
    }
}

/// OpenSSH ProxyCommand invocation.
pub struct ProxyCommand {
    /// Proxy executable path.
    pub executable: PathBuf,
    /// Proxy arguments.
    pub args: Vec<OsString>,
}

impl ProxyCommand {
    fn shell_command(&self) -> AppResult<String> {
        let executable = self.executable.to_str().ok_or_else(|| {
            ArgumentError::Invalid("proxy executable must be valid UTF-8".to_owned())
        })?;
        let mut words = vec![shell_quote(executable)?];
        for arg in &self.args {
            let arg = arg.to_str().ok_or_else(|| {
                ArgumentError::Invalid("proxy argument must be valid UTF-8".to_owned())
            })?;
            words.push(shell_quote(arg)?);
        }
        Ok(words.join(" "))
    }
}

fn shell_command(startup: &StartupCommand) -> AppResult<String> {
    let mut words = vec![shell_quote(&startup.program)?];
    for arg in &startup.args {
        words.push(shell_quote(arg)?);
    }
    Ok(words.join(" "))
}

fn shell_quote(value: &str) -> Result<String, ArgumentError> {
    if value.contains('\0') {
        return Err(ArgumentError::Invalid(
            "shell argument must not contain NUL".to_owned(),
        ));
    }
    Ok(format!("'{}'", value.replace('\'', "'\"'\"'")))
}

/// Access-side connection supervisor.
pub struct Connector {
    endpoint: Option<IrohEndpoint>,
    retry: RetryPolicy,
}

impl Connector {
    /// Create a connector for an interactive SSH session.
    pub fn new(retry: RetryPolicy) -> Self {
        Self {
            endpoint: None,
            retry,
        }
    }

    /// Create a connector for an OpenSSH stdio proxy.
    pub fn with_endpoint(endpoint: IrohEndpoint, retry: RetryPolicy) -> Self {
        Self {
            endpoint: Some(endpoint),
            retry,
        }
    }

    /// Run one interactive SSH session and apply reconnect policy.
    pub async fn run(
        &self,
        alias: HostAlias,
        host: HostConfig,
        ssh_args: Vec<OsString>,
    ) -> AppResult<i32> {
        self.run_with_runner(alias, host, ssh_args, &OpenSshRunner)
            .await
    }

    async fn run_with_runner<R>(
        &self,
        alias: HostAlias,
        host: HostConfig,
        ssh_args: Vec<OsString>,
        runner: &R,
    ) -> AppResult<i32>
    where
        R: SessionRunner,
    {
        let reconnect = host.reconnect && self.retry.enabled;
        let proxy_executable = std::env::current_exe()?;
        let command = SshCommand::new(alias, host, ssh_args, proxy_executable);
        let argv = command.argv()?;
        let retry = RetryPolicy {
            enabled: reconnect,
            max_delay: self.retry.max_delay,
            max_retries: self.retry.max_retries,
        };
        run_ssh(runner, OsStr::new("ssh"), &argv, &retry).await
    }

    /// Bridge OpenSSH stdio to one iroh stream.
    pub async fn run_stdio_proxy(&self, node_id: NodeId) -> AppResult<()> {
        let endpoint = self
            .endpoint
            .as_ref()
            .ok_or_else(|| std::io::Error::other("stdio proxy requires an iroh endpoint"))?;
        run_proxy(endpoint, node_id, stdin(), stdout()).await
    }
}

pub(crate) async fn run_proxy<T, Input, Output>(
    transport: &T,
    node_id: NodeId,
    input: Input,
    output: Output,
) -> AppResult<()>
where
    T: PeerTransport,
    Input: AsyncRead + Unpin + Send,
    Output: AsyncWrite + Unpin + Send,
{
    let remote = transport.connect(&node_id).await?;
    bridge(
        StreamParts {
            reader: input,
            writer: output,
        },
        remote,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "connect/tests.rs"]
mod tests;
