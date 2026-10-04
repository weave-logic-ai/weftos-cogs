//! End-to-end tests over real sockets: signed ingest, refusals, hardening limits, the relay
//! against a stub Seed and a stub receiver. (COG-011)

use crate::server::{serve, Ctx, IP_CONN_CAP, MAX_CONNS};
use crate::{auth, relay, unix_ms, Shared, State};
use ed25519_dalek::SigningKey;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const NODE: &str = "seed-a";

type Captured = Arc<Mutex<Vec<(u32, Vec<f64>)>>>;

fn tmp_allowlist(tag: &str, nodes: &[(&str, &SigningKey)]) -> String {
    let dir = std::env::temp_dir().join(format!("bridge-e2e-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("nodes.allow");
    let txt: String = nodes
        .iter()
        .map(|(n, k)| format!("{n} {}\n", auth::to_hex(k.verifying_key().as_bytes())))
        .collect();
    std::fs::write(&p, txt).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    p.to_str().unwrap().to_string()
}

fn new_auth(list: &str) -> auth::Auth {
    auth::Auth::new(Some(list), 60, "tok", false, unix_ms()).unwrap()
}

struct Rx {
    addr: String,
    shared: Shared,
    got: Captured,
    conns: Arc<std::sync::atomic::AtomicUsize>,
}

fn receiver_with(auth: auth::Auth, tweak: impl FnOnce(&mut Ctx)) -> Rx {
    let shared: Shared = Arc::new(Mutex::new(State::new(30, 64)));
    let got: Captured = Arc::default();
    let g = got.clone();
    let mut ctx = Ctx::new(
        shared.clone(),
        auth,
        Arc::new(move |id, v| {
            g.lock().unwrap().push((id, v.to_vec()));
            Ok(())
        }),
        false,
    );
    tweak(&mut ctx);
    let conns = ctx.conns.clone();
    Rx {
        addr: serve("127.0.0.1:0", ctx).unwrap(),
        shared,
        got,
        conns,
    }
}

fn receiver(auth: auth::Auth) -> Rx {
    receiver_with(auth, |_| {})
}

fn reading(source: &str, cog: &str) -> Value {
    serde_json::json!({"source": source, "cog": cog, "ts_ms": unix_ms(),
        "vector": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8], "metrics": {}})
}

/// Sends a raw request and returns (status, full response text).
fn raw_req(addr: &str, method: &str, path: &str, headers: &[String], body: &[u8]) -> (u16, String) {
    let mut c = TcpStream::connect(addr).unwrap();
    let mut req = format!(
        "{method} {path} HTTP/1.0\r\nHost: x\r\nContent-Length: {}\r\n",
        body.len()
    );
    for h in headers {
        req.push_str(h);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    c.write_all(req.as_bytes()).unwrap();
    c.write_all(body).unwrap();
    let mut resp = String::new();
    let _ = c.read_to_string(&mut resp);
    let code = resp
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    (code, resp)
}

fn post(addr: &str, extra: &[String], body: &[u8]) -> (u16, String) {
    let mut h = vec!["Content-Type: application/json".to_string()];
    h.extend_from_slice(extra);
    raw_req(addr, "POST", "/ingest", &h, body)
}

fn signed_post(
    addr: &str,
    k: &SigningKey,
    node: &str,
    body: &[u8],
    ts: u64,
    nonce: &str,
) -> (u16, String) {
    post(
        addr,
        &auth::sign_headers(k, node, "POST", "/ingest", body, ts, nonce),
        body,
    )
}

fn n(i: u32) -> String {
    format!("{i:032x}")
}

#[test]
fn http_signed_ingest_accepts_valid_and_refuses_the_rest() {
    let (k, other) = (
        SigningKey::from_bytes(&[7; 32]),
        SigningKey::from_bytes(&[8; 32]),
    );
    let list = tmp_allowlist("http", &[(NODE, &k)]);
    let rx = receiver(new_auth(&list));
    let body = serde_json::to_vec(&reading(NODE, "seed-stream")).unwrap();
    let now = unix_ms();

    let (c, b) = signed_post(&rx.addr, &k, NODE, &body, now, &n(1));
    assert_eq!(c, 200, "{b}");
    assert!(b.contains("\"ok\":true"), "{b}");
    assert!(
        !b.to_lowercase().contains("access-control-allow-origin"),
        "no CORS on POST"
    );
    assert_eq!(rx.got.lock().unwrap().len(), 1);

    let refused = |(c, b): (u16, String), code: u16, what: &str| {
        assert_eq!((c, b.contains(what)), (code, true), "{b}");
    };
    refused(
        signed_post(&rx.addr, &other, "intruder", &body, now, &n(2)),
        401,
        "unknown_node",
    );
    refused(
        signed_post(&rx.addr, &other, NODE, &body, now, &n(3)),
        401,
        "bad_signature",
    );
    refused(
        signed_post(&rx.addr, &k, NODE, &body, now - 120_000, &n(4)),
        401,
        "stale_timestamp",
    );
    assert_eq!(signed_post(&rx.addr, &k, NODE, &body, now, &n(5)).0, 200);
    refused(
        signed_post(&rx.addr, &k, NODE, &body, now, &n(5)),
        401,
        "replayed_nonce",
    );
    let spoof = serde_json::to_vec(&reading("someone-else", "x")).unwrap();
    refused(
        signed_post(&rx.addr, &k, NODE, &spoof, now, &n(6)),
        403,
        "source_mismatch",
    );
    assert_eq!(post(&rx.addr, &[], &body).0, 401);
    assert_eq!(
        post(&rx.addr, &["X-Bridge-Token: tok".into()], &body).0,
        401
    );

    let st = rx.shared.lock().unwrap();
    assert_eq!(st.snapshot(true)["readings"], 2);
    assert_eq!(st.auth_refused, 7);
    assert_eq!(
        rx.got.lock().unwrap().len(),
        2,
        "refused requests must not reach the store"
    );
}

#[test]
fn a_request_signed_before_a_restart_is_not_replayable_after_it() {
    let k = SigningKey::from_bytes(&[7; 32]);
    let list = tmp_allowlist("restart", &[(NODE, &k)]);
    let body = serde_json::to_vec(&reading(NODE, "seed-stream")).unwrap();
    let captured = auth::sign_headers(&k, NODE, "POST", "/ingest", &body, unix_ms(), &n(1));
    std::thread::sleep(Duration::from_millis(30));
    let rx = receiver(new_auth(&list)); // "restarted": empty nonce cache, later start time
    let (c, b) = post(&rx.addr, &captured, &body);
    assert_eq!((c, b.contains("predates_restart")), (401, true), "{b}");
    // The relay retries this one (re-signing each attempt) instead of dropping the reading.
    let json = b.split("\r\n\r\n").nth(1).unwrap_or("");
    assert_eq!(relay::classify(c, json), relay::Class::Transient);
}

#[test]
fn ingest_needs_json_content_type_and_get_keeps_cors() {
    let rx = receiver(auth::Auth::new(None, 60, "", false, unix_ms()).unwrap());
    let body = serde_json::to_vec(&reading("pi5", "tof")).unwrap();
    let (c, _) = raw_req(
        &rx.addr,
        "POST",
        "/ingest",
        &["Content-Type: text/plain".into()],
        &body,
    );
    assert_eq!(
        c, 415,
        "a no-preflight cross-site form POST must not be accepted"
    );
    let (c, _) = raw_req(&rx.addr, "POST", "/ingest", &[], &body);
    assert_eq!(c, 415);
    let (c, b) = raw_req(&rx.addr, "OPTIONS", "/ingest", &[], b"");
    assert_eq!(c, 405);
    assert!(
        !b.to_lowercase().contains("access-control"),
        "no CORS on preflight"
    );
    let (c, b) = raw_req(&rx.addr, "GET", "/status", &[], b"");
    assert_eq!(c, 200);
    assert!(b.to_lowercase().contains("access-control-allow-origin"));
}

#[test]
fn deprecated_token_works_only_behind_the_flag_and_is_case_exact() {
    let body = serde_json::to_vec(&reading("pi5", "tof")).unwrap();
    let off = receiver(auth::Auth::new(None, 60, "tok", false, unix_ms()).unwrap());
    assert_eq!(
        post(&off.addr, &["X-Bridge-Token: tok".into()], &body).0,
        401
    );
    let on = receiver(auth::Auth::new(None, 60, "Tok-Mixed", true, unix_ms()).unwrap());
    assert_eq!(
        post(&on.addr, &["X-Bridge-Token: Tok-Mixed".into()], &body).0,
        200
    );
    assert_eq!(
        post(&on.addr, &["X-Bridge-Token: tok-mixed".into()], &body).0,
        401
    );
}

#[test]
fn oversized_and_excessive_headers_are_refused() {
    let rx = receiver(auth::Auth::new(None, 60, "", false, unix_ms()).unwrap());
    let long = format!("X-Junk: {}", "a".repeat(20_000));
    let (c, _) = raw_req(&rx.addr, "GET", "/status", &[long], b"");
    assert_eq!(c, 431);
    let many: Vec<String> = (0..200).map(|i| format!("X-H{i}: v")).collect();
    let (c, _) = raw_req(&rx.addr, "GET", "/status", &many, b"");
    assert_eq!(c, 431);
    // The server is still healthy afterwards.
    assert_eq!(raw_req(&rx.addr, "GET", "/status", &[], b"").0, 200);
}

#[test]
fn a_slow_sender_is_cut_off_at_the_request_deadline() {
    let rx = receiver_with(
        auth::Auth::new(None, 60, "", false, unix_ms()).unwrap(),
        |c| c.header_deadline = Duration::from_millis(600),
    );
    let mut c = TcpStream::connect(&rx.addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let start = std::time::Instant::now();
    // Drip a header a byte at a time, never finishing the request.
    for b in b"GET /status HTTP/1.0\r\nX-Slow: yyyyyyyyyyyyyyyyyyyyyyyyyyyyyy" {
        if c.write_all(&[*b]).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
        if start.elapsed() > Duration::from_secs(3) {
            break;
        }
    }
    let mut rest = Vec::new();
    let _ = c.read_to_end(&mut rest);
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "server held the connection {:?}",
        start.elapsed()
    );
    assert_eq!(rx.conns_in_flight_soon(), 0);
}

impl Rx {
    /// The in-flight connection count once handlers have had a moment to unwind.
    fn conns_in_flight_soon(&self) -> usize {
        std::thread::sleep(Duration::from_millis(300));
        self.conns.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[test]
fn connections_beyond_the_cap_get_503_and_slots_come_back() {
    let rx = receiver_with(
        auth::Auth::new(None, 60, "", false, unix_ms()).unwrap(),
        |c| c.deadline = Duration::from_millis(1500),
    );
    let idle: Vec<TcpStream> = (0..MAX_CONNS)
        .map(|_| TcpStream::connect(&rx.addr).unwrap())
        .collect();
    std::thread::sleep(Duration::from_millis(200));
    let (c, _) = raw_req(&rx.addr, "GET", "/status", &[], b"");
    // The 503 may be lost to a reset if the request outruns the close; either way it is refused.
    assert!(
        c == 503 || c == 0,
        "the {}th connection must be refused, got {c}",
        MAX_CONNS + 1
    );
    drop(idle);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        raw_req(&rx.addr, "GET", "/status", &[], b"").0,
        200,
        "capacity recovered"
    );
}

#[test]
fn per_source_cog_quota_and_name_charset_are_enforced() {
    let rx = receiver(auth::Auth::new(None, 60, "", false, unix_ms()).unwrap());
    for i in 0..8 {
        let b = serde_json::to_vec(&reading("pi5", &format!("cog{i}"))).unwrap();
        assert!(post(&rx.addr, &[], &b).1.contains("\"ok\":true"), "cog{i}");
    }
    let b = serde_json::to_vec(&reading("pi5", "cog9")).unwrap();
    let (c, resp) = post(&rx.addr, &[], &b);
    assert!(
        c == 200 && resp.contains("\"ok\":false") && resp.contains("already has 8 cogs"),
        "{resp}"
    );
    // Another source is unaffected, and an existing pair still updates.
    let b = serde_json::to_vec(&reading("opi", "cog0")).unwrap();
    assert!(post(&rx.addr, &[], &b).1.contains("\"ok\":true"));
    let b = serde_json::to_vec(&reading("pi5", "cog3")).unwrap();
    assert!(post(&rx.addr, &[], &b).1.contains("\"ok\":true"));
    // '/' (key collision) and other odd names are rejected outright.
    for (s, c) in [
        ("a/b", "c"),
        ("a", "b/c"),
        ("a b", "c"),
        ("a", "c\n"),
        ("", "c"),
    ] {
        let b = serde_json::to_vec(&reading(s, c)).unwrap();
        assert_eq!(post(&rx.addr, &[], &b).0, 400, "{s:?}/{c:?}");
    }
}

#[test]
fn status_is_summary_only_for_unprivileged_callers() {
    let k = SigningKey::from_bytes(&[7; 32]);
    let list = tmp_allowlist("status", &[(NODE, &k)]);
    // status_auth = true: even loopback is unprivileged (the local-terminator setup).
    let rx = receiver_with(new_auth(&list), |c| c.status_auth = true);
    let body = serde_json::to_vec(&reading(NODE, "seed-stream")).unwrap();
    assert_eq!(
        signed_post(&rx.addr, &k, NODE, &body, unix_ms(), &n(1)).0,
        200
    );

    let (_, b) = raw_req(&rx.addr, "GET", "/status", &[], b"");
    assert!(
        b.contains("\"readings\":1")
            && !b.contains("seed-a/seed-stream")
            && !b.contains("last_vector"),
        "{b}"
    );
    let (_, b) = raw_req(&rx.addr, "GET", "/sources", &[], b"");
    assert!(b.contains("\"sources\":1") && !b.contains("seed-a"), "{b}");

    let h = auth::sign_headers(&k, NODE, "GET", "/status", b"", unix_ms(), &n(2));
    let (c, b) = raw_req(&rx.addr, "GET", "/status", &h, b"");
    assert_eq!(c, 200);
    assert!(
        b.contains("seed-a/seed-stream") && b.contains("allowlist_keys"),
        "{b}"
    );
    // A bad signature on a GET is refused, not silently downgraded.
    let other = SigningKey::from_bytes(&[8; 32]);
    let h = auth::sign_headers(&other, NODE, "GET", "/status", b"", unix_ms(), &n(3));
    assert_eq!(raw_req(&rx.addr, "GET", "/status", &h, b"").0, 401);

    // Default (loopback privileged): full detail without signing.
    let rx2 = receiver(new_auth(&list));
    let (_, b) = raw_req(&rx2.addr, "GET", "/status", &[], b"");
    assert!(
        b.contains("allowlist_error") && b.contains("allowlist_age_s"),
        "{b}"
    );
}

/// A stub Seed that answers every GET with the given stream JSON.
fn stub_seed(json: &'static str) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for mut c in l.incoming().flatten() {
            let mut req = Vec::new();
            let mut buf = [0u8; 512];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                match c.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => req.extend_from_slice(&buf[..n]),
                }
            }
            let _ = write!(
                c,
                "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{json}"
            );
        }
    });
    addr
}

