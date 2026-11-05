//! The wiki read in the human's language: the agents write its pages in
//! English, so that every page is found by the same words, and the Wiki
//! plugin shows a page in the language `settings.json` names, as
//! `{"wiki_translation": {"language": "Japanese"}}` (see
//! [`crate::settings::Translation`]), translated as the human opens it.
//!
//! What translates is the human's to choose: a command, as
//! `["plamo-translate", "--from", "English", "--to", "{language}"]`,
//! which reads a stretch of the page on its standard input and writes
//! its translation on its standard output, `{language}` standing for the
//! language in its arguments; or else an agent, over ACP, which the
//! human names among the agents Cox knows, as `"agent": "Codex"`, or the
//! one a new chat talks to: no chat, as the judge of a goal is not, given
//! no tools, and its permission requests refused.
//!
//! A page is translated a block at a time, its paragraphs, headings,
//! lists, quotes and tables, in the order of the page, while its code,
//! the HTML it holds, and its rules stay as they are, so that a model
//! made to translate, and not to keep Markdown, has only prose to
//! translate. Each block's translation is kept, by what it says and the
//! language, in `wiki-translations` where Cox keeps what lasts, so that a
//! page that changed is translated again only where it changed, and the
//! wiki's own folder holds only what the agents and the human wrote.
//! Until a block is translated, the page shows it as it is written.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use cox_acp::{PermissionAnswer, StopReason};
use pulldown_cmark::{Event, Parser, Tag};
use sha2::{Digest, Sha256};

use super::told::options;
use crate::agent::setup::{Policy, Setup, StageUse};
use crate::agent::{Agent, AgentEvent, Command, Start};
use crate::agents::Agents;
use crate::settings::Translation;

/// Where Cox keeps the translations, where it keeps what lasts.
const FOLDER: &str = "wiki-translations";

/// The translator of the wiki, once Cox has started it.
static TRANSLATOR: OnceLock<Translator> = OnceLock::new();

/// A page as the human reads it in their language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translated {
    /// The language, as the human names it.
    pub language: String,
    /// The page's text, each block translated in place once it is.
    pub markdown: String,
    /// How many of its blocks are translated.
    pub done: usize,
    /// How many of its blocks are to be translated.
    pub of: usize,
    /// Why translating failed last, if it did, as the translator said.
    pub error: Option<String>,
}

/// Starts the translator of the wiki, which works on a thread of its
/// own, its agent, when it needs one, being one of `agents`.
pub fn start(agents: Agents) {
    let Some(folder) = crate::storage::path(FOLDER) else {
        tracing::error!("the wiki is not translated: there is no place to keep translations");
        return;
    };
    let _ = TRANSLATOR.set(Translator::start(folder, Some(agents)));
}

/// The page whose text is `body`, as the human reads it in the language
/// `settings.json` names, as far as it is translated, the rest of it
/// being translated from now on; or `None` when the human reads the wiki
/// as it is written.
#[must_use]
pub fn page(body: &str) -> Option<Translated> {
    let translation = crate::settings::wiki_translation()?;
    Some(TRANSLATOR.get()?.page(body, &translation))
}

/// What translates the wiki, a block at a time, on a thread of its own.
struct Translator {
    /// Where the translations are kept.
    folder: PathBuf,
    jobs: Sender<Job>,
    state: Arc<Mutex<State>>,
}

/// A block to translate: what it says, how, and the file its translation
/// is kept in.
struct Job {
    text: String,
    translation: Translation,
    file: PathBuf,
}

/// What the translator is at.
#[derive(Default)]
struct State {
    /// The files of the translations on the way.
    pending: HashSet<PathBuf>,
    /// The files of the translations that failed, with how, which are
    /// not tried again until the human says another way.
    failed: HashMap<PathBuf, Translation>,
    /// Why translating failed last, and how it was translated then.
    error: Option<(Translation, String)>,
}

