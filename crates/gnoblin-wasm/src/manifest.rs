//! What a module may do, how long it may take and how much it may hold.
//!
//! A module's manifest is the whole of its authority (design/rust-compositor.md,
//! D6: "A module gets only the capabilities its manifest declares and the user
//! grants"). It is data, checked when the module is loaded, and the host enforces
//! it at every entry point: a read the manifest does not name is refused before the
//! mirror is touched, a method it does not name is refused before a frame is sent,
//! and `watch` is refused if the manifest does not grant subscribing.
//!
//! Three things are deliberately absent:
//!
//! - **No wildcard.** An empty capability set grants nothing and there is no
//!   "read everything": a wildcard makes a manifest unreadable at review time,
//!   which is exactly the property the capability model exists for.
//! - **No per-identity capability.** A capability names an object *type*, because
//!   the ABI's events and lookups name a type too; restricting to particular
//!   objects would need a rule about how an object's identity becomes known and
//!   that rule is the policy layer's, not the sandbox's.
//! - **No clock.** A deadline is [`Budget`], a quantity of fuel, never a reading of
//!   `Instant`: a busy machine and an idle one make the same decision, and a test
//!   replays exactly. Epoch interruption would need a ticker thread holding a clock,
//!   which is one more thing a module's failure could stop.

use std::collections::BTreeSet;

use crate::abi;

/// How much work one entry into a module may do.
///
/// The unit is WebAssembly fuel, the runtime's own accounting of executed
/// instructions, which is why a host call the module makes is *not* bounded by it:
/// fuel is spent by compiled code, and a call into the host blocks until the host
/// returns. D6 says the same thing from the other side ("Fuel and epochs do not
/// bound time spent inside host calls"), and the answer is the host's own
/// deadlines, not this type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    fuel: u64,
}

impl Budget {
    /// A budget of `fuel` WebAssembly fuel units.
    pub const fn fuel(fuel: u64) -> Budget {
        Budget { fuel }
    }

    /// The fuel this budget grants.
    pub fn as_fuel(self) -> u64 {
        self.fuel
    }
}

impl Default for Budget {
    /// Enough for a module to walk a large graph once.
    ///
    /// A concrete number and not "unbounded": an exhausted budget is how the host
    /// finds out a module did not finish, so there has to be one.
    fn default() -> Budget {
        Budget { fuel: 10_000_000 }
    }
}

/// What one module may hold while it runs.
///
/// The defaults are small on purpose. A module that computes a placement strategy
/// works in kilobytes; a module that wants a hundred megabytes is either doing
/// something this host does not offer or is the failure this limit exists for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleLimits {
    /// The module's linear memory may not grow past this many bytes.
    pub memory_bytes: usize,
    /// The module's tables may not hold more than this many elements.
    pub table_elements: u32,
    /// How many memories, tables and instances the module may create.
    pub memories: usize,
    /// How many tables the module may have.
    pub tables: usize,
}

impl Default for ModuleLimits {
    fn default() -> ModuleLimits {
        ModuleLimits {
            memory_bytes: 16 * 1024 * 1024,
            table_elements: 10_000,
            memories: 1,
            tables: 4,
        }
    }
}

/// What a manifest declares a module may do.
///
/// Ordered sets and not vectors, so a denied capability is found by lookup rather
/// than by scanning, and so two manifests that grant the same things compare equal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    read: BTreeSet<String>,
    call: BTreeSet<(String, String)>,
    subscribe: bool,
}

impl Capabilities {
    /// Nothing at all: a module with this set can start and stop and no more.
    pub fn none() -> Capabilities {
        Capabilities::default()
    }

    /// Read `object`'s properties and open a handle for it.
    pub fn read(mut self, object: &str) -> Capabilities {
        self.read.insert(object.to_owned());
        self
    }

    /// Call `method` on objects of type `object`.
    ///
    /// Calling a method does not imply reading the object: a module that only
    /// focuses windows learns nothing about them.
    pub fn call(mut self, object: &str, method: &str) -> Capabilities {
        self.call.insert((object.to_owned(), method.to_owned()));
        self
    }

    /// Receive events for objects the module watches.
    pub fn subscribe(mut self) -> Capabilities {
        self.subscribe = true;
        self
    }

