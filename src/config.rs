//! OpenSSH `Host` block management (`enable`/`disable`) and systemd user-service management (`install`/`uninstall`) for `sshp config`.

use std::{
    fmt, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::shared::{
    AppError, AppResult, ConfigError, NodeId, RelayUrl, absolute_path, default_config_base_dir,
    non_empty, write_file_atomic,
};

const SERVICE_NAME: &str = "sshp-listen.service";
const SERVICE_MARKER: &str = "# Managed by sshp.\n";
const SERVICE_RESTART_DELAY: &str = "3";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// Validated OpenSSH host alias.
pub struct HostAlias(String);

impl fmt::Display for HostAlias {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for HostAlias {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for HostAlias {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = non_empty(value, "host alias")?;
        if value.starts_with('-') {
            return Err(ConfigError::Invalid(
                "host alias must not start with '-'".to_owned(),
            ));
        }
        if value
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '*' | '?' | '!' | '#' | '\'' | '"'))
        {
            return Err(ConfigError::Invalid(
                "host alias must not contain wildcards, quotes, comments, or control characters"
                    .to_owned(),
            ));
        }
        Ok(Self(value.to_owned()))
    }
}

/// Resolve the default OpenSSH user configuration path (`$HOME/.ssh/config`).
pub fn default_ssh_config_path() -> AppResult<PathBuf> {
    resolve_ssh_config_path(std::env::var_os("HOME").map(PathBuf::from))
}

pub(crate) fn resolve_ssh_config_path(home: Option<PathBuf>) -> AppResult<PathBuf> {
    let home = absolute_path(home.ok_or(ConfigError::Missing("HOME"))?, "HOME")?;
    Ok(home.join(".ssh").join("config"))
}

/// Managed OpenSSH host entry for `~/.ssh/config`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshConfigEntry {
    /// OpenSSH `Host` alias.
    pub alias: HostAlias,
    /// Target worker endpoint ID.
    pub node_id: NodeId,
    /// Path to the `sshp` executable used in `ProxyCommand`.
    pub executable: PathBuf,
    /// Optional SSH login user.
    pub user: Option<String>,
    /// Optional SSH remote port.
    pub port: Option<u16>,
    /// Optional SSH identity file path.
    pub identity_file: Option<PathBuf>,
    /// Optional custom iroh relay URL passed to `sshp connect`.
    pub relay: Option<RelayUrl>,
    /// Whether `ProxyCommand` should pass `--trace` to `sshp connect`.
    pub trace: bool,
}

impl SshConfigEntry {
    /// Render the managed `Host` block for `~/.ssh/config`.
    pub fn render(&self) -> AppResult<String> {
        let executable = self.executable.to_str().ok_or_else(|| {
            ConfigError::Invalid("sshp executable must be valid UTF-8".to_owned())
        })?;
        if executable.trim().is_empty() {
            return Err(
                ConfigError::Invalid("sshp executable must not be empty".to_owned()).into(),
            );
        }
        let quoted_exe = shell_quote(executable)?;
        let mut command_parts = vec![quoted_exe, "connect".to_owned()];
        if let Some(relay) = &self.relay {
            command_parts.push("--relay".to_owned());
            command_parts.push(shell_quote(&relay.to_string())?);
        }
        if self.trace {
            command_parts.push("--trace".to_owned());
        }
        command_parts.push(self.node_id.to_string());
        let proxy_command = command_parts.join(" ");

        let mut lines = vec![
            begin_marker(&self.alias),
            format!("Host {}", self.alias),
            format!("    ProxyCommand {proxy_command}"),
        ];

        if let Some(user) = &self.user {
            let user = non_empty(user, "ssh user")?;
            if user
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, '#' | '\'' | '"'))
            {
                return Err(ConfigError::Invalid(
                    "ssh user must not contain quotes, comments, or control characters".to_owned(),
                )
                .into());
            }
            lines.push(format!("    User {user}"));
        }

        if let Some(port) = self.port {
            if port == 0 {
                return Err(ConfigError::Invalid(
                    "ssh port must be between 1 and 65535".to_owned(),
                )
                .into());
            }
            lines.push(format!("    Port {port}"));
        }

        if let Some(identity_file) = &self.identity_file {
            let identity_str = identity_file.to_str().ok_or_else(|| {
                ConfigError::Invalid("identity file path must be valid UTF-8".to_owned())
            })?;
            let quoted_identity = ssh_config_quote(identity_str)?;
            lines.push(format!("    IdentityFile {quoted_identity}"));
        }

        lines.push(end_marker(&self.alias));
        Ok(format!("{}\n", lines.join("\n")))
    }
}

