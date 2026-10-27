use bytes::Bytes;
use fastwebsockets::upgrade;
use handlebars::Handlebars;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::body::Frame;
use hyper::body::Body;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

use crate::websocket::{proximity_websocket_channel, ProximityClientConfig, ProximityClientHandler};

#[derive(Debug)]
pub enum WebStartUpMessage {
    Start,
    Error,
}

pub enum Web2UiMessage {
    UInputInaccessible,
}

pub const INDEX_HTML: &str = std::include_str!("../www/templates/index.html");
pub const ACCESS_HTML: &str = std::include_str!("../www/static/access_code.html");
pub const STYLE_CSS: &str = std::include_str!("../www/static/style.css");
pub const LIB_JS: &str = std::include_str!("../www/static/lib.js");

#[derive(Serialize)]
struct IndexTemplateContext {
    access_code: Option<String>,
    uinput_enabled: bool,
    capture_cursor_enabled: bool,
    log_level: String,
    enable_custom_input_areas: bool,
}

fn response_from_str(s: &str, content_type: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", content_type)
        .header("cache-control", "no-store, max-age=0")
        .header("pragma", "no-cache")
        .body(s.to_string().into())
        .unwrap()
}

fn response_not_found() -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("content-type", "text/html; charset=utf-8")
        .body("Not found!".into())
        .unwrap()
}

// endless HTTP body driven by the audio capture thread; ends once the
// thread stops (client gone or capture failure)
struct AudioBody {
    rx: mpsc::Receiver<Bytes>,
}

impl Body for AudioBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.get_mut().rx.poll_recv(cx) {
            std::task::Poll::Ready(Some(data)) => {
                std::task::Poll::Ready(Some(Ok(Frame::data(data))))
            }
            std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        false
    }
}

async fn response_from_path_or_default(
    path: Option<&PathBuf>,
    default: &str,
    content_type: &str,
) -> Response<Full<Bytes>> {
    match path {
        Some(path) => match tokio::fs::read_to_string(path).await {
            Ok(s) => response_from_str(&s, content_type),
            Err(err) => {
                warn!("Failed to load file: {}", err);
                response_from_str(default, content_type)
            }
        },
        None => response_from_str(default, content_type),
    }
}

