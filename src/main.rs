use std::{ffi::OsString, path::Path, process, time::Duration};

use clap::{Args, Parser, Subcommand};

use ssh_workspaces::{
    connect::{Connector, RetryPolicy},
    listen::{
        Listener, ServiceInstaller, ServiceRequest, SystemdInstaller, default_service_unit_path,
    },
    shared::{
        AppError, AppResult, BackendAddress, DEFAULT_BACKEND, HostAlias, IrohEndpoint,
        ListenConfig, TomlConfig, default_config_path, default_identity_path,
    },
};

const DEFAULT_RETRY_DELAY: Duration = Duration::from_secs(1);
const DEFAULT_MAX_RETRIES: u32 = 16;

#[derive(Debug, Parser)]
#[command(
    name = "sshp",
    bin_name = "sshp",
    version,
    about = "P2P SSH workspace client"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the iroh listener and forward to a local SSH backend.
    Listen(ListenArgs),
    /// Connect to a configured workspace alias.
    Connect(ConnectArgs),
}

#[derive(Debug, Args)]
struct ListenArgs {
    /// Override the local SSH backend address.
    #[arg(long, value_name = "ADDR")]
    backend: Option<String>,

    /// Print the persistent endpoint ID and exit.
    #[arg(
        long,
        conflicts_with_all = ["install_service", "uninstall_service", "backend"]
    )]
    print_id: bool,

    /// Install and start the systemd user service.
    #[arg(long, conflicts_with_all = ["uninstall_service", "backend"])]
    install_service: bool,

    /// Stop, disable, and remove the systemd user service.
    #[arg(long, conflicts_with_all = ["install_service", "backend"])]
    uninstall_service: bool,
}

#[derive(Debug, Args)]
struct ConnectArgs {
    /// Configured workspace alias.
    #[arg(value_name = "ALIAS")]
    alias: String,

    /// Internal stdio proxy mode used by OpenSSH ProxyCommand.
    #[arg(long, hide = true)]
    stdio: bool,

    /// Disable reconnect for this invocation.
    #[arg(long)]
    no_reconnect: bool,

    /// Arguments passed to OpenSSH after `--`.
    #[arg(last = true, allow_hyphen_values = true)]
    ssh_args: Vec<OsString>,
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
        Command::Connect(args) => run_connect(args).await,
    }
}

async fn run_listen(args: ListenArgs) -> AppResult<i32> {
    if args.install_service {
        install_service(service_request()?).await?;
        return Ok(0);
    }
    if args.uninstall_service {
        uninstall_service(default_service_unit_path()?).await?;
        return Ok(0);
    }

    let print_id = args.print_id;
    let config = load_listen_config(&args).await?;
    if print_id {
        let endpoint = IrohEndpoint::bind(&config.identity_path).await?;
        println!("{}", endpoint.id());
        return Ok(0);
    }

    let listener = Listener::from_config(config).await?;
    eprintln!("sshp listen: endpoint id {}", listener.id());
    listener.run().await?;
    Ok(0)
}

fn service_request() -> AppResult<ServiceRequest> {
    let config_path = default_config_path()?;
    let config_dir = config_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| std::io::Error::other("config path has no parent"))?
        .to_owned();
    Ok(ServiceRequest {
        executable: std::env::current_exe()?,
        unit_path: default_service_unit_path()?,
        config_dir,
    })
}

async fn install_service(request: ServiceRequest) -> AppResult<()> {
    tokio::task::spawn_blocking(move || SystemdInstaller.install(&request))
        .await
        .map_err(|error| AppError::Service(Box::new(error)))??;
    Ok(())
}

async fn uninstall_service(unit_path: std::path::PathBuf) -> AppResult<()> {
    tokio::task::spawn_blocking(move || SystemdInstaller.uninstall(&unit_path))
        .await
        .map_err(|error| AppError::Service(Box::new(error)))??;
    Ok(())
}

async fn load_listen_config(args: &ListenArgs) -> AppResult<ListenConfig> {
    let configured = match TomlConfig::load_async(&default_config_path()?).await {
        Ok(config) => Some(config.listen),
        Err(AppError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let backend = match args.backend.clone() {
        Some(backend) => BackendAddress::try_from(backend)?,
        None => match configured.as_ref() {
            Some(config) => config.backend.clone(),
            None => BackendAddress::try_from(DEFAULT_BACKEND.to_owned())?,
        },
    };
    let identity_path = configured
        .map(|config| config.identity_path)
        .unwrap_or(default_identity_path()?);
    Ok(ListenConfig {
        backend,
        identity_path,
    })
}

async fn run_connect(args: ConnectArgs) -> AppResult<i32> {
    if args.stdio && (args.no_reconnect || !args.ssh_args.is_empty()) {
        return Err(AppError::InvalidArgument(
            ssh_workspaces::shared::ArgumentError::Invalid(
                "--stdio cannot be combined with reconnect or SSH arguments".to_owned(),
            ),
        ));
    }

    let alias = HostAlias::try_from(args.alias)?;
    let config = TomlConfig::load_async(&default_config_path()?).await?;
    let host = config.host(&alias)?;
    if args.stdio {
        let endpoint = IrohEndpoint::bind_ephemeral().await?;
        let connector = Connector::with_endpoint(
            endpoint,
            RetryPolicy {
                enabled: false,
                max_delay: DEFAULT_RETRY_DELAY,
                max_retries: 0,
            },
        );
        connector.run_stdio_proxy(host.node_id).await?;
        return Ok(0);
    }

    let connector = Connector::new(RetryPolicy {
        enabled: !args.no_reconnect,
        max_delay: DEFAULT_RETRY_DELAY,
        max_retries: DEFAULT_MAX_RETRIES,
    });
    connector.run(alias, host, args.ssh_args).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_listen_defaults() {
        let cli = Cli::try_parse_from(["sshp", "listen"]).expect("valid listen args");

        let Command::Listen(args) = cli.command else {
            panic!("expected listen command");
        };

        assert_eq!(args.backend, None);
        assert!(!args.print_id);
    }

    #[test]
    fn parses_connect_ssh_arguments_after_separator() {
        let cli =
            Cli::try_parse_from(["sshp", "connect", "hanoi", "--", "-L", "8080:localhost:80"])
                .expect("valid connect args");

        let Command::Connect(args) = cli.command else {
            panic!("expected connect command");
        };

        assert_eq!(args.alias, "hanoi");
        assert_eq!(
            args.ssh_args,
            [OsString::from("-L"), OsString::from("8080:localhost:80")]
        );
    }

    #[test]
    fn rejects_print_id_with_backend_override() {
        assert!(
            Cli::try_parse_from(["sshp", "listen", "--print-id", "--backend", "127.0.0.1:22"])
                .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_stdio_with_ssh_arguments() {
        let cli = Cli::try_parse_from(["sshp", "connect", "hanoi", "--stdio", "--", "-v"])
            .expect("valid CLI syntax");

        let error = run(cli).await.expect_err("stdio must reject SSH arguments");
        assert!(matches!(error, AppError::InvalidArgument(_)));
    }
}
