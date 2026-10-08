//! weft-cog-repo (COG-008): off-device WeaveLogic cog repository tooling.
//!
//!   weft-cog-repo sign   --from <dist> --out <repo> --key <priv.pem>   # sign binaries -> registry.json
//!   weft-cog-repo verify <repo-url-or-dir>                             # fetch + check every artifact
//!   weft-cog-repo install <repo-url-or-dir> <cog-id> --seed <host> [--arch arm|arm64]
//!
//! `sign` signs each cog binary with the WeaveLogic release key (PEM via `--key`, or the
//! `WEAVELOGIC_RELEASE_KEY` env var holding the raw 32-byte seed as hex) and writes `registry.json`.
//! `verify` and `install` pin the public key ([`cog_repo::WEAVELOGIC_PUBKEY_HEX`]) and refuse
//! any artifact that is unsigned, signed by another key, or whose sha256/size does not match —
//! signed-only, always. `install` sideloads the verified binary + its agent manifest onto a Seed
//! over SSH into `/var/lib/cognitum/apps/<id>/`.

use ed25519_dalek::pkcs8::DecodePrivateKey;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
mod private;
use cog_repo::{
    Artifact, COG_ARCHES, CogEntry, Registry, RevokedKeys, SCHEMA, VerifyError, check_signable,
    sha256_hex, verify_artifact_unrevoked, weavelogic_key,
};

type R<T> = Result<T, String>;

fn arg<'a>(a: &'a [String], f: &str) -> Option<&'a str> {
    a.iter()
        .position(|x| x == f)
        .and_then(|i| a.get(i + 1))
        .map(String::as_str)
}

