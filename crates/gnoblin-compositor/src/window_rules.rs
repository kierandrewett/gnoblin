//! Turns the configuration's `window-rules` into the window manager's rules.
//!
//! The configuration checks the shape of each rule when it is read. This module converts
//! the values the window manager needs and refuses what this session cannot carry out,
//! naming the field, so a commit fails at the door instead of being accepted and then
//! ignored.

use crate::log;
use gnoblin_config::{RuleClientType, RuleWorkspace, WindowMatch, WindowRule, WindowRuleType, Workspace};
use gnoblin_types::WorkspaceId;
use gnoblin_wm::{Match, Pattern, Rule, RuleFields, WindowProtocol, WorkspaceSelector, validate_list};

/// Converts a whole rule list, refusing it if any rule cannot be carried out.
///
/// `declared` is the document's `workspaces` list. A workspace is named by its declared id,
/// and its window-manager id is its position in that list.
///
/// # Errors
///
/// Returns the message to show the caller, which names the field that is refused.
pub fn convert(rules: &[WindowRule], declared: &[Workspace]) -> Result<Vec<Rule>, String> {
    let converted = rules
        .iter()
        .enumerate()
        .map(|(index, rule)| convert_rule(index, rule, declared))
        .collect::<Result<Vec<_>, _>>()?;
    validate_list(&converted).map_err(|error| error.to_string())?;
    Ok(converted)
}

/// Sets the compositor's rule list from the document's rules. A list this session refuses
/// leaves the rules empty and logs the field, so a stored document cannot place windows
/// by a rule the session would not carry out.
pub fn configure(config: &mut gnoblin_wayland::Config, rules: &[WindowRule], declared: &[Workspace]) {
    match convert(rules, declared) {
        Ok(rules) => config.window_rules = rules,
        Err(message) => {
            log::warn!(area = "window-rules", %message, "the stored window rules are refused; no rules apply");
            config.window_rules = Vec::new();
        }
    }
}

fn convert_rule(index: usize, rule: &WindowRule, declared: &[Workspace]) -> Result<Rule, String> {
    let matcher = match &rule.r#match {
        Some(matcher) => convert_match(index, matcher, declared)?,
        None => Match::default(),
    };
    let workspace = rule
        .workspace
        .as_ref()
        .map(|workspace| convert_workspace(index, workspace, declared))
        .transpose()?;
    Ok(Rule {
        matcher,
        fields: RuleFields {
            workspace,
            maximise: rule.maximise,
            secure: rule.secure,
            opacity: rule.opacity.map(|opacity| opacity as f32),
            corner_radius: rule
                .corners
                .as_ref()
                .and_then(|corners| corners.radius)
                .map(|radius| radius as f32),
        },
    })
}

fn convert_match(index: usize, matcher: &WindowMatch, declared: &[Workspace]) -> Result<Match, String> {
    let field = |name: &str| format!("window-rules[{index}].match.{name}");
    // A layer rule is refused by the configuration already; it is repeated here so the
    // window manager is never handed a matcher it would silently fail to match.
    if matches!(matcher.r#type, Some(WindowRuleType::Layer)) || matcher.layer.is_some() {
        return Err(format!(
            "{}: layer rules are not supported by this session",
            field("type")
        ));
    }
    Ok(Match {
        // Every managed toplevel is a window; dialogs and modals are matched by their
        // parent and not by kind here, so no kind is required.
        kind: None,
        app_id: pattern(matcher.app_id.as_deref(), &field("app-id"))?,
        title: pattern(matcher.title.as_deref(), &field("title"))?,
        focused: matcher.focused,
        layer: None,
        workspace_id: matcher
            .workspace_id
            .as_deref()
            .map(|text| workspace_id(text, declared, &field("workspace-id")))
            .transpose()?,
        workspace_number: matcher
            .workspace_number
            .map(|number| workspace_number(number, &field("workspace-number")))
            .transpose()?,
        client_type: matcher.client_type.as_ref().map(|client| match client {
            RuleClientType::Wayland => WindowProtocol::Wayland,
            RuleClientType::X11 => WindowProtocol::X11,
        }),
    })
}

fn convert_workspace(
    index: usize,
    workspace: &RuleWorkspace,
    declared: &[Workspace],
) -> Result<WorkspaceSelector, String> {
    let field = |name: &str| format!("window-rules[{index}].workspace.{name}");
    match (&workspace.id, workspace.number) {
        (Some(text), None) => Ok(WorkspaceSelector::Id(workspace_id(text, declared, &field("id"))?)),
        (None, Some(number)) => Ok(WorkspaceSelector::Number(workspace_number(number, &field("number"))?)),
        _ => Err(format!(
            "window-rules[{index}].workspace: name exactly one of id or number"
        )),
    }
}

fn pattern(source: Option<&str>, field: &str) -> Result<Option<Pattern>, String> {
    source
        .map(|text| Pattern::new(text).map_err(|error| format!("{field}: {error}")))
        .transpose()
}

/// Resolves a workspace id written in a rule.
///
/// A declared name resolves to its workspace: the window manager numbers workspaces from one,
/// so the workspace declared at position `i` is id `i + 1`. A number is taken as the id it
/// has always named, so rules written against numbered workspaces, and against workspaces
/// that dynamic workspaces create, keep working. Any other text is refused, naming the field.
fn workspace_id(text: &str, declared: &[Workspace], field: &str) -> Result<WorkspaceId, String> {
    if let Some(position) = declared
        .iter()
        .position(|workspace| workspace.id.as_deref() == Some(text))
    {
        return Ok(WorkspaceId::new(position as u64 + 1));
    }
    text.parse::<u64>()
        .map(WorkspaceId::new)
        .map_err(|_| format!("{field}: no workspace named {text:?} is declared in `workspaces`"))
}