async fn serve(
    addr: SocketAddr,
    mut req: Request<Incoming>,
    context: Arc<Context<'_>>,
    sender_ui: mpsc::Sender<Web2UiMessage>,
    num_clients: Arc<AtomicUsize>,
    semaphore_websocket_shutdown: Arc<tokio::sync::Semaphore>,
    notify_disconnect: Arc<tokio::sync::Notify>,
) -> Result<Response<BoxBody<Bytes, Infallible>>, hyper::Error> {
    debug!("Got request: {:?}", req);
    let mut authed = false;
    if let Some(access_code) = &context.web_config.access_code {
        if req.method() == Method::GET
            && (req.uri().path() == "/"
                || req.uri().path() == "/ws"
                || req.uri().path() == "/audio_stream")
        {
            use url::form_urlencoded;
            if let Some(query) = req.uri().query() {
                let params = form_urlencoded::parse(query.as_bytes())
                    .into_owned()
                    .collect::<HashMap<String, String>>();
                if let Some(code) = params.get("access_code") {
                    if code == access_code {
                        authed = true;
                        debug!(address = ?addr, "Web-Client authenticated.");
                    }
                }
            }
        }
    } else {
        authed = true;
    }
    if req.method() != Method::GET {
        return Ok(response_not_found().map(|r| r.boxed()));
    }
    match req.uri().path() {
        "/" => {
            if !authed {
                return Ok(response_from_path_or_default(
                    context.web_config.custom_access_html.as_ref(),
                    ACCESS_HTML,
                    "text/html; charset=utf-8",
                )
                .await
                .map(|r| r.boxed()));
            }
            let config = IndexTemplateContext {
                access_code: context.web_config.access_code.clone(),
                uinput_enabled: cfg!(target_os = "linux"),
                capture_cursor_enabled: cfg!(not(target_os = "windows")),
                log_level: crate::log::get_log_level().to_string(),
                enable_custom_input_areas: context.web_config.enable_custom_input_areas,
            };

            let html = if let Some(path) = context.web_config.custom_index_html.as_ref() {
                let mut reg = Handlebars::new();
                if let Err(err) = reg.register_template_file("index", path) {
                    warn!("Failed to register template from path: {}", err);
                    context.templates.render("index", &config)
                } else {
                    reg.render("index", &config)
                }
            } else {
                context.templates.render("index", &config)
            };

            match html {
                Ok(html) => {
                    Ok(response_from_str(&html, "text/html; charset=utf-8").map(|r| r.boxed()))
                }
                Err(err) => {
                    error!("Failed to render index template: {}", err);
                    Ok(response_not_found().map(|r| r.boxed()))
                }
            }
        }
        "/ws" => {
            if !authed {
                return Ok(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body("unauthorized".to_string().boxed())
                    .unwrap());
            }

            // A plain GET without websocket upgrade headers must not panic
            // the connection task; answer with 400 instead.
            let (response, fut) = match upgrade::upgrade(&mut req) {
                Ok(upgrade) => upgrade,
                Err(err) => {
                    warn!("Not a websocket upgrade request: {err}.");
                    return Ok(Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body("expected a websocket upgrade".to_string().boxed())
                        .unwrap());
                }
            };
            num_clients.fetch_add(1, Ordering::Relaxed);

            let config = context.proximity_client_config.clone();
            tokio::spawn(async move {
                match fut.await {
                    Ok(ws) => {
                        let (sender, receiver) =
                            proximity_websocket_channel(ws, semaphore_websocket_shutdown);
                        std::thread::spawn(move || {
                            let client = ProximityClientHandler::new(
                                sender,
                                receiver,
                                || {
                                    if let Err(err) =
                                        sender_ui.blocking_send(Web2UiMessage::UInputInaccessible)
                                    {
                                        warn!(
                                            "Failed to send message 'UInputInaccessible': {err}."
                                        );
                                    }
                                },
                                config,
                            );
                            client.run();
                            num_clients.fetch_sub(1, Ordering::Relaxed);
                            notify_disconnect.notify_waiters();
                        });
                    }
                    Err(err) => {
                        eprintln!("Error in websocket connection: {}", err);
                        num_clients.fetch_sub(1, Ordering::Relaxed);
                        notify_disconnect.notify_waiters();
                    }
                }
            });

            Ok(response.map(|r| r.boxed()))
        }
        "/audio_stream" => {
            if !authed {
                return Ok(response_from_path_or_default(
                    context.web_config.custom_access_html.as_ref(),
                    ACCESS_HTML,
                    "text/html; charset=utf-8",
                )
                .await
                .map(|r| r.boxed()));
            }
            if !cfg!(target_os = "windows") {
                return Ok(response_not_found().map(|r| r.boxed()));
            }

            // one capture + encode thread per client; when the browser
            // stops reading, the channel closes and the thread exits
            let (sender, receiver) = mpsc::channel::<Bytes>(64);
            std::thread::Builder::new()
                .name("audio_loopback".into())
                .spawn(move || crate::audio::audio_loopback(sender))
                .map_err(|err| {
                    error!("Failed to spawn audio thread: {}", err)
                })
                .ok();

            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "audio/aac")
                .header("cache-control", "no-store")
                .body(BoxBody::new(AudioBody { rx: receiver }))
                .unwrap());
        }
        "/style.css" => Ok(response_from_path_or_default(
            context.web_config.custom_style_css.as_ref(),
            STYLE_CSS,
            "text/css; charset=utf-8",
        )
        .await
        .map(|r| r.boxed())),
        "/lib.js" => Ok(response_from_path_or_default(
            context.web_config.custom_lib_js.as_ref(),
            LIB_JS,
            "text/javascript; charset=utf-8",
        )
        .await
        .map(|r| r.boxed())),
        _ => Ok(response_not_found().map(|r| r.boxed())),
    }
}

#[derive(Clone)]
pub struct WebServerConfig {
    pub bind_addr: SocketAddr,
    pub access_code: Option<String>,
    pub custom_index_html: Option<PathBuf>,
    pub custom_access_html: Option<PathBuf>,
    pub custom_style_css: Option<PathBuf>,
    pub custom_lib_js: Option<PathBuf>,
    pub enable_custom_input_areas: bool,
}

