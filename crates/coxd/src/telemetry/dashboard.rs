//! The Grafana dashboard of the phone's page,
//! `telemetry/grafana/dashboards/cox-phone.json`, made from the table of
//! [`instruments`](super::instruments): a panel asks only for an
//! instrument the table has, by labels and values that instrument has,
//! under the name Prometheus keeps it by.
//!
//! The file in the repository is what [`dashboard`] makes, and a test
//! fails when the two differ. To write the file again after a change
//! here or in the table:
//!
//! ```sh
//! COX_WRITE_DASHBOARD=1 cargo test -p coxd telemetry::dashboard
//! ```

use serde_json::{Value, json};

use super::instruments::{
    ASK, CLIENT, COLD_PHASE, EVENT, FILE, HEARD, Instrument, Label, OPEN_PHASE, ROUTE, STATUS,
    TOLD, Unit, VIEW,
};

/// The dashboard as the repository holds it.
const KEPT: &str = include_str!("../../../../telemetry/grafana/dashboards/cox-phone.json");

/// Where the repository holds it.
const PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../telemetry/grafana/dashboards/cox-phone.json"
);

/// The environment variable that has the test write the file.
const WRITE: &str = "COX_WRITE_DASHBOARD";

/// The data source `telemetry/grafana/datasource.yaml` sets up.
fn prometheus() -> Value {
    json!({"type": "prometheus", "uid": "prometheus"})
}

/// What a query tells of a histogram.
#[derive(Clone, Copy)]
enum Measure {
    /// The value this many of a hundred measurements stayed under.
    Quantile(u8),
    /// The sum of the measurements in each second: for a duration, the
    /// share of the time it took.
    Total,
    /// How many measurements there were in each second.
    Events,
}

/// A query of one instrument.
struct Query {
    instrument: Instrument,
    measure: Measure,
    /// The labels it tells a line for each value of.
    by: Vec<&'static Label>,
    /// The one value of a label it is of.
    only: Option<(&'static Label, &'static str)>,
    /// What its line is called, when no label names it.
    named: Option<&'static str>,
}

impl Query {
    /// A query of `instrument`, with a line for each value of `by`.
    fn of(instrument: Instrument, measure: Measure, by: &[&'static Label]) -> Self {
        for label in by {
            assert!(
                instrument.spec().labels.contains(label),
                "{} has no label {}",
                instrument.spec().name,
                label.key
            );
        }
        Self {
            instrument,
            measure,
            by: by.to_vec(),
            only: None,
            named: None,
        }
    }

    /// The query of the measurements whose `label` is `value` alone.
    fn only(mut self, label: &'static Label, value: &'static str) -> Self {
        let spec = self.instrument.spec();
        assert!(
            spec.labels.contains(&label) && label.values.contains(&value),
            "{} has no {}={value}",
            spec.name,
            label.key
        );
        self.only = Some((label, value));
        self
    }

    /// The query with its line called `name`.
    fn named(mut self, name: &'static str) -> Self {
        self.named = Some(name);
        self
    }

    /// The name of the instrument's `part`, as Prometheus names what it
    /// takes over OTLP: the dots as underscores, then the unit.
    fn name(&self, part: &str) -> String {
        let spec = self.instrument.spec();
        let unit = match spec.unit {
            Unit::Seconds => "_seconds",
            Unit::Bytes => "_bytes",
            Unit::Count => "",
        };
        format!("{}{unit}_{part}", spec.name.replace('.', "_"))
    }

    /// The series of the instrument's `part` the query is of: those of
    /// the phones chosen on the dashboard, for what a page measures.
    fn series(&self, part: &str) -> String {
        let mut matchers = Vec::new();
        if self.instrument.spec().labels.contains(&&CLIENT) {
            matchers.push(format!("{}=~\"$client\"", CLIENT.key));
        }
        if let Some((label, value)) = self.only {
            matchers.push(format!("{}=\"{value}\"", label.key));
        }
        let name = self.name(part);
        if matchers.is_empty() {
            name
        } else {
            format!("{name}{{{}}}", matchers.join(", "))
        }
    }

    /// The query, as Prometheus reads it, over `window`.
    fn expr(&self, window: &str) -> String {
        let keys = |first: Option<&'static str>| {
            let keys: Vec<_> = first
                .into_iter()
                .chain(self.by.iter().map(|label| label.key))
                .collect();
            if keys.is_empty() {
                String::new()
            } else {
                format!(" by ({}) ", keys.join(", "))
            }
        };
        let rate =
            |part: &str, by: String| format!("sum{by}(rate({}[{window}]))", self.series(part));
        match self.measure {
            Measure::Quantile(of_hundred) => format!(
                "histogram_quantile({}, {})",
                f64::from(of_hundred) / 100.0,
                rate("bucket", keys(Some("le")))
            ),
            Measure::Total => rate("sum", keys(None)),
            Measure::Events => rate("count", keys(None)),
        }
    }

