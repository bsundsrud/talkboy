use crate::archive::ArchivedRequest;
use crate::archive::HarLoader;
use failure::Error;
use hyper::Uri;
use serde_derive::Deserialize;
use slog::Logger;
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(rename = "project")]
    projects: Vec<ProjectConfig>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectConfig {
    name: String,
    addr: Option<String>,
    port: Option<u16>,
    playback: Option<PlaybackConfig>,
    record: Option<ProxyConfig>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(tag = "method")]
pub enum DelayOptions {
    None,
    Original,
    Static { millis: u64 },
}

#[derive(Debug, Deserialize)]
pub struct PlaybackConfig {
    delay: Option<DelayOptions>,
}

#[derive(Debug, Deserialize)]
pub struct ProxyConfig {
    uri: String,
}

pub struct PlaybackServerConfig {
    pub name: String,
    pub socket: SocketAddr,
    pub archives: Vec<ArchivedRequest>,
    pub delay: DelayOptions,
}

impl PlaybackServerConfig {
    pub fn new<S: Into<String>>(
        name: S,
        socket: SocketAddr,
        archives: Vec<ArchivedRequest>,
        delay: DelayOptions,
    ) -> PlaybackServerConfig {
        PlaybackServerConfig {
            name: name.into(),
            socket,
            archives,
            delay,
        }
    }
}

pub struct ProxyServerConfig {
    pub name: String,
    pub socket: SocketAddr,
    pub archive_path: PathBuf,
    pub proxy_for: Uri,
}

impl ProxyServerConfig {
    pub fn new<S: Into<String>, P: Into<PathBuf>>(
        name: S,
        socket: SocketAddr,
        proxy_for: Uri,
        archive_path: P,
    ) -> ProxyServerConfig {
        ProxyServerConfig {
            name: name.into(),
            socket,
            proxy_for,
            archive_path: archive_path.into(),
        }
    }
}

struct NextUnusedPort {
    current: u16,
    used: Vec<u16>,
}

impl NextUnusedPort {
    fn new(start: u16) -> NextUnusedPort {
        NextUnusedPort {
            current: start,
            used: Vec::new(),
        }
    }

    fn observe(&mut self, val: u16) -> u16 {
        self.used.push(val);
        val
    }
}

impl Iterator for NextUnusedPort {
    type Item = u16;
    fn next(&mut self) -> Option<Self::Item> {
        while self.used.contains(&self.current) {
            self.current += 1;
            if self.current == std::u16::MAX {
                return None;
            }
        }
        let p = self.current.clone();
        self.observe(p);
        self.current += 1;
        if self.current == std::u16::MAX {
            return None;
        }
        Some(p)
    }
}

impl Config {
    pub fn try_into_proxy_servers(
        self,
        recording_dir: &str,
        logger: Logger,
    ) -> Result<Vec<ProxyServerConfig>, Error> {
        trace!(logger, "Creating proxy servers from config");
        let mut next_port = NextUnusedPort::new(8080);
        self.projects
            .into_iter()
            .filter(|p| p.record.is_some())
            .map(|p| {
                let addr = p.addr.unwrap_or_else(|| "127.0.0.1".to_string());
                let port = p.port.map(|p| next_port.observe(p)).unwrap_or_else(|| {
                    next_port
                        .next()
                        .expect("Ran out of ports trying to assign for proxy")
                });

                let socket_addr: SocketAddr = format!("{}:{}", addr, port).parse()?;
                let proxy = p.record.unwrap();
                let uri: Uri = proxy.uri.parse()?;
                Ok(ProxyServerConfig::new(
                    p.name,
                    socket_addr,
                    uri,
                    &recording_dir,
                ))
            })
            .collect::<Result<Vec<ProxyServerConfig>, Error>>()
    }

    pub fn try_into_playback_servers(
        self,
        recording_dir: &str,
        logger: Logger,
    ) -> Result<Vec<PlaybackServerConfig>, Error> {
        let logger = &logger;
        let mut next_port = NextUnusedPort::new(8080);
        self.projects
            .into_iter()
            .filter(|p| p.playback.is_some())
            .map(|p| {
                let addr = p.addr.unwrap_or_else(|| "127.0.0.1".to_string());
                let port = p.port.map(|p| next_port.observe(p)).unwrap_or_else(|| {
                    next_port
                        .next()
                        .expect("Ran out of ports trying to assign for playback")
                });
                let playback = p.playback.unwrap();
                (addr, port, p.name, playback)
            })
            .map(move |(addr, port, name, playback)| {
                let socket_addr: SocketAddr = format!("{}:{}", addr, port).parse()?;
                let delay = playback.delay.unwrap_or(DelayOptions::None);
                let logger = logger.new(o!("loader" => "HarLoader"));
                let loader = HarLoader::new(logger);
                let p: PathBuf = recording_dir.into();
                let p = p.join(&name);
                let archives = loader.load_all(&p)?;

                Ok(PlaybackServerConfig::new(
                    name,
                    socket_addr,
                    archives,
                    delay,
                ))
            })
            .collect::<Result<Vec<PlaybackServerConfig>, Error>>()
    }
}

#[cfg(test)]
mod test {
    use super::Config;
    use toml;
    #[test]
    fn test_parse_config() {
        let conf = r#"
[[project]]
name = "foo"
addr = "127.0.0.1"
port = 8080

[project.playback]
delay = { method = "NoDelay" }

[[project]]
name = "bar"
port = 8081

[project.playback]
delay = { method = "Static", millis = 500 }

[project.proxy]
uri = "https://www.google.com"
"#;
        let val: Result<Config, _> = toml::from_str(&conf);
        match val {
            Ok(c) => {
                assert_eq!(2, c.projects.len());
            }
            Err(e) => assert!(false, "Didn't parse correctly: {}", e),
        }
    }
}
