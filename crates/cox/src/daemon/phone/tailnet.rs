//! The human's tailnet, which the phone reaches the page through: coxd
//! has `tailscale serve` hand the page to it, and each port the relay
//! passes on (see [`super::relay`]).
//!
//! `tailscale serve` passes on what comes to the Mac's name in the
//! tailnet, `https://<name>/`, to the page on `127.0.0.1`, saying the
//! login of whoever sent it in `Tailscale-User-Login`. coxd does not go
//! by it: anything on the Mac that reaches `127.0.0.1` can send the
//! same, the Mac's name and login being no secret, and what the Mac
//! itself sends to its name in the tailnet comes through `tailscale
//! serve` as of the Mac's login. So every phone pairs, the human's own
//! too (see [`super::Phones`]).

use std::process::Command;
use std::time::Duration;

use crate::process::{Output, run_with_deadline};

/// Where `tailscale` is: on `PATH`, as Homebrew's or the app's own
/// installed from its settings, or in the app.
const PROGRAMS: &[&str] = &[
    "tailscale",
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
];

/// How long `tailscale` may take to answer: the app's own program, asked
/// from where the app does not run it, may never answer, and `tailscale
/// serve` waits for Serve to be turned on in a tailnet that has it off.
const ASKS: Duration = Duration::from_secs(5);

/// The Mac in the tailnet: its name there, as `mac.tail1234.ts.net`, and
/// the login it is of, as `someone@github`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub host: String,
    pub login: String,
}

/// What `tailscale status` says of the tailnet: the Mac, unless it is of
/// no login, and whether the tailnet's devices find each other by name.
#[derive(Debug, PartialEq, Eq)]
pub struct Status {
    pub owner: Option<Owner>,
    pub magic_dns: bool,
}

/// What `tailscale status` says, through the first `tailscale` that
/// answers, with that program.
pub fn status() -> Option<(&'static str, Status)> {
    PROGRAMS.iter().find_map(|program| {
        let ran = run(program, &["status", "--json"], ASKS)?;
        if !ran.success() {
            return None;
        }
        Some((*program, status_in(&ran.stdout)?))
    })
}

/// The Mac and the tailnet, as `tailscale status --json` says them in
/// `bytes`.
fn status_in(bytes: &[u8]) -> Option<Status> {
    let status: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let host = status
        .pointer("/Self/DNSName")?
        .as_str()?
        .trim_end_matches('.');
    let user = status.pointer("/Self/UserID")?.as_u64()?;
    let login = status
        .pointer(&format!("/User/{user}/LoginName"))
        .and_then(serde_json::Value::as_str)
        // A device given a tag is of no one.
        .filter(|login| login.contains('@'));
    let owner = match login {
        Some(login) if !host.is_empty() => Some(Owner {
            host: host.to_owned(),
            login: login.to_owned(),
        }),
        _ => None,
    };
    let magic_dns = status
        .pointer("/CurrentTailnet/MagicDNSEnabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    Some(Status { owner, magic_dns })
}

/// How the tailnet hands what coxd serves on `port` to the phone.
#[derive(Debug, PartialEq, Eq)]
pub enum Served {
    /// `tailscale serve` passes it on already.
    Already,
    /// Something else is served at the address it would be, which coxd
    /// leaves.
    Other(String),
    /// Nothing is served there.
    Nothing,
}

/// What `tailscale serve status --json` says, in `bytes`, of how what
/// coxd serves on `port` is served, at the port `https` of the tailnet.
fn served_in(bytes: &[u8], port: u16, https: u16) -> Served {
    let Ok(config) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Served::Nothing;
    };
    let page = [
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
    ];
    let mut root = None;
    for (at, path, proxy) in handlers(&config) {
        if !at.ends_with(&format!(":{https}")) {
            continue;
        }
        if proxy.is_some_and(|proxy| page.iter().any(|page| page == proxy)) {
            return Served::Already;
        }
        if path == "/" {
            root = Some(format!("https://{}/", at.trim_end_matches(":443")));
        }
    }
    root.map_or(Served::Nothing, Served::Other)
}

/// Each handler `config`, as `tailscale serve status --json` says it,
/// has: where it serves, as `mac.tail1234.ts.net:443`, at which path,
/// and what it passes on to, if it passes on.
fn handlers(config: &serde_json::Value) -> impl Iterator<Item = (&str, &str, Option<&str>)> {
    config
        .get("Web")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten()
        .flat_map(|(at, web)| {
            web.get("Handlers")
                .and_then(serde_json::Value::as_object)
                .into_iter()
                .flatten()
                .map(move |(path, handler)| {
                    let proxy = handler.get("Proxy").and_then(serde_json::Value::as_str);
                    (
                        at.as_str(),
                        path.as_str(),
                        proxy.map(|proxy| proxy.trim_end_matches('/')),
                    )
                })
        })
}

/// The ports of the tailnet that `tailscale serve status --json` says,
/// in `bytes`, are handed to the relay, on `relay`.
fn relayed_in(bytes: &[u8], relay: u16) -> std::collections::BTreeSet<u16> {
    let Ok(config) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return std::collections::BTreeSet::new();
    };
    let to = [
        format!("http://127.0.0.1:{relay}"),
        format!("http://localhost:{relay}"),
        format!("127.0.0.1:{relay}"),
        format!("localhost:{relay}"),
    ];
    handlers(&config)
        .filter(|(_, path, proxy)| {
            *path == "/" && proxy.is_some_and(|proxy| to.iter().any(|to| to == proxy))
        })
        .filter_map(|(at, _, _)| at.rsplit_once(':')?.1.parse().ok())
        .collect()
}

