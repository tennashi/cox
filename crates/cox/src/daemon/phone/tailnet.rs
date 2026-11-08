//! The human's tailnet, which the phone reaches the page through: coxd
//! has `tailscale serve` hand the page to it, and takes a phone that
//! comes through it as the human's own when Tailscale says it is of the
//! same login as the Mac, with no secret to pair with.
//!
//! `tailscale serve` passes on what comes to the Mac's name in the
//! tailnet, `https://<name>/`, to the page on `127.0.0.1`, with the
//! login of whoever sent it, in `Tailscale-User-Login`, which it takes
//! out of what a phone sends itself, so that no one fakes it through
//! the tailnet. A page in a browser on the Mac, which reaches
//! `127.0.0.1` too, cannot add the header without asking first, as
//! browsers have it, which nothing here answers, nor send it under the
//! Mac's name in the tailnet, as a page whose name was turned to
//! `127.0.0.1` would send its own; so a request is the human's when it
//! comes for the Mac's name with the Mac's login. What else runs on the
//! Mac as the human can send it so too: that is the human's.

use std::io::Read as _;
use std::time::Duration;

use hyper::HeaderMap;
use hyper::header;

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

/// The header `tailscale serve` says who sent a request in.
const LOGIN: &str = "tailscale-user-login";

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
        if !ran.ok {
            return None;
        }
        Some((*program, status_in(&ran.out)?))
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

/// Whether a request with `headers` is the human's, of `owner`'s login
/// and for the Mac's name in the tailnet at the port `https`, as
/// `tailscale serve` passes one on.
pub fn of_the_owner(headers: &HeaderMap, owner: &Owner, https: u16) -> bool {
    let login = headers.get(LOGIN).and_then(|login| login.to_str().ok());
    let host = headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok());
    let at_port = format!("{}:{https}", owner.host);
    let came = host.is_some_and(|host| host == at_port || (https == 443 && host == owner.host));
    login == Some(owner.login.as_str()) && came
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
    for (at, web) in config
        .get("Web")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten()
    {
        for (path, handler) in web
            .get("Handlers")
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flatten()
        {
            let proxy = handler.get("Proxy").and_then(serde_json::Value::as_str);
            let proxy = proxy.map(|proxy| proxy.trim_end_matches('/'));
            if proxy.is_some_and(|proxy| page.iter().any(|page| page == proxy)) {
                return Served::Already;
            }
            if path == "/" && at.ends_with(&format!(":{https}")) {
                root = Some(format!("https://{}/", at.trim_end_matches(":443")));
            }
        }
    }
    root.map_or(Served::Nothing, Served::Other)
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
        .filter(|ran| ran.ok)
        .map_or(Served::Nothing, |ran| served_in(&ran.out, port, https));
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
        Some(ran) if ran.ok => Ok(format!("tailscale serves port {port} now")),
        Some(ran) if ran.ended => Err(said(&ran)),
        // As in a tailnet with Serve off, which it says where to turn on.
        Some(ran) => Err(format!("`tailscale serve` waits: {}", said(&ran))),
        None => Err(format!("`{program}` does not start")),
    }
}

/// What a `tailscale` that failed said, on a line.
fn said(ran: &Ran) -> String {
    let text = String::from_utf8_lossy(&ran.err).into_owned() + &String::from_utf8_lossy(&ran.out);
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("Warning: client version"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a program wrote, whether it ended in time, and whether it ended
/// well.
struct Ran {
    ended: bool,
    ok: bool,
    out: Vec<u8>,
    err: Vec<u8>,
}

/// Runs `program` with `args`, unless it fails to start, stopping it once
/// it takes longer than `time`.
fn run(program: &str, args: &[&str], time: Duration) -> Option<Ran> {
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let read = |mut from: Box<dyn std::io::Read + Send>| {
        let (wrote, written) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = from.read_to_end(&mut bytes);
            let _ = wrote.send(bytes);
        });
        written
    };
    let out = read(Box::new(child.stdout.take()?));
    let err = read(Box::new(child.stderr.take()?));
    let started = std::time::Instant::now();
    let wrote = out.recv_timeout(time).ok();
    let said = err
        .recv_timeout(time.saturating_sub(started.elapsed()))
        .ok();
    if let (Some(out), Some(err)) = (wrote.clone(), said.clone()) {
        let status = child.wait().ok()?;
        return Some(Ran {
            ended: true,
            ok: status.success(),
            out,
            err,
        });
    }
    // Stopped, it ends what it wrote, which the readers then have whole.
    let _ = child.kill();
    let _ = child.wait();
    let rest = |had: Option<Vec<u8>>, from: std::sync::mpsc::Receiver<Vec<u8>>| {
        had.or_else(|| from.recv_timeout(Duration::from_secs(1)).ok())
            .unwrap_or_default()
    };
    Some(Ran {
        ended: false,
        ok: false,
        out: rest(wrote, out),
        err: rest(said, err),
    })
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
    fn takes_only_the_owner_coming_for_the_mac() {
        let owner = Owner {
            host: "mac.tail1234.ts.net".to_owned(),
            login: "someone@github".to_owned(),
        };
        let headers = |host: &str, login: Option<&str>| {
            let mut headers = HeaderMap::new();
            headers.insert(header::HOST, host.parse().unwrap());
            if let Some(login) = login {
                headers.insert(LOGIN, login.parse().unwrap());
            }
            headers
        };
        assert!(of_the_owner(
            &headers("mac.tail1234.ts.net", Some("someone@github")),
            &owner,
            443
        ));
        assert!(of_the_owner(
            &headers("mac.tail1234.ts.net:443", Some("someone@github")),
            &owner,
            443
        ));
        // Someone else the Mac is shared with.
        assert!(!of_the_owner(
            &headers("mac.tail1234.ts.net", Some("other@github")),
            &owner,
            443
        ));
        // A page whose own name was turned to 127.0.0.1.
        assert!(!of_the_owner(
            &headers("evil.example:7421", Some("someone@github")),
            &owner,
            443
        ));
        assert!(!of_the_owner(
            &headers("mac.tail1234.ts.net", None),
            &owner,
            443
        ));
        // The sketches, served at a port of their own.
        assert!(of_the_owner(
            &headers("mac.tail1234.ts.net:7422", Some("someone@github")),
            &owner,
            7422
        ));
        assert!(!of_the_owner(
            &headers("mac.tail1234.ts.net", Some("someone@github")),
            &owner,
            7422
        ));
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
    fn stops_a_program_that_does_not_answer() {
        let ran = run("sh", &["-c", "printf hi; printf no >&2; exit 3"], ASKS).unwrap();
        assert!(!ran.ok);
        assert_eq!(ran.out, b"hi");
        assert_eq!(ran.err, b"no");
        let started = std::time::Instant::now();
        let waits = run(
            "sh",
            &[
                "-c",
                "echo visit https://login.tailscale.com/f/serve; exec sleep 30",
            ],
            Duration::from_millis(500),
        )
        .unwrap();
        assert!(!waits.ended && !waits.ok);
        assert_eq!(waits.out, b"visit https://login.tailscale.com/f/serve\n");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(run("cox-no-such-program", &[], ASKS).is_none());
    }
}
