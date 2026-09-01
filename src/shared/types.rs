use std::{collections::BTreeMap, net::Ipv6Addr, path::PathBuf, str::FromStr};

use derive_more::{Display, From, Into};

use super::errors::{AppResult, ConfigError};

#[derive(Clone, Copy, Debug, Display, From, Into, PartialEq, Eq, PartialOrd, Ord)]
#[display("{}", _0)]
/// Validated iroh endpoint identifier.
pub struct NodeId(iroh::EndpointId);

#[derive(Clone, Debug, Display, PartialEq, Eq, PartialOrd, Ord)]
#[display("{}", _0)]
/// Validated access configuration alias.
pub struct HostAlias(String);

#[derive(Clone, Debug, Display, PartialEq, Eq, PartialOrd, Ord)]
#[display("{}", _0)]
/// Validated OpenSSH host name.
pub struct SshHost(String);

#[derive(Clone, Debug, Display, PartialEq, Eq)]
#[display("{}", _0)]
/// Validated local backend address.
pub struct BackendAddress(String);

impl AsRef<iroh::EndpointId> for NodeId {
    fn as_ref(&self) -> &iroh::EndpointId {
        &self.0
    }
}

impl TryFrom<String> for NodeId {
    type Error = ConfigError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let value = non_empty(value, "host.node_id")?;
        iroh::EndpointId::from_str(&value)
            .map(Self)
            .map_err(ConfigError::InvalidNodeId)
    }
}

impl TryFrom<String> for HostAlias {
    type Error = ConfigError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let value = non_empty(value, "host alias")?;
        if value.starts_with('-') {
            return Err(ConfigError::Invalid(
                "host alias must not start with '-'".to_owned(),
            ));
        }
        Ok(Self(value))
    }
}

impl TryFrom<String> for SshHost {
    type Error = ConfigError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let value = non_empty(value, "host.ssh_host")?;
        if value.starts_with('-') {
            return Err(ConfigError::Invalid(
                "host.ssh_host must not start with '-'".to_owned(),
            ));
        }
        Ok(Self(value))
    }
}

impl TryFrom<String> for BackendAddress {
    type Error = ConfigError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let value = non_empty(value, "listen.backend")?;
        let (host, port) = if let Some(value) = value.strip_prefix('[') {
            let Some((host, port)) = value.split_once("]:") else {
                return Err(ConfigError::Invalid(
                    "listen.backend must use host:port format".to_owned(),
                ));
            };
            if host.parse::<Ipv6Addr>().is_err() {
                return Err(ConfigError::Invalid(
                    "listen.backend IPv6 host is invalid".to_owned(),
                ));
            }
            (host, port)
        } else {
            let Some((host, port)) = value.rsplit_once(':') else {
                return Err(ConfigError::Invalid(
                    "listen.backend must use host:port format".to_owned(),
                ));
            };
            if host.contains(':') || !valid_hostname(host) {
                return Err(ConfigError::Invalid(
                    "listen.backend host must be a hostname or IPv4 address".to_owned(),
                ));
            }
            (host, port)
        };
        if host.trim().is_empty() || host.chars().any(char::is_whitespace) {
            return Err(ConfigError::Invalid(
                "listen.backend host must not be empty or contain whitespace".to_owned(),
            ));
        }
        if port.parse::<u16>().is_err() || port == "0" {
            return Err(ConfigError::Invalid(
                "listen.backend port must be between 1 and 65535".to_owned(),
            ));
        }
        Ok(Self(value))
    }
}

impl AsRef<str> for HostAlias {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for SshHost {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for BackendAddress {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Command and arguments started after SSH login.
pub struct StartupCommand {
    /// Executable name or path.
    pub program: String,
    /// Arguments passed to the executable.
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Configuration for one access host.
pub struct HostConfig {
    /// Worker endpoint identifier.
    pub node_id: NodeId,
    /// OpenSSH host name.
    pub ssh_host: SshHost,
    /// Optional login startup command.
    pub startup: Option<StartupCommand>,
    /// Whether the access process retries a disconnect.
    pub reconnect: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Listener configuration.
pub struct ListenConfig {
    /// Local SSH backend address.
    pub backend: BackendAddress,
    /// Persistent iroh secret key path.
    pub identity_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Complete application configuration.
pub struct Config {
    /// Listener settings.
    pub listen: ListenConfig,
    /// Access hosts indexed by alias.
    pub hosts: BTreeMap<HostAlias, HostConfig>,
}

impl Config {
    /// Return a configured host or a descriptive lookup error.
    pub fn host(&self, alias: &HostAlias) -> AppResult<HostConfig> {
        self.hosts
            .get(alias)
            .cloned()
            .ok_or_else(|| ConfigError::UnknownHost(alias.to_string()).into())
    }
}

pub(crate) fn non_empty(value: String, field: &'static str) -> Result<String, ConfigError> {
    if value.trim().is_empty() || value.chars().any(char::is_whitespace) {
        return Err(ConfigError::Invalid(format!(
            "{field} must not be empty or contain whitespace"
        )));
    }
    Ok(value)
}

pub(crate) fn valid_hostname(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}
