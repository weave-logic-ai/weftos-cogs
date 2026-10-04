//! The bridge's HTTP server: bounded, deadline-limited request parsing, per-IP throttling, and
//! the routes. Hardened against pre-auth resource exhaustion (COG-011 amendment 2): a connection
//! cap, capped header lines and counts, one overall deadline per request, no panics on socket
//! errors (the cog builds with `panic = "abort"`).

use crate::auth::{self, AuthReq, Refusal};
use crate::net::{read_capped_line, write_all_deadline, DeadlineStream, LogGate, Throttle};
use crate::{apply_reading, guide, parse_reading, unix_ms, Shared, TAG};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::io::{BufReader, Read};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Simultaneous connections served; the rest get an immediate 503.
pub const MAX_CONNS: usize = 32;
const MAX_LINE: usize = 8 * 1024;
const MAX_HEADERS: usize = 64;
const MAX_BODY: usize = 64 * 1024;
/// One budget for the whole request (line, headers, body), however slowly the peer sends.
pub const REQUEST_DEADLINE: Duration = Duration::from_secs(10);
/// The request line and headers must arrive within this, well inside the whole-request budget.
pub const HEADER_DEADLINE: Duration = Duration::from_secs(3);
/// The whole reply (the 44 KB guide included) must be written within this.
const WRITE_DEADLINE: Duration = Duration::from_secs(5);
/// Concurrent connections per source IP. One legitimate node needs one (two with a retry
/// overlapping a slow reply), so 4 leaves headroom while stopping one host from taking the 32
/// slots. Loopback (a local terminator or proxy fronting many clients) gets 16.
pub const IP_CONN_CAP: usize = 4;
pub const LOOPBACK_IP_CAP: usize = 16;
/// Per source IP (non-loopback): sustained requests per second, and burst.
const IP_RATE: f64 = 20.0;
const IP_BURST: f64 = 40.0;
const IP_TABLE: usize = 1024;

pub type StoreFn = Arc<dyn Fn(u32, &[f64]) -> Result<(), String> + Send + Sync>;

/// Everything a connection handler needs.
#[derive(Clone)]
pub struct Ctx {
    pub shared: Shared,
    pub auth: Arc<Mutex<auth::Auth>>,
    pub store: StoreFn,
    pub throttle: Arc<Mutex<Throttle<IpAddr>>>,
    pub conns: Arc<AtomicUsize>,
    /// When true, loopback callers get no special read privilege (use behind a local TLS
    /// terminator, where every client looks like loopback).
    pub status_auth: bool,
    pub deadline: Duration,
    pub header_deadline: Duration,
    pub per_ip: Arc<Mutex<HashMap<IpAddr, usize>>>,
    pub logs: Arc<Mutex<LogGate>>,
    /// Loopback skips the per-IP throttle and gets the larger connection cap. Tests turn this
    /// off to exercise the per-IP logic over loopback.
    pub loopback_exempt: bool,
}

impl Ctx {
    pub fn new(shared: Shared, auth: auth::Auth, store: StoreFn, status_auth: bool) -> Self {
        Ctx {
            shared,
            auth: Arc::new(Mutex::new(auth)),
            store,
            throttle: Arc::new(Mutex::new(Throttle::new(IP_TABLE, IP_RATE, IP_BURST))),
            conns: Arc::new(AtomicUsize::new(0)),
            status_auth,
            deadline: REQUEST_DEADLINE,
            header_deadline: HEADER_DEADLINE,
            per_ip: Arc::default(),
            logs: Arc::default(),
            loopback_exempt: true,
        }
    }
}

