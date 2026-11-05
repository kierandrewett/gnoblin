//! Model-based test: random mutations, checked against the snapshot-then-diffs
//! rule.
//!
//! The generator is a small hand-written xorshift so the suite needs no dependency
//! and every failure is reproducible from its seed. Each seed is replayed twice and
//! the two observation streams must match, which is the determinism the parity
//! harness and the QEMU runs will rely on.
//!
//! What is checked after every step:
//!
//! - a subscription receives nothing at or before its snapshot's revision;
//! - its revisions are strictly increasing, so no diff is reordered or repeated;
//! - every observation it receives matches its filter;
//! - applying its whole stream to its snapshot yields exactly the state the graph
//!   holds now, for every filter, including one whose object has closed;
//! - a refused mutation publishes nothing.
//!
//! The generator drives the real schema (`crates/gnoblin-schema/schema`), so an
//! object type, property, signal or method added to the contract is exercised here
//! by the next run rather than needing a new case.

use std::collections::BTreeMap;
use std::path::Path;

use gnoblin_model::{
    Fields, Filter, Graph, ModelError, ObjectId, Observation, OperationOutcome, Revision, Subscription, TypeKey, Value,
};
use gnoblin_schema::{Catalog, Type};

/// Seeds replayed.
const SEQUENCES: u64 = 128;

/// Mutations per seed.
const STEPS_PER_SEQUENCE: u32 = 200;

/// Live objects a sequence keeps before closing one instead of creating.
const OBJECT_POOL: usize = 6;

/// Subscriptions a sequence keeps at once.
const SUBSCRIPTION_POOL: usize = 4;

/// A failing run must report at least this many invariant checks, so a suite that
/// silently stops exercising the graph is visible.
const MIN_CHECKS: usize = 200_000;

/// A deterministic xorshift64 generator.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    /// A number below `bound`, which must be greater than zero.
    fn below(&mut self, bound: u32) -> u32 {
        (self.next() % u64::from(bound)) as u32
    }

    fn chance(&mut self, one_in: u32) -> bool {
        self.below(one_in) == 0
    }
}

/// The schema's members for one object type, in declaration order.
struct Members {
    key: TypeKey,
    properties: Vec<(String, Type)>,
    signals: Vec<(String, Vec<(String, Type)>)>,
    methods: Vec<(String, Vec<(String, Type)>)>,
}

/// Every object type in the schema.
fn members(catalog: &Catalog) -> Vec<Members> {
    let mut members = Vec::new();
    for namespace in catalog.namespaces() {
        for object in catalog.objects(&namespace) {
            members.push(Members {
                key: TypeKey::new(namespace.as_str(), object.name.clone()),
                properties: object
                    .properties
                    .iter()
                    .map(|property| (property.name.clone(), property.ty.clone()))
                    .collect(),
                signals: object
                    .signals
                    .iter()
                    .map(|signal| {
                        (
                            signal.name.clone(),
                            signal
                                .fields
                                .iter()
                                .map(|field| (field.name.clone(), field.ty.clone()))
                                .collect(),
                        )
                    })
                    .collect(),
                methods: object
                    .methods
                    .iter()
                    .map(|method| {
                        (
                            method.name.clone(),
                            method
                                .params
                                .iter()
                                .map(|parameter| (parameter.name.clone(), parameter.ty.clone()))
                                .collect(),
                        )
                    })
                    .collect(),
            });
        }
    }
    members
}

/// A value of the declared type, or a value of a different type when `wrong`.
fn value(rng: &mut Rng, ty: &Type, wrong: bool) -> Value {
    if wrong {
        // A type nothing in the current schema declares for this member, chosen so
        // that the value still exercises the recursion for lists.
        return match ty {
            Type::Text => Value::Uint(rng.next()),
            _ => Value::Text("wrong".to_owned()),
        };
    }
    match ty {
        Type::Bool => Value::Bool(rng.chance(2)),
        Type::Int => Value::Int(rng.next() as i64),
        Type::Uint => Value::Uint(rng.next()),
        Type::Double => Value::Double(f64::from(rng.below(4_000)) / 8.0),
        Type::Text => Value::Text(format!("value {}", rng.below(7))),
        Type::Bytes => Value::Bytes(vec![rng.below(256) as u8]),
        Type::ObjectRef(_) => Value::Object(ObjectId::new(u64::from(rng.below(4) + 1))),
        Type::List(inner) => {
            let count = rng.below(3);
            Value::List((0..count).map(|_| value(rng, inner, false)).collect())
        }
    }
}

