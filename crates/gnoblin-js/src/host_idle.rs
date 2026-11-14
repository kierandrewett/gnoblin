//! `gnoblin.idle` on the Rust side: the shell's connection to the compositor's `idle` object.
//!
//! The compositor publishes how long the session has been idle (`idle_ms`) and how many holds keep it awake
//! (`inhibitors`) as one object of the `gnoblin` namespace, and `inhibit` and `uninhibit` are its methods. The tree
//! socket carries the `tree` namespace only, so the shell reads the object over an owner connection of its own,
//! [`JsIdleLink`]. The connection is opened when the running generation has a watch or a hold and closed when it has
//! neither, so a shell that never watches idleness costs the compositor nothing: every change of `idle_ms` is sent to
//! each subscriber.
//!
//! The script's side is `data/js/gnoblin/idle.ts`. A watch there is a threshold over the reading this module hands it;
//! the compositor sends no per-watch notification. A hold is a name recorded with `inhibit`; this module gives every
//! hold back when the generation is retired (a reload or a shutdown). The compositor records a hold by name and not by
//! connection, so a shell that is killed without retiring leaves its holds standing until the same name is released:
//! unlike a chord claim or a slot, nothing releases them when the connection closes.

use std::path::Path;
use std::time::Duration;

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ClientError, Event, Transport};
use gnoblin_model::{Fields, Filter, ObjectId, ObjectView, Observation, TypeKey, Value};
use gnoblin_proto::Version;
use rquickjs::{Ctx, Object};

use crate::config::JsLinkError;

gnoblin_log::logger!(log = "js");

/// How long a poll waits for bytes that have not arrived. Short, so a frame never stalls on it.
const READ_WAIT: Duration = Duration::from_millis(1);

/// How long a call waits for the compositor's reply. The compositor answers an `idle` call at once.
const REPLY_WAIT: Duration = Duration::from_secs(5);

/// The object type that publishes the idle state.
const IDLE: &str = "idle";

/// What the compositor said about idleness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JsIdleReading {
    /// Milliseconds since the last input event.
    pub idle_ms: u64,
    /// How many holds keep the session awake.
    pub inhibitors: u64,
}

/// The shell's connection for idle state.
pub trait JsIdleLink {
    /// The newest reading if the compositor sent one since the last call, else `None`. The first call after the
    /// connection opened returns the snapshot's reading. An error is the loss of the connection.
    ///
    /// # Errors
    ///
    /// The connection ended, or the `idle` object went away.
    fn poll(&mut self) -> Result<Option<JsIdleReading>, JsLinkError>;
    /// Holds the session awake under `holder`.
    ///
    /// # Errors
    ///
    /// The compositor refused: `too-many-inhibitors` past its cap, or `idle-unavailable`.
    fn inhibit(&mut self, holder: &str) -> Result<(), JsLinkError>;
    /// Gives the hold `holder` names back. A name that holds nothing is not an error.
    ///
    /// # Errors
    ///
    /// The connection ended.
    fn uninhibit(&mut self, holder: &str) -> Result<(), JsLinkError>;
}

/// Opens the shell's idle connection. The shell calls it when the script first wants one.
pub type JsIdleConnector = Box<dyn FnMut() -> Result<Box<dyn JsIdleLink>, JsLinkError>>;

/// [`JsIdleLink`] over a handshaken owner connection that is subscribed to the `idle` object alone.
pub struct JsClientIdleLink<T: Transport> {
    client: Client<T>,
    object: ObjectId,
    latest: JsIdleReading,
    unread: bool,
}

fn read(view: &ObjectView) -> Result<JsIdleReading, JsLinkError> {
    let uint = |name: &str| match view.property(name) {
        Some(Value::Uint(value)) => Ok(*value),
        Some(other) => Err(JsLinkError(format!(
            "the idle object's `{name}` is not a uint but {}",
            other.kind()
        ))),
        None => Err(JsLinkError(format!("the idle object has no `{name}`"))),
    };
    Ok(JsIdleReading {
        idle_ms: uint("idle_ms")?,
        inhibitors: uint("inhibitors")?,
    })
}