struct Context<'a> {
    web_config: WebServerConfig,
    proximity_client_config: ProximityClientConfig,
    templates: Handlebars<'a>,
}

pub fn run(
    sender_ui: tokio::sync::mpsc::Sender<Web2UiMessage>,
    sender_startup: oneshot::Sender<WebStartUpMessage>,
    notify_shutdown: Arc<tokio::sync::Notify>,
    web_server_config: WebServerConfig,
    proximity_client_config: ProximityClientConfig,
) -> std::thread::JoinHandle<()> {
    let mut templates = Handlebars::new();
    templates
        .register_template_string("index", INDEX_HTML)
        .unwrap();

    let context = Context {
        web_config: web_server_config,
        proximity_client_config,
        templates,
    };
    std::thread::spawn(move || run_server(context, sender_ui, sender_startup, notify_shutdown))
}

#[tokio::main]
async fn run_server(
    context: Context<'static>,
    sender_ui: tokio::sync::mpsc::Sender<Web2UiMessage>,
    sender_startup: oneshot::Sender<WebStartUpMessage>,
    notify_shutdown: Arc<tokio::sync::Notify>,
) {
    let addr = context.web_config.bind_addr;

    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => {
            error!("Failed to bind to socket: {err}.");
            sender_startup.send(WebStartUpMessage::Error).unwrap();
            return;
        }
    };

    sender_startup.send(WebStartUpMessage::Start).unwrap();

    let context = Arc::new(context);

    let broadcast_shutdown = Arc::new(tokio::sync::Notify::new());

    let num_clients = Arc::new(AtomicUsize::new(0));
    let notify_disconnect = Arc::new(tokio::sync::Notify::new());
    let semaphore_websocket_shutdown = Arc::new(tokio::sync::Semaphore::new(0));

    loop {
        let (tcp, remote_address) = tokio::select! {
            res = listener.accept() => {
                match res {
                    Ok(conn) => conn,
                    Err(err) => {
                        warn!("Connection failed: {err}.");
                        continue;
                    }
                }
            },
            _ = notify_shutdown.notified() => {
                info!("Webserver is shutting down.");
                broadcast_shutdown.notify_waiters();
                break;
            }
        };

        debug!(address = ?remote_address, "Client connected.");

        let io = TokioIo::new(tcp);

        let sender_ui = sender_ui.clone();
        let broadcast_shutdown = broadcast_shutdown.clone();
        let context = context.clone();
        let num_clients = num_clients.clone();
        let semaphore_websocket_shutdown = semaphore_websocket_shutdown.clone();
        let notify_disconnect = notify_disconnect.clone();

        tokio::task::spawn(async move {
            let conn = http1::Builder::new().serve_connection(
                io,
                service_fn({
                    move |req| {
                        let context = context.clone();
                        let num_clients = num_clients.clone();
                        let semaphore_websocket_shutdown = semaphore_websocket_shutdown.clone();
                        let notify_disconnect = notify_disconnect.clone();
                        serve(
                            remote_address,
                            req,
                            context,
                            sender_ui.clone(),
                            num_clients,
                            semaphore_websocket_shutdown,
                            notify_disconnect,
                        )
                    }
                }),
            );

            let conn = conn.with_upgrades();

            tokio::select! {
                conn = conn => match conn {
                    Ok(_) => (),
                    Err(err) => {
                        warn!("Error polling connection ({remote_address}): {err}.")
                    }
                },
                _ = broadcast_shutdown.notified() => {
                    info!("Closing connection to: {remote_address}.");
                }
            }
        });
    }

    semaphore_websocket_shutdown.add_permits(num_clients.load(Ordering::Relaxed));

    loop {
        let remaining_clients = num_clients.load(Ordering::Relaxed);
        if remaining_clients == 0 {
            break;
        } else {
            debug!("Waiting for remaining clients ({remaining_clients}) to disconnect.");
        }
        tokio::select! {
            _ = notify_disconnect.notified() => (),
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                semaphore_websocket_shutdown.add_permits(num_clients.load(Ordering::Relaxed));
            },
        }
    }
}
