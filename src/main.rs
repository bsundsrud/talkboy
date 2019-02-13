#[macro_use]
extern crate slog;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate failure;
mod archive;
mod playback;
mod proxy;

use failure::Error;
use slog_async;
use slog_term;

use slog::Drain;

use futures::Future;
use hyper::{rt, Server, Uri};

use std::net::SocketAddr;
lazy_static! {
    pub static ref VERSION: &'static str = option_env!("CARGO_PKG_VERSION").unwrap_or("unknown");
}

fn new_root_logger() -> slog::Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let async_drain = slog_async::Async::new(drain).build().fuse();
    slog::Logger::root(async_drain, o!("version" => VERSION.clone()))
}

fn main() -> Result<(), Error> {
    let root_logger = new_root_logger();

    let proxy_servers = vec![
        proxy::ProxyServer::new(
            "dagbladet",
            ([127, 0, 0, 1], 8080).into(),
            "https://www.dagbladet.no".parse()?,
            "recordings",
        ),
        proxy::ProxyServer::new(
            "google",
            ([127, 0, 0, 1], 8081).into(),
            "https://www.google.com".parse()?,
            "recordings",
        ),
    ];
    let servers = proxy::get_proxy_servers(root_logger, proxy_servers);
    rt::run(servers);

    Ok(())
}