impl Translator {
    /// Starts the translator, keeping translations in `folder`, its agent,
    /// when it needs one, being one of `agents`.
    fn start(folder: PathBuf, agents: Option<Agents>) -> Self {
        let (jobs, received) = channel();
        let state = Arc::new(Mutex::new(State::default()));
        let works = Worker {
            agents,
            agent: None,
            cwd: folder.clone(),
            state: state.clone(),
        };
        thread::Builder::new()
            .name("wiki-translate".to_owned())
            .spawn(move || works.work(&received))
            .expect("a thread starts");
        Self {
            folder,
            jobs,
            state,
        }
    }

    /// The page whose text is `body` in the language of `translation`, as
    /// far as it is translated, the blocks that are not yet given to the
    /// translator.
    fn page(&self, body: &str, translation: &Translation) -> Translated {
        let mut state = self.state.lock().expect("no one panics holding it");
        let mut done = 0;
        let mut of = 0;
        let markdown = rebuilt(body, |text| {
            of += 1;
            let file = self.folder.join(key(&translation.language, text));
            if let Ok(translated) = fs::read_to_string(&file) {
                done += 1;
                return Some(translated);
            }
            let failed = state.failed.get(&file) == Some(translation);
            if !failed && state.pending.insert(file.clone()) {
                let _ = self.jobs.send(Job {
                    text: text.to_owned(),
                    translation: translation.clone(),
                    file,
                });
            }
            None
        });
        let error = state
            .error
            .as_ref()
            .filter(|(how, _)| how == translation)
            .map(|(_, error)| error.clone());
        Translated {
            language: translation.language.clone(),
            markdown,
            done,
            of,
            error,
        }
    }
}

/// The translator's thread: what it translates with.
struct Worker {
    agents: Option<Agents>,
    /// The agent that translates, once one has started, until it closes
    /// or the human names another, with the name it was named by.
    agent: Option<(Option<String>, Agent)>,
    /// Where the agent works.
    cwd: PathBuf,
    state: Arc<Mutex<State>>,
}

impl Worker {
    /// Translates each job of `jobs` in turn, keeping what it makes.
    fn work(mut self, jobs: &Receiver<Job>) {
        let span = tracing::info_span!("wiki_translate");
        let _entered = span.enter();
        for job in jobs {
            let language = &job.translation.language;
            let translated = match &job.translation.command {
                Some(command) => run(command, language, &job.text),
                None => self.ask(job.translation.agent.as_deref(), language, &job.text),
            }
            .and_then(|translated| {
                keep(&job.file, &translated)
                    .map_err(|error| format!("cannot keep the translation: {error}"))
            });
            let mut state = self.state.lock().expect("no one panics holding it");
            state.pending.remove(&job.file);
            match translated {
                Ok(()) => {
                    if state
                        .error
                        .as_ref()
                        .is_some_and(|(how, _)| *how == job.translation)
                    {
                        state.error = None;
                    }
                }
                Err(error) => {
                    tracing::error!("the wiki cannot be translated: {error}");
                    state.failed.insert(job.file, job.translation.clone());
                    state.error = Some((job.translation, error));
                }
            }
        }
    }

    /// What the agent named `named` says `text` is in `language`, starting
    /// it first if it is not running.
    fn ask(&mut self, named: Option<&str>, language: &str, text: &str) -> Result<String, String> {
        let asked = self.asked(named, language, text);
        if asked.is_err() {
            // A turn that failed may leave the agent in a state of its own;
            // the next block starts another.
            self.agent = None;
        }
        asked
    }