fn fields(rng: &mut Rng, declared: &[(String, Type)]) -> Fields {
    declared
        .iter()
        .map(|(name, ty)| (name.clone(), value(rng, ty, false)))
        .collect()
}

/// One subscription and the state its stream has rebuilt so far.
struct Shadow {
    subscription: Subscription,
    filter: Filter,
    /// Every property value the shadow believes in, per live object.
    views: BTreeMap<ObjectId, (TypeKey, Fields)>,
    last_revision: Revision,
}

/// The graph's state, in the same shape a shadow keeps.
fn state(graph: &Graph) -> BTreeMap<ObjectId, (TypeKey, Fields)> {
    graph
        .objects()
        .map(|object| (object.id(), (object.ty().clone(), object.properties().clone())))
        .collect()
}

/// Drain one subscription, checking order and applying each observation.
fn drain(graph: &mut Graph, shadow: &mut Shadow, seen: &mut Vec<Observation>) -> Result<usize, String> {
    let mut checks = 0;
    while let Some(observation) = graph.next(&shadow.subscription) {
        let revision = observation.revision();
        if revision <= shadow.subscription.snapshot().revision() {
            return Err(format!(
                "{revision} is at or before the snapshot at {}",
                shadow.subscription.snapshot().revision()
            ));
        }
        if revision <= shadow.last_revision {
            return Err(format!(
                "{revision} does not follow {} in the same subscription",
                shadow.last_revision
            ));
        }
        if revision > graph.revision() {
            return Err(format!("{revision} is beyond the graph at {}", graph.revision()));
        }
        match shadow.filter {
            Filter::All => {}
            Filter::Object(id) if observation.object() != id => {
                return Err(format!("{:?} does not match the filter", observation));
            }
            Filter::Object(_) => {}
        }
        shadow.last_revision = revision;
        match &observation {
            Observation::Opened { view, .. } => {
                if shadow
                    .views
                    .insert(view.id(), (view.ty().clone(), view.properties().clone()))
                    .is_some()
                {
                    return Err(format!("{} opened twice", view.id()));
                }
            }
            Observation::Changed {
                object,
                property,
                value,
                ..
            } => {
                let (_, properties) = shadow
                    .views
                    .get_mut(object)
                    .ok_or_else(|| format!("a diff for absent {object}"))?;
                properties.insert(property.clone(), value.clone());
            }
            Observation::Closed { object, .. } => {
                if shadow.views.remove(object).is_none() {
                    return Err(format!("{object} closed twice"));
                }
            }
            Observation::Signalled { object, .. } => {
                if !shadow.views.contains_key(object) {
                    return Err(format!("a signal for absent {object}"));
                }
            }
            Observation::Finished { .. } => {}
        }
        seen.push(observation);
        checks += 1;
    }
    Ok(checks)
}

/// Compare one shadow with the graph, for its filter.
fn compare(graph: &Graph, shadow: &Shadow) -> Result<usize, String> {
    let current = state(graph);
    match shadow.filter {
        Filter::All => {
            if shadow.views != current {
                return Err(format!(
                    "the stream rebuilt {} objects, the graph holds {}",
                    shadow.views.len(),
                    current.len()
                ));
            }
        }
        Filter::Object(id) => match current.get(&id) {
            Some((ty, properties)) => {
                if shadow.views.get(&id) != Some(&(ty.clone(), properties.clone())) {
                    return Err(format!("the stream does not match the graph for {id}"));
                }
                if shadow.views.len() != 1 {
                    return Err(format!("a one-object filter holds {} objects", shadow.views.len()));
                }
            }
            None => {
                if !shadow.views.is_empty() {
                    return Err(format!("the stream still holds a closed {id}"));
                }
            }
        },
    }
    Ok(1)
}

