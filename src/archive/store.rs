use super::convert;
use crate::VERSION;
use chrono::prelude::*;
use failure::Error;
use har::v1_2::*;
use har::{Har, Spec};
use hyper::header;
use hyper::http::request::Parts as ReqParts;
use hyper::http::response::Parts as ResParts;
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

        let spec = Spec::V1_2(log);
        let har = Har { log: spec };
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

    fn get_log_mut(&mut self) -> &mut Log {
        match self.har.log {
            Spec::V1_2(ref mut s) => s,
            _ => unimplemented!(),
        }
    }

    fn get_log(&self) -> &Log {
        match self.har.log {
            Spec::V1_2(ref s) => s,
            _ => unimplemented!(),
        }
    }

    pub fn add_entry(&mut self, entry: Entries) {
        self.get_log_mut().entries.push(entry);
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
        let path_and_query = head
            .uri
            .path_and_query()
            .map(|pq| format!("{}", pq))
            .unwrap_or_else(|| "".to_string());
        let http_version = convert::HttpVersion::har(head.version);
        digest.input(&method);
        digest.input(&path_and_query);
        digest.input(&http_version);
        digest.input(&body);
        let request_hash = format!("{:x}", digest.result());
        let r = Request {
            method,
            url,
            http_version,
            cookies: convert::ClientCookies::har(&head.headers),
            headers: head
                .headers
                .iter()
                .map(|(k, v)| convert::Header::har(&k, &v))
                .collect(),
            query_string: convert::Query::har(&head.uri.query()),
            post_data: convert::RequestBody::har(body, mime_type.to_string()),
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
            status: i64::from(head.status.as_u16()),
            status_text: head.status.as_str().to_string(),
            http_version: convert::HttpVersion::har(head.version),
            cookies: convert::ServerCookies::har(&head.headers),
            headers: head
                .headers
                .iter()
                .map(|(k, v)| convert::Header::har(k, v))
                .collect(),
            content: convert::ResponseBody::har(body, mime_type.to_string()),
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
        self.get_log().entries.first().map(|e: &Entries| {
            e.request
                .comment
                .as_ref()
                .map(|h| h.replace("hash:", ""))
                .map(|h| h.chars().take(8).collect())
                .unwrap_or_else(|| "".to_string())
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
