//! What a shell that keeps its own objects asks of the desk, beyond the objects themselves: the facts about an output, the room the
//! shell's bars take from it, a workspace stretched over several outputs, the areas a window is in, and the window a reload gives
//! back to its workspace.
//!
//! The TypeScript library reads these from its own desk table (`host/desks.ts`, `work_area.ts`, `displays.ts`); with the library's
//! desks behind its workspaces, a shell reaches them here.
#![allow(unsafe_code)]

use gnoblin_core::Rect;
use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinNode};
use gnoblin_std_abi::{
    GnoblinStdAreas, GnoblinStdOutputInfo, GnoblinStdRect, GnoblinStdSpan, GnoblinStdWorkspace, STD_AREAS_HAS_AREA,
    STD_AREAS_HAS_OUTPUT, STD_OUTPUT_HAS_DESK, STD_OUTPUT_HAS_RECT, STD_OUTPUT_PARKED,
};

use crate::desks::output_rect;
use crate::error::{Fail, Res};
use crate::state::{NodeId, State};
use crate::table::{call, fill_out, mutate, read, read_ext};

fn rect_out(rect: &Rect) -> GnoblinStdRect {
    GnoblinStdRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

impl State {
    /// Whether `output` is one the library tracks.
    pub(crate) fn tracks_output(&self, output: NodeId) -> bool {
        self.outputs.contains(&output)
    }

    /// What the desk knows about `output`.
    pub(crate) fn output_info_of(&self, output: NodeId) -> GnoblinStdOutputInfo {
        let desk = self.desk_of_output(output).and_then(|id| self.desk(id));
        let mut flags = 0;
        if let Some(desk) = desk {
            flags |= STD_OUTPUT_HAS_DESK;
            if desk.parked {
                flags |= STD_OUTPUT_PARKED;
            }
        }
        let shown = desk
            .map(|desk| desk.spanned_by.unwrap_or(desk.id))
            .and_then(|id| self.desk(id))
            .and_then(|desk| desk.shown);
        let (rect, area) = match output_rect(&self.g, output) {
            Some(rect) => {
                flags |= STD_OUTPUT_HAS_RECT;
                (rect, self.area_of(output).unwrap_or(rect))
            }
            None => (Rect::default(), Rect::default()),
        };
        GnoblinStdOutputInfo {
            size: size_of::<GnoblinStdOutputInfo>() as u32,
            flags,
            workspace: shown.unwrap_or(0),
            rect: rect_out(&rect),
            area: rect_out(&area),
        }
    }

    /// The room the shell's bars take from the top and the bottom of `output` changed: every workspace is laid out in what is left.
    pub(crate) fn set_insets(&mut self, output: NodeId, top: f64, bottom: f64) {
        let before = self.insets.get(&output).copied().unwrap_or((0.0, 0.0));
        if before == (top, bottom) {
            return;
        }
        if (top, bottom) == (0.0, 0.0) {
            self.insets.remove(&output);
        } else {
            self.insets.insert(output, (top, bottom));
        }
        for ws in self.ws.values_mut() {
            ws.dirty = true;
        }
    }

    /// The output a client is on and the room it may be laid out in (for client 0, the primary output's).
    pub(crate) fn client_areas_raw(&self, client: NodeId) -> Res<GnoblinStdAreas> {
        if client != 0 && !self.rec.contains_key(&client) {
            return Err(Fail::new(abi::E_NO_SUCH_NODE, format!("node {client} is not a client")));
        }
        let (output, area) = if client == 0 {
            (
                self.primary_output().and_then(|output| output_rect(&self.g, output)),
                self.screen_area(),
            )
        } else {
            (self.output_rect_of(client), self.work_area_of_client(client))
        };
        let mut flags = 0;
        if output.is_some() {
            flags |= STD_AREAS_HAS_OUTPUT;
        }
        if area.is_some() {
            flags |= STD_AREAS_HAS_AREA;
        }
        Ok(GnoblinStdAreas {
            size: size_of::<GnoblinStdAreas>() as u32,
            flags,
            output: rect_out(&output.unwrap_or_default()),
            area: rect_out(&area.unwrap_or_default()),
        })
    }
}

pub(crate) unsafe extern "C" fn output_info(
    ctx: GnoblinCtx,
    output: GnoblinNode,
    out: *mut GnoblinStdOutputInfo,
) -> i32 {
    call("output_info", || {
        let info = read(ctx, |st| {
            if st.tracks_output(output) {
                Ok(st.output_info_of(output))
            } else {
                Err(Fail::new(
                    abi::E_NO_SUCH_NODE,
                    format!("node {output} is not an output"),
                ))
            }
        })?;
        // SAFETY: the caller guarantees `out` is null or a `GnoblinStdOutputInfo` whose first field is its size.
        unsafe { fill_out(out, &info, "output info") }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn output_set_insets(ctx: GnoblinCtx, output: GnoblinNode, top: f64, bottom: f64) -> i32 {
    call("output_set_insets", || {
        if !(top.is_finite() && bottom.is_finite() && top >= 0.0 && bottom >= 0.0) {
            return Err(Fail::type_error(
                "the room bars take must be two finite numbers of at least 0",
            ));
        }
        mutate(ctx, |st| {
            if !st.tracks_output(output) {
                return Err(Fail::new(
                    abi::E_NO_SUCH_NODE,
                    format!("node {output} is not an output"),
                ));
            }
            st.set_insets(output, top, bottom);
            Ok(abi::E_OK)
        })
    })
}

pub(crate) unsafe extern "C" fn output_span(ctx: GnoblinCtx, span: *const GnoblinStdSpan) -> i32 {
    call("output_span", || {
        // SAFETY: the caller guarantees `span` is null or a `GnoblinStdSpan` whose first field is its size.
        let span = unsafe { read_ext(span, "output span") }?;
        let members: Vec<NodeId> = if span.members.is_null() || span.n_members == 0 {
            Vec::new()
        } else {
            // SAFETY: the caller guarantees `n_members` readable nodes at `members`.
            unsafe { std::slice::from_raw_parts(span.members, span.n_members) }.to_vec()
        };
        mutate(ctx, |st| {
            if !st.tracks_output(span.lead) {
                return Err(Fail::new(
                    abi::E_NO_SUCH_NODE,
                    format!("node {} is not an output", span.lead),
                ));
            }
            st.span_desks(span.lead, Some(&members));
            Ok(abi::E_OK)
        })
    })
}

pub(crate) unsafe extern "C" fn client_areas(ctx: GnoblinCtx, client: GnoblinNode, out: *mut GnoblinStdAreas) -> i32 {
    call("client_areas", || {
        let areas = read(ctx, |st| st.client_areas_raw(client))?;
        // SAFETY: the caller guarantees `out` is null or a `GnoblinStdAreas` whose first field is its size.
        unsafe { fill_out(out, &areas, "client areas") }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn workspace_restore_client(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    client: GnoblinNode,
    arrived: u32,
) -> i32 {
    call("workspace_restore_client", || {
        mutate(ctx, |st| {
            st.restore_client(workspace, client, arrived != 0).map(i32::from)
        })
    })
}

pub(crate) unsafe extern "C" fn client_restore_snap(
    ctx: GnoblinCtx,
    client: GnoblinNode,
    zone: u32,
    floating: *const GnoblinStdRect,
) -> i32 {
    call("client_restore_snap", || {
        // SAFETY: the caller guarantees `floating` is null or a readable `GnoblinStdRect`.
        let Some(floating) = (unsafe { floating.as_ref() }) else {
            return Err(Fail::err("client_restore_snap: a null rectangle"));
        };
        let floating = Rect::new(floating.x, floating.y, floating.width, floating.height);
        mutate(ctx, |st| {
            let Some(rec) = st.rec.get_mut(&client) else {
                return Err(Fail::new(abi::E_NO_SUCH_NODE, format!("node {client} is not a client")));
            };
            rec.snap = Some(crate::client_state::Snap { zone, floating });
            Ok(abi::E_OK)
        })
    })
}
