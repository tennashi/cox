//! What Cox reads of a project's Git repository, and how it runs Git.
//!
//! - [`git`] runs Git on a folder's repository, asking nothing.
//! - [`changes`] is what changed in its files, as Git tells it.
//! - [`commits`] is the commits that made the changes, each with the turn
//!   of a chat it was made in.
//! - [`tree`] is the files of a branch as Git keeps them, to read them
//!   where no checkout has them on disk.

pub mod changes;
pub mod commits;
pub mod git;
pub mod tree;

#[cfg(test)]
mod scratch;
