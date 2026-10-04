//! Fleet heartbeat: how the glasses join the mesh as a node.
//!
//! The weft-cog-host exposes an open `POST /fleet/heartbeat` (COG-010). An edge node is "joined"
//! simply by posting one; the host tracks it online while heartbeats keep arriving and ages it out
//! when they stop. We post on behalf of the glasses on every successful ADB poll, so the node goes
//! offline on its own the moment the link drops — no faked liveness.

use crate::adb::Telemetry;
use std::io::{Read, Write};
use std::time::Duration;

/// Split a base URL like `http://203.0.113.10:9480` into (host, port). Defaults to port 9480.
pub fn host_port(base: &str) -> (String, u16) {
    let s = base.trim();
    let s = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))
        .unwrap_or(s);
    let s = s.split('/').next().unwrap_or(s); // drop any path
    match s.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(9480)),
        None => (s.to_string(), 9480),
    }
}

/// Post one heartbeat for the glasses. Best-effort: returns an error the caller logs but does not
/// treat as fatal.
pub fn send(host_base: &str, node_id: &str, t: &Telemetry) -> Result<(), String> {
    let (host, port) = host_port(host_base);
    let mut body = serde_json::json!({
        "id": node_id,
        "kind": "glasses",
        "sensor": t.model,
        "fw": t.fw,
    });
    if let Some(b) = t.battery_pct {
        body["battery"] = serde_json::json!(f64::from(b));
    }
    if let Some(ip) = &t.ip {
        body["ip"] = serde_json::json!(ip);
    }
    let body = serde_json::to_vec(&body).map_err(|e| format!("json: {e}"))?;

    let mut conn = std::net::TcpStream::connect((host.as_str(), port))
        .map_err(|e| format!("connect {host}:{port}: {e}"))?;
    conn.set_read_timeout(Some(Duration::from_secs(5))).ok();
    conn.set_write_timeout(Some(Duration::from_secs(5))).ok();
    write!(
        conn,
        "POST /fleet/heartbeat HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .map_err(|e| format!("write: {e}"))?;
    conn.write_all(&body).map_err(|e| format!("body: {e}"))?;

    let mut resp = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        match conn.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                resp.extend_from_slice(&buf[..n]);
                if resp.len() > 4096 {
                    break;
                }
            }
        }
    }
    let status = String::from_utf8_lossy(&resp)
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();
    if status.starts_with('2') {
        Ok(())
    } else {
        Err(format!("heartbeat replied {status:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_and_port() {
        assert_eq!(
            host_port("http://203.0.113.10:9480"),
            ("203.0.113.10".into(), 9480)
        );
        assert_eq!(
            host_port("https://pi5.lan:8088/x"),
            ("pi5.lan".into(), 8088)
        );
        assert_eq!(host_port("127.0.0.1"), ("127.0.0.1".into(), 9480));
    }
}
