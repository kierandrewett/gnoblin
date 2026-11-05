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
//! `app_id` and `title` accept a deliberately small pattern language instead of the
//! Lua patterns the configuration documents:
//!
//! - plain characters match themselves anywhere in the value;
//! - `.` matches any single character;
//! - `%` escapes the next character, so `%.` matches a literal dot and `%%` a
//!   literal percent sign;
//! - a leading `^` anchors at the start and a trailing bare `$` anchors at the end.
//!
//! Because the language has no quantifiers, matching is a single linear pass with
//! no backtracking: a pattern cannot loop, and it cannot blow up on a long title.
//! Quantifiers are the part of Lua patterns a rule author would notice missing, and
//! the configuration layer is the place to translate them.

use gnoblin_types::WorkspaceId;

use crate::window::{Window, WindowKind};

/// A compiled match pattern.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pattern {
    tokens: Vec<Token>,
    anchored_start: bool,
    anchored_end: bool,
}

/// One position in a pattern.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Token {
    /// Any single character.
    Any,
    /// Exactly this character.
    Literal(char),
}

impl Pattern {
    /// Compiles a pattern.
    #[must_use]
    pub fn new(source: &str) -> Self {
        let mut chars = source.chars().peekable();
        let anchored_start = chars.peek() == Some(&'^');
        if anchored_start {
            chars.next();
        }
        let mut tokens = Vec::new();
        let mut escaped = false;
        while let Some(character) = chars.next() {
            if escaped {
                tokens.push(Token::Literal(character));
                escaped = false;
                continue;
            }
            match character {
                '%' => escaped = true,
                '.' => tokens.push(Token::Any),
                // A trailing bare `$` anchors; anywhere else it is a literal.
                '$' if chars.peek().is_none() => {
                    return Self {
                        tokens,
                        anchored_start,
                        anchored_end: true,
                    };
                }
                other => tokens.push(Token::Literal(other)),
            }
        }
        // A dangling `%` matches a literal percent sign rather than swallowing the
        // end of the pattern.
        if escaped {
            tokens.push(Token::Literal('%'));
        }
        Self {
            tokens,
            anchored_start,
            anchored_end: false,
        }
    }

    /// Whether the value matches.
    #[must_use]
    pub fn matches(&self, value: &str) -> bool {
        let text: Vec<char> = value.chars().collect();
        if self.anchored_start {
            return self.matches_at(&text, 0);
        }
        (0..=text.len()).any(|start| self.matches_at(&text, start))
    }

    /// Whether the pattern matches when it starts at `start`.
    fn matches_at(&self, text: &[char], start: usize) -> bool {
        let mut index = start;
        for token in &self.tokens {
            let Some(character) = text.get(index) else {
                return false;
            };
            if let Token::Literal(expected) = token
                && character != expected
            {
                return false;
            }
            index += 1;
        }
        !self.anchored_end || index == text.len()
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
    use gnoblin_types::{OutputId, Rect, WindowId};

    fn window(app_id: &str, title: &str, kind: WindowKind) -> Window {
        Window {
            id: WindowId::new(1),
            app_id: app_id.to_owned(),
            title: title.to_owned(),
            kind,
            transient_for: None,
            workspace: WorkspaceId::new(1),
            output: Some(OutputId::new(0)),
            geometry: Rect::new(0, 0, 100, 100),
            restore: Rect::new(0, 0, 100, 100),
            state: Default::default(),
            applied_rules: 0,
        }
    }

    #[test]
    fn a_plain_pattern_matches_anywhere_in_the_value() {
        let pattern = Pattern::new("Notes");
        assert!(pattern.matches("Notes"));
        assert!(pattern.matches("Notes - Work"));
        assert!(pattern.matches("My Notes - Work"));
        assert!(!pattern.matches("note"));
    }

    #[test]
    fn anchors_require_the_whole_value() {
        let pattern = Pattern::new("^Notes$");
        assert!(pattern.matches("Notes"));
        assert!(!pattern.matches("Notes - Work"));
        assert!(!pattern.matches("My Notes"));
        assert!(Pattern::new("^Notes").matches("Notes - Work"));
        assert!(Pattern::new("Notes$").matches("My Notes"));
    }

    #[test]
    fn a_dot_matches_one_character_and_percent_escapes_a_literal() {
        assert!(Pattern::new("org.gnome.Nautilus").matches("orgxgnomeyNautilus"));
        assert!(Pattern::new("^org%.gnome%.Nautilus$").matches("org.gnome.Nautilus"));
        assert!(!Pattern::new("^org%.gnome%.Nautilus$").matches("orgxgnomexNautilus"));
        assert!(Pattern::new("100%%").matches("100%"));
        assert!(Pattern::new("done%").matches("done%"));
        assert!(Pattern::new("a$b").matches("a$b"));
    }

    #[test]
    fn an_empty_pattern_matches_everything() {
        assert!(Pattern::new("").matches(""));
        assert!(Pattern::new("").matches("anything"));
    }

    #[test]
    fn every_set_condition_must_hold() {
        let rule = Rule {
            matcher: Match {
                kind: Some(WindowKind::Normal),
                app_id: Some(Pattern::new("^org.example.Editor$")),
                title: Some(Pattern::new("Untitled")),
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
                title: Some(Pattern::new("^Notes$")),
                ..Match::default()
            },
            actions: Vec::new(),
        };
        assert!(rule.matches(&window("anything", "Notes", WindowKind::Modal)));
    }

    #[test]
    fn a_long_title_does_not_slow_matching_to_a_crawl() {
        // A pattern with no quantifiers can only scan linearly; a pathological
        // title mixing dots and literals must still finish immediately.
        let title: String = "a.".repeat(20_000);
        assert!(!Pattern::new("^a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.a%.b$").matches(&title));
    }
}
