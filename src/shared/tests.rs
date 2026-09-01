use std::{fs, path::PathBuf, sync::atomic::Ordering};

use super::identity::load_identity;
use super::{BackendAddress, ConfigError, HostAlias, NodeId, TomlConfig};

fn valid_node_id() -> String {
    iroh::SecretKey::from([7; 32]).public().to_string()
}

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        loop {
            let counter = super::IDENTITY_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("sshp-test-{}-{counter}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("test directory: {error}"),
            }
        }
    }

    fn path(&self) -> &PathBuf {
        &self.0
    }

    fn cleanup(&mut self) -> std::io::Result<()> {
        fs::remove_dir_all(&self.0)?;
        self.0 = PathBuf::new();
        Ok(())
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

#[test]
fn node_id_supports_display_and_conversions() {
    let endpoint_id = iroh::SecretKey::generate().public();
    let node_id = NodeId::from(endpoint_id);
    let borrowed: &iroh::EndpointId = node_id.as_ref();

    assert_eq!(borrowed, &endpoint_id);
    let owned: iroh::EndpointId = node_id.into();
    assert_eq!(owned, endpoint_id);
}

#[test]
fn backend_address_supports_display_and_conversions() {
    let address =
        BackendAddress::try_from(String::from("127.0.0.1:22")).expect("valid backend address");
    let borrowed: &str = address.as_ref();

    assert_eq!(borrowed, "127.0.0.1:22");
    assert_eq!(address.to_string(), "127.0.0.1:22");
}

#[test]
fn resolves_config_paths_with_xdg_precedence() {
    let xdg = super::resolve_config_dir(
        Some(PathBuf::from("/xdg")),
        Some(PathBuf::from("/home/user")),
    )
    .expect("valid XDG path");
    assert_eq!(xdg, PathBuf::from("/xdg/sshp"));
    assert_eq!(
        xdg.join("config.toml"),
        PathBuf::from("/xdg/sshp/config.toml")
    );
    assert_eq!(
        xdg.join("worker.key"),
        PathBuf::from("/xdg/sshp/worker.key")
    );

    let fallback = super::resolve_config_dir(None, Some(PathBuf::from("/home/user")))
        .expect("valid HOME path");
    assert_eq!(fallback, PathBuf::from("/home/user/.config/sshp"));
}

#[test]
fn rejects_invalid_config_paths() {
    for (xdg, home) in [
        (
            Some(PathBuf::from("relative")),
            Some(PathBuf::from("/home/user")),
        ),
        (None, Some(PathBuf::from("relative"))),
        (None, None),
    ] {
        assert!(super::resolve_config_dir(xdg, home).is_err());
    }
}

#[test]
fn loads_custom_config_with_sibling_identity_path() {
    let mut directory = TestDir::new();
    let identity_path = directory.path().join("worker.key");
    let config_path = directory.path().join("config.toml");
    fs::write(
        &config_path,
        format!("[host.hanoi]\nnode_id = \"{}\"\n", valid_node_id()),
    )
    .expect("config file");

    let contents = fs::read_to_string(&config_path).expect("config file");
    let config = TomlConfig::parse_file(&contents, &config_path).expect("custom config");
    assert_eq!(config.listen.identity_path, identity_path);
    directory.cleanup().expect("test directory cleanup");
}

#[test]
fn parses_defaults_and_startup_arguments() {
    let node_id = valid_node_id();
    let config = TomlConfig::parse(
        &format!(
            r#"
            [host.hanoi]
            node_id = "{node_id}"
            startup = ["tmux", "new-session", "-A", "-s", "hanoi"]
            "#
        ),
        PathBuf::from("/config/sshp/worker.key"),
    )
    .expect("valid config");

    let host = config
        .hosts
        .get(&HostAlias::try_from("hanoi".to_owned()).expect("valid alias"));
    assert_eq!(config.listen.backend.as_ref(), "127.0.0.1:22");
    assert_eq!(host.expect("hanoi host").ssh_host.as_ref(), "hanoi");
    assert_eq!(
        host.expect("hanoi host").startup,
        Some(super::StartupCommand {
            program: "tmux".to_owned(),
            args: vec![
                "new-session".to_owned(),
                "-A".to_owned(),
                "-s".to_owned(),
                "hanoi".to_owned(),
            ],
        })
    );
}

#[test]
fn parses_custom_host_values() {
    let node_id = valid_node_id();
    let config = TomlConfig::parse(
        &format!(
            r#"
            [listen]
            backend = "localhost:2200"

            [host.ide]
            node_id = "{node_id}"
            ssh_host = "ide-host"
            reconnect = false
            "#
        ),
        PathBuf::from("/config/sshp/worker.key"),
    )
    .expect("valid config");

    let host = config
        .hosts
        .get(&HostAlias::try_from("ide".to_owned()).expect("valid alias"))
        .expect("ide host");
    assert_eq!(config.listen.backend.as_ref(), "localhost:2200");
    assert_eq!(host.ssh_host.as_ref(), "ide-host");
    assert!(!host.reconnect);
    assert_eq!(
        config
            .host(&HostAlias::try_from("ide".to_owned()).expect("valid alias"))
            .expect("configured host"),
        *host
    );
    assert!(matches!(
        config.host(&HostAlias::try_from("missing".to_owned()).expect("valid alias")),
        Err(super::AppError::Config(ConfigError::UnknownHost(alias)))
            if alias == "missing"
    ));
}

#[test]
fn empty_startup_means_login_shell() {
    let node_id = valid_node_id();
    let config = TomlConfig::parse(
        &format!(
            r#"
            [host.shell]
            node_id = "{node_id}"
            startup = []
            "#
        ),
        PathBuf::from("/config/sshp/worker.key"),
    )
    .expect("valid config");

    assert!(
        config
            .hosts
            .get(&HostAlias::try_from("shell".to_owned()).expect("valid alias"))
            .expect("shell host")
            .startup
            .is_none()
    );
}

#[test]
fn rejects_missing_node_id_and_empty_startup_program() {
    let node_id = valid_node_id();
    let missing_node_id = TomlConfig::parse(
        "[host.hanoi]\nstartup = [\"tmux\"]",
        PathBuf::from("/config/sshp/worker.key"),
    );
    assert!(matches!(
        missing_node_id,
        Err(super::AppError::Config(super::ConfigError::Missing(
            "host.node_id"
        )))
    ));

    let empty_program = TomlConfig::parse(
        &format!("[host.hanoi]\nnode_id = \"{node_id}\"\nstartup = [\"\"]"),
        PathBuf::from("/config/sshp/worker.key"),
    );
    assert!(matches!(empty_program, Err(super::AppError::Config(_))));

    let empty_backend = TomlConfig::parse(
        "[listen]\nbackend = \"\"",
        PathBuf::from("/config/sshp/worker.key"),
    );
    assert!(matches!(empty_backend, Err(super::AppError::Config(_))));

    let unknown_field = TomlConfig::parse(
        &format!("[host.hanoi]\nnode_id = \"{node_id}\"\nunknown = true"),
        PathBuf::from("/config/sshp/worker.key"),
    );
    assert!(matches!(unknown_field, Err(super::AppError::Config(_))));

    let invalid_node_id = TomlConfig::parse(
        "[host.hanoi]\nnode_id = \"not-an-endpoint-id\"",
        PathBuf::from("/config/sshp/worker.key"),
    );
    assert!(matches!(
        invalid_node_id,
        Err(super::AppError::Config(super::ConfigError::InvalidNodeId(
            _
        )))
    ));

    let invalid_backend = TomlConfig::parse(
        "[listen]\nbackend = \"localhost\"",
        PathBuf::from("/config/sshp/worker.key"),
    );
    assert!(matches!(invalid_backend, Err(super::AppError::Config(_))));

    for backend in ["foo:22:33", "[not-ipv6]:22", "localhost:0"] {
        let result = TomlConfig::parse(
            &format!("[listen]\nbackend = \"{backend}\""),
            PathBuf::from("/config/sshp/worker.key"),
        );
        assert!(
            matches!(result, Err(super::AppError::Config(_))),
            "{backend}"
        );
    }

    let whitespace_host = TomlConfig::parse(
        &format!("[host.\"bad host\"]\nnode_id = \"{}\"", valid_node_id()),
        PathBuf::from("/config/sshp/worker.key"),
    );
    assert!(matches!(whitespace_host, Err(super::AppError::Config(_))));

    let option_host = TomlConfig::parse(
        &format!(
            "[host.hanoi]\nnode_id = \"{}\"\nssh_host = \"-oProxyCommand=x\"",
            valid_node_id()
        ),
        PathBuf::from("/config/sshp/worker.key"),
    );
    assert!(matches!(option_host, Err(super::AppError::Config(_))));

    let option_alias = TomlConfig::parse(
        &format!(
            "[host.\"-oProxyCommand=x\"]\nnode_id = \"{}\"",
            valid_node_id()
        ),
        PathBuf::from("/config/sshp/worker.key"),
    );
    assert!(matches!(option_alias, Err(super::AppError::Config(_))));
}

#[tokio::test]
async fn identity_is_persistent_and_concurrent_creation_is_safe() {
    let mut directory = TestDir::new();
    let path = directory.path().join("worker.key");
    let (first, second) = tokio::join!(load_identity(&path), load_identity(&path));
    let first = first.expect("first identity");
    let second = second.expect("second identity");

    assert_eq!(first.public(), second.public());
    assert_eq!(
        tokio::fs::read(&path).await.expect("identity file").len(),
        32
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = tokio::fs::metadata(&path)
            .await
            .expect("identity metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    directory.cleanup().expect("test directory cleanup");
}
