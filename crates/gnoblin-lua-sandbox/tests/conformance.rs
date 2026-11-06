//! A conformance probe for the sandboxed Lua states.
//!
//! Each case is a small Lua snippet that returns `true` when the feature works. The
//! probe runs every case under each [`Profile`] and compares the outcome with the
//! expectation written beside it. The support matrix in the docs is therefore checked
//! against what the VM really does. Run `cargo test -p gnoblin-lua-sandbox --test
//! conformance -- --nocapture` to print the matrix.
//!
//! The language rows follow the Lua 5.5 reference manual, https://www.lua.org/manual/5.5/
//! (section 3 for the language and section 6 for the libraries). A row answers only
//! "does this snippet return true?", so a feature that exists but misbehaves is also a
//! failure. `__gc` is only probed for acceptance: the manual does not promise when, or
//! whether, a finaliser runs.

use std::rc::Rc;

use gnoblin_lua_sandbox::{Guard, Limits, Profile, create};

/// One probe: a name, the snippet, and whether each profile should return `true`.
struct Case {
    feature: &'static str,
    snippet: &'static str,
    config: bool,
    layout: bool,
}

const fn case(feature: &'static str, snippet: &'static str, config: bool, layout: bool) -> Case {
    Case {
        feature,
        snippet,
        config,
        layout,
    }
}

/// The language and library rows. `config` and `layout` are the expected outcomes.
const CASES: &[Case] = &[
    // Language (section 3).
    case(
        "integer and float subtypes",
        "return math.type(1) == 'integer' and math.type(1.0) == 'float'",
        true,
        true,
    ),
    case("floor division //", "return 7 // 2 == 3 and -7 // 2 == -4", true, true),
    case(
        "bitwise operators",
        "return (5 & 3) == 1 and (5 | 3) == 7 and (5 ~ 3) == 6 and (~0) == -1 and (1 << 4) == 16",
        true,
        true,
    ),
    case(
        "goto",
        "local r do local n = 0 ::again:: n = n + 1 if n < 3 then goto again end r = n == 3 end return r",
        true,
        true,
    ),
    case("const attribute", "local x <const> = 4 return x == 4", true, true),
    case(
        "close attribute",
        "local closed = false do local c <close> = setmetatable({}, {__close = function() closed = true end}) end return closed",
        true,
        false,
    ),
    case(
        "varargs and table.pack",
        "local p = table.pack(1, nil, 3) return p.n == 3 and select('#', table.unpack(p, 1, p.n)) == 3",
        true,
        true,
    ),
    // Layout has no setmetatable (D17 allowlist), so no value can carry a metamethod or a
    // <close> handler there; the layout column is false for these rows by design.
    case(
        "arithmetic metamethods",
        "local o = setmetatable({}, {__add = function() return 42 end}) return o + 1 == 42",
        true,
        false,
    ),
    case(
        "concat and len metamethods",
        "local o = setmetatable({}, {__concat = function() return 'c' end, __len = function() return 7 end}) return o .. '' == 'c' and #o == 7",
        true,
        false,
    ),
    case(
        "call metamethod",
        "local o = setmetatable({}, {__call = function() return 9 end}) return o() == 9",
        true,
        false,
    ),
    case(
        "__gc accepted",
        "return setmetatable({}, {__gc = function() end}) ~= nil",
        true,
        false,
    ),
    case(
        "closures",
        "local function mk() local n = 0 return function() n = n + 1 return n end end local f = mk() f() return f() == 2",
        true,
        true,
    ),
    case(
        "multiple assignment",
        "local a, b = 1, 2 a, b = b, a return a == 2 and b == 1",
        true,
        true,
    ),
    case(
        "string methods via metatable",
        "return ('abc'):len() == 3 and ('abc'):sub(2) == 'bc'",
        true,
        true,
    ),
    case("string.upper", "return ('abc'):upper() == 'ABC'", true, false),
    case(
        "integer for-loop at maxinteger",
        "local n = 0 for i = math.maxinteger - 2, math.maxinteger do n = n + 1 end return n == 3",
        true,
        true,
    ),
    case("utf8 escapes in strings", "return #'\\u{20AC}' == 3", true, true),
    // Base library.
    case("pcall", "return pcall(error, 'x') == false", true, false),
    case(
        "xpcall",
        "return xpcall(error, function() return 1 end, 'x') == false",
        true,
        false,
    ),
    case("error and assert", "return not pcall(assert, false)", true, false),
    case(
        "select, type, tostring, tonumber",
        "return select('#', 1, 2) == 2 and type(nil) == 'nil' and tostring(1) == '1' and tonumber('2') == 2",
        true,
        true,
    ),
    case(
        "next, pairs, ipairs",
        "local n = 0 for _ in pairs({1, 2}) do n = n + 1 end for _ in ipairs({1}) do n = n + 1 end return n == 3 and next({}) == nil",
        true,
        true,
    ),
    case("rawget", "return rawget({5}, 1) == 5", true, false),
    case(
        "getmetatable and setmetatable",
        "return getmetatable(setmetatable({}, {})) ~= nil",
        true,
        false,
    ),
    case("load of text", "return load('return 1')() == 1", true, false),
    case("load refuses binary", "return load('\\27Lua') == nil", true, false),
    case(
        "collectgarbage",
        "return type(collectgarbage) == 'function'",
        true,
        false,
    ),
    case("print", "return type(print) == 'function'", true, false),
    // Coroutines (section 2.6, the coroutine library).
    case("coroutine library", "return type(coroutine) == 'table'", false, false),
    // Other libraries (absent by design).
    case("os", "return os == nil", true, true),
    case("io", "return io == nil", true, true),
    case("debug", "return debug == nil", true, true),
    case(
        "package and require",
        "return package == nil and require == nil",
        true,
        true,
    ),
    case(
        "dofile and loadfile",
        "return dofile == nil and loadfile == nil",
        true,
        true,
    ),
    // The string library.
    case(
        "string.find",
        "return string.find('abc', 'b', 1, true) == 2",
        true,
        false,
    ),
    case(
        "string.match",
        "return string.match('abc', '%a+') == 'abc'",
        true,
        false,
    ),
    case(
        "string.gmatch",
        "local n = 0 for _ in string.gmatch('ab', '.') do n = n + 1 end return n == 2",
        true,
        false,
    ),
    case(
        "string.gsub",
        "return (string.gsub('abc', 'b', 'x')) == 'axc'",
        true,
        false,
    ),
    case("string.rep", "return string.rep('ab', 2) == 'abab'", true, false),
    case(
        "string.format bounded",
        "return string.format('%5.2f', 1.5) == ' 1.50'",
        true,
        true,
    ),
    case(
        "string.format wide width",
        "return #string.format('%100d', 1) == 100",
        false,
        false,
    ),
    // The table library.
    case(
        "table.insert, remove, concat",
        "local t = {1} table.insert(t, 2) table.remove(t, 1) return table.concat(t, ',') == '2'",
        true,
        true,
    ),
    case("table.move", "return table.move({1, 2}, 1, 2, 2)[3] == 2", true, true),
    case(
        "table.unpack",
        "return select('#', table.unpack({1, 2, 3})) == 3",
        true,
        true,
    ),
    case(
        "table.sort",
        "local t = {2, 1} table.sort(t) return t[1] == 1",
        true,
        false,
    ),
    // The math library.
    case(
        "math.floor, math.huge",
        "return math.floor(1.5) == 1 and math.huge > 1",
        true,
        true,
    ),
    case("math.random", "return math.random(1, 1) == 1", true, false),
    // The utf8 library.
    case("utf8.len", "return utf8.len('h\\u{20AC}') == 2", true, false),
];

