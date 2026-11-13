//! The commands of the line Vim's `:` opens (see [`super::line`]): what
//! a line typed there says, as Vim reads it, Cox's commands it can name,
//! and what the line can go on to be.

use cox_editor::fuzzy;

use super::keymap::Follow;

/// A line typed after `:`, as Vim reads one: a command's name, maybe cut
/// short, as `q` of `quit`, a `!` after it, and what follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ex<'a> {
    pub name: &'a str,
    pub bang: bool,
    pub args: &'a str,
}

impl<'a> Ex<'a> {
    /// What `line` says, if it names anything. A name is letters, digits,
    /// `_` and `:`, as GPUI's actions' names are too, as
    /// `workspace::NewChat`.
    #[must_use]
    pub fn parse(line: &'a str) -> Option<Self> {
        let line = line.trim_start().trim_start_matches(':').trim_start();
        let end = line
            .find(|letter: char| !(letter.is_alphanumeric() || letter == '_' || letter == ':'))
            .unwrap_or(line.len());
        let (name, rest) = line.split_at(end);
        if name.is_empty() {
            return None;
        }
        let (bang, rest) = match rest.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, rest),
        };
        Some(Self {
            name,
            bang,
            args: rest.trim(),
        })
    }
}

/// What a command of the table does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Does {
    /// Closes the tab the stage shows.
    Quit,
    /// Stops the turn the agent of the chat shown is taking.
    Stop,
    /// Opens a new chat in the project chosen.
    New,
    /// Opens a URL or a file, or searches for other words, sorted as the
    /// menu Vim's `t` opens sorts what is typed there: in the page shown,
    /// or on a tab of its own.
    Open(Follow),
    Settings,
    /// Sets the goal of the chat shown, or with nothing after it opens the
    /// input that sets it.
    Goal,
    /// Opens the input that starts a sketch of the chat shown.
    Sketch,
    Reload,
    Back,
    Forward,
    /// Shows the chat of the project chosen whose name fits.
    Chat,
    /// Chooses the project whose name fits.
    Project,
    /// Lists the commands, with what each does.
    Help,
}

/// What a command takes after its name, which the line can go on to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Takes {
    Nothing,
    /// Words of the human's own.
    Text,
    /// A URL or a file.
    Place,
    /// The name of a chat.
    Chat,
    /// The name of a project.
    Project,
}

/// A command of the table.
#[derive(Debug)]
pub struct Entry {
    pub name: &'static str,
    /// How much of the name is enough, as Vim writes `q[uit]`: the start
    /// of it as long as this or longer names it.
    pub least: usize,
    pub takes: Takes,
    pub does: Does,
    /// What it does, as the list of commands says.
    pub doc: &'static str,
}

impl Entry {
    /// Its name as Vim writes it, with what can be left off in brackets:
    /// `q[uit]`.
    #[must_use]
    pub fn written(&self) -> String {
        let (needed, rest) = self.name.split_at(self.least.min(self.name.len()));
        if rest.is_empty() {
            needed.to_owned()
        } else {
            format!("{needed}[{rest}]")
        }
    }
}

const fn entry(
    name: &'static str,
    least: usize,
    takes: Takes,
    does: Does,
    doc: &'static str,
) -> Entry {
    Entry {
        name,
        least,
        takes,
        does,
        doc,
    }
}

