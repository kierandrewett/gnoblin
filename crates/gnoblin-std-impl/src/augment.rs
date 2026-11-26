//! The members `libgnoblin-std` adds to classes of the core's catalog: the `[[augment]]` blocks of the std schema.
//!
//! The catalog lowers the augments of one core class into one service of `gnoblin.std` (`gnoblin_augment` for `core.gnoblin`,
//! `output_collection_augment` for `core.output_collection`). The generators install each member on the core's class of every language
//! when the std module is loaded, as a call of this service for the generation that is running, so a shell that loads no std library has
//! none of them and the core has no symbol of this library. What the members answer is what the libraries' own services answer:
//!
//! | member | answers |
//! |---|---|
//! | `gnoblin.workspaces` | the service `workspaces` (`area`, `show`, the list) |
//! | `gnoblin.displays` | the service `displays` (the list, `replace`) |
//! | `gnoblin.scene` | the service `scene` |
//! | `gnoblin.gestures` | the service `gestures` |
//! | `gnoblin.requestTransition` | the same call as `transition.request` |
//! | `gnoblin.outputs.profiles` | the service `output_profiles` |
//! | `input.activeDisplay` | the display under the context's pointer (`display_objects::manager`) |
//! | `collection.style` | the style of the collection's windows (`style::objects`) |
//! | `client.style`, `computedStyle`, `styleWarnings` | the window's style, as `ClientStyle.ofClient`, `computed`, `warnings` |
//! | `client.workspace`, `moveToWorkspace` | the window's workspace, and a move to another (`workspace_objects`) |
//! | `client.moveWithKeys`, `resizeWithKeys` | the keyboard mode of `Repositioning` for the window (`policy`) |
//! | `output.workspace`, `show` | the workspace the output shows, and showing one there (`workspace_objects`) |
//!
//! The members of `input`, `collection`, `client` and `output` are `receiver = true` augments: each call is handed the core object it
//! acts on first.
//!
//! `data/js/gnoblin-std/host/roots.ts`, `data/js/gnoblin-std/host/members.ts` and `data/js/gnoblin-std/index.ts` are the reference: the
//! TypeScript host makes the same members when the package is loaded.

use gnoblin_object::server::Builder;

/// Implements the classes that hold the members of an augment.
pub(crate) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<()>("gnoblin_augment")
        .ctor(|_, _| Ok(()))
        .get("workspaces", |_, cx| cx.service("workspaces"))
        .get("displays", |_, cx| cx.service("displays"))
        .get("scene", |_, cx| cx.service("scene"))
        .get("gestures", |_, cx| cx.service("gestures"))
        .method("request_transition", |_, cx, args| {
            crate::animation::objects::request(cx, args)
        });
    let _ = builder
        .class::<()>("output_collection_augment")
        .ctor(|_, _| Ok(()))
        .get("profiles", |_, cx| cx.service("output_profiles"));
    crate::style::objects::register_collection_augment(builder);
    let _ = builder
        .class::<()>("client_augment")
        .ctor(|_, _| Ok(()))
        .method("style", |_, cx, args| crate::style::objects::client_style(cx, &args[0]))
        .method("computed_style", |_, cx, args| {
            crate::style::objects::client_computed_style(cx, &args[0])
        })
        .method("style_warnings", |_, cx, args| {
            crate::style::objects::client_style_warnings(cx, &args[0])
        })
        .method("workspace", |_, cx, args| {
            crate::workspace_objects::client_workspace(cx, &args[0])
        })
        .method("move_to_workspace", |_, cx, args| {
            crate::workspace_objects::move_client(cx, &args[0], &args[1])
        })
        .method("move_with_keys", |_, cx, args| {
            crate::policy::reposition_with_keys(cx, &args[0], false)
        })
        .method("resize_with_keys", |_, cx, args| {
            crate::policy::reposition_with_keys(cx, &args[0], true)
        });
    let _ = builder
        .class::<()>("output_augment")
        .ctor(|_, _| Ok(()))
        .method("workspace", |_, cx, args| {
            crate::workspace_objects::output_workspace(cx, &args[0])
        })
        .method("show", |_, cx, args| {
            crate::workspace_objects::show_on_output(cx, &args[0], &args[1], &args[2])?;
            Ok(gnoblin_object::Val::None)
        });
}
