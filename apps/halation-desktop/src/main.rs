use tracing::info;
use tracing_subscriber::EnvFilter;

#[cfg(not(target_os = "windows"))]
compile_error!("Halation Desktop is currently supported on Windows only.");

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    info!("Starting Halation Desktop (Windows Engine)...");
    halation_ui::run()
}
