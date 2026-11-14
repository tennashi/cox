//! The programs of the ACP Registry's agents that Cox downloads, as the
//! registry lists them for each platform, and of the language servers, as
//! their `servers.json` does (see `cox::stage::servers`): Cox
//! downloads the archive the first time it starts the program, checks it
//! by the SHA-256 it is given, and unpacks it in `<id>/<version>`, in
//! `cache/agents` or `cache/servers` where Cox keeps what lasts (see
//! [`crate::storage`]), for every chat and run after to start the program
//! from there.
//!
//! An archive is a `.tar.gz` (or `.tgz`), a `.tar.bz2` (or `.tbz2`), a
//! `.zip`, the program gzipped (`.gz`), or else the program itself.
//!
//! A language server's program may also be the asset of a release on
//! GitHub, which Cox checks by the SHA-256 GitHub gives of it (see
//! [`from_github`]), or a Go package, which `go install` installs, checking
//! it by the Go checksum database (see [`go_installed`]).

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::storage::Kept;

/// Where the program of an agent of the registry is downloaded from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    /// The agent's version, which the folder it is unpacked in is named
    /// after, so that a new version is downloaded again.
    pub version: String,
    /// The URL of the archive.
    pub archive: String,
    /// The archive's SHA-256, in hexadecimal.
    pub sha256: String,
}

/// What a folder holds once all of the archive is unpacked in it.
const UNPACKED: &str = ".cox-unpacked";

/// The platform Cox runs on, as the registry names them, such as
/// `linux-x86_64` or `darwin-aarch64`.
#[must_use]
pub fn platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        os => os,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

/// The folder that holds the program kept as `id` in `kept`, as an agent
/// of the registry in [`Kept::Downloads`], at `download`'s version,
/// downloading and unpacking it first if it is not there yet, after
/// `told` is told so.
///
/// # Errors
///
/// When the archive cannot be downloaded, its SHA-256 is not the one
/// `download` gives, or it cannot be unpacked.
pub fn installed(
    kept: Kept,
    id: &str,
    download: &Download,
    told: &dyn Fn(String),
) -> anyhow::Result<PathBuf> {
    let downloads = crate::storage::path(kept)
        .ok_or_else(|| anyhow::anyhow!("Cox has nowhere to keep what it downloads"))?;
    let folder = downloads.join(name(id)).join(name(&download.version));
    installed_in(&folder, download, told)?;
    Ok(folder)
}

/// Downloads and unpacks `download` in `folder`, unless it is there
/// already, after `told` is told so.
fn installed_in(folder: &Path, download: &Download, told: &dyn Fn(String)) -> anyhow::Result<()> {
    if folder.join(UNPACKED).exists() {
        return Ok(());
    }
    told(format!("Downloading {} …", download.archive));
    one_at_a_time(folder, || {
        let fetched = beside(folder, "download");
        fetch(&download.archive, &fetched)?;
        let unpacked = check(&fetched, &download.sha256)
            .and_then(|()| unpack(&fetched, file_name(&download.archive), folder));
        let _ = fs::remove_file(&fetched);
        unpacked
    })
}

/// Makes `folder` with `install`, unless it is there already, one at a
/// time: while one install of `folder` runs, in any thread or process of
/// Cox, another waits for it and then takes what it made.
fn one_at_a_time(
    folder: &Path,
    install: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    if let Some(parent) = folder.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock = File::create(beside(folder, "lock"))?;
    lock.lock()?;
    if folder.join(UNPACKED).exists() {
        return Ok(());
    }
    install()
}

/// Whether the program kept as `id` in `kept` is there at `version`,
/// downloaded and unpacked (see [`installed`]).
#[must_use]
pub fn is_installed(kept: Kept, id: &str, version: &str) -> bool {
    installed_at(kept, id, version).is_some()
}