/// True once `resp` holds a full HTTP response (headers plus Content-Length bytes). The Seed
/// agent keeps its socket open after replying, so reads stop here instead of waiting for EOF.
pub fn response_complete(resp: &[u8]) -> bool {
    let Some(end) = resp.windows(4).position(|w| w == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&resp[..end]).to_ascii_lowercase();
    let len = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    resp.len() >= end + 4 + len
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// Writes a reply. CORS (`Access-Control-Allow-Origin: *`) is only ever sent on read-only GETs,
/// never on `POST /ingest`, so a web page in someone's browser cannot drive the ingest.
fn reply(conn: &mut TcpStream, code: u16, ctype: &str, body: &str, cors: bool) {
    reply_within(conn, code, ctype, body, cors, WRITE_DEADLINE);
}

fn reply_within(
    conn: &mut TcpStream,
    code: u16,
    ctype: &str,
    body: &str,
    cors: bool,
    deadline: Duration,
) {
    let cors = if cors {
        "Access-Control-Allow-Origin: *\r\n"
    } else {
        ""
    };
    let msg = format!(
        "HTTP/1.0 {code} {}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n{cors}Connection: close\r\n\r\n{body}",
        reason(code),
        body.len()
    );
    let _ = write_all_deadline(conn, msg.as_bytes(), deadline);
}

fn refuse(conn: &mut TcpStream, ctx: &Ctx, r: &Refusal) {
    if let Ok(mut st) = ctx.shared.lock() {
        st.auth_refused += 1;
    }
    log_limited(
        ctx,
        r.code(),
        &format!("refused: {} ({})", r.code(), r.message()),
    );
    let body = serde_json::json!({"ok": false, "error": r.code(), "detail": r.message()});
    reply(
        conn,
        r.status(),
        "application/json",
        &body.to_string(),
        false,
    );
}

/// One log line per key per second, with a count of what was swallowed.
fn log_limited(ctx: &Ctx, key: &'static str, line: &str) {
    let gate = ctx
        .logs
        .lock()
        .ok()
        .and_then(|mut g| g.check(key, unix_ms()));
    match gate {
        Some(0) => eprintln!("{TAG} {line}"),
        Some(n) => eprintln!("{TAG} {line} [+{n} suppressed]"),
        None => {}
    }
}

/// A 429 from the per-IP request throttle. Counted apart from authentication refusals.
fn throttled(conn: &mut TcpStream, ctx: &Ctx) {
    if let Ok(mut st) = ctx.shared.lock() {
        st.throttled += 1;
    }
    log_limited(
        ctx,
        "throttled",
        "throttled a source IP (per-IP request rate)",
    );
    let body = serde_json::json!({"ok": false, "error": "rate_limited", "detail": Refusal::RateLimited.message()});
    reply(conn, 429, "application/json", &body.to_string(), false);
}

/// Releases the global slot and the per-IP slot when a handler ends, however it ends.
struct ConnGuard {
    conns: Arc<AtomicUsize>,
    per_ip: Arc<Mutex<HashMap<IpAddr, usize>>>,
    ip: IpAddr,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.conns.fetch_sub(1, Ordering::SeqCst);
        if let Ok(mut m) = self.per_ip.lock() {
            if let Some(n) = m.get_mut(&self.ip) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    m.remove(&self.ip);
                }
            }
        }
    }
}

/// Takes a per-IP slot if the IP is under its cap.
fn take_ip_slot(ctx: &Ctx, ip: IpAddr) -> bool {
    let cap = if ip.is_loopback() && ctx.loopback_exempt {
        LOOPBACK_IP_CAP
    } else {
        IP_CONN_CAP
    };
    match ctx.per_ip.lock() {
        Ok(mut m) => {
            let n = m.entry(ip).or_insert(0);
            if *n >= cap {
                false
            } else {
                *n += 1;
                true
            }
        }
        Err(_) => false,
    }
}

struct Request {
    method: String,
    path: String,
    content_length: Option<usize>,
    bad_length: bool,
    content_type: String,
    hdr: BTreeMap<String, String>,
}

enum ReadErr {
    TooLarge,
    Gone,
}

fn read_request(rd: &mut impl std::io::BufRead) -> Result<Request, ReadErr> {
    let line = |rd: &mut dyn std::io::BufRead| match read_capped_line(&mut *rd, MAX_LINE) {
        Ok(Some(l)) => Ok(l),
        Ok(None) => Err(ReadErr::Gone),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Err(ReadErr::TooLarge),
        Err(_) => Err(ReadErr::Gone),
    };
    let first = line(rd)?;
    let mut parts = first.split_whitespace();
    let (method, path) = (
        parts.next().unwrap_or("").to_string(),
        parts.next().unwrap_or("/").to_string(),
    );
    let mut req = Request {
        method,
        path,
        content_length: None,
        bad_length: false,
        content_type: String::new(),
        hdr: BTreeMap::new(),
    };
    for _ in 0..=MAX_HEADERS {
        let h = line(rd)?;
        if h == "\r\n" || h == "\n" {
            return Ok(req);
        }
        if let Some((k, v)) = h.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
            match k.as_str() {
                "content-length" => match v.parse::<usize>() {
                    // A repeated header with a different value is request smuggling, not a hint.
                    Ok(n) if req.content_length.is_none_or(|p| p == n) => {
                        req.content_length = Some(n)
                    }
                    _ => req.bad_length = true,
                },
                "content-type" => req.content_type = v.to_ascii_lowercase(),
                k if k.starts_with("x-bridge-") => {
                    req.hdr.insert(k.to_string(), v);
                }
                _ => {}
            }
        }
    }
    Err(ReadErr::TooLarge)
}

