//! The rules of a tagged family: a record with a `tag` and the variants that stand for the values of that tag.
//!
//! A family is one record on the wire: its fields, one of which (the `tag`, an enum) says which variant a value is. A variant is a
//! value class with fields of its own, and each field says where the family record carries it (`field = "amount"`, or
//! `field = "shader.source"` for a field of the record a family field holds). The wire does not change, so a family can be added to
//! a record that exists, and a variant adds no class to the object table.
//!
//! The checks, which need the enums and the records of the whole namespace and so run in [`crate::Catalog::new`]:
//!
//! - the `tag` names a property of the family that is an enum, always there (no default, not `optional`);
//! - every value of the enum has exactly one variant, so a value read from the wire is always a variant, and no variant names a
//!   value the enum lacks;
//! - a variant's name is not the name of a class, an enum or a callback of the namespace, nor of another variant;
//! - every `field` names a family field other than the tag, or a field of the record that family field holds, with the same type as
//!   the variant's property. A property that is `optional` is carried by a field that is too;
//! - no two properties of a variant are carried by one field;
//! - every field of the family a value must have (no default, not `optional`) is carried by the variant, and so is every such field
//!   of a record the variant carries a part of, so any variant makes a complete family record.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{ObjectKind, ObjectType, Property, Schema, TaggedVariant, Type};

/// Checks every family of `schemas`, and pushes each problem it finds.
pub(crate) fn check(schemas: &[Schema], problems: &mut Vec<String>) {
    let namespaces: BTreeSet<&str> = schemas.iter().map(|schema| schema.namespace.as_str()).collect();
    for namespace in namespaces {
        let files: Vec<&Schema> = schemas
            .iter()
            .filter(|schema| schema.namespace.as_str() == namespace)
            .collect();
        let objects: BTreeMap<&str, &ObjectType> = files
            .iter()
            .flat_map(|schema| schema.objects.iter())
            .map(|object| (object.name.as_str(), object))
            .collect();
        // The names a variant's class may not have: a class, an enum or a callback of the namespace. A variant adds its own.
        let mut taken: BTreeSet<&str> = objects.keys().copied().collect();
        for schema in &files {
            taken.extend(schema.enums.iter().map(|declared| declared.name.as_str()));
            taken.extend(schema.callbacks.iter().map(|declared| declared.name.as_str()));
        }
        for schema in &files {
            for object in schema.objects.iter().filter(|object| !object.variants.is_empty()) {
                check_family(&files, namespace, object, &objects, &mut taken, problems);
            }
        }
    }
}

fn check_family<'a>(
    files: &[&'a Schema],
    namespace: &str,
    family: &'a ObjectType,
    objects: &BTreeMap<&str, &'a ObjectType>,
    taken: &mut BTreeSet<&'a str>,
    problems: &mut Vec<String>,
) {
    let owner = format!("family `{}`", family.name);
    if family.kind != ObjectKind::Record {
        return; // parse refused it already
    }
    let Some(tag) = family.tag.as_deref() else {
        return;
    };
    let Some(tag_property) = family.properties.iter().find(|property| property.name == tag) else {
        problems.push(format!("{owner}: its `tag` `{tag}` is not a property of the record"));
        return;
    };
    let Type::Enum(enum_name) = &tag_property.ty else {
        problems.push(format!(
            "{owner}: its `tag` `{tag}` must be an enum, got `{}`",
            tag_property.ty
        ));
        return;
    };
    if tag_property.default.is_some() || tag_property.optional {
        problems.push(format!(
            "{owner}: its `tag` `{tag}` is always there: no default, not `optional`"
        ));
    }
    let Some(declared) = files
        .iter()
        .flat_map(|schema| schema.enums.iter())
        .find(|declared| &declared.name == enum_name)
    else {
        problems.push(format!(
            "{owner}: its `tag` is the enum `{enum_name}`, which the namespace does not declare"
        ));
        return;
    };

    let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
    for variant in &family.variants {
        if !declared.values.contains(&variant.tag) {
            problems.push(format!(
                "variant `{}` of {owner}: `{}` is not a value of the enum `{enum_name}`",
                variant.name, variant.tag
            ));
        } else if let Some(other) = seen.insert(variant.tag.as_str(), variant.name.as_str()) {
            problems.push(format!(
                "{owner}: the variants `{other}` and `{}` both stand for `{}`",
                variant.name, variant.tag
            ));
        }
        if !taken.insert(variant.name.as_str()) {
            problems.push(format!(
                "variant `{}` of {owner}: the name is a class, an enum, a callback or a variant of namespace `{namespace}` already",
                variant.name
            ));
        }
        check_variant(family, tag, variant, objects, problems);
    }
    for value in &declared.values {
        if !seen.contains_key(value.as_str()) {
            problems.push(format!(
                "{owner}: no variant stands for `{value}`, a value of `{enum_name}`"
            ));
        }
    }
}

