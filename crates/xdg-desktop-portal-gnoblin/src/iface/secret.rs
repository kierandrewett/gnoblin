//! `org.freedesktop.impl.portal.Secret`.
//!
//! One method, `RetrieveSecret`, hands the calling app its per-app secret. The bytes never
//! cross the bus: the caller passes a file descriptor, the secret is written to it, and the
//! buffer holding it is wiped once written. The secret is derived inside the keyring
//! service from its master key, so this interface only carries a granted request to it.
//!
//! The same consent as every other portal applies. A secret is the most sensitive thing
//! the portal hands out, so the user is always asked, and a sandboxed caller is asked the
//! same way; there is no narrowed grant to fall back to.

use std::collections::BTreeMap;
use std::io::Write;

use zbus::message::Header;
use zbus::{Connection, ObjectPath, fdo};

use super::capture::{Options, sandboxed};
use super::{Portal, Reply, await_consent, export_request, refused, remove, sender, unsupported};
use crate::backends::Unavailable;
use crate::consent::{ConsentOutcome, Interface, NewSession, PortalCapability};

use crate::log;

/// The version of the `Secret` interface this build serves.
const SECRET_VERSION: u32 = 1;

/// `org.freedesktop.impl.portal.Secret`.
#[derive(Debug)]
pub struct SecretPortal {
    portal: Portal,
}

impl SecretPortal {
    /// The interface sharing the portal's state.
    #[must_use]
    pub fn new(shared: std::sync::Arc<std::sync::Mutex<super::Shared>>, connection: Connection) -> SecretPortal {
        SecretPortal {
            portal: Portal { shared, connection },
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Secret")]
impl SecretPortal {
    /// The interface version, so a frontend can tell what this build serves.
    #[zbus(property)]
    fn version(&self) -> u32 {
        SECRET_VERSION
    }

    /// Hand the caller its secret, written to `fd`, once the user allows it.
    ///
    /// The handle is exported before the prompt, so the frontend can cancel the wait. The
    /// secret is requested from the keyring service only after consent is granted. A
    /// refusal, an expiry or a missing keyring writes nothing to `fd`.
    async fn retrieve_secret(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        fd: zbus::OwnedFd,
        options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = handle.as_str().to_owned();
        export_request(&self.portal, &path).await?;
        {
            let mut shared = self.portal.lock();
            shared
                .guard
                .create(NewSession {
                    bus_name: caller.clone(),
                    app_id: app_id.clone(),
                    sandboxed: sandboxed(&options),
                    interface: Interface::Secret,
                    path: path.clone(),
                })
                .map_err(refused)?;
        }
        let outcome = await_consent(&self.portal, &path, &caller, &path, PortalCapability::Secret, "").await;
        let reply = match outcome {
            Ok(ConsentOutcome::Granted) => self.deliver(&app_id, fd).await,
            Ok(outcome) => Ok((outcome.response(), BTreeMap::new())),
            Err(refusal) => Err(refusal),
        };
        let _ = self.portal.close_session(&caller, &path);
        let _ = remove(&self.portal.connection, &path).await;
        match &reply {
            Ok((response, _)) => {
                log::info!(app = %app_id, response = *response, "a secret request finished");
            }
            Err(_) => log::info!(app = %app_id, "a secret request failed"),
        }
        reply
    }
}

impl SecretPortal {
    /// Ask the keyring for the app's secret and write it to `fd`.
    ///
    /// The read is offered to the keyring service, which answers with the bytes or with
    /// nothing. Nothing is written unless bytes come back, so a refused read leaves the
    /// caller's descriptor untouched. The bytes are a `Zeroizing` buffer, so they are wiped
    /// when this function returns, whether or not the write succeeded.
    async fn deliver(&self, app_id: &str, fd: zbus::OwnedFd) -> fdo::Result<Reply> {
        let offered = {
            let mut shared = self.portal.lock();
            shared.guard.offer_secret(app_id)
        };
        let receiver = offered.map_err(refused)?;
        // A dropped sender means the keyring went away before it answered; no answer is a
        // cancelled read for the caller, the same as the user declining.
        let Ok(answer) = receiver.await else {
            return Ok((2, BTreeMap::new()));
        };
        let Some(bytes) = answer else {
            return Ok((2, BTreeMap::new()));
        };
        let mut file = std::fs::File::from(std::os::fd::OwnedFd::from(fd));
        if let Err(error) = file.write_all(&bytes) {
            log::warn!(%error, "a secret could not be written to the caller");
            return Err(unsupported(Unavailable {
                reason: "the secret could not be written to the caller's descriptor".to_owned(),
            }));
        }
        Ok((0, BTreeMap::new()))
    }
}