fn workspace_number(number: i64, field: &str) -> Result<u32, String> {
    u32::try_from(number).map_err(|_| format!("{field}: {number} is outside the workspace numbers"))
}

#[cfg(test)]
mod tests {
    use gnoblin_config::{RuleClientType, RuleWorkspace, WindowMatch, WindowRule, Workspace};
    use gnoblin_types::WorkspaceId;
    use gnoblin_wm::{WindowProtocol, WorkspaceSelector};

    use super::convert;

    fn rule_for(app_id: &str) -> WindowRule {
        WindowRule {
            r#match: Some(WindowMatch {
                app_id: Some(app_id.to_owned()),
                ..WindowMatch::default()
            }),
            workspace: None,
            maximise: Some(true),
            secure: None,
            opacity: None,
            corners: None,
        }
    }

    fn declared(ids: &[&str]) -> Vec<Workspace> {
        ids.iter()
            .map(|id| Workspace {
                id: Some((*id).to_owned()),
                name: Some(format!("{id} name")),
            })
            .collect()
    }

    #[test]
    fn a_rule_converts_its_match_and_fields() {
        let mut rule = rule_for("^firefox$");
        rule.r#match.as_mut().expect("the match").client_type = Some(RuleClientType::X11);
        rule.workspace = Some(RuleWorkspace {
            id: None,
            number: Some(2),
        });
        let rules = convert(&[rule], &[]).expect("the rule converts");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].matcher.client_type, Some(WindowProtocol::X11));
        assert_eq!(rules[0].fields.workspace, Some(WorkspaceSelector::Number(2)));
        assert_eq!(rules[0].fields.maximise, Some(true));
    }

    #[test]
    fn an_opacity_and_radius_carry_into_the_rule() {
        let mut rule = rule_for("^firefox$");
        rule.opacity = Some(0.5);
        rule.corners = Some(gnoblin_config::RuleCorners { radius: Some(8.0) });
        let rules = convert(&[rule], &[]).expect("the rule converts");
        assert_eq!(rules[0].fields.opacity, Some(0.5));
        assert_eq!(rules[0].fields.corner_radius, Some(8.0));
    }

    #[test]
    fn a_declared_workspace_name_resolves_to_its_position() {
        let mut rule = rule_for("^firefox$");
        rule.workspace = Some(RuleWorkspace {
            id: Some("web".to_owned()),
            number: None,
        });
        let rules = convert(&[rule], &declared(&["main", "web"])).expect("the declared name converts");
        assert_eq!(
            rules[0].fields.workspace,
            Some(WorkspaceSelector::Id(WorkspaceId::new(2)))
        );
    }

    #[test]
    fn a_match_on_a_declared_workspace_name_resolves_to_its_position() {
        let mut rule = rule_for("^firefox$");
        rule.r#match.as_mut().expect("the match").workspace_id = Some("main".to_owned());
        let rules = convert(&[rule], &declared(&["main", "web"])).expect("the declared name converts");
        assert_eq!(rules[0].matcher.workspace_id, Some(WorkspaceId::new(1)));
    }

    #[test]
    fn an_undeclared_workspace_name_is_refused_with_its_field() {
        let mut rule = rule_for("^firefox$");
        rule.workspace = Some(RuleWorkspace {
            id: Some("web".to_owned()),
            number: None,
        });
        let message = convert(&[rule], &declared(&["main"])).expect_err("an undeclared name is refused");
        assert!(message.starts_with("window-rules[0].workspace.id:"), "{message}");
        assert!(message.contains("\"web\""), "{message}");
    }

    #[test]
    fn a_match_on_an_undeclared_workspace_name_is_refused_with_its_field() {
        let mut rule = rule_for("^firefox$");
        rule.r#match.as_mut().expect("the match").workspace_id = Some("web".to_owned());
        let message = convert(&[rule], &declared(&["main"])).expect_err("an undeclared name is refused");
        assert!(message.starts_with("window-rules[0].match.workspace-id:"), "{message}");
    }

    #[test]
    fn a_numeric_workspace_id_is_kept_as_it_was_written() {
        let mut rule = rule_for("^firefox$");
        rule.workspace = Some(RuleWorkspace {
            id: Some("5".to_owned()),
            number: None,
        });
        let rules = convert(&[rule], &declared(&["main", "web"])).expect("a number converts");
        assert_eq!(
            rules[0].fields.workspace,
            Some(WorkspaceSelector::Id(WorkspaceId::new(5)))
        );
    }

    #[test]
    fn a_bad_pattern_is_refused_with_its_field() {
        let message = convert(&[rule_for("[")], &[]).expect_err("an unclosed set is refused");
        assert!(message.starts_with("window-rules[0].match.app-id:"), "{message}");
    }

    #[test]
    fn a_rule_naming_no_workspace_or_two_is_refused() {
        let mut rule = rule_for("^a$");
        rule.workspace = Some(RuleWorkspace { id: None, number: None });
        let message = convert(&[rule], &[]).expect_err("a rule must name one workspace");
        assert!(message.contains("name exactly one of id or number"), "{message}");
    }
}
