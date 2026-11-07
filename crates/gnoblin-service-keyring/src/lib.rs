//! The Secret Service provider on `org.freedesktop.secrets`: the keyring's face on the session bus.
//!
//! Clients such as libsecret and the browsers open a session, search for items and read secrets
//! through the `org.freedesktop.Secret.Service` interface. Only the `dh-ietf1024` session is
//! offered. A plain session is refused, so a secret never crosses the session bus in the clear.
//!
//! The store starts locked. The login password reaches it over the hand-off socket (see
//! [`handoff`]) from `pam_gnoblin_keyring`, or from a prompt when the login had none. Until then
//! every object is registered and the calls that need the store answer `IsLocked`.
//!
//! The name is requested with `DO_NOT_QUEUE`. When another service already owns it (gnome-keyring
//! or a second gnoblin), `serve` fails and nothing is registered, so the two can run side by side:
//! the one that lost the name simply does not serve. See `design/keyring.md`.

pub mod dh;
mod handoff;
mod objects;
mod unlock;

use std::path::Path;
use std::sync::{Arc, Mutex};

use futures_lite::future::block_on;
use zbus::Connection;
use zbus::fdo::RequestNameFlags;

pub use objects::SERVICE_PATH;
pub use unlock::{PasswordPrompt, PromptOutcome, UnlockError, UnlockKeySource};

gnoblin_log::logger!(log = "keyring-service");

/// The well-known name the Secret Service is served on.
pub const BUS_NAME: &str = "org.freedesktop.secrets";

/// Serve the Secret Service over the session bus, with the store at `store_path` and the unlock
/// hand-off on `socket`.
///
/// The store starts locked. The objects are registered before the name is requested, so the
/// first call that reaches the name finds them all. The hand-off socket is bound only after the
/// name is ours, so a service that lost the name never accepts a password. If the name is already
/// taken, the error is returned and the connection is dropped, which removes the objects again.
/// The returned connection serves until it is dropped.
pub async fn serve(store_path: &Path, socket: &Path) -> zbus::Result<Connection> {
    let state = objects::new_state(None);
    let connection = Connection::session().await?;
    connection
        .object_server()
        .at(
            SERVICE_PATH,
            objects::ServiceObject {
                state: Arc::clone(&state),
            },
        )
        .await?;
    connection
        .request_name_with_flags(BUS_NAME, RequestNameFlags::DO_NOT_QUEUE)
        .await?;
    let exporter = connection.clone();
    let exported = Arc::clone(&state);
    // Called on the hand-off thread once the store is open, so the collections appear on the bus.
    let on_unlock = Box::new(move || {
        if let Err(error) = block_on(objects::export_tree(&exporter.object_server(), &exported)) {
            log::warn!("the collections could not be exported: {error:?}");
        }
    });
    let source = Arc::new(Mutex::new(UnlockKeySource::new(
        store_path.to_path_buf(),
        state,
        on_unlock,
    )));
    handoff::listen(socket, source).map_err(|error| zbus::Error::Failure(error.to_string()))?;
    log::info!(name = BUS_NAME, path = SERVICE_PATH, "serving the Secret Service");
    Ok(connection)
}