    /// What Grafana calls each line of the query.
    fn legend(&self) -> String {
        let mut words: Vec<String> = Vec::new();
        words.extend(self.named.map(str::to_owned));
        words.extend(self.only.map(|(_, value)| value.to_owned()));
        words.extend(self.by.iter().map(|label| format!("{{{{{}}}}}", label.key)));
        if let Measure::Quantile(of_hundred) = self.measure {
            words.push(format!("p{of_hundred}"));
        }
        words.join(" ")
    }

    /// The unit Grafana shows its values in.
    fn unit(&self) -> &'static str {
        match (self.measure, self.instrument.spec().unit) {
            (Measure::Quantile(_), Unit::Seconds) => "s",
            (Measure::Quantile(_), Unit::Bytes) => "bytes",
            (Measure::Total, Unit::Seconds) => "percentunit",
            (Measure::Total, Unit::Bytes) => "Bps",
            (Measure::Quantile(_) | Measure::Total, Unit::Count) => "none",
            (Measure::Events, _) => "ops",
        }
    }
}

/// How a panel draws its queries.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// A bar for each query, over the whole of the time the dashboard
    /// shows: for what happens too seldom to make a line.
    Bars,
    /// A line for each, over time.
    Lines,
    /// The lines one upon the other, so that their top is their sum.
    Stacked,
}

/// A panel: its queries are in one unit, the first's.
struct Panel {
    title: &'static str,
    shape: Shape,
    /// How many of a wide screen's 24 columns it takes: a phone's screen
    /// gives each panel its whole width.
    wide: u8,
    queries: Vec<Query>,
}

impl Panel {
    /// How many of the grid's lines it is high: enough for a line of
    /// its own for each bar.
    fn high(&self) -> usize {
        match self.shape {
            Shape::Bars => HIGH.max(self.queries.len() + 1),
            Shape::Lines | Shape::Stacked => HIGH,
        }
    }
}

/// A row of panels under a title, which starts open or folded.
struct Row {
    title: &'static str,
    open: bool,
    panels: Vec<Panel>,
}

const P50: Measure = Measure::Quantile(50);
const P95: Measure = Measure::Quantile(95);

/// The two quantiles of `instrument` for each value of `phase`, in the
/// order the page reaches them.
fn by_phase(instrument: Instrument, phase: &'static Label) -> Vec<Query> {
    phase
        .values
        .iter()
        .flat_map(|value| {
            [P50, P95].map(|measure| Query::of(instrument, measure, &[]).only(phase, value))
        })
        .collect()
}

