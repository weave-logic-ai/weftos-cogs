//! The ADR-106 start-time licence check in weft-cog-host.
//!
//! A configured licence directory means this Seed intends to be bound. The
//! host then asks the local daemon. It does not decide the grant. No
//! directory means the gate does not apply. A directory that cannot be read
//! fails closed and does not open a socket.
//!
//! - `not_seed_bound` from the daemon, while the directory is configured, is
//!   `binding_inactive` until a binding is imported.
//! - A permit whose BLAKE3 is not the file that was hashed is `malformed_reply`.
//! - Transport failures use `daemon_unavailable`, `malformed_reply`, `timeout`,
//!   and `version_mismatch`. Grant refusals keep the daemon's spellings.
//!
//! A lapse refuses the next start and leaves a running cog alone. A revoked
//! artifact hash also stops a running cog (`RunGate::revoked`, every tick).
//! On a transport error that poll returns false, so a daemon blip does not
//! kill a cog that already started. A denial fails closed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use weftos_cog_protocol::{CallError, CheckRunParams, CheckRunResult, RefusalCode, DEFAULT_TIMEOUT, PROTOCOL};

pub use weftos_cog_protocol::MAX_IMPORT_RECORDS;

use crate::CogRecord;
use crate::Source;

#[path = "licence_backend.rs"]
mod backend;

pub(crate) use backend::LicenceBackend;
#[cfg(unix)]
use backend::DaemonLicence;

/// Overrides the licence directory.
pub const LICENCE_DIR_ENV: &str = "WEFT_COG_HOST_LICENCE_DIR";
/// `{"mesh_id": "<64 lower-case hex>"}`.
pub const CONFIG_FILE: &str = "config.json";
const MAX_CONFIG_BYTES: u64 = 256 * 1024;
/// Largest import body (file or `POST /licence/records`).
pub const MAX_IMPORT_BYTES: usize = 2 * 1024 * 1024;

/// The licence directory: `$WEFT_COG_HOST_LICENCE_DIR`, else `<root>/.licence`.
pub fn default_licence_dir(root: &Path) -> PathBuf {
    match std::env::var(LICENCE_DIR_ENV) {
        Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => root.join(".licence"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostConfig {
    mesh_id: String,
}

enum State {
    Unconfigured,
    Broken(String),
    Ready,
}

type Fingerprint = Vec<Option<(u64, SystemTime)>>;

/// The host's licence directory and the daemon it asks.
pub struct HostLicence {
    dir: PathBuf,
    state: RwLock<(Fingerprint, Arc<State>)>,
    daemon_socket: Option<PathBuf>,
    daemon_timeout: Duration,
    backend: Option<Arc<dyn LicenceBackend>>,
}

/// Signed records for [`HostLicence::import`]. The daemon verifies them.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Records {
    /// Operator-signed Seed binding.
    #[serde(default)]
    pub binding: Option<Value>,
    /// Grants, withdrawals included.
    #[serde(default)]
    pub grants: Vec<Value>,
    /// Operator hash approvals.
    #[serde(default)]
    pub approvals: Vec<Value>,
    /// Operator revocation notices.
    #[serde(default)]
    pub revocations: Vec<Value>,
}

/// What one imported record did.
#[derive(Debug, Clone, Serialize)]
pub struct ImportLine {
    /// `binding`, `grant`, `approval` or `revocation`.
    pub kind: &'static str,
    /// `applied`, `duplicate`, `ignored` or `refused`.
    pub outcome: String,
    /// Why it was refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A start the gate allowed. `blake3` is the file that was hashed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPermit {
    pub grant_id: String,
    pub approval_id: String,
    /// BLAKE3 the daemon echoed. The supervisor stores the file hash separately.
    pub blake3: String,
}

/// What a permitted Cognitum start ran, for the revocation sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LicensedStart {
    /// BLAKE3 of the bytes that were started.
    pub blake3: String,
    /// The grant and approval that covered them.
    pub permit: RunPermit,
}

/// Answer of [`RunGate::check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunAnswer {
    /// The gate does not apply.
    NotSeedBound,
    /// Grant and approval cover the binary.
    Permit {
        grant_id: String,
        approval_id: String,
        blake3: String,
    },
}

/// A refusal the supervisor can print. `shown` is the operator sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRefusal {
    pub code: &'static str,
    pub shown: String,
}

/// What the supervisor asks. Tests implement this with fixed answers.
pub trait RunGate: Send + Sync {
    fn check(&self, cog_id: &str, version: &str, sha256: &str, blake3: &str) -> Result<RunAnswer, GateRefusal>;
    fn claims(&self, sha256: &str, blake3: &str) -> bool;
    fn revoked(&self, blake3: &str) -> bool;
}