/// Insert or replace a managed `Host` block in an OpenSSH config document.
pub fn upsert_ssh_config(existing: &str, alias: &HostAlias, block: &str) -> AppResult<String> {
    let begin = begin_marker(alias);
    let end = end_marker(alias);
    let block = block.trim_end_matches('\n');

    let mut output: Vec<&str> = Vec::new();
    let mut inside_managed_block = false;
    let mut replaced = false;

    for line in existing.lines() {
        if !inside_managed_block && line.trim() == begin {
            inside_managed_block = true;
            if !replaced {
                output.extend(block.lines());
                replaced = true;
            }
            continue;
        }
        if inside_managed_block {
            if line.trim() == end {
                inside_managed_block = false;
            }
            continue;
        }
        output.push(line);
    }

    if inside_managed_block {
        return Err(ConfigError::Invalid(format!(
            "unterminated managed block '{begin}' in ssh config"
        ))
        .into());
    }

    if !replaced {
        if !output.is_empty() && !output.last().is_some_and(|line| line.is_empty()) {
            output.push("");
        }
        output.extend(block.lines());
    }

    Ok(format!("{}\n", output.join("\n")))
}

/// Comment out `ProxyCommand` inside a managed `Host` block while preserving other directives.
pub fn disable_ssh_config(existing: &str, alias: &HostAlias) -> AppResult<String> {
    let begin = begin_marker(alias);
    let end = end_marker(alias);

    let mut output: Vec<String> = Vec::new();
    let mut inside_managed_block = false;
    let mut found = false;

    for line in existing.lines() {
        if !inside_managed_block && line.trim() == begin {
            inside_managed_block = true;
            found = true;
            output.push(line.to_owned());
            continue;
        }
        if inside_managed_block {
            if line.trim() == end {
                inside_managed_block = false;
                output.push(line.to_owned());
                continue;
            }
            let trimmed = line.trim_start();
            if trimmed.starts_with("ProxyCommand ") {
                let indent = &line[..line.len() - trimmed.len()];
                output.push(format!("{indent}# {trimmed}"));
            } else {
                output.push(line.to_owned());
            }
            continue;
        }
        output.push(line.to_owned());
    }

    if inside_managed_block {
        return Err(ConfigError::Invalid(format!(
            "unterminated managed block '{begin}' in ssh config"
        ))
        .into());
    }

    if !found {
        return Err(ConfigError::Invalid(format!(
            "managed host '{alias}' not found in ssh config"
        ))
        .into());
    }

    Ok(format!("{}\n", output.join("\n")))
}