/// Build a shadow from a fresh snapshot, and check the snapshot against the graph.
fn make_shadow(graph: &Graph, subscription: Subscription, filter: Filter) -> Result<(Shadow, usize), String> {
    let mut views = BTreeMap::new();
    for view in subscription.snapshot().objects() {
        views.insert(view.id(), (view.ty().clone(), view.properties().clone()));
    }
    let shadow = Shadow {
        last_revision: subscription.snapshot().revision(),
        subscription,
        filter,
        views,
    };
    compare(graph, &shadow)?;
    Ok((shadow, 2))
}

/// One action, which may or may not change the graph.
fn step(rng: &mut Rng, graph: &mut Graph, members: &[Members], live: &mut Vec<ObjectId>) -> Result<(), String> {
    let before = graph.revision();
    match rng.below(100) {
        // Create an object of a random type with a complete, valid state.
        0..=19 => {
            if live.len() >= OBJECT_POOL {
                return Ok(());
            }
            let shape = &members[rng.below(members.len() as u32) as usize];
            let properties = fields(rng, &shape.properties);
            let id = graph
                .create(shape.key.clone(), properties)
                .map_err(|error| format!("a valid create was refused: {error}"))?;
            live.push(id);
        }
        // Change one property, usually to a valid value.
        20..=49 => {
            let Some(object) = pick(rng, graph, live)? else {
                return Ok(());
            };
            let shape = shape_of(graph, members, object)?;
            // An object type with no properties has nothing to set.
            if shape.properties.is_empty() {
                return Ok(());
            }
            let (property, ty) = &shape.properties[rng.below(shape.properties.len() as u32) as usize];
            let wrong = rng.chance(6);
            let value = value(rng, ty, wrong);
            let result = graph.set(object, property, value);
            if wrong && !matches!(&result, Err(ModelError::WrongType { .. })) {
                return Err(format!("a wrongly typed `{property}` was accepted: {result:?}"));
            }
            refusals(result, before, graph)?;
        }
        // Raise a declared signal.
        50..=59 => {
            let Some(object) = pick(rng, graph, live)? else {
                return Ok(());
            };
            let shape = shape_of(graph, members, object)?;
            if shape.signals.is_empty() {
                return Ok(());
            }
            let (signal, declared) = &shape.signals[rng.below(shape.signals.len() as u32) as usize];
            let fields = fields(rng, declared);
            refusals(graph.signal(object, signal, fields), before, graph)?;
        }
        // Call a declared method, then complete one pending operation.
        60..=69 => {
            let Some(object) = pick(rng, graph, live)? else {
                return Ok(());
            };
            let shape = shape_of(graph, members, object)?;
            if !shape.methods.is_empty() {
                let (method, declared) = &shape.methods[rng.below(shape.methods.len() as u32) as usize];
                let arguments = fields(rng, declared);
                let operation = graph
                    .call(object, method, arguments)
                    .map_err(|error| format!("a valid call was refused: {error}"))?;
                let outcome = if rng.chance(2) {
                    OperationOutcome::Done
                } else {
                    OperationOutcome::Failed(gnoblin_model::ErrorCode::parse("refused").expect("kebab-case"))
                };
                refusals(graph.complete(operation.id(), outcome), before, graph)?;
            }
        }
        // Close an object, which also finishes its pending operations.
        70..=79 => {
            let Some(object) = pick(rng, graph, live)? else {
                return Ok(());
            };
            live.retain(|candidate| *candidate != object);
            refusals(graph.close(object), before, graph)?;
        }
        // Probe members the schema does not declare, which must be refused.
        80..=87 => {
            let Some(object) = pick(rng, graph, live)? else {
                return Ok(());
            };
            if graph.set(object, "undeclared_property", Value::Bool(true)).is_ok() {
                return Err("an undeclared property was accepted".to_owned());
            }
            if graph.call(object, "undeclared_method", Fields::new()).is_ok() {
                return Err("an undeclared method was accepted".to_owned());
            }
            if graph.signal(object, "undeclared_signal", Fields::new()).is_ok() {
                return Err("an undeclared signal was accepted".to_owned());
            }
            if graph.revision() != before {
                return Err("a refused member published something".to_owned());
            }
        }
        _ => {}
    }
    Ok(())
}

/// A live object, or `None` when the graph holds none.
fn pick(rng: &mut Rng, graph: &Graph, live: &[ObjectId]) -> Result<Option<ObjectId>, String> {
    if live.is_empty() {
        return Ok(None);
    }
    let object = live[rng.below(live.len() as u32) as usize];
    if graph.object(object).is_none() {
        return Err(format!("{object} is in the live list but not in the graph"));
    }
    Ok(Some(object))
}

