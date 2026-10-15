use std::{num::NonZeroU16, path::PathBuf, process};

use clap::{Args, Parser, Subcommand};

use ssh_workspaces::{
    config::{
        HostAlias, SshConfigEntry, default_ssh_config_path, disable_ssh_config_entry,
        install_service, uninstall_service, write_ssh_config_entry,
    },
    connect::{autoconnect, connect_stdio},
    listen::{BackendAddress, DEFAULT_BACKEND, Listener},
    shared::{AppError, AppResult, NodeId, RelayUrl, default_identity_path, load_node_id},
    trace::{format_trace_report, trace_node},
};

#[derive(Debug, Parser)]
#[command(
    name = "sshp",
    bin_name = "sshp",
    version,
    about = "P2P iroh pipe for OpenSSH"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the iroh listener and forward to a local SSH backend.
    Listen(ListenArgs),
    /// Bridge stdio to a worker iroh endpoint for OpenSSH ProxyCommand.
    Connect(ConnectArgs),
    /// Run an interactive SSH session and automatically reconnect when the connection drops.
    Autoconnect(AutoconnectArgs),
    /// Manage OpenSSH Host blocks (`enable`/`disable`) and the worker systemd service (`install`/`uninstall`).
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Probe connectivity and measure latency to a worker iroh endpoint.
    Trace(TraceArgs),
}

#[derive(Debug, Args)]
struct ListenArgs {
    /// Override the local SSH backend address (default: 127.0.0.1:22).
    #[arg(long, value_name = "ADDR")]
    backend: Option<BackendAddress>,

    /// Use a custom iroh relay server URL.
    #[arg(long, value_name = "URL")]
    relay: Option<RelayUrl>,

    /// Print the persistent endpoint ID and exit.
    #[arg(long, conflicts_with_all = ["backend", "relay", "trace"])]
    print_id: bool,

    /// Emit peer connection trace events to stderr.
    #[arg(long)]
    trace: bool,
}

#[derive(Debug, Args)]
struct ConnectArgs {
    /// Target worker iroh endpoint ID.
    #[arg(value_name = "NODE_ID")]
    node_id: NodeId,

    /// Use a custom iroh relay server URL.
    #[arg(long, value_name = "URL")]
    relay: Option<RelayUrl>,

    /// Emit iroh connection and byte-transfer diagnostics to stderr.
    #[arg(long)]
    trace: bool,
}

#[derive(Debug, Args)]
struct AutoconnectArgs {
    /// OpenSSH Host alias from `~/.ssh/config`.
    #[arg(value_name = "HOST")]
    host: HostAlias,

    /// Optional remote command to run with a PTY (for example: `tmux new -A -s main`).
    #[arg(value_name = "COMMAND")]
    command: Option<String>,

    /// Emit ssh invocation diagnostics to stderr.
    #[arg(long)]
    trace: bool,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Write or update an enabled OpenSSH Host block with ProxyCommand in `~/.ssh/config`.
    Enable(EnableHostArgs),
    /// Comment out ProxyCommand in a managed OpenSSH Host block while keeping other directives.
    Disable(DisableHostArgs),
    /// Install and start the worker systemd user service (`sshp-listen.service`).
    Install(InstallServiceArgs),
    /// Stop, disable, and remove the worker systemd user service (`sshp-listen.service`).
    Uninstall,
}

#[derive(Debug, Args)]
struct EnableHostArgs {
    /// OpenSSH Host alias.
    #[arg(value_name = "ALIAS")]
    alias: HostAlias,

    /// Target worker iroh endpoint ID.
    #[arg(value_name = "NODE_ID")]
    node_id: NodeId,

    /// SSH login user (`User` directive).
    #[arg(long, value_name = "USER")]
    user: Option<String>,

    /// SSH remote port (`Port` directive).
    #[arg(long, value_name = "PORT")]
    port: Option<NonZeroU16>,

    /// SSH identity file path (`IdentityFile` directive).
    #[arg(long, value_name = "PATH")]
    identity_file: Option<PathBuf>,

    /// Include `--relay <URL>` in the generated ProxyCommand.
    #[arg(long, value_name = "URL")]
    relay: Option<RelayUrl>,

    /// Include `--trace` in the generated ProxyCommand.
    #[arg(long)]
    trace: bool,

    /// Print the generated Host block to stdout without modifying the OpenSSH config file.
    #[arg(long, conflicts_with = "ssh_config")]
    dry_run: bool,