/// The ports of the tailnet handed to the relay, on `relay`, as
/// `tailscale serve`, through `program`, says, as coxd left them.
pub fn relayed(program: &str, relay: u16) -> std::collections::BTreeSet<u16> {
    run(program, &["serve", "status", "--json"], ASKS)
        .filter(Output::success)
        .map(|ran| relayed_in(&ran.stdout, relay))
        .unwrap_or_default()
}

/// Has `tailscale serve`, through `program`, serve nothing at its port
/// `https` any more.
///
/// # Errors
///
/// When `tailscale serve` fails.
pub fn unserve(program: &str, https: u16) -> Result<(), String> {
    let at = format!("--https={https}");
    match run(program, &["serve", &at, "off"], ASKS) {
        Some(ran) if ran.success() => Ok(()),
        Some(ran) => Err(said(&ran)),
        None => Err(format!("`{program}` does not start")),
    }
}

/// Has `tailscale serve` hand what coxd serves on `port` to the
/// tailnet, at its port `https`, unless it does, through `program`: what
/// coxd did, in words, or why it did not.
///
/// # Errors
///
/// When something else is served at the Mac's name, or `tailscale serve`
/// fails, as in a tailnet that has Serve off, when it says where to turn
/// it on.
pub fn serve(program: &str, port: u16, https: u16) -> Result<String, String> {
    let status = run(program, &["serve", "status", "--json"], ASKS)
        .filter(Output::success)
        .map_or(Served::Nothing, |ran| served_in(&ran.stdout, port, https));
    let at = if https == 443 {
        String::new()
    } else {
        format!("--https={https} ")
    };
    match status {
        Served::Already => return Ok(format!("tailscale serves port {port} already")),
        Served::Other(there) => {
            return Err(format!(
                "tailscale serves something else at {there}, which coxd leaves: \
                 run `tailscale serve --bg {at}{port}` to serve port {port} there"
            ));
        }
        Served::Nothing => {}
    }
    let https = format!("--https={https}");
    let port_said = port.to_string();
    match run(program, &["serve", "--bg", &https, &port_said], ASKS) {
        Some(ran) if ran.success() => Ok(format!("tailscale serves port {port} now")),
        Some(ran) if ran.status.is_some() => Err(said(&ran)),
        // As in a tailnet with Serve off, which it says where to turn on.
        Some(ran) => Err(format!("`tailscale serve` waits: {}", said(&ran))),
        None => Err(format!("`{program}` does not start")),
    }
}

/// What a `tailscale` that failed said, on a line.
fn said(ran: &Output) -> String {
    let text =
        String::from_utf8_lossy(&ran.stderr).into_owned() + &String::from_utf8_lossy(&ran.stdout);
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("Warning: client version"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Runs `program` with `args`, unless it fails to start, stopping it once
/// it takes longer than `time`.
fn run(program: &str, args: &[&str], time: Duration) -> Option<Output> {
    run_with_deadline(Command::new(program).args(args), None, time, |_| {}).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_mac_and_the_tailnet() {
        let status = br#"{
            "Self": {"DNSName": "mac.tail1234.ts.net.", "UserID": 42},
            "User": {"42": {"LoginName": "someone@github"}},
            "CurrentTailnet": {"MagicDNSEnabled": true}
        }"#;
        assert_eq!(
            status_in(status),
            Some(Status {
                owner: Some(Owner {
                    host: "mac.tail1234.ts.net".to_owned(),
                    login: "someone@github".to_owned(),
                }),
                magic_dns: true,
            })
        );
        let tagged = br#"{
            "Self": {"DNSName": "mac.tail1234.ts.net.", "UserID": 7},
            "User": {"7": {"LoginName": "tagged-devices"}}
        }"#;
        assert_eq!(
            status_in(tagged),
            Some(Status {
                owner: None,
                magic_dns: false
            })
        );
        assert_eq!(status_in(b"not json"), None);
    }

    #[test]
    fn tells_how_the_page_is_served() {
        let served = br#"{"TCP": {"443": {"HTTPS": true}}, "Web": {"mac.tail1234.ts.net:443":
            {"Handlers": {"/": {"Proxy": "http://127.0.0.1:7421"}}}}}"#;
        assert_eq!(served_in(served, 7421, 443), Served::Already);
        assert_eq!(served_in(served, 7422, 7422), Served::Nothing);
        let other = br#"{"Web": {"mac.tail1234.ts.net:443":
            {"Handlers": {"/": {"Proxy": "http://127.0.0.1:3000"}}}}}"#;
        assert_eq!(
            served_in(other, 7421, 443),
            Served::Other("https://mac.tail1234.ts.net/".to_owned())
        );
        assert_eq!(served_in(b"{}", 7421, 443), Served::Nothing);
        assert_eq!(served_in(b"", 7421, 443), Served::Nothing);
    }

    #[test]
    fn tells_which_ports_are_handed_to_the_relay() {
        let served = br#"{"Web": {
            "mac.tail1234.ts.net:443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:7421"}}},
            "mac.tail1234.ts.net:5173": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:7422"}}},
            "mac.tail1234.ts.net:7423": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:7422"}}},
            "mac.tail1234.ts.net:3000": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:3000"}}}
        }}"#;
        assert_eq!(
            relayed_in(served, 7422),
            std::collections::BTreeSet::from([5173, 7423])
        );
        assert_eq!(served_in(served, 7422, 5173), Served::Already);
        assert_eq!(served_in(served, 7422, 8080), Served::Nothing);
        assert_eq!(
            served_in(served, 7422, 3000),
            Served::Other("https://mac.tail1234.ts.net:3000/".to_owned())
        );
        assert!(relayed_in(b"", 7422).is_empty());
    }

    #[test]
    fn a_program_not_installed_is_not_run() {
        assert!(run("cox-no-such-program", &[], ASKS).is_none());
    }
}
