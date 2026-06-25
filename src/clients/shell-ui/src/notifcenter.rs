//! Notification history state shared by `gnoblin-notifyd` and the topbar.
//!
//! The old standalone notification-center flag is still exposed so legacy
//! callers can clear it, but notifyd no longer opens a separate right-side
//! layer-shell panel. History is rendered inside the quick-settings popout.

use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub count: usize,
    pub latest_summary: String,
    pub latest_body: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub icon_name: String,
    pub timestamp_secs: u64,
}

fn path() -> Option<PathBuf> {
    std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|d| PathBuf::from(d).join("gnoblin-notif-center"))
}

/// Is the legacy notification-center compatibility flag present?
pub fn is_open() -> bool {
    path().map(|p| p.exists()).unwrap_or(false)
}

/// Clear the legacy notification-center compatibility flag if present. Nothing
/// sets it any more — the center folded into the quick-settings popout — so this
/// only cleans up a stale on-disk flag left by an older binary.
pub fn clear_legacy_flag() {
    let Some(p) = path() else { return };
    let _ = std::fs::remove_file(&p);
}

fn pending_path() -> Option<PathBuf> {
    std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|d| PathBuf::from(d).join("gnoblin-notif-pending"))
}

fn summary_path() -> Option<PathBuf> {
    std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|d| PathBuf::from(d).join("gnoblin-notif-summary"))
}

fn history_path() -> Option<PathBuf> {
    std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|d| PathBuf::from(d).join("gnoblin-notif-history"))
}

/// Does history hold any notifications? (notifyd writes this; the topbar's bell
/// reads it for its unread dot.)
pub fn has_pending() -> bool {
    pending_path().map(|p| p.exists()).unwrap_or(false)
}

/// Set whether history holds any notifications (called by notifyd).
pub fn set_pending(pending: bool) {
    let Some(p) = pending_path() else { return };
    if pending {
        let _ = std::fs::write(&p, b"");
    } else {
        let _ = std::fs::remove_file(&p);
    }
}

pub fn write_summary(summary: &Summary) {
    let Some(p) = summary_path() else { return };
    if summary.count == 0 {
        let _ = std::fs::remove_file(&p);
        return;
    }
    let text = format!(
        "{}\n{}\n{}\n",
        summary.count,
        summary.latest_summary.replace('\n', " "),
        summary.latest_body.replace('\n', " ")
    );
    let _ = std::fs::write(&p, text);
}

fn clean_field(value: &str) -> String {
    value.replace(['\n', '\r', '\t'], " ")
}

pub fn write_history(entries: &[Entry]) {
    let Some(p) = history_path() else { return };
    if entries.is_empty() {
        let _ = std::fs::remove_file(&p);
        return;
    }
    let mut text = String::new();
    for entry in entries {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            entry.timestamp_secs,
            clean_field(&entry.app_name),
            clean_field(&entry.summary),
            clean_field(&entry.body),
            clean_field(&entry.icon_name),
        ));
    }
    let _ = std::fs::write(&p, text);
}

pub fn history() -> Vec<Entry> {
    let Some(p) = history_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(p) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(5, '\t');
            Some(Entry {
                timestamp_secs: parts.next()?.parse().ok()?,
                app_name: parts.next().unwrap_or_default().to_string(),
                summary: parts.next().unwrap_or_default().to_string(),
                body: parts.next().unwrap_or_default().to_string(),
                icon_name: parts.next().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

pub fn dismiss_history_index(index: usize) -> bool {
    let mut entries = history();
    if index >= entries.len() {
        return false;
    }
    entries.remove(index);
    write_history(&entries);
    set_pending(!entries.is_empty());
    let summary = entries
        .first()
        .map_or_else(Summary::default, |entry| Summary {
            count: entries.len(),
            latest_summary: entry.summary.clone(),
            latest_body: entry.body.clone(),
        });
    write_summary(&summary);
    true
}

pub fn summary() -> Summary {
    let Some(p) = summary_path() else {
        return Summary::default();
    };
    let Ok(text) = std::fs::read_to_string(p) else {
        return Summary::default();
    };
    let mut lines = text.lines();
    Summary {
        count: lines
            .next()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or_default(),
        latest_summary: lines.next().unwrap_or_default().to_string(),
        latest_body: lines.next().unwrap_or_default().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn with_runtime_dir(test: impl FnOnce()) {
        let old = std::env::var_os("XDG_RUNTIME_DIR");
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("gnoblin-notif-test-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_RUNTIME_DIR", &dir);
        test();
        if let Some(old) = old {
            std::env::set_var("XDG_RUNTIME_DIR", old);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dismiss_history_index_removes_entry_and_updates_summary() {
        with_runtime_dir(|| {
            let entries = vec![
                Entry {
                    app_name: "One".into(),
                    summary: "First".into(),
                    body: "Body 1".into(),
                    icon_name: "one".into(),
                    timestamp_secs: 10,
                },
                Entry {
                    app_name: "Two".into(),
                    summary: "Second".into(),
                    body: "Body 2".into(),
                    icon_name: "two".into(),
                    timestamp_secs: 20,
                },
            ];
            write_history(&entries);
            write_summary(&Summary {
                count: entries.len(),
                latest_summary: entries[0].summary.clone(),
                latest_body: entries[0].body.clone(),
            });
            set_pending(true);

            assert!(dismiss_history_index(0));
            assert_eq!(history().len(), 1);
            assert_eq!(history()[0].summary, "Second");
            assert_eq!(summary().count, 1);
            assert_eq!(summary().latest_summary, "Second");
            assert!(has_pending());

            assert!(dismiss_history_index(0));
            assert!(history().is_empty());
            assert_eq!(summary(), Summary::default());
            assert!(!has_pending());
        });
    }
}