const STREAM: &str = r#"{"healthy":true,"sample_rate_hz":10,"samples":[
    {"channel":"ch0","value":1.0,"normalized":0.25},{"channel":"ch1","value":2.0,"normalized":0.5},
    {"channel":"ch2","value":3.0,"normalized":0.75}]}"#;

#[test]
fn relay_republishes_a_seed_reading_to_the_receiver_signed() {
    let key = SigningKey::from_bytes(&[9; 32]);
    let list = tmp_allowlist("relay", &[(NODE, &key)]);
    let rx = receiver(new_auth(&list));
    let seed = stub_seed(STREAM);

    let stream = relay::fetch_seed_stream(&seed).unwrap();
    let r = relay::reading_from_stream(&stream, NODE, "seed-stream", unix_ms()).unwrap();
    let mut q = relay::Relay::new(8);
    q.push(r);
    let to = format!("http://{}/ingest", rx.addr);
    let err = q.flush(
        unix_ms(),
        |r| relay::post_signed(&to, NODE, &key, r, unix_ms()),
        || {},
    );
    assert!(err.is_none(), "{err:?}");
    assert_eq!((q.sent, q.queued()), (1, 0));

    let snap = rx.shared.lock().unwrap().snapshot(true);
    assert_eq!(snap["detail"][0]["key"], "seed-a/seed-stream");
    assert_eq!(
        snap["detail"][0]["last_vector"].as_array().unwrap()[..3],
        [0.25, 0.5, 0.75]
    );
    assert_eq!(snap["detail"][0]["last_metrics"]["sample_count"], 3);
    assert_eq!(rx.got.lock().unwrap()[0].1[..3], [0.25, 0.5, 0.75]);
}

