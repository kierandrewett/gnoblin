//! The neutral extension fixture parses as an ordinary schema file. Extensions are
//! loaded from host configuration at run time, so this file is test data only.

use gnoblin_schema::parse_schema;

const FIXTURE: &str = include_str!("fixtures/extensions/example-ext.toml");

#[test]
fn example_extension_parses_with_its_namespace_objects_and_secret_field() {
    let schema = parse_schema("example-ext.toml", FIXTURE).expect("fixture parses");

    assert_eq!(schema.namespace.as_str(), "example.ext");
    let names: Vec<&str> = schema.objects.iter().map(|object| object.name.as_str()).collect();
    assert_eq!(names, ["probe", "counter"]);

    let probe = &schema.objects[0];
    let answer = probe
        .methods
        .iter()
        .find(|method| method.name == "answer")
        .expect("probe has an answer method");
    assert!(
        answer
            .params
            .iter()
            .any(|param| param.name == "response" && param.secret)
    );
    assert!(answer.is_sensitive(), "a method with a secret param is sensitive");

    let question = probe
        .signals
        .iter()
        .find(|signal| signal.name == "question")
        .expect("probe has a question signal");
    assert_eq!(question.fields[0].ty.to_string(), "string");
}

#[test]
fn without_secrets_drops_the_secret_method_and_keeps_the_rest() {
    let schema = parse_schema("example-ext.toml", FIXTURE).expect("fixture parses");
    let probe = schema
        .objects
        .iter()
        .find(|object| object.name == "probe")
        .expect("probe");
    let kept = probe.without_secrets().expect("probe is not secret");
    let methods: Vec<&str> = kept.methods.iter().map(|method| method.name.as_str()).collect();
    assert_eq!(methods, ["begin"], "answer carries a secret param and is removed");
    assert_eq!(kept.signals.len(), 1, "the question signal has no secret field");

    let mut hidden = probe.clone();
    hidden.secret = true;
    assert!(hidden.without_secrets().is_none(), "a secret object is hidden whole");
}
