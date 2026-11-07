//! Window rules: which windows a rule selects, and what it sets on them.
//!
//! A rule has a `match`, which selects windows, and `fields`, which set window state
//! when the window is first taken into management. Every rule is evaluated once per
//! window, so a title change, an application-id change or a rules reload never applies
//! a rule a second time. Later rules override earlier ones field by field, which is
//! why the whole list is walked rather than stopping at the first match.
//!
//! # Patterns
//!
//! `app_id` and `title` take Lua 5.4 `string.find` patterns, the language the
//! configuration has always documented. A pattern is checked when it is built, so a
//! malformed one is refused before any window exists. Matching draws from a step
//! budget: a match that runs out does not match, so one pathological title cannot hold
//! the policy thread.

use std::fmt;

use gnoblin_types::WorkspaceId;

use crate::event::WindowProtocol;
use crate::window::{Window, WindowKind};

pub use gnoblin_lua_pattern::PatternSyntaxError;

/// Most rules one configuration may declare. The walk over the list runs for every
/// new window, so an unbounded list would make each window pay for it.
pub const MAX_RULES: usize = 256;

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

/// Facts about a window that are not stored on it, because they belong to the session
/// around it rather than to the window's own state.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Facts {
    /// Whether the window holds keyboard focus when the rule is evaluated.
    pub focused: bool,
    /// The one-based position of the window's workspace in the declared list.
    pub workspace_number: Option<u32>,
}

/// The conditions a rule requires of a window.
///
/// Every field that is set must hold; an unset field does not constrain. A rule with
/// no conditions matches every window, which is how a global appearance rule is
/// written.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Match {
    /// Restrict to one kind of toplevel.
    pub kind: Option<WindowKind>,
    /// Restrict to application ids matching this pattern.
    pub app_id: Option<Pattern>,
    /// Restrict to titles matching this pattern.
    pub title: Option<Pattern>,
    /// Restrict to windows that hold, or do not hold, keyboard focus.
    pub focused: Option<bool>,
    /// A layer-shell surface name. Layer surfaces are not windows, so a window never
    /// satisfies this condition, and a rule that sets it selects nothing. It is
    /// accepted so the same vocabulary serves surfaces and windows.
    pub layer: Option<Pattern>,
    /// Restrict to windows on this workspace.
    pub workspace_id: Option<WorkspaceId>,
    /// Restrict to windows on the workspace at this one-based position.
    pub workspace_number: Option<u32>,
    /// Restrict to one client protocol: Wayland toplevels or X11 windows.
    pub client_type: Option<WindowProtocol>,
}

impl Match {
    /// Whether the conditions all hold for `window`, given the session `facts`.
    #[must_use]
    pub fn matches(&self, window: &Window, facts: Facts) -> bool {
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
        if let Some(focused) = self.focused
            && facts.focused != focused
        {
            return false;
        }
        if self.layer.is_some() {
            return false;
        }
        if let Some(id) = self.workspace_id
            && window.workspace != id
        {
            return false;
        }
        if let Some(number) = self.workspace_number
            && facts.workspace_number != Some(number)
        {
            return false;
        }
        if let Some(protocol) = self.client_type
            && window.protocol != protocol
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

/// The state a matching rule sets on the window. Each field that is `None` leaves the
/// window's state alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RuleFields {
    /// Moves the window to this workspace when it is first managed.
    pub workspace: Option<WorkspaceSelector>,
    /// Maximises (`true`) or unmaximises (`false`) the window when it is first managed.
    pub maximise: Option<bool>,
}

/// One rule: the conditions that select a window and the state it is given.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Rule {
    /// The conditions the rule requires.
    pub matcher: Match,
    /// The state the rule sets when the conditions hold.
    pub fields: RuleFields,
}

/// The combined effect of every rule that matched one window.
///
/// Later rules override earlier ones field by field, so each field holds the value of
/// the last matching rule that set it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RuleOutcome {
    /// How many rules matched, whether or not their fields can be carried out.
    pub matched: usize,
    /// The winning workspace target, with the zero-based index of the rule that set it.
    pub workspace: Option<(usize, WorkspaceSelector)>,
    /// The winning maximise state.
    pub maximise: Option<bool>,
}

