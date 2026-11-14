//! The file a tab of code shows: its text, its language and syntax, and
//! what the tab has worked out of them to draw, which all go together.
//! Showing another file is making a new one of these, so that nothing of
//! the file before can stay behind.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use cox_editor::buffer::Buffer;
use cox_editor::language::{Highlight, Language};
use cox_editor::line::LINE_LENGTH;
use cox_editor::syntax::Syntax;

use coxd::storage::Stamp;

/// The stretches of a line, each as bytes of it with what it is, in order.
pub(super) type Spans = Vec<(Range<usize>, Highlight)>;

/// A file, as the tab shows it.
pub(super) struct File {
    /// The file, as an absolute path.
    pub(super) path: PathBuf,
    pub(super) buffer: Buffer,
    /// How many characters the longest line has, which says how far the
    /// tab scrolls across.
    pub(super) longest: usize,
    /// The language of the file, if the editor knows it.
    pub(super) language: Option<&'static Language>,
    /// The syntax of the text, once parsed: until it is parsed again, as
    /// the text changes, it is only edited as the text changed.
    pub(super) syntax: Option<Syntax>,
    /// What the stretches of the lines drawn so far are, by line, as the
    /// syntax says, until the text or the syntax changes: a line coming
    /// into sight is the only one worked out.
    pub(super) highlighted: HashMap<usize, Spans>,
    /// The file's, as the tab last read it, or `None` until the tab looks
    /// at the file.
    pub(super) stamp: Option<Stamp>,
    /// Why the file could not be read the last time the tab looked, if it
    /// could not.
    pub(super) unread: Option<String>,
}

impl File {
    /// The file at `path`, an absolute path, which says `text`, not parsed
    /// yet.
    pub(super) fn new(path: PathBuf, text: &str) -> Self {
        let buffer = Buffer::new(text);
        Self {
            longest: buffer.longest_line(),
            buffer,
            language: Language::of_path(&path),
            syntax: None,
            highlighted: HashMap::new(),
            path,
            stamp: None,
            unread: None,
        }
    }

    /// The file at `path`, an absolute path, as it says now.
    pub(super) fn read(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(Self::new(path.to_owned(), &text))
    }

    /// The name the tab goes by: the file's.
    pub(super) fn title(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    /// The file says `text` now: answers whether that changed anything.
    /// The syntax follows the change until the text is parsed again, and
    /// what was worked out to draw is dropped.
    pub(super) fn set_text(&mut self, text: &str) -> bool {
        let Some(edit) = self.buffer.set_text(text) else {
            return false;
        };
        self.longest = self.buffer.longest_line();
        if let Some(syntax) = &mut self.syntax {
            syntax.edit(&edit);
        }
        self.highlighted.clear();
        true
    }

    /// What the stretches of `line`, from 0, are, as bytes of it, as the
    /// syntax says once parsed.
    pub(super) fn highlights(&self, line: usize) -> Spans {
        self.syntax.as_ref().map_or_else(Vec::new, |syntax| {
            syntax.highlights(&self.buffer, line, LINE_LENGTH)
        })
    }

    /// What the stretches of `line` are, as the last frames worked out, or
    /// else worked out now and added to `highlighted`, for the tab to keep.
    pub(super) fn highlights_at(
        &self,
        line: usize,
        highlighted: &mut Vec<(usize, Spans)>,
    ) -> Spans {
        match (self.highlighted.get(&line), &self.syntax) {
            (Some(highlights), _) => highlights.clone(),
            (None, Some(syntax)) => {
                let highlights = syntax.highlights(&self.buffer, line, LINE_LENGTH);
                highlighted.push((line, highlights.clone()));
                highlights
            }
            (None, None) => Vec::new(),
        }
    }
}
