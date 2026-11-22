//! Golden fixtures recorded from the TypeScript std, for the lanes of the native std port to match.
//!
//! The TypeScript library under `data/js` is the only specification of the desktop's toolkit, frame, menus, style and input
//! behaviour. A test that runs it calls [`Fixtures::json`] or [`Fixtures::bytes`] with what the
//! library did. In record mode the value is written to the fixture directory. In check mode (the default) it is compared with the
//! committed file and the test fails on the first difference, so a change to the TypeScript that alters a fixture is seen, and so is a
//! fixture that went stale.
//!
//! Which mode a test runs in is the `GNOBLIN_ORACLE` environment variable: `record`, or `check` (unset means `check`).
//! `GNOBLIN_ORACLE_OUT` redirects what record mode writes to a scratch directory laid out like the workspace, which is how
//! `cargo xtask oracle` proves a recording is stable and equal to the committed files without touching them.
//!
//! The native lanes read the same files back through [`Fixtures::read_json`] and [`Fixtures::read_bytes`].

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use gnoblin_tree_client::tree_mirror::{MirrorValue, TxOp};
pub use serde_json::{Map, Value, json};

/// The environment variable that selects the mode.
pub const MODE_VARIABLE: &str = "GNOBLIN_ORACLE";
/// The environment variable that redirects record mode to a scratch directory.
pub const OUT_VARIABLE: &str = "GNOBLIN_ORACLE_OUT";

/// Whether fixtures are compared or written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OracleMode {
    /// Compare with the committed file. The default.
    Check,
    /// Write the file.
    Record,
}

/// The mode this process runs in.
///
/// # Panics
///
/// `GNOBLIN_ORACLE` holds something other than `record` or `check`.
pub fn mode() -> OracleMode {
    match std::env::var(MODE_VARIABLE).as_deref() {
        Ok("record") => OracleMode::Record,
        Ok("check" | "") | Err(_) => OracleMode::Check,
        Ok(other) => panic!("{MODE_VARIABLE} is `{other}`; it is `record` or `check`"),
    }
}

/// Whether this process records.
pub fn recording() -> bool {
    mode() == OracleMode::Record
}

/// The workspace root: the testkit sits at `crates/gnoblin-testkit`.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A directory of fixtures, named relative to the workspace root.
#[derive(Clone, Debug)]
pub struct Fixtures {
    relative: PathBuf,
}

impl Fixtures {
    /// The fixtures under `relative`, for example `crates/gnoblin-std-impl/tests/fixtures/reference`.
    pub fn new(relative: impl Into<PathBuf>) -> Fixtures {
        Fixtures {
            relative: relative.into(),
        }
    }

    /// The committed directory.
    pub fn committed(&self) -> PathBuf {
        workspace_root().join(&self.relative)
    }

    /// Where record mode writes: the committed directory, or its place under `GNOBLIN_ORACLE_OUT`.
    fn target(&self) -> PathBuf {
        match std::env::var_os(OUT_VARIABLE) {
            Some(out) if !out.is_empty() => PathBuf::from(out).join(&self.relative),
            _ => self.committed(),
        }
    }

    /// The text of `value` as a fixture file holds it: two-space indented JSON and one final newline.
    pub fn render(value: &Value) -> String {
        let mut text = serde_json::to_string_pretty(value).expect("a serde_json::Value always serialises");
        text.push('\n');
        text
    }

    /// Records `value` as `<name>.json`, or checks it against the committed file.
    ///
    /// # Panics
    ///
    /// In check mode, the committed file is missing or differs.
    #[track_caller]
    pub fn json(&self, name: &str, value: &Value) {
        self.settle(&format!("{name}.json"), Self::render(value).into_bytes());
    }

    /// Records `bytes` as `<name>.bin`, or checks them against the committed file.
    ///
    /// # Panics
    ///
    /// In check mode, the committed file is missing or differs.
    #[track_caller]
    pub fn bytes(&self, name: &str, bytes: &[u8]) {
        self.settle(&format!("{name}.bin"), bytes.to_vec());
    }

