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
    case("pcall", "return pcall(error, 'x') == false", true, true),
    case(
        "xpcall",
        "return xpcall(error, function() return 1 end, 'x') == false",
        true,
        true,
    ),
    case("error and assert", "return not pcall(assert, false)", true, true),
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
    case("coroutine library", "return type(coroutine) == 'table'", true, true),
    // The os library keeps its host-clock and pure-time functions. The layout profile
    // refuses the clock reads, so `os.clock` is absent there.
    case("os.clock absent in layout", "return os.clock == nil", false, true),
    case(
        "os host-reaching functions",
        "return os.execute == nil and os.remove == nil and os.rename == nil and os.getenv == nil and os.tmpname == nil and os.exit == nil",
        true,
        true,
    ),
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
        true,
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
    let mut mismatches = Vec::new();
    for case in CASES {
        let config = outcome(Profile::Config, case.snippet);
        let layout = outcome(Profile::Layout, case.snippet);
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
    assert!(mismatches.is_empty(), "conformance drifted:\n{}", mismatches.join("\n"));
}

/// Render the support page from the probe results. The page is generated, never hand
/// edited: the probe is the source, so the docs cannot drift from the VM.
fn render_page() -> String {
    let mut page = String::from(
        "# Lua language and library support\n\n\
         This page is generated from `crates/gnoblin-lua-sandbox/tests/conformance.rs`. \
         Regenerate it with `GNOBLIN_WRITE_LUA_DOCS=1 cargo test -p gnoblin-lua-sandbox \
         --test conformance`.\n\n\
         A yes means the probe snippet returns `true` in that profile. The sandbox removes \
         `io`, `package`, `debug`, `require`, `dofile` and `loadfile` in both profiles, and \
         removes `os.execute`, `os.remove`, `os.rename`, `os.getenv`, `os.tmpname` and \
         `os.exit` in both profiles. This is the only deliberate exception to full Lua: \
         those functions reach the host system, which is the sandbox boundary.\n\n",
    );
    // Rows are padded to a common width so the committed page is already prettier-formatted,
    // which keeps `prettier --check` clean on the docs.
    let mut rows: Vec<[String; 3]> = vec![["Feature".into(), "Config".into(), "Layout".into()]];
    for case in CASES {
        let config = outcome(Profile::Config, case.snippet);
        let layout = outcome(Profile::Layout, case.snippet);
        rows.push([case.feature.into(), mark(config).into(), mark(layout).into()]);
    }
    let mut widths = [0usize; 3];
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    for (index, row) in rows.iter().enumerate() {
        let cells: Vec<String> = row
            .iter()
            .zip(widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        page.push_str(&format!("| {} |\n", cells.join(" | ")));
        if index == 0 {
            let rule: Vec<String> = widths.iter().map(|width| "-".repeat(*width)).collect();
            page.push_str(&format!("| {} |\n", rule.join(" | ")));
        }
    }
    page
}

/// The generated page must match the committed copy. Set `GNOBLIN_WRITE_LUA_DOCS=1` to
/// rewrite the committed page instead.
#[test]
fn support_page_is_current_with_the_probe() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../", "docs/lua-language-support.md");
    let page = render_page();
    if std::env::var_os("GNOBLIN_WRITE_LUA_DOCS").is_some() {
        std::fs::write(path, &page).expect("the support page writes");
    }
    let committed = std::fs::read_to_string(path).unwrap_or_default();
    assert_eq!(committed, page, "docs/lua-language-support.md is stale; regenerate it");
}

/// The matrix cell for a yes or no outcome.
fn mark(works: bool) -> &'static str {
    if works { "yes" } else { "no" }
}
