use crate::archive::HarSession;
use crate::config::ProxyServerConfig;
use failure::Error;
use futures::future::{self, FutureResult};
use futures::{Future, Stream};
use hyper::client::{Client as HyperClient, HttpConnector};
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::http::uri::Authority;
use hyper::service::{MakeService, Service};
use hyper::{Body, Chunk, Request, Response, Server, Uri};
use hyper_rustls::HttpsConnector;
use slog::FnValue;
use slog::Logger;
use std::path::{Path, PathBuf};

type Client = HyperClient<HttpsConnector<HttpConnector>, Body>;

// hop-by-hop headers as according to http://www.w3.org/Protocols/rfc2616/rfc2616-sec13.html
lazy_static! {
    static ref HOP_HEADERS: Vec<HeaderName> = vec![
        header::CONNECTION,
        HeaderName::from_static("keep-alive"),
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        header::TE,
        HeaderName::from_static("trailers"),
        header::TRANSFER_ENCODING,
        header::UPGRADE,
    ];
}

#[derive(Debug, Fail)]
#[fail(display = "Proxy target '{}' has no authority", uri)]
pub struct AuthorityError {
    uri: Uri,
}

pub struct MakeProxyService {
    logger: Logger,
    proxy_for: Uri,
    client: Client,
    archive_path: PathBuf,
    ignored_status_codes: Vec<u16>,
}

pub struct ProxyService {
    logger: Logger,
    proxy_for: Uri,
    host_header: HeaderValue,
    client: Client,
    archive_path: PathBuf,
    ignored_status_codes: Vec<u16>,
}

fn remove_hop_headers(headers: &mut HeaderMap) {
    for h in HOP_HEADERS.iter() {
        headers.remove(h);
    }
}

fn extract_authority(uri: &Uri) -> Result<Authority, AuthorityError> {
    uri.authority_part()
        .cloned()
        .ok_or_else(|| AuthorityError { uri: uri.clone() })
}

fn calculate_target_uri<B>(requested: &Uri, proxied: &Uri) -> Result<Uri, Error> {
    let authority = extract_authority(proxied)?;
    let mut builder = Uri::builder();
    builder
        .scheme(proxied.scheme_str().unwrap_or("http"))
        .authority(authority.clone());
    if let Some(p) = requested.path_and_query() {
        builder.path_and_query(p.clone());
    }

    Ok(builder.build()?)
}

fn create_proxied_response<B>(mut response: Response<B>) -> Response<B> {
    remove_hop_headers(response.headers_mut());
    response
}

impl MakeProxyService {
    pub fn new<S: Into<String>, P: AsRef<Path>, V: Into<Vec<u16>>>(
        logger: &Logger,
        proxy_for: Uri,
        name: S,
        archive_path: P,
        ignored_status_codes: V,
    ) -> MakeProxyService {
        let name = name.into();
        let uri = format!("{}", proxy_for);
        let logger = logger.new(o!("for" => uri));
        let https = HttpsConnector::new(4);
        let client: Client = HyperClient::builder().build(https);
        MakeProxyService {
            logger,
            proxy_for,
            client,
            archive_path: archive_path.as_ref().join(name),
            ignored_status_codes: ignored_status_codes.into(),
        }
    }
}

impl ProxyService {
    fn new(
        logger: Logger,
        proxy_for: Uri,
        host_header: HeaderValue,
        client: Client,
        archive_path: PathBuf,
        ignored_status_codes: Vec<u16>,
    ) -> ProxyService {
        ProxyService {
            logger,
            proxy_for,
            client,
            host_header,
            archive_path,
            ignored_status_codes,
        }
    }

    fn create_proxied_request<B>(
        &self,
        mut req: Request<B>,
        target: Uri,
        host_header: HeaderValue,
    ) -> Request<B> {
        remove_hop_headers(req.headers_mut());
        req.headers_mut().insert(header::HOST, host_header);
        *req.uri_mut() = target;
        req
    }
}

impl<C> MakeService<C> for MakeProxyService {
    type ReqBody = <ProxyService as Service>::ReqBody;
    type ResBody = <ProxyService as Service>::ResBody;
    type Error = <ProxyService as Service>::Error;
    type Future = FutureResult<Self::Service, Self::MakeError>;
    type Service = ProxyService;
    type MakeError = Error;

    fn make_service(&mut self, _ctx: C) -> Self::Future {
        let authority = match extract_authority(&self.proxy_for) {
            Ok(a) => a,
            Err(e) => {
                error!(self.logger, "{}", e);
                return future::err(e.into());
            }
        };
        trace!(self.logger, "Extracted authority '{}'", authority);

        let host_header: HeaderValue = match authority.as_str().parse() {
            Ok(h) => h,
            Err(e) => return future::err(e.into()),
        };
        trace!(self.logger, "Calculated new Host value {:?}", host_header);

        let proxy = ProxyService::new(
            self.logger.clone(),
            self.proxy_for.clone(),
            host_header,
            self.client.clone(),
            self.archive_path.clone(),
            self.ignored_status_codes.clone(),
        );
        trace!(self.logger, "Created ProxyService instance");
        future::ok(proxy)
    }
}

