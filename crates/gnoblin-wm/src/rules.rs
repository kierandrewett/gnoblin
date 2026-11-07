//! Window rules: matching on application id and title, with typed actions.
//!
//! A rule's `match` selects windows and its actions change window-manager state.
//! Every rule is evaluated once, when a window is first taken into management, so a
//! rule can never be applied twice by a title change, an application-id change or a
//! rules reload. Later rules override earlier ones field by field, which is why the
//! whole list is walked rather than stopping at the first match.
//!
//! # Patterns
//!
//! `app_id` and `title` take Lua 5.4 `string.find` patterns, the language the
//! configuration has always documented. A pattern is checked when it is built, so a
//! malformed one is refused before any window exists. Matching draws from a step
//! budget: a match that runs out does not match, so one pathological title cannot hold
//! the policy thread.

use gnoblin_types::WorkspaceId;

use crate::window::{Window, WindowKind};

pub use gnoblin_lua_pattern::PatternSyntaxError;

/// Steps one match may take before it gives up and reports no match.
const MATCH_BUDGET: u64 = 1 << 20;

/// A compiled match pattern. Construction checks the whole pattern, so a stored
/// `Pattern` is always well formed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pattern {
    source: String,
}

impl Pattern {
    /// Checks `source` and stores it.
    ///
    /// # Errors
    ///
    /// Returns the first syntax error, with the byte offset where it was found.
    pub fn new(source: &str) -> Result<Self, PatternSyntaxError> {
        gnoblin_lua_pattern::validate(source.as_bytes())?;
        Ok(Self {
            source: source.to_owned(),
        })
    }

    /// Whether the value matches. A match that exhausts its budget, or recurses past
    /// the depth limit, does not match.
    #[must_use]
    pub fn matches(&self, value: &str) -> bool {
        gnoblin_lua_pattern::find(self.source.as_bytes(), value.as_bytes(), MATCH_BUDGET).unwrap_or(false)
    }
}

/// The conditions a rule requires of a window.
///
/// Every field that is set must match; an unset field does not constrain. A rule
/// with no conditions matches every window, which is how a global appearance rule
/// is written.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Match {
    /// Restrict to one kind of toplevel.
    pub kind: Option<WindowKind>,
    /// Restrict to application ids matching this pattern.
    pub app_id: Option<Pattern>,
    /// Restrict to titles matching this pattern.
    pub title: Option<Pattern>,
}

impl Match {
    /// Whether the conditions all hold for `window`.
    #[must_use]
    pub fn matches(&self, window: &Window) -> bool {
        if let Some(kind) = self.kind
            && window.kind != kind
        {
            return false;
        }
        if let Some(pattern) = &self.app_id
            && !pattern.matches(&window.app_id)
        {
            return false;
        }
        if let Some(pattern) = &self.title
            && !pattern.matches(&window.title)
        {
            return false;
        }
        true
    }
}

/// How a rule names the workspace it places a window on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorkspaceSelector {
    /// A stable workspace id.
    Id(WorkspaceId),
    /// A one-based position in the declared list.
    Number(u32),
}

/// What a matching rule does to the window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RuleAction {
    /// Moves the window to a workspace when it is first managed.
    PlaceOnWorkspace(WorkspaceSelector),
    /// Maximises the window when it is first managed.
    Maximise,
}

/// One rule: a matcher and the actions that follow from it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Rule {
    /// The conditions the rule requires.
    pub matcher: Match,
    /// What happens when the conditions hold.
    pub actions: Vec<RuleAction>,
}