/// Atomically write or update a managed `Host` block in an OpenSSH config file
/// and return a unified diff of the change.
pub fn write_ssh_config_entry(path: &Path, entry: &SshConfigEntry) -> AppResult<String> {
    let block = entry.render()?;
    let existing = match fs::read_to_string(path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let old_block = existing
        .as_deref()
        .and_then(|contents| extract_managed_block(contents, &entry.alias));
    let updated = upsert_ssh_config(existing.as_deref().unwrap_or(""), &entry.alias, &block)?;
    write_file_atomic(path, updated.as_bytes(), true)?;
    let before = match (&existing, &old_block) {
        (None, _) => None,
        (Some(_), Some(previous)) => Some(previous.as_str()),
        (Some(_), None) => Some(""),
    };
    Ok(format_diff(path, before, Some(&block)))
}

/// Atomically comment out `ProxyCommand` for a managed `Host` block in an OpenSSH config file
/// and return a unified diff of the change.
pub fn disable_ssh_config_entry(path: &Path, alias: &HostAlias) -> AppResult<String> {
    let existing = fs::read_to_string(path)?;
    let updated = disable_ssh_config(&existing, alias)?;
    let old_block = extract_managed_block(&existing, alias);
    let new_block = extract_managed_block(&updated, alias);
    write_file_atomic(path, updated.as_bytes(), true)?;
    Ok(format_diff(
        path,
        old_block.as_deref(),
        new_block.as_deref(),
    ))
}

/// Render and install the systemd user unit (`sshp-listen.service`),
/// returning a unified diff of the unit file change.
pub fn install_service(relay: Option<&RelayUrl>) -> AppResult<String> {
    let executable = std::env::current_exe()?;
    if !executable.is_absolute() {
        return Err(service_error("service executable must be an absolute path"));
    }
    let config_dir = default_config_base_dir()?;
    let unit_path = default_service_unit_path()?;
    let existing = read_owned_unit(&unit_path)?;
    let unit = render_unit(&executable, &config_dir, relay)?;

    run_systemctl(["show", "--property=Version", "--value"])?;
    write_file_atomic(&unit_path, unit.as_bytes(), false)?;
    run_systemctl(["daemon-reload"])?;
    run_systemctl(["enable", "--now", SERVICE_NAME])?;
    Ok(format_diff(&unit_path, existing.as_deref(), Some(&unit)))
}

/// Stop, disable, and remove the managed systemd user unit (`sshp-listen.service`),
/// returning a unified diff of the unit file removal.
pub fn uninstall_service() -> AppResult<String> {
    let unit_path = default_service_unit_path()?;
    let existing = read_owned_unit(&unit_path)?;
    run_systemctl(["disable", "--now", SERVICE_NAME])?;
    match fs::remove_file(&unit_path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    run_systemctl(["daemon-reload"])?;
    Ok(format_diff(&unit_path, existing.as_deref(), None))
}

/// Format a `git diff`-style unified diff for a file change.
pub fn format_diff(path: &Path, before: Option<&str>, after: Option<&str>) -> String {
    let old_lines: Vec<&str> = before.unwrap_or("").lines().collect();
    let new_lines: Vec<&str> = after.unwrap_or("").lines().collect();
    let mut dp = vec![vec![0usize; new_lines.len() + 1]; old_lines.len() + 1];

    for i in (0..old_lines.len()).rev() {
        for j in (0..new_lines.len()).rev() {
            dp[i][j] = if old_lines[i] == new_lines[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut lines = vec![
        diff_header("---", 'a', path, before),
        diff_header("+++", 'b', path, after),
    ];
    let (mut i, mut j) = (0, 0);
    while i < old_lines.len() || j < new_lines.len() {
        if i < old_lines.len() && j < new_lines.len() && old_lines[i] == new_lines[j] {
            lines.push(format!(" {}", old_lines[i]));
            i += 1;
            j += 1;
        } else if i < old_lines.len() && (j == new_lines.len() || dp[i + 1][j] >= dp[i][j + 1]) {
            lines.push(format!("-{}", old_lines[i]));
            i += 1;
        } else {
            lines.push(format!("+{}", new_lines[j]));
            j += 1;
        }
    }

    format!("{}\n", lines.join("\n"))
}

fn diff_header(marker: &str, prefix: char, path: &Path, content: Option<&str>) -> String {
    match content {
        Some(_) => {
            let rendered = path.display().to_string();
            let sep = if rendered.starts_with('/') { "" } else { "/" };
            format!("{marker} {prefix}{sep}{rendered}")
        }
        None => format!("{marker} /dev/null"),
    }
}

fn extract_managed_block(existing: &str, alias: &HostAlias) -> Option<String> {
    let (begin, end) = (begin_marker(alias), end_marker(alias));
    let mut lines = Vec::new();
    let mut inside = false;
    for line in existing.lines() {
        if !inside && line.trim() == begin {
            inside = true;
            lines.push(line);
        } else if inside {
            lines.push(line);
            if line.trim() == end {
                return Some(format!("{}\n", lines.join("\n")));
            }
        }
    }
    None
}

/// Resolve the default systemd user-unit path.
pub fn default_service_unit_path() -> AppResult<PathBuf> {
    Ok(default_config_base_dir()?
        .join("systemd")
        .join("user")
        .join(SERVICE_NAME))
}

pub(crate) fn render_unit(
    executable: &Path,
    config_dir: &Path,
    relay: Option<&RelayUrl>,
) -> AppResult<String> {
    let executable = executable
        .to_str()
        .ok_or_else(|| service_error("service executable must be valid UTF-8"))?;
    let config_dir = config_dir
        .to_str()
        .ok_or_else(|| service_error("service config directory must be valid UTF-8"))?;
    let mut exec_start = format!("{} listen", systemd_quote(executable, true)?);
    if let Some(relay) = relay {
        exec_start.push_str(&format!(
            " --relay {}",
            systemd_quote(&relay.to_string(), true)?
        ));
    }
    Ok(format!(
        "{SERVICE_MARKER}[Unit]\nDescription=sshp iroh listener\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nEnvironment=XDG_CONFIG_HOME={}\nExecStart={exec_start}\nRestart=always\nRestartSec={}\n\n[Install]\nWantedBy=default.target\n",
        systemd_quote(config_dir, false)?,
        SERVICE_RESTART_DELAY,
    ))
}

fn read_owned_unit(path: &Path) -> AppResult<Option<String>> {
    match fs::read_to_string(path) {
        Ok(contents) if contents.starts_with(SERVICE_MARKER) => Ok(Some(contents)),
        Ok(_) => Err(service_error(
            "refusing to remove a service unit not created by sshp",
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn systemd_quote(value: &str, escape_dollar: bool) -> AppResult<String> {
    if value.chars().any(char::is_control) {
        return Err(service_error(
            "systemd value must not contain control characters",
        ));
    }
    let value = value.replace('%', "%%");
    let value = if escape_dollar {
        value.replace('$', "$$")
    } else {
        value
    };
    if value
        .chars()
        .all(|character| !character.is_whitespace() && character != '"' && character != '\\')
    {
        return Ok(value);
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn run_systemctl<const N: usize>(arguments: [&str; N]) -> AppResult<()> {
    let status = std::process::Command::new("systemctl")
        .arg("--user")
        .args(arguments)
        .status()
        .map_err(|error| service_error(format!("failed to run systemctl --user: {error}")))?;
    if status.success() {
        return Ok(());
    }
    Err(service_error(format!(
        "systemctl --user exited with {status}"
    )))
}

fn service_error(message: impl Into<String>) -> AppError {
    AppError::Service(Box::new(std::io::Error::other(message.into())))
}

fn begin_marker(alias: &HostAlias) -> String {
    format!("# BEGIN sshp {alias}")
}

fn end_marker(alias: &HostAlias) -> String {
    format!("# END sshp {alias}")
}

fn shell_quote(value: &str) -> Result<String, ConfigError> {
    if value.chars().any(char::is_control) {
        return Err(ConfigError::Invalid(
            "shell argument must not contain control characters".to_owned(),
        ));
    }
    let escaped = value.replace('%', "%%").replace('\'', "'\"'\"'");
    Ok(format!("'{escaped}'"))
}

fn ssh_config_quote(value: &str) -> Result<String, ConfigError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::Invalid(
            "identity file path must not be empty".to_owned(),
        ));
    }
    if value.chars().any(char::is_control) || value.contains('"') {
        return Err(ConfigError::Invalid(
            "identity file path must not contain quotes or control characters".to_owned(),
        ));
    }
    let escaped = value.replace('%', "%%");
    if escaped.chars().any(char::is_whitespace) {
        Ok(format!("\"{escaped}\""))
    } else {
        Ok(escaped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::TestDir;

    fn valid_node_id() -> NodeId {
        iroh::SecretKey::from([7; 32])
            .public()
            .to_string()
            .parse()
            .expect("valid node id")
    }

    #[test]
    fn host_alias_and_ssh_config_path_validation() {
        assert_eq!(
            "hanoi".parse::<HostAlias>().expect("valid alias").as_ref(),
            "hanoi"
        );
        for invalid in [
            "",
            "bad host",
            "-oProxyCommand=x",
            "bad\nalias",
            "*",
            "host?",
            "!host",
            "host#1",
            "host\"1",
            "host'1",
        ] {
            assert!(invalid.parse::<HostAlias>().is_err(), "{invalid:?}");
        }

        let ssh_config = resolve_ssh_config_path(Some(PathBuf::from("/home/user")))
            .expect("valid SSH config path");
        assert_eq!(ssh_config, PathBuf::from("/home/user/.ssh/config"));
        assert!(resolve_ssh_config_path(None).is_err());
        assert!(resolve_ssh_config_path(Some(PathBuf::from("relative"))).is_err());
    }

    #[test]
    fn renders_ssh_config_block_with_options_and_trace() {
        let node_id = valid_node_id();
        let entry = SshConfigEntry {
            alias: "hanoi".parse().expect("valid alias"),
            node_id,
            executable: PathBuf::from("/opt/ssh%workspaces/sshp"),
            user: Some("giang".to_owned()),
            port: Some(2222),
            identity_file: Some(PathBuf::from("/home/giang/.ssh/work %key")),
            relay: Some("https://relay.example.com:3340".parse().expect("relay")),
            trace: true,
        };

        let rendered = entry.render().expect("valid ssh config block");
        assert_eq!(
            rendered,
            format!(
                "# BEGIN sshp hanoi\nHost hanoi\n    ProxyCommand '/opt/ssh%%workspaces/sshp' connect --relay 'https://relay.example.com:3340/' --trace {node_id}\n    User giang\n    Port 2222\n    IdentityFile \"/home/giang/.ssh/work %%key\"\n# END sshp hanoi\n"
            )
        );
    }

    #[test]
    fn upserts_and_disables_ssh_config_block_idempotently() {
        let alias: HostAlias = "hanoi".parse().expect("valid alias");
        let first_block = "# BEGIN sshp hanoi\nHost hanoi\n    ProxyCommand 'sshp' connect node-1\n    User giang\n# END sshp hanoi\n";
        let second_block = "# BEGIN sshp hanoi\nHost hanoi\n    ProxyCommand 'sshp' connect node-2\n    User giang\n# END sshp hanoi\n";

        let initial = "Host github.com\n    User git\n";
        let appended = upsert_ssh_config(initial, &alias, first_block).expect("append block");
        assert_eq!(
            appended,
            "Host github.com\n    User git\n\n# BEGIN sshp hanoi\nHost hanoi\n    ProxyCommand 'sshp' connect node-1\n    User giang\n# END sshp hanoi\n"
        );

        let disabled = disable_ssh_config(&appended, &alias).expect("disable block");
        assert_eq!(
            disabled,
            "Host github.com\n    User git\n\n# BEGIN sshp hanoi\nHost hanoi\n    # ProxyCommand 'sshp' connect node-1\n    User giang\n# END sshp hanoi\n"
        );
        let disabled_twice =
            disable_ssh_config(&disabled, &alias).expect("disable block idempotently");
        assert_eq!(disabled_twice, disabled);

        let reenabled = upsert_ssh_config(&disabled, &alias, second_block).expect("reenable block");
        assert_eq!(
            reenabled,
            "Host github.com\n    User git\n\n# BEGIN sshp hanoi\nHost hanoi\n    ProxyCommand 'sshp' connect node-2\n    User giang\n# END sshp hanoi\n"
        );

        let unterminated = "# BEGIN sshp hanoi\nHost hanoi\n";
        assert!(upsert_ssh_config(unterminated, &alias, first_block).is_err());
        assert!(disable_ssh_config(unterminated, &alias).is_err());
        assert!(disable_ssh_config(initial, &alias).is_err());
    }

    #[test]
    fn writes_and_disables_ssh_config_atomically_with_restrictive_permissions() {
        let mut directory = TestDir::new();
        let ssh_config_path = directory.path().join(".ssh").join("config");
        let custom_config_path = directory.path().join("custom_ssh_config");
        let node_id = valid_node_id();
        let alias: HostAlias = "hanoi".parse().expect("valid alias");
        let entry = SshConfigEntry {
            alias: alias.clone(),
            node_id,
            executable: PathBuf::from("/usr/local/bin/sshp"),
            user: Some("giang".to_owned()),
            port: None,
            identity_file: None,
            relay: None,
            trace: false,
        };

        let create_diff =
            write_ssh_config_entry(&ssh_config_path, &entry).expect("write ssh config");
        write_ssh_config_entry(&custom_config_path, &entry).expect("write custom ssh config");
        let contents = fs::read_to_string(&ssh_config_path).expect("read ssh config");
        assert_eq!(contents, entry.render().expect("rendered entry"));
        assert!(create_diff.starts_with(&format!(
            "--- /dev/null\n+++ b{}\n+# BEGIN sshp hanoi\n+Host hanoi\n+    ProxyCommand '/usr/local/bin/sshp' connect {node_id}\n+    User giang\n+# END sshp hanoi\n",
            ssh_config_path.display()
        )));

        let disable_diff =
            disable_ssh_config_entry(&ssh_config_path, &alias).expect("disable ssh config");
        let disabled_contents =
            fs::read_to_string(&ssh_config_path).expect("read disabled ssh config");
        assert!(disabled_contents.contains(&format!(
            "    # ProxyCommand '/usr/local/bin/sshp' connect {node_id}"
        )));
        assert!(disabled_contents.contains("    User giang"));
        assert_eq!(
            disable_diff,
            format!(
                "--- a{path}\n+++ b{path}\n # BEGIN sshp hanoi\n Host hanoi\n-    ProxyCommand '/usr/local/bin/sshp' connect {node_id}\n+    # ProxyCommand '/usr/local/bin/sshp' connect {node_id}\n     User giang\n # END sshp hanoi\n",
                path = ssh_config_path.display()
            )
        );

        let remove_diff = format_diff(
            Path::new("/home/user/.config/systemd/user/sshp-listen.service"),
            Some("# Managed by sshp.\n[Unit]\n"),
            None,
        );
        assert_eq!(
            remove_diff,
            "--- a/home/user/.config/systemd/user/sshp-listen.service\n+++ /dev/null\n-# Managed by sshp.\n-[Unit]\n"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let file_mode = fs::metadata(&ssh_config_path)
                .expect("config metadata")
                .permissions()
                .mode();
            assert_eq!(file_mode & 0o777, 0o600);
        }

        directory.cleanup().expect("test directory cleanup");
    }

    #[test]
    fn renders_systemd_unit_with_quoted_executable_and_expansions() {
        let unit = render_unit(
            Path::new("/opt/ssh workspaces/sshp"),
            Path::new("/config/base"),
            None,
        )
        .expect("unit");
        assert!(unit.contains("ExecStart=\"/opt/ssh workspaces/sshp\" listen"));
        assert!(unit.contains("Environment=XDG_CONFIG_HOME=/config/base"));
        assert!(unit.contains("Restart=always"));
        assert!(unit.contains("WantedBy=default.target"));

        let relay: RelayUrl = "https://relay.example.com:3340".parse().expect("relay");
        let escaped = render_unit(
            Path::new("/opt/$HOME/sshp"),
            Path::new("/config/%root"),
            Some(&relay),
        )
        .expect("unit");
        assert!(escaped.contains("Environment=XDG_CONFIG_HOME=/config/%%root"));
        assert!(
            escaped.contains(
                "ExecStart=/opt/$$HOME/sshp listen --relay https://relay.example.com:3340/"
            )
        );
        assert!(systemd_quote("bad\nvalue", true).is_err());
    }
}
