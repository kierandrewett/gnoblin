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
pub mod secret;
mod unlock;

use std::fmt;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use futures_lite::future::block_on;
use zbus::Connection;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::names::BusName;

pub use objects::SERVICE_PATH;
pub use secret::{AppSecret, AppSecretError, Secrets};
pub use unlock::{PasswordOffer, PromptId, PromptOutcome, PromptRefusal, UnlockError, UnlockKeySource};

gnoblin_log::logger!(log = "keyring-service");

/// The well-known name the Secret Service is served on.
pub const BUS_NAME: &str = "org.freedesktop.secrets";

/// Why the Secret Service is not served. Each case is named so the log line says what the operator
/// should look at, and none of them ever leads to a second owner of the name.
#[derive(Debug)]
pub enum ServeError {
    /// Another connection owns [`BUS_NAME`]. `owner` is that connection's unique bus name, or why
    /// it could not be read.
    NameTaken { owner: String },
    /// The session bus refused the connection, an export or the name request.
    Bus(zbus::Error),
    /// The hand-off socket could not be bound, so the store could never be unlocked.
    Socket(io::Error),
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NameTaken { owner } => write!(f, "{BUS_NAME} is already owned by {owner}"),
            Self::Bus(error) => write!(f, "the session bus refused the Secret Service: {error}"),
            Self::Socket(error) => write!(f, "the unlock hand-off socket could not be bound: {error}"),
        }
    }
}

/// Serve the Secret Service over the session bus, with the store at `store_path` and the unlock
/// hand-off on `socket`.
///
/// The store starts locked. The objects are registered before the name is requested, so the
/// first call that reaches the name finds them all. The hand-off socket is bound only after the
/// name is ours, so a service that lost the name never accepts a password. If the name is already
/// taken, [`ServeError::NameTaken`] is returned and the connection is dropped, which removes the
/// objects again. The returned connection serves until it is dropped. The [`Secrets`] handle reads
/// app secrets from the same store, and the [`PromptAnswers`] hands each answer to the prompt that
/// asked.
pub async fn serve(
    store_path: &Path,
    socket: &Path,
    offers: Box<dyn PasswordOffer>,
) -> Result<(Connection, Secrets, PromptAnswers), ServeError> {
    let connection = Connection::session().await.map_err(ServeError::Bus)?;
    serve_on(connection, store_path, socket, offers).await
}

/// [`serve`] on a connection the caller chose. The tests use it to serve on a private bus, which
/// `Connection::session()` cannot do without changing the developer's environment.
pub async fn serve_on(
    connection: Connection,
    store_path: &Path,
    socket: &Path,
    offers: Box<dyn PasswordOffer>,
) -> Result<(Connection, Secrets, PromptAnswers), ServeError> {
    let state = objects::new_state(None);
    let exporter = connection.clone();
    let exported = Arc::clone(&state);
    // Called on the hand-off thread once the store is open, so the collections appear on the bus.
    let on_unlock = Box::new(move || {
        if let Err(error) = block_on(objects::export_tree(exporter.object_server(), &exported)) {
            log::warn!("the collections could not be exported: {error:?}");
        }
    });
    let source = Arc::new(Mutex::new(UnlockKeySource::new(
        store_path.to_path_buf(),
        Arc::clone(&state),
        on_unlock,
        offers,
    )));
    objects::install_source(&state, &source);
    connection
        .object_server()
        .at(
            SERVICE_PATH,
            objects::ServiceObject {
                state: Arc::clone(&state),
                source: Arc::clone(&source),
            },
        )
        .await
        .map_err(ServeError::Bus)?;
    objects::export_default_collection(connection.object_server(), &state)
        .await
        .map_err(ServeError::Bus)?;
    match connection
        .request_name_with_flags(BUS_NAME, RequestNameFlags::DO_NOT_QUEUE)
        .await
    {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {}
        Ok(RequestNameReply::Exists | RequestNameReply::InQueue) | Err(zbus::Error::NameTaken) => {
            return Err(ServeError::NameTaken {
                owner: name_owner(&connection).await,
            });
        }
        Err(error) => return Err(ServeError::Bus(error)),
    }
    let answers = PromptAnswers {
        source: Arc::clone(&source),
        connection: connection.clone(),
    };
    let secrets = Secrets::new(Arc::clone(&state));
    handoff::listen(socket, source).map_err(ServeError::Socket)?;
    log::info!(name = BUS_NAME, path = SERVICE_PATH, "serving the Secret Service");
    Ok((connection, secrets, answers))
}

/// Hands the answer to a password request to the prompt that asked for it. The services role
/// calls [`PromptAnswers::answer`] with each outcome it receives, and the prompt's `Completed`
/// signal goes out from here. The password is read by the store and is never logged here.
pub struct PromptAnswers {
    source: Arc<Mutex<UnlockKeySource>>,
    connection: Connection,
}

impl PromptAnswers {
    /// Answer or end `request`. A request this source did not offer changes nothing.
    pub fn answer(&self, request: PromptId, outcome: PromptOutcome) {
        let end = self
            .source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .answer_password(request, outcome);
        let Some(end) = end else {
            return;
        };
        for path in &end.paths {
            if let Err(error) = block_on(objects::emit_completed(
                self.connection.object_server(),
                path,
                end.dismissed,
            )) {
                log::warn!("the prompt at {path} could not be finished: {error:?}");
            }
        }
    }
}

/// The unique bus name that owns [`BUS_NAME`], for the refusal's log line. It is read after the
/// request failed, so a racing release can leave it unknown; that is reported as such, not guessed.
async fn name_owner(connection: &Connection) -> String {
    let owner: zbus::Result<String> = async {
        let proxy = zbus::fdo::DBusProxy::new(connection).await?;
        let name = BusName::try_from(BUS_NAME).map_err(|error| zbus::Error::Failure(error.to_string()))?;
        Ok(proxy.get_name_owner(name).await?.to_string())
    }
    .await;
    owner.unwrap_or_else(|error| format!("an unknown program ({error})"))
}
