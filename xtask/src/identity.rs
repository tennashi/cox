//! The certificate Cox is signed with on macOS: one made by Cox itself and
//! trusted by no one else, named [`NAME`]. macOS keeps what the human
//! allowed an app, such as recording the screen, for the certificate the
//! app is signed with, so every build signed with the same one keeps it.
//!
//! `identity new` makes the certificate and writes it, with its key, into
//! a folder to keep; `identity import` puts a kept one in another Mac's
//! login keychain.

use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{Context, bail};
use xshell::{Shell, cmd};

/// The certificate's name, as its common name and as Keychain Access shows it.
pub const NAME: &str = "Cox";

/// What a folder from `identity new` holds: the certificate and its key, and
/// the password that opens them.
const IDENTITY: &str = "identity.p12";
const PASSWORD: &str = "password";

/// The system's `LibreSSL`: it writes a PKCS#12 file the keychain can read,
/// which OpenSSL 3, as Homebrew installs it, does not by default.
const OPENSSL: &str = "/usr/bin/openssl";

/// Runs `identity new <folder>` or `identity import <folder>`.
pub fn run(sh: &Shell, args: &[String]) -> anyhow::Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("identity is for signing the macOS app; run it on a Mac");
    }
    match args {
        [task, folder] if task == "new" => new(sh, Path::new(folder)),
        [task, folder] if task == "import" => import(sh, Path::new(folder)),
        _ => bail!("usage: cargo xtask identity new <folder> | identity import <folder>"),
    }
}

/// Makes the certificate into `folder`, which must not exist yet, and puts
/// it in the login keychain.
fn new(sh: &Shell, folder: &Path) -> anyhow::Result<()> {
    let keychain = login_keychain()?;
    if find(sh, &keychain).is_some() {
        bail!(
            "the login keychain already holds a {NAME:?}; to make another, delete it in \
             Keychain Access first"
        );
    }
    fs::create_dir(folder).with_context(|| format!("making {}", folder.display()))?;
    // The folder holds the key and the password that opens it.
    cmd!(sh, "chmod 700 {folder}").quiet().run()?;

    let work = sh.create_temp_dir()?;
    let config = work.path().join("openssl.cnf");
    sh.write_file(&config, openssl_config())?;
    let key = work.path().join("key.pem");
    let certificate = work.path().join("certificate.pem");
    // Twenty years: when it runs out, macOS asks again for all it allowed.
    cmd!(
        sh,
        "{OPENSSL} req -x509 -newkey rsa:2048 -nodes -days 7300 -config {config} -keyout {key} -out {certificate}"
    )
    .quiet()
    .ignore_stderr()
    .run()?;
    let password = folder.join(PASSWORD);
    let random = cmd!(sh, "{OPENSSL} rand -hex 24").read()?;
    sh.write_file(&password, random)?;
    let identity = folder.join(IDENTITY);
    let passout = format!("file:{}", password.display());
    cmd!(
        sh,
        "{OPENSSL} pkcs12 -export -passout {passout} -inkey {key} -in {certificate} -out {identity}"
    )
    .quiet()
    .run()?;
    cmd!(sh, "chmod 600 {password} {identity}").quiet().run()?;

    import(sh, folder)?;
    println!(
        "Keep {} somewhere safe, apart from this Mac: `cargo xtask identity import` \
         puts it on another, and builds signed with another {NAME:?} lose what macOS allowed Cox.",
        folder.display()
    );
    Ok(())
}

