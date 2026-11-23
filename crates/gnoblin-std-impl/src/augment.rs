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
//!
//! `data/js/gnoblin-std/host/roots.ts` and `data/js/gnoblin-std/index.ts` are the reference: the TypeScript host makes the same members on the
//! root object when the package is loaded.

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
}
