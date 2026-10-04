//! Per-node Ed25519 identity for the bridge (COG-011, ADR-160 amendments 1 and 2).
//!
//! Every sending node holds an Ed25519 keypair. The bridge holds an allowlist of node public
//! keys (a plain text file). A request is signed over
//!
//! ```text
//! weft-bridge-v1\n METHOD \n PATH \n NODE \n TIMESTAMP_MS \n NONCE \n SHA256_HEX(BODY)
//! ```
//!
//! and carries `X-Bridge-Node`, `X-Bridge-Timestamp`, `X-Bridge-Nonce`, `X-Bridge-Signature`
//! (hex). The bridge checks the node is allowlisted, the timestamp is inside a window and not
//! older than the process, the signature verifies (strictly), the nonce was not seen, and the
//! node is inside its rate and nonce budgets. Replay and rate state are only written after the
//! signature verifies, so unauthenticated traffic cannot fill them or drain a node's budget.
//! The signature check runs outside the state lock.

use crate::net::Bucket;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use zeroize::Zeroizing;

pub const DOMAIN: &str = "weft-bridge-v1";
pub const H_NODE: &str = "x-bridge-node";
pub const H_TS: &str = "x-bridge-timestamp";
pub const H_NONCE: &str = "x-bridge-nonce";
pub const H_SIG: &str = "x-bridge-signature";
pub const H_TOKEN: &str = "x-bridge-token";
/// Live nonces across all nodes (backstop).
const NONCE_CAP: usize = 4096;
/// Live nonces for one node: one node cannot evict or crowd out the others.
pub const NODE_NONCE_CAP: usize = 512;
/// Verified requests per second per node, and burst. 16 + 8 * 60 s stays under NODE_NONCE_CAP.
const NODE_RATE: f64 = 8.0;
const NODE_BURST: f64 = 16.0;
/// Largest accepted timestamp (year 2100, in ms); bounds all timestamp arithmetic.
const MAX_TS_MS: u64 = 4_102_444_800_000;
const ALLOWLIST_MAX_BYTES: u64 = 65_536;
const ALLOWLIST_CHECK_MS: u64 = 1_000;
/// A Seed or Pi without a battery-backed clock boots at the epoch until NTP runs. Before the
/// clock is past this floor (2026-05-28) signed requests get `clock_not_set`, and the process
/// start time used by the restart-replay rule is taken when the clock first becomes sane.
pub const CLOCK_FLOOR_MS: u64 = 1_780_000_000_000;

pub fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Reads `n` bytes from the OS RNG (std only; avoids pulling a rand crate onto the Seed).
pub fn random_bytes(n: usize) -> Result<Zeroizing<Vec<u8>>, String> {
    let mut f = std::fs::File::open("/dev/urandom").map_err(|e| format!("/dev/urandom: {e}"))?;
    let mut b = Zeroizing::new(vec![0u8; n]);
    f.read_exact(&mut b)
        .map_err(|e| format!("/dev/urandom: {e}"))?;
    Ok(b)
}

/// Names (nodes, sources, cogs) are bounded and charset-limited so they are safe in headers,
/// logs and store keys, and `/` never appears (a `source/cog` key cannot collide).
pub fn valid_node_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// The exact bytes that are signed.
pub fn signing_string(
    method: &str,
    path: &str,
    node: &str,
    ts_ms: u64,
    nonce: &str,
    body: &[u8],
) -> String {
    format!(
        "{DOMAIN}\n{method}\n{path}\n{node}\n{ts_ms}\n{nonce}\n{}",
        to_hex(&Sha256::digest(body))
    )
}

/// Header lines (without CRLF) a client adds to sign a request.
pub fn sign_headers(
    key: &SigningKey,
    node: &str,
    method: &str,
    path: &str,
    body: &[u8],
    ts_ms: u64,
    nonce: &str,
) -> Vec<String> {
    let sig = key.sign(signing_string(method, path, node, ts_ms, nonce, body).as_bytes());
    vec![
        format!("X-Bridge-Node: {node}"),
        format!("X-Bridge-Timestamp: {ts_ms}"),
        format!("X-Bridge-Nonce: {nonce}"),
        format!("X-Bridge-Signature: {}", to_hex(&sig.to_bytes())),
    ]
}

pub fn new_nonce() -> Result<String, String> {
    Ok(to_hex(&random_bytes(16)?))
}

/// Permission bits of a file (always 0600 off unix, so the checks pass there).
#[cfg(unix)]
fn mode_bits(path: &std::path::Path) -> Result<u32, String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o777)
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(not(unix))]
fn mode_bits(_: &std::path::Path) -> Result<u32, String> {
    Ok(0o600)
}