/// The two quantiles of `instrument`, for each value of `by`.
fn spread(instrument: Instrument, by: &[&'static Label]) -> Vec<Query> {
    vec![
        Query::of(instrument, P50, by),
        Query::of(instrument, P95, by),
    ]
}

/// The rows of the dashboard: what the human asks first on top, open,
/// and under it every other instrument, folded.
#[expect(
    clippy::too_many_lines,
    reason = "it is the table of the panels, a line or two for each"
)]
fn rows() -> Vec<Row> {
    use Instrument as I;
    let panel = |title, shape, wide, queries| Panel {
        title,
        shape,
        wide,
        queries,
    };
    vec![
        Row {
            title: "At a glance",
            open: true,
            panels: vec![
                panel(
                    "Cold start: how long until each phase",
                    Shape::Bars,
                    12,
                    by_phase(I::PageColdStart, &COLD_PHASE),
                ),
                panel(
                    "Opening a chat: how long after the tap until each phase",
                    Shape::Bars,
                    12,
                    by_phase(I::PageChatOpen, &OPEN_PHASE),
                ),
                panel(
                    "What coxd tells the phone, by kind of event",
                    Shape::Stacked,
                    8,
                    vec![Query::of(I::SseEventSize, Measure::Total, &[&TOLD])],
                ),
                panel(
                    "How busy coxd's one thread is",
                    Shape::Stacked,
                    8,
                    vec![
                        Query::of(I::HearDuration, Measure::Total, &[]).named("hearing"),
                        Query::of(I::SettleDuration, Measure::Total, &[]).named("settling"),
                    ],
                ),
                panel(
                    "How long the page's script takes over a draw",
                    Shape::Lines,
                    8,
                    spread(I::PageDrawDuration, &[&VIEW]),
                ),
            ],
        },
        Row {
            title: "The page, as the phone measures it",
            open: false,
            panels: vec![
                panel(
                    "Cold start, over time",
                    Shape::Lines,
                    12,
                    vec![Query::of(I::PageColdStart, P95, &[&COLD_PHASE])],
                ),
                panel(
                    "Opening a chat, over time",
                    Shape::Lines,
                    12,
                    vec![Query::of(I::PageChatOpen, P95, &[&OPEN_PHASE])],
                ),
                panel(
                    "Fetching the page's files",
                    Shape::Lines,
                    8,
                    vec![Query::of(I::PageResourceDuration, P95, &[&FILE])],
                ),
                panel(
                    "From a draw's start until it is painted",
                    Shape::Lines,
                    8,
                    spread(I::PageDrawToPaint, &[&VIEW]),
                ),
                panel(
                    "Draws in a second",
                    Shape::Lines,
                    8,
                    vec![Query::of(I::PageDrawDuration, Measure::Events, &[&VIEW])],
                ),
                panel(
                    "Parsing an event",
                    Shape::Lines,
                    12,
                    vec![Query::of(I::PageSseParseDuration, P95, &[&HEARD])],
                ),
                panel(
                    "How late the page's timers fire",
                    Shape::Lines,
                    12,
                    spread(I::PageLag, &[]),
                ),
            ],
        },
        Row {
            title: "coxd's one thread",
            open: false,
            panels: vec![
                panel(
                    "Share of the thread, by what it heard",
                    Shape::Stacked,
                    12,
                    vec![Query::of(I::HearDuration, Measure::Total, &[&EVENT])],
                ),
                panel(
                    "How long one thing it heard takes",
                    Shape::Lines,
                    12,
                    vec![Query::of(I::HearDuration, P95, &[&EVENT])],
                ),
                panel(
                    "Settling after what it heard",
                    Shape::Lines,
                    8,
                    spread(I::SettleDuration, &[]),
                ),
                panel(
                    "How late the thread comes back to a timer",
                    Shape::Lines,
                    8,
                    spread(I::LoopLag, &[]),
                ),
                panel(
                    "Reading a closed chat as it is opened",
                    Shape::Lines,
                    4,
                    spread(I::ChatReadDuration, &[]),
                ),
                panel(
                    "Entries of a closed chat as it is opened",
                    Shape::Lines,
                    4,
                    spread(I::ChatReadEntries, &[]),
                ),
            ],
        },
        Row {
            title: "coxd's answers to the phone",
            open: false,
            panels: vec![
                panel(
                    "How long an answer takes, by route",
                    Shape::Lines,
                    8,
                    vec![Query::of(I::HttpDuration, P95, &[&ROUTE])],
                ),
                panel(
                    "Answers sent, by route",
                    Shape::Stacked,
                    8,
                    vec![Query::of(I::HttpResponseSize, Measure::Total, &[&ROUTE])],
                ),
                panel(
                    "Requests in a second, by route and status",
                    Shape::Lines,
                    8,
                    vec![Query::of(
                        I::HttpDuration,
                        Measure::Events,
                        &[&ROUTE, &STATUS],
                    )],
                ),
                panel(
                    "Waiting for the thread's answer, by what was asked",
                    Shape::Lines,
                    12,
                    vec![Query::of(I::AskWait, P95, &[&ASK])],
                ),
                panel(
                    "Size of an event",
                    Shape::Lines,
                    12,
                    vec![Query::of(I::SseEventSize, P95, &[&TOLD])],
                ),
                panel(
                    "Writing an event as JSON",
                    Shape::Lines,
                    12,
                    vec![Query::of(I::SseEncodeDuration, P95, &[&TOLD])],
                ),
                panel(
                    "Events waiting behind the one told",
                    Shape::Lines,
                    12,
                    spread(I::SseQueue, &[]),
                ),
            ],
        },
    ]
}

