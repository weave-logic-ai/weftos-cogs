//! Phase 2 outbound mode (COG-011): republish the Seed's own sensor stream to a bridge
//! receiver (a WeftOS-side bridge, or any bridge-protocol endpoint), signed per node identity.
//!
//! Loop: fetch `GET <seed>/api/v1/sensor/stream` -> build one reading -> push into a bounded
//! buffer -> flush oldest-first with exponential backoff. The buffer never grows past `cap`
//! (oldest readings are dropped and counted), so a dead receiver cannot exhaust Seed memory.

use crate::auth;
use crate::net::{self, DeadlineStream};
use ed25519_dalek::SigningKey;
use serde_json::Value;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

const BACKOFF_MIN_MS: u64 = 1_000;
const BACKOFF_MAX_MS: u64 = 60_000;
/// Delay between sends while draining a backlog (about 5 requests a second), so recovery does
/// not hit the receiver's per-node rate limit or burst the Seed.
pub const DRAIN_PACE: Duration = Duration::from_millis(200);
/// Whole-request budget for a Seed read or a receiver round trip.
const IO_DEADLINE: Duration = Duration::from_secs(5);

pub struct RelayOpts {
    pub seed: String,
    pub to: String,
    pub node: String,
    pub cog: String,
    pub key: SigningKey,
    pub interval: u64,
    pub buffer: usize,
}

/// A send outcome. Transient errors keep the reading queued; permanent ones drop it.
pub struct SendErr {
    pub transient: bool,
    /// The refusal is about clock skew (`stale_timestamp`): retried, but only a bounded number
    /// of times, since a persistent skew will not fix itself by waiting.
    pub clock: bool,
    pub msg: String,
}

/// Consecutive clock-skew refusals tolerated before the reading is dropped.
pub const MAX_CLOCK_RETRIES: u32 = 10;

/// How a receiver's reply should be treated.
#[derive(Debug, PartialEq)]
pub enum Class {
    Stored,
    Transient,
    Clock,
    Permanent,
}

/// Classifies a reply by HTTP status and the bridge's JSON `error` code. A request signed
/// before the receiver restarted (`predates_restart`) is retried (it is re-signed on every
/// attempt); clock skew is retried a bounded number of times; other 4xx are permanent.
pub fn classify(code: u16, reply: &str) -> Class {
    let v: Option<Value> = serde_json::from_str(reply).ok();
    let err = v.as_ref().and_then(|v| v["error"].as_str()).unwrap_or("");
    match code {
        200 if v.as_ref().is_some_and(|v| v["ok"] == true) => Class::Stored,
        200 => Class::Transient,
        401 if err == "predates_restart" => Class::Transient,
        401 if err == "stale_timestamp" => Class::Clock,
        0 | 408 | 429 | 500..=599 => Class::Transient,
        _ => Class::Permanent,
    }
}

/// Turns the Seed's `{"samples":[{"channel","value","normalized",...}]}` into a bridge reading:
/// the first eight normalized channel values (clamped by the receiver) plus summary metrics.
pub fn reading_from_stream(
    stream: &Value,
    node: &str,
    cog: &str,
    ts_ms: u64,
) -> Result<Value, String> {
    let samples = stream["samples"]
        .as_array()
        .filter(|s| !s.is_empty())
        .ok_or("seed stream has no samples")?;
    let vector: Vec<f64> = samples
        .iter()
        .take(8)
        .map(|s| {
            s["normalized"]
                .as_f64()
                .or(s["value"].as_f64())
                .unwrap_or(0.0)
        })
        .collect();
    Ok(serde_json::json!({
        "source": node,
        "cog": cog,
        "ts_ms": ts_ms,
        "vector": vector,
        "metrics": {
            "healthy": stream["healthy"].as_bool().unwrap_or(false),
            "sample_count": samples.len(),
            "sample_rate_hz": stream["sample_rate_hz"].as_f64().unwrap_or(0.0),
        },
    }))
}

