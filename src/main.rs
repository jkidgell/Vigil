use clap::Parser;
use std::path::PathBuf;
use tokio::task::LocalSet;
use tracing_subscriber::EnvFilter;

use vigil::api::handlers::AppState;
use vigil::api::routes::build_router;
use vigil::config::VigilConfig;
use vigil::db::schema::init_database;
use vigil::engine::core::Engine;

#[derive(Parser)]
#[command(name = "vigil", version, about = "Network monitoring engine")]
struct Cli {
    /// Config file path
    #[arg(short, long, default_value = "/etc/vigil/vigil.toml")]
    config: PathBuf,

    /// Database path (overrides config)
    #[arg(short, long)]
    db: Option<PathBuf>,

    /// API bind address (overrides config)
    #[arg(short, long)]
    bind: Option<String>,

    /// Increase log verbosity (repeatable: -v debug, -vv trace)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let mut config = VigilConfig::load(&cli.config)?;

    // CLI overrides.
    if let Some(db) = cli.db {
        config.engine.db_path = db;
    }
    if let Some(bind) = cli.bind {
        config.api.bind_address = bind;
    }

    // Tracing.
    let log_level = match cli.verbose {
        0 => config.logging.level.clone(),
        1 => "debug".to_string(),
        _ => "trace".to_string(),
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_new(&log_level).unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Vigil starting...");
    tracing::info!("DB:  {}", config.engine.db_path.display());
    tracing::info!("API: {}", config.api.bind_address);

    // Open database (create parent directories if needed).
    if let Some(parent) = config.engine.db_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let db = init_database(config.engine.db_path.to_str().unwrap_or(":memory:"))?;

    // Create engine.
    let (mut engine, handle) = Engine::new(db, config.engine.max_concurrent_polls)?;

    // Bind API listener.
    let listener = tokio::net::TcpListener::bind(&config.api.bind_address).await?;
    tracing::info!("API listening on {}", config.api.bind_address);

    // Build router.
    let state = AppState { engine: handle.clone(), started_at: std::time::Instant::now() };
    let router = build_router(state);

    // Signal handler → triggers shutdown token.
    let shutdown_token = handle.shutdown_token();
    let token_for_signal = shutdown_token.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Shutdown signal received");
        token_for_signal.cancel();
    });

    // API server with graceful shutdown.
    let token_for_api = shutdown_token.clone();
    let api_task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { token_for_api.cancelled().await })
            .await
            .ok();
    });

    // Engine runs in a LocalSet (rusqlite::Connection is !Send).
    let local = LocalSet::new();
    local
        .run_until(async move {
            tokio::task::spawn_local(async move {
                if let Err(e) = engine.run().await {
                    tracing::error!("Engine error: {e}");
                }
            });

            // Wait for both engine shutdown (via token) and API task.
            shutdown_token.cancelled().await;
            api_task.await.ok();
        })
        .await;

    tracing::info!("Vigil shutdown complete");
    Ok(())
}
