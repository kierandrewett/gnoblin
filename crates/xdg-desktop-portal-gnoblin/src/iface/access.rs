//! `org.freedesktop.impl.portal.Access`.
//!
//! One method, `AccessDialog`, asks the user a question on an app's behalf, with the
//! dialog's title, subtitle and body. The question is answered by the `portal.ui` provider,
//! the same as every consent. It grants no capability, so the answer is the D-Bus response
//! and nothing outlives the call.

use std::collections::BTreeMap;

use zbus::message::Header;
use zbus::{Connection, ObjectPath, fdo};

use super::capture::{Options, sandboxed};
use super::{Portal, Reply, await_question, export_request, refused, remove, sender};
use crate::consent::{Interface, NewSession};

use crate::log;

/// `org.freedesktop.impl.portal.Access`.
#[derive(Debug)]
pub struct AccessPortal {
    portal: Portal,
}

impl AccessPortal {
    /// The interface sharing the portal's state.
    #[must_use]
    pub fn new(shared: std::sync::Arc<std::sync::Mutex<super::Shared>>, connection: Connection) -> AccessPortal {
        AccessPortal {
            portal: Portal { shared, connection },
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Access")]
impl AccessPortal {
    /// Ask the user the dialog's question and answer the caller with the user's choice.
    ///
    /// The handle is exported before the prompt, so the frontend can cancel the wait. The
    /// results dictionary is always empty: the dialog returns a response and nothing else.
    // The argument list is the D-Bus method's signature, fixed by the spec.
    #[allow(clippy::too_many_arguments)]
    async fn access_dialog(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        title: String,
        subtitle: String,
        body: String,
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
                    interface: Interface::Access,
                    path: path.clone(),
                })
                .map_err(refused)?;
        }
        let outcome = await_question(
            &self.portal,
            &path,
            &caller,
            &path,
            title,
            message_of(&subtitle, &body),
            &parent_window,
        )
        .await;
        let _ = self.portal.close_session(&caller, &path);
        let _ = remove(&self.portal.connection, &path).await;
        let outcome = outcome?;
        log::info!(app = %app_id, response = outcome.response(), "an access dialog finished");
        Ok((outcome.response(), BTreeMap::new()))
    }
}

/// The dialog's text: the subtitle, then the body, each one only when it is not empty.
fn message_of(subtitle: &str, body: &str) -> String {
    match (subtitle.is_empty(), body.is_empty()) {
        (true, _) => body.to_owned(),
        (false, true) => subtitle.to_owned(),
        (false, false) => format!("{subtitle}\n\n{body}"),
    }
}

#[cfg(test)]
mod tests {
    use super::message_of;

    #[test]
    fn message_keeps_each_part_that_is_present() {
        assert_eq!(message_of("", ""), "");
        assert_eq!(message_of("", "body"), "body");
        assert_eq!(message_of("subtitle", ""), "subtitle");
        assert_eq!(message_of("subtitle", "body"), "subtitle\n\nbody");
    }
}
