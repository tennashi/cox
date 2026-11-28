//! Carrying over to the copy an index is made in the trust mise has in
//! the checkout's config. mise trusts a config by where it is, so the
//! copy's, though the same file, is trusted by no one, and a program
//! that runs through one of mise's shims there, as `npx`, fails for it.
//! Where the human trusted the checkout's config, the indexer runs with
//! the copy among `MISE_TRUSTED_CONFIG_PATHS`; where they did not, or
//! there is no mise, as it would have run.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::agents::check;

/// What names the folders whose configs mise trusts, with `:` between
/// them.
const TRUSTED: &str = "MISE_TRUSTED_CONFIG_PATHS";

/// Has `run`, which runs `program` in `copy`, a copy of the code checked
/// out in `checkout`, trust the copy's mise config, if mise trusts the
/// checkout's.
pub fn carry(run: &mut Command, program: &str, checkout: &Path, copy: &Path) {
    let path = std::env::var_os("PATH");
    let Some(mise) = found(program, path.as_deref()) else {
        return;
    };
    if trusts(&mise, checkout, std::env::home_dir().as_deref()) {
        run.env(TRUSTED, with(std::env::var_os(TRUSTED).as_deref(), copy));
    }
}

/// Where mise is: what `program` is a shim of, as mise's shims are links
/// to it, or else on `path`.
fn found(program: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    let root = Path::new("/");
    check::find(program, path, root)
        .and_then(|shim| std::fs::canonicalize(shim).ok())
        .filter(|of| of.file_name() == Some(OsStr::new("mise")))
        .or_else(|| check::find("mise", path, root))
}

/// Whether `mise` trusts the config of `checkout`, as it says itself,
/// with `home` for the `~` it writes.
fn trusts(mise: &Path, checkout: &Path, home: Option<&Path>) -> bool {
    let shown = Command::new(mise)
        .args(["trust", "--show"])
        .current_dir(checkout)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    shown.is_ok_and(|shown| {
        shown.status.success() && trusted(&String::from_utf8_lossy(&shown.stdout), checkout, home)
    })
}

/// Whether `shown`, what `mise trust --show` wrote in `checkout`, says
/// its config is trusted: a line each, as `~/work/app: trusted`, for the
/// folder and those it is in. A folder with no config has no line, and
/// nothing to trust in its copy either.
fn trusted(shown: &str, checkout: &Path, home: Option<&Path>) -> bool {
    let real = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let checkout = real(checkout);
    let mut of_checkout = shown
        .lines()
        .filter_map(|line| {
            let (path, standing) = line.rsplit_once(": ")?;
            let path = match (path.strip_prefix('~'), home) {
                (Some(rest), Some(home)) => home.join(rest.trim_start_matches('/')),
                _ => PathBuf::from(path),
            };
            real(&path)
                .starts_with(&checkout)
                .then_some(standing.trim())
        })
        .peekable();
    of_checkout.peek().is_some() && of_checkout.all(|standing| standing == "trusted")
}

/// The folders mise trusts, `already` and `copy`.
fn with(already: Option<&OsStr>, copy: &Path) -> OsString {
    let mut trusted = already.unwrap_or_default().to_owned();
    if !trusted.is_empty() {
        trusted.push(":");
    }
    trusted.push(copy);
    trusted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn the_copy_is_added_to_the_folders_trusted_already() {
        let copy = Path::new("/tmp/cox-1/index-1/code");
        assert_eq!(with(None, copy), "/tmp/cox-1/index-1/code");
        assert_eq!(with(Some("".as_ref()), copy), "/tmp/cox-1/index-1/code");
        assert_eq!(
            with(Some("/work/a:/work/b".as_ref()), copy),
            "/work/a:/work/b:/tmp/cox-1/index-1/code"
        );
    }

    #[test]
    fn a_checkout_is_trusted_as_mise_says_of_its_own_folder() {
        let home = Some(Path::new("/home/me"));
        let checkout = Path::new("/home/me/work/app");
        let says = |shown: &str| trusted(shown, checkout, home);
        assert!(says("~/work/app: trusted\n"));
        assert!(says("/home/me/work/app: trusted\n"));
        // An older mise names the config, not its folder.
        assert!(says("~/work/app/mise.toml: trusted\n"));
        assert!(!says("~/work/app: untrusted\n"));
        assert!(!says("~/work/app: ignored\n"));
        assert!(!says(
            "~/work/app/mise.toml: trusted\n~/work/app/mise.local.toml: untrusted\n"
        ));
        // A folder the checkout is in is none of the copy's.
        assert!(says("~/work: untrusted\n~/work/app: trusted\n"));
        assert!(!says("~/work: trusted\n~/work/app: untrusted\n"));
        assert!(!says("~/work: trusted\n"));
        assert!(!says("~/work/app-2: trusted\n"));
        // No config, or nothing that reads as mise's.
        assert!(!says(""));
        assert!(!says("mise No trusted config files found.\n"));
        // Without a home, `~` is no folder Cox knows.
        assert!(!trusted("~/work/app: trusted\n", checkout, None));
    }

    /// A fake mise in `folder` that says `standing` of the folder it is
    /// asked in, with `npx` as a shim of it; and the `PATH` they are on.
    fn fake(folder: &Path, standing: &str) -> OsString {
        use std::os::unix::fs::PermissionsExt as _;
        let mise = folder.join("mise");
        let script = format!(
            "#!/bin/sh\n[ \"$1 $2\" = 'trust --show' ] && echo \"$(pwd -P): {standing}\"\n"
        );
        std::fs::write(&mise, script).unwrap();
        let mut permissions = std::fs::metadata(&mise).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&mise, permissions).unwrap();
        let shims = folder.join("shims");
        std::fs::create_dir_all(&shims).unwrap();
        std::os::unix::fs::symlink(&mise, shims.join("npx")).unwrap();
        shims.into_os_string()
    }

    #[test]
    fn mise_is_asked_where_the_shim_leads_or_on_the_path() {
        let folder = TempDir::new();
        let shims = fake(&folder, "trusted");
        let mise = std::fs::canonicalize(folder.join("mise")).unwrap();
        // The shims alone are on the PATH: the shim leads to mise.
        assert_eq!(found("npx", Some(&shims)), Some(mise.clone()));
        // A program that is no shim: mise is found by its name, or not.
        assert_eq!(found("sh", Some("/bin".as_ref())), None);
        let mut both = OsString::from("/bin:");
        both.push(folder.path());
        assert_eq!(found("sh", Some(&both)), Some(folder.join("mise")));
        assert_eq!(found("npx", None), None);

        let checkout = TempDir::new();
        assert!(trusts(&mise, &checkout, None));
        assert!(!trusts(Path::new("/nowhere/mise"), &checkout, None));
    }

    #[test]
    fn a_checkout_mise_does_not_trust_is_not_trusted() {
        let folder = TempDir::new();
        fake(&folder, "untrusted");
        let checkout = TempDir::new();
        assert!(!trusts(&folder.join("mise"), &checkout, None));
    }
}
