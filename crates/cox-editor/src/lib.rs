//! What Cox's editor holds of a file, without drawing it: the text, which
//! the editor reads by lines, and its syntax, which says how to draw it.
//!
//! - [`buffer`] is the text, and the places in it.
//! - [`language`] is the languages the editor knows, and what in their
//!   code is drawn how.
//! - [`syntax`] is the syntax of a text in its language.

pub mod buffer;
pub mod language;
pub mod syntax;
