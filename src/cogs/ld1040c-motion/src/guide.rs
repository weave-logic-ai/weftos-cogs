//! The cog's sensor guide (WeftOS ADR-104): `guide/guide.toml` plus one Markdown page per topic,
//! compiled in and served at `GET /guide` as `{"toml": "...", "pages": {id: md}, "images": {name: b64}}`
//! so a companion app always shows the guide that matches this exact cog version.

pub const TOML: &str = include_str!("../guide/guide.toml");

pub const PAGES: [(&str, &str); 7] = [
    ("start", include_str!("../guide/start.md")),
    ("sensor", include_str!("../guide/sensor.md")),
    ("wiring", include_str!("../guide/wiring.md")),
    ("connect", include_str!("../guide/connect.md")),
    ("telemetry", include_str!("../guide/telemetry.md")),
    ("troubleshoot", include_str!("../guide/troubleshoot.md")),
    ("api", include_str!("../guide/api.md")),
];

/// Board/pinout photos embedded in the guide bundle (ADR-104), served as base64 under "images".
pub const IMAGES: [(&str, &[u8]); 1] = [(
    "pizero2w-pinout.jpg",
    include_bytes!("../guide/pizero2w-pinout.jpg"),
)];

pub fn bundle_json() -> serde_json::Value {
    use base64::Engine as _;
    let pages: serde_json::Map<String, serde_json::Value> = PAGES
        .iter()
        .map(|(id, md)| ((*id).to_string(), serde_json::Value::String((*md).to_string())))
        .collect();
    let images: serde_json::Map<String, serde_json::Value> = IMAGES
        .iter()
        .map(|(name, bytes)| {
            (
                (*name).to_string(),
                serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(bytes)),
            )
        })
        .collect();
    serde_json::json!({ "toml": TOML, "pages": pages, "images": images })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_pages_match_the_guide_toml_page_list() {
        let line = TOML
            .lines()
            .find(|l| l.starts_with("pages = ["))
            .expect("pages line");
        let listed: Vec<String> = line
            .trim_start_matches("pages = [")
            .trim_end_matches(']')
            .split(',')
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let have: Vec<String> = PAGES.iter().map(|(id, _)| id.to_string()).collect();
        assert_eq!(listed, have, "guide.toml pages must match the PAGES array");
    }
}
