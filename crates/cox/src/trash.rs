//! The system's trash, where a project the human deletes goes, its
//! folder whole, for them to take back from there as they take back any
//! file: the Finder's Trash on macOS, and the trash of the freedesktop.org
//! specification elsewhere, which the desktop's file managers share.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The trash a run keeps in place of the system's, as a test's.
static KEPT_IN: OnceLock<PathBuf> = OnceLock::new();

/// Has this run put what it trashes in `trash`, laid out as the
/// specification has it, in place of the system's trash, as a test that
/// leaves the human's trash be; once set, it stays.
pub fn keep_in(trash: PathBuf) {
    let _ = KEPT_IN.set(trash);
}

/// Moves `path` to the trash.
///
/// # Errors
///
/// When it cannot, saying why, as when the trash is on another disk.
pub fn put(path: &Path) -> Result<(), String> {
    if let Some(trash) = KEPT_IN.get() {
        return put_in(trash, path);
    }
    #[cfg(target_os = "macos")]
    {
        put_on_macos(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let trash = home_trash().ok_or("Cox cannot tell where the trash is.")?;
        put_in(&trash, path)
    }
}

/// Moves `path` to the Trash through the system's `trash`, which macOS
/// has since 15, or else through the Finder, which asks the human once to
/// let Cox have it do so.
#[cfg(target_os = "macos")]
fn put_on_macos(path: &Path) -> Result<(), String> {
    use std::process::Command;
    // `trash` takes `--` for a file too, so the path goes whole, from `/`,
    // which no option starts with.
    let path = &std::path::absolute(path).map_err(|error| error.to_string())?;
    let trash = Path::new("/usr/bin/trash");
    let output = if trash.exists() {
        Command::new(trash).arg(path).output()
    } else {
        let quoted = path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        Command::new("osascript")
            .arg("-e")
            .arg(format!(
                "tell application \"Finder\" to delete (POSIX file \"{quoted}\" as alias)"
            ))
            .output()
    }
    .map_err(|error| format!("The trash cannot be reached: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    Err(said
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("The folder is not moved to the Trash.")
        .to_owned())
}

/// The trash of the human's home, `Trash` in `XDG_DATA_HOME`, as
/// `~/.local/share/Trash`.
#[cfg(not(target_os = "macos"))]
fn home_trash() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|data| data.is_absolute())
        .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".local/share")))?;
    Some(data.join("Trash"))
}

/// Moves `path` to the trash in `trash`, with the file that says where
/// it was and when it went, under a name no file there has yet, as the
/// specification has it.
fn put_in(trash: &Path, path: &Path) -> Result<(), String> {
    use std::io::Write as _;
    let path = std::path::absolute(path).map_err(|error| error.to_string())?;
    let name = path
        .file_name()
        .ok_or("The folder has no name to put in the trash under.")?
        .to_string_lossy()
        .into_owned();
    let (files, info) = (trash.join("files"), trash.join("info"));
    for folder in [&files, &info] {
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("The trash cannot be made: {error}"))?;
    }
    let url = url::Url::from_file_path(&path)
        .map_err(|()| format!("{} is not a path the trash keeps", path.display()))?;
    let said = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        url.path(),
        jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M:%S")
    );
    for number in 1..=u32::MAX {
        let named = if number == 1 {
            name.clone()
        } else {
            format!("{name} {number}")
        };
        let told = info.join(format!("{named}.trashinfo"));
        // Making the file that says where it was claims the name.
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&told)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("The trash cannot be written: {error}")),
        };
        let destination = files.join(&named);
        if destination.exists() {
            let _ = std::fs::remove_file(&told);
            continue;
        }
        let moved = file
            .write_all(said.as_bytes())
            .map_err(|error| format!("The trash cannot be written: {error}"))
            .and_then(|()| {
                std::fs::rename(&path, &destination).map_err(|error| {
                    format!("{} is not moved to the trash: {error}", path.display())
                })
            });
        if moved.is_err() {
            let _ = std::fs::remove_file(&told);
        }
        return moved;
    }
    Err("The trash has no name left for the folder.".to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::put_in;
    use crate::testing::TempDir;

    /// A folder `my shop` with a folder in it, made in `root`.
    fn shop(root: &Path) -> PathBuf {
        let folder = root.join("my shop");
        std::fs::create_dir_all(folder.join("directories")).unwrap();
        folder
    }

    #[test]
    fn a_folder_put_in_the_trash_is_moved_there_whole() {
        let root = TempDir::new();
        let trash = root.join("Trash");
        let folder = shop(&root);

        put_in(&trash, &folder).unwrap();

        assert!(!folder.exists());
        assert!(trash.join("files/my shop/directories").is_dir());
    }

    #[test]
    fn a_folder_of_a_name_the_trash_has_already_is_numbered() {
        let root = TempDir::new();
        let trash = root.join("Trash");
        put_in(&trash, &shop(&root)).unwrap();

        put_in(&trash, &shop(&root)).unwrap();

        assert!(trash.join("files/my shop/directories").is_dir());
        assert!(trash.join("files/my shop 2/directories").is_dir());
    }

    #[test]
    fn a_folder_in_the_trash_says_where_it_was() {
        let root = TempDir::new();
        let trash = root.join("Trash");

        put_in(&trash, &shop(&root)).unwrap();

        let info = std::fs::read_to_string(trash.join("info/my shop.trashinfo")).unwrap();
        let path = format!("Path={}/my%20shop\n", root.display());
        assert!(
            info.starts_with("[Trash Info]\n") && info.contains(&path),
            "{info}"
        );
    }

    #[test]
    fn a_folder_that_is_not_there_is_not_put_in_the_trash() {
        let root = TempDir::new();
        let trash = root.join("Trash");

        assert!(put_in(&trash, &root.join("gone")).is_err());

        assert!(!trash.join("info/gone.trashinfo").exists());
    }
}
