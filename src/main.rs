use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};

use hyper_util::{client::legacy::connect::HttpConnector, rt::TokioExecutor};
use tokio::{
    net::TcpStream,
    time::{interval, timeout},
};

type Client = hyper_util::client::legacy::Client<HttpConnector, Body>;

#[derive(Clone)]
struct AppState {
    client: Client,
    backends: Arc<Vec<String>>,
    healthy: Arc<Vec<AtomicBool>>,
    next: Arc<AtomicUsize>,
}

fn pick_backend(state: &AppState) -> Option<usize> {
    let n = state.backends.len();
    for _ in 0..n {
        let i = state.next.fetch_add(1, Ordering::Relaxed) % n;
        if state.healthy[i].load(Ordering::Relaxed) {
            return Some(i);
        }
    }
    None
}

fn spawn_health_checks(backends: Arc<Vec<String>>, healthy: Arc<Vec<AtomicBool>>) {
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(2));
        loop {
            tick.tick().await;
            for (i, addr) in backends.iter().enumerate() {
                let up = matches!(
                    timeout(Duration::from_secs(1), TcpStream::connect(addr)).await,
                    Ok(Ok(_))
                );
                let was_up = healthy[i].swap(up, Ordering::Relaxed);
                if was_up != up {
                    println!("backend {} is now {}", addr, if up { "UP" } else { "DOWN" });
                }
            }
        }
    });
}

fn build_request(
    method: &Method,
    headers: &HeaderMap,
    backend: &str,
    path_and_query: &str,
    body: Bytes,
) -> Result<axum::http::Request<Body>, StatusCode> {
    let uri = Uri::try_from(format!("http://{}{}", backend, path_and_query))
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut req = axum::http::Request::builder()
        .method(method.clone())
        .uri(uri)
        .body(Body::from(body))
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    *req.headers_mut() = headers.clone();
    Ok(req)
}

async fn proxy(State(state): State<AppState>, req: Request) -> Result<Response, StatusCode> {
    let (parts, body) = req.into_parts();
    let body = to_bytes(body, 1024 * 1024)
        .await
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/")
        .to_string();

    for _attempt in 0..2 {
        let i = pick_backend(&state).ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let backend = &state.backends[i];
        let req = build_request(
            &parts.method,
            &parts.headers,
            backend,
            &path_and_query,
            body.clone(),
        )?;

        match state.client.request(req).await {
            Ok(res) => {
                let mut response = res.into_response();
                response
                    .headers_mut()
                    .insert("x-backend", backend.parse().unwrap());
                return Ok(response);
            }

            Err(e) if e.is_connect() => {
                state.healthy[i].store(false, Ordering::Relaxed);
                println!("backend {} refused connection, marked DOWN", backend);
                continue;
            }
            Err(_) => return Err(StatusCode::BAD_GATEWAY),
        }
    }
    Err(StatusCode::BAD_GATEWAY)
}

#[tokio::main]

async fn main() {
    let client: Client =
        hyper_util::client::legacy::Client::<(), ()>::builder(TokioExecutor::new())
            .build(HttpConnector::new());

    let backends = Arc::new(vec![
        "127.0.0.1:8081".to_string(),
        "127.0.0.1:8082".to_string(),
    ]);
    let healthy: Arc<Vec<AtomicBool>> =
        Arc::new(backends.iter().map(|_| AtomicBool::new(true)).collect());
    spawn_health_checks(backends.clone(), healthy.clone());

    let state = AppState {
        client,
        backends,
        healthy,
        next: Arc::new(AtomicUsize::new(0)),
    };

    let app = Router::new().fallback(proxy).with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
    println!("Proxy listening on 127.0.0.1:3000");
    axum::serve(listener, app).await.unwrap();
}
