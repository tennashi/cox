//! What Cox's editor holds of a file, without drawing it: the text, which
//! the editor reads by lines, its syntax, which says how to draw it, and
//! what its language's server says of it; and the files of the project,
//! to open one.
//!
//! - [`buffer`] is the text, and the places and words in it.
//! - [`line`] is a line of it as it is shown.
//! - [`history`] is where a tab has been, to go back and forward.
//! - [`files`] lists what is in a project's folders.
//! - [`changes`] is what changed in them, as Git tells it.
//! - [`language`] is the languages the editor knows, and what in their
//!   code is drawn how.
//! - [`syntax`] is the syntax of a text in its language.
//! - [`lsp`] talks to a language's server, which knows what the code
//!   means: what a name is, and where it is defined and used.

pub mod buffer;
pub mod changes;
pub mod files;
pub mod history;
pub mod language;
pub mod line;
pub mod lsp;
pub mod syntax;
