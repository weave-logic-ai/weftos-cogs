//! The bridge cog's sensor guide (WeftOS ADR-104): `guide/guide.toml` plus Markdown pages,
//! compiled in and served at `GET /guide`.

pub const TOML: &str = include_str!("../guide/guide.toml");

pub const PAGES: [(&str, &str); 7] = [
    ("start", include_str!("../guide/start.md")),
    ("setup", include_str!("../guide/setup.md")),
    ("protocol", include_str!("../guide/protocol.md")),
    ("troubleshoot", include_str!("../guide/troubleshoot.md")),
    ("api", include_str!("../guide/api.md")),
    ("security", include_str!("../guide/security.md")),
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
    fn page_list_matches_guide_toml() {
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
        assert_eq!(listed, PAGES.iter().map(|(id, _)| *id).collect::<Vec<_>>());
    }

    #[test]
    fn every_page_has_title_and_summary() {
        for (id, md) in PAGES {
            assert!(
                md.lines().next().is_some_and(|l| l.starts_with("# ")),
                "{id}: first line '# Title'"
            );
            assert!(
                md.lines().any(|l| l.starts_with("> ")),
                "{id}: needs '> summary'"
            );
        }
    }

    #[test]
    fn bundle_has_toml_and_pages() {
        assert_eq!(
            bundle_json()["pages"].as_object().unwrap().len(),
            PAGES.len()
        );
    }
}
