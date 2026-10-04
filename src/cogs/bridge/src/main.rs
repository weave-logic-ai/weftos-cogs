//! Cognitum Cog: Sensor Bridge (COG-007, ADR-160)
//!
//! Receives sensor readings from external WeftOS nodes (a Pi 5, an Orange Pi) over HTTP and
//! writes them into the Seed store, so sensing done on cheap boards joins the Seed's memory.
//! The external node runs our cogs and `weft-bridge-send`, which posts a compact reading here.
//!
//!   POST /ingest   {"source","cog","ts_ms"?,"vector":[8 floats],"metrics":{...}}  -> store
//!   GET  /status   counts, sources, last reading per source
//!   GET  /sources  the (source, cog) -> store id table
//!   GET  /guide    this cog's sensor guide (WeftOS ADR-104)
//!
//! Each distinct (source, cog) gets its own store-vector id from `base_store_id` up. The bridge
//! treats the vector as an opaque 8-dim point, exactly as the Seed store does; rich data (an ECG
//! waveform, a depth frame) stays on the originating node's own export.
//!
//! Auth (COG-011): each sending node signs its requests with its own Ed25519 key and the bridge
//! checks an allowlist of node public keys (`auth.rs`). The shared `X-Bridge-Token` survives only
//! as a deprecated fallback behind `--allow-shared-token`.
//!
//! Outbound (phase 2): `--relay-to <url>` republishes this Seed's sensor stream to another bridge
//! receiver, signed with `--key-file` (`relay.rs`).
//!
//! Usage:
//!   cog-bridge --interval 5                 # listen and report every 5 s
//!   cog-bridge --once --simulate            # inject one synthetic source, report, exit
//!   cog-bridge keygen <node> <key-file>     # make a node keypair; prints the allowlist line

mod auth;
mod guide;
mod net;
mod relay;
mod server;

#[cfg(test)]
mod e2e_tests;

use serde_json::Value;
use server::{serve, Ctx, StoreFn};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TAG: &str = "[cog-bridge]";
const STORE_DIM: usize = 8;
/// Distinct cogs one source (node) may create store ids for.
const MAX_COGS_PER_SOURCE: usize = 8;
/// Largest `metrics` object kept per source, serialized.
const MAX_METRICS_BYTES: usize = 4096;

struct Opts {
    once: bool,
    interval: u64,
    base_store_id: u32,
    max_sources: usize,
    token: String,
    api_bind: String,
    simulate: bool,
    allowlist: String,
    allow_shared_token: bool,
    auth_window: u64,
    no_store: bool,
    relay_to: String,
    node_id: String,
    key_file: String,
    seed_url: String,
    relay_cog: String,
    relay_buffer: usize,
    insecure_open: bool,
    status_auth: bool,
}

fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn num<T: std::str::FromStr + PartialOrd + Copy>(
    args: &[String],
    flag: &str,
    d: T,
    lo: T,
    hi: T,
) -> T {
    match arg(args, flag).and_then(|v| v.parse::<T>().ok()) {
        Some(v) if v >= lo && v <= hi => v,
        _ => d,
    }
}

fn parse_opts(args: &[String]) -> Opts {
    Opts {
        once: args.iter().any(|a| a == "--once"),
        interval: num(args, "--interval", 5, 1, 60),
        base_store_id: num(args, "--base-store-id", 30, 1, 200),
        max_sources: num(args, "--max-sources", 16, 1, 64),
        // --token shows in `ps`; BRIDGE_TOKEN keeps it out of argv.
        token: arg(args, "--token")
            .map(String::from)
            .or_else(|| std::env::var("BRIDGE_TOKEN").ok())
            .unwrap_or_default(),
        api_bind: arg(args, "--api-bind")
            .unwrap_or("127.0.0.1:8048")
            .to_string(),
        simulate: args.iter().any(|a| a == "--simulate"),
        allowlist: arg(args, "--allowlist").unwrap_or("").to_string(),
        allow_shared_token: args.iter().any(|a| a == "--allow-shared-token"),
        auth_window: num(args, "--auth-window", 60, 5, 600),
        no_store: args.iter().any(|a| a == "--no-store"),
        relay_to: arg(args, "--relay-to").unwrap_or("").to_string(),
        node_id: arg(args, "--node-id").unwrap_or("").to_string(),
        key_file: arg(args, "--key-file").unwrap_or("").to_string(),
        seed_url: arg(args, "--seed-url")
            .unwrap_or("127.0.0.1:80")
            .to_string(),
        relay_cog: arg(args, "--relay-cog")
            .unwrap_or("seed-stream")
            .to_string(),
        relay_buffer: num(args, "--relay-buffer", 256, 1, 4096),
        insecure_open: args.iter().any(|a| a == "--insecure-open"),
        status_auth: args.iter().any(|a| a == "--status-auth"),
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Default)]
struct Source {
    store_id: u32,
    count: u64,
    last_ts_ms: u64,
    last_recv_ms: u64,
    last_vector: Vec<f64>,
    last_metrics: Value,
}