/// The folder that holds the program kept as `id` in `kept` at
/// `version`, if it is there, downloaded and unpacked (see
/// [`installed`]).
#[must_use]
pub fn installed_at(kept: Kept, id: &str, version: &str) -> Option<PathBuf> {
    let folder = crate::storage::path(kept)?
        .join(name(id))
        .join(name(version));
    folder.join(UNPACKED).exists().then_some(folder)
}

/// Where the asset named `asset` of the release tagged `tag` of the
/// GitHub repository `repository`, as `rust-lang/rust-analyzer`, is
/// downloaded from, with the SHA-256 GitHub gives of it, at `tag` as its
/// version.
///
/// # Errors
///
/// When GitHub cannot be asked, has no such release or asset, or gives no
/// SHA-256 of it, as for assets uploaded before GitHub began keeping them.
pub fn from_github(repository: &str, tag: &str, asset: &str) -> anyhow::Result<Download> {
    let url = format!("https://api.github.com/repos/{repository}/releases/tags/{tag}");
    let release = ureq::get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "cox")
        .call()
        .map_err(|error| anyhow::anyhow!("asking GitHub for {repository} {tag}: {error}"))?
        .body_mut()
        .read_to_string()?;
    asset_of(&release, repository, tag, asset)
}

/// The asset named `asset` of `release`, a release of `repository` tagged
/// `tag` as GitHub's API answers it.
fn asset_of(release: &str, repository: &str, tag: &str, asset: &str) -> anyhow::Result<Download> {
    #[derive(serde::Deserialize)]
    struct Release {
        assets: Vec<Asset>,
    }
    #[derive(serde::Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
        digest: Option<String>,
    }
    let release: Release = serde_json::from_str(release)?;
    let found = release
        .assets
        .into_iter()
        .find(|found| found.name == asset)
        .ok_or_else(|| anyhow::anyhow!("{repository} {tag} has no asset {asset}"))?;
    let sha256 = found
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "GitHub gives no SHA-256 of {asset} of {repository} {tag}, so Cox does not \
                 download it"
            )
        })?;
    Ok(Download {
        version: tag.to_owned(),
        archive: found.browser_download_url,
        sha256: sha256.to_owned(),
    })
}

/// The folder that holds the program `go install` makes of the Go package
/// `package` at `version`, kept as `id` in `kept`, installing it first if
/// it is not there yet, after `told` is told so. `go` has to be on `PATH`.
///
/// # Errors
///
/// When `go` cannot be run, or does not install the package.
pub fn go_installed(
    kept: Kept,
    id: &str,
    package: &str,
    version: &str,
    told: &dyn Fn(String),
) -> anyhow::Result<PathBuf> {
    let downloads = crate::storage::path(kept)
        .ok_or_else(|| anyhow::anyhow!("Cox has nowhere to keep what it downloads"))?;
    let folder = downloads.join(name(id)).join(name(version));
    if folder.join(UNPACKED).exists() {
        return Ok(folder);
    }
    told(format!("Installing {package}@{version} with go install …"));
    one_at_a_time(&folder, || {
        let partial = beside(&folder, "partial");
        let _ = fs::remove_dir_all(&partial);
        fs::create_dir_all(&partial)?;
        let installed = std::process::Command::new("go")
            .args(["install", &format!("{package}@{version}")])
            .env("GOBIN", &partial)
            .output()
            .map_err(|error| anyhow::anyhow!("go could not be run: {error}"))
            .and_then(|output| {
                anyhow::ensure!(
                    output.status.success(),
                    "go install {package}@{version} failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
                File::create(partial.join(UNPACKED))?;
                let _ = fs::remove_dir_all(&folder);
                fs::rename(&partial, &folder)?;
                Ok(())
            });
        if installed.is_err() {
            let _ = fs::remove_dir_all(&partial);
        }
        installed
    })?;
    Ok(folder)
}

/// The path beside `folder` named after it, with `.` and `suffix`.
fn beside(folder: &Path, suffix: &str) -> PathBuf {
    let mut name = folder.file_name().unwrap_or_default().to_owned();
    name.push(".");
    name.push(suffix);
    folder.with_file_name(name)
}

/// `text` as the name of a folder, with no separator or `..` in it.
fn name(text: &str) -> String {
    let name: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.chars().all(|c| c == '.') {
        name.replace('.', "_")
    } else {
        name
    }
}

/// The last part of the path of `url`, as `amp-acp-linux-x86_64.tar.gz`.
fn file_name(url: &str) -> &str {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit('/').next().unwrap_or(path)
}

/// Downloads `url` to the file at `to`.
fn fetch(url: &str, to: &Path) -> anyhow::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut response = ureq::get(url).call()?;
    let mut file = File::create(to)?;
    io::copy(&mut response.body_mut().as_reader(), &mut file)?;
    file.flush()?;
    Ok(())
}

