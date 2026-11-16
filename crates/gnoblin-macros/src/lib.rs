//! The attribute macros of the Gnoblin shell SDK.
//!
//! `#[gnoblin::main]` turns an ordinary function into a shell. The function stays as the author wrote it; the macro adds
//! the one symbol a native shell exports, `gnoshell_entry_v1`, and the descriptor behind it. Everything the descriptor
//! points at lives in `gnoblin-core` (its `__private` module), so the macro emits no `unsafe` and no logic of its own.
//!
//! ```ignore
//! use gnoblin_core as gnoblin;
//! use gnoblin_core::Gnoblin;
//!
//! #[gnoblin::main]
//! fn main(g: &Gnoblin) {
//!     g.log("hello from a shell");
//! }
//! ```
//!
//! The function takes `&Gnoblin` and returns `()` or `Result<(), E>` for any `E: Display`. An `Err` fails the load and its
//! text is shown to the user.
//!
//! The macro is written against `proc_macro` alone, with no `syn`, so the SDK's build graph stays small: it reads the
//! function's name and nothing else, and leaves type checking to the compiler.

use proc_macro::{TokenStream, TokenTree};

/// Marks the function that starts a native shell.
///
/// Exactly one function per shell crate carries it. The generated entry point records the function, and the SDK calls it
/// once per generation on the shell thread with the generation's `Gnoblin` handle.
#[proc_macro_attribute]
pub fn main(args: TokenStream, item: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return compile_error("#[gnoblin::main] takes no arguments");
    }
    let name = match function_name(&item) {
        Ok(name) => name,
        Err(message) => return compile_error(message),
    };
    let generated = entry_source(&name);
    let mut output = item;
    match generated.parse::<TokenStream>() {
        Ok(tokens) => output.extend(tokens),
        Err(error) => return compile_error(&format!("#[gnoblin::main] generated invalid code: {error}")),
    }
    output
}

/// The name of the function the attribute is on, or why the attribute does not apply.
fn function_name(item: &TokenStream) -> Result<String, &'static str> {
    let mut tokens = item.clone().into_iter();
    while let Some(token) = tokens.next() {
        let TokenTree::Ident(ident) = token else { continue };
        match ident.to_string().as_str() {
            "async" => return Err("#[gnoblin::main] cannot be async: a shell's entry function runs to completion"),
            "fn" => {
                let Some(TokenTree::Ident(name)) = tokens.next() else {
                    return Err("#[gnoblin::main] must be on a function with a name");
                };
                if matches!(tokens.next(), Some(TokenTree::Punct(punct)) if punct.as_char() == '<') {
                    return Err("#[gnoblin::main] cannot be generic");
                }
                return Ok(name.to_string());
            }
            _ => {}
        }
    }
    Err("#[gnoblin::main] applies to a function: `fn main(g: &Gnoblin) { ... }`")
}

/// The source of the entry point for the function called `name`.
fn entry_source(name: &str) -> String {
    format!(
        r#"
#[doc(hidden)]
const _: () = {{
    fn __gnoblin_user(
        g: &::gnoblin_core::Gnoblin,
    ) -> ::core::result::Result<(), ::std::string::String> {{
        ::gnoblin_core::__private::IntoInit::into_init({name}(g))
    }}

    static __GNOBLIN_ENTRY: ::gnoblin_core::__private::GnoblinShellEntry =
        ::gnoblin_core::__private::entry(::gnoblin_core::__private::GnoblinStr::borrowed(env!("CARGO_PKG_NAME")));

    #[unsafe(no_mangle)]
    pub extern "C" fn gnoshell_entry_v1() -> *const ::gnoblin_core::__private::GnoblinShellEntry {{
        ::gnoblin_core::__private::set_user(__gnoblin_user);
        &__GNOBLIN_ENTRY
    }}
}};
"#
    )
}

/// A `compile_error!` that reports `message` at the attribute.
fn compile_error(message: &str) -> TokenStream {
    format!("::core::compile_error!({message:?});")
        .parse()
        .unwrap_or_default()
}