/// Walks the rule list for one window and combines the rules that match it.
#[must_use]
pub fn merge(rules: &[Rule], window: &Window, facts: Facts) -> RuleOutcome {
    let mut outcome = RuleOutcome::default();
    for (index, rule) in rules.iter().enumerate() {
        if !rule.matcher.matches(window, facts) {
            continue;
        }
        outcome.matched += 1;
        if let Some(selector) = rule.fields.workspace {
            outcome.workspace = Some((index, selector));
        }
        if let Some(maximise) = rule.fields.maximise {
            outcome.maximise = Some(maximise);
        }
    }
    outcome
}

/// A rule list the window manager refuses, and why.
///
/// The configuration is checked as a whole before it is applied, so a refused list
/// leaves the rules already in force untouched.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RuleError {
    /// More rules were declared than [`MAX_RULES`] allows.
    TooManyRules {
        /// How many were declared.
        count: usize,
        /// The most that are allowed.
        max: usize,
    },
    /// One rule sets a field to a value that cannot select or place anything.
    InvalidField {
        /// The zero-based position of the rule in the list.
        rule: usize,
        /// The dotted name of the field, as the configuration spells it.
        field: &'static str,
        /// Why the value is refused.
        reason: &'static str,
    },
}

impl fmt::Display for RuleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRules { count, max } => {
                write!(formatter, "{count} window rules declared, at most {max} are allowed")
            }
            Self::InvalidField { rule, field, reason } => {
                write!(formatter, "window rule {}: {field} {reason}", rule + 1)
            }
        }
    }
}

impl std::error::Error for RuleError {}

