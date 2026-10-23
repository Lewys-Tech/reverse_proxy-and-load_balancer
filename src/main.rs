use axum::{
    body::Body,
    extract::{Request, State},
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
    Router,
};

use hyper_util::{client::legacy::connect::HttpConnector, rt::TokioExecutor};

type Client = hyper_util::client::legacy::Client<HttpConnector, Body>;

const BACKEND: &str = "127.0.0.1:8080";


async fn proxy(State(client):  State<Client>, mut req: Request) -> Result<Response, StatusCode> {
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    let uri = format!("http://{}{}", BACKEND, path_and_query);
    *req.uri_mut() = Uri::try_from(uri).map_err(|_| StatusCode::BAD_REQUEST)?;


    let response = client
        .request(req)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    Ok(response.into_response())


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
