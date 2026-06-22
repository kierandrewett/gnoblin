//! Minimal reader for gnoblin.conf — the sectioned INI the compositor uses.
//! (Mirrors src/config/gnoblin-config.c: `[section]` headers, `key = value`,
//! `#` comments, quoted values, last value wins.) Extract to a shared crate
//! once a second client needs it.

use std::collections::HashMap;
use std::path::PathBuf;

pub struct Config {
    values: HashMap<(String, String), String>,
    // All values of a repeated key, in file order (e.g. each `item` in [menu]).
    lists: HashMap<(String, String), Vec<String>>,
    // Section names in first-seen file order (so callers can enumerate dynamic
    // sections like `[qs-plugin.NAME]` whose names aren't known ahead of time).
    sections: Vec<String>,
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        config_path()
    }

    pub fn load() -> Self {
        let mut values = HashMap::new();
        let mut lists = HashMap::new();
        let mut sections = Vec::new();
        if let Some(text) = read_file() {
            parse(&text, &mut values, &mut lists, &mut sections);
        }
        Config {
            values,
            lists,
            sections,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_text(text: &str) -> Self {
        let mut values = HashMap::new();
        let mut lists = HashMap::new();
        let mut sections = Vec::new();
        parse(text, &mut values, &mut lists, &mut sections);
        Config {
            values,
            lists,
            sections,
        }
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.values
            .get(&(section.to_string(), key.to_string()))
            .map(String::as_str)
    }

    /// All section names beginning with `prefix`, in first-seen file order.
    /// Used to enumerate dynamic sections (e.g. each `[qs-plugin.NAME]`).
    pub fn sections_with_prefix(&self, prefix: &str) -> Vec<&str> {
        self.sections
            .iter()
            .filter(|s| s.starts_with(prefix))
            .map(String::as_str)
            .collect()
    }

    /// Every `(key, value)` in `[section]` whose key begins with `prefix`, in
    /// first-seen file order. Used for the `[providers] get-NAME = cmd`
    /// shorthand where the key names aren't known ahead of time.
    pub fn entries_with_prefix(&self, section: &str, prefix: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for ((sec, key), value) in &self.values {
            if sec == section && key.starts_with(prefix) && seen.insert(key.clone()) {
                out.push((key.clone(), value.clone()));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Every value of a repeated `key` in `[section]`, in file order.
    pub fn get_list(&self, section: &str, key: &str) -> Vec<String> {
        self.lists
            .get(&(section.to_string(), key.to_string()))
            .cloned()
            .unwrap_or_default()
    }
}

fn config_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("GNOBLIN_CONFIG") {
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }

    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config"))
        })?;

    Some(base.join("gnoblin/gnoblin.conf"))
}

fn read_file() -> Option<String> {
    std::fs::read_to_string(config_path()?).ok()
}

fn parse(
    text: &str,
    out: &mut HashMap<(String, String), String>,
    lists: &mut HashMap<(String, String), Vec<String>>,
    sections: &mut Vec<String>,
) {
    let mut section = String::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if let Some(rest) = line.strip_prefix('[') {
            if let Some(close) = rest.find(']') {
                section = rest[..close].trim().to_string();
                if !section.is_empty() && !sections.iter().any(|s| s == &section) {
                    sections.push(section.clone());
                }
            }
            continue;
        }

        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim().to_string();
            let value = clean_value(&line[eq + 1..]);
            lists
                .entry((section.clone(), key.clone()))
                .or_default()
                .push(value.clone());
            out.insert((section.clone(), key), value);
        }
    }
}

// Turn the raw text after `=` into the final value, mirroring the compositor's
// C parser (src/config/gnoblin-config.c `clean_value`): a fully-quoted string is
// the text between the opening quote and its match; otherwise strip a `#` inline
// comment only when it's introduced by whitespace, so a `#` inside e.g. a
// wallpaper path is kept rather than truncating the value to nothing.
fn clean_value(raw: &str) -> String {
    let raw = raw.trim();

    if let Some(quote) = raw.chars().next().filter(|&c| c == '"' || c == '\'') {
        let rest = &raw[quote.len_utf8()..];
        if let Some(end) = rest.find(quote) {
            return rest[..end].to_string();
        }
    }

    let mut quote = None;
    let mut prev = '\0';
    for (idx, ch) in raw.char_indices() {
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch == '#' && idx != 0 && (prev == ' ' || prev == '\t') {
            return raw[..idx].trim().to_string();
        }
        prev = ch;
    }
    raw.to_string()
}

