//! The programs of the ACP Registry's agents that Cox downloads, as the
//! registry lists them for each platform, and of the language servers, as
//! their `servers.json` does (see [`crate::stage::servers`]): Cox
//! downloads the archive the first time it starts the program, checks it
//! by the SHA-256 it is given, and unpacks it in `<id>/<version>`, in
//! `cache/agents` or `cache/servers` where Cox keeps what lasts (see
//! [`crate::storage`]), for every chat and run after to start the program
//! from there.
//!
//! An archive is a `.tar.gz` (or `.tgz`), a `.tar.bz2` (or `.tbz2`), a
//! `.zip`, the program gzipped (`.gz`), or else the program itself.

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
    if folder.join(UNPACKED).exists() {
        return Ok(folder);
    }
    told(format!("Downloading {} …", download.archive));
    let fetched = beside(&folder, "download");
    fetch(&download.archive, &fetched)?;
    let unpacked = check(&fetched, &download.sha256)
        .and_then(|()| unpack(&fetched, file_name(&download.archive), &folder));
    let _ = fs::remove_file(&fetched);
    unpacked?;
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

    /// A folder of its own for each test.
    fn scratch(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("cox-download-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

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
        let scratch = scratch("unpack");
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
        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn an_archive_of_another_sha256_is_refused() {
        let scratch = scratch("refused");
        let archive = scratch.join("agent.tar.gz");
        tar_gz(&archive);
        let error = check(&archive, &"0".repeat(64)).unwrap_err();
        assert!(error.to_string().contains("not 0000"), "{error}");
        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_program_downloaded_as_it_is_is_made_to_run() {
        let scratch = scratch("program");
        let downloaded = scratch.join("downloaded");
        fs::write(&downloaded, "#!/bin/sh\n").unwrap();
        let folder = scratch.join("sigit");
        unpack(&downloaded, "sigit-linux-amd64", &folder).unwrap();
        assert!(folder.join("sigit-linux-amd64").exists());
        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_gzipped_program_is_unzipped_and_made_to_run() {
        let scratch = scratch("gz");
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
        let _ = fs::remove_dir_all(&scratch);
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
