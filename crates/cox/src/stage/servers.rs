//! The language servers that the tabs showing code ask what the code
//! means (see [`cox_editor::lsp`]): one for each project and server,
//! started when a tab first needs it, and shared by the tabs of every chat
//! in the project, until Cox quits. Which server knows a file is the
//! project's to say, over the human's `servers.json`, where Cox keeps what
//! lasts, which the settings screen changes (see [`listing`]), over what
//! Cox knows (see [`cox_editor::lsp::servers`]). The app holds them as a
//! global, which the stages find them in.

pub mod listing;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;

use cox_editor::lsp::servers::Got;
use cox_editor::lsp::{self, Client, Error, Server, Status, languages};
use gpui::{App, AppContext as _, Context, Entity, Global, Task};

use crate::agents::download;
use crate::settings::DIRECTORIES;
use crate::storage::Kept;

/// What starts a server in a project's folder, and answers it once it is
/// ready.
pub type Starter =
    Rc<dyn Fn(Server, PathBuf) -> Pin<Box<dyn Future<Output = Result<Client, Error>> + Send>>>;

/// The language servers of every project.
#[derive(Clone)]
pub struct Servers {
    running: Rc<RefCell<Running>>,
    start: Starter,
}

/// The servers started, by the folder each works in, and their names.
type Running = HashMap<(PathBuf, String), Entity<LanguageServer>>;

impl Global for Servers {}

impl Default for Servers {
    /// Servers started as programs: got as each says, for a server that
    /// says where from on this platform, or else found on `PATH`.
    fn default() -> Self {
        Self::new(Rc::new(|server, root| {
            Box::pin(async move {
                let server = downloaded(server)?;
                Client::start(&server, &root).await
            })
        }))
    }
}

/// `server`, to start from where Cox gets its program, in
/// `cache/servers/<name>/<version>` where Cox keeps what lasts, when it
/// says where Cox gets it from on this platform: got first if it is not
/// there yet, downloaded from GitHub, or installed with `go install`.
fn downloaded(mut server: Server) -> Result<Server, Error> {
    let told = |note: String| tracing::info!(server = %server.name, "{note}");
    let got = match server.got_on(&download::platform()) {
        None => return Ok(server),
        Some(Got::GitHub {
            repository,
            version,
            asset,
            program,
        }) => download::installed_at(Kept::ServerDownloads, &server.name, version)
            .map_or_else(
                || {
                    let from = download::from_github(repository, version, &asset)?;
                    download::installed(Kept::ServerDownloads, &server.name, &from, &told)
                },
                Ok,
            )
            .map(|folder| folder.join(program)),
        Some(Got::Go {
            package,
            version,
            program,
        }) => download::go_installed(Kept::ServerDownloads, &server.name, package, version, &told)
            .map(|folder| folder.join(program)),
    };
    let program = got.map_err(|error| Error::Io(std::io::Error::other(format!("{error:#}"))))?;
    if let Some(command) = server.command.first_mut() {
        *command = program.to_string_lossy().into_owned();
    }
    Ok(server)
}

impl Servers {
    /// Servers that `start` starts.
    #[must_use]
    pub fn new(start: Starter) -> Self {
        Self {
            running: Rc::default(),
            start,
        }
    }

    /// The server that knows the code of the file at `path`, an absolute
    /// path, for a chat working in `cwd`: started if it was not, if the
    /// project, or Cox, knows a server for the file's language. Started
    /// once, it goes on as it was started until Cox quits, whatever the
    /// project says of it since.
    ///
    /// The server works in the folder [`root_of`] says, so that each of a
    /// project's directories gets one of its own.
    pub fn of(&self, cwd: &Path, path: &Path, cx: &mut App) -> Option<Entity<LanguageServer>> {
        let language = languages::id_of(path)?;
        let user = crate::storage::path(Kept::Servers);
        let (known, left_out) = lsp::Servers::of_project(user.as_deref(), cwd);
        for why in left_out {
            tracing::warn!("{why}");
        }
        let server = known.of(language)?.clone();
        let root = root_of(cwd, path, &server.markers)?;
        let key = (root, server.name.clone());
        if let Some(running) = self.running.borrow().get(&key) {
            return Some(running.clone());
        }
        let started = (self.start)(server.clone(), key.0.clone());
        let running = cx.new(|cx| LanguageServer::new(server, started, cx));
        self.running.borrow_mut().insert(key, running.clone());
        Some(running)
    }
}

/// The folder a server of a language works in, for the file at `path`,
/// an absolute path, of a chat working in `cwd`: the outermost folder
/// above the file that is a project of the language, as one with one of
/// `markers` is, as a `Cargo.toml` is for Rust, but no further out than
/// `cwd` when the file is in it. When none is, it is the directory of the
/// project the file is in (see [`DIRECTORIES`]), as
/// `<cwd>/directories/cox`, or else `cwd`, for a file in it, and the
/// file's own folder for one that is not.
fn root_of(cwd: &Path, path: &Path, markers: &[String]) -> Option<PathBuf> {
    let folder = path.parent()?;
    let inside = path.starts_with(cwd);
    let marked = folder
        .ancestors()
        .take_while(|folder| !inside || folder.starts_with(cwd))
        .filter(|folder| markers.iter().any(|marker| folder.join(marker).exists()))
        .last();
    if let Some(marked) = marked {
        return Some(marked.to_owned());
    }
    if !inside {
        return Some(folder.to_owned());
    }
    let directories = cwd.join(DIRECTORIES);
    let directory = path
        .strip_prefix(&directories)
        .ok()
        .and_then(|rest| rest.components().next())
        .filter(|_| folder.starts_with(&directories) && folder != directories)
        .map(|name| directories.join(name));
    Some(directory.unwrap_or_else(|| cwd.to_owned()))
}

