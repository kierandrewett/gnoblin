//! The Settings interface's `reduced-motion` and `contrast` keys, end to end on a private bus.
//!
//! A toolkit asks `Read` and subscribes to `SettingChanged`; the compositor's values arrive
//! as graph observations. These tests serve the portal on its own bus connection, feed it
//! the observations the settings object would send, and read the answers back through a
//! second connection, as a toolkit would. Nothing touches the developer's session bus: the
//! address comes from `support::bus::Bus`, see that module.

mod support;

use std::time::Duration;

use futures_lite::StreamExt as _;
use gnoblin_model::{ObjectId, ObjectView, Observation, Revision, TypeKey, Value};
use gnoblin_types::RequestId;
use support::bus::Bus;
use xdg_desktop_portal_gnoblin::consent::{Host, Offer, Prompt, Refusal};
use xdg_desktop_portal_gnoblin::{Backends, PATH, Portal};
use zbus::connection::Builder;
use zbus::{Connection, OwnedVariant, Proxy};

/// The interface a toolkit talks to.
const INTERFACE: &str = "org.freedesktop.impl.portal.Settings";
/// The namespace the spec names the two preferences in.
const APPEARANCE: &str = "org.freedesktop.appearance";
/// How long a signal may take to arrive before the test calls it lost.
const DEADLINE: Duration = Duration::from_secs(15);

/// A host with no prompt provider. The settings interface never asks for a prompt, so
/// refusing every offer is the honest answer and keeps the test off the consent path.
struct NoProvider;

impl Host for NoProvider {
    fn offer(&mut self, _prompt: &Prompt, _timeout: Duration) -> Result<Offer, Refusal> {
        Err(Refusal::NoProvider { slot: "portal.ui" })
    }

    fn withdraw(&mut self, _request: RequestId) {}
}

/// A snapshot of the settings object with the given properties.
fn snapshot(properties: Vec<(&str, Value)>) -> Observation {
    let properties = properties
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect();
    Observation::Opened {
        view: ObjectView::new(ObjectId::new(1), TypeKey::new("gnoblin", "settings"), properties),
        revision: Revision::ZERO,
    }
}

/// One property of the settings object changed.
fn changed(property: &str, value: Value) -> Observation {
    Observation::Changed {
        object: ObjectId::new(1),
        property: property.to_owned(),
        value,
        revision: Revision::ZERO,
    }
}

/// Serve a portal on `bus` and return it with the unique name a caller addresses it by.
///
/// The portal's own connection is the only one that serves the object, so the caller must
/// address that connection's unique name rather than the well-known name, which these tests
/// do not claim.
async fn serve(bus: &Bus) -> (Portal, String) {
    let connection = Builder::address(bus.address())
        .build()
        .await
        .expect("the portal connects to the private bus");
    let name = connection
        .unique_name()
        .expect("a connected bus gives the portal a unique name")
        .to_string();
    let portal = Portal::new(connection, Box::new(NoProvider), Backends::default(), 1_000);
    portal.export().await.expect("the portal exports its interfaces");
    (portal, name)
}

/// A caller connection on `bus`, as a toolkit would have.
async fn caller(bus: &Bus) -> Connection {
    Builder::address(bus.address())
        .build()
        .await
        .expect("a caller connects to the private bus")
}

/// The settings interface on the portal at `destination`.
async fn settings<'a>(connection: &'a Connection, destination: &'a str) -> Proxy<'a> {
    Proxy::new(connection, destination, PATH, INTERFACE)
        .await
        .expect("the settings proxy")
}

/// Ask `Read` for one value and return it as a `u32`, the type the spec gives both keys.
async fn read_u32(settings: &Proxy<'_>, namespace: &str, key: &str) -> u32 {
    let value: OwnedVariant = settings
        .call("Read", &(namespace, key))
        .await
        .expect("Read answers for a published key");
    u32::try_from(value).expect("the published value is a u32")
}

/// Run `future`, or give up at the deadline, returning `None` when it did not finish.
async fn within<T>(future: impl Future<Output = T>) -> Option<T> {
    futures_lite::future::or(async { Some(future.await) }, async {
        async_io::Timer::after(DEADLINE).await;
        None
    })
    .await
}

#[test]
fn read_returns_the_reduced_motion_and_contrast_values_of_the_snapshot() {
    let bus = Bus::start();
    zbus::block_on(async {
        let (portal, destination) = serve(&bus).await;
        portal
            .observe(&snapshot(vec![
                ("reduced_motion", Value::Uint(1)),
                ("contrast", Value::Uint(1)),
            ]))
            .await;

        let caller = caller(&bus).await;
        let settings = settings(&caller, &destination).await;
        assert_eq!(read_u32(&settings, APPEARANCE, "reduced-motion").await, 1);
        assert_eq!(read_u32(&settings, APPEARANCE, "contrast").await, 1);
    });
}

#[test]
fn a_reduced_motion_change_is_announced_and_read_back() {
    let bus = Bus::start();
    zbus::block_on(async {
        let (portal, destination) = serve(&bus).await;
        portal
            .observe(&snapshot(vec![("reduced_motion", Value::Uint(0))]))
            .await;

        let caller = caller(&bus).await;
        let settings = settings(&caller, &destination).await;
        assert_eq!(read_u32(&settings, APPEARANCE, "reduced-motion").await, 0);

        // Subscribe before the change so the signal cannot arrive unheard.
        let mut changes = settings
            .receive_signal("SettingChanged")
            .await
            .expect("the caller subscribes to SettingChanged");
        portal.observe(&changed("reduced_motion", Value::Uint(1))).await;

        let message = within(changes.next())
            .await
            .expect("a SettingChanged arrives before the deadline")
            .expect("the signal stream yields a message");
        let (namespace, key, value): (String, String, OwnedVariant) =
            message.body().deserialize().expect("SettingChanged carries (ssv)");
        assert_eq!(namespace, APPEARANCE);
        assert_eq!(key, "reduced-motion");
        assert_eq!(u32::try_from(value).expect("a u32"), 1);

        assert_eq!(read_u32(&settings, APPEARANCE, "reduced-motion").await, 1);
    });
}

#[test]
fn a_high_contrast_change_is_read_back_as_contrast() {
    let bus = Bus::start();
    zbus::block_on(async {
        let (portal, destination) = serve(&bus).await;
        portal.observe(&snapshot(vec![("contrast", Value::Uint(0))])).await;

        let caller = caller(&bus).await;
        let settings = settings(&caller, &destination).await;
        portal.observe(&changed("contrast", Value::Uint(1))).await;
        assert_eq!(read_u32(&settings, APPEARANCE, "contrast").await, 1);
    });
}