impl JsClientIdleLink<UnixTransport> {
    /// Connects to the owner socket at `path`, finds the `idle` object and subscribes to it alone.
    ///
    /// # Errors
    ///
    /// The socket is not there, the handshake failed, or the graph publishes no `idle` object: a zero would read as
    /// "the user is active", so the connection is refused instead.
    pub fn connect(path: &Path, version: Version) -> Result<Self, JsLinkError> {
        let open = || {
            let transport = UnixTransport::connect(path)
                .map_err(|error| JsLinkError(format!("cannot connect to {}: {error}", path.display())))?;
            Client::connect(transport, version).map_err(|error| JsLinkError(error.to_string()))
        };
        // The id is found on a connection that is dropped, so the one that is kept is sent the idle object's changes
        // and not those of every window and output of the graph.
        let object = {
            let mut lookup = open()?;
            let snapshot = lookup
                .subscribe(Filter::All)
                .map_err(|error| JsLinkError(error.to_string()))?;
            snapshot
                .objects()
                .iter()
                .find(|view| view.ty() == &TypeKey::gnoblin(IDLE))
                .map(ObjectView::id)
                .ok_or_else(|| JsLinkError("the compositor publishes no idle object".to_owned()))?
        };
        Self::new(open()?, object)
    }
}

impl<T: Transport> JsClientIdleLink<T> {
    /// Subscribes `client`, whose handshake is done, to the object `object`.
    ///
    /// # Errors
    ///
    /// The subscription failed or the object has no readable `idle_ms` and `inhibitors`.
    pub fn new(mut client: Client<T>, object: ObjectId) -> Result<Self, JsLinkError> {
        let snapshot = client
            .subscribe(Filter::Object(object))
            .map_err(|error| JsLinkError(error.to_string()))?;
        let view = snapshot
            .objects()
            .iter()
            .find(|view| view.id() == object)
            .ok_or_else(|| JsLinkError("the idle object is gone".to_owned()))?;
        let latest = read(view)?;
        let mut link = Self {
            client,
            object,
            latest,
            unread: true,
        };
        link.wait(READ_WAIT)?;
        Ok(link)
    }

    fn wait(&mut self, timeout: Duration) -> Result<(), JsLinkError> {
        self.client
            .set_read_timeout(Some(timeout))
            .map_err(|error| JsLinkError(format!("the idle connection refuses a read deadline: {error}")))
    }

    /// Calls `method` on the idle object with the long reply deadline, then puts the short poll deadline back.
    fn call(&mut self, method: &str, holder: &str) -> Result<(), JsLinkError> {
        self.wait(REPLY_WAIT)?;
        let arguments = Fields::from([("holder".to_owned(), Value::Text(holder.to_owned()))]);
        let outcome = self
            .client
            .call(self.object, method, &arguments)
            .and_then(|operation| self.client.await_operation(operation));
        self.wait(READ_WAIT)?;
        match outcome {
            Ok(outcome) if outcome.is_done() => Ok(()),
            Ok(other) => Err(JsLinkError(format!("idle.{method} was refused: {other:?}"))),
            Err(error) => Err(JsLinkError(error.to_string())),
        }
    }
}

impl<T: Transport> JsIdleLink for JsClientIdleLink<T> {
    fn poll(&mut self) -> Result<Option<JsIdleReading>, JsLinkError> {
        loop {
            match self.client.next_event() {
                Ok(Event::Model(Observation::Changed {
                    object,
                    property,
                    value: Value::Uint(value),
                    ..
                })) if object == self.object => {
                    let before = self.latest;
                    match property.as_str() {
                        "idle_ms" => self.latest.idle_ms = value,
                        "inhibitors" => self.latest.inhibitors = value,
                        _ => {}
                    }
                    self.unread |= self.latest != before;
                }
                Ok(Event::Model(Observation::Closed { object, .. })) if object == self.object => {
                    return Err(JsLinkError("the idle object went away".to_owned()));
                }
                Ok(_) => {}
                Err(ClientError::Idle) => break,
                Err(error) => return Err(JsLinkError(error.to_string())),
            }
        }
        Ok(std::mem::take(&mut self.unread).then_some(self.latest))
    }

    fn inhibit(&mut self, holder: &str) -> Result<(), JsLinkError> {
        self.call("inhibit", holder)
    }

    fn uninhibit(&mut self, holder: &str) -> Result<(), JsLinkError> {
        self.call("uninhibit", holder)
    }
}