/// Loads a node secret key: a file holding 64 hex characters (the 32-byte seed). Refuses a file
/// that group or others can access, like ssh does for private keys.
pub fn load_key(path: &str) -> Result<SigningKey, String> {
    let mode = mode_bits(std::path::Path::new(path))?;
    if mode & 0o077 != 0 {
        return Err(format!(
            "key file {path} has mode {mode:o}; it must not be accessible by group or others (chmod 600)"
        ));
    }
    let txt =
        Zeroizing::new(std::fs::read_to_string(path).map_err(|e| format!("key file {path}: {e}"))?);
    let bytes = Zeroizing::new(from_hex(txt.trim()).ok_or("key file must hold 64 hex characters")?);
    let seed: Zeroizing<[u8; 32]> = Zeroizing::new(
        bytes[..]
            .try_into()
            .map_err(|_| "key file must hold 64 hex characters (32 bytes)")?,
    );
    Ok(SigningKey::from_bytes(&seed))
}

/// Generates a keypair, writes the secret (mode 0600, never overwriting) and returns the
/// allowlist line `NODE PUBKEY_HEX`.
pub fn keygen(node: &str, out: &str) -> Result<String, String> {
    if !valid_node_name(node) {
        return Err("node name must be 1-64 chars of [A-Za-z0-9._-]".into());
    }
    let rnd = random_bytes(32)?;
    let seed: Zeroizing<[u8; 32]> = Zeroizing::new(rnd[..].try_into().map_err(|_| "rng")?);
    let key = SigningKey::from_bytes(&seed);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts
        .open(out)
        .map_err(|e| format!("{out}: {e} (refusing to overwrite an existing key)"))?;
    let line = Zeroizing::new(format!("{}\n", to_hex(&seed[..])));
    std::io::Write::write_all(&mut f, line.as_bytes()).map_err(|e| format!("{out}: {e}"))?;
    Ok(format!("{node} {}", to_hex(key.verifying_key().as_bytes())))
}

/// Parses an allowlist: `NODE PUBKEY_HEX` per line, `#` comments, blank lines ignored.
pub fn parse_allowlist(txt: &str) -> Result<HashMap<String, VerifyingKey>, String> {
    let mut m = HashMap::new();
    for (i, raw) in txt.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut p = line.split_whitespace();
        let (Some(name), Some(hex), None) = (p.next(), p.next(), p.next()) else {
            return Err(format!("allowlist line {}: want `NODE PUBKEY_HEX`", i + 1));
        };
        if !valid_node_name(name) {
            return Err(format!("allowlist line {}: bad node name", i + 1));
        }
        let bytes: [u8; 32] = from_hex(hex)
            .and_then(|b| b.try_into().ok())
            .ok_or(format!("allowlist line {}: pubkey must be 64 hex", i + 1))?;
        let vk = VerifyingKey::from_bytes(&bytes)
            .map_err(|e| format!("allowlist line {}: bad pubkey: {e}", i + 1))?;
        if m.insert(name.to_string(), vk).is_some() {
            return Err(format!("allowlist line {}: duplicate node {name}", i + 1));
        }
    }
    Ok(m)
}

type Loaded = (HashMap<String, VerifyingKey>, [u8; 32]);