#[derive(Default)]
struct State {
    sources: BTreeMap<String, Source>,
    next_id: u32,
    base_id: u32,
    max_sources: usize,
    rejected: u64,
    store_errors: u64,
    auth_refused: u64,
    /// 429s from the per-IP request throttle (not auth refusals).
    throttled: u64,
    /// Connections refused at the global or per-IP connection cap.
    busy_refused: u64,
    auth_mode: &'static str,
    bind_scope: &'static str,
}

impl State {
    fn new(base: u32, max: usize) -> Self {
        Self {
            next_id: base,
            base_id: base,
            max_sources: max,
            ..Default::default()
        }
    }

    /// Assigns (or finds) the store id for a `(source, cog)` pair. Err says which limit hit:
    /// the global source table, or one source's own quota of cogs.
    fn store_id_for(&mut self, source: &str, cog: &str) -> Result<u32, String> {
        let key = format!("{source}/{cog}");
        if let Some(s) = self.sources.get(&key) {
            return Ok(s.store_id);
        }
        if self.sources.len() >= self.max_sources {
            return Err(format!(
                "at capacity ({} sources); rejected {key}",
                self.max_sources
            ));
        }
        let prefix = format!("{source}/");
        if self
            .sources
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .count()
            >= MAX_COGS_PER_SOURCE
        {
            return Err(format!(
                "source '{source}' already has {MAX_COGS_PER_SOURCE} cogs; rejected {key}"
            ));
        }
        let id = self.next_id;
        self.next_id += 1;
        self.sources.insert(
            key,
            Source {
                store_id: id,
                ..Default::default()
            },
        );
        Ok(id)
    }

    /// `full` adds per-source names, vectors and metrics; the summary is counts only.
    fn snapshot(&self, full: bool) -> Value {
        let srcs: Vec<Value> = self
            .sources
            .iter()
            .map(|(k, s)| {
                serde_json::json!({
                    "key": k, "store_id": s.store_id, "count": s.count,
                    "last_ts_ms": s.last_ts_ms, "age_ms": unix_ms().saturating_sub(s.last_recv_ms),
                    "last_vector": s.last_vector, "last_metrics": s.last_metrics,
                })
            })
            .collect();
        let mut v = serde_json::json!({
            "status": "ok",
            "sources": self.sources.len(),
            "readings": self.sources.values().map(|s| s.count).sum::<u64>(),
            "rejected": self.rejected,
            "store_errors": self.store_errors,
            "auth_mode": self.auth_mode,
            "auth_refused": self.auth_refused,
            "throttled": self.throttled,
            "busy_refused": self.busy_refused,
            "bind_scope": self.bind_scope,
            "timestamp": unix_ms() / 1000,
        });
        if full {
            v["base_store_id"] = self.base_id.into();
            v["detail"] = srcs.into();
        }
        v
    }
}

type Shared = Arc<Mutex<State>>;

/// A received reading, after validation.
struct Reading {
    source: String,
    cog: String,
    ts_ms: u64,
    vector: Vec<f64>,
    metrics: Value,
}

