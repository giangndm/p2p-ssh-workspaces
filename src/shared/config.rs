use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use super::errors::{AppResult, ConfigError};
use super::types::{
    BackendAddress, Config, HostAlias, HostConfig, ListenConfig, NodeId, SshHost, StartupCommand,
    non_empty,
};

/// TOML configuration source.
pub struct TomlConfig;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    listen: RawListenConfig,
    #[serde(default)]
    host: BTreeMap<String, RawHostConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawListenConfig {
    backend: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHostConfig {
    node_id: Option<String>,
    ssh_host: Option<String>,
    startup: Option<Vec<String>>,
    reconnect: Option<bool>,
}

impl TomlConfig {
    /// Load and validate TOML without blocking the async runtime.
    pub async fn load_async(path: &Path) -> AppResult<Config> {
        let contents = tokio::fs::read_to_string(path).await?;
        Self::parse_file(&contents, path)
    }

    /// Parse and validate TOML using the supplied identity path.
    pub fn parse(contents: &str, identity_path: PathBuf) -> AppResult<Config> {
        let raw: RawConfig = toml::from_str(contents).map_err(ConfigError::from)?;
        let listen = ListenConfig {
            backend: BackendAddress::try_from(
                raw.listen
                    .backend
                    .unwrap_or_else(|| super::DEFAULT_BACKEND.to_owned()),
            )?,
            identity_path,
        };

        let hosts = raw
            .host
            .into_iter()
            .map(|(alias, raw_host)| build_host(alias, raw_host))
            .collect::<AppResult<BTreeMap<_, _>>>()?;

        Ok(Config { listen, hosts })
    }

    pub(crate) fn parse_file(contents: &str, path: &Path) -> AppResult<Config> {
        let identity_path = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("worker.key");
        Self::parse(contents, identity_path)
    }
}

fn build_host(alias: String, raw: RawHostConfig) -> AppResult<(HostAlias, HostConfig)> {
    let alias = HostAlias::try_from(alias)?;
    let node_id = NodeId::try_from(raw.node_id.ok_or(ConfigError::Missing("host.node_id"))?)?;
    let ssh_host = HostConfig {
        node_id,
        ssh_host: SshHost::try_from(raw.ssh_host.unwrap_or_else(|| alias.as_ref().to_owned()))?,
        startup: build_startup(raw.startup)?,
        reconnect: raw.reconnect.unwrap_or(true),
    };
    Ok((alias, ssh_host))
}

fn build_startup(startup: Option<Vec<String>>) -> Result<Option<StartupCommand>, ConfigError> {
    match startup.unwrap_or_default().as_slice() {
        [] => Ok(None),
        [program, args @ ..] => Ok(Some(StartupCommand {
            program: non_empty(program.clone(), "host.startup[0]")?,
            args: args.to_vec(),
        })),
    }
}

/// Resolve the default application configuration path.
pub fn default_config_path() -> AppResult<PathBuf> {
    Ok(default_config_dir()?.join("config.toml"))
}

/// Resolve the default persistent worker key path.
pub fn default_identity_path() -> AppResult<PathBuf> {
    Ok(default_config_dir()?.join("worker.key"))
}

pub(crate) fn default_config_dir() -> AppResult<PathBuf> {
    resolve_config_dir(
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

pub(crate) fn resolve_config_dir(
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> AppResult<PathBuf> {
    let config_dir = match xdg_config_home {
        Some(path) => absolute_path(path, "XDG_CONFIG_HOME")?,
        None => absolute_path(
            home.ok_or(ConfigError::Missing("HOME"))?.join(".config"),
            "HOME",
        )?,
    };
    Ok(config_dir.join("sshp"))
}

fn absolute_path(path: PathBuf, field: &'static str) -> AppResult<PathBuf> {
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return Err(ConfigError::Invalid(format!("{field} must be an absolute path")).into());
    }
    Ok(path)
}
