//! Start-time licence checks against fake gates, and host overrides against a fake daemon.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use weftos_cog_protocol::{CallError, CheckRunParams, CheckRunResult, RefusalCode};

use super::*;
use crate::supervise::Supervisor;

struct FakeGate {
    answer: Mutex<Result<RunAnswer, GateRefusal>>,
    claims: bool,
    revoked: Mutex<Option<String>>,
    asked: Mutex<Vec<(String, String, String, String)>>,
}

impl FakeGate {
    fn new(answer: Result<RunAnswer, GateRefusal>) -> Self {
        Self {
            answer: Mutex::new(answer),
            claims: false,
            revoked: Mutex::new(None),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn set(&self, answer: Result<RunAnswer, GateRefusal>) {
        *self.answer.lock().unwrap() = answer;
    }

    fn asked(&self) -> usize {
        self.asked.lock().unwrap().len()
    }
}

impl RunGate for FakeGate {
    fn check(&self, cog_id: &str, version: &str, sha256: &str, blake3: &str) -> Result<RunAnswer, GateRefusal> {
        self.asked.lock().unwrap().push((cog_id.into(), version.into(), sha256.into(), blake3.into()));
        self.answer.lock().unwrap().clone()
    }

    fn claims(&self, _: &str, _: &str) -> bool {
        self.claims
    }

    fn revoked(&self, blake3: &str) -> bool {
        self.revoked.lock().unwrap().as_deref() == Some(blake3)
    }
}

fn permit() -> RunAnswer {
    RunAnswer::Permit { grant_id: "g-1".into(), approval_id: "a-1".into(), blake3: "ignored".into() }
}

fn refused(code: &'static str) -> GateRefusal {
    GateRefusal { code, shown: code.into() }
}

fn rec(source: Source) -> CogRecord {
    CogRecord {
        id: "fall-detect".into(),
        version: "1.2.0".into(),
        source,
        enabled: false,
        binary: "cog-fall-detect-arm".into(),
        args: vec![],
        signed: false,
    }
}

#[test]
fn a_non_cognitum_cog_never_reaches_the_gate() {
    let gate = FakeGate::new(Err(refused("no_grant")));
    assert_eq!(check_start(&gate, &rec(Source::WeaveLogic), b"bytes"), Ok(None));
    assert_eq!(check_start(&gate, &rec(Source::Local), b"bytes"), Ok(None));
    assert_eq!(gate.asked(), 0);
}

#[test]
fn known_cognitum_bytes_relabelled_local_are_still_gated() {
    let mut gate = FakeGate::new(Err(refused("no_approval")));
    gate.claims = true;
    let err = check_start(&gate, &rec(Source::Local), b"bytes").unwrap_err();
    assert_eq!(err.code, "no_approval");
}

#[test]
fn the_gate_sees_hashes_of_the_bytes_and_the_record_identity() {
    let gate = FakeGate::new(Ok(permit()));
    let lic = check_start(&gate, &rec(Source::Cognitum), b"the binary").unwrap().unwrap();
    let (sha, blake3) = hashes(b"the binary");
    assert_eq!(gate.asked.lock().unwrap()[0], ("fall-detect".into(), "1.2.0".into(), sha, blake3.clone()));
    assert_eq!(lic.blake3, blake3);
    assert_eq!(lic.permit.grant_id, "g-1");
    assert_eq!(lic.permit.blake3, "ignored");
}

#[test]
fn not_seed_bound_lets_a_cognitum_cog_start() {
    let gate = FakeGate::new(Ok(RunAnswer::NotSeedBound));
    assert_eq!(check_start(&gate, &rec(Source::Cognitum), b"x"), Ok(None));
}

#[test]
fn every_refusal_keeps_the_run_gate_code() {
    let cases = ["binding_inactive", "no_grant", "grant_lapsed", "not_in_grant", "hash_revoked", "no_approval"];
    for code in cases {
        let gate = FakeGate::new(Err(refused(code)));
        let err = check_start(&gate, &rec(Source::Cognitum), b"x").unwrap_err();
        assert_eq!(err.code, code);
        assert!(err.reason.starts_with(&format!("licence run gate: [{code}]")), "{}", err.reason);
        assert!(err.reason.contains("remedy:"), "{}", err.reason);
    }
}

#[cfg(unix)]
fn sleeper(root: &Path, source: Source) {
    use std::os::unix::fs::PermissionsExt;
    let dir = root.join("fall-detect");
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("cog-fall-detect-arm");
    std::fs::write(&bin, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    crate::save_record(root, &rec(source)).unwrap();
}

#[test]
#[cfg(unix)]
fn a_refused_start_does_not_spawn_and_reports_the_code() {
    let root = tempfile::tempdir().unwrap();
    sleeper(root.path(), Source::Cognitum);
    let gate = Arc::new(FakeGate::new(Err(refused("no_grant"))));
    let mut supervisor = Supervisor::new(root.path().to_path_buf());
    supervisor.set_licence_gate(gate.clone());
    let err = supervisor.start("fall-detect").unwrap_err();
    assert!(err.contains("[no_grant]"), "{err}");
    let status = &supervisor.status()[0];
    assert!(!status.running && status.enabled);
    assert_eq!(status.licence_refusal.as_deref(), Some("no_grant"));

    gate.set(Ok(permit()));
    supervisor.start("fall-detect").unwrap();
    let status = &supervisor.status()[0];
    assert!(status.running && status.licence_refusal.is_none());
    assert_eq!(status.licence_grant.as_deref(), Some("g-1"));
    supervisor.stop_all_running();
}

#[test]
#[cfg(unix)]
fn a_revoked_hash_stops_a_running_cog_and_a_lapse_does_not() {
    let root = tempfile::tempdir().unwrap();
    sleeper(root.path(), Source::Cognitum);
    let gate = Arc::new(FakeGate::new(Ok(permit())));
    let mut supervisor = Supervisor::new(root.path().to_path_buf());
    supervisor.set_licence_gate(gate.clone());
    supervisor.start("fall-detect").unwrap();

    gate.set(Err(refused("grant_lapsed")));
    supervisor.tick();
    assert!(supervisor.status()[0].running);

    let bytes = std::fs::read(root.path().join("fall-detect/cog-fall-detect-arm")).unwrap();
    *gate.revoked.lock().unwrap() = Some(hashes(&bytes).1);
    gate.set(Err(refused("hash_revoked")));
    supervisor.tick();
    let status = &supervisor.status()[0];
    assert!(!status.running, "torn down");
    assert_eq!(status.licence_refusal.as_deref(), Some("hash_revoked"));
    assert!(supervisor.start("fall-detect").unwrap_err().contains("[hash_revoked]"));
    supervisor.stop_all_running();
}

#[cfg(unix)]
struct SwapGate {
    bin: std::path::PathBuf,
}

#[cfg(unix)]
impl RunGate for SwapGate {
    fn check(&self, _: &str, _: &str, _: &str, _: &str) -> Result<RunAnswer, GateRefusal> {
        std::fs::write(&self.bin, "#!/bin/sh\necho swapped > ran.txt\nsleep 30\n").unwrap();
        Ok(permit())
    }

    fn claims(&self, _: &str, _: &str) -> bool {
        false
    }

    fn revoked(&self, _: &str) -> bool {
        false
    }
}

#[test]
#[cfg(unix)]
fn the_bytes_that_were_hashed_are_the_bytes_that_run() {
    let root = tempfile::tempdir().unwrap();
    sleeper(root.path(), Source::Cognitum);
    let bin = root.path().join("fall-detect/cog-fall-detect-arm");
    std::fs::write(&bin, "#!/bin/sh\necho original > ran.txt\nsleep 30\n").unwrap();
    let mut supervisor = Supervisor::new(root.path().to_path_buf());
    supervisor.set_licence_gate(Arc::new(SwapGate { bin: bin.clone() }));
    supervisor.start("fall-detect").unwrap();
    assert!(std::fs::read_to_string(&bin).unwrap().contains("swapped"));
    let ran = root.path().join("fall-detect/ran.txt");
    for _ in 0..100 {
        if ran.exists() && !std::fs::read_to_string(&ran).unwrap().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(std::fs::read_to_string(&ran).unwrap().trim(), "original");
    let run_dir = root.path().join(".run");
    if let Ok(rd) = std::fs::read_dir(&run_dir) {
        use std::os::unix::fs::PermissionsExt;
        let copies: Vec<_> = rd.flatten().collect();
        assert_eq!(copies.len(), 1);
        assert_eq!(copies[0].metadata().unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&run_dir).unwrap().permissions().mode() & 0o777, 0o700);
        supervisor.stop("fall-detect").unwrap();
        let _fresh = Supervisor::new(root.path().to_path_buf());
        assert!(!run_dir.exists());
    } else {
        supervisor.stop("fall-detect").unwrap();
    }
}

#[test]
#[cfg(unix)]
fn a_dangling_or_unreadable_licence_dir_fails_closed_not_open() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(".licence");
    std::os::unix::fs::symlink(root.path().join("nowhere"), &dir).unwrap();
    let lic = HostLicence::open(dir);
    assert_eq!(lic.status()["state"], "broken");
    let err = check_start(&lic, &rec(Source::Cognitum), b"bytes").unwrap_err();
    assert_eq!(err.code, "binding_inactive");
    assert!(lic.import(&Records::default()).unwrap_err().contains("dangling"));
}

#[test]
#[cfg(unix)]
fn without_a_gate_nothing_changes() {
    let root = tempfile::tempdir().unwrap();
    sleeper(root.path(), Source::Cognitum);
    let mut supervisor = Supervisor::new(root.path().to_path_buf());
    supervisor.start("fall-detect").unwrap();
    assert!(supervisor.status()[0].running && supervisor.status()[0].licence_refusal.is_none());
    supervisor.stop_all_running();
}

struct FakeBackend {
    check: Mutex<Result<CheckRunResult, CallError>>,
    imported: Mutex<Option<Value>>,
    lines: Mutex<Vec<ImportLine>>,
    status: Mutex<Result<Value, CallError>>,
    claims: Mutex<Result<bool, CallError>>,
    revoked: Mutex<Result<(bool, u64), CallError>>,
}

impl LicenceBackend for FakeBackend {
    fn check_run(&self, _: &CheckRunParams) -> Result<CheckRunResult, CallError> {
        self.check.lock().unwrap().clone()
    }

    fn import_records(&self, records: &Value) -> Result<Vec<ImportLine>, CallError> {
        *self.imported.lock().unwrap() = Some(records.clone());
        Ok(self.lines.lock().unwrap().clone())
    }

    fn licence_status(&self) -> Result<Value, CallError> {
        self.status.lock().unwrap().clone()
    }

    fn claims(&self, _: &str, _: &str) -> Result<bool, CallError> {
        self.claims.lock().unwrap().clone()
    }

    fn revoked(&self, _: &str) -> Result<(bool, u64), CallError> {
        self.revoked.lock().unwrap().clone()
    }
}

fn fake_backend() -> Arc<FakeBackend> {
    Arc::new(FakeBackend {
        check: Mutex::new(Ok(CheckRunResult::NotSeedBound)),
        imported: Mutex::new(None),
        lines: Mutex::new(vec![ImportLine { kind: "binding", outcome: "applied".into(), error: None }]),
        status: Mutex::new(Ok(json!({"protocol": "weftos.cog.v1", "mesh_id": null}))),
        claims: Mutex::new(Ok(false)),
        revoked: Mutex::new(Ok((false, 0))),
    })
}

fn configured(dir: &Path, backend: Arc<FakeBackend>) -> HostLicence {
    let lic_dir = dir.join(".licence");
    std::fs::create_dir_all(&lic_dir).unwrap();
    std::fs::write(lic_dir.join(CONFIG_FILE), format!(r#"{{"mesh_id":"{}"}}"#, "ab".repeat(32))).unwrap();
    HostLicence::open(lic_dir).with_backend(backend)
}

#[test]
fn an_unconfigured_directory_does_not_ask_the_daemon() {
    let root = tempfile::tempdir().unwrap();
    let lic = HostLicence::open(root.path().join(".licence"));
    assert_eq!(lic.status()["state"], "unconfigured");
    assert_eq!(check_start(&lic, &rec(Source::Cognitum), b"x"), Ok(None));
    assert!(!RunGate::claims(&lic, "ab", "cd"));
    assert!(!RunGate::revoked(&lic, "cd"));
}

#[test]
fn import_limits_reject_unknown_fields_and_a_missing_directory() {
    let root = tempfile::tempdir().unwrap();
    let lic = HostLicence::open(root.path().join(".licence"));
    let many = Records { approvals: vec![json!({}); MAX_IMPORT_RECORDS + 1], ..Records::default() };
    assert!(lic.import(&many).unwrap_err().contains("512"));
    assert!(serde_json::from_str::<Records>(r#"{"bogus": 1}"#).is_err());
    assert!(lic.import(&Records::default()).unwrap_err().contains("no licence directory"));
}

#[test]
fn a_configured_not_seed_bound_reply_asks_for_the_binding() {
    let root = tempfile::tempdir().unwrap();
    let backend = fake_backend();
    let lic = configured(root.path(), Arc::clone(&backend));
    let err = check_start(&lic, &rec(Source::Cognitum), b"bytes").unwrap_err();
    assert_eq!(err.code, "binding_inactive");
    assert!(err.reason.contains("no binding yet"), "{}", err.reason);
}

#[test]
fn a_permit_whose_blake3_is_not_the_file_is_malformed() {
    let root = tempfile::tempdir().unwrap();
    let backend = fake_backend();
    *backend.check.lock().unwrap() = Ok(CheckRunResult::Permit {
        grant_id: "g-1".into(),
        approval_id: "a-1".into(),
        blake3: "ab".repeat(32),
    });
    let lic = configured(root.path(), backend);
    let err = check_start(&lic, &rec(Source::Cognitum), b"bytes").unwrap_err();
    assert_eq!(err.code, "malformed_reply");
}

#[test]
fn import_forwards_the_envelope_and_keeps_the_daemon_lines() {
    let root = tempfile::tempdir().unwrap();
    let backend = fake_backend();
    let lic = configured(root.path(), Arc::clone(&backend));
    let recs = Records { binding: Some(json!({"payload":"{}"})), ..Records::default() };
    let lines = lic.import(&recs).unwrap();
    assert_eq!(lines[0].kind, "binding");
    assert_eq!(lines[0].outcome, "applied");
    let sent = backend.imported.lock().unwrap().clone().unwrap();
    assert_eq!(sent["protocol"], "weftos.cog.v1");
    assert!(sent["binding"].is_object());
    assert!(sent.get("grants").unwrap().as_array().unwrap().is_empty());
}

#[test]
fn a_daemon_error_on_claims_still_runs_the_check_and_a_blip_does_not_revoke() {
    let root = tempfile::tempdir().unwrap();
    let backend = fake_backend();
    *backend.claims.lock().unwrap() = Err(CallError::DaemonUnavailable("down".into()));
    *backend.revoked.lock().unwrap() = Err(CallError::Timeout);
    *backend.check.lock().unwrap() = Err(CallError::Denial {
        code: RefusalCode::NoGrant,
        message: "no checkout grant is held for this cog version".into(),
    });
    let lic = configured(root.path(), Arc::clone(&backend));
    let err = check_start(&lic, &rec(Source::Local), b"bytes").unwrap_err();
    assert_eq!(err.code, "no_grant");
    assert!(err.reason.contains("remedy:"), "{}", err.reason);
    assert!(!RunGate::revoked(&lic, &"cd".repeat(32)));
    *backend.revoked.lock().unwrap() = Err(CallError::Denial {
        code: RefusalCode::BindingInactive,
        message: "revocation list unreadable: boom".into(),
    });
    assert!(RunGate::revoked(&lic, &"cd".repeat(32)));
}

#[test]
fn a_configured_host_reports_a_down_daemon_without_forgetting_it_is_configured() {
    static N: AtomicU64 = AtomicU64::new(0);
    let _ = N.fetch_add(1, Ordering::Relaxed);
    let root = tempfile::tempdir().unwrap();
    let backend = fake_backend();
    *backend.status.lock().unwrap() = Err(CallError::DaemonUnavailable("down".into()));
    let lic = configured(root.path(), backend);
    let status = lic.status();
    assert_eq!(status["state"], "broken");
    assert!(status["error"].as_str().unwrap().contains("down"), "{status}");
    assert_eq!(check_start(&lic, &rec(Source::Cognitum), b"x").unwrap_err().code, "binding_inactive");
}

#[test]
fn uppercase_mesh_id_is_broken_and_does_not_ask() {
    let root = tempfile::tempdir().unwrap();
    let lic_dir = root.path().join(".licence");
    std::fs::create_dir_all(&lic_dir).unwrap();
    std::fs::write(lic_dir.join(CONFIG_FILE), format!(r#"{{"mesh_id":"{}"}}"#, "AB".repeat(32))).unwrap();
    let lic = HostLicence::open(lic_dir);
    assert_eq!(lic.status()["state"], "broken");
    assert!(lic.status()["error"].as_str().unwrap().contains("64 hex"));
}