#[test]
fn relay_buffers_while_the_receiver_is_down_then_drains_in_order() {
    let key = SigningKey::from_bytes(&[9; 32]);
    let list = tmp_allowlist("relay-down", &[(NODE, &key)]);
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let to = format!("http://127.0.0.1:{port}/ingest");

    let mut q = relay::Relay::new(2);
    let mk = |i: u32| {
        let mut v = reading(NODE, "seed-stream");
        v["vector"] = serde_json::json!([i as f64 / 10.0]);
        v
    };
    for i in 1..=3 {
        q.push(mk(i));
    }
    assert_eq!((q.queued(), q.dropped), (2, 1));
    let t0 = unix_ms();
    assert!(q
        .flush(
            t0,
            |r| relay::post_signed(&to, NODE, &key, r, unix_ms()),
            || {}
        )
        .is_some());
    assert_eq!((q.backoff_ms(), q.queued()), (1000, 2));

    let order: Arc<Mutex<Vec<f64>>> = Arc::default();
    let o = order.clone();
    let ctx = Ctx::new(
        Arc::new(Mutex::new(State::new(30, 8))),
        new_auth(&list),
        Arc::new(move |_, v| {
            o.lock().unwrap().push(v[0]);
            Ok(())
        }),
        false,
    );
    serve(&format!("127.0.0.1:{port}"), ctx).unwrap();
    let mut paced = 0;
    let err = q.flush(
        q.next_try_ms() + 1,
        |r| relay::post_signed(&to, NODE, &key, r, unix_ms()),
        || paced += 1,
    );
    assert!(err.is_none(), "{err:?}");
    assert_eq!(*order.lock().unwrap(), [0.2, 0.3]);
    assert_eq!((q.backoff_ms(), paced), (0, 1));
}

