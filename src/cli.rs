use crate::archive::HarLoader;
use crate::config::{Config, DelayOptions, PlaybackServerConfig, ProxyServerConfig};
use crate::VERSION;
use clap::{App, AppSettings, Arg, ArgGroup, SubCommand};
use failure::Error;
use hyper::Uri;
use slog::Logger;
use std::fmt::Display;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use toml;

fn addr_validator(v: String) -> Result<(), String> {
    let socket_addr = format!("{}:8080", v).parse::<SocketAddr>();
    socket_addr.map(|_| ()).map_err(|e| format!("{}", e))
}

fn port_validator(v: String) -> Result<(), String> {
    let port = v
        .parse::<u16>()
        .map_err(|_| "Port must be an integer in range 1-65535".to_string())?;
    if port == 0 {
        Err("Port must be in range 1-65535".to_string())
    } else {
        Ok(())
    }
}

fn type_validator<T>(v: String) -> Result<(), String>
where
    T: FromStr,
    T::Err: Display,
{
    let val = v.parse::<T>();
    val.map(|_| ()).map_err(|e| format!("{}", e))
}

pub enum CliConfig {
    Proxy(Vec<ProxyServerConfig>),
    Playback(Vec<PlaybackServerConfig>),
}

fn proxy_config_from_file(
    logger: Logger,
    path: &str,
    recording_dir: &str,
) -> Result<Vec<ProxyServerConfig>, Error> {
    let contents = fs::read_to_string(&path)?;
    let cfg: Config = toml::from_str(&contents)?;
    cfg.try_into_proxy_servers(&recording_dir, logger)
}

fn proxy_config_from_cli(
    logger: Logger,
    recording_dir: &str,
    addr: &str,
    port: u16,
    project: &str,
    proxy_for: &str,
) -> Result<Vec<ProxyServerConfig>, Error> {
    trace!(logger, "Creating Proxy config from CLI params");
    let socket_addr: SocketAddr = format!("{}:{}", addr, port).parse()?;
    let uri: Uri = proxy_for.parse()?;
    let s = ProxyServerConfig::new(project, socket_addr, uri, recording_dir);

    Ok(vec![s])
}

fn playback_config_from_file(
    logger: Logger,
    path: &str,
    recording_dir: &str,
) -> Result<Vec<PlaybackServerConfig>, Error> {
    let contents = fs::read_to_string(&path)?;
    let cfg: Config = toml::from_str(&contents)?;
    cfg.try_into_playback_servers(&recording_dir, logger)
}

fn playback_config_from_cli(
    logger: Logger,
    recording_dir: &str,
    addr: &str,
    port: u16,
    project: &str,
    delay: DelayOptions,
) -> Result<Vec<PlaybackServerConfig>, Error> {
    trace!(logger, "Creating Playback config from CLI params");
    let socket_addr: SocketAddr = format!("{}:{}", addr, port).parse()?;
    let loader = HarLoader::new(logger);
    let p: PathBuf = PathBuf::from(&recording_dir).join(&project);
    let archives = loader.load_all(&p)?;
    let s = PlaybackServerConfig::new(project, socket_addr, archives, delay);
    Ok(vec![s])
}

