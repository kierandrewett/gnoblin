//! Differential test: the sandbox's `string.find`, `string.match`, `string.gmatch` and
//! `string.gsub` against the stock Lua 5.5 C implementations.
//!
//! Every case runs twice. The stock side is a plain `mlua` state with the full string
//! library and no sandbox. The sandbox side is the configuration profile, whose four
//! pattern functions are the Rust replacements. Both sides run the same Lua source,
//! wrapped in `pcall`, so an error reaches Lua as its text and each result is
//! rendered with its type. `nil` and `false` therefore stay distinct.
//!
//! A case that exhausts the sandbox budget is skipped: the stock C matcher never
//! stops, so the budget is the one allowed difference. Every other difference fails
//! the test.
//!
//! The corpus is hand-written. The random cases come from a fixed-seed generator, so
//! a failure reproduces exactly. The 100k run is `#[ignore]`d and runs with
//! `--ignored` on Agent37.

use std::rc::Rc;

use gnoblin_lua_sandbox::{Guard, Limits, LogLevel, Profile, Sink, create};
use mlua::{Lua, LuaOptions, StdLib};

/// Lua prelude prepended to every case. It renders each returned value with its type
/// so that two results compare equal only when they match exactly.
const PRELUDE: &str = r##"
local function render(...)
    local out = {}
    for i = 1, select("#", ...) do
        local v = select(i, ...)
        local t = type(v)
        if t == "nil" then out[#out + 1] = "nil"
        elseif t == "boolean" then out[#out + 1] = tostring(v)
        elseif t == "number" then out[#out + 1] = "n:" .. tostring(v)
        elseif t == "string" then out[#out + 1] = "s:" .. string.format("%q", v)
        else out[#out + 1] = t end
    end
    return table.concat(out, " ")
end
"##;

/// One library call to compare. Strings are embedded with `{:?}`, which escapes every
/// byte Lua cannot read back, so any subject survives the round trip.
#[derive(Clone, Debug)]
enum Call {
    Find {
        subject: String,
        pattern: String,
        init: Option<i64>,
        plain: bool,
    },
    Match {
        subject: String,
        pattern: String,
        init: Option<i64>,
    },
    Gmatch {
        subject: String,
        pattern: String,
        init: Option<i64>,
    },
    GsubStr {
        subject: String,
        pattern: String,
        repl: String,
        max: Option<i64>,
    },
    GsubTable {
        subject: String,
        pattern: String,
        key: String,
        max: Option<i64>,
    },
    GsubFunc {
        subject: String,
        pattern: String,
        max: Option<i64>,
    },
}

/// The Lua expression for `call`, returning its rendered results as one string.
fn expression(call: &Call) -> String {
    fn opt(v: Option<i64>) -> String {
        v.map_or_else(|| "nil".into(), |n| n.to_string())
    }
    match call {
        Call::Find {
            subject,
            pattern,
            init,
            plain,
        } => {
            format!("render(string.find({subject:?}, {pattern:?}, {}, {plain}))", opt(*init))
        }
        Call::Match { subject, pattern, init } => {
            format!("render(string.match({subject:?}, {pattern:?}, {}))", opt(*init))
        }
        Call::Gmatch { subject, pattern, init } => format!(
            "(function() local it = string.gmatch({subject:?}, {pattern:?}, {}) \
             local out = {{}} for i = 1, 200 do local v = render(it()) \
             if v == 'nil' then break end out[#out + 1] = v end \
             return table.concat(out, ' | ') end)()",
            opt(*init)
        ),
        Call::GsubStr {
            subject,
            pattern,
            repl,
            max,
        } => {
            format!("render(string.gsub({subject:?}, {pattern:?}, {repl:?}, {}))", opt(*max))
        }
        Call::GsubTable {
            subject,
            pattern,
            key,
            max,
        } => format!(
            "render(string.gsub({subject:?}, {pattern:?}, {{[{key:?}] = 'X', [1] = false}}, {}))",
            opt(*max)
        ),
        Call::GsubFunc { subject, pattern, max } => format!(
            "render(string.gsub({subject:?}, {pattern:?}, function(...) \
             if select('#', ...) > 0 then return (...) end return nil end, {}))",
            opt(*max)
        ),
    }
}

/// The complete chunk for one case: the prelude, then the call under `pcall`.
fn chunk(call: &Call) -> String {
    format!(
        "{PRELUDE}\nlocal ok, r = pcall(function() return {} end)\n\
         if ok then return 'ok ' .. r end\nreturn 'err ' .. tostring(r)",
        expression(call)
    )
}

fn stock_lua() -> Lua {
    Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default()).expect("stock state")
}

fn sandbox_lua() -> (Lua, Rc<Guard>, Limits) {
    let limits = Limits::default();
    let guard = Rc::new(Guard::new());
    let sink: Sink = Rc::new(|_: LogLevel, _: &str| {});
    let lua = create(Profile::Config, &limits, Rc::clone(&guard), sink).expect("sandbox state");
    (lua, guard, limits)
}

/// Run every case on both states. Returns `(compared, skipped_over_budget)`.
fn check(cases: &[Call]) -> (usize, usize) {
    let stock = stock_lua();
    let (sandbox, guard, limits) = sandbox_lua();
    let (mut compared, mut skipped) = (0, 0);
    for call in cases {
        let source = chunk(call);
        guard.begin(&limits);
        let want: mlua::LuaString = stock
            .load(source.as_str())
            .set_name("=case")
            .eval()
            .expect("stock case runs");
        guard.begin(&limits);
        let got: Result<mlua::LuaString, mlua::Error> = sandbox.load(source.as_str()).set_name("=case").eval();
        if guard.aborted() {
            skipped += 1;
            continue;
        }
        let got = got.unwrap_or_else(|e| panic!("sandbox case failed to run for {call:?}: {e}"));
        compared += 1;
        assert_eq!(
            got.as_bytes(),
            want.as_bytes(),
            "parity mismatch for {call:?}: got {:?}, want {:?}",
            String::from_utf8_lossy(&got.as_bytes()),
            String::from_utf8_lossy(&want.as_bytes()),
        );
    }
    (compared, skipped)
}

/// Hand-written corpus: every class, set, range, `%b`, `%f`, back-reference, anchor,
/// greedy and lazy repetition, position capture, and the error paths.
fn corpus() -> Vec<Call> {
    let subjects = [
        "",
        "a",
        "hello world",
        "  x  ",
        "abc123def",
        "key = value",
        "[[nested]]",
        "THE (quick) fox",
        "aaa",
        "f(a(b)c)d",
        "\u{0}\u{1}\u{ff}",
        "%d+",
        "x.y-z",
    ];
    let patterns = [
        "",
        "a",
        "^a",
        "a$",
        "^$",
        ".",
        "%a+",
        "%d+",
        "%s*",
        "%w+",
        "%p",
        "%c",
        "%x",
        "%u",
        "%l",
        "%g",
        "%%",
        "[%a_][%w_]*",
        "[^%s]+",
        "[a-c]+",
        "[%]]",
        "[]]",
        "[^]]",
        "a*",
        "a-",
        "a?",
        "a+",
        ".-",
        ".*$",
        "(a)(b)",
        "()a()",
        "(%d+)%s*=%s*(%w+)",
        "%b()",
        "%bxy",
        "%f[%w]%w+",
        "%f[%W]",
        "(a)%1",
        "(.)%1",
        "((a)(b))",
        "[a-]",
        "[-a]",
        "[%a-]",
        "%z",
        "a%",
        "[a",
        "(",
        ")",
        "(a",
        "a)",
        "%",
        "[",
        "%b",
        "%f",
        "%1",
        "(a)%2",
        "%0",
        "()",
        "(()())",
        "[%f[%a]]",
        "x*y*z*",
        "%.",
        "%-",
        "(a(b)",
        "%g+",
        "[a%-z]",
        "[%w%.]+",
        "[^%-]",
    ];
    let inits: [Option<i64>; 7] = [None, Some(1), Some(2), Some(-1), Some(-3), Some(0), Some(100)];
    let mut out = Vec::new();
    for s in subjects {
        for p in patterns {
            for init in inits {
                out.push(Call::Find {
                    subject: s.into(),
                    pattern: p.into(),
                    init,
                    plain: false,
                });
                out.push(Call::Match {
                    subject: s.into(),
                    pattern: p.into(),
                    init,
                });
                out.push(Call::Gmatch {
                    subject: s.into(),
                    pattern: p.into(),
                    init,
                });
            }
            out.push(Call::Find {
                subject: s.into(),
                pattern: p.into(),
                init: None,
                plain: true,
            });
            out.push(Call::GsubStr {
                subject: s.into(),
                pattern: p.into(),
                repl: "[%0]".into(),
                max: None,
            });
            out.push(Call::GsubStr {
                subject: s.into(),
                pattern: p.into(),
                repl: "%%".into(),
                max: Some(1),
            });
            out.push(Call::GsubStr {
                subject: s.into(),
                pattern: p.into(),
                repl: "%1".into(),
                max: Some(2),
            });
            out.push(Call::GsubTable {
                subject: s.into(),
                pattern: p.into(),
                key: "a".into(),
                max: None,
            });
            out.push(Call::GsubFunc {
                subject: s.into(),
                pattern: p.into(),
                max: Some(3),
            });
        }
    }
    out
}

#[test]
fn corpus_matches_stock_lua() {
    let cases = corpus();
    let (compared, skipped) = check(&cases);
    assert!(
        compared > 500,
        "corpus too small: {compared} compared, {skipped} skipped"
    );
}

/// A deterministic xorshift generator. It needs no crate, and a fixed seed means a
/// failing case reproduces exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const PATTERN_TOKENS: &[&str] = &[
    "a", "b", "c", ".", "%a", "%d", "%s", "%w", "%p", "%u", "%l", "%x", "%A", "%D", "*", "+", "-", "?", "^", "$", "(",
    ")", "[", "]", "%b()", "%f[%w]", "%1", "%2", "%0", "%%", "%", "^a", "[^a]", "[a-c]", "()", "x", " ", "%.",
];
const SUBJECT_CHARS: &[char] = &['a', 'b', 'c', 'x', '1', '2', ' ', '(', ')', '.', '%', '-'];

fn random_pattern(rng: &mut Rng) -> String {
    let n = rng.below(7);
    (0..n)
        .map(|_| PATTERN_TOKENS[rng.below(PATTERN_TOKENS.len() as u64) as usize])
        .collect()
}

fn random_subject(rng: &mut Rng) -> String {
    let n = rng.below(13);
    (0..n)
        .map(|_| SUBJECT_CHARS[rng.below(SUBJECT_CHARS.len() as u64) as usize])
        .collect()
}

fn random_case(rng: &mut Rng) -> Call {
    let subject = random_subject(rng);
    let pattern = random_pattern(rng);
    let init = match rng.below(4) {
        0 => None,
        1 => Some(rng.below(20) as i64 - 10),
        _ => Some(rng.below(6) as i64 + 1),
    };
    let max = match rng.below(3) {
        0 => None,
        _ => Some(rng.below(5) as i64),
    };
    match rng.below(7) {
        0 => Call::Find {
            subject,
            pattern,
            init,
            plain: rng.below(4) == 0,
        },
        1 => Call::Match { subject, pattern, init },
        2 => Call::Gmatch { subject, pattern, init },
        3 => {
            let repl = ["%0", "%1", "%%", "x", "%"][rng.below(5) as usize].to_string();
            Call::GsubStr {
                subject,
                pattern,
                repl,
                max,
            }
        }
        4 => Call::GsubTable {
            subject,
            pattern,
            key: "a".into(),
            max,
        },
        _ => Call::GsubFunc { subject, pattern, max },
    }
}

fn random_run(count: usize) {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let cases: Vec<Call> = (0..count).map(|_| random_case(&mut rng)).collect();
    let (compared, skipped) = check(&cases);
    eprintln!("random: {compared} compared, {skipped} skipped over budget");
}

#[test]
fn random_small_matches_stock_lua() {
    random_run(2_000);
}

#[test]
#[ignore = "100k cases; run with --ignored on Agent37"]
fn random_100k_matches_stock_lua() {
    random_run(100_000);
}