/// A start the gate refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRefused {
    pub code: &'static str,
    pub reason: String,
}

impl std::fmt::Display for StartRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
    let len = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?.len();
    if len > MAX_CONFIG_BYTES {
        return Err(format!("{} is too large", path.display()));
    }
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn dir_present(dir: &Path) -> Result<bool, String> {
    match std::fs::metadata(dir) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match std::fs::symlink_metadata(dir) {
            Ok(_) => Err(format!("{}: dangling link", dir.display())),
            Err(e2) if e2.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e2) => Err(format!("{}: {e2}", dir.display())),
        },
        Err(e) => Err(format!("{}: {e}", dir.display())),
    }
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn load(dir: &Path) -> State {
    match dir_present(dir) {
        Ok(false) => return State::Unconfigured,
        Err(e) => return State::Broken(e),
        Ok(true) => {}
    }
    let cfg = read_capped(&dir.join(CONFIG_FILE))
        .and_then(|bytes| serde_json::from_slice::<HostConfig>(&bytes).map_err(|e| format!("{CONFIG_FILE}: {e}")));
    match cfg {
        Ok(cfg) if is_hex64(&cfg.mesh_id) => State::Ready,
        Ok(_) => State::Broken(format!("{CONFIG_FILE}: mesh_id must be 64 hex characters")),
        Err(e) => State::Broken(e),
    }
}

fn fingerprint(dir: &Path) -> Fingerprint {
    [dir.join(CONFIG_FILE), dir.to_path_buf()]
        .into_iter()
        .map(|path| std::fs::metadata(path).ok().map(|meta| (meta.len(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH))))
        .collect()
}

fn binding_inactive(inner: impl std::fmt::Display) -> GateRefusal {
    GateRefusal {
        code: "binding_inactive",
        shown: format!("no Seed binding is in effect on this node ({inner})"),
    }
}

fn transport(code: &'static str, shown: impl Into<String>) -> GateRefusal {
    GateRefusal { code, shown: shown.into() }
}

fn shown_for_denial(code: RefusalCode, message: String) -> String {
    match code {
        RefusalCode::BindingInactive if !message.starts_with("no Seed binding is in effect") => {
            format!("no Seed binding is in effect on this node ({message})")
        }
        RefusalCode::NotHolder if !message.starts_with("Cognitum cogs on this machine") => {
            format!("Cognitum cogs on this machine run under its licence holder, not this daemon ({message})")
        }
        _ => message,
    }
}

fn from_call(err: CallError) -> GateRefusal {
    match err {
        CallError::DaemonUnavailable(message) => transport("daemon_unavailable", message),
        CallError::MalformedReply(message) => transport("malformed_reply", message),
        CallError::Timeout => transport("timeout", "the daemon did not answer before the deadline"),
        CallError::VersionMismatch(message) => transport("version_mismatch", message),
        CallError::Denial { code, message } => GateRefusal { code: code.as_str(), shown: shown_for_denial(code, message) },
    }
}

/// Operator remedy for a refusal code. The daemon decides; the host prints the matching sentence.
fn remedy_for(code: &str, cog_id: &str, version: &str) -> String {
    match code {
        "binding_inactive" => "check `weaver workload node status`".into(),
        "no_grant" | "not_in_grant" => format!("weaver cog checkout {cog_id}@{version} --arch <arch>"),
        "grant_lapsed" => "renew through the steward (a lapsed licence stops new starts)".into(),
        "hash_revoked" => "the artifact was revoked; it cannot run".into(),
        "no_approval" => format!("weaver cog checkout approve {cog_id}@{version}"),
        "not_holder" => {
            format!("place {cog_id}@{version} from the cluster owner's daemon (the machine's licence holder)")
        }
        _ => "start the local WeftOS daemon and retry the licence check".into(),
    }
}

impl HostLicence {
    /// Open the licence directory.
    pub fn open(dir: PathBuf) -> Self {
        let fp = fingerprint(&dir);
        let state = Arc::new(load(&dir));
        Self {
            dir,
            state: RwLock::new((fp, state)),
            daemon_socket: None,
            daemon_timeout: DEFAULT_TIMEOUT,
            backend: None,
        }
    }

    /// Ask this socket instead of `$WEFTOS_RUNTIME_DIR/kernel.sock`.
    pub fn with_daemon_socket(mut self, socket: impl Into<PathBuf>) -> Self {
        self.daemon_socket = Some(socket.into());
        self
    }

