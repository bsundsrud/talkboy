mod convert;
mod load;
mod store;

use failure::Error;
use har::v1_2::Response as HarResponse;
use hyper::header::{HeaderName, HeaderValue};
use hyper::http::Method;
use hyper::{Body, Response as HyperResponse};

use crate::config::DelayOptions;
pub use load::{HarLoader, HarLoadingError};
use std::time::{Duration, Instant};
pub use store::{HarSession, IncompleteEntryError};
use tokio::timer::Delay;

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
        builder.version(convert::HttpVersion::hyper(&self.response.http_version)?);
        for h in &self.response.headers {
            let (k, v) = convert::Header::hyper(&h)?;
            builder.header(k, v);
        }
        // ignoring the mime type from the Content object because the Content-Type header should
        // should have already been set
        let (body, _mime_type) = convert::ResponseBody::hyper(&self.response.content)?;
        Ok(builder.body(body)?)
    }

    pub fn delay(&self, d: &DelayOptions) -> Delay {
        match d {
            DelayOptions::None => Delay::new(Instant::now()),
            DelayOptions::Original => Delay::new(Instant::now() + self.original_timing),
            DelayOptions::Static { millis: ms } => {
                Delay::new(Instant::now() + Duration::from_millis(*ms))
            }
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
