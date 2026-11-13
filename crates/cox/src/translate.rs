//! Text read in the human's language: the agents write the wiki in
//! English, so that every page is found by the same words, and the human
//! reads it, and whatever else they select in the window, in the language
//! `settings.json` names, as `{"translation": {"language": "Japanese"}}`
//! (see [`crate::settings::Translation`]).
//!
//! What translates is the human's to choose: a command, as
//! `["plamo-translate", "--from", "{from}", "--to", "{to}"]`, which reads
//! the text on its standard input and writes its translation on its
//! standard output, `{from}` and `{to}` standing for the languages in its
//! arguments; or else an agent, over ACP, which the human names among the
//! agents Cox knows, as `"agent": "Codex"`, or the one a new chat talks
//! to: no chat, as the judge of a goal is not, given no tools, and its
//! permission requests refused.
//!
//! Text goes into the human's language when it is in English, and into
//! English when it is not (see [`Way::of`]). A page of the wiki goes into
//! the human's language a block at a time, its paragraphs, headings,
//! lists, quotes and tables, in the order of the page, while its code,
//! the HTML it holds, and its rules stay as they are, so that a model
//! made to translate, and not to keep Markdown, has only prose to
//! translate (see [`page`]); what the human selects goes whole, ahead of
//! the pages' blocks, its translation told as far as it has arrived, as
//! the command or the agent writes it (see [`text`]). Each translation is
//! kept, by what it says, its way and what translated it, in
//! `cache/translations` where Cox keeps what lasts, so
//! that a page that changed is translated again only where it changed,
//! and the wiki's own folder holds only what the agents and the human
//! wrote.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read, Write as _};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use cox_acp::{PermissionAnswer, StopReason};
use pulldown_cmark::{Event, Parser, Tag};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::agent::setup::{Policy, Setup, StageUse};
use crate::agent::{Agent, AgentEvent, Command, Start};
use crate::agents::Agents;
use crate::settings::Translation;
use crate::wiki::told::options;

/// The language the agents write in, and that text not in it goes into.
pub const ENGLISH: &str = "English";

/// How long a translation may take before it is given up, its command
/// stopped or its agent closed, for the next to go on.
const GIVES_UP_AFTER: Duration = Duration::from_mins(5);

/// The translator, once Cox has started it.
static TRANSLATOR: OnceLock<Translator> = OnceLock::new();

/// Which way text is translated: from one language into another, by the
/// names the human gives them, as the command and the agent are told.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Way {
    pub from: String,
    pub to: String,
}

impl Way {
    /// The way the human reads `text`, they reading `language`: into it,
    /// when `text` is in English, as most of its letters are of the Latin
    /// alphabet; or else into English, from `language`.
    #[must_use]
    pub fn of(text: &str, language: &str) -> Self {
        let (latin, other) = text.chars().filter(|c| c.is_alphabetic()).fold(
            (0usize, 0usize),
            |(latin, other), c| {
                if c.is_ascii() {
                    (latin + 1, other)
                } else {
                    (latin, other + 1)
                }
            },
        );
        if latin >= other {
            Self::into(language)
        } else {
            Self {
                from: language.to_owned(),
                to: ENGLISH.to_owned(),
            }
        }
    }

    /// From English into `language`, as the wiki is read.
    #[must_use]
    pub fn into(language: &str) -> Self {
        Self {
            from: ENGLISH.to_owned(),
            to: language.to_owned(),
        }
    }
}

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

/// What arrives of a translation of text the human selected: as much of
/// it as the translator has written, as it writes it, and then what it
/// comes to, the translation or why there is none.
pub type Answer = mpsc::UnboundedReceiver<Arriving>;

/// What arrives of a translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arriving {
    /// The translation as far as it has been written.
    SoFar(String),
    /// The translation whole, or why there is none: the last to arrive.
    Done(Result<String, String>),
}

/// Starts the translator, which works on a thread of its own, its agent,
/// when it needs one, being one of `agents`.
pub fn start(agents: Agents) {
    let Some(folder) = crate::storage::path(crate::storage::Kept::Translations) else {
        tracing::error!("nothing is translated: there is no place to keep translations");
        return;
    };
    let _ = TRANSLATOR.set(Translator::start(folder, Some(agents), GIVES_UP_AFTER));
}

