//! Routing for loaded extensions.
//!
//! An extension's namespace is served by one privileged connection, its provider, and
//! granted to other privileged connections. The compositor forwards a granted caller's
//! request to the provider, sends the provider's reply or error back under the caller's
//! own request id, and fans the provider's events out to the grantees. The payload is
//! opaque here: nothing in this module decodes it, so the compositor never needs an
//! extension's schema to route it.

use std::collections::{BTreeMap, BTreeSet};

use gnoblin_client::wire;
use gnoblin_proto::{Envelope, Kind};
use gnoblin_types::ClientId;
use tracing::debug;

use super::requests::FrameDelivery;

/// The wire code a caller gets when the provider of its extension is not connected.
pub(crate) const PROVIDER_GONE: &str = "provider-gone";

/// Where a loaded extension's envelopes go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionRoute {
    /// The namespace the extension provides, such as `example.ext`.
    pub namespace: String,
    /// The privileged connection role that serves the namespace, such as `services`.
    pub provider: String,
}

/// A request forwarded to a provider and not yet answered.
#[derive(Debug)]
struct ForwardedCall {
    caller: ClientId,
    caller_request: u64,
    namespace: String,
    member: String,
    provider: ClientId,
}

/// The routing state of every loaded extension.
#[derive(Debug, Default)]
pub(crate) struct ExtensionRouter {
    /// The provider role of each loaded namespace.
    routes: BTreeMap<String, String>,
    /// The role of each connected privileged connection.
    roles: BTreeMap<ClientId, String>,
    /// The namespaces each connected privileged connection is granted.
    grants: BTreeMap<ClientId, BTreeSet<String>>,
    /// Forwarded requests, keyed by the request id the provider sees.
    pending: BTreeMap<u64, ForwardedCall>,
    /// The next request id handed to a provider.
    next: u64,
}

impl ExtensionRouter {
    /// The routing state for `routes`, with no connection yet.
    pub fn new(routes: &[ExtensionRoute]) -> ExtensionRouter {
        ExtensionRouter {
            routes: routes
                .iter()
                .map(|route| (route.namespace.clone(), route.provider.clone()))
                .collect(),
            next: 1,
            ..ExtensionRouter::default()
        }
    }

    /// Records a connected privileged connection, its role and the namespaces it is granted.
    pub fn connect(&mut self, client: ClientId, role: &str, namespaces: impl IntoIterator<Item = String>) {
        self.roles.insert(client, role.to_owned());
        self.grants.insert(client, namespaces.into_iter().collect());
    }

    /// Whether `namespace` is a loaded extension.
    pub fn is_loaded(&self, namespace: &str) -> bool {
        self.routes.contains_key(namespace)
    }

    /// The connection that currently serves `namespace`, if its provider is connected.
    fn provider_of(&self, namespace: &str) -> Option<ClientId> {
        let role = self.routes.get(namespace)?;
        self.roles
            .iter()
            .find_map(|(client, connected)| (connected == role).then_some(*client))
    }

    /// Routes one envelope in a namespace other than `gnoblin`, from `client`.
    ///
    /// Returns the frames to write, each to the connection it is addressed to. A request
    /// the router cannot serve is answered with an error to its caller; anything else it
    /// cannot place is dropped.
    pub fn route(&mut self, client: ClientId, envelope: &Envelope) -> Vec<FrameDelivery> {
        let namespace = envelope.namespace.clone();
        if !self.is_loaded(&namespace) {
            return refuse(
                client,
                envelope,
                wire::code::UNKNOWN_MEMBER,
                &format!("no extension `{namespace}` is loaded"),
            );
        }
        if self.provider_of(&namespace) == Some(client) {
            return self.route_from_provider(client, envelope);
        }
        if !self.grants.get(&client).is_some_and(|set| set.contains(&namespace)) {
            return refuse(
                client,
                envelope,
                wire::code::DENIED,
                &format!("`{namespace}` is not granted to this connection"),
            );
        }
        let Some(Kind::Request(_)) = &envelope.kind else {
            return refuse(
                client,
                envelope,
                wire::code::UNKNOWN_MEMBER,
                "a granted connection may send requests to an extension, not replies or events",
            );
        };
        let Some(provider) = self.provider_of(&namespace) else {
            return refuse(
                client,
                envelope,
                PROVIDER_GONE,
                &format!("the provider of `{namespace}` is not connected"),
            );
        };
        let id = self.next;
        self.next += 1;
        self.pending.insert(
            id,
            ForwardedCall {
                caller: client,
                caller_request: envelope.request_id,
                namespace: namespace.clone(),
                member: envelope.member.clone(),
                provider,
            },
        );
        let payload = envelope.payload().unwrap_or_default().to_vec();
        vec![FrameDelivery {
            client: provider,
            envelope: Envelope::request(namespace, envelope.member.clone(), id, payload),
        }]
    }