/// Where the program of a server is, as the settings screen says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// Downloaded, at this version.
    Downloaded(String),
    /// Downloaded at this version the first time a tab needs it.
    ToDownload(String),
    /// Here: on `PATH`, or where the command says.
    At(PathBuf),
    /// Nowhere: the program the command names is not on `PATH`, or not
    /// where it says.
    Missing(String),
}

impl Found {
    /// Where the program of `server` is, on `PATH` as Cox has it.
    #[must_use]
    pub fn of(server: &Server) -> Self {
        if let Some(got) = server.got_on(&download::platform()) {
            let version = got.version();
            return if download::is_installed(Kept::ServerDownloads, &server.name, version) {
                Self::Downloaded(version.to_owned())
            } else {
                Self::ToDownload(version.to_owned())
            };
        }
        let program = server.command.first().cloned().unwrap_or_default();
        let here = std::env::current_dir().unwrap_or_default();
        crate::agents::check::find(&program, std::env::var_os("PATH").as_deref(), &here)
            .map_or(Self::Missing(program), Self::At)
    }
}

/// A project's language server, as the tabs see it.
pub struct LanguageServer {
    server: Server,
    state: State,
    /// What it says it is doing, once it runs.
    status: Status,
    _task: Task<()>,
}

enum State {
    Starting,
    Running(Client),
    /// It could not be started, for the reason given.
    Failed(String),
}

impl LanguageServer {
    /// `server`, which `started` answers once it is ready.
    fn new(
        server: Server,
        started: Pin<Box<dyn Future<Output = Result<Client, Error>> + Send>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let task = cx.spawn(async move |this, cx| {
            let started = cx.background_executor().spawn(started).await;
            let status = started.as_ref().ok().map(Client::status);
            if this
                .update(cx, |this, cx| this.started(started, cx))
                .is_err()
            {
                return;
            }
            let Some(mut status) = status else {
                return;
            };
            // The status changes as the server says, until it stops.
            loop {
                let now = status.borrow_and_update().clone();
                let stopped = now == Status::Stopped;
                if this
                    .update(cx, |this, cx| {
                        this.status = now;
                        cx.notify();
                    })
                    .is_err()
                    || stopped
                    || status.changed().await.is_err()
                {
                    break;
                }
            }
        });
        Self {
            server,
            state: State::Starting,
            status: Status::Ready,
            _task: task,
        }
    }

    fn started(&mut self, started: Result<Client, Error>, cx: &mut Context<Self>) {
        self.state = match started {
            Ok(client) => State::Running(client),
            Err(error) => {
                tracing::warn!("{} did not start: {error}", self.server.name);
                State::Failed(error.to_string())
            }
        };
        cx.notify();
    }

    /// What the server goes by.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.server.name
    }

    /// The language of the file at `path`, as the protocol names it, when
    /// the server knows it.
    #[must_use]
    pub fn language_of(&self, path: &Path) -> Option<&'static str> {
        languages::id_of(path).filter(|language| self.server.knows(language))
    }

    /// The server, once it is ready to be asked, until it stops.
    #[must_use]
    pub fn client(&self) -> Option<&Client> {
        match &self.state {
            State::Running(client) if self.status != Status::Stopped => Some(client),
            State::Running(_) | State::Starting | State::Failed(_) => None,
        }
    }

    /// What the tabs say of the server, if anything: nothing while it is
    /// ready.
    #[must_use]
    pub fn said(&self) -> Option<String> {
        let name = &self.server.name;
        match (&self.state, &self.status) {
            (State::Starting, _) => Some(format!("{name}: starting")),
            (State::Failed(error), _) => Some(format!("{name} did not start: {error}")),
            (State::Running(_), Status::Ready) => None,
            (State::Running(_), Status::Working(work)) => Some(format!("{name}: {work}")),
            (State::Running(_), Status::Stopped) => Some(format!("{name} stopped")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn a_server_works_in_the_directory_of_the_code_it_reads() {
        let scratch = std::env::temp_dir().join(format!("cox-servers-root-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        let project = scratch.join("projects").join("cox");
        let cox = project.join(DIRECTORIES).join("cox");
        let notes = project.join(DIRECTORIES).join("notes");
        fs::create_dir_all(cox.join("crates/cox-editor/src")).unwrap();
        fs::create_dir_all(notes.join("tools")).unwrap();
        fs::write(cox.join("Cargo.toml"), "").unwrap();
        fs::write(cox.join("crates/cox-editor/Cargo.toml"), "").unwrap();
        let markers = ["Cargo.toml".to_owned()];
        let root = |path: &Path| root_of(&project, path, &markers).unwrap();

        // The outermost folder of the language, as a Cargo workspace.
        assert_eq!(root(&cox.join("crates/cox-editor/src/lsp.rs")), cox);
        // A directory with no marker is the server's all the same.
        assert_eq!(root(&notes.join("tools/fmt.rs")), notes);
        // What is in the project but not in a directory goes in the project.
        assert_eq!(root(&project.join("scratch.rs")), project);
        // No further out than the project, even when a folder above it is
        // marked.
        fs::write(scratch.join("Cargo.toml"), "").unwrap();
        assert_eq!(root(&notes.join("tools/fmt.rs")), notes);
        // A file out of the project goes to its own outermost folder.
        let elsewhere = scratch.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        assert_eq!(root(&elsewhere.join("main.rs")), scratch);
        let _ = fs::remove_dir_all(&scratch);
    }
}
