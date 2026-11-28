//! The table of what coxd and the phone's page measure of themselves:
//! each instrument's name, its unit, which sets its buckets, and the
//! labels a measurement carries, each with the few values it may take.
//!
//! The table is the one place that says them. coxd records by
//! [`Instrument`], and what the page reports is read against the same
//! table, by the name the page says an instrument by (see
//! [`Instrument::reported`]). A label's value the table does not list is
//! told as [`OTHER`], so that neither a path, with a chat's number or a
//! file's name in it, nor anything a page sends makes a series of its own.

/// What a label's value is told as when the table does not list it.
pub const OTHER: &str = "other";

/// What an instrument counts in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Seconds,
    Bytes,
    /// A number of things, as the events that wait in a channel.
    Count,
}

impl Unit {
    /// The unit as OpenTelemetry writes it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Seconds => "s",
            Self::Bytes => "By",
            Self::Count => "1",
        }
    }

    /// The upper bounds of the buckets a histogram in this unit sorts its
    /// measurements into.
    #[must_use]
    pub fn buckets(self) -> Vec<f64> {
        match self {
            Self::Seconds => vec![
                0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.15, 0.25, 0.35, 0.5,
                0.75, 1.0, 1.5, 2.5, 5.0, 10.0, 30.0,
            ],
            Self::Bytes => (8..=24).map(|power| f64::from(1_u32 << power)).collect(),
            Self::Count => vec![
                0.0, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 500.0, 2000.0, 10000.0,
            ],
        }
    }

    /// Whether `value` is one a measurement in this unit can be: a number,
    /// not under zero, and not so large that only a mistake, or a page
    /// that is not Cox's, would say it.
    #[must_use]
    pub fn takes(self, value: f64) -> bool {
        let most = match self {
            Self::Seconds => 600.0,
            Self::Bytes | Self::Count => 1e9,
        };
        value.is_finite() && (0.0..=most).contains(&value)
    }
}

/// A label of an instrument: its key, and the values it may take.
#[derive(Debug, PartialEq, Eq)]
pub struct Label {
    pub key: &'static str,
    pub values: &'static [&'static str],
}

impl Label {
    /// The value `said` is told as: itself when the table lists it, else
    /// [`OTHER`].
    #[must_use]
    pub fn value(&self, said: &str) -> &'static str {
        self.values
            .iter()
            .find(|value| **value == said)
            .copied()
            .unwrap_or(OTHER)
    }
}

/// What a phone asked for, as the pattern of its path: a value ending in
/// `/*` stands for every path under it (see [`route`]).
pub const ROUTE: Label = Label {
    key: "route",
    values: &[
        "/",
        "/app.js",
        "/app.css",
        "/theme.css",
        "/sw.js",
        "/manifest.webmanifest",
        "/icon.svg",
        "/events",
        "/activity",
        "/activity/timeline",
        "/activity/resume",
        "/activity/fix",
        "/content",
        "/entries",
        "/changes",
        "/pages",
        "/sketch",
        "/diagram",
        "/act",
        "/push",
        "/push/end",
        "/telemetry",
        "/pair",
        "/pair/*",
        "/media/*",
        "/project-icon/*",
        "/map/api/*",
        "/map/*",
    ],
};

/// The route a request at `path` is counted under: the value of [`ROUTE`]
/// that is the path whole, or else the longest that stands for it, or
/// [`OTHER`].
#[must_use]
pub fn route(path: &str) -> &'static str {
    let whole = ROUTE.values.iter().find(|route| **route == path);
    let under = || {
        ROUTE
            .values
            .iter()
            .filter(|route| {
                route
                    .strip_suffix('*')
                    .is_some_and(|start| path.starts_with(start))
            })
            .max_by_key(|route| route.len())
    };
    whole.or_else(under).copied().unwrap_or(OTHER)
}

/// The status coxd answered a phone with.
pub const STATUS: Label = Label {
    key: "status",
    values: &[
        "200", "204", "206", "303", "400", "401", "403", "404", "413", "500", "503",
    ],
};

/// What a phone's request asked coxd's main loop for.
pub const ASK: Label = Label {
    key: "ask",
    values: &[
        "key",
        "pages",
        "content",
        "entries",
        "folder",
        "sketching",
        "map",
        "activity",
    ],
};

/// What coxd's main loop heard: `window` for what a window said, else
/// the kind of what coxd's own tasks told it.
pub const EVENT: Label = Label {
    key: "event",
    values: &[
        "window",
        "agent",
        "asked",
        "stage",
        "checked",
        "logged_in",
        "judge",
        "trigger",
        "gathered",
        "gathered_enough",
        "stop_due",
        "settled",
        "cancel",
        "attach",
        "detach",
        "view",
        "view_chat",
        "key",
        "unview",
        "act",
        "folder",
        "map",
        "activity",
        "content",
        "entries",
        "pages",
        "sketching",
        "push",
        "quit",
        "schedules",
        "pulls",
        "pulled",
        "listed",
        "tidied",
    ],
};