    /// How long one daemon call waits.
    pub fn with_daemon_timeout(mut self, timeout: Duration) -> Self {
        self.daemon_timeout = timeout;
        self
    }

    /// Use `backend` instead of opening a socket. Tests pass a fixed answer.
    pub fn with_backend(mut self, backend: Arc<dyn LicenceBackend>) -> Self {
        self.backend = Some(backend);
        self
    }

    /// The licence directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn state(&self) -> Arc<State> {
        Arc::clone(&self.state.read().unwrap_or_else(|poison| poison.into_inner()).1)
    }

    /// Re-read when `config.json` changes. The supervisor calls this every tick.
    pub fn refresh(&self) {
        let fp = fingerprint(&self.dir);
        let mut guard = self.state.write().unwrap_or_else(|poison| poison.into_inner());
        if guard.0 != fp {
            *guard = (fp, Arc::new(load(&self.dir)));
        }
    }

    fn backend_for_call(&self) -> Result<Arc<dyn LicenceBackend>, CallError> {
        if let Some(backend) = &self.backend {
            return Ok(Arc::clone(backend));
        }
        #[cfg(unix)]
        {
            let socket = self.resolve_socket()?;
            Ok(Arc::new(DaemonLicence { socket, timeout: self.daemon_timeout }))
        }
        #[cfg(not(unix))]
        {
            Err(backend::non_unix_socket(&self.daemon_socket, self.daemon_timeout))
        }
    }

    #[cfg(unix)]
    fn resolve_socket(&self) -> Result<PathBuf, CallError> {
        if let Some(path) = &self.daemon_socket {
            return Ok(path.clone());
        }
        match std::env::var("WEFTOS_RUNTIME_DIR") {
            Ok(dir) if !dir.trim().is_empty() => Ok(PathBuf::from(dir.trim()).join("kernel.sock")),
            _ => Err(CallError::DaemonUnavailable(
                "WEFTOS_RUNTIME_DIR is unset and no daemon socket was configured".into(),
            )),
        }
    }

    /// Forward signed records. The daemon verifies them and writes its store.
    pub fn import(&self, recs: &Records) -> Result<Vec<ImportLine>, String> {
        if recs.grants.len() > MAX_IMPORT_RECORDS
            || recs.approvals.len() > MAX_IMPORT_RECORDS
            || recs.revocations.len() > MAX_IMPORT_RECORDS
        {
            return Err(format!("at most {MAX_IMPORT_RECORDS} records of each kind per import"));
        }
        if !dir_present(&self.dir)? {
            return Err(format!(
                "no licence directory at {}: create it and write {CONFIG_FILE} before importing",
                self.dir.display()
            ));
        }
        self.refresh();
        match &*self.state() {
            State::Broken(e) => return Err(format!("licence state unusable: {e}")),
            State::Unconfigured => return Err("licence state is not configured".into()),
            State::Ready => {}
        }
        let params = import_params(recs)?;
        let backend = self.backend_for_call().map_err(|e| e.to_string())?;
        backend.import_records(&params).map_err(|e| e.to_string())
    }

    /// `GET /licence` and `weft-cog-host licence status`.
    ///
    /// A configured directory whose daemon is down is `broken` in this
    /// document. The in-memory state stays ready, and the next start still
    /// asks the daemon.
    pub fn status(&self) -> Value {
        let dir = self.dir.display().to_string();
        match &*self.state() {
            State::Unconfigured => json!({"state": "unconfigured", "dir": dir}),
            State::Broken(e) => json!({"state": "broken", "dir": dir, "error": e}),
            State::Ready => match self.backend_for_call().and_then(|backend| backend.licence_status()) {
                Ok(mut value) => {
                    let Some(obj) = value.as_object_mut() else {
                        return json!({"state": "broken", "dir": dir, "error": "licence status was not an object"});
                    };
                    obj.remove("protocol");
                    obj.insert("state".into(), json!("ready"));
                    obj.insert("dir".into(), json!(dir));
                    value
                }
                Err(e) => json!({"state": "broken", "dir": dir, "error": e.to_string()}),
            },
        }
    }

    fn check_ready(&self, cog_id: &str, version: &str, sha256: &str, blake3: &str) -> Result<RunAnswer, GateRefusal> {
        let params = CheckRunParams::new(cog_id, version, sha256, blake3)
            .map_err(|e| transport("malformed_reply", e.to_string()))?;
        let backend = self.backend_for_call().map_err(from_call)?;
        match backend.check_run(&params) {
            Ok(CheckRunResult::NotSeedBound) => Err(binding_inactive(format!(
                "the licence directory {} is configured but holds no binding yet: import the signed binding first",
                self.dir.display()
            ))),
            Ok(CheckRunResult::Permit { grant_id, approval_id, blake3: permit_blake3 }) => {
                if permit_blake3 != blake3 {
                    return Err(transport("malformed_reply", "permit blake3 does not match the request"));
                }
                Ok(RunAnswer::Permit { grant_id, approval_id, blake3: permit_blake3 })
            }
            Err(e) => Err(from_call(e)),
        }
    }
}

