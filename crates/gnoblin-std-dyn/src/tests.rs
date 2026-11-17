//! The call layer without the library: what converts and what refuses.

use super::*;

#[test]
fn numbers_convert_when_whole_and_in_range() {
    assert_eq!(Dyn::Float(3.0).to_u32("n").expect("a whole number"), 3);
    assert_eq!(Dyn::Int(-1).to_i32("n").expect("a signed number"), -1);
    assert!(Dyn::Float(3.5).to_u32("n").is_err());
    assert!(Dyn::Int(-1).to_u64("n").is_err());
    assert!(Dyn::Uint(1 << 40).to_u32("n").is_err());
    assert_eq!(Dyn::Bool(true).to_u32("n").expect("a flag"), 1);
}

#[test]
fn a_record_refuses_a_field_it_does_not_have() {
    let value = Dyn::Map(vec![
        ("x".to_owned(), Dyn::Float(1.0)),
        ("y".to_owned(), Dyn::Float(2.0)),
    ]);
    let mut reader = MapReader::new(&value, "Rect").expect("a record");
    assert!(reader.take("x").is_some());
    let error = reader.finish().expect_err("y was never taken");
    assert_eq!(error.code, gnoblin_core_abi::E_TYPE);
    assert!(error.message.contains("`Rect` has no field `y`"), "{error}");
}

#[test]
fn nothing_is_the_empty_record_and_a_nil_field_is_absent() {
    let mut reader = MapReader::new(&Dyn::Nil, "Rect").expect("nothing is a record");
    assert!(reader.take("x").is_none());
    reader.finish().expect("no fields");
    let value = Dyn::Map(vec![("x".to_owned(), Dyn::Nil)]);
    let mut reader = MapReader::new(&value, "Rect").expect("a record");
    assert!(reader.take("x").is_none());
    reader.finish().expect("a nil field was taken");
}

#[test]
fn an_arena_frees_what_the_library_never_adopted() {
    use std::cell::Cell;
    struct Probe(Rc<Cell<u32>>);
    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    impl Callable for Probe {
        fn invoke(&self, _args: Vec<Dyn>) -> Result<Dyn, String> {
            Ok(Dyn::Nil)
        }
        fn invoke_method(&self, _name: &str, _args: Vec<Dyn>) -> Result<Dyn, String> {
            Ok(Dyn::Nil)
        }
        fn has_method(&self, _name: &str) -> bool {
            false
        }
    }
    let drops = Rc::new(Cell::new(0));
    let mut refused = Arena::new();
    refused.user(Rc::new(Probe(Rc::clone(&drops))));
    drop(refused);
    assert_eq!(drops.get(), 1, "a refused registration frees its object");
    let mut adopted = Arena::new();
    let user = adopted.user(Rc::new(Probe(Rc::clone(&drops))));
    adopted.adopt();
    drop(adopted);
    assert_eq!(drops.get(), 1, "an adopted object is the library's to free");
    // SAFETY: `user` came from `Arena::user` and nothing else owns it.
    unsafe { drop_user(user) };
    assert_eq!(drops.get(), 2);
}

#[test]
fn a_record_of_functions_is_an_object() {
    struct Echo;
    impl Callable for Echo {
        fn invoke(&self, args: Vec<Dyn>) -> Result<Dyn, String> {
            Ok(Dyn::List(args))
        }
        fn invoke_method(&self, _name: &str, _args: Vec<Dyn>) -> Result<Dyn, String> {
            Err("no methods".to_owned())
        }
        fn has_method(&self, _name: &str) -> bool {
            false
        }
    }
    let table = Dyn::Map(vec![("compute".to_owned(), Dyn::Object(Rc::new(Echo)))]);
    let object = table.to_callable("layout").expect("a record of functions");
    assert!(object.has_method("compute"));
    assert!(!object.has_method("revision"));
    let answer = object.invoke_method("compute", vec![Dyn::Uint(1)]).expect("compute");
    assert!(matches!(answer, Dyn::List(items) if items.len() == 1));
}
