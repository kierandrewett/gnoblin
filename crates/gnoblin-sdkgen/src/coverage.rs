//! What the schema leaves undescribed: the members of the libraries' catalogs that have no comment above their table, by kind.
//!
//! `cargo xtask docs-coverage` prints it. A description is the `#` comment above a table (`docs.rs`); the count is of the tables the
//! files have, so a member an object inherits from its base is counted once, under the class that declares it.

use gnoblin_schema::{Namespace, augment_class_name};

use crate::SdkSpec;
use crate::docs::NamespaceDocs;
use crate::reference::own_members;

/// The tables of one kind, and the ones without a description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindCoverage {
    /// The kind: `class`, `property`, `method`, `parameter`, `signal`, `signal field`, `enum`, `enum value`, `callback`,
    /// `callback parameter`, `variant`, `variant property`.
    pub kind: &'static str,
    /// How many tables of the kind the files have.
    pub total: usize,
    /// The ones without a description, as `namespace::class.member`.
    pub missing: Vec<String>,
}

/// What the schema says and what it leaves out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// Each kind, in a fixed order.
    pub kinds: Vec<KindCoverage>,
    /// The schema files of the libraries' catalogs that have no `title`, as their file names.
    pub untitled: Vec<String>,
}

impl Coverage {
    /// How many tables have no description, whatever their kind.
    #[must_use]
    pub fn missing(&self) -> usize {
        self.kinds.iter().map(|kind| kind.missing.len()).sum()
    }
}

struct CoverageCount {
    kind: &'static str,
    total: usize,
    missing: Vec<String>,
}

impl CoverageCount {
    fn new(kind: &'static str) -> CoverageCount {
        CoverageCount {
            kind,
            total: 0,
            missing: Vec::new(),
        }
    }

    fn see(&mut self, described: &str, name: impl FnOnce() -> String) {
        self.total += 1;
        if described.is_empty() {
            self.missing.push(name());
        }
    }
}

/// Counts the tables of every namespace the libraries serve, by kind, and which have no description.
#[must_use]
pub fn coverage(spec: &SdkSpec) -> Coverage {
    let mut classes = CoverageCount::new("class");
    let mut properties = CoverageCount::new("property");
    let mut methods = CoverageCount::new("method");
    let mut parameters = CoverageCount::new("parameter");
    let mut signals = CoverageCount::new("signal");
    let mut fields = CoverageCount::new("signal field");
    let mut enums = CoverageCount::new("enum");
    let mut values = CoverageCount::new("enum value");
    let mut callbacks = CoverageCount::new("callback");
    let mut callback_params = CoverageCount::new("callback parameter");
    let mut variants = CoverageCount::new("variant");
    let mut variant_properties = CoverageCount::new("variant property");
    let mut untitled = Vec::new();
    for namespace in spec.objects.namespaces() {
        let docs = spec.docs.namespace(&namespace);
        let at = |class: &str, member: &str| format!("{namespace}::{class}.{member}");
        for schema in spec
            .objects
            .schemas()
            .iter()
            .filter(|schema| schema.namespace == namespace)
        {
            if schema.title.is_none() {
                untitled.push(
                    std::path::Path::new(&schema.origin)
                        .file_name()
                        .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
                );
            }
            for object in schema.objects.iter().filter(|object| object.augment_of.is_none()) {
                classes.see(docs.class(&object.name), || format!("{namespace}::{}", object.name));
                let own = own_members(&spec.objects, object, &namespace);
                for property in &own.properties {
                    properties.see(docs.member(&object.name, &property.name, |_| None), || {
                        at(&object.name, &property.name)
                    });
                }
                for method in &own.methods {
                    methods.see(docs.member(&object.name, &method.name, |_| None), || {
                        at(&object.name, &method.name)
                    });
                    for param in &method.params {
                        parameters.see(docs.child(&object.name, &method.name, &param.name, |_| None), || {
                            at(&object.name, &format!("{}({})", method.name, param.name))
                        });
                    }
                }
                for signal in &own.signals {
                    signals.see(docs.member(&object.name, &signal.name, |_| None), || {
                        at(&object.name, &signal.name)
                    });
                    for field in &signal.fields {
                        fields.see(docs.child(&object.name, &signal.name, &field.name, |_| None), || {
                            at(&object.name, &format!("{}({})", signal.name, field.name))
                        });
                    }
                }
                for variant in &object.variants {
                    variants.see(docs.variant(&object.name, &variant.name), || {
                        at(&object.name, &variant.name)
                    });
                    for property in &variant.properties {
                        variant_properties.see(
                            docs.variant_property(&object.name, &variant.name, &property.name),
                            || at(&object.name, &format!("{}.{}", variant.name, property.name)),
                        );
                    }
                }
            }
            augments(&namespace, docs, schema, &mut properties, &mut methods, &mut parameters);
            for declared in &schema.enums {
                enums.see(docs.enum_doc(&declared.name), || {
                    format!("{namespace}::{}", declared.name)
                });
                for value in &declared.values {
                    values.see(docs.enum_value(&declared.name, value), || at(&declared.name, value));
                }
            }
            for declared in &schema.callbacks {
                callbacks.see(docs.callback(&declared.name), || {
                    format!("{namespace}::{}", declared.name)
                });
                for param in &declared.params {
                    callback_params.see(docs.callback_param(&declared.name, &param.name), || {
                        at(&declared.name, &param.name)
                    });
                }
            }
        }
    }
    let kinds = [
        classes,
        properties,
        methods,
        parameters,
        signals,
        fields,
        variants,
        variant_properties,
        enums,
        values,
        callbacks,
        callback_params,
    ]
    .into_iter()
    .map(|counter| KindCoverage {
        kind: counter.kind,
        total: counter.total,
        missing: counter.missing,
    })
    .collect();
    Coverage { kinds, untitled }
}

/// The members a library adds to the core's classes count with the properties and methods of the classes.
fn augments(
    namespace: &Namespace,
    docs: &NamespaceDocs,
    schema: &gnoblin_schema::Schema,
    properties: &mut CoverageCount,
    methods: &mut CoverageCount,
    parameters: &mut CoverageCount,
) {
    for augment in &schema.augments {
        let class = augment_class_name(&augment.class);
        for property in &augment.properties {
            properties.see(docs.member(&class, &property.name, |_| None), || {
                format!("{namespace}::{class}.{}", property.name)
            });
        }
        for method in &augment.methods {
            methods.see(docs.member(&class, &method.name, |_| None), || {
                format!("{namespace}::{class}.{}", method.name)
            });
            for param in &method.params {
                parameters.see(docs.child(&class, &method.name, &param.name, |_| None), || {
                    format!("{namespace}::{class}.{}({})", method.name, param.name)
                });
            }
        }
    }
}