fn import_params(recs: &Records) -> Result<Value, String> {
    if recs.binding.as_ref().is_some_and(|value| !value.is_object()) {
        return Err("binding must be an object".into());
    }
    for (name, rows) in [("grant", &recs.grants), ("approval", &recs.approvals), ("revocation", &recs.revocations)] {
        if rows.iter().any(|row| !row.is_object()) {
            return Err(format!("{name} must be an object"));
        }
    }
    let mut params = serde_json::to_value(recs).map_err(|e| e.to_string())?;
    let obj = params.as_object_mut().expect("records object");
    if obj.get("binding").is_some_and(Value::is_null) {
        obj.remove("binding");
    }
    obj.insert("protocol".into(), json!(PROTOCOL));
    Ok(params)
}

impl RunGate for HostLicence {
    fn check(&self, cog_id: &str, version: &str, sha256: &str, blake3: &str) -> Result<RunAnswer, GateRefusal> {
        match &*self.state() {
            State::Unconfigured => Ok(RunAnswer::NotSeedBound),
            State::Broken(e) => Err(binding_inactive(format!("licence state unusable: {e}"))),
            State::Ready => self.check_ready(cog_id, version, sha256, blake3),
        }
    }

    fn claims(&self, sha256: &str, blake3: &str) -> bool {
        if !matches!(&*self.state(), State::Ready) {
            return false;
        }
        let Ok(backend) = self.backend_for_call() else {
            return true;
        };
        backend.claims(sha256, blake3).unwrap_or(true)
    }

    fn revoked(&self, blake3: &str) -> bool {
        if !matches!(&*self.state(), State::Ready) {
            return false;
        }
        let Ok(backend) = self.backend_for_call() else {
            return false;
        };
        match backend.revoked(blake3) {
            Ok((revoked, _)) => revoked,
            Err(CallError::Denial { .. }) => true,
            Err(_) => false,
        }
    }
}

/// The (sha256, BLAKE3) of a binary, lower-case hex, from its bytes.
pub fn hashes(bytes: &[u8]) -> (String, String) {
    (weftos_cog_repo::sha256_hex(bytes), blake3::hash(bytes).to_hex().to_string())
}

/// Decide whether `rec` may start with these `bytes`.
pub fn check_start(gate: &dyn RunGate, rec: &CogRecord, bytes: &[u8]) -> Result<Option<LicensedStart>, StartRefused> {
    let (sha256, blake3) = hashes(bytes);
    check_start_hashed(gate, rec, &sha256, &blake3)
}

/// [`check_start`] for hashes already computed from the bytes that will run.
pub fn check_start_hashed(
    gate: &dyn RunGate,
    rec: &CogRecord,
    sha256: &str,
    blake3: &str,
) -> Result<Option<LicensedStart>, StartRefused> {
    if rec.source != Source::Cognitum && !gate.claims(sha256, blake3) {
        return Ok(None);
    }
    match gate.check(cog_id_of(rec), &rec.version, sha256, blake3) {
        Ok(RunAnswer::NotSeedBound) => Ok(None),
        Ok(RunAnswer::Permit { grant_id, approval_id, blake3: permit_blake3 }) => Ok(Some(LicensedStart {
            blake3: blake3.to_string(),
            permit: RunPermit { grant_id, approval_id, blake3: permit_blake3 },
        })),
        Err(refusal) => Err(start_refused(&refusal, rec, sha256)),
    }
}

fn cog_id_of(rec: &CogRecord) -> &str {
    &rec.id
}

fn start_refused(refusal: &GateRefusal, rec: &CogRecord, sha256: &str) -> StartRefused {
    let remedy = remedy_for(refusal.code, &rec.id, &rec.version);
    let prefix = &sha256[..sha256.len().min(16)];
    StartRefused {
        code: refusal.code,
        reason: format!(
            "licence run gate: [{}] {} ({} {}, sha256 {prefix}); remedy: {remedy}",
            refusal.code, refusal.shown, rec.id, rec.version
        ),
    }
}

#[cfg(all(test, unix))]
#[path = "licence_daemon_tests.rs"]
mod daemon_tests;

#[cfg(test)]
#[path = "licence_tests.rs"]
mod tests;
