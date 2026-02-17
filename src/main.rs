use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    tracing::info!("Vigil starting...");

    // TODO: Load config, init DB, start engine, start API (Stages 4-9)

    tracing::info!("Vigil shutdown complete");
}