#[test]
fn a_slow_body_is_cut_off_at_the_overall_deadline_after_fast_headers() {
    let rx = receiver_with(
        auth::Auth::new(None, 60, "", false, unix_ms()).unwrap(),
        |c| {
            c.header_deadline = Duration::from_millis(500);
            c.deadline = Duration::from_millis(1200);
        },
    );
    let mut c = TcpStream::connect(&rx.addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let start = std::time::Instant::now();
    c.write_all(
        b"POST /ingest HTTP/1.0\r\nContent-Type: application/json\r\nContent-Length: 500\r\n\r\n",
    )
    .unwrap();
    // Headers arrived inside the header budget; now drip the body past the overall budget.
    for _ in 0..40 {
        if c.write_all(b"x").is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        if start.elapsed() > Duration::from_secs(3) {
            break;
        }
    }
    let mut rest = Vec::new();
    let _ = c.read_to_end(&mut rest);
    assert!(
        start.elapsed() < Duration::from_millis(3500),
        "{:?}",
        start.elapsed()
    );
    assert!(
        String::from_utf8_lossy(&rest).contains("400"),
        "short body is a 400"
    );
}

#[test]
fn body_cap_and_content_length_are_enforced() {
    let rx = receiver(auth::Auth::new(None, 60, "", false, unix_ms()).unwrap());
    let ct = "Content-Type: application/json".to_string();
    let send = |len_hdr: &str, body: &[u8]| {
        let mut c = TcpStream::connect(&rx.addr).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let req = format!("POST /ingest HTTP/1.0\r\n{ct}\r\n{len_hdr}\r\n\r\n");
        c.write_all(req.as_bytes()).unwrap();
        let _ = c.write_all(body);
        let mut r = String::new();
        let _ = c.read_to_string(&mut r);
        r.split_whitespace()
            .nth(1)
            .and_then(|c| c.parse::<u16>().ok())
            .unwrap_or(0)
    };
    // Over the 64 KiB cap: refused from the header alone, before any allocation.
    assert_eq!(send("Content-Length: 65537", b""), 413);
    assert_eq!(send(&format!("Content-Length: {}", usize::MAX), b""), 413);
    // Unparseable or conflicting lengths are a 400, never a huge allocation.
    for bad in [
        "Content-Length: abc",
        "Content-Length: -1",
        "Content-Length: 99999999999999999999999",
        "Content-Length: 5\r\nContent-Length: 6",
    ] {
        assert_eq!(send(bad, b""), 400, "{bad}");
    }
    // A body exactly at the cap is accepted for reading (then fails as JSON, 400), not 413.
    let body = vec![b' '; 65536];
    assert_eq!(send("Content-Length: 65536", &body), 400);
}

#[test]
fn one_ip_cannot_hold_every_connection_slot() {
    // Loopback exemption off so the per-IP cap applies over loopback.
    let rx = receiver_with(
        auth::Auth::new(None, 60, "", false, unix_ms()).unwrap(),
        |c| {
            c.loopback_exempt = false;
            c.deadline = Duration::from_millis(1500);
            c.header_deadline = Duration::from_millis(1500);
        },
    );
    let idle: Vec<TcpStream> = (0..IP_CONN_CAP)
        .map(|_| TcpStream::connect(&rx.addr).unwrap())
        .collect();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        rx.conns.load(std::sync::atomic::Ordering::SeqCst),
        IP_CONN_CAP
    );
    let (c, _) = raw_req(&rx.addr, "GET", "/status", &[], b"");
    assert!(
        c == 503 || c == 0,
        "the {}th connection from one IP is refused, got {c}",
        IP_CONN_CAP + 1
    );
    assert!(
        rx.conns.load(std::sync::atomic::Ordering::SeqCst) <= IP_CONN_CAP,
        "global slots were not all taken"
    );
    assert!(rx.shared.lock().unwrap().busy_refused >= 1);
    drop(idle);
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        raw_req(&rx.addr, "GET", "/status", &[], b"").0,
        200,
        "slots come back"
    );
}

#[test]
fn per_ip_request_throttle_returns_429_counted_apart_from_auth_refusals() {
    let rx = receiver_with(
        auth::Auth::new(None, 60, "", false, unix_ms()).unwrap(),
        |c| c.loopback_exempt = false,
    );
    let codes: Vec<u16> = (0..80)
        .map(|_| raw_req(&rx.addr, "GET", "/status", &[], b"").0)
        .collect();
    let limited = codes.iter().filter(|c| **c == 429).count();
    assert!(
        codes.iter().take(30).all(|c| *c == 200),
        "burst is allowed: {codes:?}"
    );
    assert!(limited >= 10, "{limited} throttled of 80");
    let st = rx.shared.lock().unwrap();
    assert_eq!(st.throttled as usize, limited);
    assert_eq!(st.auth_refused, 0, "throttling is not an auth refusal");
}

#[test]
fn loopback_is_exempt_from_the_throttle_by_default() {
    let rx = receiver(auth::Auth::new(None, 60, "", false, unix_ms()).unwrap());
    assert!((0..80).all(|_| raw_req(&rx.addr, "GET", "/status", &[], b"").0 == 200));
    assert_eq!(rx.shared.lock().unwrap().throttled, 0);
}