/// One hold command of the script, as `takeIdleCommands` returned it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct JsIdleCommand {
    pub inhibit: bool,
    pub holder: String,
}

/// What one poll of the shell's idle connection found.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct JsIdlePoll {
    pub reading: Option<JsIdleReading>,
    /// The connection ended in this poll: the script forgets its last reading.
    pub lost: bool,
}

/// The shell's idle connection and when it is open.
pub(crate) struct JsIdle {
    connector: Option<JsIdleConnector>,
    link: Option<Box<dyn JsIdleLink>>,
    /// A connection failed or was lost, and that has been reported: it is not tried again (and not reported again)
    /// until the generation has no watch and no hold, or a new one takes over.
    gave_up: bool,
}

impl JsIdle {
    pub(crate) fn new() -> Self {
        Self {
            connector: None,
            link: None,
            gave_up: false,
        }
    }

    pub(crate) fn set_connector(&mut self, connector: JsIdleConnector) {
        self.connector = Some(connector);
    }

    /// Whether the connection is open: the frame must keep running its side of the exchange.
    pub(crate) fn is_open(&self) -> bool {
        self.link.is_some()
    }

    /// Opens the connection while the generation wants one and closes it when it does not.
    pub(crate) fn sync(&mut self, wanted: bool, diagnostics: &mut Vec<String>) {
        if !wanted {
            self.gave_up = false;
            self.link = None;
            return;
        }
        if self.link.is_some() || self.gave_up {
            return;
        }
        let Some(connector) = self.connector.as_mut() else {
            self.gave_up = true;
            diagnostics.push(
                "the shell has no idle connection, so idle watches never fire and idle holds are not recorded"
                    .to_owned(),
            );
            return;
        };
        match connector() {
            Ok(link) => self.link = Some(link),
            Err(error) => {
                self.gave_up = true;
                diagnostics.push(format!(
                    "the idle connection was not opened, so idle watches never fire and idle holds are not recorded: {error}"
                ));
            }
        }
    }

    /// What the compositor sent since the last frame. A lost connection is dropped and reported once.
    pub(crate) fn poll(&mut self, diagnostics: &mut Vec<String>) -> JsIdlePoll {
        let Some(link) = self.link.as_mut() else {
            return JsIdlePoll::default();
        };
        match link.poll() {
            Ok(reading) => JsIdlePoll { reading, lost: false },
            Err(error) => {
                self.link = None;
                self.gave_up = true;
                diagnostics.push(format!(
                    "the idle connection is lost ({error}); idle watches stop firing and the compositor has given back its holds"
                ));
                JsIdlePoll {
                    reading: None,
                    lost: true,
                }
            }
        }
    }

    /// Sends the script's hold commands. The holders the compositor refused (or that had no connection to go over)
    /// are returned, so the script's hold says it is not one.
    pub(crate) fn run(&mut self, commands: &[JsIdleCommand], diagnostics: &mut Vec<String>) -> Vec<String> {
        let mut failed = Vec::new();
        for command in commands {
            let Some(link) = self.link.as_mut() else {
                if command.inhibit {
                    failed.push(command.holder.clone());
                }
                continue;
            };
            let result = if command.inhibit {
                link.inhibit(&command.holder)
            } else {
                link.uninhibit(&command.holder)
            };
            if let Err(error) = result {
                let verb = if command.inhibit { "inhibit" } else { "uninhibit" };
                diagnostics.push(format!("idle.{verb}({}) failed: {error}", command.holder));
                if command.inhibit {
                    failed.push(command.holder.clone());
                }
            }
        }
        failed
    }

    /// Gives every hold the generation still had back and closes the connection.
    pub(crate) fn retire(&mut self, holders: &[String]) {
        if let Some(link) = self.link.as_mut() {
            for holder in holders {
                if let Err(error) = link.uninhibit(holder) {
                    log::debug!(area = "idle", "the hold {holder} was not given back: {error}");
                }
            }
        }
        self.link = None;
        self.gave_up = false;
    }
}

/// A reading as the script object `idle.deliver` takes.
pub(crate) fn reading_to_js<'js>(ctx: &Ctx<'js>, reading: JsIdleReading) -> rquickjs::Result<Object<'js>> {
    let item = Object::new(ctx.clone())?;
    item.set("idleMs", reading.idle_ms as f64)?;
    item.set("inhibitors", reading.inhibitors as f64)?;
    Ok(item)
}