    fn asked(&mut self, named: Option<&str>, language: &str, text: &str) -> Result<String, String> {
        if self
            .agent
            .as_ref()
            .is_some_and(|(was, _)| was.as_deref() != named)
        {
            self.agent = None;
        }
        if self.agent.is_none() {
            let agents = self
                .agents
                .as_ref()
                .ok_or("there is no agent to translate with")?;
            let known = match named {
                Some(named) => agents
                    .all()
                    .into_iter()
                    .find(|known| known.name.eq_ignore_ascii_case(named) || known.id == named)
                    .ok_or_else(|| format!("Cox knows no agent named {named}"))?,
                None => agents.of(None),
            };
            let start = Start::New {
                instructions: None,
                setup: Some(setup()),
            };
            let agent = crate::chat::launcher::spawn(known, self.cwd.clone(), Vec::new(), start);
            self.agent = Some((named.map(str::to_owned), agent));
        }
        let (_, agent) = self.agent.as_mut().expect("started above");
        agent
            .commands
            .send(Command::Prompt(prompt(language, text)))
            .map_err(|_| "the agent closed".to_owned())?;
        let mut reply = String::new();
        loop {
            let Some(event) = agent.events.blocking_recv() else {
                return Err("the agent closed".to_owned());
            };
            match event {
                AgentEvent::ReplyText(text) => reply.push_str(&text),
                AgentEvent::PermissionRequest { .. } => {
                    let _ = agent
                        .commands
                        .send(Command::Answer(PermissionAnswer::Cancel));
                }
                AgentEvent::LoginNeeded { .. } => {
                    return Err(
                        "the agent asks to log in: log in from a chat of it, then open the page \
                         again"
                            .to_owned(),
                    );
                }
                AgentEvent::TurnEnded(StopReason::EndTurn) => {
                    return Ok(unfenced(&reply, text));
                }
                AgentEvent::TurnEnded(reason) => {
                    return Err(format!("the agent's turn ended early: {reason:?}"));
                }
                AgentEvent::TurnFailed(error) => {
                    return Err(format!("the agent's turn failed: {error}"));
                }
                AgentEvent::Closed(error) => {
                    return Err(error.map_or_else(
                        || "the agent closed".to_owned(),
                        |error| format!("the agent closed: {error}"),
                    ));
                }
                _ => {}
            }
        }
    }
}

/// How the agent that translates is set up: with no tools of Cox's own
/// and none for the stage, every permission request it makes refused.
fn setup() -> Setup {
    Setup {
        context: None,
        tools: Some(Vec::new()),
        stage: StageUse::Look,
        policy: Policy {
            allow: Vec::new(),
            reject: vec!["*".to_owned()],
            within: None,
        },
        folder: None,
        sketch: None,
    }
}

/// What the agent is asked, to translate `text` into `language`.
fn prompt(language: &str, text: &str) -> String {
    format!(
        "Translate this Markdown, a block of a page of a wiki, into {language}. Answer with the \
         translation alone, as Markdown: no words of your own before or after it, and no code \
         fence around it. Keep the Markdown as it is: links and where they go, the names in \
         [[name]] and [[name|text]] links, of which you translate only the text after the |, \
         inline code, and paths. Use no tools.\n\n{text}"
    )
}

/// `reply`, the translation of `text`, without the code fence a model
/// may put around it, when `text` is not code itself.
fn unfenced(reply: &str, text: &str) -> String {
    let reply = reply.trim();
    if text.trim_start().starts_with("```") || !reply.starts_with("```") || !reply.ends_with("```")
    {
        return reply.to_owned();
    }
    let inside = reply.trim_end_matches('`');
    match inside.split_once('\n') {
        Some((_, inside)) => inside.trim().to_owned(),
        None => reply.to_owned(),
    }
}

/// What `command` writes for `text`, given on its standard input, with
/// `{language}` in its arguments standing for `language`.
fn run(command: &[String], language: &str, text: &str) -> Result<String, String> {
    let words: Vec<String> = command
        .iter()
        .map(|word| word.replace("{language}", language))
        .collect();
    let (program, arguments) = words.split_first().ok_or("the command is empty")?;
    let mut child = Process::new(program)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{program} does not start: {error}"))?;
    let mut stdin = child.stdin.take().ok_or("the command takes no input")?;
    let input = format!("{}\n", text.trim_end());
    // Written apart from the reading, so that a command that writes as it
    // reads does not wait on Cox while Cox waits on it.
    let writes = thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = child
        .wait_with_output()
        .map_err(|error| format!("{program} cannot be read: {error}"))?;
    let _ = writes.join();
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        let said = said.trim().lines().last().unwrap_or_default();
        return Err(format!("{program} failed ({}): {said}", output.status));
    }
    let translated = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if translated.is_empty() {
        return Err(format!("{program} wrote nothing"));
    }
    Ok(translated)
}

