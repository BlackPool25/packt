use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

mod proxy;

#[derive(Parser)]
#[command(name = "packt-proxy")]
#[command(about = "Transparent Docker registry proxy with layer dedup")]
struct Cli {
    /// Listen address (default: 0.0.0.0:5000)
    #[arg(long, default_value = "0.0.0.0:5000")]
    listen: SocketAddr,

    /// Upstream registry URL (default: https://registry-1.docker.io)
    #[arg(long, default_value = "https://registry-1.docker.io")]
    upstream: String,

    /// Local cache directory for blob storage
    #[arg(long, default_value = "/var/lib/packt-proxy/cache")]
    cache_dir: PathBuf,

    /// Chunk size preset for dedup worker (default: 8k)
    #[arg(long, default_value = "8k")]
    chunk_size: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    let state = Arc::new(proxy::AppState::new(
        &cli.upstream,
        &cli.cache_dir,
        &cli.chunk_size,
    )?);

    let app = proxy::build_router(state.clone());
    let listener = tokio::net::TcpListener::bind(cli.listen).await?;

    tracing::info!("packt-proxy listening on {}", cli.listen);
    tracing::info!("upstream registry: {}", cli.upstream);
    tracing::info!("cache dir: {}", cli.cache_dir.display());

    axum::serve(listener, app).await?;
    Ok(())
}
