use base64;
use bytes::Bytes;
use cookie::Cookie;
use failure::Error;
use har::v1_2::*;
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::{Body, Chunk, Version};
use std::borrow::Cow;
use std::convert::From;

#[derive(Debug, Fail)]
pub enum ConversionError {
    #[fail(display = "Invalid HTTP version {}", _0)]
    InvalidHttpVersion(String),
}

pub struct Header;
impl Header {
    pub fn har(key: &HeaderName, val: &HeaderValue) -> Headers {
        let name = key.as_str().to_string();
        let bytes = val.as_bytes();
        let (value, encoded) = maybe_encode(bytes.to_vec());
        let comment = if encoded {
            Some("base64".to_string())
        } else {
            None
        };
        Headers {
            name,
            value,
            comment,
        }
    }

    pub fn hyper(headers: &Headers) -> Result<(HeaderName, HeaderValue), Error> {
        let header_name = HeaderName::from_lowercase(headers.name.to_lowercase().as_bytes())?;
        let bytes = if let Some(c) = &headers.comment {
            if c == "base64" {
                Cow::from(base64::decode(&headers.value)?)
            } else {
                Cow::from(headers.value.as_bytes())
            }
        } else {
            Cow::from(headers.value.as_bytes())
        };

        // We use the unsafe here to return to the client the
        // same value we saw the first time around.
        // This is technically valid because the RFC states
        // that Header Values *should* be ASCII, but if
        // they're not they're supposed to be treated as "opaque data"[1]
        //
        // [1]: see last paragraph of https://tools.ietf.org/html/rfc7230#section-3.2.4
        let header_val = unsafe { HeaderValue::from_shared_unchecked(Bytes::from(bytes.as_ref())) };
        Ok((header_name, header_val))
    }
}

fn maybe_encode(b: Vec<u8>) -> (String, bool) {
    match String::from_utf8(b) {
        Ok(s) => (s, false),
        Err(e) => (base64::encode(e.as_bytes()), true),
    }
}

pub struct RequestBody;

impl RequestBody {
    pub fn har(body: Vec<u8>, mime_type: String) -> Option<PostData> {
        if body.is_empty() {
            None
        } else {
            let (text, base64_encoded) = maybe_encode(body);
            Some(PostData {
                mime_type,
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
    pub fn bytes(data: &PostData) -> Result<(Vec<u8>, String), Error> {
        let encoding = data.mime_type.to_string();
        let body = if let Some(c) = &data.comment {
            if c == "base64" {
                base64::decode(&data.text)?
            } else {
                data.text.as_bytes().to_vec()
            }
        } else {
            data.text.as_bytes().to_vec()
        };
        Ok((body, encoding))
    }
}

pub struct ResponseBody;

impl ResponseBody {
    pub fn har(body: Vec<u8>, mime_type: String) -> Content {
        let size = body.len() as i64;
        let (text, encoded) = maybe_encode(body);
        Content {
            size,
            compression: None,
            mime_type,
            text: if text.is_empty() { None } else { Some(text) },
            encoding: if encoded { Some("base64".into()) } else { None },
            comment: None,
        }
    }

    pub fn hyper(content: &Content) -> Result<(Body, String), Error> {
        if content.text.is_none() {
            return Ok((Body::empty(), "".to_string()));
        }
        let text = content.text.as_ref().map(|t| t.to_string()).unwrap();
        let mime_type = content.mime_type.to_string();
        let body = if let Some(e) = &content.encoding {
            if e == "base64" {
                base64::decode(&text)?
            } else {
                text.as_bytes().to_vec()
            }
        } else {
            text.as_bytes().to_vec()
        };
        Ok((Body::from(Chunk::from(body)), mime_type))
    }
}

pub struct HttpVersion;

impl HttpVersion {
    pub fn har(v: Version) -> String {
        match v {
            Version::HTTP_09 => "HTTP/0.9",
            Version::HTTP_10 => "HTTP/1.0",
            Version::HTTP_11 => "HTTP/1.1",
            Version::HTTP_2 => "HTTP/2",
        }
        .into()
    }

    pub fn hyper(v: &str) -> Result<Version, ConversionError> {
        match v.to_uppercase().as_str() {
            "HTTP/0.9" => Ok(Version::HTTP_09),
            "HTTP/1.0" => Ok(Version::HTTP_10),
            "HTTP/1.1" => Ok(Version::HTTP_11),
            "HTTP/2" => Ok(Version::HTTP_2),
            _ => Err(ConversionError::InvalidHttpVersion(v.to_string())),
        }
    }
}

fn cookie_to_har(c: Cookie) -> Cookies {
    Cookies {
        name: c.name().into(),
        value: c.value().into(),
        path: c.path().map(|p| p.into()),
        domain: c.domain().map(|d| d.into()),
        expires: c.expires().map(|e| format!("{}", e.rfc3339())),
        http_only: c.http_only(),
        secure: c.secure(),
        comment: None,
    }
}

pub struct ClientCookies;

impl ClientCookies {
    pub fn har(m: &HeaderMap) -> Vec<Cookies> {
        m.get_all(header::COOKIE)
            .iter()
            .flat_map(|c| c.to_str().expect("Invalid encoding in cookie").split("; "))
            .map(|c| Cookie::parse(c).expect("Couldn't parse cookie"))
            .map(cookie_to_har)
            .collect()
    }
}

pub struct ServerCookies;

impl ServerCookies {
    pub fn har(m: &HeaderMap) -> Vec<Cookies> {
        m.get_all(header::SET_COOKIE)
            .iter()
            .map(|c| c.to_str().expect("Invalid encoding in cookie"))
            .map(|c| Cookie::parse(c).expect("Couldn't parse cookie"))
            .map(cookie_to_har)
            .collect()
    }
}

pub struct Query;

impl Query {
    pub fn har(q: &Option<&str>) -> Vec<QueryString> {
        q.map(|qs| {
            qs.split('&')
                .filter_map(|pair| {
                    let mut iter = pair.splitn(2, '=');
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
        .unwrap_or_else(Vec::new)
    }
}