/// Reads and parses the allowlist file, refusing one that group/others can write (they could
/// add themselves). Returns the keys and a hash of the exact content.
fn load_allowlist(path: &std::path::Path) -> Result<Loaded, String> {
    let mode = mode_bits(path)?;
    if mode & 0o022 != 0 {
        return Err(format!(
            "allowlist {} has mode {mode:o}; it must not be writable by group or others",
            path.display()
        ));
    }
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if len > ALLOWLIST_MAX_BYTES {
        return Err(format!(
            "allowlist {} is larger than {ALLOWLIST_MAX_BYTES} bytes",
            path.display()
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("allowlist {}: {e}", path.display()))?;
    let txt = std::str::from_utf8(&bytes).map_err(|_| "allowlist is not UTF-8".to_string())?;
    Ok((parse_allowlist(txt)?, Sha256::digest(&bytes).into()))
}

/// Why a request was refused. Each has a stable machine code, an HTTP status and a message that
/// says what to fix.
#[derive(Debug, PartialEq)]
pub enum Refusal {
    MissingSignature,
    Malformed(String),
    UnknownNode,
    BadSignature,
    Stale { skew_ms: i64, window_ms: u64 },
    PredatesStart,
    Replay,
    CacheFull,
    RateLimited,
    ClockNotSet,
    SourceMismatch { node: String, source: String },
    TokenDisabled,
    BadToken,
}

impl Refusal {
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingSignature => "missing_signature",
            Self::Malformed(_) => "malformed_auth",
            Self::UnknownNode => "unknown_node",
            Self::BadSignature => "bad_signature",
            Self::Stale { .. } => "stale_timestamp",
            Self::PredatesStart => "predates_restart",
            Self::Replay => "replayed_nonce",
            Self::CacheFull => "nonce_cache_full",
            Self::RateLimited => "rate_limited",
            Self::ClockNotSet => "clock_not_set",
            Self::SourceMismatch { .. } => "source_mismatch",
            Self::TokenDisabled => "shared_token_disabled",
            Self::BadToken => "bad_token",
        }
    }

    pub fn status(&self) -> u16 {
        match self {
            Self::SourceMismatch { .. } => 403,
            Self::CacheFull | Self::ClockNotSet => 503,
            Self::RateLimited => 429,
            Self::Malformed(_) => 400,
            _ => 401,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::MissingSignature => "signed request required: send X-Bridge-Node, X-Bridge-Timestamp, X-Bridge-Nonce and X-Bridge-Signature".into(),
            Self::Malformed(m) => format!("malformed auth headers: {m}"),
            Self::UnknownNode => "node is not in the bridge allowlist".into(),
            Self::BadSignature => "signature does not verify for this node's key".into(),
            Self::Stale { skew_ms, window_ms } => format!("timestamp is {skew_ms} ms from the bridge clock; allowed window is +/-{window_ms} ms (check the sender's clock)"),
            Self::PredatesStart => "timestamp is older than the bridge process start (requests signed before a restart are refused); sign a fresh request".into(),
            Self::Replay => "nonce already used; every request needs a fresh nonce".into(),
            Self::CacheFull => "replay cache is full for this node; retry shortly".into(),
            Self::RateLimited => "too many requests; slow down and retry".into(),
            Self::ClockNotSet => "the bridge's clock is not set yet (no RTC, waiting for time sync); signed requests are refused until it is".into(),
            Self::SourceMismatch { node, source } => format!("node '{node}' may only send readings with source '{node}', not '{source}'"),
            Self::TokenDisabled => "the shared X-Bridge-Token is deprecated and disabled; use a signed request or set allow_shared_token".into(),
            Self::BadToken => "bad or missing X-Bridge-Token".into(),
        }
    }
}

/// The auth-relevant parts of one request.
#[derive(Default)]
pub struct AuthReq<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub body: &'a [u8],
    pub node: Option<&'a str>,
    pub ts: Option<&'a str>,
    pub nonce: Option<&'a str>,
    pub sig: Option<&'a str>,
    pub token: Option<&'a str>,
}

/// A signed request that passed the cheap checks and still needs its signature verified.
pub struct Pending {
    node: String,
    ts_ms: u64,
    nonce: String,
    vk: VerifyingKey,
    sig: Signature,
    msg: String,
}

impl Pending {
    /// The Ed25519 check, done outside the state lock.
    pub fn verify(&self) -> Result<(), Refusal> {
        self.vk
            .verify_strict(self.msg.as_bytes(), &self.sig)
            .map_err(|_| Refusal::BadSignature)
    }
}

pub enum Plan {
    /// Decided without a signature (legacy token or open mode).
    Done(Option<String>),
    Verify(Box<Pending>),
}

pub struct Auth {
    allow: HashMap<String, VerifyingKey>,
    path: Option<PathBuf>,
    file_hash: Option<[u8; 32]>,
    failed_hash: Option<[u8; 32]>,
    error: Option<String>,
    loaded_ms: u64,
    last_check_ms: u64,
    window_ms: u64,
    started_ms: u64,
    nonces: HashMap<(String, String), u64>,
    rates: HashMap<String, Bucket>,
    token: String,
    allow_token: bool,
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

impl Auth {
    pub fn new(
        allowlist: Option<&str>,
        window_secs: u64,
        token: &str,
        allow_token: bool,
        now_ms: u64,
    ) -> Result<Self, String> {
        let mut a = Auth {
            allow: HashMap::new(),
            path: allowlist.filter(|p| !p.is_empty()).map(PathBuf::from),
            file_hash: None,
            failed_hash: None,
            error: None,
            loaded_ms: now_ms,
            last_check_ms: now_ms,
            window_ms: window_secs.saturating_mul(1000),
            started_ms: now_ms,
            nonces: HashMap::new(),
            rates: HashMap::new(),
            token: token.to_string(),
            allow_token,
        };
        if let Some(p) = &a.path {
            let (keys, hash) = load_allowlist(p)?;
            a.allow = keys;
            a.file_hash = Some(hash);
        }
        Ok(a)
    }