/// The kind of an event coxd tells a phone: `patch_first` is the patch
/// that tells a conversation whole, as a phone opens it.
pub const TOLD: Label = Label {
    key: "kind",
    values: &["listing", "patch_first", "patch", "news"],
};

/// The kind of phone a page's report came from, which coxd reads from
/// the request and the page does not say.
pub const CLIENT: Label = Label {
    key: "client",
    values: &["ios", "android"],
};

/// What a cold start of the page had reached: the answer's first byte,
/// the document read, the first list of chats, that list drawn on the
/// screen, and the browser's first contentful paint.
pub const COLD_PHASE: Label = Label {
    key: "phase",
    values: &["response", "dom", "listing", "drawn", "fcp"],
};

/// A file of the page, as the browser timed its fetch.
pub const FILE: Label = Label {
    key: "file",
    values: &["document", "app.js", "app.css", "theme.css"],
};

/// What the opening of a chat had reached: the list of chats that came
/// after the tap, the conversation's first patch, and the conversation
/// drawn on the screen.
pub const OPEN_PHASE: Label = Label {
    key: "phase",
    values: &["listing", "patch", "drawn"],
};

/// The view the page drew: its top, or a chat.
pub const VIEW: Label = Label {
    key: "view",
    values: &["home", "chat"],
};

/// The kind of an event the page was told.
pub const HEARD: Label = Label {
    key: "kind",
    values: &["listing", "patch", "news"],
};

/// What the table says of an instrument.
#[derive(Debug)]
pub struct Spec {
    /// Its name, as OpenTelemetry is told it.
    pub name: &'static str,
    pub unit: Unit,
    /// The labels a measurement of it carries, in the order their values
    /// are given in.
    pub labels: &'static [&'static Label],
    /// The name the phone's page reports it by, for an instrument the
    /// page measures. Its first label is then [`CLIENT`], which coxd
    /// gives, and the page says the rest.
    pub page: Option<&'static str>,
}