/// Checks that the file at `path` has the SHA-256 `sha256`.
fn check(path: &Path, sha256: &str) -> anyhow::Result<()> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let found = hex(&hasher.finalize());
    anyhow::ensure!(
        found.eq_ignore_ascii_case(sha256.trim()),
        "the archive's SHA-256 is {found}, not {sha256} as it should be"
    );
    Ok(())
}

/// `bytes` in hexadecimal.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

/// How an archive is packed.
#[derive(Debug, PartialEq, Eq)]
enum Packing {
    TarGz,
    TarBz2,
    Zip,
    /// The program, gzipped.
    Gz,
    /// Not at all: it is the program itself.
    Program,
}

/// How the archive named `name` is packed, as its name says.
fn packing(name: &str) -> Packing {
    let extension = |path: &Path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
    };
    let path = Path::new(name);
    let inner = path.file_stem().map(Path::new).and_then(extension);
    match (extension(path).as_deref(), inner.as_deref()) {
        (Some("tgz"), _) | (Some("gz"), Some("tar")) => Packing::TarGz,
        (Some("tbz2"), _) | (Some("bz2"), Some("tar")) => Packing::TarBz2,
        (Some("zip"), _) => Packing::Zip,
        (Some("gz"), _) => Packing::Gz,
        _ => Packing::Program,
    }
}

/// Unpacks the archive at `path`, named `name` where it was downloaded
/// from, in `folder`, which then holds all of it or nothing.
fn unpack(path: &Path, name: &str, folder: &Path) -> anyhow::Result<()> {
    let partial = beside(folder, "partial");
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial)?;
    let unpacked = unpack_in(path, name, &partial).and_then(|()| {
        File::create(partial.join(UNPACKED))?;
        let _ = fs::remove_dir_all(folder);
        fs::rename(&partial, folder)?;
        Ok(())
    });
    if unpacked.is_err() {
        let _ = fs::remove_dir_all(&partial);
    }
    unpacked
}

/// Unpacks the archive at `path`, named `name`, in `folder`, as its name
/// says it is packed.
fn unpack_in(path: &Path, name: &str, folder: &Path) -> anyhow::Result<()> {
    let file = File::open(path)?;
    match packing(name) {
        Packing::TarGz => tar::Archive::new(flate2::read::GzDecoder::new(file)).unpack(folder)?,
        Packing::TarBz2 => tar::Archive::new(bzip2::read::BzDecoder::new(file)).unpack(folder)?,
        Packing::Zip => zip::ZipArchive::new(file)?.extract(folder)?,
        Packing::Gz => {
            let stem = Path::new(name).file_stem().unwrap_or_default();
            let program = folder.join(self::name(&stem.to_string_lossy()));
            io::copy(
                &mut flate2::read::GzDecoder::new(file),
                &mut File::create(&program)?,
            )?;
            executable(&program)?;
        }
        Packing::Program => {
            let program = folder.join(self::name(name));
            fs::copy(path, &program)?;
            executable(&program)?;
        }
    }
    Ok(())
}