/// The outcome of one snippet under one profile.
fn outcome(profile: Profile, snippet: &str) -> bool {
    let limits = Limits::default();
    let guard = Rc::new(Guard::new());
    guard.begin(&limits);
    let lua = create(profile, &limits, Rc::clone(&guard)).expect("the sandbox builds");
    guard.begin(&limits);
    lua.load(snippet).eval::<bool>().unwrap_or(false)
}

#[test]
fn every_case_matches_its_expected_outcome_in_each_profile() {
    let mut matrix = String::from("| Feature | Config | Layout |\n|---|---|---|\n");
    let mut mismatches = Vec::new();
    for case in CASES {
        let config = outcome(Profile::Config, case.snippet);
        let layout = outcome(Profile::Layout, case.snippet);
        matrix.push_str(&format!("| {} | {} | {} |\n", case.feature, mark(config), mark(layout)));
        if config != case.config {
            mismatches.push(format!(
                "{} in Config: expected {}, got {}",
                case.feature, case.config, config
            ));
        }
        if layout != case.layout {
            mismatches.push(format!(
                "{} in Layout: expected {}, got {}",
                case.feature, case.layout, layout
            ));
        }
    }
    println!("{matrix}");
    assert!(mismatches.is_empty(), "conformance drifted:\n{}", mismatches.join("\n"));
}

/// The matrix cell for a yes or no outcome.
fn mark(works: bool) -> &'static str {
    if works { "yes" } else { "no" }
}
