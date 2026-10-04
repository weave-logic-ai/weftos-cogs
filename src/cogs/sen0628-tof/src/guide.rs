//! The cog's sensor guide (WeftOS ADR-104): `guide/guide.toml` plus one Markdown page per
//! topic, compiled in and served at `GET /guide` as `{"toml": "...", "pages": {id: md}}` so a
//! companion app always shows the guide that matches this exact cog version.

pub const TOML: &str = include_str!("../guide/guide.toml");

pub const PAGES: [(&str, &str); 10] = [
    ("start", include_str!("../guide/start.md")),
    ("parts", include_str!("../guide/parts.md")),
    ("wiring", include_str!("../guide/wiring.md")),
    ("mounting", include_str!("../guide/mounting.md")),
    ("setup", include_str!("../guide/setup.md")),
    ("calibrate", include_str!("../guide/calibrate.md")),
    ("troubleshoot", include_str!("../guide/troubleshoot.md")),
    ("api", include_str!("../guide/api.md")),
    ("safety", include_str!("../guide/safety.md")),
    ("glossary", include_str!("../guide/glossary.md")),
];

pub fn bundle_json() -> serde_json::Value {
    let pages: serde_json::Map<String, serde_json::Value> = PAGES
        .iter()
        .map(|(id, md)| {
            (
                (*id).to_string(),
                serde_json::Value::String((*md).to_string()),
            )
        })
        .collect();
    serde_json::json!({ "toml": TOML, "pages": pages })
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
        let listed: Vec<&str> = line
            .trim_start_matches("pages = [")
            .trim_end_matches(']')
            .split(',')
            .map(|s| s.trim().trim_matches('"'))
            .collect();
        let embedded: Vec<&str> = PAGES.iter().map(|(id, _)| *id).collect();
        assert_eq!(listed, embedded);
    }

    #[test]
    fn every_page_starts_with_a_title_and_summary() {
        for (id, md) in PAGES {
            assert!(
                md.lines().next().is_some_and(|l| l.starts_with("# ")),
                "{id}: first line must be '# Title'"
            );
            assert!(
                md.lines().any(|l| l.starts_with("> ")),
                "{id}: needs a '> summary' line"
            );
        }
    }

    #[test]
    fn bundle_has_toml_and_all_pages() {
        let b = bundle_json();
        assert!(b["toml"].as_str().unwrap().contains("schema = 1"));
        assert_eq!(b["pages"].as_object().unwrap().len(), PAGES.len());
    }
}