/// One bounded HTTP GET of the Seed's sensor stream; takes the first JSON object in the reply
/// (works for a plain JSON body or an SSE `data:` line).
pub fn fetch_seed_stream(target: &str) -> Result<Value, String> {
    let mut conn = net::connect(target, IO_DEADLINE)?;
    conn.set_write_timeout(Some(IO_DEADLINE)).ok();
    write!(
        conn,
        "GET /api/v1/sensor/stream HTTP/1.0\r\nHost: {target}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| format!("write: {e}"))?;
    let mut rd = DeadlineStream::new(
        conn.try_clone().map_err(|e| format!("clone: {e}"))?,
        Instant::now() + IO_DEADLINE,
    );
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match rd.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.len() > 262_144 || json_object_complete(&buf) {
                    break;
                }
            }
            Err(_) if !buf.is_empty() => break,
            Err(e) => return Err(format!("read: {e}")),
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let start = text.find('{').ok_or("seed stream: no JSON")?;
    let mut de = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
    de.next()
        .ok_or("seed stream: empty")?
        .map_err(|e| format!("seed stream parse: {e}"))
}

/// True once the buffer holds a balanced top-level JSON object (so an SSE socket that stays
/// open does not stall the read until timeout).
fn json_object_complete(buf: &[u8]) -> bool {
    let Some(start) = buf.iter().position(|b| *b == b'{') else {
        return false;
    };
    let (mut depth, mut in_str, mut esc) = (0i32, false, false);
    for &b in &buf[start..] {
        if in_str {
            match (esc, b) {
                (true, _) => esc = false,
                (false, b'\\') => esc = true,
                (false, b'"') => in_str = false,
                _ => {}
            }
        } else {
            match b {
                b'"' => in_str = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return true;
                    }
                }
                _ => {}
            }
        }
    }
    false
}

/// Splits `http://host:port[/path]` into (host:port, "/path"). TLS is not spoken here; run the
/// receiver behind a tailnet / TLS terminator (see the Security guide page).
pub fn split_url(url: &str) -> Result<(String, String), String> {
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        format!("only http:// receivers are supported (use a tailnet or TLS terminator): {url}")
    })?;
    Ok(match rest.split_once('/') {
        Some((h, p)) => (h.to_string(), format!("/{p}")),
        None => (rest.to_string(), "/ingest".to_string()),
    })
}