/// Makes the file at `path` one that runs.
#[cfg(unix)]
fn executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn executable(_: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    /// A `.tar.gz` that holds `bin/agent`, a program.
    fn tar_gz(at: &Path) {
        let file = File::create(at).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut archive = tar::Builder::new(encoder);
        let program = b"#!/bin/sh\necho agent\n";
        let mut header = tar::Header::new_gnu();
        header.set_size(program.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, "bin/agent", &program[..])
            .unwrap();
        archive.into_inner().unwrap().finish().unwrap();
    }

    fn sha256_of(path: &Path) -> String {
        hex(&Sha256::digest(fs::read(path).unwrap()))
    }

    #[test]
    fn unpacks_an_archive_it_has_checked() {
        let scratch = TempDir::new();
        let archive = scratch.join("agent.tar.gz");
        tar_gz(&archive);
        check(&archive, &sha256_of(&archive).to_uppercase()).unwrap();

        let folder = scratch.join("agent").join("1.0.0");
        unpack(&archive, "agent-linux-x86_64.tar.gz", &folder).unwrap();
        let program = folder.join("bin/agent");
        assert_eq!(
            fs::read_to_string(&program).unwrap(),
            "#!/bin/sh\necho agent\n"
        );
        assert!(folder.join(UNPACKED).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&program).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
    }

    /// Serves `body` over HTTP on a port of its own to as many as ask at
    /// once, holding each answer until `held` is dropped, counting how
    /// many asked.
    fn serve_held(
        body: Vec<u8>,
        held: std::sync::mpsc::Receiver<()>,
    ) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{BufRead as _, BufReader};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = format!("http://{}/agent.tar.gz", listener.local_addr().unwrap());
        let asked = Arc::new(AtomicUsize::new(0));
        let counted = asked.clone();
        let body = Arc::new(body);
        let held = Arc::new(Mutex::new(held));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let (body, gate) = (body.clone(), held.clone());
                counted.fetch_add(1, Ordering::SeqCst);
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    while reader.read_line(&mut line).unwrap() > 2 {
                        line.clear();
                    }
                    let _ = gate.lock().unwrap().recv();
                    let status = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(status.as_bytes()).unwrap();
                    stream.write_all(&body).unwrap();
                });
            }
        });
        (url, asked)
    }

    #[test]
    fn two_starts_at_once_download_an_agent_once() {
        let scratch = TempDir::new();
        let archive = scratch.join("agent.tar.gz");
        tar_gz(&archive);
        let (release, held) = std::sync::mpsc::channel();
        let (url, asked) = serve_held(fs::read(&archive).unwrap(), held);
        let download = Download {
            version: "1.0.0".to_owned(),
            archive: url,
            sha256: sha256_of(&archive),
        };
        let folder = scratch.join("agent").join("1.0.0");

        let (downloading, told) = std::sync::mpsc::channel();
        let starts: Vec<_> = (0..2)
            .map(|_| {
                let (folder, download) = (folder.clone(), download.clone());
                let downloading = downloading.clone();
                std::thread::spawn(move || {
                    installed_in(&folder, &download, &|_| {
                        let _ = downloading.send(());
                    })
                })
            })
            .collect();
        for _ in 0..2 {
            told.recv_timeout(std::time::Duration::from_secs(10))
                .expect("both start downloading before either has the agent");
        }
        drop(release);
        for start in starts {
            start.join().unwrap().unwrap();
        }

        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(folder.join(UNPACKED).exists());
        assert_eq!(
            fs::read_to_string(folder.join("bin/agent")).unwrap(),
            "#!/bin/sh\necho agent\n"
        );
    }

    #[test]
    fn an_archive_of_another_sha256_is_refused() {
        let scratch = TempDir::new();
        let archive = scratch.join("agent.tar.gz");
        tar_gz(&archive);
        let error = check(&archive, &"0".repeat(64)).unwrap_err();
        assert!(error.to_string().contains("not 0000"), "{error}");
    }

    #[test]
    fn a_program_downloaded_as_it_is_is_made_to_run() {
        let scratch = TempDir::new();
        let downloaded = scratch.join("downloaded");
        fs::write(&downloaded, "#!/bin/sh\n").unwrap();
        let folder = scratch.join("sigit");
        unpack(&downloaded, "sigit-linux-amd64", &folder).unwrap();
        let program = folder.join("sigit-linux-amd64");
        assert_eq!(fs::read_to_string(&program).unwrap(), "#!/bin/sh\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&program).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
    }

    #[test]
    fn a_gzipped_program_is_unzipped_and_made_to_run() {
        let scratch = TempDir::new();
        let downloaded = scratch.join("downloaded");
        let mut encoder = flate2::write::GzEncoder::new(
            File::create(&downloaded).unwrap(),
            flate2::Compression::fast(),
        );
        encoder.write_all(b"#!/bin/sh\n").unwrap();
        encoder.finish().unwrap();
        let folder = scratch.join("rust-analyzer");
        unpack(
            &downloaded,
            "rust-analyzer-x86_64-unknown-linux-gnu.gz",
            &folder,
        )
        .unwrap();
        let program = folder.join("rust-analyzer-x86_64-unknown-linux-gnu");
        assert_eq!(fs::read_to_string(&program).unwrap(), "#!/bin/sh\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&program).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
    }

    #[test]
    fn takes_an_asset_of_a_release_with_the_sha256_github_gives() {
        let release = r#"{ "tag_name": "2026-11-09", "assets": [
            { "name": "rust-analyzer-x86_64-unknown-linux-gnu.gz",
              "browser_download_url": "https://github.com/rust-lang/rust-analyzer/releases/download/2026-11-09/rust-analyzer-x86_64-unknown-linux-gnu.gz",
              "digest": "sha256:8c039b25cb338b3970c731646677a4c9d4e9dd7a92c9ab9ae459c71467fbf4a9" },
            { "name": "rust-analyzer-old.gz", "browser_download_url": "https://example.com/old.gz", "digest": null }
        ] }"#;
        let of = |asset| asset_of(release, "rust-lang/rust-analyzer", "2026-11-09", asset);
        assert_eq!(
            of("rust-analyzer-x86_64-unknown-linux-gnu.gz").unwrap(),
            Download {
                version: "2026-11-09".to_owned(),
                archive: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-11-09/rust-analyzer-x86_64-unknown-linux-gnu.gz".to_owned(),
                sha256: "8c039b25cb338b3970c731646677a4c9d4e9dd7a92c9ab9ae459c71467fbf4a9".to_owned(),
            }
        );
        let unchecked = of("rust-analyzer-old.gz").unwrap_err().to_string();
        assert!(unchecked.contains("no SHA-256"), "{unchecked}");
        let missing = of("rust-analyzer-windows.zip").unwrap_err().to_string();
        assert!(missing.contains("has no asset"), "{missing}");
    }

    #[test]
    fn names_a_folder_with_nothing_that_leads_out_of_it() {
        assert_eq!(name("registry:amp-acp"), "registry_amp-acp");
        assert_eq!(name("../x"), ".._x");
        assert_eq!(name(".."), "__");
        assert_eq!(
            file_name("https://example.com/a/agent.tar.gz?raw=1"),
            "agent.tar.gz"
        );
        assert!(platform().contains('-'));
    }

    #[test]
    fn tells_how_an_archive_is_packed_by_its_name() {
        assert_eq!(packing("goose-x86_64.tar.bz2"), Packing::TarBz2);
        assert_eq!(packing("amp-acp-linux.TAR.GZ"), Packing::TarGz);
        assert_eq!(packing("agent.tgz"), Packing::TarGz);
        assert_eq!(
            packing("agy-acp-server-1.4.5-linux-x86_64.zip"),
            Packing::Zip
        );
        assert_eq!(packing("sigit-linux-amd64"), Packing::Program);
        assert_eq!(
            packing("rust-analyzer-aarch64-apple-darwin.gz"),
            Packing::Gz
        );
    }
}
