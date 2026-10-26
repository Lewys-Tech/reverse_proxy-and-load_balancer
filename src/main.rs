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

#[deriv(Clone)]
struct AppState {
    client: Client,
    backends: Arc<Vec<String>>,
    next: Arc<AtomicUsize>,
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