    /// Re-reads the allowlist when its content changed (checked at most once a second, by hash,
    /// so a same-size same-mtime edit is still seen). A bad edit keeps the previous keys, is
    /// reported in `/status`, and is not re-logged until the file changes again.
    fn refresh(&mut self, now_ms: u64) {
        let Some(p) = self.path.clone() else { return };
        if now_ms.saturating_sub(self.last_check_ms) < ALLOWLIST_CHECK_MS {
            return;
        }
        self.last_check_ms = now_ms;
        match load_allowlist(&p) {
            Ok((keys, hash)) => {
                if Some(hash) != self.file_hash {
                    eprintln!("[cog-bridge] allowlist reloaded ({} nodes)", keys.len());
                    self.allow = keys;
                    self.file_hash = Some(hash);
                    self.loaded_ms = now_ms;
                }
                self.error = None;
                self.failed_hash = None;
            }
            Err(e) => {
                let hash = std::fs::read(&p)
                    .ok()
                    .map(|b| <[u8; 32]>::from(Sha256::digest(&b)));
                if self.error.as_deref() != Some(&e) || hash != self.failed_hash {
                    eprintln!("[cog-bridge] allowlist reload failed, keeping the old list: {e}");
                }
                self.error = Some(e);
                self.failed_hash = hash;
            }
        }
    }

    pub fn nodes(&self) -> usize {
        self.allow.len()
    }

    /// One-line description of the active mode, for the start-up log and `/status`.
    pub fn mode(&self) -> &'static str {
        match (self.path.is_some(), self.token.is_empty(), self.allow_token) {
            (true, false, true) => "signed+deprecated-token",
            (true, _, _) => "signed",
            (false, false, true) => "deprecated-token",
            (false, false, false) => "locked",
            (false, true, _) => "open",
        }
    }

    /// Allowlist health for `/status` (privileged callers only).
    pub fn info(&self, now_ms: u64) -> serde_json::Value {
        serde_json::json!({
            "allowlist_keys": self.allow.len(),
            "allowlist_error": self.error,
            "allowlist_age_s": now_ms.saturating_sub(self.loaded_ms) / 1000,
            "clock_synced": now_ms >= CLOCK_FLOOR_MS,
        })
    }

    /// Phase 1 (under the lock): refresh, cheap checks, key lookup.
    pub fn plan(&mut self, r: &AuthReq, now_ms: u64) -> Result<Plan, Refusal> {
        self.refresh(now_ms);
        if now_ms >= CLOCK_FLOOR_MS && self.started_ms < CLOCK_FLOOR_MS {
            // The clock was unset at start; "process start" is when time first became real.
            self.started_ms = now_ms;
        }
        if r.node.is_some() || r.sig.is_some() {
            return self
                .plan_signed(r, now_ms)
                .map(|p| Plan::Verify(Box::new(p)));
        }
        if self.path.is_some() {
            return self
                .legacy_token(r, Refusal::MissingSignature)
                .map(Plan::Done);
        }
        if self.token.is_empty() {
            return Ok(Plan::Done(None));
        }
        self.legacy_token(r, Refusal::TokenDisabled).map(Plan::Done)
    }

    fn legacy_token(&self, r: &AuthReq, disabled: Refusal) -> Result<Option<String>, Refusal> {
        if self.token.is_empty() || !self.allow_token {
            return Err(disabled);
        }
        match r.token {
            Some(t) if ct_eq(t, &self.token) => Ok(None),
            _ => Err(Refusal::BadToken),
        }
    }

    fn plan_signed(&self, r: &AuthReq, now_ms: u64) -> Result<Pending, Refusal> {
        let (Some(node), Some(ts), Some(nonce), Some(sig)) = (r.node, r.ts, r.nonce, r.sig) else {
            return Err(Refusal::MissingSignature);
        };
        if now_ms < CLOCK_FLOOR_MS {
            return Err(Refusal::ClockNotSet);
        }
        if !valid_node_name(node) {
            return Err(Refusal::Malformed("bad node name".into()));
        }
        let bad_ts = || Refusal::Malformed("timestamp must be unix milliseconds".into());
        if ts.is_empty() || ts.len() > 16 || !ts.bytes().all(|c| c.is_ascii_digit()) {
            return Err(bad_ts());
        }
        let ts_ms: u64 = ts.parse().map_err(|_| bad_ts())?;
        if ts_ms > MAX_TS_MS {
            return Err(Refusal::Malformed("timestamp out of range".into()));
        }
        if nonce.len() < 16 || nonce.len() > 64 || !nonce.bytes().all(|c| c.is_ascii_alphanumeric())
        {
            return Err(Refusal::Malformed(
                "nonce must be 16-64 alphanumerics".into(),
            ));
        }
        let sig_bytes: [u8; 64] = from_hex(sig)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| Refusal::Malformed("signature must be 128 hex characters".into()))?;
        let vk = *self.allow.get(node).ok_or(Refusal::UnknownNode)?;
        let skew = i128::from(now_ms) - i128::from(ts_ms);
        if skew.unsigned_abs() > u128::from(self.window_ms) {
            let skew_ms = skew.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
            return Err(Refusal::Stale {
                skew_ms,
                window_ms: self.window_ms,
            });
        }
        if ts_ms < self.started_ms {
            return Err(Refusal::PredatesStart);
        }
        Ok(Pending {
            node: node.to_string(),
            ts_ms,
            nonce: nonce.to_string(),
            vk,
            sig: Signature::from_bytes(&sig_bytes),
            msg: signing_string(r.method, r.path, node, ts_ms, nonce, r.body),
        })
    }

    /// Phase 3 (under the lock, after the signature verified): replay, rate, nonce budgets.
    pub fn commit(&mut self, p: &Pending, now_ms: u64) -> Result<String, Refusal> {
        self.nonces.retain(|_, exp| *exp >= now_ms);
        let key = (p.node.clone(), p.nonce.clone());
        // A replay is refused before it can spend the node's rate budget, so replaying a
        // captured request cannot starve the real node.
        if self.nonces.contains_key(&key) {
            return Err(Refusal::Replay);
        }
        // Capacity is checked before a rate token is spent, so a refusal for a full cache
        // does not also cost the node budget.
        let mine = self.nonces.keys().filter(|(n, _)| *n == p.node).count();
        if mine >= NODE_NONCE_CAP || self.nonces.len() >= NONCE_CAP {
            return Err(Refusal::CacheFull);
        }
        let bucket = self
            .rates
            .entry(p.node.clone())
            .or_insert_with(|| Bucket::full(NODE_BURST, now_ms));
        if !bucket.take(NODE_RATE, NODE_BURST, now_ms) {
            return Err(Refusal::RateLimited);
        }
        self.nonces
            .insert(key, p.ts_ms.saturating_add(self.window_ms));
        Ok(p.node.clone())
    }

    /// All three phases in one call, for single-threaded use (tests). The server uses
    /// [`authorize_shared`] so the signature check runs outside the lock.
    #[cfg(test)]
    pub fn authorize(&mut self, r: &AuthReq, now_ms: u64) -> Result<Option<String>, Refusal> {
        match self.plan(r, now_ms)? {
            Plan::Done(n) => Ok(n),
            Plan::Verify(p) => {
                p.verify()?;
                self.commit(&p, now_ms).map(Some)
            }
        }
    }
}

