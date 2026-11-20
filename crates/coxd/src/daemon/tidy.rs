//! The worktrees Cox made for its chats (see [`crate::worktrees`]), taken
//! away once the chat each was made for is done, and it holds nothing
//! that is not elsewhere: Cox looks as a chat is marked done, and now and
//! then, as a pull request merges after its chat was done, and tells the
//! chat of each it took away. A chat that worked in one, sent a line
//! again, has it made again before its agent starts.

use std::path::Path;
use std::time::{Duration, Instant};

use super::{Daemon, Internal};
use crate::worktrees::{self, Busy, Removed};

/// How long after Cox last looked which worktrees it may take away it
/// looks again by itself.
const TIDY_EVERY: Duration = Duration::from_mins(2);

/// Whether Cox looks which worktrees it may take away, and when it last
/// did.
#[derive(Default)]
pub(super) struct Tidying {
    /// Whether it looks now.
    now: bool,
    /// Whether it is to look again once it is done, a chat having been
    /// marked done while it looked.
    again: bool,
    /// When it last started to.
    last: Option<Instant>,
}

impl Daemon {
    /// Looks which worktrees Cox may take away, in the background, if it
    /// is time to, at `now`, or `soon`, as a chat was marked done.
    pub(super) fn tidy_worktrees(&mut self, now: Instant, soon: bool) {
        let tidying = &mut self.tidying;
        if tidying.now {
            tidying.again |= soon;
            return;
        }
        if !soon && tidying.last.is_some_and(|last| now < last + TIDY_EVERY) {
            return;
        }
        tidying.now = true;
        tidying.last = Some(now);
        let projects: Vec<_> = self
            .projects
            .iter()
            .skip(1)
            .map(|project| project.folder.clone())
            .filter(|folder| folder.is_dir())
            .collect();
        let busy = self.busy();
        let internal = self.internal.clone();
        // Git takes a while over a big repository: on a thread of its own.
        std::thread::spawn(move || {
            let removed = projects
                .iter()
                .flat_map(|project| worktrees::tidy(project, &busy))
                .collect();
            let _ = internal.send(Internal::Tidied { removed });
        });
    }

    /// Whether Cox looks which worktrees it may take away now.
    #[cfg(test)]
    pub(super) fn tidying_now(&self) -> bool {
        self.tidying.now
    }

    /// What the chats not done hold on to: those chats, and the folders
    /// they work in.
    fn busy(&self) -> Busy {
        let open = self.chats.iter().filter(|slot| slot.resolved.is_none());
        Busy {
            chats: open
                .clone()
                .filter_map(|slot| slot.kept.map(cox_core::store::ChatId::number))
                .collect(),
            folders: open.map(|slot| self.cwd(slot)).collect(),
        }
    }

    /// Tells each chat of the worktrees made for it that Cox took away,
    /// as `removed` says, and looks again if a chat was marked done since
    /// it looked.
    pub(super) fn tidied(&mut self, removed: Vec<Removed>) {
        for Removed { chat, said } in removed {
            tracing::info!(chat, "{said}");
            let key = self
                .chats
                .iter()
                .chain(&self.archived)
                .find(|slot| slot.kept.is_some_and(|kept| kept.number() == chat))
                .map(|slot| slot.key);
            if let Some(key) = key
                && let Some(conversation) = self.chat(key)
            {
                conversation.note(format!(
                    "{said} This chat is done, and it held nothing that is not elsewhere."
                ));
            }
        }
        self.tidying.now = false;
        if std::mem::take(&mut self.tidying.again) {
            self.tidy_worktrees(Instant::now(), true);
        }
    }

    /// Makes again the worktree the closed chat `key` would start its
    /// agent again in, if its folder is gone, as Cox took it away (see
    /// [`worktrees::remake`]), and tells the chat it did.
    ///
    /// # Errors
    ///
    /// When Cox cannot make it again, with why, as the chat is told.
    pub(super) fn remake_worktree(&mut self, key: u64) -> Result<(), String> {
        let Some(cwd) = self
            .chat(key)
            .and_then(|chat| chat.again_cwd())
            .map(Path::to_owned)
        else {
            return Ok(());
        };
        let remade = worktrees::remake(&cwd);
        let Some(chat) = self.chat(key) else {
            return Ok(());
        };
        match remade {
            Ok(None) => Ok(()),
            Ok(Some(said)) => {
                tracing::info!(path = %cwd.display(), "the worktree is made again");
                chat.note(said);
                Ok(())
            }
            Err(why) => {
                tracing::warn!(path = %cwd.display(), "the worktree is not made again: {why}");
                chat.note(format!(
                    "The worktree this chat works in, at {}, is gone, and Cox cannot make it \
                     again, so its agent does not start: {why}.",
                    cwd.display()
                ));
                Err(why)
            }
        }
    }
}