/// The schema members for the type of a live object.
fn shape_of<'a>(graph: &Graph, members: &'a [Members], object: ObjectId) -> Result<&'a Members, String> {
    let ty = graph
        .object(object)
        .ok_or_else(|| format!("{object} is not live"))?
        .ty();
    members
        .iter()
        .find(|shape| &shape.key == ty)
        .ok_or_else(|| format!("the graph holds a type the schema does not declare: {ty}"))
}

/// A refused mutation must publish nothing; a successful one must publish something.
fn refusals(result: Result<Revision, ModelError>, before: Revision, graph: &Graph) -> Result<(), String> {
    match result {
        Ok(revision) => {
            if revision <= before {
                return Err(format!("{revision} did not advance past {before}"));
            }
        }
        Err(error) => {
            if graph.revision() != before {
                return Err(format!("a refused mutation published something: {error}"));
            }
        }
    }
    Ok(())
}

/// One seed: the observation stream it produced and the checks it ran.
fn run(seed: u64) -> (Vec<Observation>, usize) {
    let catalog = Catalog::load_dir(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gnoblin-schema/schema"
    )))
    .expect("the checked-in schema loads");
    let shapes = members(&catalog);
    let mut rng = Rng::new(seed);
    let mut graph = Graph::new(catalog);

    let mut live: Vec<ObjectId> = Vec::new();
    let mut shadows: Vec<Shadow> = Vec::new();
    let mut seen = Vec::new();
    let mut checks = 0_usize;

    for _ in 0..STEPS_PER_SEQUENCE {
        step(&mut rng, &mut graph, &shapes, &mut live).expect("a step is valid");

        // Subscribe, filtering by every filter shape, including one object that has
        // already closed.
        if shadows.len() < SUBSCRIPTION_POOL && rng.chance(6) {
            let filter = match rng.below(3) {
                0 | 1 => Filter::All,
                _ => Filter::Object(if live.is_empty() || rng.chance(3) {
                    ObjectId::new(u64::from(rng.below(12) + 1))
                } else {
                    live[rng.below(live.len() as u32) as usize]
                }),
            };
            let subscription = graph.subscribe(filter);
            let (shadow, snapshot_checks) =
                make_shadow(&graph, subscription, filter).unwrap_or_else(|error| panic!("seed {seed}: {error}"));
            shadows.push(shadow);
            checks += snapshot_checks;
        }

        if !shadows.is_empty() && rng.chance(20) {
            let index = rng.below(shadows.len() as u32) as usize;
            let dropped = shadows.remove(index);
            if !graph.unsubscribe(dropped.subscription.id()) {
                panic!("seed {seed}: a live subscription was not known to the graph");
            }
        }

        for shadow in &mut shadows {
            checks += drain(&mut graph, shadow, &mut seen).unwrap_or_else(|error| panic!("seed {seed}: {error}"));
            checks += compare(&graph, shadow).unwrap_or_else(|error| panic!("seed {seed}: {error}"));
        }

        for object in graph.objects() {
            if object.revision() > graph.revision() {
                panic!("seed {seed}: {} is ahead of the graph", object.id());
            }
            checks += 1;
        }
    }

    (seen, checks)
}

#[test]
fn random_sequences_keep_the_snapshot_then_diffs_rule() {
    let mut checks = 0_usize;
    let mut observations = 0_usize;
    for seed in 0..SEQUENCES {
        let (first, first_checks) = run(seed);
        let (second, second_checks) = run(seed);
        assert_eq!(first, second, "seed {seed} produced two different observation streams");
        assert_eq!(first_checks, second_checks);
        checks += first_checks;
        observations += first.len();
    }
    let steps = SEQUENCES * u64::from(STEPS_PER_SEQUENCE);
    println!(
        "model: {SEQUENCES} sequences x {STEPS_PER_SEQUENCE} steps = {steps} steps, {observations} observations, \
         {checks} invariant checks"
    );
    assert!(
        checks >= MIN_CHECKS,
        "expected at least {MIN_CHECKS} invariant checks, ran {checks}"
    );
}