/// Authorises one request against shared state, holding the lock only for the cheap phases.
/// `Ok(Some(node))` is a verified identity, `Ok(None)` an accepted legacy request.
pub fn authorize_shared(
    m: &Mutex<Auth>,
    r: &AuthReq,
    now_ms: u64,
) -> Result<Option<String>, Refusal> {
    let plan = m.lock().map_err(|_| Refusal::CacheFull)?.plan(r, now_ms)?;
    match plan {
        Plan::Done(n) => Ok(n),
        Plan::Verify(p) => {
            p.verify()?;
            m.lock()
                .map_err(|_| Refusal::CacheFull)?
                .commit(&p, now_ms)
                .map(Some)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_790_000_000_000;

    fn key(n: u8) -> SigningKey {
        SigningKey::from_bytes(&[n; 32])
    }

    fn auth_with(nodes: &[(&str, &SigningKey)]) -> Auth {
        // Started before NOW so the "predates start" rule does not fire by accident.
        let mut a = Auth::new(None, 60, "", false, NOW - 1_000_000).unwrap();
        for (n, k) in nodes {
            a.allow.insert((*n).into(), k.verifying_key());
        }
        a
    }

    fn signed(k: &SigningKey, node: &str, body: &[u8], ts: u64, nonce: &str) -> [String; 4] {
        let h = sign_headers(k, node, "POST", "/ingest", body, ts, nonce);
        let v = |i: usize| h[i].split_once(": ").unwrap().1.to_string();
        [v(0), v(1), v(2), v(3)]
    }

    fn req<'a>(h: &'a [String; 4], body: &'a [u8]) -> AuthReq<'a> {
        AuthReq {
            method: "POST",
            path: "/ingest",
            body,
            node: Some(&h[0]),
            ts: Some(&h[1]),
            nonce: Some(&h[2]),
            sig: Some(&h[3]),
            token: None,
        }
    }

    fn run(
        a: &mut Auth,
        h: &[String; 4],
        body: &[u8],
        now: u64,
    ) -> Result<Option<String>, Refusal> {
        a.authorize(&req(h, body), now)
    }

    fn nonce(i: u32) -> String {
        format!("{i:032x}")
    }

    #[test]
    fn valid_signed_request_is_accepted() {
        let k = key(1);
        let mut a = auth_with(&[("pi5", &k)]);
        let h = signed(&k, "pi5", b"{}", NOW, &nonce(1));
        assert_eq!(run(&mut a, &h, b"{}", NOW + 500), Ok(Some("pi5".into())));
    }

    #[test]
    fn unknown_key_is_refused() {
        let (good, evil) = (key(1), key(2));
        let mut a = auth_with(&[("pi5", &good)]);
        let h = signed(&evil, "mallory", b"{}", NOW, &nonce(1));
        assert_eq!(run(&mut a, &h, b"{}", NOW), Err(Refusal::UnknownNode));
    }

    #[test]
    fn known_node_with_wrong_key_has_bad_signature() {
        let (good, evil) = (key(1), key(2));
        let mut a = auth_with(&[("pi5", &good)]);
        let h = signed(&evil, "pi5", b"{}", NOW, &nonce(1));
        assert_eq!(run(&mut a, &h, b"{}", NOW), Err(Refusal::BadSignature));
    }

    #[test]
    fn tampered_body_is_refused() {
        let k = key(1);
        let mut a = auth_with(&[("pi5", &k)]);
        let h = signed(&k, "pi5", b"{\"a\":1}", NOW, &nonce(1));
        assert_eq!(
            run(&mut a, &h, b"{\"a\":2}", NOW),
            Err(Refusal::BadSignature)
        );
    }

    #[test]
    fn stale_and_future_timestamps_are_refused() {
        let k = key(1);
        let mut a = auth_with(&[("pi5", &k)]);
        let h = signed(&k, "pi5", b"{}", NOW, &nonce(1));
        let r = run(&mut a, &h, b"{}", NOW + 61_000);
        assert!(matches!(r, Err(Refusal::Stale { .. })), "{r:?}");
        let h2 = signed(&k, "pi5", b"{}", NOW + 120_000, &nonce(2));
        assert!(matches!(
            run(&mut a, &h2, b"{}", NOW),
            Err(Refusal::Stale { .. })
        ));
        let h3 = signed(&k, "pi5", b"{}", NOW, &nonce(3));
        assert!(run(&mut a, &h3, b"{}", NOW + 59_000).is_ok());
    }

    #[test]
    fn timestamp_extremes_cannot_overflow() {
        let k = key(1);
        let mut a = auth_with(&[("pi5", &k)]);
        for ts in [
            "18446744073709551615",
            "9999999999999999",
            "4102444800001",
            "-5",
            "+5",
            "",
        ] {
            let mut h = signed(&k, "pi5", b"{}", NOW, &nonce(1));
            h[1] = ts.to_string();
            assert!(
                matches!(run(&mut a, &h, b"{}", NOW), Err(Refusal::Malformed(_))),
                "{ts}"
            );
        }
        // The largest allowed ts against a huge "now" is a clean Stale, not an overflow.
        let h = signed(&k, "pi5", b"{}", MAX_TS_MS, &nonce(2));
        assert!(matches!(
            run(&mut a, &h, b"{}", u64::MAX),
            Err(Refusal::Stale { .. })
        ));
        let h = signed(&k, "pi5", b"{}", 0, &nonce(3));
        assert!(matches!(
            run(&mut a, &h, b"{}", u64::MAX),
            Err(Refusal::Stale { .. })
        ));
    }

    #[test]
    fn requests_signed_before_process_start_are_refused() {
        let k = key(1);
        let mut a = auth_with(&[("pi5", &k)]);
        a.started_ms = NOW + 10_000; // "restarted" 10 s after the request was signed
        let h = signed(&k, "pi5", b"{}", NOW, &nonce(1));
        assert_eq!(
            run(&mut a, &h, b"{}", NOW + 20_000),
            Err(Refusal::PredatesStart)
        );
        let fresh = signed(&k, "pi5", b"{}", NOW + 20_000, &nonce(2));
        assert!(run(&mut a, &fresh, b"{}", NOW + 20_000).is_ok());
    }

    #[test]
    fn signed_requests_wait_for_a_real_clock_and_start_time_is_taken_then() {
        let k = key(1);
        let mut a = Auth::new(None, 60, "", false, 5_000).unwrap(); // booted at ~1970, no RTC
        a.allow.insert("pi5".into(), k.verifying_key());
        let h = signed(&k, "pi5", b"{}", 6_000, &nonce(1));
        assert_eq!(run(&mut a, &h, b"{}", 6_000), Err(Refusal::ClockNotSet));
        assert_eq!(Refusal::ClockNotSet.status(), 503);
        assert_eq!(a.info(6_000)["clock_synced"], false);
        // NTP sets the clock: a request signed before the jump is refused as pre-start (the
        // sender retries), one signed after it is accepted.
        let old = signed(&k, "pi5", b"{}", NOW - 1_000, &nonce(2));
        assert_eq!(run(&mut a, &old, b"{}", NOW), Err(Refusal::PredatesStart));
        let fresh = signed(&k, "pi5", b"{}", NOW + 1, &nonce(3));
        assert!(run(&mut a, &fresh, b"{}", NOW + 1).is_ok());
        assert_eq!(a.info(NOW)["clock_synced"], true);
    }

    #[test]
    fn a_full_nonce_cache_does_not_spend_rate_budget() {
        let k = key(1);
        let mut a = auth_with(&[("pi5", &k)]);
        for i in 0..NODE_NONCE_CAP as u32 {
            a.nonces.insert(("pi5".into(), nonce(i)), NOW + 60_000);
        }
        for i in 0..40u32 {
            let h = signed(&k, "pi5", b"{}", NOW, &nonce(10_000 + i));
            assert_eq!(run(&mut a, &h, b"{}", NOW), Err(Refusal::CacheFull));
        }
        a.nonces.clear();
        let h = signed(&k, "pi5", b"{}", NOW, &nonce(99_999));
        assert!(
            run(&mut a, &h, b"{}", NOW).is_ok(),
            "full burst still available"
        );
    }

    #[test]
    fn replayed_nonce_is_refused_but_a_fresh_one_passes() {
        let k = key(1);
        let mut a = auth_with(&[("pi5", &k)]);
        let h = signed(&k, "pi5", b"{}", NOW, &nonce(1));
        assert!(run(&mut a, &h, b"{}", NOW).is_ok());
        assert_eq!(run(&mut a, &h, b"{}", NOW + 10), Err(Refusal::Replay));
        let h2 = signed(&k, "pi5", b"{}", NOW, &nonce(2));
        assert!(run(&mut a, &h2, b"{}", NOW + 10).is_ok());
    }

    #[test]
    fn replays_do_not_spend_the_nodes_rate_budget() {
        let k = key(1);
        let mut a = auth_with(&[("pi5", &k)]);
        let h = signed(&k, "pi5", b"{}", NOW, &nonce(1));
        assert!(run(&mut a, &h, b"{}", NOW).is_ok());
        for _ in 0..200 {
            assert_eq!(run(&mut a, &h, b"{}", NOW), Err(Refusal::Replay));
        }
        let h2 = signed(&k, "pi5", b"{}", NOW, &nonce(2));
        assert!(
            run(&mut a, &h2, b"{}", NOW).is_ok(),
            "the real node is not starved"
        );
    }

    #[test]
    fn nonces_expire_with_the_window_so_the_cache_stays_bounded() {
        let k = key(1);
        let mut a = auth_with(&[("pi5", &k)]);
        for i in 0..12u32 {
            let h = signed(&k, "pi5", b"{}", NOW, &nonce(i));
            run(&mut a, &h, b"{}", NOW).unwrap();
        }
        assert_eq!(a.nonces.len(), 12);
        let h = signed(&k, "pi5", b"{}", NOW + 200_000, &nonce(999));
        run(&mut a, &h, b"{}", NOW + 200_000).unwrap();
        assert_eq!(a.nonces.len(), 1);
    }

    #[test]
    fn a_node_is_rate_limited_and_cannot_crowd_out_another() {
        let (ka, kb) = (key(1), key(2));
        let mut a = auth_with(&[("noisy", &ka), ("quiet", &kb)]);
        let mut limited = 0;
        for i in 0..100u32 {
            let h = signed(&ka, "noisy", b"{}", NOW, &nonce(i));
            if run(&mut a, &h, b"{}", NOW) == Err(Refusal::RateLimited) {
                limited += 1;
            }
        }
        assert!(limited >= 80, "limited {limited}");
        let h = signed(&kb, "quiet", b"{}", NOW, &nonce(1));
        assert!(run(&mut a, &h, b"{}", NOW).is_ok());
    }

    #[test]
    fn per_node_nonce_cap_holds_and_spares_other_nodes() {
        let (ka, kb) = (key(1), key(2));
        let mut a = auth_with(&[("noisy", &ka), ("quiet", &kb)]);
        for i in 0..NODE_NONCE_CAP as u32 {
            a.nonces.insert(("noisy".into(), nonce(i)), NOW + 60_000);
        }
        let h = signed(&ka, "noisy", b"{}", NOW, &nonce(9_999));
        assert_eq!(run(&mut a, &h, b"{}", NOW), Err(Refusal::CacheFull));
        let h = signed(&kb, "quiet", b"{}", NOW, &nonce(1));
        assert!(run(&mut a, &h, b"{}", NOW).is_ok());
    }

    #[test]
    fn partial_auth_headers_are_missing() {
        let k = key(1);
        let mut a = auth_with(&[("pi5", &k)]);
        let r = AuthReq {
            method: "POST",
            path: "/ingest",
            node: Some("pi5"),
            ..Default::default()
        };
        assert_eq!(a.authorize(&r, NOW), Err(Refusal::MissingSignature));
    }

    #[test]
    fn modes_token_is_deprecated_and_gated_by_flag() {
        let tok = |t| AuthReq {
            method: "POST",
            path: "/ingest",
            token: t,
            ..Default::default()
        };
        let mut a = Auth::new(None, 60, "s3cret", false, NOW).unwrap();
        assert_eq!(a.mode(), "locked");
        assert_eq!(
            a.authorize(&tok(Some("s3cret")), NOW),
            Err(Refusal::TokenDisabled)
        );
        let mut a = Auth::new(None, 60, "s3cret", true, NOW).unwrap();
        assert_eq!(a.mode(), "deprecated-token");
        assert_eq!(a.authorize(&tok(Some("s3cret")), NOW), Ok(None));
        assert_eq!(a.authorize(&tok(Some("nope")), NOW), Err(Refusal::BadToken));
        assert_eq!(a.authorize(&tok(None), NOW), Err(Refusal::BadToken));
        let mut a = Auth::new(None, 60, "", false, NOW).unwrap();
        assert_eq!(a.mode(), "open");
        assert_eq!(a.authorize(&tok(None), NOW), Ok(None));
    }

    #[test]
    fn allowlist_parsing() {
        let k = key(3);
        let pk = to_hex(k.verifying_key().as_bytes());
        let ok = parse_allowlist(&format!("# nodes\npi5 {pk}  # lab pi\n\nopi {pk}\n")).unwrap();
        assert_eq!(ok.len(), 2);
        assert!(parse_allowlist("pi5").is_err());
        assert!(parse_allowlist("pi5 zz").is_err());
        assert!(parse_allowlist(&format!("pi5 {pk}\npi5 {pk}")).is_err());
        assert!(parse_allowlist(&format!("bad/name {pk}")).is_err());
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bridge-auth-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_private(p: &std::path::Path, txt: &str) {
        std::fs::write(p, txt).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    fn line(n: &str, k: &SigningKey) -> String {
        format!("{n} {}\n", to_hex(k.verifying_key().as_bytes()))
    }

    #[test]
    fn allowlist_reloads_on_content_change_even_with_same_size_and_mtime() {
        let dir = tmpdir("reload");
        let p = dir.join("nodes.allow");
        let (k1, k2) = (key(1), key(2));
        write_private(&p, &line("pi5", &k1));
        let mut a = Auth::new(p.to_str(), 60, "", false, NOW - 100_000).unwrap();
        assert_eq!(a.nodes(), 1);
        // Swap the key for another of identical size, then restore the old mtime.
        let mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
        write_private(&p, &line("pi5", &k2));
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        let h = signed(&k2, "pi5", b"{}", NOW, &nonce(1));
        assert!(
            a.authorize(&req(&h, b"{}"), NOW).is_ok(),
            "new key honoured"
        );
        let h = signed(&k1, "pi5", b"{}", NOW, &nonce(2));
        assert_eq!(
            a.authorize(&req(&h, b"{}"), NOW),
            Err(Refusal::BadSignature)
        );
        // Revocation: removing the node takes effect.
        write_private(&p, "# empty\n");
        let h = signed(&k2, "pi5", b"{}", NOW + 2_000, &nonce(3));
        assert_eq!(
            a.authorize(&req(&h, b"{}"), NOW + 2_000),
            Err(Refusal::UnknownNode)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_bad_allowlist_edit_keeps_old_keys_and_is_reported() {
        let dir = tmpdir("bad-edit");
        let p = dir.join("nodes.allow");
        let k = key(1);
        write_private(&p, &line("pi5", &k));
        let mut a = Auth::new(p.to_str(), 60, "", false, NOW).unwrap();
        write_private(&p, "pi5 not-hex\n");
        a.refresh(NOW + 2_000);
        assert_eq!(a.nodes(), 1, "old list kept");
        let info = a.info(NOW + 3_000);
        assert!(
            info["allowlist_error"].as_str().unwrap().contains("pubkey"),
            "{info}"
        );
        assert_eq!(info["allowlist_keys"], 1);
        write_private(&p, &(line("pi5", &k) + &line("opi", &k)));
        a.refresh(NOW + 4_000);
        assert!(a.info(NOW + 4_000)["allowlist_error"].is_null());
        assert_eq!(a.nodes(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn loose_permissions_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmpdir("perms");
        let key_path = dir.join("node.key");
        keygen("pi5", key_path.to_str().unwrap()).unwrap();
        assert!(load_key(key_path.to_str().unwrap()).is_ok());
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(load_key(key_path.to_str().unwrap())
            .unwrap_err()
            .contains("chmod 600"));

        let al = dir.join("nodes.allow");
        write_private(&al, "");
        assert!(Auth::new(al.to_str(), 60, "", false, NOW).is_ok());
        std::fs::set_permissions(&al, std::fs::Permissions::from_mode(0o666)).unwrap();
        let err = Auth::new(al.to_str(), 60, "", false, NOW).err().unwrap();
        assert!(err.contains("writable"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn keygen_writes_a_loadable_key_and_refuses_to_overwrite() {
        let dir = tmpdir("keygen");
        let p = dir.join("node.key");
        let l = keygen("pi5", p.to_str().unwrap()).unwrap();
        let loaded = load_key(p.to_str().unwrap()).unwrap();
        assert_eq!(
            l,
            format!("pi5 {}", to_hex(loaded.verifying_key().as_bytes()))
        );
        assert!(keygen("pi5", p.to_str().unwrap()).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