/// Puts the certificate kept in `folder` in the login keychain, unless it is
/// there already.
fn import(sh: &Shell, folder: &Path) -> anyhow::Result<()> {
    let keychain = login_keychain()?;
    let identity = folder.join(IDENTITY);
    let password_file = folder.join(PASSWORD);
    let password = sh
        .read_file(&password_file)
        .with_context(|| format!("reading the password in {}", folder.display()))?;
    let password = password.trim();

    let passin = format!("file:{}", password_file.display());
    let certificate = cmd!(
        sh,
        "{OPENSSL} pkcs12 -in {identity} -passin {passin} -nokeys -clcerts"
    )
    .quiet()
    .ignore_stderr()
    .read()?;
    let fingerprint = cmd!(sh, "{OPENSSL} x509 -noout -fingerprint -sha1")
        .stdin(certificate)
        .read()?;
    let hash = sha1(&fingerprint).context("reading the certificate's fingerprint")?;
    match find(sh, &keychain) {
        Some(found) if found == hash => {
            println!("{NAME:?} is in the login keychain already");
            return Ok(());
        }
        Some(_) => {
            bail!("the login keychain holds another {NAME:?}; delete it in Keychain Access first")
        }
        None => {}
    }
    // `-T`: codesign may use the key without asking for the keychain's password.
    cmd!(
        sh,
        "security import {identity} -k {keychain} -P {password} -T /usr/bin/codesign"
    )
    .quiet()
    .run()?;
    println!("Put {NAME:?} in the login keychain");
    Ok(())
}

/// The SHA-1 hash of the certificate named [`NAME`] that the login keychain
/// holds with its key, as codesign takes it: by its hash, since codesign
/// takes a name for any certificate whose name begins with it.
pub fn find(sh: &Shell, keychain: &Path) -> Option<String> {
    // Without `-v`: a certificate of one's own is not one the system trusts,
    // which codesign signs with all the same.
    let listing = cmd!(sh, "security find-identity -p codesigning {keychain}")
        .quiet()
        .ignore_stderr()
        .read()
        .ok()?;
    hash_of(&listing, NAME)
}

pub fn login_keychain() -> anyhow::Result<PathBuf> {
    let home = env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Library/Keychains/login.keychain-db"))
}

fn openssl_config() -> String {
    format!(
        "[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = {NAME}
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
"
    )
}

/// The hash of the identity named exactly `name` in what `security
/// find-identity` lists, such as `  1) 0123…CDEF "Cox"`.
fn hash_of(listing: &str, name: &str) -> Option<String> {
    let quoted = format!("\"{name}\"");
    listing.lines().find_map(|line| {
        let (_, entry) = line.trim().split_once(") ")?;
        let (hash, rest) = entry.split_once(' ')?;
        (rest == quoted || rest.starts_with(&format!("{quoted} "))).then(|| hash.to_owned())
    })
}

/// The hash in what `openssl x509 -fingerprint -sha1` writes, such as
/// `SHA1 Fingerprint=01:23:…:EF`, as `security` writes it: `0123…EF`.
fn sha1(fingerprint: &str) -> Option<String> {
    let (_, hex) = fingerprint.trim().split_once('=')?;
    let hash: String = hex.split(':').collect::<String>().to_uppercase();
    (hash.len() == 40 && hash.chars().all(|c| c.is_ascii_hexdigit())).then_some(hash)
}

#[cfg(test)]
mod tests {
    use super::{hash_of, sha1};

    #[test]
    fn the_identity_is_found_by_its_exact_name() {
        let listing = "\
Policy: Code Signing
  Matching identities
  1) 1111111111111111111111111111111111111111 \"Cox Local Signing\" (CSSMERR_TP_NOT_TRUSTED)
  2) 2222222222222222222222222222222222222222 \"Cox\" (CSSMERR_TP_NOT_TRUSTED)
     2 identities found

  Valid identities only
     0 valid identities found
";
        assert_eq!(
            hash_of(listing, "Cox").as_deref(),
            Some("2222222222222222222222222222222222222222")
        );
        assert_eq!(
            hash_of(listing, "Cox Local Signing").as_deref(),
            Some("1111111111111111111111111111111111111111")
        );
        assert_eq!(hash_of(listing, "Co"), None);
    }

    #[test]
    fn the_fingerprint_is_read_as_security_writes_it() {
        assert_eq!(
            sha1("SHA1 Fingerprint=01:23:45:67:89:ab:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67\n")
                .as_deref(),
            Some("0123456789ABCDEF0123456789ABCDEF01234567")
        );
        assert_eq!(sha1("unable to load certificate"), None);
    }
}
