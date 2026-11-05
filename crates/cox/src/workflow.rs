//! What Cox tells a chat's agent in the human's place at the turns of a
//! trial: as it starts, as a trial an agent opened is open, and before
//! it is discarded. Each is the human's to say, since how they work with
//! trials is theirs: a file of their workflow's folder (see
//! [`crate::settings::workflow`]), as `trial/start.md`, or else Cox's own,
//! in `workflow/` beside this file. A file says where Cox puts what it
//! knows then with a name in braces, as `{folder}`, and a name a text
//! does not know stays as it is.

use std::path::Path;

/// A text Cox says at a turn of the human's workflow.
pub struct Text {
    /// Where the human keeps their own, in their workflow's folder.
    name: &'static str,
    /// Cox's own, for a human who keeps none.
    own: &'static str,
}

/// What a chat's agent is told as its chat becomes a trial: in
/// `{folder}`, which is `{from}`, to try `{what}`.
pub const TRIAL_START: Text = Text {
    name: "trial/start.md",
    own: include_str!("workflow/trial/start.md"),
};

/// What the agent that opened a trial with `open_chat` is told of it:
/// that it works in `{folder}`, which is `{from}`.
pub const TRIAL_OPENED: Text = Text {
    name: "trial/opened.md",
    own: include_str!("workflow/trial/opened.md"),
};

/// What a trial's agent is asked before the trial is discarded, with
/// `{left_out}`, the files too big to keep, when there are any.
pub const TRIAL_DISCARD: Text = Text {
    name: "trial/discard.md",
    own: include_str!("workflow/trial/discard.md"),
};

impl Text {
    /// The text, the human's or else Cox's, with `values` in place of the
    /// names they stand for.
    #[must_use]
    pub fn say(&self, values: &[(&str, &str)]) -> String {
        self.say_in(crate::settings::workflow().as_deref(), values)
    }

    /// As [`Self::say`], with the human's workflow in `workflow`.
    #[must_use]
    pub fn say_in(&self, workflow: Option<&Path>, values: &[(&str, &str)]) -> String {
        let human = workflow.and_then(|workflow| {
            let path = workflow.join(self.name);
            match std::fs::read_to_string(&path) {
                Ok(text) if !text.trim().is_empty() => Some(text),
                Ok(_) => None,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    tracing::error!("cannot read {}: {error}", path.display());
                    None
                }
            }
        });
        fill(human.as_deref().unwrap_or(self.own), values)
            .trim_end()
            .to_owned()
    }
}

/// `text` with each `{name}` of `values` in place, in one pass, so that a
/// value that holds a name in braces stays as it is.
fn fill(text: &str, values: &[(&str, &str)]) -> String {
    let mut filled = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        filled.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let value = after.find('}').and_then(|close| {
            let name = &after[..close];
            values
                .iter()
                .find(|(known, _)| *known == name)
                .map(|(_, value)| (close, *value))
        });
        if let Some((close, value)) = value {
            filled.push_str(value);
            rest = &after[close + 1..];
        } else {
            filled.push('{');
            rest = after;
        }
    }
    filled.push_str(rest);
    filled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_is_filled_in_once() {
        assert_eq!(
            fill(
                "in {folder}, {from}: {what} {unknown} {",
                &[
                    ("folder", "/t/{what}"),
                    ("from", "a worktree"),
                    ("what", "x")
                ]
            ),
            "in /t/{what}, a worktree: x {unknown} {"
        );
    }

    #[test]
    fn the_human_s_text_goes_ahead_of_cox_s() {
        let folder = std::env::temp_dir().join(format!("cox-workflow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let values = [
            ("folder", "/t"),
            ("from", "nothing"),
            ("what", "a greeting"),
        ];

        let own = TRIAL_START.say_in(Some(&folder), &values);
        assert!(
            own.starts_with("You work in a trial now, in /t, nothing,"),
            "{own}"
        );
        assert!(
            own.ends_with("Go on with what you try: a greeting"),
            "{own}"
        );

        std::fs::create_dir_all(folder.join("trial")).unwrap();
        std::fs::write(
            folder.join("trial/start.md"),
            "Spike {what} in {folder}.\n\n",
        )
        .unwrap();
        assert_eq!(
            TRIAL_START.say_in(Some(&folder), &values),
            "Spike a greeting in /t."
        );
        // An empty file says nothing of the human's.
        std::fs::write(folder.join("trial/start.md"), "\n").unwrap();
        assert_eq!(TRIAL_START.say_in(Some(&folder), &values), own);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
