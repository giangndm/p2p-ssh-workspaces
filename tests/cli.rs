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
fn help_lists_all_four_subcommands() {
    let output = Command::new(env!("CARGO_BIN_EXE_sshp"))
        .arg("--help")
        .output()
        .expect("run sshp help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("listen"));
    assert!(stdout.contains("connect"));
    assert!(stdout.contains("config"));
    assert!(stdout.contains("trace"));
}

#[test]
fn config_enables_and_disables_managed_ssh_config_block_with_log_output() {
    let temp = TempDir::new();
    let ssh_config = temp.path().join(".ssh").join("config");
    let node_id = iroh::SecretKey::from([7; 32]).public().to_string();

    let dry_run_output = Command::new(env!("CARGO_BIN_EXE_sshp"))
        .args([
            "config",
            "enable",
            "hanoi",
            &node_id,
            "--user",
            "giang",
            "--relay",
            "https://relay.example.com:3340",
            "--trace",
            "--dry-run",
        ])
        .output()
        .expect("run sshp config enable --dry-run");

    assert!(dry_run_output.status.success());
    let rendered_block = String::from_utf8_lossy(&dry_run_output.stdout);
    assert!(rendered_block.contains("Host hanoi"));
    assert!(rendered_block.contains(&format!(
        "connect --relay 'https://relay.example.com:3340/' --trace {node_id}"
    )));
    assert!(rendered_block.contains("User giang"));

    let enable_output = Command::new(env!("CARGO_BIN_EXE_sshp"))
        .args([
            "config",
            "enable",
            "hanoi",
            &node_id,
            "--user",
            "giang",
            "--relay",
            "https://relay.example.com:3340",
            "--trace",
            "--ssh-config",
        ])
        .arg(&ssh_config)
        .output()
        .expect("run sshp config enable");

    assert!(enable_output.status.success());
    let enable_stdout = String::from_utf8_lossy(&enable_output.stdout);
    assert!(enable_stdout.contains("--- /dev/null\n"));
    assert!(enable_stdout.contains(&format!("+++ b{}\n", ssh_config.display())));
    assert!(enable_stdout.contains("+# BEGIN sshp hanoi\n"));
    assert!(enable_stdout.contains("+Host hanoi\n"));
    assert!(enable_stdout.contains("+    User giang\n"));
    let written = fs::read_to_string(&ssh_config).expect("read written ssh config");
    assert_eq!(written, rendered_block);

    let disable_output = Command::new(env!("CARGO_BIN_EXE_sshp"))
        .args(["config", "disable", "hanoi", "--ssh-config"])
        .arg(&ssh_config)
        .output()
        .expect("run sshp config disable");

    assert!(disable_output.status.success());
    let disable_stdout = String::from_utf8_lossy(&disable_output.stdout);
    let disabled = fs::read_to_string(&ssh_config).expect("read disabled ssh config");
    assert!(disable_stdout.contains(&format!("--- a{}\n", ssh_config.display())));
    assert!(disable_stdout.contains(&format!("+++ b{}\n", ssh_config.display())));
    assert!(disable_stdout.contains("-    ProxyCommand "));
    assert!(disable_stdout.contains("+    # ProxyCommand "));
    assert!(disabled.contains("Host hanoi"));
    assert!(disabled.contains("    # ProxyCommand "));
    assert!(disabled.contains("    User giang"));
}

#[test]
fn connect_and_trace_reject_invalid_endpoint_ids_and_relays_before_network() {
    let valid_node_id = iroh::SecretKey::from([7; 32]).public().to_string();
    for subcommand in ["connect", "trace"] {
        let output = Command::new(env!("CARGO_BIN_EXE_sshp"))
            .args([subcommand, "not-an-endpoint-id"])
            .output()
            .expect("run sshp with invalid node id");

        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("invalid endpoint ID"), "{stderr}");

        let invalid_relay = Command::new(env!("CARGO_BIN_EXE_sshp"))
            .args([subcommand, &valid_node_id, "--relay", "ftp://bad-relay"])
            .output()
            .expect("run sshp with invalid relay url");

        assert!(!invalid_relay.status.success());
        let relay_stderr = String::from_utf8_lossy(&invalid_relay.stderr);
        assert!(
            relay_stderr.contains("relay_url must use http or https scheme"),
            "{relay_stderr}"
        );
    }
}

#[test]
fn print_id_creates_and_reuses_persistent_worker_key() {
    let temp = TempDir::new();
    let first = Command::new(env!("CARGO_BIN_EXE_sshp"))
        .args(["listen", "--print-id"])
        .env("XDG_CONFIG_HOME", temp.path())
        .output()
        .expect("first sshp listen --print-id");
    let second = Command::new(env!("CARGO_BIN_EXE_sshp"))
        .args(["listen", "--print-id"])
        .env("XDG_CONFIG_HOME", temp.path())
        .output()
        .expect("second sshp listen --print-id");

    assert!(first.status.success());
    assert!(second.status.success());
    let first_id = String::from_utf8_lossy(&first.stdout).trim().to_owned();
    let second_id = String::from_utf8_lossy(&second.stdout).trim().to_owned();
    assert!(!first_id.is_empty());
    assert_eq!(first_id, second_id);
    assert!(temp.path().join("sshp").join("worker.key").exists());
}
