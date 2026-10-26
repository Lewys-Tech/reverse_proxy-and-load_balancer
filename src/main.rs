use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use axum::{
    body::Body,
    extract::{Request, State},
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
    Router,
};

use hyper_util::{client::legacy::connect::HttpConnector, rt::TokioExecutor};

type Client = hyper_util::client::legacy::Client<HttpConnector, Body>;

#[derive(Clone)]
struct AppState {
    client: Client,
    backends: Arc<Vec<String>>,
    next: Arc<AtomicUsize>,
}

async fn proxy(State(state): State<AppState>, mut req: Request) -> Result<Response, StatusCode> {
    let i = state.next.fetch_add(1, Ordering::Relaxed) % state.backends.len();
    let backend = &state.backends[i];

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    let uri = format!("http://{}{}", backend, path_and_quey);
    *req.uri_mut() = Uri::try_from(uri).map_err(|_| StatusCode::BAD_REQUEST)?;

    let mut response = state
        .client
        .request(req)
        .await
        .map_err(|_|StatusCode::BAD_GATEWAY)?
        .into_response();

    response
        
        .header_mut()
        .insert("x-backend", backend.parse().unwrap());

    Ok(response)
}

#[tokio::main]

async fn main() {
    let client: Client = 
        hyper_util::client::legacy::Client::<(),  ()>::builder(TokioExecutor::new())
            .build(HttpConnector::new());

    let app = Router::new().fallback(proxy).with_state(client);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    println!("Proxy listening on 127.0.0.1:3000 -> {}", BACKEND);
    axum::serve(listener, app).await.unwrap();
}
