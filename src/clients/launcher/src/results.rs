use crate::desktop::App;
use crate::{provider, usage, AppEntry};
use gnoblin_shell_ui::find_icon;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// What activating a result does. Apps launch; computed results copy their
/// payload; provider results run a shell command.
enum Action {
    Launch(String),
    Copy(String),
    Run(String),
}

/// One result row: an app, calculator answer, provider hit, or web fallback.
pub(crate) struct Row {
    name: String,
    subtitle: String,
    /// Theme icon name for app/provider/web rows; empty for built-in glyphs.
    icon: String,
    /// "app" | "calc" | "provider" | "web"; lets the view pick styling.
    kind: String,
    /// Right-aligned accessory text, e.g. a calculator result like "= 4".
    accessory: String,
    action: Action,
}

impl Row {
    pub(crate) fn calculator(answer: String) -> Self {
        Self {
            name: answer.clone(),
            subtitle: "Calculator — press ⏎ to copy".into(),
            icon: String::new(),
            kind: "calc".into(),
            accessory: String::new(),
            action: Action::Copy(answer),
        }
    }

    pub(crate) fn provider(result: provider::ProviderResult) -> Self {
        let action = if result.action.is_empty() {
            Action::Copy(result.title.clone())
        } else {
            Action::Run(result.action)
        };
        Self {
            name: result.title,
            subtitle: result.subtitle,
            icon: result.icon,
            kind: "provider".into(),
            accessory: String::new(),
            action,
        }
    }

    pub(crate) fn app(app: &App) -> Self {
        Self {
            name: app.name.clone(),
            subtitle: "Application".into(),
            icon: app.icon.clone(),
            kind: "app".into(),
            accessory: String::new(),
            action: Action::Launch(app.id.clone()),
        }
    }

    pub(crate) fn web_search(query: &str, template: &str) -> Self {
        let encoded = urlencode(query);
        Self {
            name: format!("Search the web for “{query}”"),
            subtitle: "Open in your browser".into(),
            icon: "web-browser".into(),
            kind: "web".into(),
            accessory: String::new(),
            action: Action::Run(format!(
                "xdg-open {}",
                shell_quote(&template.replace("%s", &encoded))
            )),
        }
    }

    pub(crate) fn entry(&self) -> AppEntry {
        let icon = if self.icon.is_empty() {
            None
        } else {
            find_icon(&self.icon, "")
        };
        AppEntry {
            name: self.name.clone().into(),
            subtitle: self.subtitle.clone().into(),
            has_icon: icon.is_some(),
            icon: icon.unwrap_or_default(),
            kind: self.kind.clone().into(),
            accessory: self.accessory.clone().into(),
        }
    }
}

pub(crate) fn activate(
    rows: &[Row],
    index: usize,
    usage_counts: &Rc<RefCell<HashMap<String, u32>>>,
) {
    if let Some(row) = rows.get(index) {
        match &row.action {
            Action::Launch(id) => launch_app(id, usage_counts),
            Action::Copy(text) => copy_to_clipboard(text),
            Action::Run(cmd) => run_shell(cmd),
        }
    }
}

/// Bump `id`'s launch count, persist, then run it.
fn launch_app(id: &str, usage_counts: &Rc<RefCell<HashMap<String, u32>>>) {
    {
        let mut usage_counts = usage_counts.borrow_mut();
        *usage_counts.entry(id.to_string()).or_insert(0) += 1;
        usage::save(&usage_counts);
    }
    gnoblin_shell_ui::launch_desktop_app(id);
}

/// Put `text` on the Wayland clipboard. Best-effort: if `wl-copy` is missing,
/// nothing happens.
fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    if let Ok(mut child) = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

fn run_shell(cmd: &str) {
    use std::process::{Command, Stdio};
    let _ = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Minimal URL query encoder (RFC 3986 unreserved chars pass through).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Single-quote a string for safe use inside `sh -c`.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
