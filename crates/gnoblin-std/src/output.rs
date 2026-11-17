//! [`Output`]: a connected monitor, as the std library sees it.

#![allow(unsafe_code)]

use gnoblin_core::{Node, NodeId, Rect};
use gnoblin_std_abi as abi;

use crate::api::Std;
use crate::collection::Member;
use crate::error::Result;
use crate::workspace::Workspace;

/// An output (a monitor) of the compositor. Cheap to clone and compare (by node id).
#[derive(Clone)]
pub struct Output {
    std: Std,
    node: Node,
}

impl std::fmt::Debug for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Output").field(&self.node.id().0).finish()
    }
}

impl PartialEq for Output {
    fn eq(&self, other: &Output) -> bool {
        self.node.id() == other.node.id()
    }
}

impl Eq for Output {}

impl std::ops::Deref for Output {
    type Target = Node;

    fn deref(&self) -> &Node {
        &self.node
    }
}

impl Output {
    pub(crate) fn from_node(std: &Std, node: Node) -> Output {
        Output { std: std.clone(), node }
    }

    /// The core node of this output.
    pub fn node(&self) -> &Node {
        &self.node
    }

    /// The node id, which is the output's `id` in a query.
    pub fn id(&self) -> NodeId {
        self.node.id()
    }

    /// The name the compositor gives the output.
    ///
    /// # Errors
    ///
    /// The output is gone, or the generation retired.
    pub fn name(&self) -> Result<String> {
        Ok(self.node.get_text("name")?)
    }

    /// The connector the backend knows the output by (`DP-1`, `eDP-1`).
    ///
    /// # Errors
    ///
    /// The output is gone, or the generation retired.
    pub fn connector(&self) -> Result<String> {
        Ok(self.node.get_text("connector")?)
    }

    /// The current mode as `WxH@Hz`.
    ///
    /// # Errors
    ///
    /// The output is gone, or the generation retired.
    pub fn mode(&self) -> Result<String> {
        Ok(self.node.get_text("mode")?)
    }

    /// Whether this is the primary output.
    ///
    /// # Errors
    ///
    /// The output is gone, or the generation retired.
    pub fn is_primary(&self) -> Result<bool> {
        Ok(self.node.get_bool("primary")?)
    }

    /// The output's rectangle in the compositor's logical pixels.
    ///
    /// # Errors
    ///
    /// The output is gone, or the generation retired.
    pub fn rect(&self) -> Result<Rect> {
        let size = |own: &str, screen: &str| -> Result<f64> {
            let value = self.node.get_f64(own)?;
            Ok(if value != 0.0 {
                value
            } else {
                self.node.get_f64(screen)?
            })
        };
        Ok(Rect::new(
            size("x", "screen_x")?,
            size("y", "screen_y")?,
            size("width", "screen_width")?,
            size("height", "screen_height")?,
        ))
    }

    /// The workspace this output shows, or `None` while it shows none (it mirrors another output, or no workspace exists).
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn workspace(&self) -> Result<Option<Workspace>> {
        let mut workspace = 0;
        // SAFETY: `output_workspace` writes one handle through the pointer it is given, a live local.
        self.std
            .check(unsafe { (self.std.api().output_workspace)(self.std.ctx(), self.node.id().0, &mut workspace) })?;
        Ok((workspace != 0).then(|| Workspace::from_handle(&self.std, workspace)))
    }

    /// Shows `workspace` on this output (see [`Workspace::show_on`]).
    ///
    /// # Errors
    ///
    /// The workspace is not in the list, or the generation retired.
    pub fn show(&self, workspace: &Workspace) -> Result<()> {
        workspace.show_on(self, crate::workspace::ShowOptions::default())
    }

    /// An output that is not connected any more is gone from reads.
    pub fn is_connected(&self) -> bool {
        self.node.is_alive()
    }
}

impl Member for Output {
    const KIND: u32 = abi::STD_COLLECTION_OUTPUTS;

    fn from_raw(std: &Std, raw: u64) -> Option<Output> {
        Some(Output::from_node(std, std.gnoblin().node(NodeId(raw))))
    }

    fn raw(&self) -> u64 {
        self.node.id().0
    }

    fn key(&self) -> String {
        self.node.id().0.to_string()
    }
}
