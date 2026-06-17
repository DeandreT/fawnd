//! fawnd daemon: owns the keyboard and serves clients over a Unix socket.

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "fawnd=info".into()),
        )
        .init();

    fawnd::daemon::run()?;
    Ok(())
}
