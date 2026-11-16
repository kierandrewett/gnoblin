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
//! The handle the function receives lives for the whole shell, so the macro reads `g: &Gnoblin` as `g: &'static Gnoblin`: a
//! closure the author hands to `g.bind(...)` or `g.on_client_created(...)` may use `g` without cloning it. That is the one edit
//! the macro makes to the function; everything else is left as written.
//!
//! The macro is written against `proc_macro` alone, with no `syn`, so the SDK's build graph stays small: it reads the
//! function's name and nothing else, and leaves type checking to the compiler.

use proc_macro::{Delimiter, Group, Ident, Punct, Spacing, TokenStream, TokenTree};

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
    let mut output = static_handle(item);
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

/// Reads the first parameter's `&Gnoblin` as `&'static Gnoblin`. A parameter that already names a lifetime, or is another type,
/// is left alone.
fn static_handle(item: TokenStream) -> TokenStream {
    let mut done = false;
    let mut after_fn = false;
    item.into_iter()
        .map(|token| match token {
            TokenTree::Ident(ident) if ident.to_string() == "fn" => {
                after_fn = true;
                TokenTree::Ident(ident)
            }
            TokenTree::Group(group) if after_fn && !done && group.delimiter() == Delimiter::Parenthesis => {
                done = true;
                TokenTree::Group(Group::new(Delimiter::Parenthesis, static_in_params(group.stream())))
            }
            other => other,
        })
        .collect()
}

/// `&Gnoblin` becomes `&'static Gnoblin` (also `&gnoblin_core::Gnoblin`) in the first parameter of `params`.
fn static_in_params(params: TokenStream) -> TokenStream {
    let tokens: Vec<TokenTree> = params.into_iter().collect();
    let mut out: Vec<TokenTree> = Vec::with_capacity(tokens.len() + 2);
    let mut index = 0;
    let mut done = false;
    while index < tokens.len() {
        out.push(tokens[index].clone());
        let is_ref = matches!(&tokens[index], TokenTree::Punct(punct) if punct.as_char() == '&');
        if is_ref && !done {
            // The type after `&`: a path of identifiers. A lifetime or `mut` ends the search.
            let mut end = index + 1;
            let mut last = None;
            while let Some(token) = tokens.get(end) {
                match token {
                    TokenTree::Ident(ident) if ident.to_string() != "mut" => last = Some(ident.to_string()),
                    TokenTree::Punct(punct) if punct.as_char() == ':' => {}
                    _ => break,
                }
                end += 1;
            }
            let is_lifetime = matches!(tokens.get(index + 1), Some(TokenTree::Punct(punct)) if punct.as_char() == '\'');
            if last.as_deref() == Some("Gnoblin") && !is_lifetime {
                let span = tokens[index].span();
                let mut tick = Punct::new('\'', Spacing::Joint);
                tick.set_span(span);
                out.push(TokenTree::Punct(tick));
                out.push(TokenTree::Ident(Ident::new("static", span)));
            }
            done = true;
        }
        index += 1;
    }
    out.into_iter().collect()
}

/// The source of the entry point for the function called `name`.
fn entry_source(name: &str) -> String {
    format!(
        r#"
#[doc(hidden)]
const _: () = {{
    fn __gnoblin_user(
        g: &'static ::gnoblin_core::Gnoblin,
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