#[cfg(test)]
mod tests {
    use super::{clean_value, Config};

    #[test]
    fn quoted_value_drops_trailing_comment() {
        assert_eq!(clean_value(r##""#1d1f21"   # desktop colour"##), "#1d1f21");
    }

    #[test]
    fn unquoted_value_strips_whitespace_comment() {
        assert_eq!(clean_value("zoom   # style"), "zoom");
    }

    #[test]
    fn hash_inside_unquoted_path_is_kept() {
        assert_eq!(
            clean_value("/photos/trip#3/bg.jpg"),
            "/photos/trip#3/bg.jpg"
        );
    }

    #[test]
    fn hash_inside_quoted_span_is_kept_like_c_parser() {
        assert_eq!(
            clean_value("printf 'value # kept' > /tmp/marker # trailing comment"),
            "printf 'value # kept' > /tmp/marker"
        );
    }

    #[test]
    fn no_comment_passes_through() {
        assert_eq!(clean_value("scaled"), "scaled");
    }

    #[test]
    fn section_headers_allow_trailing_comments_like_c_parser() {
        let cfg = Config::from_text(
            "[topbar] # shell clients should parse this like src/config/gnoblin-config.c\n\
             left = status\n",
        );

        assert_eq!(cfg.get("topbar", "left"), Some("status"));
    }

    #[test]
    fn semicolon_lines_are_comments_like_c_parser() {
        let cfg = Config::from_text(
            "[topbar]\n\
             ; left = status\n\
             left = clock\n",
        );

        assert_eq!(cfg.get("topbar", "left"), Some("clock"));
        assert_eq!(cfg.get("topbar", "; left"), None);
    }

    #[test]
    fn shipped_example_parses_for_rust_clients() {
        let cfg = Config::from_text(include_str!("../../../data/gnoblin.conf.example"));

        assert_eq!(cfg.get("appearance", "background"), Some("#1d1f21"));
        assert_eq!(
            cfg.get("appearance", "shadow"),
            Some("0 20px 48px -20px rgba(0,0,0,.22), 0 4px 12px -6px rgba(0,0,0,.14)")
        );
        assert_eq!(cfg.get("appearance", "wallpaper"), None);
        assert_eq!(cfg.get("appearance", "surface-background"), None);

        assert_eq!(
            cfg.get_list("startup", "exec_per_output"),
            vec![
                "gnoblin-topbar".to_string(),
                "gnoblin-dock".to_string(),
                "gnoblin-wallpaper".to_string(),
            ]
        );
        assert_eq!(
            cfg.get_list("startup", "exec"),
            vec![
                "gnoblin-notifyd".to_string(),
                "gnoblin-night-light".to_string(),
            ]
        );

        assert_eq!(
            cfg.get("topbar", "left"),
            Some("workspaces, focused-app, appmenu, spring")
        );
        assert_eq!(cfg.get("topbar", "center"), Some("clock"));
        assert_eq!(cfg.get("topbar", "right"), Some("launcher, tray, status"));
        assert_eq!(cfg.get("topbar", "appmenu-backend"), Some("auto"));

        assert_eq!(cfg.get("features", "appmenu"), Some("on"));
        assert_eq!(cfg.get("protocols", "kde-appmenu"), Some("on"));

        assert_eq!(cfg.get("animations", "enabled"), None);
        assert_eq!(cfg.get("animations", "slint-overlay-open"), None);
        assert_eq!(
            cfg.get("animations", "maximize"),
            Some("300, ease-out-quart")
        );
        assert_eq!(
            cfg.get("animations", "open.popup-menu"),
            Some("80, ease-out-quad, 0.995")
        );

        let raise = cfg
            .get("bind", "XF86AudioRaiseVolume")
            .expect("shipped media bind exists");
        assert!(raise.contains("5%+; gnoblin-osd volume"));
        assert!(raise.starts_with("spawn sh -c "));
    }
}
