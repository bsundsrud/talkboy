#[macro_use]
extern crate slog;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate failure;
mod archive;
mod cli;
mod config;
mod playback;
mod proxy;

use failure::Error;
use slog_async;
use slog_term;

use slog::Drain;

use cli::CliConfig;
use hyper::rt;

lazy_static! {
    pub static ref VERSION: &'static str = option_env!("CARGO_PKG_VERSION").unwrap_or("unknown");
}

fn new_root_logger() -> slog::Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let async_drain = slog_async::Async::new(drain).build().fuse();
    slog::Logger::root(async_drain, o!())
}

fn main() -> Result<(), Error> {
    let root_logger = new_root_logger();
    let logger = root_logger.new(o!("lifecycle" => "config"));
    let config = cli::get_config(logger)?;
    match config {
        CliConfig::Proxy(servers) => {
            let server = proxy::get_proxy_servers(root_logger, servers);
            rt::run(server);
        }
        CliConfig::Playback(servers) => {
            let server = playback::get_playback_servers(root_logger, servers);
            rt::run(server);
        }
    }

    Ok(())
}