    /// The committed `<name>.json`.
    ///
    /// # Panics
    ///
    /// The file is missing or is not JSON.
    pub fn read_json(&self, name: &str) -> Value {
        let path = self.committed().join(format!("{name}.json"));
        let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    /// The committed `<name>.bin`.
    ///
    /// # Panics
    ///
    /// The file is missing.
    pub fn read_bytes(&self, name: &str) -> Vec<u8> {
        let path = self.committed().join(format!("{name}.bin"));
        fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    #[track_caller]
    fn settle(&self, file: &str, actual: Vec<u8>) {
        match mode() {
            OracleMode::Record => {
                let dir = self.target();
                fs::create_dir_all(&dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
                let path = dir.join(file);
                if fs::read(&path).ok().as_deref() != Some(actual.as_slice()) {
                    fs::write(&path, &actual).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                }
            }
            OracleMode::Check => {
                let path = self.committed().join(file);
                let expected = fs::read(&path).unwrap_or_else(|error| {
                    panic!(
                        "fixture {} is missing ({error}); `cargo xtask oracle record` writes it",
                        path.display()
                    )
                });
                if expected != actual {
                    panic!(
                        "fixture {} differs from what the TypeScript std did:\n{}",
                        path.display(),
                        difference(&expected, &actual)
                    );
                }
            }
        }
    }
}

/// The first lines where two files differ, for a failure message.
fn difference(expected: &[u8], actual: &[u8]) -> String {
    let (expected, actual) = (String::from_utf8_lossy(expected), String::from_utf8_lossy(actual));
    let (left, right): (Vec<&str>, Vec<&str>) = (expected.lines().collect(), actual.lines().collect());
    let mut out = String::new();
    let mut shown = 0;
    for at in 0..left.len().max(right.len()) {
        let (want, got) = (left.get(at), right.get(at));
        if want != got {
            let _ = writeln!(
                out,
                "line {}:\n  committed: {}\n  recorded:  {}",
                at + 1,
                want.map_or("(end of file)", |line| *line),
                got.map_or("(end of file)", |line| *line)
            );
            shown += 1;
            if shown == 5 {
                out.push_str("  ...\n");
                break;
            }
        }
    }
    if out.is_empty() {
        out.push_str("the files differ in bytes that are not text lines\n");
    }
    out
}

// ---- writes, as JSON -----------------------------------------------------------------------------------------------

/// A number as JSON: a finite one as itself, anything else as the word that names it, since JSON has no such number.
pub fn number(value: f64) -> Value {
    serde_json::Number::from_f64(value).map_or_else(|| Value::String(value.to_string()), Value::Number)
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Turns the ops a shell sent into JSON, naming each node it made by the order it made them in.
///
/// Every shell numbers the nodes it creates by its own counter, so the number of a node is not part of the behaviour. A node the
/// recorder saw created is `n1`, `n2`... in creation order, and a node that was there before (the compositor's) is its number.
#[derive(Debug)]
pub struct Writes {
    made: std::collections::BTreeMap<u64, usize>,
    bytes: fn(&[u8]) -> Value,
}

impl Default for Writes {
    fn default() -> Writes {
        Writes::new()
    }
}

impl Writes {
    /// A recorder that has seen no node made, and shows bytes as hex.
    pub fn new() -> Writes {
        Writes {
            made: std::collections::BTreeMap::new(),
            bytes: |bytes| json!({ "bytes": hex(bytes) }),
        }
    }

    /// A recorder that shows a `bytes` value with `render`: a test that links the painter shows a paint list as its ops.
    pub fn with_bytes(render: fn(&[u8]) -> Value) -> Writes {
        Writes {
            made: std::collections::BTreeMap::new(),
            bytes: render,
        }
    }

    /// The name of node `id`.
    pub fn node(&mut self, id: u64) -> Value {
        match self.made.get(&id) {
            Some(index) => Value::String(format!("n{index}")),
            None => json!(id),
        }
    }

    /// The nodes made so far, as `(id, name)` in creation order.
    pub fn names(&self) -> Vec<(u64, String)> {
        let mut made: Vec<(u64, usize)> = self.made.iter().map(|(id, index)| (*id, *index)).collect();
        made.sort_by_key(|(_, index)| *index);
        made.into_iter().map(|(id, index)| (id, format!("n{index}"))).collect()
    }

    /// One value. Doubles are plain numbers; every other kind is an object that names it.
    pub fn value(&mut self, value: &MirrorValue) -> Value {
        match value {
            MirrorValue::Bool(flag) => json!(flag),
            MirrorValue::Double(number_) => number(*number_),
            MirrorValue::Int(int) => json!({ "int": int }),
            MirrorValue::Uint(uint) => json!({ "uint": uint }),
            MirrorValue::Text(text) => json!(text),
            MirrorValue::Node(id) => json!({ "node": self.node(*id) }),
            MirrorValue::List(list) => Value::Array(list.iter().map(|item| number(*item)).collect()),
            MirrorValue::Texts(texts) => json!({ "texts": texts }),
            MirrorValue::Effects(effects) => {
                json!({ "effects": effects.iter().map(|effect| format!("{effect:?}")).collect::<Vec<_>>() })
            }
            MirrorValue::Bytes(bytes) => (self.bytes)(bytes),
        }
    }

    /// The ops of one run of transactions, in order.
    ///
    /// A call's secret text is never recorded; its arguments are.
    pub fn ops(&mut self, ops: &[TxOp]) -> Vec<Value> {
        ops.iter().map(|op| self.op(op)).collect()
    }

    fn op(&mut self, op: &TxOp) -> Value {
        match op {
            TxOp::New { id, class } => {
                let next = self.made.len() + 1;
                self.made.entry(*id).or_insert(next);
                json!({ "op": "new", "node": self.node(*id), "class": class })
            }
            TxOp::Set {
                node,
                key,
                value,
                seconds,
                delay,
                tween,
                samples,
            } => {
                let mut set = Map::new();
                set.insert("op".to_owned(), json!("set"));
                set.insert("node".to_owned(), self.node(*node));
                set.insert("key".to_owned(), json!(key));
                set.insert("value".to_owned(), self.value(value));
                if let Some(seconds) = seconds {
                    set.insert("seconds".to_owned(), number(*seconds));
                }
                if let Some(delay) = delay {
                    set.insert("delay".to_owned(), number(*delay));
                }
                if let Some(tween) = tween {
                    set.insert("tween".to_owned(), self.node(*tween));
                }
                if let Some(samples) = samples {
                    set.insert(
                        "samples".to_owned(),
                        Value::Array(samples.iter().map(|s| number(*s)).collect()),
                    );
                }
                Value::Object(set)
            }
            TxOp::Insert { node, parent, index } => {
                json!({ "op": "insert", "node": self.node(*node), "parent": self.node(*parent), "index": index })
            }
            TxOp::Destroy { node } => json!({ "op": "destroy", "node": self.node(*node) }),
            TxOp::Call { node, method, args, .. } => {
                json!({ "op": "call", "node": self.node(*node), "method": method, "args": hex(args) })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_a_shell_made_is_named_by_the_order_it_was_made_in() {
        let mut writes = Writes::new();
        let ops = [
            TxOp::New {
                id: 900,
                class: "canvas".to_owned(),
            },
            TxOp::New {
                id: 7,
                class: "canvas".to_owned(),
            },
            TxOp::Insert {
                node: 7,
                parent: 900,
                index: 0,
            },
            TxOp::Destroy { node: 3 },
        ];
        let rendered = writes.ops(&ops);
        assert_eq!(rendered[1]["node"], json!("n2"));
        assert_eq!(rendered[2]["node"], json!("n2"));
        assert_eq!(rendered[2]["parent"], json!("n1"));
        assert_eq!(
            rendered[3]["node"],
            json!(3),
            "a node of the compositor keeps its number"
        );
    }

    #[test]
    fn a_double_is_a_plain_number_and_other_kinds_say_what_they_are() {
        let mut writes = Writes::new();
        assert_eq!(writes.value(&MirrorValue::Double(12.0)), json!(12.0));
        assert_eq!(writes.value(&MirrorValue::Int(3)), json!({ "int": 3 }));
        assert_eq!(writes.value(&MirrorValue::List(vec![1.0, 0.5])), json!([1.0, 0.5]));
        assert_eq!(number(f64::NAN), json!("NaN"));
    }

    #[test]
    fn a_difference_names_the_first_lines_that_differ() {
        let text = difference(b"a\nb\nc\n", b"a\nx\nc\n");
        assert!(
            text.contains("line 2") && text.contains("committed: b") && text.contains("recorded:  x"),
            "{text}"
        );
    }
}