/// Cox's commands, as the line names them: as Vim and Vimperator name
/// them where they have one, the first to fit a short name the one it
/// names.
pub const COMMANDS: &[Entry] = &[
    entry(
        "quit",
        1,
        Takes::Nothing,
        Does::Quit,
        "Close the tab the stage shows",
    ),
    entry(
        "stop",
        2,
        Takes::Nothing,
        Does::Stop,
        "Stop the agent's turn in the chat shown",
    ),
    entry(
        "new",
        3,
        Takes::Nothing,
        Does::New,
        "Open a new chat in the project",
    ),
    entry(
        "open",
        1,
        Takes::Place,
        Does::Open(Follow::Here),
        "Open a URL or search in the page shown, or a file",
    ),
    entry(
        "edit",
        1,
        Takes::Place,
        Does::Open(Follow::Here),
        "Open a file, or a URL or search in the page shown",
    ),
    entry(
        "tabopen",
        1,
        Takes::Place,
        Does::Open(Follow::NewTab),
        "Open a URL, a file or a search on a tab of its own",
    ),
    entry(
        "settings",
        3,
        Takes::Nothing,
        Does::Settings,
        "Open the settings",
    ),
    entry(
        "goal",
        2,
        Takes::Text,
        Does::Goal,
        "Set the goal of the chat shown",
    ),
    entry(
        "sketch",
        2,
        Takes::Nothing,
        Does::Sketch,
        "Start a sketch of the chat shown",
    ),
    entry(
        "reload",
        2,
        Takes::Nothing,
        Does::Reload,
        "Load the page shown again",
    ),
    entry(
        "back",
        2,
        Takes::Nothing,
        Does::Back,
        "Go back in the page shown",
    ),
    entry(
        "forward",
        2,
        Takes::Nothing,
        Does::Forward,
        "Go forward in the page shown",
    ),
    entry(
        "chat",
        2,
        Takes::Chat,
        Does::Chat,
        "Show the chat of the project by its name",
    ),
    entry(
        "project",
        3,
        Takes::Project,
        Does::Project,
        "Choose the project by its name",
    ),
    entry("help", 1, Takes::Nothing, Does::Help, "List the commands"),
];

/// The command of the table `name` names, whole or cut short.
#[must_use]
pub fn command(name: &str) -> Option<&'static Entry> {
    COMMANDS
        .iter()
        .find(|entry| name.len() >= entry.least && entry.name.starts_with(name))
}

/// The names the line can go on to, as the window has them when the line
/// opens.
#[derive(Clone, Debug, Default)]
pub struct Names {
    /// The chats of the project chosen.
    pub chats: Vec<String>,
    pub projects: Vec<String>,
    /// GPUI's actions, by their names, for a name with `::` in it.
    pub actions: Vec<String>,
}

/// What the line can go on to be, in the list over it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fit {
    /// The line, once it goes on to this.
    pub line: String,
    /// What the list shows of it, and beside that what it does.
    pub shown: String,
    pub doc: String,
}

/// The most the list over the line holds.
const MOST: usize = 50;

/// What `line` can go on to, the best first: a command's name, while the
/// name is being typed, or what the command takes after it.
#[must_use]
pub fn fits(line: &str, names: &Names) -> Vec<Fit> {
    let typed = Ex::parse(line);
    let naming = typed.is_none_or(|ex| ex.args.is_empty() && !line.ends_with(char::is_whitespace));
    if naming {
        let name = typed.map_or("", |ex| ex.name);
        return name_fits(name, names);
    }
    let Some(ex) = typed else {
        return Vec::new();
    };
    let listed = match command(ex.name).map(|entry| entry.takes) {
        Some(Takes::Chat) => &names.chats,
        Some(Takes::Project) => &names.projects,
        _ => return Vec::new(),
    };
    fuzzy::find(ex.args, listed, MOST)
        .into_iter()
        .map(|name| Fit {
            line: format!("{} {name}", ex.name),
            shown: name.to_owned(),
            doc: String::new(),
        })
        .collect()
}