/// The page whose text is `body`, as the human reads it in the language
/// `settings.json` names, as far as it is translated, the rest of it
/// being translated from now on; or `None` when the human reads as it is
/// written.
#[must_use]
pub fn page(body: &str) -> Option<Translated> {
    let translation = crate::settings::translation()?;
    Some(TRANSLATOR.get()?.page(body, &translation))
}

/// Translates `text`, which the human selected, the way they read it, and
/// answers which way, and what it comes to; or `None` when the human
/// names no language to read in.
#[must_use]
pub fn text(text: &str) -> Option<(Way, Answer)> {
    let translation = crate::settings::translation()?;
    let translator = TRANSLATOR.get()?;
    let way = Way::of(text, &translation.language);
    let answer = translator.text(text.trim(), way.clone(), translation);
    Some((way, answer))
}

/// What translates, a text at a time, on a thread of its own.
struct Translator {
    /// Where the translations are kept.
    folder: PathBuf,
    shared: Arc<Shared>,
}

/// What the translator and its thread share.
struct Shared {
    state: Mutex<State>,
    /// Told when a job is added.
    added: Condvar,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("no one panics holding it")
    }
}

/// A text to translate: what it says, which way, how, the file its
/// translation is kept in, and who waits for it, if anyone does.
struct Job {
    text: String,
    way: Way,
    translation: Translation,
    file: PathBuf,
    answer: Option<mpsc::UnboundedSender<Arriving>>,
}

/// What the translator is at.
#[derive(Default)]
struct State {
    /// The texts to translate, in turn.
    jobs: VecDeque<Job>,
    /// The files of the pages' translations on the way.
    pending: HashSet<PathBuf>,
    /// The files of the pages' translations that failed, with how, which
    /// are not tried again until the human says another way.
    failed: HashMap<PathBuf, Translation>,
    /// Why translating a page failed last, and how it was translated then.
    error: Option<(Translation, String)>,
}

impl Translator {
    /// Starts the translator, keeping translations in `folder`, its agent,
    /// when it needs one, being one of `agents`, giving up a translation
    /// that takes longer than `gives_up_after`.
    fn start(folder: PathBuf, agents: Option<Agents>, gives_up_after: Duration) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            added: Condvar::new(),
        });
        let works = Worker {
            agents,
            agent: None,
            cwd: folder.clone(),
            gives_up_after,
            shared: shared.clone(),
        };
        thread::Builder::new()
            .name("translate".to_owned())
            .spawn(move || works.work())
            .expect("a thread starts");
        Self { folder, shared }
    }

    /// The page whose text is `body` in the language of `translation`, as
    /// far as it is translated, the blocks that are not yet given to the
    /// translator.
    fn page(&self, body: &str, translation: &Translation) -> Translated {
        let way = Way::into(&translation.language);
        let mut state = self.shared.state();
        let mut done = 0;
        let mut of = 0;
        let mut added = false;
        let markdown = rebuilt(body, |text| {
            of += 1;
            let file = self.folder.join(key(&way, translation, text));
            if let Ok(translated) = fs::read_to_string(&file) {
                done += 1;
                return Some(translated);
            }
            let failed = state.failed.get(&file) == Some(translation);
            if !failed && state.pending.insert(file.clone()) {
                state.jobs.push_back(Job {
                    text: text.to_owned(),
                    way: way.clone(),
                    translation: translation.clone(),
                    file,
                    answer: None,
                });
                added = true;
            }
            None
        });
        let error = state
            .error
            .as_ref()
            .filter(|(how, _)| how == translation)
            .map(|(_, error)| error.clone());
        drop(state);
        if added {
            self.shared.added.notify_one();
        }
        Translated {
            language: translation.language.clone(),
            markdown,
            done,
            of,
            error,
        }
    }

    /// Translates `text` the way `way` says, as `translation` says, ahead
    /// of the pages' blocks, or answers what was kept of it.
    fn text(&self, text: &str, way: Way, translation: Translation) -> Answer {
        let (answer, answered) = mpsc::unbounded_channel();
        let file = self.folder.join(key(&way, &translation, text));
        if let Ok(translated) = fs::read_to_string(&file) {
            let _ = answer.send(Arriving::Done(Ok(translated)));
            return answered;
        }
        self.shared.state().jobs.push_front(Job {
            text: text.to_owned(),
            way,
            translation,
            file,
            answer: Some(answer),
        });
        self.shared.added.notify_one();
        answered
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
    /// How long a translation may take before it is given up.
    gives_up_after: Duration,
    shared: Arc<Shared>,
}

