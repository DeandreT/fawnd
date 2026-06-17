//! Native GUI front-end for fawnd.

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "fawnd=info".into()),
        )
        .init();

    fawnd::gui::run()
}
