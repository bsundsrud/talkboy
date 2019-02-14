use crate::archive::{ArchivedRequest, RequestFacts};
use crate::config::{DelayOptions, PlaybackServerConfig};
use failure::Error;
use futures::future::{self, Either, FutureResult};
use futures::{Future, Stream};
use hyper::http::request::Parts as RequestParts;
use hyper::service::{MakeService, Service};
use hyper::{header, Body, Chunk, Request, Response, Server};
use slog::Logger;
use std::sync::{Arc, RwLock};

pub struct MakePlaybackService {
    logger: Logger,
    transactions: Arc<RwLock<Vec<ArchivedRequest>>>,
    delay: DelayOptions,
}

pub struct PlaybackService {
    logger: Logger,
    transactions: Arc<RwLock<Vec<ArchivedRequest>>>,
    delay: DelayOptions,
}

impl<C> MakeService<C> for MakePlaybackService {
    type ReqBody = <PlaybackService as Service>::ReqBody;
    type ResBody = <PlaybackService as Service>::ResBody;
    type Error = <PlaybackService as Service>::Error;
    type Future = FutureResult<Self::Service, Self::MakeError>;
    type Service = PlaybackService;
    type MakeError = Error;

    fn make_service(&mut self, _ctx: C) -> Self::Future {
        trace!(self.logger, "Creating Playback Service");
        future::ok(PlaybackService::new(
            self.logger.clone(),
            self.transactions.clone(),
            self.delay.clone(),
        ))
    }
}

impl Service for PlaybackService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = Error;
    type Future = Box<Future<Item = Response<Self::ResBody>, Error = Self::Error> + Send>;
    fn call(&mut self, req: Request<Self::ReqBody>) -> Self::Future {
        let (parts, body) = req.into_parts();
        let transactions = self.transactions.clone();
        let method = parts.method.to_string();
        let path = parts
            .uri
            .path_and_query()
            .map(|pq| format!("{}", pq))
            .unwrap_or("/".to_string());

        let logger = self.logger.new(o!("method" => method, "path" => path));
        let delay = self.delay;
        let r = body
            .concat2()
            .map_err(|e| Error::from(e))
            .and_then(move |b| {
                let transactions = &transactions.read().unwrap();
                if let Some(m) = find_match(&transactions, &parts, b.into_bytes().to_vec()) {
                    info!(logger, "Serving archived response");
                    let response = m.hyper_response();
                    Either::A(
                        m.delay(&delay)
                            .map_err(|e| Error::from(e))
                            .and_then(move |_| response),
                    )
                } else {
                    error!(logger, "Response for request not found in archives");
                    Either::B(future::ok(
                        Response::builder()
                            .status(404)
                            .body(Body::from(Chunk::from("Not Found")))
                            .unwrap(),
                    ))
                }
            });
        Box::new(r)
    }
}

impl MakePlaybackService {
    pub fn new(
        logger: Logger,
        transactions: Vec<ArchivedRequest>,
        delay: DelayOptions,
    ) -> MakePlaybackService {
        MakePlaybackService {
            logger,
            transactions: Arc::new(RwLock::new(transactions)),
            delay,
        }
    }
}

impl PlaybackService {
    fn new(
        logger: Logger,
        transactions: Arc<RwLock<Vec<ArchivedRequest>>>,
        delay: DelayOptions,
    ) -> PlaybackService {
        PlaybackService {
            logger,
            transactions,
            delay,
        }
    }
}

fn hyper_request_to_facts(parts: &RequestParts, body: Vec<u8>) -> Vec<RequestFacts> {
    let mut results = Vec::with_capacity(4);
    let method = parts.method.clone();
    results.push(RequestFacts::Method(method));
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("")
        .to_string();
    results.push(RequestFacts::PathAndQuery(path_and_query));

    let content_type = parts
        .headers
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or_else(|| "")
        .to_string();

    if !body.is_empty() {
        results.push(RequestFacts::Body {
            content_type,
            data: body,
        });
    }

    let headers = parts
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    results.push(RequestFacts::Headers(headers));

    results
}

fn find_match<'a, 'b>(
    transactions: &'a [ArchivedRequest],
    parts: &'b RequestParts,
    body: Vec<u8>,
) -> Option<&'a ArchivedRequest> {
    let facts = hyper_request_to_facts(&parts, body);
    transactions.iter().find(|t| t.matches(&facts))
}

pub fn get_playback_servers<I: IntoIterator<Item = PlaybackServerConfig>>(
    logger: Logger,
    servers: I,
) -> impl Future<Item = (), Error = ()> {
    let futs = servers.into_iter().map(move |s| {
        let req_logger = logger.new(o!("server" => s.name.to_string(), "lifecycle" => "run"));
        let start_logger = req_logger.new(o!("lifecycle" => "startup"));
        let serve_logger = req_logger.new(o!("lifecycle" => "error"));
        let socket = s.socket.clone();
        let factory = MakePlaybackService::new(req_logger, s.archives, s.delay);
        future::lazy(move || {
            info!(start_logger, "Playback listening on {}", &socket);
            Ok(())
        })
        .then(move |_: Result<(), ()>| {
            Server::bind(&socket)
                .serve(factory)
                .map_err(move |e| error!(serve_logger, "{}", e))
        })
    });

    future::join_all(futs).map(|_| ()).map_err(|_| ())
}