impl Worker {
    /// Translates each job in turn, keeping what it makes.
    fn work(mut self) {
        let span = tracing::info_span!("translate");
        let _entered = span.enter();
        loop {
            let job = {
                let mut state = self.shared.state();
                loop {
                    if let Some(job) = state.jobs.pop_front() {
                        break job;
                    }
                    state = self
                        .shared
                        .added
                        .wait(state)
                        .expect("no one panics holding it");
                }
            };
            // Only a selection, which a card shows, is told as it arrives.
            let told = job.answer.clone();
            let so_far = |so_far: &str| {
                if let Some(told) = &told {
                    let _ = told.send(Arriving::SoFar(so_far.to_owned()));
                }
            };
            let translated = match &job.translation.command {
                Some(command) => run(command, &job.way, &job.text, self.gives_up_after, so_far),
                None => self.ask(
                    job.translation.agent.as_deref(),
                    &job.way,
                    &job.text,
                    so_far,
                ),
            }
            .and_then(|translated| {
                keep(&job.file, &translated)
                    .map_err(|error| format!("cannot keep the translation: {error}"))?;
                Ok(translated)
            });
            if let Err(error) = &translated {
                tracing::error!("cannot translate: {error}");
            }
            if let Some(answer) = job.answer {
                let _ = answer.send(Arriving::Done(translated));
                continue;
            }
            let mut state = self.shared.state();
            state.pending.remove(&job.file);
            match translated {
                Ok(_) => {
                    if state
                        .error
                        .as_ref()
                        .is_some_and(|(how, _)| *how == job.translation)
                    {
                        state.error = None;
                    }
                }
                Err(error) => {
                    state.failed.insert(job.file, job.translation.clone());
                    state.error = Some((job.translation, error));
                }
            }
        }
    }

    /// What the agent named `named` says `text` is, the way `way` says,
    /// starting it first if it is not running, telling `so_far` what it
    /// has said as it says it.
    fn ask(
        &mut self,
        named: Option<&str>,
        way: &Way,
        text: &str,
        so_far: impl FnMut(&str),
    ) -> Result<String, String> {
        let asked = self.asked(named, way, text, so_far);
        if asked.is_err() {
            // A turn that failed may leave the agent in a state of its own;
            // the next text starts another.
            self.agent = None;
        }
        asked
    }

