use crate::VERSION;
use base64;
use chrono::prelude::*;
use cookie::Cookie;
use failure::Error;
use har::v1_2::*;
use har::Har;
use hyper::http::request::Parts as ReqParts;
use hyper::http::response::Parts as ResParts;
use hyper::Version;
use hyper::{header, HeaderMap};
use regex::Regex;
use serde_json;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::path::{Path, PathBuf};

pub struct HarSession {
    har: Har,
    start_date: Option<DateTime<Utc>>,
    request: Option<Request>,
    response: Option<Response>,
    request_hash: Option<String>,
}

impl HarSession {
    pub fn new() -> HarSession {
        let log = Log {
            version: "1.2".to_string(),
            creator: Creator {
                name: "Talkboy".into(),
                version: VERSION.to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: Vec::new(),
            comment: None,
        };

        let spec = Spec { log };
        let har = Har::V1_2(spec);
        HarSession {
            har,
            start_date: None,
            request: None,
            response: None,
            request_hash: None,
        }
    }

    pub fn start_session(&mut self) {
        self.start_date = Some(Utc::now());
    }

    fn get_spec_mut(&mut self) -> &mut Spec {
        let spec = match self.har {
            Har::V1_2(ref mut s) => s,
        };
        spec
    }

    fn get_spec(&self) -> &Spec {
        let spec = match self.har {
            Har::V1_2(ref s) => s,
        };
        spec
    }

    pub fn add_entry(&mut self, entry: Entries) {
        self.get_spec_mut().log.entries.push(entry);
    }

    pub fn record_request(&mut self, head: &ReqParts, body: Vec<u8>) {
        let mime_type = head
            .headers
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap_or(""))
            .unwrap_or_else(|| "");
        let mut digest = Sha256::new();
        let method = head.method.as_str().into();
        let url = format!("{}", &head.uri);
        let http_version = string_for_http_version(&head.version);
        digest.input(&method);
        digest.input(&url);
        digest.input(&http_version);
        digest.input(&body);
        let request_hash = format!("{:x}", digest.result());
        let r = Request {
            method,
            url,
            http_version,
            cookies: parse_request_cookies(&head.headers),
            headers: headers_to_har(&head.headers),
            query_string: query_string_to_har(&head.uri.query()),
            post_data: request_body_to_har(body, mime_type),
            headers_size: -1,
            body_size: -1,
            comment: Some(format!("hash:{}", &request_hash)),
        };
        self.request_hash = Some(request_hash);
        self.request = Some(r);
    }

    pub fn record_response(&mut self, head: &ResParts, body: Vec<u8>) {
        let mime_type = head
            .headers
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap_or(""))
            .unwrap_or("");
        let redirect_url = head
            .headers
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap_or(""))
            .unwrap_or("")
            .to_string();
        let r = Response {
            charles_status: None,
            status: head.status.as_u16() as i64,
            status_text: head.status.as_str().to_string(),
            http_version: string_for_http_version(&head.version),
            cookies: parse_response_cookies(&head.headers),
            headers: headers_to_har(&head.headers),
            content: response_body_to_har(body, mime_type),
            redirect_url,
            headers_size: -1,
            body_size: -1,
            comment: None,
        };

        self.response = Some(r);
    }

    fn build(&mut self) -> Result<Entries, IncompleteEntryError> {
        match (&self.request, &self.response) {
            (Some(_), Some(_)) => (),
            (None, None) => return Err(IncompleteEntryError::MissingBoth),
            (None, _) => return Err(IncompleteEntryError::MissingRequest),
            (_, None) => return Err(IncompleteEntryError::MissingResponse),
        };

        match &self.start_date {
            Some(_) => (),
            None => return Err(IncompleteEntryError::MissingStart),
        }

        let entry = Entries {
            pageref: None,
            started_date_time: self.start_date.unwrap().to_rfc3339(),
            time: (Utc::now() - self.start_date.unwrap()).num_milliseconds(),
            request: self.request.take().unwrap(),
            response: self.response.take().unwrap(),
            cache: Cache {
                before_request: None,
                after_request: None,
            },
            timings: Timings {
                blocked: None,
                dns: None,
                connect: None,
                send: -1,
                wait: -1,
                receive: -1,
                ssl: None,
                comment: None,
            },
            server_ip_address: None,
            connection: None,
            comment: None,
        };

        Ok(entry)
    }

    pub fn file_hash(&self) -> Option<String> {
        self.get_spec().log.entries.first().map(|e: &Entries| {
            e.request
                .comment
                .as_ref()
                .map(|h| h.replace("hash:", ""))
                .map(|h| h.chars().take(8).collect())
                .unwrap_or("".to_string())
        })
    }

    pub fn commit(&mut self) -> Result<String, IncompleteEntryError> {
        let entry = self.build()?;
        self.add_entry(entry);
        self.start_date = None;
        self.request = None;
        self.response = None;
        let hash = self.request_hash.take().unwrap();
        Ok(hash)
    }

    pub fn write_to_dir<P: AsRef<Path>, S: AsRef<str>>(
        &self,
        path: P,
        base_name: S,
    ) -> Result<PathBuf, Error> {
        let path = path.as_ref();
        let hash = self
            .file_hash()
            .ok_or_else(|| IncompleteEntryError::EmptySession)?;
        let file_name = path.join(format!(
            "{}.{}.json",
            normalize_path(base_name.as_ref()),
            hash
        ));
        let file = File::create(&file_name)?;
        serde_json::to_writer_pretty(file, &self.har)?;
        Ok(file_name)
    }
}