/// Keeps `translated` in `file`, whole or not at all, as the page may be
/// read at any time.
fn keep(file: &Path, translated: &str) -> std::io::Result<()> {
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }
    let part = file.with_extension("part");
    fs::write(&part, translated)?;
    fs::rename(&part, file)
}

/// The name of the file the translation of `text` into `language` is
/// kept in.
fn key(language: &str, text: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(language.trim().to_lowercase().as_bytes());
    hash.update(b"\n");
    hash.update(text.as_bytes());
    let mut name = String::new();
    for byte in hash.finalize() {
        let _ = write!(name, "{byte:02x}");
    }
    name.push_str(".md");
    name
}

/// A block of a page to translate: where it is in the page, what goes
/// ahead of its translation, as a heading's `## `, and what it says.
struct Block {
    range: Range<usize>,
    prefix: String,
    text: String,
}

/// The blocks of `body` to translate, in order: its paragraphs, headings,
/// lists, quotes and tables, and not its code, the HTML it holds, or its
/// rules, which stay as they are.
fn blocks(body: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut depth = 0usize;
    for (event, range) in Parser::new_ext(body, options()).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    let source = &body[range.clone()];
                    let block = match tag {
                        Tag::Heading { level, .. } => {
                            let text = if source.starts_with('#') {
                                source
                                    .trim()
                                    .trim_start_matches('#')
                                    .trim_end_matches('#')
                                    .trim()
                                    .to_owned()
                            } else {
                                // Underlined, its last line being the line.
                                let mut lines: Vec<&str> = source.trim_end().lines().collect();
                                lines.pop();
                                lines.join("\n").trim().to_owned()
                            };
                            Some(("#".repeat(level as usize) + " ", text))
                        }
                        Tag::Paragraph | Tag::List(_) | Tag::BlockQuote(_) | Tag::Table(_) => {
                            Some((String::new(), source.trim().to_owned()))
                        }
                        _ => None,
                    };
                    if let Some((prefix, text)) = block
                        && !text.is_empty()
                    {
                        blocks.push(Block {
                            range: range.clone(),
                            prefix,
                            text,
                        });
                    }
                }
                depth += 1;
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    blocks
}

