//! The overview: dims the windows of the workspaces that are not shown when it is off, and shows every window at full strength when it is
//! on. It changes opacity only: it draws no thumbnails and moves no windows.

use std::rc::Rc;

use gnoblin_object::{Fail, Res};

use crate::ui::widget::Ui;

/// What a workspace shows of itself to the overview: whether it is shown, and its windows.
struct WorkspaceWindows {
    shown: bool,
    clients: Vec<u64>,
}

/// Flips the overview and sets the opacity of every window of every workspace to match. Answers whether it is on.
///
/// # Errors
///
/// The library cannot start its desktop.
pub(crate) fn toggle(ui: &Rc<Ui>) -> Res<bool> {
    let ctx = ui.ctx;
    crate::events::open(ctx).map_err(Fail::from)?;
    let groups: Vec<WorkspaceWindows> = crate::table::read(ctx, |st| {
        Ok(st
            .workspaces
            .iter()
            .filter_map(|id| {
                st.ws.get(id).map(|workspace| WorkspaceWindows {
                    shown: st.is_shown(*id),
                    clients: workspace.clients.clone(),
                })
            })
            .collect())
    })
    .map_err(Fail::from)?;
    let (on, dimmed) = {
        let mut shell = ui.shell.borrow_mut();
        shell.overview.on = !shell.overview.on;
        (shell.overview.on, shell.overview.dimmed)
    };
    for group in groups {
        let opacity = if on || group.shown { 1.0 } else { dimmed };
        for client in group.clients {
            if let Err(error) = ui.node(client).set("opacity", opacity) {
                ui.warn(&format!(
                    "Overview.toggle: the window {client} refused its opacity ({error})"
                ));
            }
        }
    }
    Ok(on)
}
