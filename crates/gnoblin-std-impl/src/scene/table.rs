//! The scene functions of the std table.
#![allow(unsafe_code)]

use gnoblin_core::host::NodeId;
use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinNode};
use gnoblin_std_abi::{GnoblinStdMembers, GnoblinStdProp};

use super::Kind;
use crate::animation::table::{prop_in, prop_struct};
use crate::error::Fail;
// The scene functions need no desktop: they open the library's model alone (`state::lookup_quiet`).
use crate::table::{call, fill_out, mutate_quiet as mutate, put, quiet_of as std_of, read_quiet as read, utf8};

pub(crate) unsafe extern "C" fn scene_new(ctx: GnoblinCtx, kind: u32, out: *mut GnoblinNode) -> i32 {
    call("scene_new", || {
        let kind = match kind {
            gnoblin_std_abi::STD_SCENE_NODE => Kind::Node,
            gnoblin_std_abi::STD_SCENE_VIEW => Kind::View,
            gnoblin_std_abi::STD_SCENE_CAMERA3D => Kind::Camera3D,
            gnoblin_std_abi::STD_SCENE_CAMERA2D => Kind::Camera2D,
            other => return Err(Fail::type_error(format!("{other} is not a kind of scene node"))),
        };
        let id = mutate(ctx, |st| st.scene_new(kind))?;
        // SAFETY: the caller guarantees `out` is null or a writable node id.
        unsafe { put(out, id) }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn scene_destroy(ctx: GnoblinCtx, node: GnoblinNode) -> i32 {
    call("scene_destroy", || {
        mutate(ctx, |st| st.scene_destroy(node))?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn scene_insert(
    ctx: GnoblinCtx,
    parent: GnoblinNode,
    node: GnoblinNode,
    index: i64,
) -> i32 {
    call("scene_insert", || {
        let std = std_of(ctx)?;
        let g = std.g();
        let at = usize::try_from(index).ok();
        g.node(NodeId(node)).insert_into(&g.node(NodeId(parent)), at)?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn scene_root(ctx: GnoblinCtx, out: *mut GnoblinNode) -> i32 {
    call("scene_root", || {
        let root = mutate(ctx, |st| {
            let root = st.g.root()?.id().0;
            // The root takes the writes of any node (a script mounts nodes under it and may move it), so the library knows it as one.
            st.scene.kinds.entry(root).or_insert(Kind::Node);
            Ok(root)
        })?;
        // SAFETY: the caller guarantees `out` is null or a writable node id.
        unsafe { put(out, root) }?;
        Ok(abi::E_OK)
    })
}

thread_local! {
    /// The children `scene_children` lent, valid until the next call on this thread.
    static LENT_CHILDREN: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub(crate) unsafe extern "C" fn scene_children(ctx: GnoblinCtx, node: GnoblinNode, out: *mut GnoblinStdMembers) -> i32 {
    call("scene_children", || {
        let std = std_of(ctx)?;
        let children = std.g().node(NodeId(node)).children()?;
        let (ptr, len) = LENT_CHILDREN.with(|lent| {
            *lent.borrow_mut() = children.iter().map(|child| child.id().0).collect();
            let lent = lent.borrow();
            (lent.as_ptr(), lent.len())
        });
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe {
            put(
                out,
                GnoblinStdMembers {
                    ptr: if len == 0 { core::ptr::null() } else { ptr },
                    len,
                },
            )
        }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn scene_parent(ctx: GnoblinCtx, node: GnoblinNode, out: *mut GnoblinNode) -> i32 {
    call("scene_parent", || {
        let std = std_of(ctx)?;
        let parent = std.g().node(NodeId(node)).parent()?.map_or(0, |parent| parent.id().0);
        // SAFETY: the caller guarantees `out` is null or a writable node id.
        unsafe { put(out, parent) }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn scene_set(
    ctx: GnoblinCtx,
    node: GnoblinNode,
    key: *const u8,
    key_len: usize,
    value: *const GnoblinStdProp,
) -> i32 {
    call("scene_set", || {
        // SAFETY: the caller guarantees `key` is `key_len` readable bytes and `value` a readable struct.
        let (key, value) = unsafe { (utf8(key, key_len)?, prop_in(value)?) };
        mutate(ctx, |st| st.scene_set(node, key, value))?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn scene_get(
    ctx: GnoblinCtx,
    node: GnoblinNode,
    key: *const u8,
    key_len: usize,
    out: *mut GnoblinStdProp,
) -> i32 {
    call("scene_get", || {
        // SAFETY: the caller guarantees `key` is `key_len` readable bytes.
        let key = unsafe { utf8(key, key_len) }?;
        let value = read(ctx, |st| st.scene_get(node, key))?;
        let raw = prop_struct(value.as_ref());
        // SAFETY: the caller guarantees `out` is a writable struct whose first field is its size.
        unsafe { fill_out(out, &raw, "scene_get") }?;
        Ok(abi::E_OK)
    })
}
