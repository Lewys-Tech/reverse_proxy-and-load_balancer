# rustdb-lb ⚖️

A small HTTP reverse proxy and load balancer written in Rust with [Tokio](https://tokio.rs), [Axum](https://docs.rs/axum) and [hyper-util](https://docs.rs/hyper-util). It spreads requests across several backends, stops sending traffic to backends that are down, and retries safely when a connection can't be made.

It sits in front of [`rustdb-api-pg`](../rustdb-api-pg), the PostgreSQL-backed key-value API from the same series.

---

## Demo

Two copies of the API run on ports 8081 and 8082. The proxy listens on 3000:

```bash
curl -s -X POST http://127.0.0.1:3000/set -d "city=nairobi"
for i in 1 2 3 4; do curl -si http://127.0.0.1:3000/get/city | grep -i x-backend; done
```

```
x-backend: 127.0.0.1:8081
x-backend: 127.0.0.1:8082
x-backend: 127.0.0.1:8081
x-backend: 127.0.0.1:8082
```

Stop one backend and the proxy notices within about two seconds:

```
backend 127.0.0.1:8082 is now DOWN
```

From then on every response comes from `8081`. Stop both and the proxy answers `503 Service Unavailable`. Start one again and it prints `is now UP` and resumes sending traffic to it.

---

## Features

- **Forwards any method and path**: the proxy is a single fallback route, so it needs no knowledge of the API behind it
- **Round-robin balancing** with a lock-free atomic counter
- **Active health checks**: a background task probes every backend every 2 seconds
- **Passive health marking**: a request that can't connect marks that backend down immediately
- **Safe retry**: on a connection failure only, the request is retried once on another backend
- **Honest error codes**: `502` when a backend fails, `503` when none are available, `413` for oversized bodies
- **`x-backend` response header** showing which backend answered, for debugging

---

## Getting Started

### Prerequisites

- Rust (developed on 1.93)
- Two or more running backends. The examples use [`rustdb-api-pg`](../rustdb-api-pg), which reads its port from the `PORT` environment variable.

### Run

```bash
# backends, one terminal each
cd rustdb-api-pg
export DATABASE_URL=postgres://USER:PASSWORD@localhost/rustdb
PORT=8081 cargo run
PORT=8082 cargo run

# the proxy
cd rustdb-lb
cargo run
```

The proxy listens on `127.0.0.1:3000`. Backend addresses are currently set in `main()`.

---

## How It Works

### Request flow

```
client ──► proxy :3000 ──► pick a healthy backend ──► backend :8081 / :8082
                │                    ▲
                │            health flags (AtomicBool per backend)
                │                    ▲
                └── background task probes each backend every 2s
```

1. The proxy reads the whole request body into memory (up to 1 MB).
2. `pick_backend` advances a shared counter and skips any backend marked unhealthy.
3. The request is rewritten to point at that backend and sent.
4. The response is returned with an `x-backend` header added.

### Round-robin without a lock

```rust
let i = state.next.fetch_add(1, Ordering::Relaxed) % n;
```

`fetch_add` returns the old value and increments in one indivisible step, so two simultaneous requests can never receive the same number. A `Mutex` would also work, but there's no critical section here beyond one integer, so an atomic is simpler and can't be poisoned by a panic.

### Two kinds of health checking

- **Active:** a `tokio::spawn`ed task connects to each backend every 2 seconds with a 1-second timeout and updates an `AtomicBool`. It logs only when the state changes.
- **Passive:** if a real request gets *connection refused*, the proxy marks that backend down at once instead of waiting for the next probe, then retries.

### Retry, and why only on connection failure

```rust
Err(e) if e.is_connect() => { /* mark down, try the next backend */ }
Err(_) => return Err(StatusCode::BAD_GATEWAY),
```

A refused connection means no bytes of the request reached the backend, so it cannot have acted on anything and retrying is safe, even for `POST`. If the connection drops after the request was sent, the backend may already have written to the database, and a retry could repeat the action. For that case the proxy returns `502` and does not retry. At most one retry is attempted.

### Why the backends must be stateless

Balancing only works if any backend can answer any request. `rustdb-api-pg` keeps all state in one shared PostgreSQL database, so that holds. The earlier `rustdb-api-async` keeps a private `HashMap` and log file per process, so a write on one instance would be invisible to the others.

### 502 vs 503

- `502 Bad Gateway`: the proxy tried a backend and it failed
- `503 Service Unavailable`: the proxy has no healthy backend to try

---

## Testing Status

Verified manually:

- ✅ Requests are distributed round-robin across two backends
- ✅ A stopped backend is detected and removed from rotation within about 2 seconds
- ✅ A restarted backend is detected and returned to rotation
- ✅ All backends down returns `503`
- ✅ `GET`, `POST` and `DELETE` pass through the proxy with correct status codes

Implemented but not yet demonstrated:

- ⚠️ **Passive marking and retry on connection refused.** The code is in place, but in manual runs the active health checker always detected a failed backend before a request reached it, so the retry path was never exercised. A reliable test needs the health checker disabled and a backend address that nothing is listening on.

---

## Known Limitations

- **Backends are hardcoded** in `main()`. A config file or CLI arguments would be better.
- **The health check only tests that a TCP connection opens.** A backend that is running but can't reach its database still passes. A `/health` endpoint that checks the database would be more accurate.
- **Request bodies are buffered** (max 1 MB) to allow retry. Large uploads and streaming are not supported.
- **Headers are forwarded unchanged.** `Host` is not rewritten, hop-by-hop headers are not stripped, and `X-Forwarded-For` is not added.
- **No timeout on backend requests.** A backend that accepts a connection and then hangs will hang the request.
- **No TLS**, authentication, rate limiting or metrics.
- **Only round-robin.** No weighting or least-connections.
- **Race window.** A backend can die up to 2 seconds before the active check notices. The passive path is designed to cover this, per the testing note above.

---

## Ideas for Extending

- [ ] Automated test with in-process fake backends, including one that refuses connections
- [ ] Load backends from a config file
- [ ] `/health` endpoint on the API that checks the database
- [ ] Request timeouts with `tokio::time::timeout`
- [ ] Add `X-Forwarded-For` and strip hop-by-hop headers
- [ ] Least-connections or weighted balancing
- [ ] Structured logging with `tracing`
- [ ] Graceful shutdown

---

## Resources

- [Tokio tutorial](https://tokio.rs/tokio/tutorial)
- [Axum reverse-proxy example](https://github.com/tokio-rs/axum/tree/main/examples/reverse-proxy)
- [`hyper-util` client](https://docs.rs/hyper-util/latest/hyper_util/client/legacy/index.html)
- [MDN: Idempotent methods](https://developer.mozilla.org/en-US/docs/Glossary/Idempotent)
- [nginx `proxy_next_upstream`](https://nginx.org/en/docs/http/ngx_http_proxy_module.html#proxy_next_upstream)
- [Cloudflare Pingora](https://github.com/cloudflare/pingora), a production Rust proxy

---

## License

MIT