/// A panel as Grafana reads it, with the number `id`, at `x` and `y` of
/// the grid, `high` of its lines high.
fn drawn(panel: &Panel, id: usize, x: u8, y: usize, high: usize) -> Value {
    let bars = panel.shape == Shape::Bars;
    let window = if bars { "$__range" } else { "$__rate_interval" };
    let targets: Vec<_> = ('A'..='Z')
        .zip(&panel.queries)
        .map(|(letter, query)| {
            json!({
                "refId": letter.to_string(),
                "datasource": prometheus(),
                "expr": query.expr(window),
                "legendFormat": query.legend(),
                "instant": bars,
                "range": !bars,
            })
        })
        .collect();
    assert_eq!(targets.len(), panel.queries.len(), "{}", panel.title);
    let unit = panel.queries[0].unit();
    assert!(
        panel.queries.iter().all(|query| query.unit() == unit),
        "{} mixes units",
        panel.title
    );
    let (kind, custom, options) = if bars {
        (
            "bargauge",
            json!({}),
            json!({
                "orientation": "horizontal",
                "displayMode": "basic",
                "valueMode": "text",
                "namePlacement": "left",
                "showUnfilled": true,
                "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": false},
            }),
        )
    } else {
        let stacked = panel.shape == Shape::Stacked;
        (
            "timeseries",
            json!({
                "showPoints": "auto",
                "lineWidth": 1,
                "fillOpacity": if stacked { 30 } else { 5 },
                "stacking": {"mode": if stacked { "normal" } else { "none" }},
            }),
            json!({
                "legend": {"displayMode": "list", "placement": "bottom"},
                "tooltip": {"mode": "multi", "sort": "desc"},
            }),
        )
    };
    json!({
        "id": id,
        "type": kind,
        "title": panel.title,
        "datasource": prometheus(),
        "gridPos": {"h": high, "w": panel.wide, "x": x, "y": y},
        "fieldConfig": {
            "defaults": {
                "unit": unit,
                "min": 0,
                "color": if bars {
                    json!({"mode": "fixed", "fixedColor": "blue"})
                } else {
                    json!({"mode": "palette-classic"})
                },
                "custom": custom,
            },
            "overrides": [],
        },
        "options": options,
        "targets": targets,
    })
}

/// How many of the grid's lines a panel is high, at the least.
const HIGH: usize = 8;