/// The commands a script's frame produced, as the module's `takeIdleCommands` returned them.
pub(crate) fn commands_from_js(list: &rquickjs::Array<'_>) -> rquickjs::Result<Vec<JsIdleCommand>> {
    let mut commands = Vec::new();
    for item in list.iter::<Object<'_>>() {
        let item = item?;
        let kind: String = item.get("kind")?;
        commands.push(JsIdleCommand {
            inhibit: kind == "inhibit",
            holder: item.get("holder")?,
        });
    }
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scripted {
        readings: Vec<Result<Option<JsIdleReading>, JsLinkError>>,
        refuse_inhibit: bool,
        calls: Vec<String>,
    }

    impl JsIdleLink for Scripted {
        fn poll(&mut self) -> Result<Option<JsIdleReading>, JsLinkError> {
            if self.readings.is_empty() {
                Ok(None)
            } else {
                self.readings.remove(0)
            }
        }

        fn inhibit(&mut self, holder: &str) -> Result<(), JsLinkError> {
            self.calls.push(format!("inhibit {holder}"));
            if self.refuse_inhibit {
                Err(JsLinkError("too-many-inhibitors".to_owned()))
            } else {
                Ok(())
            }
        }

        fn uninhibit(&mut self, holder: &str) -> Result<(), JsLinkError> {
            self.calls.push(format!("uninhibit {holder}"));
            Ok(())
        }
    }

    fn idle_with(link: Scripted) -> JsIdle {
        let mut idle = JsIdle::new();
        let mut once = Some(Box::new(link) as Box<dyn JsIdleLink>);
        idle.set_connector(Box::new(move || {
            once.take().ok_or_else(|| JsLinkError("opened twice".to_owned()))
        }));
        idle
    }

    #[test]
    fn a_connection_that_cannot_open_is_reported_once_and_not_retried() {
        let mut idle = JsIdle::new();
        idle.set_connector(Box::new(|| Err(JsLinkError("no socket".to_owned()))));
        let mut diagnostics = Vec::new();
        idle.sync(true, &mut diagnostics);
        idle.sync(true, &mut diagnostics);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].contains("no socket"));
        idle.sync(false, &mut diagnostics);
        idle.sync(true, &mut diagnostics);
        assert_eq!(diagnostics.len(), 2, "a generation that wants one again tries again");
    }

    #[test]
    fn a_lost_connection_is_reported_once_and_reported_to_the_script() {
        let mut idle = idle_with(Scripted {
            readings: vec![Err(JsLinkError("closed".to_owned()))],
            refuse_inhibit: false,
            calls: Vec::new(),
        });
        let mut diagnostics = Vec::new();
        idle.sync(true, &mut diagnostics);
        let poll = idle.poll(&mut diagnostics);
        assert!(poll.lost && poll.reading.is_none());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(idle.poll(&mut diagnostics), JsIdlePoll::default());
        idle.sync(true, &mut diagnostics);
        assert_eq!(
            diagnostics.len(),
            1,
            "a lost connection is not opened again behind the script's back"
        );
    }

    #[test]
    fn a_refused_hold_is_returned_to_the_script_and_a_missing_connection_refuses_too() {
        let mut idle = idle_with(Scripted {
            readings: Vec::new(),
            refuse_inhibit: true,
            calls: Vec::new(),
        });
        let mut diagnostics = Vec::new();
        let commands = [
            JsIdleCommand {
                inhibit: true,
                holder: "player".to_owned(),
            },
            JsIdleCommand {
                inhibit: false,
                holder: "gone".to_owned(),
            },
        ];
        assert_eq!(
            idle.run(&commands, &mut diagnostics),
            vec!["player".to_owned()],
            "no connection yet: the hold cannot be recorded, and giving one back needs nothing sent"
        );
        assert!(diagnostics.is_empty());
        idle.sync(true, &mut diagnostics);
        let failed = idle.run(&commands, &mut diagnostics);
        assert_eq!(failed, vec!["player".to_owned()]);
        assert!(diagnostics.iter().any(|line| line.contains("too-many-inhibitors")));
    }
}
