//! `uclog-hub`: the hub on its own, as a small executable (`uclog hub` is the same thing).

use clap::Parser;

#[derive(Parser)]
#[command(name = "uclog-hub", version, about = "ucLog hub: one device, one TCP/WebSocket/UDP client at a time")]
struct Cli {
    /// -v: statistics every 10 s, -vv: debug
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
    #[command(flatten)]
    hub: uclog_hub::HubArgs,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let level = if cli.verbose >= 2 { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| level.into()))
        .with_writer(std::io::stderr)
        .init();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let r = rt.block_on(uclog_hub::run(cli.hub, cli.verbose));
    rt.shutdown_timeout(std::time::Duration::from_millis(200));
    match r {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("uclog-hub: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