/// `body` with each block that `translated` has a translation of in its
/// place, keeping the space that follows it.
fn rebuilt(body: &str, mut translated: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(body.len());
    let mut at = 0;
    for block in blocks(body) {
        let Some(translation) = translated(&block.text) else {
            continue;
        };
        let source = &body[block.range.clone()];
        let trailing = &source[source.trim_end().len()..];
        out.push_str(&body[at..block.range.start]);
        out.push_str(&block.prefix);
        out.push_str(translation.trim());
        out.push_str(trailing);
        at = block.range.end;
    }
    out.push_str(&body[at..]);
    out
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    const PAGE: &str = "# GPUI layout\n\nHow [[taffy]] lays out `div`s.\n\n\
        Underlined\n----------\n\n- one\n- two\n\n```rust\nlet x = 1;\n```\n\n\
        <div>kept</div>\n\n---\n\n> Quoted.\n";

    #[test]
    fn a_page_is_translated_by_its_prose_and_not_its_code() {
        let texts: Vec<(String, String)> = blocks(PAGE)
            .into_iter()
            .map(|block| (block.prefix, block.text))
            .collect();
        assert_eq!(
            texts,
            [
                ("# ", "GPUI layout"),
                ("", "How [[taffy]] lays out `div`s."),
                ("## ", "Underlined"),
                ("", "- one\n- two"),
                ("", "> Quoted."),
            ]
            .map(|(prefix, text)| (prefix.to_owned(), text.to_owned()))
        );
    }

    #[test]
    fn a_block_not_yet_translated_stays_as_it_is() {
        let upper = rebuilt(PAGE, |text| {
            (!text.starts_with('-')).then(|| text.to_uppercase())
        });
        assert_eq!(
            upper,
            "# GPUI LAYOUT\n\nHOW [[TAFFY]] LAYS OUT `DIV`S.\n\n## UNDERLINED\n\n- one\n- two\n\n\
             ```rust\nlet x = 1;\n```\n\n<div>kept</div>\n\n---\n\n> QUOTED.\n"
        );
        assert_eq!(rebuilt(PAGE, |_| None), PAGE);
    }

    #[test]
    fn a_translation_is_kept_by_what_it_says_and_the_language() {
        assert_eq!(key("Japanese", "a"), key("japanese", "a"));
        assert_ne!(key("Japanese", "a"), key("Japanese", "b"));
        assert_ne!(key("Japanese", "a"), key("French", "a"));
    }

    #[test]
    fn a_fence_around_a_reply_goes() {
        assert_eq!(
            unfenced("```markdown\nこんにちは\n```", "Hello"),
            "こんにちは"
        );
        assert_eq!(unfenced(" こんにちは \n", "Hello"), "こんにちは");
        assert_eq!(unfenced("```\nx\n```", "```\nx\n```"), "```\nx\n```");
    }

    fn scratch(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("cox-wiki-translate-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        folder
    }

    /// The page as `translator` translates it once it has all of it, or
    /// as it is when it fails.
    fn translated(translator: &Translator, body: &str, translation: &Translation) -> Translated {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let page = translator.page(body, translation);
            if page.done == page.of || page.error.is_some() || Instant::now() > until {
                return page;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_command_translates_the_page_a_block_at_a_time() {
        let folder = scratch("command");
        let translator = Translator::start(folder.clone(), None);
        let shouting = Translation {
            language: "Shouting".to_owned(),
            command: Some(vec![
                "sh".to_owned(),
                "-c".to_owned(),
                "printf '%s: ' \"$0\"; tr a-z A-Z".to_owned(),
                "{language}".to_owned(),
            ]),
            agent: None,
        };
        let page = translated(&translator, "# Hello\n\nA [[link]].\n", &shouting);
        assert_eq!(
            page,
            Translated {
                language: "Shouting".to_owned(),
                markdown: "# Shouting: HELLO\n\nShouting: A [[LINK]].\n".to_owned(),
                done: 2,
                of: 2,
                error: None,
            }
        );
        // Kept, it is not asked again: a block that changed is.
        let changed = translator.page("# Hello\n\nAnother.\n", &shouting);
        assert_eq!((changed.done, changed.of), (1, 2));
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_command_that_fails_says_why_until_another_is_named() {
        let folder = scratch("fails");
        let translator = Translator::start(folder.clone(), None);
        let failing = Translation {
            language: "Japanese".to_owned(),
            command: Some(vec![
                "sh".to_owned(),
                "-c".to_owned(),
                "echo no model >&2; exit 3".to_owned(),
            ]),
            agent: None,
        };
        let page = translated(&translator, "Hello.\n", &failing);
        assert_eq!(page.markdown, "Hello.\n");
        assert_eq!(page.done, 0);
        let error = page.error.unwrap();
        assert!(error.starts_with("sh failed"), "{error}");
        assert!(error.ends_with("no model"), "{error}");
        // It is not tried again the same way, but another way is.
        assert!(translator.state.lock().unwrap().pending.is_empty());
        let _ = translator.page("Hello.\n", &failing);
        assert!(translator.state.lock().unwrap().pending.is_empty());
        let working = Translation {
            command: Some(vec!["cat".to_owned()]),
            ..failing
        };
        let page = translated(&translator, "Hello.\n", &working);
        assert_eq!((page.done, page.error), (1, None));
        let _ = fs::remove_dir_all(&folder);
    }
}
