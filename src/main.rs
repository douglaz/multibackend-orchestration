use clap::Parser;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .without_time()
        .init();

    let cli = ralph::cli::Cli::parse();
    if let Err(err) = ralph::cli::run(cli).await {
        eprintln!("error: {err}");
        std::process::exit(err.exit_code());
    }
}
