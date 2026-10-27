//! What Cox's editor holds of a file, without drawing it: the text, which
//! the editor reads by lines, its syntax, which says how to draw it, and
//! what its language's server says of it.
//!
//! - [`buffer`] is the text, and the places in it.
//! - [`language`] is the languages the editor knows, and what in their
//!   code is drawn how.
//! - [`syntax`] is the syntax of a text in its language.
//! - [`lsp`] talks to a language's server, which knows what the code
//!   means: what a name is, and where it is defined and used.

pub mod buffer;
pub mod language;
pub mod lsp;
pub mod syntax;
