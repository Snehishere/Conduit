//! The operator-facing HTTP surface: `/healthz`, `/health`, `/metrics`, `/pin`.
//!
//! Separate from the WebSocket listeners because it answers a different
//! question (is the relay alive, and what is it doing) on a different port, and
//! because a health check must not be able to reach the routing table.

use std::net::SocketAddr;
use std::sync::Arc;

use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use log::info;
use tokio::net::TcpListener;
use tokio::sync::watch;

use super::config::bearer_token_authorized;
use super::metrics::{build_metrics_json, build_prometheus_metrics};
use super::state::AppState;

pub(crate) async fn spawn_health_server(
    state: Arc<AppState>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> std::io::Result<tokio::task::JoinHandle<()>> {
    // Loopback unless the host bound it elsewhere: these endpoints publish the
    // peer count and the certificate pin, and are not for the LAN.
    let health_addr = SocketAddr::new(state.config.health_bind, state.config.health_port);
    let health_listener = TcpListener::bind(health_addr).await?;
    Ok(tokio::spawn(async move {
        info!("Health check listener on: {}", health_addr);
        loop {
            tokio::select! {
                result = health_listener.accept() => {
                    if let Ok((stream, _)) = result {
                        let io = TokioIo::new(stream);
                        let state = state.clone();
                        tokio::spawn(async move {
                            let service = hyper::service::service_fn(move |req: Request<Incoming>| {
                                let state = state.clone();
                                async move {
                                    match req.uri().path() {
                                        "/healthz" => {
                                            let resp: Response<String> = Response::builder()
                                                .status(StatusCode::OK)
                                                .header("Content-Type", "application/json")
                                                .body(r#"{"status":"ok"}"#.to_string())
                                                .unwrap();
                                            Ok::<_, hyper::Error>(resp)
                                        }
                                        "/health" | "/" => {
                                            let authorized = req
                                                .headers()
                                                .get(hyper::header::AUTHORIZATION)
                                                .and_then(|v| v.to_str().ok())
                                                .map(|h| {
                                                    bearer_token_authorized(
                                                        h,
                                                        &state.config.effective_health_token(),
                                                    )
                                                })
                                                .unwrap_or(false);
                                            if !authorized {
                                                let resp: Response<String> = Response::builder()
                                                    .status(StatusCode::UNAUTHORIZED)
                                                    .header("WWW-Authenticate", "Bearer")
                                                    .header("Content-Type", "application/json")
                                                    .body(
                                                        r#"{"error":"unauthorized"}"#.to_string(),
                                                    )
                                                    .unwrap();
                                                return Ok::<_, hyper::Error>(resp);
                                            }
                                            let body = build_metrics_json(&state);
                                            let resp: Response<String> = Response::builder()
                                                .status(StatusCode::OK)
                                                .header("Content-Type", "application/json")
                                                .body(body.to_string())
                                                .unwrap();
                                            Ok::<_, hyper::Error>(resp)
                                        }
                                        "/metrics" => {
                                            // Unauthenticated by default (tested
                                            // decision: Prometheus must scrape
                                            // without the health secret), but
                                            // gateable via RELAY_METRICS_TOKEN.
                                            if let Some(expected) =
                                                state.config.metrics_token.as_deref()
                                            {
                                                let authorized = req
                                                    .headers()
                                                    .get(hyper::header::AUTHORIZATION)
                                                    .and_then(|v| v.to_str().ok())
                                                    .map(|h| {
                                                        bearer_token_authorized(h, expected)
                                                    })
                                                    .unwrap_or(false);
                                                if !authorized {
                                                    let resp: Response<String> = Response::builder()
                                                        .status(StatusCode::UNAUTHORIZED)
                                                        .header("WWW-Authenticate", "Bearer")
                                                        .header("Content-Type", "application/json")
                                                        .body(r#"{"error":"unauthorized"}"#.to_string())
                                                        .unwrap();
                                                    return Ok::<_, hyper::Error>(resp);
                                                }
                                            }
                                            let body = build_prometheus_metrics(&state);
                                            let resp: Response<String> = Response::builder()
                                                .status(StatusCode::OK)
                                                .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
                                                .body(body)
                                                .unwrap();
                                            Ok::<_, hyper::Error>(resp)
                                        }
                                        "/pin" => {
                                            // Certificate pin discovery, so an
                                            // operator never has to compute it
                                            // out-of-band from the file on disk
                                            // (and never pins the wrong hash).
                                            let body = match &state.tls_pin {
                                                Some(pin) => serde_json::json!({
                                                    "sha256": pin,
                                                    "algorithm": "spki-sha256",
                                                })
                                                .to_string(),
                                                None => serde_json::json!({
                                                    "sha256": serde_json::Value::Null,
                                                    "algorithm": "spki-sha256",
                                                    "error": "tls_unavailable",
                                                })
                                                .to_string(),
                                            };
                                            let resp: Response<String> = Response::builder()
                                                .status(StatusCode::OK)
                                                .header("Content-Type", "application/json")
                                                .body(body)
                                                .unwrap();
                                            Ok::<_, hyper::Error>(resp)
                                        }
                                        _ => {
                                            let resp: Response<String> = Response::builder()
                                                .status(StatusCode::NOT_FOUND)
                                                .body("Not Found".to_string())
                                                .unwrap();
                                            Ok::<_, hyper::Error>(resp)
                                        }
                                    }
                                }
                            });
                            hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                                .serve_connection(io, service)
                                .await
                        });
                    }
                }
                _ = shutdown_rx.changed() => {
                    info!("Health check listener shutting down");
                    break;
                }
            }
        }
    }))
}