impl Rule {
    /// Whether the rule selects the window.
    #[must_use]
    pub fn matches(&self, window: &Window) -> bool {
        self.matcher.matches(window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_lua_pattern::PatternError;
    use gnoblin_types::{OutputId, Rect, WindowId};

    fn window(app_id: &str, title: &str, kind: WindowKind) -> Window {
        Window {
            id: WindowId::new(1),
            app_id: app_id.to_owned(),
            title: title.to_owned(),
            kind,
            transient_for: None,
            workspace: WorkspaceId::new(1),
            placement: crate::layout::WindowPlacement::Managed,
            output: Some(OutputId::new(0)),
            geometry: Rect::new(0, 0, 100, 100),
            restore: Rect::new(0, 0, 100, 100),
            state: Default::default(),
            applied_rules: 0,
        }
    }

    /// A pattern the test expects to be valid.
    fn pattern(source: &str) -> Pattern {
        Pattern::new(source).expect("test pattern is valid")
    }

    #[test]
    fn a_plain_pattern_matches_anywhere_in_the_value() {
        let pattern = pattern("Notes");
        assert!(pattern.matches("Notes"));
        assert!(pattern.matches("Notes - Work"));
        assert!(pattern.matches("My Notes - Work"));
        assert!(!pattern.matches("note"));
    }

    #[test]
    fn anchors_require_the_whole_value() {
        assert!(pattern("^Notes$").matches("Notes"));
        assert!(!pattern("^Notes$").matches("Notes - Work"));
        assert!(!pattern("^Notes$").matches("My Notes"));
        assert!(pattern("^Notes").matches("Notes - Work"));
        assert!(pattern("Notes$").matches("My Notes"));
    }

    #[test]
    fn a_dot_matches_one_character_and_percent_escapes_a_literal() {
        assert!(pattern("org.gnome.Nautilus").matches("orgxgnomeyNautilus"));
        assert!(pattern("^org%.gnome%.Nautilus$").matches("org.gnome.Nautilus"));
        assert!(!pattern("^org%.gnome%.Nautilus$").matches("orgxgnomexNautilus"));
        assert!(pattern("100%%").matches("100%"));
        assert!(pattern("a$b").matches("a$b"));
    }

    #[test]
    fn quantifiers_follow_lua() {
        assert!(pattern("^v%d+%.%d+$").matches("v12.3"));
        assert!(!pattern("^v%d+%.%d+$").matches("v.3"));
        assert!(pattern("^%a+%-").matches("gnome-shell"));
        assert!(pattern("^a-b$").matches("aaab"));
        assert!(pattern("colou?r").matches("color"));
    }

    #[test]
    fn an_empty_pattern_matches_everything() {
        assert!(pattern("").matches(""));
        assert!(pattern("").matches("anything"));
    }

    #[test]
    fn a_malformed_pattern_is_refused_with_its_position() {
        let error = Pattern::new("done%").expect_err("a trailing % is malformed");
        assert_eq!(error.reason, "malformed pattern (ends with '%')");
        assert_eq!(error.offset, 4);
        assert!(Pattern::new("[abc").is_err());
    }

    #[test]
    fn every_set_condition_must_hold() {
        let rule = Rule {
            matcher: Match {
                kind: Some(WindowKind::Normal),
                app_id: Some(pattern("^org.example.Editor$")),
                title: Some(pattern("Untitled")),
            },
            actions: vec![RuleAction::Maximise],
        };
        assert!(rule.matches(&window("org.example.Editor", "Untitled 1", WindowKind::Normal)));
        assert!(!rule.matches(&window("org.example.Editor", "Saved", WindowKind::Normal)));
        assert!(!rule.matches(&window("org.example.Viewer", "Untitled", WindowKind::Normal)));
        assert!(!rule.matches(&window("org.example.Editor", "Untitled", WindowKind::Dialog)));
    }

    #[test]
    fn an_unset_condition_does_not_constrain() {
        let rule = Rule {
            matcher: Match {
                title: Some(pattern("^Notes$")),
                ..Match::default()
            },
            actions: Vec::new(),
        };
        assert!(rule.matches(&window("anything", "Notes", WindowKind::Modal)));
    }

    #[test]
    fn a_pathological_pattern_stops_at_the_budget_and_does_not_match() {
        // Three greedy repeats over a title with no `b` backtrack through every split:
        // cubic work that the budget must cut short rather than finish.
        let title = "a".repeat(2_000);
        let source = "%a*%a*%a*b";
        assert_eq!(
            gnoblin_lua_pattern::find(source.as_bytes(), title.as_bytes(), MATCH_BUDGET),
            Err(PatternError::Budget)
        );
        assert!(!pattern(source).matches(&title));
    }

    #[test]
    fn a_long_title_with_no_quantifiers_scans_linearly() {
        let title: String = "a.".repeat(20_000);
        assert!(!pattern("^a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.b$").matches(&title));
    }
}
