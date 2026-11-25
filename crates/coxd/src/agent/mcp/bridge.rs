//! `cox --mcp <socket>`: the bridge between an agent and the server of
//! its chat's tools. The agent starts its MCP servers as programs that
//! speak MCP on their standard input and output; the bridge is such a
//! program, which only carries what the agent says to the server on the
//! chat's socket, and the answers back, as they come (see [`super`]).

use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;

use anyhow::Context as _;

/// The option that runs `cox` as the bridge, followed by the socket.
pub const OPTION: &str = "--mcp";

/// Runs `cox` as the bridge between the agent, on standard input and
/// output, and the chat's server on `socket`, until either hangs up.
///
/// # Errors
///
/// When the server cannot be reached, or the bridge cannot write to the
/// agent.
pub fn bridge(socket: &Path) -> anyhow::Result<()> {
    let server = UnixStream::connect(socket)
        .with_context(|| format!("cannot reach Cox at {}", socket.display()))?;
    let mut to_server = server.try_clone()?;
    thread::spawn(move || {
        let _ = pass(&mut io::stdin().lock(), &mut to_server);
        // The server hears that the agent is done, and hangs up too.
        let _ = to_server.shutdown(Shutdown::Write);
    });
    pass(&mut &server, &mut io::stdout().lock())
}

/// Passes on what `from` says to `to` as it comes, until `from` ends.
///
/// Not with `io::copy`, which on Linux has the kernel splice the agent's
/// pipe into the socket, where a line then waited for more to come.
fn pass(from: &mut impl Read, to: &mut impl Write) -> anyhow::Result<()> {
    let mut buffer = [0; 8192];
    loop {
        match from.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => {
                to.write_all(&buffer[..read])?;
                to.flush()?;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
}

/// The socket to bridge to, if `args`, those after the program's name, ask
/// `cox` to run as the bridge.
#[must_use]
pub fn bridge_to(args: &[String]) -> Option<PathBuf> {
    match args {
        [option, socket] if option == OPTION => Some(PathBuf::from(socket)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_as_the_bridge_only_when_asked_to() {
        let args = |words: &[&str]| {
            words
                .iter()
                .map(|&word| word.to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            bridge_to(&args(&["--mcp", "/tmp/cox-1-0/mcp"])),
            Some(PathBuf::from("/tmp/cox-1-0/mcp"))
        );
        assert_eq!(bridge_to(&args(&["claude-code-acp"])), None);
        assert_eq!(bridge_to(&args(&["--mcp"])), None);
    }
}