/// Declares [`Instrument`], one variant for each line, with what the
/// table says of each, so that the instruments and the table cannot
/// differ.
macro_rules! instruments {
    ($(
        $(#[$doc:meta])*
        $instrument:ident = $name:literal, $unit:ident, [$($label:ident),*], $page:expr;
    )*) => {
        /// An instrument: a histogram of what coxd, or the phone's page,
        /// measured.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Instrument {
            $($(#[$doc])* $instrument,)*
        }

        impl Instrument {
            /// Every instrument, in the order of their numbers.
            pub const ALL: &'static [Self] = &[$(Self::$instrument,)*];

            /// What the table says of it.
            #[must_use]
            pub const fn spec(self) -> &'static Spec {
                match self {
                    $(Self::$instrument => &Spec {
                        name: $name,
                        unit: Unit::$unit,
                        labels: &[$(&$label,)*],
                        page: $page,
                    },)*
                }
            }
        }
    };
}

instruments! {
    /// How long coxd took to have its answer to a phone's request: for
    /// the stream of events, until the stream opened.
    HttpDuration = "cox.phone.http.duration", Seconds, [ROUTE, STATUS], None;
    /// The size of an answer to a phone, as sent, packed if it was.
    HttpResponseSize = "cox.phone.http.response.size", Bytes, [ROUTE, STATUS], None;
    /// How long a phone's request waited for coxd's main loop to answer
    /// what it asked.
    AskWait = "cox.phone.ask.wait", Seconds, [ASK], None;
    /// The size of an event told to a phone.
    SseEventSize = "cox.phone.sse.event.size", Bytes, [TOLD], None;
    /// How long coxd took to write an event for a phone as JSON.
    SseEncodeDuration = "cox.phone.sse.encode.duration", Seconds, [TOLD], None;
    /// How many events waited behind the one coxd told a phone.
    SseQueue = "cox.phone.sse.queue", Count, [], None;
    /// How long coxd's main loop took over one thing it heard.
    HearDuration = "cox.daemon.hear.duration", Seconds, [EVENT], None;
    /// How long coxd's main loop took to settle after what it heard, as
    /// listing the chats again for the windows and the phones.
    SettleDuration = "cox.daemon.settle.duration", Seconds, [], None;
    /// How late coxd's one thread came back to a timer.
    LoopLag = "cox.daemon.loop.lag", Seconds, [], None;
    /// How long coxd took to read a closed chat as it was opened.
    ChatReadDuration = "cox.daemon.chat.read.duration", Seconds, [], None;
    /// How many entries a closed chat held as it was opened.
    ChatReadEntries = "cox.daemon.chat.read.entries", Count, [], None;
    /// How long after the page was asked for a cold start reached each
    /// phase.
    PageColdStart = "cox.phone.page.cold_start", Seconds, [CLIENT, COLD_PHASE], Some("cold");
    /// How long the browser took to fetch a file of the page.
    PageResourceDuration =
        "cox.phone.page.resource.duration", Seconds, [CLIENT, FILE], Some("resource");
    /// How long after the tap the opening of a chat reached each phase.
    PageChatOpen = "cox.phone.page.chat_open", Seconds, [CLIENT, OPEN_PHASE], Some("open");
    /// How long the page's script took over one draw.
    PageDrawDuration = "cox.phone.page.draw.duration", Seconds, [CLIENT, VIEW], Some("draw");
    /// How long from the start of a draw until the browser had painted
    /// it.
    PageDrawToPaint = "cox.phone.page.draw.to_paint", Seconds, [CLIENT, VIEW], Some("paint");
    /// How long the page took to parse an event.
    PageSseParseDuration =
        "cox.phone.page.sse.parse.duration", Seconds, [CLIENT, HEARD], Some("sse_parse");
    /// How late a timer of the page's thread fired: the most in each
    /// report.
    PageLag = "cox.phone.page.lag", Seconds, [CLIENT], Some("lag");
}

impl Instrument {
    /// The instrument the phone's page reports as `said`, if it reports
    /// one so.
    #[must_use]
    pub fn reported(said: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|instrument| instrument.spec().page == Some(said))
    }

    /// The labels of a measurement of it whose values are `said`, in the
    /// order of its spec's: each key with the value it is told as, which
    /// is [`OTHER`] for one the table does not list, or that is not
    /// said.
    pub fn labelled<'a>(
        self,
        said: &'a [&'a str],
    ) -> impl Iterator<Item = (&'static str, &'static str)> + 'a {
        self.spec().labels.iter().enumerate().map(|(at, label)| {
            (
                label.key,
                said.get(at).map_or(OTHER, |said| label.value(said)),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn a_route_is_its_pattern_whatever_the_path_names() {
        assert_eq!(route("/"), "/");
        assert_eq!(route("/events"), "/events");
        assert_eq!(route("/media/12/shot.png"), "/media/*");
        assert_eq!(route("/map/api/map/use"), "/map/api/*");
        assert_eq!(route("/map/app.js"), "/map/*");
        assert_eq!(route("/pair"), "/pair");
        assert_eq!(route("/pair/the-secret"), "/pair/*");
        assert_eq!(route("/wp-login.php"), OTHER);
    }

    #[test]
    fn a_label_tells_a_value_it_does_not_list_as_other() {
        let labels: Vec<_> = Instrument::HttpDuration
            .labelled(&["/media/*", "418"])
            .collect();
        assert_eq!(labels, [("route", "/media/*"), ("status", OTHER)]);
        let unsaid: Vec<_> = Instrument::AskWait.labelled(&[]).collect();
        assert_eq!(unsaid, [("ask", OTHER)]);
    }

    #[test]
    fn the_page_reports_only_the_instruments_it_measures() {
        assert_eq!(
            Instrument::reported("cold"),
            Some(Instrument::PageColdStart)
        );
        assert_eq!(Instrument::reported("cox.phone.http.duration"), None);
        assert_eq!(Instrument::reported("nothing"), None);
    }

    #[test]
    fn every_instrument_has_a_name_of_its_own_and_the_page_says_its_own_by_client() {
        let names: BTreeSet<_> = Instrument::ALL
            .iter()
            .map(|instrument| instrument.spec().name)
            .collect();
        assert_eq!(names.len(), Instrument::ALL.len());
        let said: Vec<_> = Instrument::ALL
            .iter()
            .filter_map(|instrument| instrument.spec().page)
            .collect();
        assert_eq!(said.iter().collect::<BTreeSet<_>>().len(), said.len());
        for instrument in Instrument::ALL {
            let spec = instrument.spec();
            assert_eq!(
                spec.page.is_some(),
                spec.labels.first() == Some(&&CLIENT),
                "{}",
                spec.name
            );
            let keys: BTreeSet<_> = spec.labels.iter().map(|label| label.key).collect();
            assert_eq!(keys.len(), spec.labels.len(), "{}", spec.name);
        }
    }

    #[test]
    fn the_documentation_lists_every_instrument_with_its_unit_and_labels() {
        let documentation = include_str!("../../../../docs/configuration.md");
        let listed: Vec<_> = documentation
            .lines()
            .filter(|line| line.starts_with("| `cox."))
            .collect();
        assert_eq!(listed.len(), Instrument::ALL.len());
        for instrument in Instrument::ALL {
            let spec = instrument.spec();
            let labels: Vec<_> = spec
                .labels
                .iter()
                .map(|label| format!("`{}`", label.key))
                .collect();
            let labels = if labels.is_empty() {
                "none".to_owned()
            } else {
                labels.join(", ")
            };
            let row = format!("| `{}` | {} | {labels} |", spec.name, spec.unit.name());
            assert!(
                listed.iter().any(|line| line.starts_with(&row)),
                "docs/configuration.md does not list {row}"
            );
        }
    }

    #[test]
    fn a_unit_takes_only_what_a_measurement_can_be() {
        assert!(Unit::Seconds.takes(0.0));
        assert!(Unit::Seconds.takes(1.5));
        assert!(!Unit::Seconds.takes(-1.0));
        assert!(!Unit::Seconds.takes(f64::NAN));
        assert!(!Unit::Seconds.takes(f64::INFINITY));
        assert!(!Unit::Seconds.takes(86_400.0));
        assert!(Unit::Bytes.takes(400_000.0));
    }
}
