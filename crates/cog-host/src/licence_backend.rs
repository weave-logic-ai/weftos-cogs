//! What Cog Host asks the local daemon, and nothing else.
//!
//! Classified before the `clawft-kernel` calls were removed:
//!
//! | Use | Direction |
//! |---|---|
//! | Licence decision | Already `cog.check_run`. This module does not copy `check_run`. |
//! | Mesh operations | No host call. No mesh RPC is added. |
//! | ECC operations | No host call. No ECC RPC is added. `mesh_keys` keeps its own Ed25519 verify. |
//! | Imports | The host parses the product envelope and forwards it. Signature checks and store writes are `cog.licence.import` on the daemon. |
//! | Trust anchors and claims | `cog.licence.claims` and `cog.licence.status`. WeftOS stays the authority. |
//! | Revoked-hash polling | `cog.licence.revoked`, a versioned query. Not an event subscription, and not a copied list. |
//! | Shared structs | Wire types stay in `cog-protocol`. Kernel structs are not moved here. |
//!
//! There is no generic kernel RPC. Production calls go through [`DaemonLicence`].
//! Tests pass a [`LicenceBackend`] that returns fixed answers.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use cog_protocol::{
    CallError, CheckRunParams, CheckRunResult, CLAIMS_METHOD, IMPORT_METHOD, PROTOCOL, REVOKED_METHOD,
    STATUS_METHOD,
};
#[cfg(unix)]
use cog_protocol::DaemonCheck;

use super::ImportLine;

/// The daemon calls Cog Host makes. Governance stays on the daemon.
pub trait LicenceBackend: Send + Sync {
    /// `cog.check_run`.
    fn check_run(&self, params: &CheckRunParams) -> Result<CheckRunResult, CallError>;
    /// `cog.licence.import`. `records` already carries `protocol`.
    fn import_records(&self, records: &Value) -> Result<Vec<ImportLine>, CallError>;
    /// `cog.licence.status`, including its `protocol` field.
    fn licence_status(&self) -> Result<Value, CallError>;
    /// `cog.licence.claims`.
    fn claims(&self, sha256: &str, blake3: &str) -> Result<bool, CallError>;
    /// `cog.licence.revoked`: the flag and the list generation.
    fn revoked(&self, blake3: &str) -> Result<(bool, u64), CallError>;
}

/// One Unix-socket client. Built at the call so a socket override still wins.
#[cfg(unix)]
pub(super) struct DaemonLicence {
    pub(super) socket: PathBuf,
    pub(super) timeout: Duration,
}

#[cfg(unix)]
impl DaemonLicence {
    fn call(&self, id: &str, method: &str, params: &Value, auth: Option<&str>) -> Result<Value, CallError> {
        DaemonCheck::new(&self.socket)
            .with_timeout(self.timeout)
            .with_id(id)
            .call_result(method, params, auth)
    }
}

#[cfg(unix)]
impl LicenceBackend for DaemonLicence {
    fn check_run(&self, params: &CheckRunParams) -> Result<CheckRunResult, CallError> {
        DaemonCheck::new(&self.socket).with_timeout(self.timeout).check(params)
    }

    fn import_records(&self, records: &Value) -> Result<Vec<ImportLine>, CallError> {
        let result = self.call("cog-licence-import-1", IMPORT_METHOD, records, Some("admin"))?;
        import_lines(&result)
    }

    fn licence_status(&self) -> Result<Value, CallError> {
        self.call("cog-licence-status-1", STATUS_METHOD, &json!({"protocol": PROTOCOL}), None)
    }

    fn claims(&self, sha256: &str, blake3: &str) -> Result<bool, CallError> {
        let result = self.call(
            "cog-licence-claims-1",
            CLAIMS_METHOD,
            &json!({"protocol": PROTOCOL, "sha256": sha256, "blake3": blake3}),
            None,
        )?;
        result
            .get("claims")
            .and_then(Value::as_bool)
            .ok_or_else(|| CallError::MalformedReply("claims result has no claims flag".into()))
    }

    fn revoked(&self, blake3: &str) -> Result<(bool, u64), CallError> {
        let result = self.call(
            "cog-licence-revoked-1",
            REVOKED_METHOD,
            &json!({"protocol": PROTOCOL, "blake3": blake3}),
            None,
        )?;
        let revoked = result
            .get("revoked")
            .and_then(Value::as_bool)
            .ok_or_else(|| CallError::MalformedReply("revoked result has no revoked flag".into()))?;
        let generation = result
            .get("list_generation")
            .and_then(Value::as_u64)
            .ok_or_else(|| CallError::MalformedReply("revoked result has no list_generation".into()))?;
        Ok((revoked, generation))
    }
}

#[cfg(unix)]
fn import_lines(result: &Value) -> Result<Vec<ImportLine>, CallError> {
    let rows = result
        .get("lines")
        .and_then(Value::as_array)
        .ok_or_else(|| CallError::MalformedReply("import result has no lines".into()))?;
    rows.iter()
        .map(|row| {
            let kind = match row.get("kind").and_then(Value::as_str).unwrap_or("") {
                "binding" => "binding",
                "grant" => "grant",
                "approval" => "approval",
                "revocation" => "revocation",
                other => {
                    return Err(CallError::MalformedReply(format!("unknown import kind {other}")));
                }
            };
            let outcome = row.get("outcome").and_then(Value::as_str).unwrap_or("refused").to_string();
            let error = row.get("error").and_then(Value::as_str).map(str::to_string);
            Ok(ImportLine { kind, outcome, error })
        })
        .collect()
}

/// Keep the socket fields live on a host that has no Unix socket.
#[cfg(not(unix))]
pub(super) fn non_unix_socket(socket: &Option<PathBuf>, timeout: Duration) -> CallError {
    let _ = (socket, timeout);
    CallError::DaemonUnavailable("cog.check_run needs a Unix socket".into())
}