/// What a `field` path names: a family field, and when the path has two parts, a field of the record that holds.
struct FamilyField<'a> {
    family: &'a Property,
    inner: Option<&'a Property>,
}

fn check_variant(
    family: &ObjectType,
    tag: &str,
    variant: &TaggedVariant,
    objects: &BTreeMap<&str, &ObjectType>,
    problems: &mut Vec<String>,
) {
    let mut carried: Vec<(&str, Option<&str>)> = Vec::new();
    for property in &variant.properties {
        let owner = format!(
            "property `{}` of variant `{}` of family `{}`",
            property.name, variant.name, family.name
        );
        let Some(path) = property.field.as_deref() else {
            continue; // parse refused it already
        };
        let target = match resolve(family, tag, path, objects) {
            Ok(target) => target,
            Err(reason) => {
                problems.push(format!("{owner}: `field = \"{path}\"` {reason}"));
                continue;
            }
        };
        let carrier = target.inner.unwrap_or(target.family);
        if carrier.ty != property.ty {
            problems.push(format!(
                "{owner}: it is `{}` and the field `{path}` that carries it is `{}`",
                property.ty, carrier.ty
            ));
        }
        if property.optional && !carrier.optional {
            problems.push(format!(
                "{owner}: it is `optional` and the field `{path}` that carries it is not: nothing could say it is absent"
            ));
        }
        if carried.iter().any(|(family_field, inner)| {
            *family_field == target.family.name.as_str()
                && (inner.is_none()
                    || target.inner.is_none()
                    || *inner == target.inner.map(|inner| inner.name.as_str()))
        }) {
            problems.push(format!(
                "{owner}: another property of the variant is carried by `{path}` or a part of it"
            ));
        }
        carried.push((
            target.family.name.as_str(),
            target.inner.map(|inner| inner.name.as_str()),
        ));
    }

    for field in &family.properties {
        if field.name == tag {
            continue;
        }
        let mine: Vec<Option<&str>> = carried
            .iter()
            .filter(|(family_field, _)| *family_field == field.name)
            .map(|(_, inner)| *inner)
            .collect();
        let owner = format!("variant `{}` of family `{}`", variant.name, family.name);
        if mine.is_empty() {
            if !field.optional && field.default.is_none() {
                problems.push(format!(
                    "{owner}: the family field `{}` is always there, and the variant carries nothing in it",
                    field.name
                ));
            }
            continue;
        }
        if mine.contains(&None) {
            continue;
        }
        // Part of a record: every field of that record a value must have is carried.
        let Type::ObjectRef(record) = &field.ty else {
            continue;
        };
        let Some(record) = objects.get(record.as_str()) else {
            continue;
        };
        for inner in &record.properties {
            if !inner.optional && inner.default.is_none() && !mine.contains(&Some(inner.name.as_str())) {
                problems.push(format!(
                    "{owner}: the variant carries part of `{}` and leaves out `{}.{}`, which a `{}` always has",
                    field.name, field.name, inner.name, record.name
                ));
            }
        }
    }
}

/// The family field (and the field of the record it holds) that `path` names.
fn resolve<'a>(
    family: &'a ObjectType,
    tag: &str,
    path: &str,
    objects: &BTreeMap<&str, &'a ObjectType>,
) -> Result<FamilyField<'a>, String> {
    let (first, rest) = match path.split_once('.') {
        Some((first, rest)) => (first, Some(rest)),
        None => (path, None),
    };
    if first == tag {
        return Err("is the `tag`, which a variant fixes and does not carry".to_owned());
    }
    let Some(family_field) = family.properties.iter().find(|property| property.name == first) else {
        return Err(format!("names no field of the family `{}`", family.name));
    };
    let Some(rest) = rest else {
        return Ok(FamilyField {
            family: family_field,
            inner: None,
        });
    };
    let Type::ObjectRef(record) = &family_field.ty else {
        return Err(format!(
            "goes into `{first}`, which is `{}` and holds no record",
            family_field.ty
        ));
    };
    let Some(record) = objects
        .get(record.as_str())
        .filter(|record| record.kind == ObjectKind::Record)
    else {
        return Err(format!(
            "goes into `{first}`, which does not hold a record of this namespace"
        ));
    };
    if rest.contains('.') {
        return Err("goes deeper than a field of a field".to_owned());
    }
    let Some(inner) = record.properties.iter().find(|property| property.name == rest) else {
        return Err(format!("names no field of the record `{}`", record.name));
    };
    Ok(FamilyField {
        family: family_field,
        inner: Some(inner),
    })
}