    /// Whether `object` may be read.
    pub fn may_read(&self, object: &str) -> bool {
        self.read.contains(object)
    }

    /// Whether `method` may be called on `object`.
    pub fn may_call(&self, object: &str, method: &str) -> bool {
        self.call.contains(&(object.to_owned(), method.to_owned()))
    }

    /// Whether the module may subscribe to anything.
    pub fn may_subscribe(&self) -> bool {
        self.subscribe
    }

    /// The object types the manifest names for reading, for a report.
    pub fn reads(&self) -> impl Iterator<Item = &str> {
        self.read.iter().map(String::as_str)
    }

    /// The method pairs the manifest names, for a report.
    pub fn calls(&self) -> impl Iterator<Item = (&str, &str)> {
        self.call
            .iter()
            .map(|(object, method)| (object.as_str(), method.as_str()))
    }

    /// Check every name against the generated package.
    ///
    /// A capability the ABI does not have is a mistake in the manifest — a
    /// misspelt object or a method that moved — and it must be reported rather than
    /// silently granting nothing, because a module that is refused for a capability
    /// its manifest seems to grant is very hard to diagnose from the module's side.
    pub fn check(&self) -> Result<(), String> {
        for object in &self.read {
            if !abi::OBJECT_TYPES.contains(&object.as_str()) {
                return Err(format!(
                    "capability `read {object}` names no object type; the package declares {}",
                    list(abi::OBJECT_TYPES.iter().copied())
                ));
            }
        }
        for (object, method) in &self.call {
            if !abi::CALLABLE
                .iter()
                .any(|(known_object, known_method)| known_object == object && known_method == method)
            {
                return Err(format!(
                    "capability `call {object}.{method}` names no method the package declares"
                ));
            }
        }
        Ok(())
    }
}

/// A comma-separated list for an error message.
fn list<'a>(items: impl Iterator<Item = &'a str>) -> String {
    items.collect::<Vec<&str>>().join(", ")
}

/// One module's declaration: what it may do and how much it may hold.
#[derive(Clone, Debug)]
pub struct Manifest {
    name: String,
    capabilities: Capabilities,
    limits: ModuleLimits,
}

impl Manifest {
    /// A manifest for the module called `name`, granting nothing.
    pub fn new(name: impl Into<String>) -> Manifest {
        Manifest {
            name: name.into(),
            capabilities: Capabilities::none(),
            limits: ModuleLimits::default(),
        }
    }

    /// Declare the module's capabilities.
    pub fn capabilities(mut self, capabilities: Capabilities) -> Manifest {
        self.capabilities = capabilities;
        self
    }

    /// Declare the module's resource limits.
    pub fn limits(mut self, limits: ModuleLimits) -> Manifest {
        self.limits = limits;
        self
    }

    /// The module's name, which is how the host names it and how its log lines are
    /// prefixed.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What it may do.
    pub fn granted(&self) -> &Capabilities {
        &self.capabilities
    }

    /// What it may hold.
    pub fn bounds(&self) -> ModuleLimits {
        self.limits
    }
}

#[cfg(test)]
mod tests {
    use super::{Budget, Capabilities, Manifest};

    #[test]
    fn checks_names_against_the_package() {
        let good = Capabilities::none().read("window").call("window", "focus");
        assert_eq!(good.check(), Ok(()));

        let bad_object = Capabilities::none().read("widow");
        assert!(bad_object.check().is_err(), "a misspelt object is refused");

        let bad_method = Capabilities::none().call("window", "focused");
        assert!(bad_method.check().is_err(), "a property is not a method");
    }

    #[test]
    fn grants_nothing_by_default() {
        let capabilities = Capabilities::none();
        assert!(!capabilities.may_read("window"));
        assert!(!capabilities.may_call("window", "focus"));
        assert!(!capabilities.may_subscribe());
    }

    #[test]
    fn keeps_the_manifest_name() {
        let manifest = Manifest::new("placement");
        assert_eq!(manifest.name(), "placement");
        assert!(manifest.granted().reads().next().is_none());
    }

    #[test]
    fn budgets_are_fuel() {
        assert_eq!(Budget::fuel(7).as_fuel(), 7);
        assert!(Budget::default().as_fuel() > 0);
    }
}