/// Parses and validates an `/ingest` body. The vector is clamped to [0,1] and fixed to 8 dims.
fn parse_reading(body: &Value) -> Result<Reading, String> {
    let source = body["source"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("missing 'source'")?;
    let cog = body["cog"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("missing 'cog'")?;
    if !auth::valid_node_name(source) || !auth::valid_node_name(cog) {
        return Err("source and cog must be 1-64 chars of [A-Za-z0-9._-]".into());
    }
    let raw = body["vector"].as_array().ok_or("missing 'vector' array")?;
    if raw.is_empty() || raw.len() > STORE_DIM {
        return Err(format!("vector must be 1-{STORE_DIM} numbers"));
    }
    let mut vector: Vec<f64> = raw
        .iter()
        .map(|v| v.as_f64().unwrap_or(0.0))
        .map(|v| {
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                0.0
            }
        })
        .collect();
    vector.resize(STORE_DIM, 0.0);
    Ok(Reading {
        source: source.to_string(),
        cog: cog.to_string(),
        ts_ms: body["ts_ms"].as_u64().unwrap_or_else(unix_ms),
        vector,
        metrics: body
            .get("metrics")
            .filter(|m| m.to_string().len() <= MAX_METRICS_BYTES)
            .cloned()
            .unwrap_or(Value::Null),
    })
}

/// Writes one vector to the Seed store over loopback. The agent keeps the socket open after it
/// replies (cogs repo ADR-158), so the read stops at Content-Length instead of EOF.
fn store_ingest(store_id: u32, vector: &[f64]) -> Result<(), String> {
    let payload = serde_json::json!({ "vectors": [[store_id, vector]], "dedup": true });
    let body = serde_json::to_vec(&payload).map_err(|e| format!("json: {e}"))?;
    let mut conn = TcpStream::connect("127.0.0.1:80").map_err(|e| format!("connect: {e}"))?;
    conn.set_read_timeout(Some(Duration::from_secs(5))).ok();
    conn.set_write_timeout(Some(Duration::from_secs(5))).ok();
    write!(conn, "POST /api/v1/store/ingest HTTP/1.0\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).map_err(|e| format!("write: {e}"))?;
    conn.write_all(&body).map_err(|e| format!("body: {e}"))?;
    let mut resp = Vec::new();
    let mut buf = [0u8; 1024];
    while !server::response_complete(&resp) {
        match conn.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => resp.extend_from_slice(&buf[..n]),
        }
    }
    let code = String::from_utf8_lossy(&resp)
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();
    if code.starts_with('2') {
        Ok(())
    } else {
        Err(format!("store ingest replied {code:?}"))
    }
}

/// Applies a validated reading: assign/find the store id, write it, update the source. Returns
/// a one-line summary (and whether it was stored), or an error string.
fn apply_reading(
    shared: &Shared,
    r: Reading,
    store: impl Fn(u32, &[f64]) -> Result<(), String>,
) -> Result<String, String> {
    let key = format!("{}/{}", r.source, r.cog);
    let store_id = {
        let mut st = shared.lock().map_err(|_| "state poisoned")?;
        match st.store_id_for(&r.source, &r.cog) {
            Ok(id) => id,
            Err(e) => {
                st.rejected += 1;
                return Err(e);
            }
        }
    };
    let stored = store(store_id, &r.vector);
    let mut st = shared.lock().map_err(|_| "state poisoned")?;
    if let Some(s) = st.sources.get_mut(&key) {
        s.count += 1;
        s.last_ts_ms = r.ts_ms;
        s.last_recv_ms = unix_ms();
        s.last_vector = r.vector.clone();
        s.last_metrics = r.metrics;
    }
    match stored {
        Ok(()) => Ok(format!("{key} -> store id {store_id}")),
        Err(e) => {
            st.store_errors += 1;
            Err(format!("{key}: {e}"))
        }
    }
}

/// The start-up notice for a loopback-only bind, or None for a network bind. The 0.1 default
/// was open on `0.0.0.0:8048`, so an upgraded install with the default config is open mode on
/// loopback and would otherwise lose its remote nodes without a word.
fn bind_notice(loopback_only: bool, mode: &str, bind: &str) -> Option<String> {
    if !loopback_only {
        return None;
    }
    let how = if mode == "open" {
        "0.1 accepted remote nodes on 0.0.0.0:8048; 0.2 binds 127.0.0.1 by default. Set api_bind \
         and an allowlist (or token) to accept remote nodes"
    } else {
        "Remote nodes cannot reach the bridge. Set api_bind=0.0.0.0:8048 to accept them"
    };
    Some(format!(
        "NOTICE: bound to {bind} (this Seed only). {how} (guide: Setup, 'Upgrading from 0.1')."
    ))
}

/// True when `bind` (`host:port`) is a loopback address. A hostname is not trusted to be one.
fn is_loopback_bind(bind: &str) -> bool {
    bind.parse::<std::net::SocketAddr>()
        .map(|a| a.ip().is_loopback())
        .unwrap_or(false)
}

fn start_relay(o: &Opts) {
    if !auth::valid_node_name(&o.node_id) {
        eprintln!("{TAG} relay needs --node-id (1-64 chars of [A-Za-z0-9._-]); not started");
        return;
    }
    let key = match auth::load_key(&o.key_file) {
        Ok(k) => k,
        Err(e) => {
            return eprintln!("{TAG} relay: {e}; not started (make one with `cog-bridge keygen`)")
        }
    };
    let opts = relay::RelayOpts {
        seed: o.seed_url.clone(),
        to: o.relay_to.clone(),
        node: o.node_id.clone(),
        cog: o.relay_cog.clone(),
        key,
        interval: o.interval,
        buffer: o.relay_buffer,
    };
    std::thread::spawn(move || relay::run(opts, unix_ms));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    cog_sensor_sources::handle_help(
        &args,
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        include_str!("../cog.toml"),
    );
    if args.get(1).map(String::as_str) == Some("keygen") {
        match (args.get(2), args.get(3)) {
            (Some(node), Some(out)) => match auth::keygen(node, out) {
                Ok(line) => {
                    eprintln!("{TAG} secret key written to {out} (keep it private; mode 0600)");
                    println!("{line}");
                    return;
                }
                Err(e) => {
                    eprintln!("{TAG} keygen: {e}");
                    std::process::exit(1);
                }
            },
            _ => {
                eprintln!("usage: cog-bridge keygen <node-name> <key-file>");
                std::process::exit(2);
            }
        }
    }
    let o = parse_opts(&args);
    eprintln!(
        "{TAG} start (once={}, interval={}s, bind={}, base_store_id={}, max_sources={}, simulate={}, store={})",
        o.once, o.interval, o.api_bind, o.base_store_id, o.max_sources, o.simulate, if o.no_store { "none" } else { "seed" }
    );
    let auth = match auth::Auth::new(
        Some(o.allowlist.as_str()),
        o.auth_window,
        &o.token,
        o.allow_shared_token,
        unix_ms(),
    ) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{TAG} auth config: {e}");
            std::process::exit(1);
        }
    };
    let mode = auth.mode();
    eprintln!("{TAG} auth: {mode} ({} allowlisted nodes)", auth.nodes());
    match mode {
        "open" => eprintln!("{TAG} WARNING: no allowlist and no token: anyone on the network can write readings"),
        "locked" => eprintln!("{TAG} WARNING: shared token is deprecated and disabled; set allowlist (or allow_shared_token); all ingest is refused"),
        "deprecated-token" | "signed+deprecated-token" => eprintln!("{TAG} WARNING: deprecated shared token enabled"),
        _ => {}
    }
    if mode == "open" && !o.insecure_open && !is_loopback_bind(&o.api_bind) {
        eprintln!(
            "{TAG} refusing to start: no allowlist and no token, but bound to {} (not loopback). \
             Set an allowlist (recommended), bind 127.0.0.1, or pass --insecure-open to accept \
             unauthenticated writes from the network.",
            o.api_bind
        );
        std::process::exit(1);
    }
    let mut state = State::new(o.base_store_id, o.max_sources);
    state.auth_mode = mode;
    let loopback_only = is_loopback_bind(&o.api_bind);
    state.bind_scope = if loopback_only { "loopback" } else { "network" };
    if let Some(n) = bind_notice(loopback_only, mode, &o.api_bind) {
        eprintln!("{TAG} {n}");
    }
    let shared: Shared = Arc::new(Mutex::new(state));
    let store: StoreFn = if o.no_store {
        Arc::new(|_, _| Ok(()))
    } else {
        Arc::new(store_ingest)
    };
    let ctx = Ctx::new(shared.clone(), auth, store.clone(), o.status_auth);
    match serve(&o.api_bind, ctx) {
        Ok(addr) => eprintln!("{TAG} listening on http://{addr}/ingest"),
        Err(e) => eprintln!("{TAG} bind failed: {e} (readings can't be received)"),
    }
    if !o.relay_to.is_empty() {
        start_relay(&o);
    }
    if o.simulate {
        let r = Reading {
            source: "sim-node".into(),
            cog: "sen0628-tof".into(),
            ts_ms: unix_ms(),
            vector: vec![0.34, 1.0, 1.0, 0.1, 0.08, 0.68, 0.68, 0.34],
            metrics: serde_json::json!({"nearest_mm": 1193, "presence": true}),
        };
        match apply_reading(&shared, r, |id, v| store(id, v)) {
            Ok(m) => eprintln!("{TAG} simulated reading stored: {m}"),
            Err(e) => eprintln!("{TAG} simulated reading: {e}"),
        }
    }
    loop {
        std::thread::sleep(Duration::from_secs(o.interval));
        let report = shared
            .lock()
            .map(|s| s.snapshot(true))
            .unwrap_or(Value::Null);
        println!("{report}");
        if o.once {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn s(v: &[&str]) -> Vec<String> {
        std::iter::once("cog")
            .chain(v.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn options_bounded_and_defaulted() {
        let o = parse_opts(&s(&["--once", "--base-store-id", "999", "--interval", "5"]));
        assert!(o.once);
        assert_eq!(o.base_store_id, 30);
        assert_eq!(o.api_bind, "127.0.0.1:8048");
    }

    #[test]
    fn readings_validate_clamp_and_pad_to_eight() {
        let ok = parse_reading(
            &serde_json::json!({"source":"pi5","cog":"sen0628-tof","vector":[0.5, 2.0, -1.0]}),
        )
        .unwrap();
        assert_eq!(ok.vector, vec![0.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert!(parse_reading(&serde_json::json!({"cog":"x","vector":[1]})).is_err());
        assert!(parse_reading(&serde_json::json!({"source":"x","cog":"y","vector":[]})).is_err());
        assert!(parse_reading(
            &serde_json::json!({"source":"x","cog":"y","vector":[1,2,3,4,5,6,7,8,9]})
        )
        .is_err());
    }

    #[test]
    fn sources_get_distinct_ids_from_the_base_and_cap() {
        let mut st = State::new(30, 2);
        assert_eq!(st.store_id_for("pi5", "ecg"), Ok(30));
        assert_eq!(st.store_id_for("pi5", "ecg"), Ok(30));
        assert_eq!(st.store_id_for("opi", "tof"), Ok(31));
        assert!(st.store_id_for("third", "x").is_err());
    }

    #[test]
    fn one_source_cannot_take_more_than_its_cog_quota() {
        let mut st = State::new(30, 64);
        for i in 0..MAX_COGS_PER_SOURCE {
            assert!(st.store_id_for("pi5", &format!("c{i}")).is_ok());
        }
        assert!(st
            .store_id_for("pi5", "extra")
            .unwrap_err()
            .contains("already has"));
        assert!(st.store_id_for("opi", "c0").is_ok());
    }

    #[test]
    fn loopback_bind_always_gets_a_notice_with_mode_specific_wording() {
        let open = bind_notice(true, "open", "127.0.0.1:8048").unwrap();
        assert!(
            open.contains("0.1 accepted remote nodes on 0.0.0.0:8048"),
            "{open}"
        );
        assert!(open.contains("allowlist"), "{open}");
        for mode in [
            "signed",
            "locked",
            "deprecated-token",
            "signed+deprecated-token",
        ] {
            let n = bind_notice(true, mode, "127.0.0.1:8048").unwrap();
            assert!(
                n.contains("Remote nodes cannot reach the bridge"),
                "{mode}: {n}"
            );
        }
        assert!(bind_notice(false, "open", "0.0.0.0:8048").is_none());
        assert!(bind_notice(false, "signed", "0.0.0.0:8048").is_none());
    }

    #[test]
    fn open_mode_loopback_check() {
        assert!(is_loopback_bind("127.0.0.1:8048"));
        assert!(is_loopback_bind("[::1]:8048"));
        assert!(!is_loopback_bind("0.0.0.0:8048"));
        assert!(!is_loopback_bind("192.168.1.5:8048"));
        assert!(!is_loopback_bind("localhost:8048"));
    }

    #[test]
    fn apply_reading_assigns_id_counts_and_reports_store_result() {
        let shared: Shared = Arc::new(Mutex::new(State::new(40, 8)));
        let seen = Arc::new(AtomicU32::new(0));
        let seen2 = seen.clone();
        let r = parse_reading(&serde_json::json!({"source":"pi5","cog":"tof","vector":[0.3,1,1,0,0,0,0,0],"metrics":{"nearest_mm":1200}})).unwrap();
        let msg = apply_reading(&shared, r, move |id, v| {
            seen2.store(id, Ordering::SeqCst);
            assert_eq!(v.len(), 8);
            Ok(())
        })
        .unwrap();
        assert_eq!(msg, "pi5/tof -> store id 40");
        assert_eq!(seen.load(Ordering::SeqCst), 40);
        let snap = shared.lock().unwrap().snapshot(true);
        assert_eq!(snap["readings"], 1);
        assert_eq!(snap["detail"][0]["last_metrics"]["nearest_mm"], 1200);

        // A store failure is counted but the source still advances.
        let r2 = parse_reading(
            &serde_json::json!({"source":"pi5","cog":"tof","vector":[0.4,1,1,0,0,0,0,0]}),
        )
        .unwrap();
        assert!(apply_reading(&shared, r2, |_, _| Err("boom".into())).is_err());
        assert_eq!(shared.lock().unwrap().snapshot(true)["store_errors"], 1);
    }

    #[test]
    fn ingest_response_is_complete_at_content_length() {
        assert!(!server::response_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n{}"
        ));
        assert!(server::response_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}"
        ));
    }
}
