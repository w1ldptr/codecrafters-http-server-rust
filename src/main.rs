use std::str::FromStr;
use color_eyre::eyre::Result;
use tracing_subscriber::{filter::targets::Targets,
                         layer::SubscriberExt,
                         util::SubscriberInitExt};
use clap::{Command, Arg};
use basic_http_server::BasicHttpServer;

pub mod basic_http_server;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let filter_layer = Targets::from_str(std::env::var("RUST_LOG")
                                         .as_deref()
                                         .unwrap_or("info"))?;
    let format_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_line_number(true)
        .with_file(true)
        .with_target(false);
    tracing_subscriber::registry()
        .with(filter_layer)
        .with(format_layer)
        .init();

    let matches = Command::new("codecrafters-http-server-rust")
        .about("Simple asynchronous HTTP server with Tokio")
        .arg(
            Arg::new("directory")
                .help("Files directory")
                .long("directory")
                .value_parser(clap::builder::NonEmptyStringValueParser::new())
                .default_value("."),
        )
        .get_matches();
    let dir = matches.get_one::<String>("directory").unwrap();

    let server = BasicHttpServer::new("127.0.0.1:4221", dir).await?;

    server.run().await?;
    Ok(())
}