    /// Override the target OpenSSH config file path (default: `~/.ssh/config`).
    #[arg(long, value_name = "PATH")]
    ssh_config: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct DisableHostArgs {
    /// OpenSSH Host alias to disable.
    #[arg(value_name = "ALIAS")]
    alias: HostAlias,

    /// Override the target OpenSSH config file path (default: `~/.ssh/config`).
    #[arg(long, value_name = "PATH")]
    ssh_config: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct InstallServiceArgs {
    /// Pass `--relay <URL>` to `sshp listen` in the generated systemd unit.
    #[arg(long, value_name = "URL")]
    relay: Option<RelayUrl>,
}

#[derive(Debug, Args)]
struct TraceArgs {
    /// Target worker iroh endpoint ID.
    #[arg(value_name = "NODE_ID")]
    node_id: NodeId,

    /// Use a custom iroh relay server URL.
    #[arg(long, value_name = "URL")]
    relay: Option<RelayUrl>,
}

#[tokio::main]
async fn main() {
    match run(Cli::parse()).await {
        Ok(code) => process::exit(code),
        Err(error) => {
            eprintln!("sshp: {error}");
            process::exit(1);
        }
    }
}

async fn run(cli: Cli) -> AppResult<i32> {
    match cli.command {
        Command::Listen(args) => run_listen(args).await,
        Command::Connect(args) => {
            connect_stdio(args.node_id, args.relay, args.trace).await?;
            Ok(0)
        }
        Command::Autoconnect(args) => autoconnect(args.host, args.command, args.trace).await,
        Command::Config(command) => run_config(command).await,
        Command::Trace(args) => {
            let report = trace_node(args.node_id, args.relay).await?;
            print!("{}", format_trace_report(&report));
            Ok(0)
        }
    }
}

async fn run_listen(args: ListenArgs) -> AppResult<i32> {
    let identity_path = default_identity_path()?;
    if args.print_id {
        let node_id = load_node_id(&identity_path).await?;
        println!("{node_id}");
        return Ok(0);
    }

    let backend = match args.backend {
        Some(backend) => backend,
        None => DEFAULT_BACKEND.parse()?,
    };
    let listener = Listener::bind(&identity_path, backend, args.relay, args.trace).await?;
    eprintln!("sshp listen: endpoint id {}", listener.id());
    listener.run().await?;
    Ok(0)
}

async fn run_config(command: ConfigCommand) -> AppResult<i32> {
    match command {
        ConfigCommand::Enable(args) => {
            let entry = SshConfigEntry {
                alias: args.alias,
                node_id: args.node_id,
                executable: std::env::current_exe()?,
                user: args.user,
                port: args.port.map(NonZeroU16::get),
                identity_file: args.identity_file,
                relay: args.relay,
                trace: args.trace,
            };

            if args.dry_run {
                print!("{}", entry.render()?);
            } else {
                let target = match args.ssh_config {
                    Some(path) => path,
                    None => default_ssh_config_path()?,
                };
                let diff =
                    tokio::task::spawn_blocking(move || write_ssh_config_entry(&target, &entry))
                        .await
                        .map_err(|error| {
                            std::io::Error::other(format!("config write task failed: {error}"))
                        })??;
                print!("{diff}");
            }
            Ok(0)
        }
        ConfigCommand::Disable(args) => {
            let target = match args.ssh_config {
                Some(path) => path,
                None => default_ssh_config_path()?,
            };
            let diff =
                tokio::task::spawn_blocking(move || disable_ssh_config_entry(&target, &args.alias))
                    .await
                    .map_err(|error| {
                        std::io::Error::other(format!("config disable task failed: {error}"))
                    })??;
            print!("{diff}");
            Ok(0)
        }
        ConfigCommand::Install(args) => {
            let relay = args.relay;
            let diff = tokio::task::spawn_blocking(move || install_service(relay.as_ref()))
                .await
                .map_err(|error| AppError::Service(Box::new(error)))??;
            print!("{diff}");
            Ok(0)
        }
        ConfigCommand::Uninstall => {
            let diff = tokio::task::spawn_blocking(uninstall_service)
                .await
                .map_err(|error| AppError::Service(Box::new(error)))??;
            print!("{diff}");
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn sample_node_id() -> String {
        iroh::SecretKey::from([7; 32]).public().to_string()
    }

    #[test]
    fn parses_listen_connect_trace_and_autoconnect() {
        let node_id = sample_node_id();
        let listen_cli = Cli::try_parse_from([
            "sshp",
            "listen",
            "--relay",
            "https://relay.example.com:3340",
            "--trace",
        ])
        .expect("valid listen args");
        let Command::Listen(listen_args) = listen_cli.command else {
            panic!("expected listen command");
        };
        assert_eq!(listen_args.backend, None);
        assert_eq!(
            listen_args
                .relay
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("https://relay.example.com:3340/")
        );
        assert!(!listen_args.print_id);
        assert!(listen_args.trace);

        let connect_cli = Cli::try_parse_from([
            "sshp",
            "connect",
            &node_id,
            "--relay",
            "http://127.0.0.1:3340",
            "--trace",
        ])
        .expect("valid connect args");
        let Command::Connect(connect_args) = connect_cli.command else {
            panic!("expected connect command");
        };
        assert_eq!(connect_args.node_id.to_string(), node_id);
        assert_eq!(
            connect_args
                .relay
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("http://127.0.0.1:3340/")
        );
        assert!(connect_args.trace);

        let auto_cli = Cli::try_parse_from([
            "sshp",
            "autoconnect",
            "--trace",
            "hanoi",
            "tmux new -A -s work",
        ])
        .expect("valid autoconnect args");
        let Command::Autoconnect(auto_args) = auto_cli.command else {
            panic!("expected autoconnect command");
        };
        assert_eq!(auto_args.host.as_ref(), "hanoi");
        assert_eq!(auto_args.command.as_deref(), Some("tmux new -A -s work"));
        assert!(auto_args.trace);

        let default_auto =
            Cli::try_parse_from(["sshp", "autoconnect", "hanoi"]).expect("valid autoconnect host");
        let Command::Autoconnect(default_args) = default_auto.command else {
            panic!("expected autoconnect command");
        };
        assert_eq!(default_args.host.as_ref(), "hanoi");
        assert_eq!(default_args.command, None);
        assert!(!default_args.trace);

        let trace_cli = Cli::try_parse_from([
            "sshp",
            "trace",
            &node_id,
            "--relay",
            "https://relay.example.com",
        ])
        .expect("valid trace args");
        let Command::Trace(trace_args) = trace_cli.command else {
            panic!("expected trace command");
        };
        assert_eq!(trace_args.node_id.to_string(), node_id);
        assert_eq!(
            trace_args
                .relay
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("https://relay.example.com/")
        );
    }

    #[test]
    fn parses_config_enable_and_disable_options() {
        let node_id = sample_node_id();
        let cli = Cli::try_parse_from([
            "sshp",
            "config",
            "enable",
            "hanoi",
            &node_id,
            "--user",
            "giang",
            "--port",
            "2222",
            "--relay",
            "https://relay.example.com",
            "--dry-run",
        ])
        .expect("valid config enable args");
        let Command::Config(ConfigCommand::Enable(args)) = cli.command else {
            panic!("expected config enable command");
        };
        assert_eq!(args.alias.as_ref(), "hanoi");
        assert_eq!(args.node_id.to_string(), node_id);
        assert_eq!(args.user.as_deref(), Some("giang"));
        assert_eq!(args.port.map(NonZeroU16::get), Some(2222));
        assert_eq!(
            args.relay.as_ref().map(ToString::to_string).as_deref(),
            Some("https://relay.example.com/")
        );
        assert!(args.dry_run);

        let disable_cli = Cli::try_parse_from([
            "sshp",
            "config",
            "disable",
            "hanoi",
            "--ssh-config",
            "/tmp/ssh_config",
        ])
        .expect("valid config disable args");
        let Command::Config(ConfigCommand::Disable(disable_args)) = disable_cli.command else {
            panic!("expected config disable command");
        };
        assert_eq!(disable_args.alias.as_ref(), "hanoi");
        assert_eq!(
            disable_args.ssh_config,
            Some(PathBuf::from("/tmp/ssh_config"))
        );
    }

    #[test]
    fn parses_config_install_and_rejects_conflicting_flags() {
        let node_id = sample_node_id();
        let install = Cli::try_parse_from([
            "sshp",
            "config",
            "install",
            "--relay",
            "https://relay.example.com",
        ])
        .expect("valid config install");
        let Command::Config(ConfigCommand::Install(install_args)) = install.command else {
            panic!("expected config install");
        };
        assert_eq!(
            install_args
                .relay
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("https://relay.example.com/")
        );

        let uninstall =
            Cli::try_parse_from(["sshp", "config", "uninstall"]).expect("valid config uninstall");
        assert!(matches!(
            uninstall.command,
            Command::Config(ConfigCommand::Uninstall)
        ));

        for invalid in [
            vec![
                "sshp",
                "config",
                "enable",
                "hanoi",
                &node_id,
                "--dry-run",
                "--ssh-config",
                "/tmp/ssh_config",
            ],
            vec!["sshp", "listen", "--print-id", "--backend", "127.0.0.1:22"],
            vec![
                "sshp",
                "listen",
                "--print-id",
                "--relay",
                "https://relay.example.com",
            ],
        ] {
            assert!(Cli::try_parse_from(invalid).is_err());
        }
    }
}
