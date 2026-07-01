use crate::{ControlCentrePopoutWindow, NotificationItem};
use gnoblin_desktop::find_icon;
use gnoblin_runtime::{datetime, notifcenter};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

fn age_label(timestamp_secs: u64) -> String {
    if timestamp_secs == 0 {
        return String::new();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let elapsed = now.saturating_sub(timestamp_secs);
    let format = if elapsed < 86_400 { "%H:%M" } else { "%x" };
    datetime::format_unix(timestamp_secs, format).unwrap_or_default()
}

pub(crate) fn apply(p: Option<&ControlCentrePopoutWindow>) -> notifcenter::Summary {
    let summary = notifcenter::summary();
    let entries = notifcenter::history();
    let items: Vec<NotificationItem> = entries
        .iter()
        .take(8)
        .map(|entry| {
            let icon = find_icon(&entry.icon_name, "");
            NotificationItem {
                app_name: if entry.app_name.is_empty() {
                    "Notification".into()
                } else {
                    entry.app_name.clone().into()
                },
                summary: entry.summary.clone().into(),
                body: entry.body.clone().into(),
                age: age_label(entry.timestamp_secs).into(),
                has_icon: icon.is_some(),
                icon: icon.unwrap_or_default(),
            }
        })
        .collect();
    let count = summary.count.max(entries.len()).min(i32::MAX as usize) as i32;
    if let Some(p) = p {
        p.set_notification_count(count);
        p.set_notifications(Rc::new(slint::VecModel::from(items)).into());
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_uses_locale_time_not_english_relative_text() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap();
        let recent = age_label(now.saturating_sub(90));
        let older = age_label(now.saturating_sub(172_800));

        for label in [recent, older] {
            let lower = label.to_ascii_lowercase();
            assert!(!lower.contains("ago"));
            assert!(!lower.contains("yesterday"));
            assert_ne!(lower, "now");
        }
    }
}
