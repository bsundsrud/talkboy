mod load;
mod store;

use failure::Error;
use har::v1_2::Response as HarResponse;
use hyper::header::{HeaderName, HeaderValue};
use hyper::http::Method;
use hyper::{Body, Chunk, Response as HyperResponse};

pub use load::{HarLoader, HarLoadingError};
use std::time::Duration;
pub use store::{HarSession, IncompleteEntryError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestFacts {
    Method(Method),
    PathAndQuery(String),
    Body { content_type: String, data: Vec<u8> },
    Headers(Vec<(HeaderName, HeaderValue)>),
}

impl RequestFacts {
    pub fn matches_type(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

#[derive(Debug, Clone)]
pub struct ArchivedRequest {
    original_timing: Duration,
    facts: Vec<RequestFacts>,
    response: HarResponse,
}

impl ArchivedRequest {
    pub fn hyper_response(&self) -> Result<HyperResponse<Body>, Error> {
        let mut builder = HyperResponse::builder();
        builder.status(self.response.status as u16);
        builder.version(load::http_version_for_str(&self.response.http_version)?);
        for h in &self.response.headers {
            let header_name = HeaderName::from_lowercase(h.name.to_lowercase().as_bytes())?;
            builder.header(header_name, h.value.to_string());
        }
        // ignoring the mime type from the Content object because the Content-Type header should
        // should have already been set
        let (bytes, _mime_type) = load::response_body_and_encoding(&self.response.content)?;
        if let Some(b) = bytes {
            let body = Body::from(Chunk::from(b));
            Ok(builder.body(body)?)
        } else {
            Ok(builder.body(Body::empty())?)
        }
    }

    pub fn matches(&self, facts: &[RequestFacts]) -> bool {
        facts
            .iter()
            .filter_map(|f| {
                if let Some(my_fact) = self.facts.iter().find(|m| m.matches_type(f)) {
                    Some((f, my_fact))
                } else {
                    None
                }
            })
            .all(|(f, o)| f == o)
    }
}