fn usage() -> ! {
    eprintln!(
        "weft-cog-repo — WeaveLogic cog repository (COG-008), signed-only trust model\n\n\
         sign    --from <dist-dir> --out <repo-dir> [--key <priv.pem>] [--repo-name weavelogic]\n\
         \t\tSigns every cog in <dist-dir>/<id>/ (cog-<id>-arm, -arm64, -x86_64 + manifest.json) and writes <repo-dir>/registry.json.\n\
         \t\tKey: --key <pkcs8 pem>, or WEAVELOGIC_RELEASE_KEY=<32-byte seed hex>.\n\
         verify  <repo-url|repo-dir>\n\
         \t\tFetches registry.json and verifies sha256 + Ed25519 for every artifact against the pinned key.\n\
         check-matrix --from <dist-dir> --cogs <src/cogs> <repo-dir>\n\
         \t\tFails unless registry.json lists every cog under <src/cogs>, each with arm and arm64.\n\
         \t\tx86_64 is required when <dist-dir>/<id>/cog-<id>-x86_64 exists, and forbidden when it does not.\n\
         \t\tsign still succeeds when only one cog was signable; this command is the release check.\n\
         install <repo-url|repo-dir> <cog-id> --seed <user@host> [--arch arm|arm64] [--apps-dir <path>]\n\
         \t\tVerifies the chosen cog, then sideloads the binary + manifest to the Seed (default arch arm).\n\
         \t\tx86_64 is a host registry key. Seed install does not sideload it.\n\n\
         Private repositories (ADR-105): a project's own signed COG-008 repo, signed with a key it holds.\n\
         init    <repo-dir> --name <repo-name>\n\
         keygen  --out <key.pem> [--repo <repo-dir>]\n\
         \t\tWrites an Ed25519 key (PKCS#8 PEM, mode 0600, never overwrites) and prints the public key. Never commit it.\n\
         add     <repo-dir> --binary <file> [--id <id>] [--arch arm|arm64|x86_64] [--cog-toml <cog.toml>] [--manifest <manifest.json>]\n\
         \t\t[--name N] [--version V] [--category C] [--description D] [--hardware a,b]   stage a cog into <repo-dir>/dist/\n\
         sign    <repo-dir> --key <key.pem>      sign dist/ into <repo-dir>/repo/registry.json (key must match repo.toml)\n\
         verify  <repo-dir>                      verify <repo-dir>/repo against the key in repo.toml\n\
         publish <repo-dir> --to <dir>           verify, then copy repo/ to <dir> (host that directory yourself)\n\
         Both `verify` and `install` also take --pin <pubkey-hex> (repeatable) to check a repo against your own key.\n\
         They also refuse a key on the operator's signer-key revocation list: --revocations <file>, else revoked_subjects.json in the runtime dir weaver resolves ($WEFTOS_RUNTIME_DIR, the project's .weftos/runtime, or ~/.clawft); fails if neither it nor $HOME is known.\n\n\
         The pinned WeaveLogic release key is compiled in; there is no way to disable verification."
    );
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let res = match args.get(1).map(String::as_str) {
        Some("sign") => cmd_sign(&args[2..]),
        Some("verify") => cmd_verify(&args[2..]),
        Some("check-matrix") => cmd_check_matrix(&args[2..]),
        Some("install") => cmd_install(&args[2..]),
        Some("init") => private::cmd_init(&args[2..]),
        Some("keygen") => private::cmd_keygen(&args[2..]),
        Some("add") => private::cmd_add(&args[2..]),
        Some("publish") => private::cmd_publish(&args[2..]),
        _ => usage(),
    };
    if let Err(e) = res {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

// ---- signing key ----------------------------------------------------------

fn load_signing_key(args: &[String]) -> R<SigningKey> {
    if let Some(pem_path) = arg(args, "--key") {
        let pem =
            std::fs::read_to_string(pem_path).map_err(|e| format!("read key {pem_path}: {e}"))?;
        return SigningKey::from_pkcs8_pem(&pem).map_err(|e| format!("parse pkcs8 pem: {e}"));
    }
    if let Ok(hexseed) = std::env::var("WEAVELOGIC_RELEASE_KEY") {
        let seed: [u8; 32] = hex::decode(hexseed.trim())
            .map_err(|e| format!("WEAVELOGIC_RELEASE_KEY not hex: {e}"))?
            .try_into()
            .map_err(|_| "WEAVELOGIC_RELEASE_KEY must be 32 bytes (64 hex chars)".to_string())?;
        return Ok(SigningKey::from_bytes(&seed));
    }
    Err("no signing key: pass --key <priv.pem> or set WEAVELOGIC_RELEASE_KEY=<seed hex>".into())
}

// ---- sign -----------------------------------------------------------------

/// Reads a cog's agent `manifest.json` (as produced by seed-sideload) for the registry metadata.
fn meta_from_manifest(manifest: &serde_json::Value, id: &str) -> CogEntry {
    let s = |k: &str| {
        manifest
            .get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let hw = manifest
        .get("hardware_requirement")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    CogEntry {
        id: id.to_string(),
        name: if s("name").is_empty() {
            id.to_string()
        } else {
            s("name")
        },
        version: if s("version").is_empty() {
            "0.0.0".into()
        } else {
            s("version")
        },
        category: s("category"),
        description: s("description"),
        hardware_requirement: hw,
        artifacts: BTreeMap::new(),
    }
}

fn cmd_sign(args: &[String]) -> R<()> {
    // `sign <repo-dir> --key k.pem`: a private repository (ADR-105), guarded by its repo.toml key.
    if let Some(dir) = args.first().filter(|a| !a.starts_with("--")) {
        return private::cmd_sign(dir, &args[1..]);
    }
    let from = arg(args, "--from").ok_or("sign needs --from <dist-dir>")?;
    let out = arg(args, "--out").ok_or("sign needs --out <repo-dir>")?;
    let repo_name = arg(args, "--repo-name").unwrap_or("weavelogic");
    let key = load_signing_key(args)?;

    // Guard: the signing key must match the pinned public key, or installs would reject everything.
    if key.verifying_key().to_bytes() != weavelogic_key().to_bytes() {
        return Err("the signing key does not match the pinned WeaveLogic public key; installs would reject it".into());
    }
    sign_tree(from, Path::new(out), repo_name, &key)
}

/// Signs every cog under `<from>/<id>/` into the repo at `out` and writes `registry.json`.
/// The caller has already checked the key against whatever it must match.
fn sign_tree(from: &str, out: &Path, repo_name: &str, key: &SigningKey) -> R<()> {
    let out = out.to_path_buf();
    let mut cogs: Vec<CogEntry> = Vec::new();

    let mut ids: Vec<String> = std::fs::read_dir(from)
        .map_err(|e| format!("read {from}: {e}"))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    ids.sort();

    for id in &ids {
        let dist = Path::new(from).join(id);
        let manifest_path = dist.join("manifest.json");
        if !manifest_path.exists() {
            eprintln!("  skip {id}: no manifest.json");
            continue;
        }
        let manifest_bytes =
            std::fs::read(&manifest_path).map_err(|e| format!("read {manifest_path:?}: {e}"))?;
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| format!("parse {manifest_path:?}: {e}"))?;
        let mut entry = meta_from_manifest(&manifest, id);

        for (arch, suffix) in COG_ARCHES {
            let bin = dist.join(format!("cog-{id}{suffix}"));
            if !bin.exists() {
                continue;
            }
            let bytes = std::fs::read(&bin).map_err(|e| format!("read {bin:?}: {e}"))?;
            check_signable(&bytes).map_err(|e| format!("{}: {e}", bin.display()))?;
            let sig = key.sign(&bytes);
            let rel_bin = format!("cogs/{arch}/cog-{id}{suffix}");
            let rel_manifest = format!("cogs/{arch}/manifest-{id}.json");
            // stage files into the repo
            let dst_bin = out.join(&rel_bin);
            std::fs::create_dir_all(dst_bin.parent().unwrap())
                .map_err(|e| format!("mkdir: {e}"))?;
            std::fs::write(&dst_bin, &bytes).map_err(|e| format!("write {dst_bin:?}: {e}"))?;
            std::fs::write(out.join(&rel_manifest), &manifest_bytes)
                .map_err(|e| format!("write manifest: {e}"))?;
            entry.artifacts.insert(
                arch.to_string(),
                Artifact {
                    path: rel_bin,
                    size: bytes.len() as u64,
                    sha256: sha256_hex(&bytes),
                    sig: hex::encode(sig.to_bytes()),
                    manifest_path: rel_manifest,
                },
            );
            eprintln!("  signed {id} [{arch}] ({} bytes)", bytes.len());
        }
        if entry.artifacts.is_empty() {
            eprintln!("  skip {id}: no cog-{id}-arm / -arm64 / -x86_64 binaries");
            continue;
        }
        cogs.push(entry);
    }

    if cogs.is_empty() {
        return Err(format!(
            "no signable cogs found under {from} (need <id>/manifest.json and cog-<id>-arm, -arm64, or -x86_64)"
        ));
    }

    let registry = Registry {
        schema: SCHEMA,
        repo: repo_name.to_string(),
        updated: today(),
        cogs,
    };
    let json = serde_json::to_string_pretty(&registry).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&out).map_err(|e| format!("mkdir {out:?}: {e}"))?;
    std::fs::write(out.join("registry.json"), json)
        .map_err(|e| format!("write registry.json: {e}"))?;
    eprintln!(
        "wrote {}/registry.json with {} cog(s)",
        out.display(),
        registry.cogs.len()
    );
    Ok(())
}

/// Release completeness. `sign` returns success when one cog signed and skips a
/// directory that has no `manifest.json`. This compares the cog directories under
/// `cogs_dir` (each one contains `cog.toml`) with `registry` and with the files in
/// `from`. Every expected cog needs `arm` and `arm64`. `x86_64` is required exactly
/// when `cog-<id>-x86_64` is present.
fn check_release_matrix(from: &Path, cogs_dir: &Path, registry: &Registry) -> R<()> {
    let mut expected: Vec<String> = std::fs::read_dir(cogs_dir)
        .map_err(|e| format!("read {}: {e}", cogs_dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("cog.toml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    expected.sort();
    if expected.is_empty() {
        return Err(format!("no cogs under {}", cogs_dir.display()));
    }

    let mut problems = Vec::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for cog in &registry.cogs {
        *seen.entry(cog.id.clone()).or_insert(0) += 1;
    }
    for (id, n) in &seen {
        if *n > 1 {
            problems.push(format!("{id}: registry lists it {n} times"));
        }
    }

    for id in &expected {
        let dist = from.join(id);
        let manifest = dist.join("manifest.json");
        if !manifest.is_file() {
            problems.push(format!("{id}: no manifest.json"));
        } else {
            match std::fs::read(&manifest) {
                Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                    Ok(value) => {
                        let mid = value.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        if mid != id {
                            problems.push(format!("{id}: manifest id is {mid:?}"));
                        }
                    }
                    Err(e) => problems.push(format!("{id}: manifest.json: {e}")),
                },
                Err(e) => problems.push(format!("{id}: manifest.json: {e}")),
            }
        }
        for arch in ["arm", "arm64"] {
            let bin = dist.join(format!("cog-{id}-{arch}"));
            if !bin.is_file() {
                problems.push(format!("{id}: missing {}", bin.display()));
            }
        }
        let mut want = vec!["arm".to_string(), "arm64".to_string()];
        if dist.join(format!("cog-{id}-x86_64")).is_file() {
            want.push("x86_64".to_string());
        }
        match registry.cogs.iter().find(|c| &c.id == id) {
            None => problems.push(format!("{id}: not in registry.json")),
            Some(entry) => {
                let have: Vec<String> = entry.artifacts.keys().cloned().collect();
                if have != want {
                    problems.push(format!(
                        "{id}: registry arches [{}], expected [{}]",
                        have.join(","),
                        want.join(",")
                    ));
                }
            }
        }
    }
    for id in seen.keys() {
        if !expected.iter().any(|expected_id| expected_id == id) {
            problems.push(format!(
                "{id}: in registry.json but not under {}",
                cogs_dir.display()
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "release matrix ({}):\n{}",
            problems.len(),
            problems.join("\n")
        ))
    }
}

fn cmd_check_matrix(args: &[String]) -> R<()> {
    let from = arg(args, "--from").ok_or("check-matrix needs --from <dist-dir>")?;
    let cogs = arg(args, "--cogs").ok_or("check-matrix needs --cogs <cog-toml dir>")?;
    let repo = positional(args).ok_or("check-matrix needs <repo-dir>")?;
    let registry = load_registry(repo)?;
    check_release_matrix(Path::new(from), Path::new(cogs), &registry)?;
    eprintln!(
        "release matrix matches {} cog(s) in {repo}/registry.json",
        registry.cogs.len()
    );
    Ok(())
}

/// First argument that is not a flag and not the value of a flag this command takes.
fn positional(args: &[String]) -> Option<&str> {
    let mut skip_value = false;
    for a in args {
        if skip_value {
            skip_value = false;
            continue;
        }
        if a == "--from" || a == "--cogs" || a == "--key" || a == "--out" || a == "--repo-name" {
            skip_value = true;
            continue;
        }
        if !a.starts_with("--") {
            return Some(a);
        }
    }
    None
}

/// Date stamp for the registry. `Command` is allowed here (off-device CLI, not a workflow script).
fn today() -> String {
    Command::new("date")
        .args(["+%Y-%m-%d"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

// ---- fetching -------------------------------------------------------------

/// Reads a repository file by relative path, from an http(s) base URL or a local directory.
fn fetch(base: &str, rel: &str) -> R<Vec<u8>> {
    if base.starts_with("http://") || base.starts_with("https://") {
        let url = format!("{}/{}", base.trim_end_matches('/'), rel);
        let resp = reqwest::blocking::get(&url).map_err(|e| format!("GET {url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("GET {url}: HTTP {}", resp.status()));
        }
        Ok(resp
            .bytes()
            .map_err(|e| format!("body {url}: {e}"))?
            .to_vec())
    } else {
        let p = Path::new(base).join(rel);
        std::fs::read(&p).map_err(|e| format!("read {p:?}: {e}"))
    }
}

fn load_registry(base: &str) -> R<Registry> {
    let bytes = fetch(base, "registry.json")?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parse registry.json: {e}"))
}

// ---- verify ---------------------------------------------------------------

/// Keys a repo is checked against: every `--pin <hex>`, or the pinned WeaveLogic key when none.
fn pins(args: &[String]) -> R<Vec<VerifyingKey>> {
    let mut keys = explicit_pins(args)?;
    if keys.is_empty() {
        keys.push(weavelogic_key());
    }
    Ok(keys)
}

/// Only the keys given with `--pin` (empty when there are none).
fn explicit_pins(args: &[String]) -> R<Vec<VerifyingKey>> {
    let mut keys = Vec::new();
    for (i, a) in args.iter().enumerate() {
        if a == "--pin" {
            let hex_key = args
                .get(i + 1)
                .ok_or("--pin needs a public key (64 hex chars)")?;
            keys.push(private::parse_pubkey(hex_key)?);
        }
    }
    Ok(keys)
}

/// Verifies against each key; size and sha256 failures do not depend on the key, so only a
/// rejected signature or a revoked key moves on to the next one. A revoked key is reported as
/// such when no other key verifies.
fn verify_any(
    bytes: &[u8],
    art: &Artifact,
    keys: &[VerifyingKey],
    revoked: &RevokedKeys,
) -> Result<(), VerifyError> {
    let mut last = VerifyError::SignatureRejected;
    for k in keys {
        match verify_artifact_unrevoked(bytes, art, k, revoked) {
            Ok(()) => return Ok(()),
            Err(e @ (VerifyError::SignatureRejected | VerifyError::KeyRevoked)) => {
                if last != VerifyError::KeyRevoked {
                    last = e;
                }
            }
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// The operator's signer-key revocations: `--revocations <file>`, else the kernel's list in the
/// runtime dir (the same resolver `weaver` uses). Fail-closed: a list that exists but cannot be read stops the command; an
/// explicit `--revocations` file must exist.
fn revocations(args: &[String]) -> R<RevokedKeys> {
    match arg(args, "--revocations") {
        Some(p) => {
            if !Path::new(p).is_file() {
                return Err(format!("--revocations {p}: no such file"));
            }
            RevokedKeys::load(Path::new(p))
        }
        // Unit tests never read the real runtime dir.
        None if cfg!(test) => Ok(RevokedKeys::none()),
        None => RevokedKeys::load_default(),
    }
}

fn cmd_verify(args: &[String]) -> R<()> {
    let base = args.first().ok_or("verify needs <repo-url|repo-dir>")?;
    if Path::new(base).join("repo.toml").is_file() {
        // An explicit --pin wins over the repo's own declared key (and fails on a mismatch).
        return private::cmd_verify(base, &explicit_pins(args)?, &revocations(args)?);
    }
    let reg = load_registry(base)?;
    let key = pins(args)?;
    let revoked = revocations(args)?;
    eprintln!(
        "repo '{}' schema {} — {} cog(s)",
        reg.repo,
        reg.schema,
        reg.cogs.len()
    );
    let mut bad = 0u32;
    let mut ok = 0u32;
    for cog in &reg.cogs {
        for (arch, art) in &cog.artifacts {
            match fetch(base, &art.path)
                .and_then(|b| verify_any(&b, art, &key, &revoked).map_err(|e| e.to_string()))
            {
                Ok(()) => {
                    ok += 1;
                    println!(
                        "  OK    {} [{arch}] v{}  {}",
                        cog.id,
                        cog.version,
                        &art.sha256[..16]
                    );
                }
                Err(e) => {
                    bad += 1;
                    println!("  FAIL  {} [{arch}]: {e}", cog.id);
                }
            }
        }
    }
    eprintln!("{ok} verified, {bad} failed");
    if bad > 0 {
        return Err(format!("{bad} artifact(s) failed verification"));
    }
    Ok(())
}

// ---- install --------------------------------------------------------------

fn cmd_install(args: &[String]) -> R<()> {
    let base = args
        .first()
        .ok_or("install needs <repo-url|repo-dir> <cog-id>")?;
    let cog_id = args
        .get(1)
        .filter(|s| !s.starts_with("--"))
        .ok_or("install needs a <cog-id>")?;
    let seed = arg(args, "--seed").ok_or("install needs --seed <user@host>")?;
    let arch = arg(args, "--arch").unwrap_or("arm");
    if arch != "arm" && arch != "arm64" {
        return Err(format!(
            "Seed install accepts arm or arm64; {arch} is listed for a Linux host and is not sideloaded"
        ));
    }
    let apps_dir = arg(args, "--apps-dir").unwrap_or("/var/lib/cognitum/apps");

    let reg = load_registry(base)?;
    let cog = reg
        .cogs
        .iter()
        .find(|c| &c.id == cog_id)
        .ok_or_else(|| format!("cog '{cog_id}' not in registry"))?;
    let art = cog
        .artifacts
        .get(arch)
        .ok_or_else(|| format!("cog '{cog_id}' has no '{arch}' artifact"))?;

    // VERIFY before anything touches the Seed. Signed-only, always.
    let bytes = fetch(base, &art.path)?;
    verify_any(&bytes, art, &pins(args)?, &revocations(args)?)
        .map_err(|e| format!("refusing to install {cog_id}: {e}"))?;
    let manifest = fetch(base, &art.manifest_path)?;
    eprintln!(
        "verified {cog_id} [{arch}] v{} ({} bytes) — signed by a pinned key",
        cog.version,
        bytes.len()
    );

    // Stage locally, then sideload over ssh/scp into apps/<id>/.
    let staging = std::env::temp_dir().join(format!("weft-cog-install-{cog_id}"));
    std::fs::create_dir_all(&staging).map_err(|e| format!("staging: {e}"))?;
    let local_bin = staging.join(format!("cog-{cog_id}-{arch}"));
    let local_manifest = staging.join("manifest.json");
    std::fs::write(&local_bin, &bytes).map_err(|e| format!("stage bin: {e}"))?;
    std::fs::write(&local_manifest, &manifest).map_err(|e| format!("stage manifest: {e}"))?;

    let dest = format!("{apps_dir}/{cog_id}");
    let agent_bin = format!("cog-{cog_id}-arm"); // the agent always looks for cog-<id>-arm
    eprintln!("sideloading to {seed}:{dest}/ ...");
    run("ssh", &[seed, &format!("sudo mkdir -p {dest}")])?;
    scp(&local_bin, &format!("{seed}:/tmp/{agent_bin}"))?;
    scp(
        &local_manifest,
        &format!("{seed}:/tmp/manifest-{cog_id}.json"),
    )?;
    run(
        "ssh",
        &[
            seed,
            &format!(
                "sudo install -m 0755 /tmp/{agent_bin} {dest}/{agent_bin} && sudo install -m 0644 /tmp/manifest-{cog_id}.json {dest}/manifest.json && rm -f /tmp/{agent_bin} /tmp/manifest-{cog_id}.json"
            ),
        ],
    )?;
    eprintln!(
        "installed {cog_id} v{} to {seed}:{dest}/ — the agent will pick it up.",
        cog.version
    );
    Ok(())
}

fn run(cmd: &str, args: &[&str]) -> R<()> {
    let status = Command::new(cmd)
        .args(args)
        .status()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if !status.success() {
        return Err(format!("{cmd} {}: exit {status}", args.join(" ")));
    }
    Ok(())
}

fn scp(local: &Path, remote: &str) -> R<()> {
    run("scp", &[local.to_str().ok_or("bad local path")?, remote])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist_with(payload: &[u8]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let cog = d.path().join("dist/probe");
        std::fs::create_dir_all(&cog).unwrap();
        std::fs::write(
            cog.join("manifest.json"),
            br#"{"name":"probe","version":"0.1.0"}"#,
        )
        .unwrap();
        std::fs::write(cog.join("cog-probe-arm"), payload).unwrap();
        d
    }

    #[test]
    fn sign_tree_refuses_a_release_prefixed_payload_and_signs_an_elf() {
        let key = SigningKey::from_bytes(&[3; 32]);
        let d = dist_with(b"weftos-release-v1\n{\"schema\":1,\"kind\":\"weftos-release\"}");
        let from = d.path().join("dist");
        let e = sign_tree(from.to_str().unwrap(), &d.path().join("repo"), "t", &key).unwrap_err();
        assert!(e.contains("reserved"), "{e}");
        assert!(!d.path().join("repo/registry.json").exists());

        let d = dist_with(b"\x7fELF probe");
        let from = d.path().join("dist");
        sign_tree(from.to_str().unwrap(), &d.path().join("repo"), "t", &key).unwrap();
        assert!(d.path().join("repo/registry.json").is_file());
    }

    #[test]
    fn sign_tree_records_an_x86_64_artifact() {
        let key = SigningKey::from_bytes(&[3; 32]);
        let d = tempfile::tempdir().unwrap();
        let cog = d.path().join("dist/probe");
        std::fs::create_dir_all(&cog).unwrap();
        std::fs::write(
            cog.join("manifest.json"),
            br#"{"name":"probe","version":"0.1.0"}"#,
        )
        .unwrap();
        std::fs::write(cog.join("cog-probe-x86_64"), b"\x7fELF probe-x86").unwrap();
        sign_tree(
            d.path().join("dist").to_str().unwrap(),
            &d.path().join("repo"),
            "t",
            &key,
        )
        .unwrap();
        let reg: Registry =
            serde_json::from_slice(&std::fs::read(d.path().join("repo/registry.json")).unwrap())
                .unwrap();
        let art = reg.cogs[0]
            .artifacts
            .get("x86_64")
            .expect("x86_64 registry key");
        assert_eq!(art.path, "cogs/x86_64/cog-probe-x86_64");
        assert!(d.path().join("repo").join(&art.path).is_file());
        assert!(!reg.cogs[0].artifacts.contains_key("arm"));
    }

    fn elf(dir: &std::path::Path, id: &str, arch: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(format!("cog-{id}-{arch}")), b"\x7fELF bytes").unwrap();
    }

    fn cog_toml(dir: &std::path::Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("cog.toml"), b"[cog]\nid=\"x\"\n").unwrap();
    }

    fn manifest(dir: &std::path::Path, id: &str) {
        std::fs::write(
            dir.join("manifest.json"),
            format!(
                r#"{{"binary_size":9,"category":"signal","config":[],"description":"d","difficulty":"medium","id":"{id}","name":"{id}","sha256":"abc","size_kb":1,"version":"0.1.0"}}"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn meta_from_manifest_reads_the_sideload_shape() {
        let raw = br#"{"binary_size":9,"category":"signal","config":[],"description":"a room","difficulty":"medium","id":"bme280","name":"BME280 Environment","sha256":"abc","size_kb":1,"version":"0.1.0"}"#;
        let value: serde_json::Value = serde_json::from_slice(raw).unwrap();
        let entry = meta_from_manifest(&value, "bme280");
        assert_eq!(entry.name, "BME280 Environment");
        assert_eq!(entry.version, "0.1.0");
        assert_eq!(entry.category, "signal");
        assert_eq!(entry.description, "a room");
        assert!(entry.hardware_requirement.is_empty());
    }

    #[test]
    fn sign_keeps_a_partial_tree_and_the_matrix_rejects_it() {
        let key = SigningKey::from_bytes(&[3; 32]);
        let root = tempfile::tempdir().unwrap();
        let from = root.path().join("dist");
        let cogs = root.path().join("cogs");
        for id in ["kept", "skipped"] {
            let dist = from.join(id);
            elf(&dist, id, "arm");
            elf(&dist, id, "arm64");
            cog_toml(&cogs.join(id));
        }
        manifest(&from.join("kept"), "kept");
        sign_tree(from.to_str().unwrap(), &root.path().join("repo"), "t", &key).unwrap();
        let registry: Registry =
            serde_json::from_slice(&std::fs::read(root.path().join("repo/registry.json")).unwrap())
                .unwrap();
        assert_eq!(registry.cogs.len(), 1);
        let err = check_release_matrix(&from, &cogs, &registry).unwrap_err();
        assert!(err.contains("skipped: no manifest.json"), "{err}");
        assert!(err.contains("skipped: not in registry.json"), "{err}");
    }

    #[test]
    fn release_matrix_requires_arm_arm64_and_x86_64_only_when_present() {
        let root = tempfile::tempdir().unwrap();
        let from = root.path().join("dist");
        let cogs = root.path().join("cogs");
        let dist = from.join("probe");
        elf(&dist, "probe", "arm");
        elf(&dist, "probe", "arm64");
        cog_toml(&cogs.join("probe"));
        manifest(&dist, "probe");
        let mut artifacts = BTreeMap::new();
        artifacts.insert("arm".into(), artifact("arm"));
        artifacts.insert("arm64".into(), artifact("arm64"));
        let registry = registry_with("probe", artifacts.clone());
        check_release_matrix(&from, &cogs, &registry).unwrap();

        let arm_only = registry_with("probe", {
            let mut one = BTreeMap::new();
            one.insert("arm".into(), artifact("arm"));
            one
        });
        let err = check_release_matrix(&from, &cogs, &arm_only).unwrap_err();
        assert!(err.contains("expected [arm,arm64]"), "{err}");

        elf(&dist, "probe", "x86_64");
        let err = check_release_matrix(&from, &cogs, &registry).unwrap_err();
        assert!(err.contains("expected [arm,arm64,x86_64]"), "{err}");
        artifacts.insert("x86_64".into(), artifact("x86_64"));
        check_release_matrix(&from, &cogs, &registry_with("probe", artifacts)).unwrap();
    }

    fn artifact(arch: &str) -> Artifact {
        Artifact {
            path: format!("cogs/{arch}/cog"),
            size: 1,
            sha256: "abc".into(),
            sig: "00".into(),
            manifest_path: format!("cogs/{arch}/manifest.json"),
        }
    }

    fn registry_with(id: &str, artifacts: BTreeMap<String, Artifact>) -> Registry {
        Registry {
            schema: SCHEMA,
            repo: "t".into(),
            updated: String::new(),
            cogs: vec![CogEntry {
                id: id.into(),
                name: id.into(),
                version: "0.1.0".into(),
                category: String::new(),
                description: String::new(),
                hardware_requirement: vec![],
                artifacts,
            }],
        }
    }

    #[test]
    fn install_refuses_to_sideload_x86_64() {
        let e = cmd_install(&[
            "repo".into(),
            "probe".into(),
            "--seed".into(),
            "user@host".into(),
            "--arch".into(),
            "x86_64".into(),
        ])
        .unwrap_err();
        assert!(e.contains("not sideloaded"), "{e}");
    }
}