/// Runs one request, then closes politely: half-close and drain a little unread input so the
/// peer receives our reply instead of a reset (closing with unread data aborts the reply).
fn handle(mut conn: TcpStream, ctx: &Ctx) {
    handle_request(&mut conn, ctx);
    let _ = conn.shutdown(std::net::Shutdown::Write);
    let until = Instant::now() + Duration::from_millis(300);
    let mut sink = [0u8; 4096];
    let mut drained = 0usize;
    while drained < 64 * 1024 {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() || conn.set_read_timeout(Some(left)).is_err() {
            break;
        }
        match conn.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(n) => drained += n,
        }
    }
}

fn handle_request(conn: &mut TcpStream, ctx: &Ctx) {
    let started = Instant::now();
    let ip = conn.peer_addr().map(|a| a.ip()).ok();
    let loopback = ip.is_some_and(|i| i.is_loopback());
    if !loopback || !ctx.loopback_exempt {
        let ok = match (ip, ctx.throttle.lock()) {
            (Some(ip), Ok(mut t)) => t.allow(&ip, unix_ms()),
            _ => false,
        };
        if !ok {
            return throttled(conn, ctx);
        }
    }
    let Ok(clone) = conn.try_clone() else { return };
    // Headers get a short budget; the body then gets the rest of the whole-request budget.
    let header_end = started + ctx.header_deadline.min(ctx.deadline);
    let mut rd = BufReader::new(DeadlineStream::new(clone, header_end));
    let parsed = read_request(&mut rd);
    rd.get_mut().set_deadline(started + ctx.deadline);
    let req = match parsed {
        Ok(r) => r,
        Err(ReadErr::TooLarge) => {
            return reply(conn, 431, "text/plain", "headers too large\n", false)
        }
        Err(ReadErr::Gone) => return,
    };
    let route = req.path.split('?').next().unwrap_or("").to_string();
    if req.method == "GET" {
        return handle_get(conn, ctx, &req, &route, loopback && !ctx.status_auth);
    }
    if req.method != "POST" || route != "/ingest" {
        return reply(conn, 405, "text/plain", "POST /ingest only\n", false);
    }
    if !req.content_type.contains("application/json") {
        return reply(
            conn,
            415,
            "text/plain",
            "Content-Type must be application/json\n",
            false,
        );
    }
    if req.bad_length {
        return reply(conn, 400, "text/plain", "bad Content-Length\n", false);
    }
    let len = req.content_length.unwrap_or(0);
    if len > MAX_BODY {
        return reply(conn, 413, "text/plain", "too large\n", false);
    }
    let mut body = vec![0u8; len.min(MAX_BODY)];
    if rd.read_exact(&mut body).is_err() {
        return reply(conn, 400, "text/plain", "short body\n", false);
    }
    let node = match authorize(ctx, &req, "POST", &body) {
        Ok(n) => n,
        Err(r) => return refuse(conn, ctx, &r),
    };
    let parsed = serde_json::from_slice::<Value>(&body)
        .map_err(|e| e.to_string())
        .and_then(|v| parse_reading(&v));
    let r = match parsed {
        Ok(r) => r,
        Err(e) => {
            let body = serde_json::json!({"ok": false, "error": e});
            return reply(conn, 400, "application/json", &body.to_string(), false);
        }
    };
    // A verified node may only write its own source. (A legacy shared-token or open request has
    // no identity to bind, which is one more reason to retire it.)
    if let Some(n) = node.as_deref().filter(|n| *n != r.source) {
        let bad = Refusal::SourceMismatch {
            node: n.to_string(),
            source: r.source.clone(),
        };
        return refuse(conn, ctx, &bad);
    }
    let out = match apply_reading(&ctx.shared, r, |id, v| (ctx.store)(id, v)) {
        Ok(msg) => serde_json::json!({"ok": true, "stored": msg}),
        Err(e) => serde_json::json!({"ok": false, "error": e}),
    };
    reply(conn, 200, "application/json", &out.to_string(), false);
}