    /// Routes what a provider sends: a reply or error to its caller, an event to the grantees.
    fn route_from_provider(&mut self, provider: ClientId, envelope: &Envelope) -> Vec<FrameDelivery> {
        match &envelope.kind {
            Some(Kind::Reply(_)) | Some(Kind::Error(_)) => {
                let Some(pending) = self.pending.remove(&envelope.request_id) else {
                    debug!(
                        id = envelope.request_id,
                        "[ipc] a provider answered a request nobody is waiting for"
                    );
                    return Vec::new();
                };
                if pending.provider != provider || pending.namespace != envelope.namespace {
                    debug!(
                        id = envelope.request_id,
                        "[ipc] a provider answered a request addressed to another"
                    );
                    return Vec::new();
                }
                let mut reply = envelope.clone();
                reply.request_id = pending.caller_request;
                vec![FrameDelivery {
                    client: pending.caller,
                    envelope: reply,
                }]
            }
            Some(Kind::Event(_)) => self
                .grants
                .iter()
                .filter(|(client, set)| **client != provider && set.contains(&envelope.namespace))
                .map(|(client, _)| FrameDelivery {
                    client: *client,
                    envelope: envelope.clone(),
                })
                .collect(),
            _ => refuse(
                provider,
                envelope,
                wire::code::UNKNOWN_MEMBER,
                "a provider answers requests and sends replies and events, not requests",
            ),
        }
    }

    /// Fails every request forwarded to `provider` that it has not answered.
    ///
    /// Used when the provider's connection ends and when its child restarts: either way
    /// the session that was going to answer is gone.
    pub fn provider_gone(&mut self, provider: ClientId) -> Vec<FrameDelivery> {
        let lost: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.provider == provider)
            .map(|(id, _)| *id)
            .collect();
        lost.into_iter()
            .filter_map(|id| self.pending.remove(&id))
            .map(|pending| FrameDelivery {
                client: pending.caller,
                envelope: Envelope::error(
                    pending.namespace.clone(),
                    pending.member,
                    pending.caller_request,
                    PROVIDER_GONE,
                    format!("the provider of `{}` is gone", pending.namespace),
                ),
            })
            .collect()
    }

    /// Forgets a connection that ended: its role, its grants and what it was waiting for.
    ///
    /// Returns the frames owed to callers whose provider this was.
    pub fn departed(&mut self, client: ClientId) -> Vec<FrameDelivery> {
        let deliveries = self.provider_gone(client);
        self.pending.retain(|_, pending| pending.caller != client);
        self.roles.remove(&client);
        self.grants.remove(&client);
        deliveries
    }
}

/// An error answer to a request, addressed to the connection that sent it.
///
/// Anything that is not a request has no one waiting for it, so it is dropped.
fn refuse(client: ClientId, envelope: &Envelope, code: &str, message: &str) -> Vec<FrameDelivery> {
    if !matches!(&envelope.kind, Some(Kind::Request(_))) {
        debug!(code, message, "[ipc] an extension frame was refused");
        return Vec::new();
    }
    vec![FrameDelivery {
        client,
        envelope: Envelope::error(
            envelope.namespace.clone(),
            envelope.member.clone(),
            envelope.request_id,
            code,
            message,
        ),
    }]
}
