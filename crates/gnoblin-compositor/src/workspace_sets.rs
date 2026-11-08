//! Turns the configuration's `workspace-sets` into the window manager's sets.
//!
//! The configuration checks the shape of each set when it is read. This module converts
//! the workspace names into the window manager's ids and refuses what it cannot resolve,
//! naming the field, so a stored document cannot select a workspace the session does not
//! have.

use crate::log;
use gnoblin_config::{Workspace, WorkspaceSet};
use gnoblin_types::WorkspaceId;
use gnoblin_wm::{WorkspaceSet as WmWorkspaceSet, WorkspaceSetEntry};

/// Converts a whole set list, refusing it if any set names a workspace it cannot resolve.
///
/// `declared` is the document's `workspaces` list. A workspace is named by its declared id,
/// and its window-manager id is its position in that list.
///
/// # Errors
///
/// Returns the message to show the caller, which names the field that is refused.
pub fn convert(sets: &[WorkspaceSet], declared: &[Workspace]) -> Result<Vec<WmWorkspaceSet>, String> {
    sets.iter()
        .enumerate()
        .map(|(index, set)| {
            let name = set
                .name
                .clone()
                .ok_or_else(|| format!("workspace-sets[{index}].name: a set needs a name"))?;
            let entries = set
                .workspaces
                .as_deref()
                .unwrap_or_default()
                .iter()
                .enumerate()
                .map(|(entry, workspace)| {
                    let field = format!("workspace-sets[{index}].workspaces[{entry}].workspace");
                    let declared_id = workspace
                        .workspace
                        .as_deref()
                        .ok_or_else(|| format!("{field}: a workspace id is required"))?;
                    Ok(WorkspaceSetEntry {
                        workspace: workspace_id(declared_id, declared, &field)?,
                        output: workspace.output.clone(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(WmWorkspaceSet { name, entries })
        })
        .collect()
}

/// Sets the compositor's workspace sets from the document's sets. A list this session
/// refuses leaves the sets empty and logs the field, so a stored document cannot select
/// workspaces the session does not have.
pub fn configure(config: &mut gnoblin_wayland::Config, sets: &[WorkspaceSet], declared: &[Workspace]) {
    match convert(sets, declared) {
        Ok(sets) => config.workspace_sets = sets,
        Err(message) => {
            log::warn!(area = "workspace-sets", %message, "the stored workspace sets are refused; no sets apply");
            config.workspace_sets = Vec::new();
        }
    }
}

/// Resolves a workspace named in a set.
///
/// A declared id resolves to its workspace: the window manager numbers workspaces from one,
/// so the workspace declared at position `i` is id `i + 1`. A number is taken as the id it
/// has always named. Any other text is refused, naming the field.
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

#[cfg(test)]
mod tests {
    use gnoblin_config::{Workspace, WorkspaceSet, WorkspaceSetEntry as ConfigEntry};
    use gnoblin_types::WorkspaceId;

    use super::convert;

    fn declared(ids: &[&str]) -> Vec<Workspace> {
        ids.iter()
            .map(|id| Workspace {
                id: Some((*id).to_owned()),
                name: Some(format!("{id} name")),
            })
            .collect()
    }

    fn set(name: &str, workspaces: &[(&str, Option<&str>)]) -> WorkspaceSet {
        WorkspaceSet {
            name: Some(name.to_owned()),
            workspaces: Some(
                workspaces
                    .iter()
                    .map(|(workspace, output)| ConfigEntry {
                        workspace: Some((*workspace).to_owned()),
                        output: output.map(str::to_owned),
                    })
                    .collect(),
            ),
        }
    }

    #[test]
    fn a_declared_name_resolves_to_its_position_plus_one() {
        let sets = [set("work", &[("code", Some("DP-1")), ("mail", None)])];
        let converted = convert(&sets, &declared(&["code", "mail"])).expect("the set converts");
        assert_eq!(converted[0].name, "work");
        assert_eq!(converted[0].entries[0].workspace, WorkspaceId::new(1));
        assert_eq!(converted[0].entries[0].output.as_deref(), Some("DP-1"));
        assert_eq!(converted[0].entries[1].workspace, WorkspaceId::new(2));
        assert_eq!(converted[0].entries[1].output, None);
    }

    #[test]
    fn a_numeric_id_is_taken_as_the_id_it_names() {
        let sets = [set("numbered", &[("3", None)])];
        let converted = convert(&sets, &declared(&["code"])).expect("the set converts");
        assert_eq!(converted[0].entries[0].workspace, WorkspaceId::new(3));
    }

    #[test]
    fn an_undeclared_name_refuses_the_whole_list() {
        let sets = [set("work", &[("code", None)]), set("bad", &[("nowhere", None)])];
        let message = convert(&sets, &declared(&["code"])).expect_err("the list is refused");
        assert!(
            message.contains("workspace-sets[1].workspaces[0].workspace"),
            "{message}"
        );
        assert!(message.contains("nowhere"), "{message}");
    }
}