fn authorize(
    ctx: &Ctx,
    req: &Request,
    method: &str,
    body: &[u8],
) -> Result<Option<String>, Refusal> {
    let h = |k: &str| req.hdr.get(k).map(String::as_str);
    let areq = AuthReq {
        method,
        path: &req.path,
        body,
        node: h(auth::H_NODE),
        ts: h(auth::H_TS),
        nonce: h(auth::H_NONCE),
        sig: h(auth::H_SIG),
        token: h(auth::H_TOKEN),
    };
    auth::authorize_shared(&ctx.auth, &areq, unix_ms())
}

/// Read-only routes. `/status` and `/sources` show node names, vectors and metrics only to a
/// privileged caller (loopback, or a validly signed GET); everyone else gets counts.
fn handle_get(conn: &mut TcpStream, ctx: &Ctx, req: &Request, route: &str, loopback: bool) {
    let signed_get = req.hdr.contains_key(auth::H_NODE);
    let privileged = if loopback {
        true
    } else if signed_get && matches!(route, "/status" | "/sources") {
        match authorize(ctx, req, "GET", b"") {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(r) => return refuse(conn, ctx, &r),
        }
    } else {
        false
    };
    let (code, ctype, body) = match route {
        "/" => (
            200,
            "text/plain",
            "bridge cog\n  POST /ingest\n  GET /status\n  GET /sources\n  GET /guide\n".to_string(),
        ),
        "/status" => {
            let mut v = ctx
                .shared
                .lock()
                .map(|s| s.snapshot(privileged))
                .unwrap_or_default();
            if privileged {
                let info = ctx
                    .auth
                    .lock()
                    .map(|a| a.info(unix_ms()))
                    .unwrap_or_default();
                v["allowlist"] = info;
            }
            (200, "application/json", v.to_string())
        }
        "/sources" => {
            let body = ctx.shared.lock().map(|s| {
                if privileged {
                    serde_json::json!(s
                        .sources
                        .iter()
                        .map(|(k, v)| (k.clone(), v.store_id))
                        .collect::<BTreeMap<_, _>>())
                } else {
                    serde_json::json!({"sources": s.sources.len()})
                }
            });
            (
                200,
                "application/json",
                body.map(|b| b.to_string()).unwrap_or_else(|_| "{}".into()),
            )
        }
        "/guide" => (200, "application/json", guide::bundle_json().to_string()),
        _ => (404, "text/plain", "not found\n".to_string()),
    };
    reply(conn, code, ctype, &body, true);
}

pub fn serve(bind: &str, ctx: Ctx) -> Result<String, String> {
    let listener = TcpListener::bind(bind).map_err(|e| format!("bind {bind}: {e}"))?;
    let addr = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| bind.to_string());
    std::thread::spawn(move || {
        for mut conn in listener.incoming().flatten() {
            let ip = conn.peer_addr().map(|a| a.ip()).ok();
            let admitted = match ip {
                Some(ip) if ctx.conns.fetch_add(1, Ordering::SeqCst) < MAX_CONNS => {
                    if take_ip_slot(&ctx, ip) {
                        Some(ip)
                    } else {
                        ctx.conns.fetch_sub(1, Ordering::SeqCst);
                        None
                    }
                }
                Some(_) => {
                    ctx.conns.fetch_sub(1, Ordering::SeqCst);
                    None
                }
                None => None,
            };
            let Some(ip) = admitted else {
                if let Ok(mut st) = ctx.shared.lock() {
                    st.busy_refused += 1;
                }
                log_limited(
                    &ctx,
                    "busy",
                    "connection refused: over the connection cap (global or per IP)",
                );
                // Short deadline: this runs on the accept thread.
                reply_within(
                    &mut conn,
                    503,
                    "text/plain",
                    "busy\n",
                    false,
                    Duration::from_millis(200),
                );
                // Best effort: discard what has already arrived so the close is not a reset.
                let _ = conn.shutdown(std::net::Shutdown::Write);
                if conn.set_nonblocking(true).is_ok() {
                    let _ = conn.read(&mut [0u8; 4096]);
                }
                continue;
            };
            let guard = ConnGuard {
                conns: ctx.conns.clone(),
                per_ip: ctx.per_ip.clone(),
                ip,
            };
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new()
                .stack_size(256 * 1024)
                .spawn(move || {
                    let _guard = guard;
                    handle(conn, &ctx);
                });
            if let Err(e) = spawned {
                eprintln!("{TAG} could not start a handler thread: {e}");
            }
        }
    });
    Ok(addr)
}
