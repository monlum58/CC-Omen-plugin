mod config;
mod control;
mod engine;
mod hw;
mod service;

use crate::device_service::v1::device_service_server::DeviceServiceServer;
use crate::engine::Engine;
use crate::service::OmenService;
use anyhow::{Context, Result, bail};
use clap::Parser;
use log::{LevelFilter, error, info, warn};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use systemd_journal_logger::{JournalLog, connected_to_journal};
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;
use tonic::codegen::tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

pub const SERVICE_ID: &str = env!("CARGO_PKG_NAME");
pub const VERSION: &str = env!("CC_OMEN_VERSION");
const ENV_CC_LOG: &str = "CC_LOG";

pub mod models {
    pub mod v1 {
        tonic::include_proto!("coolercontrol.models.v1");
    }
}
pub mod device_service {
    pub mod v1 {
        tonic::include_proto!("coolercontrol.device_service.v1");
    }
}

/// CoolerControl device plugin for the HP OMEN 16 fans: 0 % on both channels hands the fans
/// to the firmware, which stops them when cool; anything above 0 % is manual control.
#[derive(Parser, Debug)]
#[clap(version = VERSION, about, long_about = None)]
struct Args {
    /// Unix socket to serve on (must match `address` in manifest.toml).
    #[clap(long, default_value = "/run/coolercontrol-plugin-cc_omen.sock")]
    socket: PathBuf,
    /// Settings file; defaults to config.json next to the executable.
    #[clap(long)]
    config: Option<PathBuf>,
    /// Run on boards other than 8BCA that have the same hp-wmi fan interface.
    #[clap(long)]
    allow_any_board: bool,
    /// Root of the sysfs tree (tests only).
    #[clap(long, default_value = "/sys", hide = true)]
    sysfs_root: PathBuf,
    /// Enable debug logging.
    #[clap(short, long)]
    debug: bool,
}

fn main() {
    let args = Args::parse();
    if let Err(e) = setup_logging(args.debug) {
        eprintln!("logging: {e:#}");
    }
    if let Err(e) = run(args) {
        error!("{e:#}");
        std::process::exit(1);
    }
}

#[tokio::main(flavor = "current_thread")]
async fn run(args: Args) -> Result<()> {
    info!("Starting {SERVICE_ID} v{VERSION}");
    let root = args.sysfs_root.as_path();

    let fans = hw::find_hp(root).context("refusing to start")?;
    match hw::board_name(root) {
        Some(b) if b == hw::SUPPORTED_BOARD => {}
        other if args.allow_any_board => warn!("board {other:?} is untested; running because of --allow-any-board"),
        other => bail!(
            "refusing to start: board {other:?} is not the tested {} (use --allow-any-board to run anyway)",
            hw::SUPPORTED_BOARD
        ),
    }
    let cpu_temp = hw::find_cpu_temp(root).context("refusing to start: no CPU temperature sensor (k10temp or coretemp)")?;

    let config_path = match args.config {
        Some(p) => p,
        None => std::env::current_exe()?.parent().map(|d| d.join("config.json")).unwrap_or_default(),
    };
    let (cfg, notes) = config::load(&config_path, unsafe { libc::geteuid() });
    for n in notes {
        warn!("{n}");
    }
    info!(
        "settings: dwell {} s, watchdog {} s, emergency {} °C (until {} °C)",
        cfg.dwell_ms / 1000,
        cfg.watchdog_ms / 1000,
        cfg.emergency_c,
        cfg.emergency_c - cfg.hysteresis_c
    );
    let tick = Duration::from_millis(cfg.tick_ms);
    let hwmon_dir = fans.dir().display().to_string();

    let mut engine = Engine::new(root, fans, cpu_temp, cfg);
    engine::install_panic_hook(true);
    engine.start().context("could not set firmware auto at start")?;
    let engine = Arc::new(Mutex::new(engine));

    let listener = bind_socket(&args.socket)?;
    let run_token = termination_token();

    // The control loop runs on its own timer, so the watchdog and the emergency limit act
    // even when CoolerControl sends nothing.
    let loop_engine = engine.clone();
    let loop_token = run_token.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tick);
        loop {
            tokio::select! {
                _ = interval.tick() => loop_engine.lock().unwrap_or_else(|e| e.into_inner()).cycle(),
                () = loop_token.cancelled() => break,
            }
        }
    });

    let served = Server::builder()
        .add_service(DeviceServiceServer::new(OmenService::new(engine.clone(), hwmon_dir)))
        .serve_with_incoming_shutdown(UnixListenerStream::new(listener), run_token.cancelled())
        .await;
    engine.lock().unwrap_or_else(|e| e.into_inner()).stop();
    remove_socket(&args.socket);
    served?;
    info!("stopped");
    Ok(())
}

/// Binds the socket with mode 0600. An existing socket is replaced; anything else at the path
/// (a file, a directory, a symlink) is left alone and the plugin refuses to start.
fn bind_socket(path: &Path) -> Result<UnixListener> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_socket() {
            std::fs::remove_file(path).with_context(|| format!("removing stale socket {}", path.display()))?;
        } else {
            bail!("{} exists and is not a socket; refusing to touch it", path.display());
        }
    }
    let old = unsafe { libc::umask(0o177) };
    let listener = UnixListener::bind(path);
    unsafe { libc::umask(old) };
    let listener = listener.with_context(|| format!("binding {} (the plugin must run as root)", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn remove_socket(path: &Path) {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_socket()) {
        let _ = std::fs::remove_file(path);
    }
}

/// CoolerControl passes its log level in CC_LOG.
fn setup_logging(debug: bool) -> Result<()> {
    let level = if debug {
        LevelFilter::Debug
    } else if let Ok(l) = std::env::var(ENV_CC_LOG) {
        LevelFilter::from_str(&l).unwrap_or(LevelFilter::Info)
    } else {
        LevelFilter::Info
    };
    if connected_to_journal() {
        JournalLog::new()?.with_extra_fields(vec![("VERSION", VERSION)]).install()?;
        log::set_max_level(level);
    } else {
        env_logger::Builder::new().filter_level(level).init();
    }
    Ok(())
}

fn termination_token() -> CancellationToken {
    let token = CancellationToken::new();
    let t = token.clone();
    tokio::spawn(async move {
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
        let mut int = signal(SignalKind::interrupt()).expect("SIGINT handler");
        let mut quit = signal(SignalKind::quit()).expect("SIGQUIT handler");
        tokio::select! {
            _ = term.recv() => {},
            _ = int.recv() => {},
            _ = quit.recv() => {},
        }
        info!("Shutting down");
        t.cancel();
    });
    token
}