pub fn get_config(logger: Logger) -> Result<CliConfig, Error> {
    let app = App::new("Talkboy")
        .version(VERSION.as_ref())
        .author("Benn Sundsrud <benn.sundsrud@gmail.com>")
        .about("Record/play back HTTP sessions")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .arg(
            Arg::with_name("recording_dir")
                .short("d")
                .long("recording-dir")
                .value_name("DIR")
                .help("Path to the recorded HTTP sessions")
                .default_value("recordings")
                .required(false)
                .takes_value(true),
        )
        .subcommand(
            SubCommand::with_name("record")
                .about("Start a proxy to record HTTP sessions")
                .usage("talkboy record [OPTIONS] (--config CONFIG | [--addr ADDR] [--port PORT] PROJECT URL)")
                .arg(
                    Arg::with_name("config_file")
                        .short("c")
                        .long("config")
                        .value_name("CONFIG")
                        .takes_value(true)
                        .required(false)
                        .help("Use config file to specify proxy options"),
                )
                .arg(
                    Arg::with_name("addr")
                        .short("a")
                        .long("addr")
                        .value_name("ADDR")
                        .help("Address to listen on")
                        .default_value("127.0.0.1")
                        .required(false)
                        .validator(addr_validator),
                )
                .arg(
                    Arg::with_name("port")
                        .short("p")
                        .long("port")
                        .value_name("PORT")
                        .help("Port to listen on")
                        .default_value("8080")
                        .required(false)
                        .validator(port_validator),
                )
                .arg(
                    Arg::with_name("project_name")
                        .value_name("PROJECT")
                        .help("Project name used to group HTTP sessions")
                        .index(1),
                )
                .arg(
                    Arg::with_name("proxy_for")
                        .value_name("URL")
                        .help("URL to proxy requests to")
                        .validator(type_validator::<Uri>)
                        .index(2),
                )
                .group(
                    ArgGroup::with_name("from_config")
                        .arg("config_file")
                        .conflicts_with("from_cli"),
                )
                .group(
                    ArgGroup::with_name("from_cli")
                        .arg("port")
                        .arg("project_name")
                        .arg("proxy_for")
                        .multiple(true)
                        .conflicts_with("from_config"),
                ),
        )
        .subcommand(
            SubCommand::with_name("playback")
                .about("Start a server to play back recorded HTTP sessions")
                .usage("talkboy playback [OPTIONS] (--config CONFIG | [DELAY_OPTION] [--addr ADDR] [--port PORT] PROJECT)")
                .arg(
                    Arg::with_name("config_file")
                        .short("c")
                        .long("config")
                        .value_name("CONFIG")
                        .takes_value(true)
                        .required(false)
                        .help("Use config file to specify playback options"),
                )
                .arg(
                    Arg::with_name("addr")
                        .short("a")
                        .long("addr")
                        .value_name("ADDR")
                        .help("Address to listen on")
                        .default_value("127.0.0.1")
                        .required(false)
                        .validator(addr_validator),
                )
                .arg(
                    Arg::with_name("port")
                        .short("p")
                        .long("port")
                        .value_name("PORT")
                        .help("Port to listen on")
                        .default_value("8080")
                        .required(false)
                        .validator(port_validator),
                )
                .arg(
                    Arg::with_name("original_delay")
                        .long("original-delay")
                        .required(false)
                        .help("Respond to requests with the original latency"),
                )
                .arg(
                    Arg::with_name("delay_ms")
                        .short("D")
                        .long("delay-ms")
                        .value_name("MS")
                        .takes_value(true)
                        .required(false)
                        .help("Introduce a static delay to each request")
                        .validator(type_validator::<u64>)
                )
                .arg(
                    Arg::with_name("project_name")
                        .value_name("PROJECT")
                        .help("Project name used to group HTTP sessions")
                        .index(1),
                )
                .group(
                    ArgGroup::with_name("from_config")
                        .arg("config_file")
                        .conflicts_with("from_cli"),
                )
                .group(
                    ArgGroup::with_name("from_cli")
                        .arg("port")
                        .arg("project_name")
                        .multiple(true)
                        .conflicts_with("from_config"),
                )
                .group(
                    ArgGroup::with_name("delay")
                        .arg("original_delay")
                        .arg("delay_ms")
                        .conflicts_with("from_config")
                        .requires("from_cli")
                ),
        );
    let matches = app.get_matches();
    println!("here");
    let recording_dir = matches
        .value_of("recording_dir")
        .expect("recording_dir should have a default");

    if let Some(m) = matches.subcommand_matches("record") {
        let logger = logger.new(o!("config_for" => "proxy"));
        let configs = if m.is_present("config") {
            let logger = logger.new(o!("config_from" => "file"));
            let file = m.value_of("config").unwrap();
            proxy_config_from_file(logger, &file, &recording_dir)?
        } else {
            let logger = logger.new(o!("config_from" => "cli"));
            let addr = m.value_of("addr").expect("addr has a default");
            let port: u16 = m.value_of("port").expect("port has a default").parse()?;
            let project = m
                .value_of("project_name")
                .expect("project_name is required");
            let proxy_for = m.value_of("proxy_for").expect("proxy_for is required");
            proxy_config_from_cli(logger, &recording_dir, &addr, port, &project, &proxy_for)?
        };
        Ok(CliConfig::Proxy(configs))
    } else if let Some(m) = matches.subcommand_matches("playback") {
        let logger = logger.new(o!("config_for" => "playback"));
        let configs = if m.is_present("config") {
            let logger = logger.new(o!("config_from" => "file"));
            let file = m.value_of("config").unwrap();
            playback_config_from_file(logger, &file, &recording_dir)?
        } else {
            let logger = logger.new(o!("config_from" => "cli"));
            let addr = m.value_of("addr").expect("addr has a default");
            let port: u16 = m.value_of("port").expect("port has a default").parse()?;
            let project = m
                .value_of("project_name")
                .expect("project_name is required");
            let delay = if m.is_present("original_delay") {
                DelayOptions::Original
            } else if m.is_present("delay_ms") {
                let ms: u64 = m.value_of("delay_ms").unwrap().parse()?;
                DelayOptions::Static { millis: ms }
            } else {
                DelayOptions::None
            };
            playback_config_from_cli(logger, &recording_dir, &addr, port, &project, delay)?
        };
        Ok(CliConfig::Playback(configs))
    } else {
        bail!("No recognized subcommand was provided; this should not occur")
    }
}
