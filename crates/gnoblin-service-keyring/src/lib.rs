//! The Secret Service provider on `org.freedesktop.secrets`: the keyring's face on the session bus.
//!
//! Clients such as libsecret and the browsers open a session, search for items and read secrets
//! through the `org.freedesktop.Secret.Service` interface. Only the `dh-ietf1024` session is
//! offered. A plain session is refused, so a secret never crosses the session bus in the clear.
//!
//! The name is requested with `DO_NOT_QUEUE`. When another service already owns it (gnome-keyring
//! or a second gnoblin), `serve` fails and nothing is registered, so the two can run side by side:
//! the one that lost the name simply does not serve. See `design/keyring.md`.

pub mod dh;
mod objects;

use gnoblin_keyring_store::Keyring;
use zbus::Connection;
use zbus::fdo::RequestNameFlags;

pub use objects::SERVICE_PATH;

gnoblin_log::logger!(log = "keyring-service");

/// The well-known name the Secret Service is served on.
pub const BUS_NAME: &str = "org.freedesktop.secrets";

/// Serve the Secret Service over the session bus, backed by `store`.
///
/// `None` serves a locked keyring: every object is registered, but the calls that need the store
/// answer `org.freedesktop.Secret.Error.IsLocked`. The returned connection serves until it is
/// dropped.
///
/// The objects are registered before the name is requested, so the first call that reaches the
/// name finds them all. If the name is already taken, the error is returned and the connection is
/// dropped, which removes the objects again.
pub async fn serve(store: Option<Keyring>) -> zbus::Result<Connection> {
    let state = objects::new_state(store);
    objects::ensure_default_collection(&state).map_err(|error| zbus::Error::Failure(format!("{error:?}")))?;
    let connection = Connection::session().await?;
    let server = connection.object_server();
    server
        .at(SERVICE_PATH, objects::ServiceObject { state: state.clone() })
        .await?;
    objects::export_tree(server, &state)
        .await
        .map_err(|error| zbus::Error::Failure(format!("{error:?}")))?;
    connection
        .request_name_with_flags(BUS_NAME, RequestNameFlags::DO_NOT_QUEUE)
        .await?;
    log::info!(name = BUS_NAME, path = SERVICE_PATH, "serving the Secret Service");
    Ok(connection)
}