/// Checks a whole rule list before it replaces the rules in force.
///
/// # Errors
///
/// Refuses a list longer than [`MAX_RULES`], and any rule that names workspace number
/// zero, because workspace numbers start at one.
pub fn validate_list(rules: &[Rule]) -> Result<(), RuleError> {
    if rules.len() > MAX_RULES {
        return Err(RuleError::TooManyRules {
            count: rules.len(),
            max: MAX_RULES,
        });
    }
    for (rule, entry) in rules.iter().enumerate() {
        if entry.matcher.workspace_number == Some(0) {
            return Err(RuleError::InvalidField {
                rule,
                field: "match.workspace_number",
                reason: "must be at least 1, because workspaces are numbered from 1",
            });
        }
        if entry.fields.workspace == Some(WorkspaceSelector::Number(0)) {
            return Err(RuleError::InvalidField {
                rule,
                field: "fields.workspace",
                reason: "must be at least 1, because workspaces are numbered from 1",
            });
        }
    }
    Ok(())
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
            protocol: WindowProtocol::Wayland,
        }
    }

    fn no_facts() -> Facts {
        Facts::default()
    }

    /// A pattern the test expects to be valid.
    fn pattern(source: &str) -> Pattern {
        Pattern::new(source).expect("test pattern is valid")
    }

    fn rule(matcher: Match, fields: RuleFields) -> Rule {
        Rule { matcher, fields }
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
        let matcher = Match {
            kind: Some(WindowKind::Normal),
            app_id: Some(pattern("^org.example.Editor$")),
            title: Some(pattern("Untitled")),
            ..Match::default()
        };
        assert!(matcher.matches(
            &window("org.example.Editor", "Untitled 1", WindowKind::Normal),
            no_facts()
        ));
        assert!(!matcher.matches(&window("org.example.Editor", "Saved", WindowKind::Normal), no_facts()));
        assert!(!matcher.matches(
            &window("org.example.Viewer", "Untitled", WindowKind::Normal),
            no_facts()
        ));
        assert!(!matcher.matches(
            &window("org.example.Editor", "Untitled", WindowKind::Dialog),
            no_facts()
        ));
    }

    #[test]
    fn an_unset_condition_does_not_constrain() {
        let matcher = Match {
            title: Some(pattern("^Notes$")),
            ..Match::default()
        };
        assert!(matcher.matches(&window("anything", "Notes", WindowKind::Modal), no_facts()));
    }

    #[test]
    fn focus_matches_the_session_fact_both_ways() {
        let focused = Match {
            focused: Some(true),
            ..Match::default()
        };
        let unfocused = Match {
            focused: Some(false),
            ..Match::default()
        };
        let window = window("app", "t", WindowKind::Normal);
        let held = Facts {
            focused: true,
            workspace_number: None,
        };
        assert!(focused.matches(&window, held));
        assert!(!focused.matches(&window, no_facts()));
        assert!(unfocused.matches(&window, no_facts()));
        assert!(!unfocused.matches(&window, held));
    }

    #[test]
    fn a_layer_condition_never_matches_a_window() {
        let matcher = Match {
            layer: Some(pattern("^bar$")),
            ..Match::default()
        };
        assert!(!matcher.matches(&window("bar", "bar", WindowKind::Normal), no_facts()));
        assert!(Match::default().matches(&window("bar", "bar", WindowKind::Normal), no_facts()));
    }

    #[test]
    fn workspace_id_and_number_select_by_placement_in_the_list() {
        let window = window("app", "t", WindowKind::Normal);
        let by_id = Match {
            workspace_id: Some(WorkspaceId::new(1)),
            ..Match::default()
        };
        assert!(by_id.matches(&window, no_facts()));
        let other_id = Match {
            workspace_id: Some(WorkspaceId::new(2)),
            ..Match::default()
        };
        assert!(!other_id.matches(&window, no_facts()));
        let by_number = Match {
            workspace_number: Some(2),
            ..Match::default()
        };
        let second = Facts {
            focused: false,
            workspace_number: Some(2),
        };
        assert!(by_number.matches(&window, second));
        assert!(!by_number.matches(&window, no_facts()));
    }

    #[test]
    fn client_type_selects_the_protocol() {
        let x11 = Match {
            client_type: Some(WindowProtocol::X11),
            ..Match::default()
        };
        let mut window = window("app", "t", WindowKind::Normal);
        assert!(!x11.matches(&window, no_facts()));
        window.protocol = WindowProtocol::X11;
        assert!(x11.matches(&window, no_facts()));
    }

    #[test]
    fn merge_counts_every_match_and_keeps_the_last_value_of_each_field() {
        let window = window("app", "t", WindowKind::Normal);
        let rules = vec![
            rule(
                Match::default(),
                RuleFields {
                    workspace: Some(WorkspaceSelector::Number(1)),
                    maximise: Some(true),
                },
            ),
            rule(
                Match {
                    app_id: Some(pattern("^nope$")),
                    ..Match::default()
                },
                RuleFields {
                    workspace: Some(WorkspaceSelector::Number(3)),
                    maximise: None,
                },
            ),
            rule(
                Match::default(),
                RuleFields {
                    workspace: Some(WorkspaceSelector::Number(2)),
                    maximise: Some(false),
                },
            ),
        ];
        let outcome = merge(&rules, &window, no_facts());
        assert_eq!(outcome.matched, 2);
        assert_eq!(outcome.workspace, Some((2, WorkspaceSelector::Number(2))));
        assert_eq!(
            outcome.maximise,
            Some(false),
            "a later Some(false) overrides an earlier true"
        );
    }

    #[test]
    fn a_rule_with_no_fields_still_counts_as_matched() {
        let window = window("app", "t", WindowKind::Normal);
        let outcome = merge(&[Rule::default()], &window, no_facts());
        assert_eq!(outcome.matched, 1);
        assert_eq!(outcome.workspace, None);
        assert_eq!(outcome.maximise, None);
    }

    #[test]
    fn a_full_list_is_accepted_and_one_more_is_refused() {
        let full = vec![Rule::default(); MAX_RULES];
        assert!(validate_list(&full).is_ok());
        let over = vec![Rule::default(); MAX_RULES + 1];
        assert_eq!(
            validate_list(&over),
            Err(RuleError::TooManyRules {
                count: MAX_RULES + 1,
                max: MAX_RULES,
            })
        );
    }

    #[test]
    fn workspace_number_zero_is_refused_and_named() {
        let matcher = vec![rule(
            Match {
                workspace_number: Some(0),
                ..Match::default()
            },
            RuleFields::default(),
        )];
        let error = validate_list(&matcher).expect_err("workspace_number 0 is refused");
        assert_eq!(
            error,
            RuleError::InvalidField {
                rule: 0,
                field: "match.workspace_number",
                reason: "must be at least 1, because workspaces are numbered from 1",
            }
        );
        assert!(error.to_string().starts_with("window rule 1: match.workspace_number "));

        let placement = vec![
            Rule::default(),
            rule(
                Match::default(),
                RuleFields {
                    workspace: Some(WorkspaceSelector::Number(0)),
                    maximise: None,
                },
            ),
        ];
        let error = validate_list(&placement).expect_err("fields.workspace 0 is refused");
        assert!(error.to_string().starts_with("window rule 2: fields.workspace "));
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