/// POSTs one signed reading and interprets the bridge's reply.
pub fn post_signed(
    to: &str,
    node: &str,
    key: &SigningKey,
    reading: &Value,
    now_ms: u64,
) -> Result<(), SendErr> {
    let transient = |m: String| SendErr {
        transient: true,
        clock: false,
        msg: m,
    };
    let (host, path) = split_url(to).map_err(|m| SendErr {
        transient: false,
        clock: false,
        msg: m,
    })?;
    let body = serde_json::to_vec(reading).map_err(|e| transient(format!("json: {e}")))?;
    let nonce = auth::new_nonce().map_err(transient)?;
    let mut req = format!("POST {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n");
    for h in auth::sign_headers(key, node, "POST", &path, &body, now_ms, &nonce) {
        req.push_str(&h);
        req.push_str("\r\n");
    }
    req.push_str(&format!(
        "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    ));
    let mut conn = net::connect(&host, IO_DEADLINE).map_err(transient)?;
    conn.set_write_timeout(Some(IO_DEADLINE)).ok();
    conn.write_all(req.as_bytes())
        .and_then(|_| conn.write_all(&body))
        .map_err(|e| transient(format!("write: {e}")))?;
    let mut resp = Vec::new();
    let mut tmp = [0u8; 1024];
    let mut rd = DeadlineStream::new(
        conn.try_clone()
            .map_err(|e| transient(format!("clone: {e}")))?,
        Instant::now() + IO_DEADLINE,
    );
    while !crate::server::response_complete(&resp) {
        match rd.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => resp.extend_from_slice(&tmp[..n]),
        }
    }
    let text = String::from_utf8_lossy(&resp);
    let code: u16 = text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let reply = text
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or("")
        .chars()
        .take(1024)
        .collect::<String>();
    let shown: String = reply.chars().take(160).collect();
    match classify(code, &reply) {
        Class::Stored => Ok(()),
        Class::Transient if code == 200 => {
            Err(transient(format!("receiver did not store: {shown}")))
        }
        Class::Transient => Err(transient(format!("HTTP {code}: {shown}"))),
        Class::Clock => Err(SendErr {
            transient: true,
            clock: true,
            msg: format!(
                "HTTP {code}: clocks differ (check time sync on this node and the receiver): {shown}"
            ),
        }),
        Class::Permanent => Err(SendErr {
            transient: false,
            clock: false,
            msg: format!("HTTP {code}: {shown}"),
        }),
    }
}

#[derive(Default, Debug)]
pub struct Relay {
    buf: VecDeque<Value>,
    cap: usize,
    pub dropped: u64,
    pub sent: u64,
    pub rejected: u64,
    pub failures: u64,
    backoff_ms: u64,
    next_try_ms: u64,
    rng: u64,
    clock_fails: u32,
}

impl Relay {
    pub fn new(cap: usize) -> Self {
        let seed = auth::random_bytes(8)
            .map(|b| u64::from_le_bytes(b[..8].try_into().unwrap_or([1; 8])))
            .unwrap_or(0x9e37_79b9_7f4a_7c15);
        Relay {
            cap: cap.max(1),
            rng: seed | 1,
            ..Default::default()
        }
    }

    #[cfg(test)]
    pub fn next_try_ms(&self) -> u64 {
        self.next_try_ms
    }

    /// xorshift64; jitter only needs to de-synchronise retrying nodes, not be secret.
    fn rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    pub fn queued(&self) -> usize {
        self.buf.len()
    }

    pub fn backoff_ms(&self) -> u64 {
        self.backoff_ms
    }

    /// Queues a reading; when full the oldest is dropped (newest data matters most).
    pub fn push(&mut self, r: Value) {
        if self.buf.len() >= self.cap {
            self.buf.pop_front();
            self.dropped += 1;
        }
        self.buf.push_back(r);
    }

    /// Sends queued readings oldest-first until one fails. Transient failures keep the reading
    /// and double the backoff (1 s .. 60 s); permanent ones drop it. Returns the last error.
    pub fn flush(
        &mut self,
        now_ms: u64,
        mut send: impl FnMut(&Value) -> Result<(), SendErr>,
        mut pace: impl FnMut(),
    ) -> Option<String> {
        if now_ms < self.next_try_ms {
            return None;
        }
        while let Some(front) = self.buf.front() {
            match send(front) {
                Ok(()) => {
                    self.buf.pop_front();
                    self.sent += 1;
                    self.backoff_ms = 0;
                    self.clock_fails = 0;
                    if !self.buf.is_empty() {
                        pace();
                    }
                }
                Err(e) => {
                    self.failures += 1;
                    let mut msg = e.msg;
                    let mut drop_it = !e.transient;
                    if e.clock {
                        self.clock_fails += 1;
                        if self.clock_fails > MAX_CLOCK_RETRIES {
                            drop_it = true;
                            self.clock_fails = 0;
                            msg.push_str(" (dropping this reading after repeated clock refusals)");
                        } else {
                            msg.push_str(&format!(
                                " (clock retry {}/{MAX_CLOCK_RETRIES})",
                                self.clock_fails
                            ));
                        }
                    } else {
                        self.clock_fails = 0;
                    }
                    if drop_it {
                        self.buf.pop_front();
                        self.rejected += 1;
                    }
                    self.backoff_ms = (self.backoff_ms * 2).clamp(BACKOFF_MIN_MS, BACKOFF_MAX_MS);
                    // +-20% jitter so a fleet of nodes does not retry in lockstep.
                    let span = self.backoff_ms / 5;
                    let jitter = (self.rand() % (2 * span + 1)) as i64 - span as i64;
                    self.next_try_ms = now_ms + (self.backoff_ms as i64 + jitter) as u64;
                    return Some(msg);
                }
            }
        }
        None
    }
}

/// The relay thread body: fetch, queue, flush, sleep. Logs on state changes only.
pub fn run(o: RelayOpts, unix_ms: fn() -> u64) {
    eprintln!(
        "[cog-bridge] relay: {} stream -> {} as node '{}' (buffer {}, every {}s)",
        o.seed, o.to, o.node, o.buffer, o.interval
    );
    let mut relay = Relay::new(o.buffer);
    let mut last_err = String::new();
    loop {
        let now = unix_ms();
        if now < auth::CLOCK_FLOOR_MS {
            // No RTC and not synced yet: a reading stamped 1970 would be refused as stale.
            note(
                &mut last_err,
                "waiting for the system clock to be set".into(),
            );
            std::thread::sleep(Duration::from_secs(o.interval));
            continue;
        }
        match fetch_seed_stream(&o.seed).and_then(|s| reading_from_stream(&s, &o.node, &o.cog, now))
        {
            Ok(r) => relay.push(r),
            Err(e) => note(&mut last_err, format!("seed: {e}")),
        }
        let sent = relay.flush(
            unix_ms(),
            |r| post_signed(&o.to, &o.node, &o.key, r, unix_ms()),
            || std::thread::sleep(DRAIN_PACE),
        );
        match sent {
            Some(e) => note(
                &mut last_err,
                format!(
                    "send: {e} (queued {}, backoff {} ms)",
                    relay.queued(),
                    relay.backoff_ms()
                ),
            ),
            None if !last_err.is_empty() && relay.queued() == 0 => {
                eprintln!(
                    "[cog-bridge] relay recovered ({} sent, {} dropped)",
                    relay.sent, relay.dropped
                );
                last_err.clear();
            }
            None => {}
        }
        std::thread::sleep(Duration::from_secs(o.interval));
    }
}

fn note(last: &mut String, msg: String) {
    if *last != msg {
        eprintln!("[cog-bridge] relay: {msg}");
        *last = msg;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> Result<(), SendErr> {
        Ok(())
    }
    fn transient() -> Result<(), SendErr> {
        Err(SendErr {
            transient: true,
            clock: false,
            msg: "down".into(),
        })
    }

    #[test]
    fn reading_takes_first_eight_normalized_values() {
        let s = serde_json::json!({"healthy": true, "sample_rate_hz": 10, "samples":
            (0..10).map(|i| serde_json::json!({"channel": format!("ch{i}"), "value": 100 + i, "normalized": i as f64 / 10.0})).collect::<Vec<_>>()});
        let r = reading_from_stream(&s, "seed-a", "seed-stream", 7).unwrap();
        assert_eq!(r["source"], "seed-a");
        assert_eq!(r["vector"].as_array().unwrap().len(), 8);
        assert_eq!(r["vector"][3], 0.3);
        assert_eq!(r["metrics"]["sample_count"], 10);
        assert!(reading_from_stream(&serde_json::json!({"samples": []}), "n", "c", 0).is_err());
    }

    #[test]
    fn buffer_is_bounded_and_drops_oldest() {
        let mut r = Relay::new(3);
        for i in 0..5 {
            r.push(serde_json::json!(i));
        }
        assert_eq!((r.queued(), r.dropped), (3, 2));
        assert_eq!(r.buf.front().unwrap(), &serde_json::json!(2));
    }

    #[test]
    fn backoff_doubles_to_a_cap_with_jitter_and_holds_the_queue() {
        let mut r = Relay::new(10);
        r.push(serde_json::json!(1));
        let mut now = 0;
        let mut seen = vec![];
        for _ in 0..8 {
            assert!(r.flush(now, |_| transient(), || {}).is_some());
            seen.push(r.backoff_ms());
            let wait = r.next_try_ms() - now;
            let base = r.backoff_ms();
            assert!(
                wait >= base * 8 / 10 && wait <= base * 12 / 10,
                "{wait} vs {base}"
            );
            now = r.next_try_ms();
        }
        assert_eq!(seen, [1000, 2000, 4000, 8000, 16000, 32000, 60000, 60000]);
        assert_eq!(r.queued(), 1);
        // Inside the backoff window nothing is attempted.
        let mut called = false;
        r.flush(
            now - 1,
            |_| {
                called = true;
                ok()
            },
            || {},
        );
        assert!(!called);
        // Recovery drains oldest-first, paces between sends, and resets the backoff.
        r.push(serde_json::json!(2));
        r.push(serde_json::json!(3));
        let (mut order, mut paced) = (vec![], 0);
        assert!(r
            .flush(
                now + 1,
                |v| {
                    order.push(v.clone());
                    ok()
                },
                || paced += 1,
            )
            .is_none());
        assert_eq!(
            order,
            [
                serde_json::json!(1),
                serde_json::json!(2),
                serde_json::json!(3)
            ]
        );
        assert_eq!(paced, 2, "paced between sends, not after the last");
        assert_eq!((r.queued(), r.backoff_ms(), r.sent), (0, 0, 3));
    }

    #[test]
    fn replies_are_classified_by_status_and_error_code() {
        let ok = r#"{"ok":true,"stored":"a/b -> store id 30"}"#;
        let e = |c: &str| format!(r#"{{"ok":false,"error":"{c}","detail":"x"}}"#);
        assert_eq!(classify(200, ok), Class::Stored);
        assert_eq!(
            classify(200, r#"{"ok":false,"error":"boom"}"#),
            Class::Transient
        );
        assert_eq!(classify(200, "garbage"), Class::Transient);
        assert_eq!(classify(401, &e("predates_restart")), Class::Transient);
        assert_eq!(classify(401, &e("stale_timestamp")), Class::Clock);
        for permanent in [
            "unknown_node",
            "bad_signature",
            "replayed_nonce",
            "missing_signature",
        ] {
            assert_eq!(
                classify(401, &e(permanent)),
                Class::Permanent,
                "{permanent}"
            );
        }
        assert_eq!(classify(403, &e("source_mismatch")), Class::Permanent);
        assert_eq!(classify(400, "x"), Class::Permanent);
        assert_eq!(classify(415, "x"), Class::Permanent);
        for t in [0, 408, 429, 500, 503] {
            assert_eq!(classify(t, &e("clock_not_set")), Class::Transient, "{t}");
        }
        assert_eq!(classify(401, "not json"), Class::Permanent);
    }

    #[test]
    fn a_transient_refusal_keeps_the_reading_for_a_resend() {
        let mut r = Relay::new(4);
        r.push(serde_json::json!(1));
        let now = 1_000;
        assert!(r.flush(now, |_| transient(), || {}).is_some());
        assert_eq!((r.queued(), r.rejected), (1, 0));
    }

    #[test]
    fn clock_refusals_retry_a_bounded_number_of_times_then_drop() {
        let clock = || {
            Err(SendErr {
                transient: true,
                clock: true,
                msg: "HTTP 401 stale".into(),
            })
        };
        let mut r = Relay::new(4);
        r.push(serde_json::json!(1));
        let mut now = 0;
        let mut last = String::new();
        for i in 1..=MAX_CLOCK_RETRIES {
            last = r.flush(now, |_| clock(), || {}).unwrap();
            assert!(
                last.contains(&format!("clock retry {i}/{MAX_CLOCK_RETRIES}")),
                "{last}"
            );
            assert_eq!((r.queued(), r.rejected), (1, 0), "retry {i}");
            now = r.next_try_ms();
        }
        let msg = r.flush(now, |_| clock(), || {}).unwrap();
        assert!(msg.contains("dropping"), "{msg} / {last}");
        assert_eq!((r.queued(), r.rejected), (0, 1));
        // A success resets the counter.
        r.push(serde_json::json!(2));
        assert!(r.flush(r.next_try_ms(), |_| ok(), || {}).is_none());
        assert_eq!(r.clock_fails, 0);
    }

    #[test]
    fn permanent_errors_drop_the_reading() {
        let mut r = Relay::new(4);
        r.push(serde_json::json!(1));
        r.flush(
            0,
            |_| {
                Err(SendErr {
                    transient: false,
                    clock: false,
                    msg: "HTTP 401".into(),
                })
            },
            || {},
        );
        assert_eq!((r.queued(), r.rejected), (0, 1));
    }

    #[test]
    fn urls_and_json_completeness() {
        assert_eq!(
            split_url("http://h:1/ingest").unwrap(),
            ("h:1".into(), "/ingest".into())
        );
        assert_eq!(split_url("http://h:1").unwrap().1, "/ingest");
        assert!(split_url("https://h").is_err());
        assert!(json_object_complete(
            b"HTTP/1.1 200\r\n\r\n{\"a\":\"}\",\"b\":{}}"
        ));
        assert!(!json_object_complete(b"data: {\"a\":1"));
    }
}