#[derive(Debug, Fail)]
pub enum IncompleteEntryError {
    #[fail(display = "Incomplete Entry, missing Request")]
    MissingRequest,
    #[fail(display = "Incomplete Entry, missing Response")]
    MissingResponse,
    #[fail(display = "Incomplete Entry, missing Request and Response")]
    MissingBoth,
    #[fail(display = "Incomplete Entry, missing Start time (did you call `start_session()`?)")]
    MissingStart,
    #[fail(display = "Empty Session (did you call `commit()`?)")]
    EmptySession,
}

fn normalize_path(s: &str) -> String {
    lazy_static! {
        static ref PATTERN: Regex = Regex::new("[[:^word:]--\\.]").unwrap();
    }
    let normalized = PATTERN.replace_all(s, "-").into_owned();
    if normalized.len() > 20 {
        normalized.chars().take(20).collect()
    } else {
        normalized
    }
}

fn string_for_http_version(v: &Version) -> String {
    match *v {
        Version::HTTP_09 => "HTTP/0.9",
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_11 => "HTTP/1.1",
        Version::HTTP_2 => "HTTP/2",
    }
    .into()
}

fn cookie_to_har(c: Cookie) -> Cookies {
    Cookies {
        name: c.name().into(),
        value: c.value().into(),
        path: c.path().map(|p| p.into()),
        domain: c.domain().map(|d| d.into()),
        expires: c.expires().map(|e| format!("{}", e.rfc3339())),
        http_only: c.http_only().map(|h| h.into()),
        secure: c.secure(),
        comment: None,
    }
}

fn parse_request_cookies(m: &HeaderMap) -> Vec<Cookies> {
    m.get_all(header::COOKIE)
        .iter()
        .flat_map(|c| c.to_str().expect("Invalid encoding in cookie").split("; "))
        .map(|c| Cookie::parse(c).expect("Couldn't parse cookie"))
        .map(cookie_to_har)
        .collect()
}

fn parse_response_cookies(m: &HeaderMap) -> Vec<Cookies> {
    m.get_all(header::SET_COOKIE)
        .iter()
        .map(|c| c.to_str().expect("Invalid encoding in cookie"))
        .map(|c| Cookie::parse(c).expect("Couldn't parse cookie"))
        .map(cookie_to_har)
        .collect()
}

fn headers_to_har(m: &HeaderMap) -> Vec<Headers> {
    m.iter()
        .map(|(key, val)| Headers {
            name: key.as_str().to_string(),
            value: val.to_str().unwrap().to_string(),
            comment: None,
        })
        .collect()
}

fn query_string_to_har(p: &Option<&str>) -> Vec<QueryString> {
    p.map(|qs| {
        qs.split("&")
            .filter_map(|pair| {
                let mut iter = pair.splitn(2, "=");
                if let Some(k) = iter.next() {
                    if let Some(v) = iter.next() {
                        Some(QueryString {
                            name: k.to_string(),
                            value: v.to_string(),
                            comment: None,
                        })
                    } else {
                        Some(QueryString {
                            name: k.to_string(),
                            value: "".to_string(),
                            comment: None,
                        })
                    }
                } else {
                    None
                }
            })
            .collect()
    })
    .unwrap_or_else(|| Vec::new())
}

fn body_text(b: Vec<u8>) -> (String, bool) {
    match String::from_utf8(b) {
        Ok(s) => (s, false),
        Err(e) => (base64::encode(e.as_bytes()), true),
    }
}

fn request_body_to_har<S: Into<String>>(b: Vec<u8>, mime_type: S) -> Option<PostData> {
    if b.is_empty() {
        None
    } else {
        let (text, base64_encoded) = body_text(b);
        Some(PostData {
            mime_type: mime_type.into(),
            text,
            params: None,
            comment: if base64_encoded {
                Some("base64".into())
            } else {
                None
            },
        })
    }
}

fn response_body_to_har<S: Into<String>>(b: Vec<u8>, mime_type: S) -> Content {
    let size = b.len() as i64;
    let (text, encoded) = body_text(b);
    Content {
        size,
        compression: None,
        mime_type: mime_type.into(),
        text: if text.is_empty() { None } else { Some(text) },
        encoding: if encoded { Some("base64".into()) } else { None },
        comment: None,
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn test_normalize_path() {
        use super::normalize_path;

        assert_eq!("-test-.-path-q-20", normalize_path("/test/./path?q=20"));
        assert_eq!("nOth1ng_in_Her3", normalize_path("nOth1ng_in_Her3"));
        assert_eq!(
            "this-is-longer-than-",
            normalize_path("this is longer than 20 characters")
        );
        assert_eq!("dots.ok", normalize_path("dots.ok"));
    }
}