impl Service for ProxyService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = Error;
    type Future = Box<dyn Future<Item = Response<Self::ResBody>, Error = Self::Error> + Send>;
    fn call(&mut self, req: Request<Self::ReqBody>) -> Self::Future {
        trace!(self.logger, "Starting request");
        let target = match calculate_target_uri::<Self::ReqBody>(&req.uri(), &self.proxy_for) {
            Ok(u) => u,
            Err(e) => return Box::new(future::err(e)),
        };

        trace!(self.logger, "Calculated new Uri '{}'", target);

        let proxied_req = self.create_proxied_request(req, target, self.host_header.clone());
        let path = proxied_req
            .uri()
            .path_and_query()
            .map(|pq| format!("{}", pq))
            .unwrap_or_else(|| "/".to_string());
        let path_without_query = proxied_req.uri().path().to_string();
        let method = proxied_req.method().to_string();
        if !self.archive_path.exists() {
            trace!(self.logger, "Creating dir {:?}", &self.archive_path);
            match std::fs::create_dir_all(&self.archive_path) {
                Ok(_) => {}
                Err(e) => return Box::new(future::err(e.into())),
            }
        }
        let ignored_status_codes = self.ignored_status_codes.clone();
        let archive_path = self.archive_path.clone();

        let req_logger = self
            .logger
            .new(o!("path" => path.clone(), "method" => method.clone()));

        let (head, body) = proxied_req.into_parts();
        let client = self.client.clone();
        let fut = body
            .concat2()
            .map_err(Error::from)
            .and_then(move |b| {
                let mut har = HarSession::new();
                let body: Vec<u8> = b.into_bytes().into_iter().collect();
                har.record_request(&head, body.clone());
                let new_body: Body = Body::from(Chunk::from(body));
                let req = Request::from_parts(head, new_body);
                Ok((req, har))
            })
            .and_then(move |(req, mut har)| {
                info!(req_logger, "Sending request");
                let err_logger = req_logger.new(o!("area" => "client-error"));
                har.start_session();
                client
                    .request(req)
                    .map_err(move |e| {
                        error!(err_logger, "{}", e);
                        Error::from(e)
                    })
                    .and_then(move |resp| {
                        let res = create_proxied_response(resp);
                        let (head, body) = res.into_parts();
                        let res_logger = req_logger.new(o!("status" => head.status.as_u16()));
                        let err_logger = res_logger.new(o!("area" => "body-error"));
                        let resp_err_logger = res_logger.new(o!("area" => "resp-error"));
                        body.concat2()
                            .map_err(move |e| {
                                error!(err_logger, "{}", e);
                                Error::from(e)
                            })
                            .and_then(move |b| {
                                let body: Vec<u8> = b.into_bytes().into_iter().collect();
                                har.record_response(&head, body.clone());
                                if ignored_status_codes.contains(&head.status.as_u16()) {
                                    info!(
                                        res_logger,
                                        "Ignoring response with status {}",
                                        head.status.as_u16()
                                    );
                                } else {
                                    har.commit()?;
                                    let file_name_part =
                                        format!("{}.{}", method, path_without_query);
                                    trace!(
                                        res_logger,
                                        "Writing file to dir {:?}, name fragment {}",
                                        &archive_path,
                                        file_name_part
                                    );
                                    let filename =
                                        har.write_to_dir(&archive_path, file_name_part)?;
                                    info!(
                                    res_logger,
                                    "Received Response, Wrote file"; "file_name" => FnValue(|_| {
                                        archive_path.join(&filename).to_string_lossy().into_owned()
                                    }));
                                }
                                let new_body: Body = Body::from(Chunk::from(body));
                                Ok(Response::from_parts(head, new_body))
                            })
                            .map_err(move |e| {
                                error!(resp_err_logger, "{}", e);
                                e
                            })
                    })
            });

        Box::new(fut)
    }
}

pub fn get_proxy_servers<I: IntoIterator<Item = ProxyServerConfig>>(
    logger: Logger,
    servers: I,
) -> impl Future<Item = (), Error = ()> {
    let futs = servers.into_iter().map(move |s| {
        let logger = logger.new(o!("project" => s.name.to_string(), "mode" => "recording"));
        let req_logger = logger.new(o!( "lifecycle" => "run"));
        let start_logger = logger.new(o!("lifecycle" => "startup"));
        let serve_logger = logger.new(o!("lifecycle" => "error"));
        let socket = s.socket;
        let factory = MakeProxyService::new(
            &req_logger,
            s.proxy_for,
            s.name,
            s.archive_path,
            s.ignored_status_codes,
        );
        future::lazy(move || {
            info!(start_logger, "Listening on {}", &socket);
            Ok::<(), ()>(())
        })
        .then(move |_| {
            Server::bind(&socket)
                .serve(factory)
                .map_err(move |e| error!(serve_logger, "{}", e))
        })
    });

    future::join_all(futs).map(|_| ())
}
