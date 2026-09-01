//! Debian/Ubuntu systemd user-service support.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::shared::{AppError, AppResult, default_config_path};

const SERVICE_NAME: &str = "sshp-listen.service";
const SERVICE_MARKER: &str = "# Managed by sshp.\n";
const SERVICE_RESTART_DELAY: &str = "3";
static SERVICE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Inputs needed to install the worker service.
pub struct ServiceRequest {
    /// Absolute path to the `sshp` executable.
    pub executable: PathBuf,
    /// Generated systemd user unit path.
    pub unit_path: PathBuf,
    /// Config base directory exported to the user service.
    pub config_dir: PathBuf,
}

/// System service installation boundary.
pub trait ServiceInstaller {
    /// Render and install a systemd user unit without changing config or keys.
    fn install(&self, request: &ServiceRequest) -> AppResult<()>;

    /// Stop/disable the unit and remove only the unit owned by this tool.
    fn uninstall(&self, unit_path: &Path) -> AppResult<()>;
}

/// Debian/Ubuntu systemd user-service installer.
pub struct SystemdInstaller;

impl ServiceInstaller for SystemdInstaller {
    fn install(&self, request: &ServiceRequest) -> AppResult<()> {
        validate_service_request(request)?;
        run_systemctl(["show", "--property=Version", "--value"])?;
        write_unit_atomic(
            &request.unit_path,
            &render_unit(&request.executable, &request.config_dir)?,
        )?;
        run_systemctl(["daemon-reload"])?;
        run_systemctl(["enable", "--now", SERVICE_NAME])?;
        Ok(())
    }

    fn uninstall(&self, unit_path: &Path) -> AppResult<()> {
        validate_unit_path(unit_path)?;
        ensure_owned_unit(unit_path)?;
        run_systemctl(["disable", "--now", SERVICE_NAME])?;
        match fs::remove_file(unit_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        run_systemctl(["daemon-reload"])?;
        Ok(())
    }
}

/// Resolve the default systemd user-unit path.
pub fn default_service_unit_path() -> AppResult<PathBuf> {
    Ok(default_config_base_dir()?
        .join("systemd")
        .join("user")
        .join(SERVICE_NAME))
}

fn default_config_base_dir() -> AppResult<PathBuf> {
    let config_path = default_config_path()?;
    config_path
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| std::io::Error::other("config path has no parent").into())
}

pub(crate) fn validate_service_request(request: &ServiceRequest) -> AppResult<()> {
    if !request.executable.is_absolute() {
        return Err(AppError::Service(Box::new(std::io::Error::other(
            "service executable must be an absolute path",
        ))));
    }
    if request.config_dir != default_config_base_dir()? {
        return Err(AppError::Service(Box::new(std::io::Error::other(
            "service config directory must be the default config root",
        ))));
    }
    validate_unit_path(&request.unit_path)
}

pub(crate) fn validate_unit_path(path: &Path) -> AppResult<()> {
    if !path.is_absolute() || path != default_service_unit_path()? {
        return Err(AppError::Service(Box::new(std::io::Error::other(
            "unit path must be the default sshp-listen.service path",
        ))));
    }
    Ok(())
}

pub(crate) fn render_unit(executable: &Path, config_dir: &Path) -> AppResult<String> {
    let executable = executable.to_str().ok_or_else(|| {
        AppError::Service(Box::new(std::io::Error::other(
            "service executable must be valid UTF-8",
        )))
    })?;
    let config_dir = config_dir.to_str().ok_or_else(|| {
        AppError::Service(Box::new(std::io::Error::other(
            "service config directory must be valid UTF-8",
        )))
    })?;
    Ok(format!(
        "{SERVICE_MARKER}[Unit]\nDescription=sshp iroh listener\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nEnvironment=XDG_CONFIG_HOME={}\nExecStart={} listen\nRestart=always\nRestartSec={}\n\n[Install]\nWantedBy=default.target\n",
        systemd_quote(config_dir, false)?,
        systemd_quote(executable, true)?,
        SERVICE_RESTART_DELAY,
    ))
}

fn ensure_owned_unit(path: &Path) -> AppResult<()> {
    match fs::read_to_string(path) {
        Ok(contents) if contents.starts_with(SERVICE_MARKER) => Ok(()),
        Ok(_) => Err(AppError::Service(Box::new(std::io::Error::other(
            "refusing to remove a service unit not created by sshp",
        )))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn systemd_quote(value: &str, escape_dollar: bool) -> AppResult<String> {
    if value.chars().any(char::is_control) {
        return Err(AppError::Service(Box::new(std::io::Error::other(
            "systemd value must not contain control characters",
        ))));
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

fn write_unit_atomic(path: &Path, contents: &str) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("unit path has no parent"))?;
    fs::create_dir_all(parent)?;
    let counter = SERVICE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = path.with_file_name(format!(
        ".{}-tmp-{}-{counter}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(SERVICE_NAME),
        std::process::id()
    ));
    let mut created = false;
    let result = (|| -> AppResult<()> {
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => {
                created = true;
                file
            }
            Err(error) => return Err(error.into()),
        };
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, path)?;
        Ok(())
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn run_systemctl<const N: usize>(arguments: [&str; N]) -> AppResult<()> {
    let status = std::process::Command::new("systemctl")
        .arg("--user")
        .args(arguments)
        .status()
        .map_err(|error| {
            AppError::Service(Box::new(std::io::Error::new(
                error.kind(),
                format!("failed to run systemctl --user: {error}"),
            )))
        })?;
    if status.success() {
        return Ok(());
    }
    Err(AppError::Service(Box::new(std::io::Error::other(format!(
        "systemctl --user exited with {status}"
    )))))
}
