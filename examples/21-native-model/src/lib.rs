//! Example 21: the object model of `libgnoblin-core`.
//!
//! The windows are not nodes this shell walks: `gnoblin.clients` is a live collection, and a window is an object with properties and
//! signals. This shell uses it the way a window manager would:
//!
//! - reads the windows that are there and logs how many;
//! - makes a selection of the windows of one application (`where`) that follows its members, and logs each one that joins it;
//! - listens to the titles of every window through the collection, and to the closing of every window through the `gnoblin` object.
//!
//! Every line it logs starts with `model:`. A listener is told from the library's turn, after the call or the change that caused it, and
//! a window's own listeners are told before the collections that hold it. Build it with `cargo build -p gnoblin-example-native-model`
//! and run it with `gnoblin ./target/debug/libnative_model.so`.

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{ClientQuery, ModelRoot};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = ModelRoot::get(&objects)?;
    let Some(clients) = model.clients()? else {
        g.log("model: the core has no clients");
        return Ok(());
    };
    g.log(&format!("model: {} windows at the start", clients.length()?));

    // A selection follows its members: a window joins it when its application id matches, and leaves when it stops matching.
    let terminals = clients.where_(
        &ClientQuery {
            app_id: Some("foot".to_owned()),
            ..ClientQuery::default()
        },
        None,
    )?;
    terminals
        .on_added(move |client, _| {
            g.log(&format!(
                "model: a terminal joined: {}",
                client.title().unwrap_or_default()
            ));
        })?
        .keep();
    // A selection nobody holds is freed with its listeners: this shell keeps it for as long as it runs.
    std::mem::forget(terminals);
    clients
        .on_title_changed(move |client, title| {
            g.log(&format!(
                "model: window {} is now titled {title:?}",
                client.id().unwrap_or_default()
            ));
        })?
        .keep();
    model
        .on_client_closed(move |client| {
            g.log(&format!(
                "model: window {} closed, last titled {:?}",
                client.id().unwrap_or_default(),
                client.title().unwrap_or_default()
            ));
        })?
        .keep();
    g.log("model: running");
    Ok(())
}
