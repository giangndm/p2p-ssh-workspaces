use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{self, Command},
    time::{SystemTime, UNIX_EPOCH},
};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        for attempt in 0.. {
            let path = env::temp_dir().join(format!(
                "sshp-cli-test-{}-{suffix}-{attempt}",
                process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create temporary config directory: {error}"),
            }
        }
        unreachable!("temporary directory counter exhausted")
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn help_is_available_from_the_binary() {
    let output = Command::new(env!("CARGO_BIN_EXE_sshp"))
        .arg("--help")
        .output()
        .expect("run sshp help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("listen"));
    assert!(stdout.contains("connect"));
}

#[test]
fn connect_reports_an_unknown_alias_with_isolated_config() {
    let config_dir = TempDir::new();
    let sshp_dir = config_dir.path().join("sshp");
    fs::create_dir(&sshp_dir).expect("create sshp config directory");
    let node_id = iroh::SecretKey::from([7; 32]).public();
    fs::write(
        sshp_dir.join("config.toml"),
        format!("[host.present]\nnode_id = \"{node_id}\"\n"),
    )
    .expect("write temporary config");
    let output = Command::new(env!("CARGO_BIN_EXE_sshp"))
        .args(["connect", "missing"])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .output()
        .expect("run sshp connect");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown host alias"));
}