/// The dashboard as Grafana reads it.
fn dashboard() -> Value {
    let mut panels = Vec::new();
    let (mut id, mut y) = (0, 0);
    for row in rows() {
        id += 1;
        let (head, at) = (id, y);
        y += 1;
        let mut inside = Vec::new();
        let mut rest = row.panels.as_slice();
        while !rest.is_empty() {
            // The panels that fit side by side, all as high as the
            // highest, or Grafana moves the next ones up beside it.
            let mut wide = 0;
            let fit = rest
                .iter()
                .take_while(|panel| {
                    wide += panel.wide;
                    wide <= 24
                })
                .count()
                .max(1);
            let (line, after) = rest.split_at(fit);
            let high = line.iter().map(Panel::high).max().unwrap_or(HIGH);
            let mut x = 0;
            for panel in line {
                id += 1;
                inside.push(drawn(panel, id, x, y, high));
                x += panel.wide;
            }
            y += high;
            rest = after;
        }
        let mut row_drawn = json!({
            "id": head,
            "type": "row",
            "title": row.title,
            "collapsed": !row.open,
            "gridPos": {"h": 1, "w": 24, "x": 0, "y": at},
            "panels": [],
        });
        if row.open {
            panels.push(row_drawn);
            panels.append(&mut inside);
        } else {
            row_drawn["panels"] = Value::Array(inside);
            panels.push(row_drawn);
        }
    }
    let clients = format!(
        "label_values({}, {})",
        Query::of(Instrument::PageLag, Measure::Events, &[]).name("count"),
        CLIENT.key
    );
    json!({
        "uid": "cox-phone",
        "title": "Cox: the phone's page",
        "tags": ["cox"],
        "schemaVersion": 39,
        "editable": false,
        "timezone": "browser",
        "time": {"from": "now-6h", "to": "now"},
        "refresh": "1m",
        "templating": {"list": [{
            "name": "client",
            "label": "Phone",
            "type": "query",
            "datasource": prometheus(),
            "query": {"qryType": 1, "query": clients, "refId": "client"},
            "refresh": 2,
            "includeAll": true,
            "allValue": ".*",
            "multi": true,
            "current": {"text": "All", "value": "$__all"},
        }]},
        "panels": panels,
    })
}

#[test]
fn the_dashboard_in_the_repository_is_what_the_table_of_instruments_makes() {
    let made = dashboard();
    if std::env::var_os(WRITE).is_some() {
        let mut text = serde_json::to_string_pretty(&made).unwrap();
        text.push('\n');
        std::fs::write(PATH, text).unwrap();
        return;
    }
    // As values: a feature of serde_json that any crate of the build may
    // turn on sets the order an object's keys are written in.
    assert!(
        serde_json::from_str::<Value>(KEPT).ok() == Some(made),
        "telemetry/grafana/dashboards/cox-phone.json is not what \
         crates/coxd/src/telemetry/dashboard.rs makes: write it again with \
         `{WRITE}=1 cargo test -p coxd telemetry::dashboard`"
    );
}

#[test]
fn the_dashboard_shows_every_instrument() {
    let shown: Vec<_> = rows()
        .iter()
        .flat_map(|row| &row.panels)
        .flat_map(|panel| &panel.queries)
        .map(|query| query.instrument)
        .collect();
    for instrument in Instrument::ALL {
        assert!(
            shown.contains(instrument),
            "no panel shows {}",
            instrument.spec().name
        );
    }
}

#[test]
fn a_query_asks_prometheus_by_the_names_it_keeps_an_instrument_by() {
    let phase = Query::of(Instrument::PageColdStart, P95, &[]).only(&COLD_PHASE, "drawn");
    assert_eq!(
        phase.expr("5m"),
        "histogram_quantile(0.95, sum by (le) (rate(\
         cox_phone_page_cold_start_seconds_bucket{client=~\"$client\", phase=\"drawn\"}[5m])))"
    );
    assert_eq!(phase.legend(), "drawn p95");
    let sent = Query::of(Instrument::SseEventSize, Measure::Total, &[&TOLD]);
    assert_eq!(
        sent.expr("5m"),
        "sum by (kind) (rate(cox_phone_sse_event_size_bytes_sum[5m]))"
    );
    assert_eq!(sent.legend(), "{{kind}}");
    assert_eq!(sent.unit(), "Bps");
    let waiting = Query::of(Instrument::SseQueue, P50, &[]);
    assert_eq!(
        waiting.expr("5m"),
        "histogram_quantile(0.5, sum by (le) (rate(cox_phone_sse_queue_bucket[5m])))"
    );
}