    fn asked(
        &mut self,
        named: Option<&str>,
        way: &Way,
        text: &str,
        mut so_far: impl FnMut(&str),
    ) -> Result<String, String> {
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
        let until = Instant::now() + self.gives_up_after;
        let waits = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|error| format!("cannot wait for the agent: {error}"))?;
        let (_, agent) = self.agent.as_mut().expect("started above");
        agent
            .commands
            .send(Command::Prompt(prompt(way, text).into()))
            .map_err(|_| "the agent closed".to_owned())?;
        let mut reply = String::new();
        loop {
            let event = waits.block_on(async {
                tokio::time::timeout_at(until.into(), agent.events.recv()).await
            });
            let Ok(event) = event else {
                return Err(gave_up(self.gives_up_after, "the agent did not answer"));
            };
            let Some(event) = event else {
                return Err("the agent closed".to_owned());
            };
            match event {
                AgentEvent::ReplyText(text) => {
                    reply.push_str(&text);
                    so_far(reply.trim());
                }
                AgentEvent::PermissionRequest { .. } => {
                    let _ = agent
                        .commands
                        .send(Command::Answer(PermissionAnswer::Cancel));
                }
                AgentEvent::LoginNeeded { .. } => {
                    return Err(
                        "the agent asks to log in: log in from a chat of it, then try again"
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

/// What the agent is asked, to translate `text` the way `way` says.
fn prompt(way: &Way, text: &str) -> String {
    let Way { from, to } = way;
    format!(
        "Translate this text from {from} into {to}. Answer with the translation alone: no \
         words of your own before or after it, and no code fence around it. Keep any Markdown \
         as it is: links and where they go, the names in [[name]] and [[name|text]] links, of \
         which you translate only the text after the |, inline code, code, and paths. Use no \
         tools.\n\n{text}"
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

/// Why a translation that took longer than `gives_up_after` has none.
fn gave_up(gives_up_after: Duration, what: &str) -> String {
    format!("gave up after {gives_up_after:?}: {what}")
}

/// What `command` writes for `text`, given on its standard input, with
/// `{from}` and `{to}` in its arguments standing for the languages of
/// `way`, telling `so_far` what it has written as it writes it; stopped,
/// with every process it started, when it takes longer than
/// `gives_up_after`.
fn run(
    command: &[String],
    way: &Way,
    text: &str,
    gives_up_after: Duration,
    mut so_far: impl FnMut(&str),
) -> Result<String, String> {
    use std::os::unix::process::CommandExt as _;
    let words: Vec<String> = command
        .iter()
        .map(|word| word.replace("{from}", &way.from).replace("{to}", &way.to))
        .collect();
    let (program, arguments) = words.split_first().ok_or("the command is empty")?;
    let mut child = Process::new(program)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|error| format!("{program} does not start: {error}"))?;
    let group = libc::pid_t::try_from(child.id()).map_err(|error| error.to_string())?;
    let (finished, finishing) = std::sync::mpsc::channel::<()>();
    // Its whole group, as what it started holds its output open too.
    let watches = thread::spawn(move || {
        let late = finishing.recv_timeout(gives_up_after)
            == Err(std::sync::mpsc::RecvTimeoutError::Timeout);
        if late {
            // SAFETY: this only signals the command's group, whose id can
            // be another's only once the command is waited for, a moment
            // before this is told it finished.
            unsafe { libc::killpg(group, libc::SIGKILL) };
        }
        late
    });
    let mut stdin = child.stdin.take().ok_or("the command takes no input")?;
    let input = format!("{}\n", text.trim_end());
    // Written apart from the reading, so that a command that writes as it
    // reads does not wait on Cox while Cox waits on it.
    let writes = thread::spawn(move || stdin.write_all(input.as_bytes()));
    // Read apart from what it writes, so that neither fills while the
    // other is read.
    let mut stderr = child.stderr.take().ok_or("the command says nothing")?;
    let complains = thread::spawn(move || {
        let mut said = Vec::new();
        let _ = stderr.read_to_end(&mut said);
        said
    });
    let stdout = child.stdout.take().ok_or("the command writes nothing")?;
    let written = as_written(stdout, &mut so_far)
        .map_err(|error| format!("{program} cannot be read: {error}"))?;
    let status = child
        .wait()
        .map_err(|error| format!("{program} cannot be waited for: {error}"))?;
    drop(finished);
    let late = watches.join().unwrap_or_default();
    let _ = writes.join();
    let said = complains.join().unwrap_or_default();
    if late {
        return Err(gave_up(
            gives_up_after,
            &format!("{program} did not finish"),
        ));
    }
    if !status.success() {
        let said = String::from_utf8_lossy(&said);
        let said = said.trim().lines().last().unwrap_or_default();
        return Err(format!("{program} failed ({status}): {said}"));
    }
    let translated = String::from_utf8_lossy(&written).trim().to_owned();
    if translated.is_empty() {
        return Err(format!("{program} wrote nothing"));
    }
    Ok(translated)
}

/// All `from` writes, telling `so_far` what it has written as it writes
/// it, up to its last whole character.
///
/// A read a signal interrupts is read again, as `read_to_end` does: the
/// window's process, where CEF runs too, may handle a signal without
/// having the system restart what it interrupted.
fn as_written(mut from: impl Read, mut so_far: impl FnMut(&str)) -> io::Result<Vec<u8>> {
    let mut written = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let read = match from.read(&mut chunk) {
            Ok(0) => return Ok(written),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        written.extend_from_slice(&chunk[..read]);
        // Up to the last whole character, as a chunk may end inside one.
        let whole = match std::str::from_utf8(&written) {
            Ok(whole) => whole,
            Err(error) => std::str::from_utf8(&written[..error.valid_up_to()]).unwrap_or_default(),
        };
        if !whole.trim().is_empty() {
            so_far(whole.trim());
        }
    }
}

/// Keeps `translated` in `file`, whole or not at all, as a page may be
/// read at any time.
fn keep(file: &Path, translated: &str) -> std::io::Result<()> {
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }
    let part = file.with_extension("part");
    fs::write(&part, translated)?;
    fs::rename(&part, file)
}

/// The name of the file the translation of `text` the way `way` says, by
/// what `translation` translates with, is kept in: another command or
/// agent translates it again.
fn key(way: &Way, translation: &Translation, text: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(way.from.trim().to_lowercase().as_bytes());
    hash.update(b"\n");
    hash.update(way.to.trim().to_lowercase().as_bytes());
    hash.update(b"\n");
    match (&translation.command, &translation.agent) {
        (Some(command), _) => {
            hash.update(b"command");
            for word in command {
                hash.update(b"\0");
                hash.update(word.as_bytes());
            }
        }
        (None, Some(agent)) => {
            hash.update(b"agent\0");
            hash.update(agent.as_bytes());
        }
        (None, None) => hash.update(b"agent"),
    }
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
    use super::*;
    use crate::testing::TempDir;

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
    fn a_translation_is_kept_by_what_it_says_its_way_and_its_translator() {
        let japanese = Way::into("Japanese");
        let agent = Translation {
            language: "Japanese".to_owned(),
            command: None,
            agent: None,
        };
        let key = |way: &Way, text: &str| key(way, &agent, text);
        assert_eq!(key(&japanese, "a"), key(&Way::into("japanese"), "a"));
        assert_ne!(key(&japanese, "a"), key(&japanese, "b"));
        assert_ne!(key(&japanese, "a"), key(&Way::into("French"), "a"));
        assert_ne!(key(&japanese, "a"), key(&Way::of("あ", "Japanese"), "a"));
        let codex = Translation {
            agent: Some("Codex".to_owned()),
            ..agent.clone()
        };
        let plamo = Translation {
            command: Some(vec!["plamo-translate".to_owned()]),
            ..agent.clone()
        };
        let cat = Translation {
            command: Some(vec!["cat".to_owned()]),
            ..agent.clone()
        };
        let keys = [&agent, &codex, &plamo, &cat].map(|how| super::key(&japanese, how, "a"));
        for (at, one) in keys.iter().enumerate() {
            assert!(!keys[at + 1..].contains(one), "{keys:?}");
        }
    }

    #[test]
    fn english_goes_into_the_humans_language_and_the_rest_into_english() {
        assert_eq!(Way::of("Hello, world.", "Japanese"), Way::into("Japanese"));
        assert_eq!(
            Way::of("`div` を縮める", "Japanese"),
            Way {
                from: "Japanese".to_owned(),
                to: ENGLISH.to_owned(),
            }
        );
        // Code and numbers alone go the way English does.
        assert_eq!(Way::of("42", "Japanese"), Way::into("Japanese"));
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

    #[test]
    fn a_read_a_signal_interrupts_is_read_again() {
        /// Says `parts` one at a time, each after a read a signal
        /// interrupts.
        struct Interrupted {
            parts: VecDeque<&'static [u8]>,
            interrupt: bool,
        }
        impl Read for Interrupted {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                self.interrupt = !self.interrupt;
                if self.interrupt {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                let Some(part) = self.parts.pop_front() else {
                    return Ok(0);
                };
                buffer[..part.len()].copy_from_slice(part);
                Ok(part.len())
            }
        }
        let from = Interrupted {
            parts: VecDeque::from(["こん".as_bytes(), "にちは".as_bytes()]),
            interrupt: false,
        };
        let mut told = Vec::new();
        let written = as_written(from, |so_far| told.push(so_far.to_owned())).unwrap();
        assert_eq!(String::from_utf8(written).unwrap(), "こんにちは");
        assert_eq!(told, ["こん", "こんにちは"]);
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
        let folder = TempDir::new();
        let translator = Translator::start(folder.to_path_buf(), None, GIVES_UP_AFTER);
        let shouting = Translation {
            language: "Shouting".to_owned(),
            command: Some(vec![
                "sh".to_owned(),
                "-c".to_owned(),
                "printf '%s to %s: ' \"$0\" \"$1\"; tr a-z A-Z".to_owned(),
                "{from}".to_owned(),
                "{to}".to_owned(),
            ]),
            agent: None,
        };
        let page = translated(&translator, "# Hello\n\nA [[link]].\n", &shouting);
        assert_eq!(
            page,
            Translated {
                language: "Shouting".to_owned(),
                markdown: "# English to Shouting: HELLO\n\nEnglish to Shouting: A [[LINK]].\n"
                    .to_owned(),
                done: 2,
                of: 2,
                error: None,
            }
        );
        // Kept, it is not asked again: a block that changed is.
        let changed = translator.page("# Hello\n\nAnother.\n", &shouting);
        assert_eq!((changed.done, changed.of), (1, 2));
        let changed = translated(&translator, "# Hello\n\nAnother.\n", &shouting);
        assert_eq!((changed.done, changed.of), (2, 2));
    }

    #[test]
    fn a_command_that_fails_says_why_until_another_is_named() {
        let folder = TempDir::new();
        let runs = folder.join("runs");
        let translator = Translator::start(folder.to_path_buf(), None, GIVES_UP_AFTER);
        let failing = Translation {
            language: "Japanese".to_owned(),
            command: Some(vec![
                "sh".to_owned(),
                "-c".to_owned(),
                format!(
                    "echo ran >> '{}'; echo no model >&2; exit 3",
                    runs.display()
                ),
            ]),
            agent: None,
        };
        let page = translated(&translator, "Hello.\n", &failing);
        assert_eq!(page.markdown, "Hello.\n");
        assert_eq!(page.done, 0);
        let error = page.error.unwrap();
        assert!(error.starts_with("sh failed"), "{error}");
        assert!(error.ends_with("no model"), "{error}");
        let _ = translator.page("Hello.\n", &failing);
        let working = Translation {
            command: Some(vec!["cat".to_owned()]),
            ..failing
        };
        let page = translated(&translator, "Hello.\n", &working);
        assert_eq!((page.done, page.error), (1, None));
        assert_eq!(
            fs::read_to_string(&runs).unwrap(),
            "ran\n",
            "the failing command is not run again, though asked before the working one"
        );
    }

    #[test]
    fn a_selection_is_translated_whole_and_kept() {
        let folder = TempDir::new();
        let translator = Translator::start(folder.to_path_buf(), None, GIVES_UP_AFTER);
        let marked = Translation {
            language: "Japanese".to_owned(),
            command: Some(vec![
                "sh".to_owned(),
                "-c".to_owned(),
                "printf '%s>%s ' \"$0\" \"$1\"; cat".to_owned(),
                "{from}".to_owned(),
                "{to}".to_owned(),
            ]),
            agent: None,
        };
        let text = "縮める。\n\n二つ目。";
        let way = Way::of(text, &marked.language);
        let answer = translator.text(text, way.clone(), marked.clone());
        assert_eq!(
            done(answer),
            Ok("Japanese>English 縮める。\n\n二つ目。".to_owned())
        );
        // Kept, it is answered at once.
        let mut again = translator.text(text, way, marked);
        assert_eq!(
            again.try_recv(),
            Ok(Arriving::Done(Ok(
                "Japanese>English 縮める。\n\n二つ目。".to_owned()
            )))
        );
    }

    /// What `answer` comes to, or `None` when it has not come to anything
    /// within `time`.
    fn done_within(answer: Answer, time: Duration) -> Option<Result<String, String>> {
        let (sender, came) = std::sync::mpsc::channel();
        thread::spawn(move || sender.send(done(answer)));
        came.recv_timeout(time).ok()
    }

    #[test]
    fn a_command_that_does_not_finish_is_given_up_and_the_next_translates() {
        let folder = TempDir::new();
        let translator = Translator::start(folder.to_path_buf(), None, Duration::from_secs(1));
        let stuck_with_a_child = Translation {
            language: "Japanese".to_owned(),
            command: Some(vec![
                "sh".to_owned(),
                "-c".to_owned(),
                "cat >/dev/null; sleep 60 & wait".to_owned(),
            ]),
            agent: None,
        };
        let way = Way::into("Japanese");
        let answer = translator.text("Shrink.", way.clone(), stuck_with_a_child.clone());
        let error = done_within(answer, Duration::from_secs(10))
            .expect("the translation is given up")
            .unwrap_err();
        assert!(error.starts_with("gave up"), "{error}");
        let mut not_kept = translator.text("Shrink.", way.clone(), stuck_with_a_child);
        assert!(not_kept.try_recv().is_err());
        let quick = Translation {
            command: Some(vec!["tr".to_owned(), "a-z".to_owned(), "A-Z".to_owned()]),
            language: "Japanese".to_owned(),
            agent: None,
        };
        let answer = translator.text("Grow.", way, quick);
        assert_eq!(
            done_within(answer, Duration::from_secs(20)),
            Some(Ok("GROW.".to_owned()))
        );
    }

    #[test]
    fn an_agent_that_does_not_answer_is_given_up() {
        let folder = TempDir::new();
        let silent = Agents::new(
            vec![
                "sh".to_owned(),
                "-c".to_owned(),
                "cat >/dev/null".to_owned(),
            ],
            Vec::new(),
            Vec::new(),
        );
        let translator =
            Translator::start(folder.to_path_buf(), Some(silent), Duration::from_secs(1));
        let asked = Translation {
            language: "Japanese".to_owned(),
            command: None,
            agent: None,
        };
        let answer = translator.text("Shrink.", Way::into("Japanese"), asked);
        let error = done_within(answer, Duration::from_secs(10))
            .expect("the translation is given up")
            .unwrap_err();
        assert!(error.starts_with("gave up"), "{error}");
    }

    /// What `answer` comes to once it is done.
    fn done(mut answer: Answer) -> Result<String, String> {
        loop {
            match answer.blocking_recv() {
                Some(Arriving::Done(translated)) => return translated,
                Some(Arriving::SoFar(_)) => {}
                None => panic!("the translator stopped"),
            }
        }
    }

    #[test]
    fn a_selection_is_told_as_far_as_it_has_arrived() {
        let folder = TempDir::new();
        let go_on = folder.join("go-on");
        let translator = Translator::start(folder.to_path_buf(), None, GIVES_UP_AFTER);
        let slow = Translation {
            language: "Japanese".to_owned(),
            command: Some(vec![
                "sh".to_owned(),
                "-c".to_owned(),
                format!(
                    "cat >/dev/null; printf '縮'; i=0; \
                     while [ ! -e '{}' ] && [ $i -lt 1000 ]; do sleep 0.01; i=$((i+1)); done; \
                     printf 'める'",
                    go_on.display()
                ),
            ]),
            agent: None,
        };
        let way = Way::into("Japanese");
        let mut answer = translator.text("Shrink.", way, slow);
        assert_eq!(
            answer.blocking_recv(),
            Some(Arriving::SoFar("縮".to_owned()))
        );
        fs::write(&go_on, "").unwrap();
        assert_eq!(
            answer.blocking_recv(),
            Some(Arriving::SoFar("縮める".to_owned()))
        );
        assert_eq!(
            answer.blocking_recv(),
            Some(Arriving::Done(Ok("縮める".to_owned())))
        );
    }
}
