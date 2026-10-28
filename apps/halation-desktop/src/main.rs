use tracing::info;
use tracing_subscriber::EnvFilter;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    info!("Starting Halation Desktop (Windows Engine)...");
    halation_ui::run()
}