/// The commands `name` can go on to, the one it names first, or instead
/// GPUI's actions, when it has `::` in it.
fn name_fits(name: &str, names: &Names) -> Vec<Fit> {
    if name.contains("::") {
        return fuzzy::find(name, &names.actions, MOST)
            .into_iter()
            .map(|action| Fit {
                line: action.to_owned(),
                shown: action.to_owned(),
                doc: String::new(),
            })
            .collect();
    }
    let all: Vec<String> = COMMANDS.iter().map(|entry| entry.name.to_owned()).collect();
    let mut found = fuzzy::find(name, &all, MOST);
    if let Some(wanted) = command(name) {
        found.retain(|found| *found != wanted.name);
        found.insert(0, wanted.name);
    }
    found
        .into_iter()
        .filter_map(|found| COMMANDS.iter().find(|entry| entry.name == found))
        .map(|entry| Fit {
            line: entry.name.to_owned(),
            shown: entry.written(),
            doc: entry.doc.to_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Does, Ex, Names, command, fits};
    use crate::vim::keymap::Follow;

    #[test]
    fn a_line_is_a_name_a_bang_and_what_follows() {
        let ex = |name, bang, args| Some(Ex { name, bang, args });
        assert_eq!(Ex::parse("q"), ex("q", false, ""));
        assert_eq!(Ex::parse(":q!"), ex("q", true, ""));
        assert_eq!(
            Ex::parse("  open  example.com "),
            ex("open", false, "example.com")
        );
        assert_eq!(Ex::parse("goal ship it!"), ex("goal", false, "ship it!"));
        assert_eq!(
            Ex::parse("workspace::NewChat"),
            ex("workspace::NewChat", false, "")
        );
        assert_eq!(Ex::parse(""), None);
        assert_eq!(Ex::parse("  "), None);
        assert_eq!(Ex::parse("!x"), None);
    }

    #[test]
    fn a_name_cut_short_names_its_command_as_in_vim() {
        let does = |name| command(name).map(|entry| entry.does);
        assert_eq!(does("q"), Some(Does::Quit));
        assert_eq!(does("quit"), Some(Does::Quit));
        assert_eq!(does("st"), Some(Does::Stop));
        assert_eq!(does("o"), Some(Does::Open(Follow::Here)));
        assert_eq!(does("e"), Some(Does::Open(Follow::Here)));
        assert_eq!(does("t"), Some(Does::Open(Follow::NewTab)));
        assert_eq!(does("tabo"), Some(Does::Open(Follow::NewTab)));
        assert_eq!(does("set"), Some(Does::Settings));
        assert_eq!(does("re"), Some(Does::Reload));
        assert_eq!(does("ba"), Some(Does::Back));
        assert_eq!(does("pro"), Some(Does::Project));
        assert_eq!(does("h"), Some(Does::Help));
        for unknown in ["s", "ne", "quitter", "x", "pr"] {
            assert_eq!(does(unknown), None, "{unknown}");
        }
    }

    #[test]
    fn every_command_has_a_name_of_its_own() {
        for entry in super::COMMANDS {
            assert_eq!(
                command(entry.name).map(|named| named.name),
                Some(entry.name)
            );
            assert_eq!(
                command(&entry.name[..entry.least]).map(|named| named.name),
                Some(entry.name),
                "{}",
                entry.written()
            );
            assert_ne!(entry.doc, "");
        }
        assert_eq!(command("q").unwrap().written(), "q[uit]");
        assert_eq!(command("new").unwrap().written(), "new");
    }

    #[test]
    fn the_list_narrows_to_the_names_that_fit_the_one_named_first() {
        let names = Names::default();
        let lines =
            |line| -> Vec<String> { fits(line, &names).into_iter().map(|fit| fit.line).collect() };
        assert_eq!(lines("").len(), super::COMMANDS.len());
        assert_eq!(lines("q"), ["quit"]);
        assert_eq!(lines("tab"), ["tabopen"]);
        assert_eq!(lines("st")[0], "stop");
        assert_eq!(lines("se")[0], "settings");
        assert_eq!(lines("zz"), Vec::<String>::new());
        assert_eq!(fits("q", &names)[0].shown, "q[uit]");
    }

    #[test]
    fn a_chat_or_a_project_goes_on_to_their_names() {
        let names = Names {
            chats: vec!["Fix the parser".into(), "Write docs".into()],
            projects: vec!["cox".into(), "site".into()],
            actions: vec!["workspace::NewChat".into(), "stage::NewPage".into()],
        };
        let lines =
            |line| -> Vec<String> { fits(line, &names).into_iter().map(|fit| fit.line).collect() };
        assert_eq!(lines("chat wd"), ["chat Write docs"]);
        assert_eq!(lines("ch ").len(), 2);
        assert_eq!(lines("pro si"), ["pro site"]);
        assert_eq!(lines("open exa"), Vec::<String>::new());
        assert_eq!(lines("workspace::"), ["workspace::NewChat"]);
    }
}
