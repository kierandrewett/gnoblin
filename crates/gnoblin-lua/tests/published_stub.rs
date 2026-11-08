//! The published `docs/public/gnoblin.d.lua` is the stub the build generates, byte for byte.
//!
//! The file is what a Lua language server reads, and the docs name the members in it, so a hand-edited
//! copy would describe an API the runtime does not have. `cargo xtask docs-parity` reads this file.
//! To regenerate after a schema or host change:
//!
//! ```text
//! GNOBLIN_WRITE_STUB=1 cargo test -p gnoblin-lua --test published_stub
//! ```

use std::fs;
use std::path::PathBuf;

const PUBLISHED: &str = include_str!("../../../docs/public/gnoblin.d.lua");

fn published_path() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/public/gnoblin.d.lua"))
}

#[test]
fn the_published_stub_is_the_generated_stub() {
    let generated = gnoblin_lua::stub();
    if std::env::var_os("GNOBLIN_WRITE_STUB").is_some() {
        fs::write(published_path(), generated).expect("the published stub is written");
        return;
    }
    assert!(
        PUBLISHED == generated,
        "docs/public/gnoblin.d.lua differs from the stub the build generates; regenerate it with \
         GNOBLIN_WRITE_STUB=1 cargo test -p gnoblin-lua --test published_stub"
    );
}
