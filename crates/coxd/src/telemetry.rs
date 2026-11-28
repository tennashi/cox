//! What coxd and the phone's page measure of themselves, so that the
//! human sees where the phone's page is slow without being the one who
//! notices it: sent as OpenTelemetry metrics over OTLP/HTTP to the
//! endpoint `settings.json` names, as `{"telemetry": {"endpoint":
//! "http://127.0.0.1:4318"}}`, and to nowhere unless it names one. coxd
//! reads the setting as it starts.
//!
//! Every instrument is a histogram, and [`instruments`] is the table of
//! them: coxd records by [`Instrument`] on a [`Telemetry`]. Unless the
//! settings name an endpoint, that is [`Telemetry::off`], which records
//! nothing, opens no connection and starts no thread.
//!
//! The OpenTelemetry SDK's periodic reader collects what was recorded and
//! sends it on a thread of its own, once a minute, or as often as
//! `OTEL_METRIC_EXPORT_INTERVAL` says, in milliseconds, and [`Ureq`]
//! sends from that thread: nothing of it runs on coxd's one thread but
//! the recording itself.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Histogram, MeterProvider as _};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_otlp::{MetricExporter, WithExportConfig as _, WithHttpConfig as _};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use serde_json::Value;

#[cfg(test)]
mod dashboard;
pub mod instruments;

pub use instruments::Instrument;

/// The key of `settings.json` that turns the telemetry on.
const SETTING: &str = "telemetry";

/// Where under the endpoint OTLP/HTTP takes metrics.
const METRICS: &str = "/v1/metrics";

/// How long a send may take before it is given up, so that an endpoint
/// that does not answer holds neither the reader's thread nor coxd's
/// stop for long.
const SEND_WITHIN: Duration = Duration::from_secs(5);

/// The most measurements of one report of the phone's page that are
/// read: the page reports far fewer every few seconds.
pub const MOST_REPORTED: usize = 500;

/// Where coxd's measurements go: to an OTLP endpoint, or nowhere. A
/// clone records to the same place.
#[derive(Clone, Default)]
pub struct Telemetry(Option<Arc<Sending>>);

/// What sends the measurements: the SDK's provider, whose reader has the
/// thread, and a histogram for each instrument, in the order of
/// [`Instrument::ALL`].
struct Sending {
    provider: SdkMeterProvider,
    histograms: Vec<Histogram<f64>>,
}

impl Telemetry {
    /// Records nothing and sends nothing.
    #[must_use]
    pub fn off() -> Self {
        Self(None)
    }

    /// Sends where this Cox's `settings.json` says, or is off when it
    /// names no endpoint, or one that cannot be sent to.
    #[must_use]
    pub fn read() -> Self {
        Self::set_up(crate::settings::kept(SETTING).as_ref())
    }

    /// Sends where `setting`, what `settings.json` holds as `telemetry`,
    /// says, if it says.
    fn set_up(setting: Option<&Value>) -> Self {
        let endpoint = match endpoint_in(setting) {
            Ok(Some(endpoint)) => endpoint,
            Ok(None) => return Self::off(),
            Err(why) => {
                tracing::error!("\"telemetry\" in settings.json cannot be read: {why}");
                return Self::off();
            }
        };
        match Self::to(&endpoint) {
            Ok(telemetry) => {
                tracing::info!(endpoint, "coxd sends its telemetry over OTLP");
                telemetry
            }
            Err(error) => {
                tracing::error!("coxd's telemetry is not sent: {error:#}");
                Self::off()
            }
        }
    }

    /// Sends to the OTLP endpoint `endpoint`, as `http://127.0.0.1:4318`.
    fn to(endpoint: &str) -> anyhow::Result<Self> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(SEND_WITHIN))
            .build()
            .into();
        let exporter = MetricExporter::builder()
            .with_http()
            .with_http_client(Ureq {
                agent,
                failing: AtomicBool::new(false),
            })
            .with_endpoint(format!("{endpoint}{METRICS}"))
            .build()
            .with_context(|| format!("{endpoint} is no endpoint to send to"))?;
        let resource = Resource::builder()
            .with_service_name("coxd")
            .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
            .build();
        let provider = SdkMeterProvider::builder()
            .with_resource(resource)
            .with_periodic_exporter(exporter)
            .build();
        let meter = provider.meter("coxd");
        let histograms = Instrument::ALL
            .iter()
            .map(|instrument| {
                let spec = instrument.spec();
                meter
                    .f64_histogram(spec.name)
                    .with_unit(spec.unit.name())
                    .with_boundaries(spec.unit.buckets())
                    .build()
            })
            .collect();
        Ok(Self(Some(Arc::new(Sending {
            provider,
            histograms,
        }))))
    }

    /// Whether what is recorded is sent anywhere.
    #[must_use]
    pub fn on(&self) -> bool {
        self.0.is_some()
    }

    /// Records `value` on `instrument`, with the values of its labels,
    /// `labels`, in the order of its spec's. A value the table does not
    /// list is told as `other`, and a `value` no measurement can be is
    /// left out.
    pub fn record(&self, instrument: Instrument, value: f64, labels: &[&str]) {
        let Some(sending) = &self.0 else {
            return;
        };
        if !instrument.spec().unit.takes(value) {
            return;
        }
        let attributes: Vec<KeyValue> = instrument
            .labelled(labels)
            .map(|(key, value)| KeyValue::new(key, value))
            .collect();
        sending.histograms[instrument as usize].record(value, &attributes);
    }

    /// Records the time since `since` on `instrument`.
    pub fn seconds(&self, instrument: Instrument, since: Instant, labels: &[&str]) {
        if self.on() {
            self.record(instrument, since.elapsed().as_secs_f64(), labels);
        }
    }

    /// Records `count` things on `instrument`.
    pub fn count(&self, instrument: Instrument, count: usize, labels: &[&str]) {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a count or a size too large to be whole is left out anyway"
        )]
        self.record(instrument, count as f64, labels);
    }

    /// The answer coxd's main loop gives to `ask`, recording how long the
    /// phone's request waited for it.
    ///
    /// # Errors
    ///
    /// When the main loop went without answering.
    pub async fn waited<T>(
        &self,
        ask: &'static str,
        answer: tokio::sync::oneshot::Receiver<T>,
    ) -> Result<T, tokio::sync::oneshot::error::RecvError> {
        let since = Instant::now();
        let answer = answer.await;
        self.seconds(Instrument::AskWait, since, &[ask]);
        answer
    }

    /// Records how late coxd's one thread comes back to a timer, for as
    /// long as coxd runs.
    pub async fn watch_lag(self) {
        const EVERY: Duration = Duration::from_millis(100);
        loop {
            let due = Instant::now() + EVERY;
            tokio::time::sleep(EVERY).await;
            let late = Instant::now().saturating_duration_since(due);
            self.record(Instrument::LoopLag, late.as_secs_f64(), &[]);
        }
    }

    /// Records what a phone's page measured, as `body` reports it, from a
    /// phone of the kind `client`. A report is `{"m": [[name, value,
    /// label], ...]}`: the name the page says an instrument by (see
    /// [`Instrument::reported`]), its value, and the value of the label
    /// the instrument has beside `client`, or `""` for one that has none.
    /// A measurement of a name the table does not list is left out, as
    /// are those after the first [`MOST_REPORTED`].
    pub fn report(&self, body: &[u8], client: &str) {
        #[derive(serde::Deserialize)]
        struct Report {
            m: Vec<(String, f64, String)>,
        }
        if !self.on() {
            return;
        }
        let Ok(report) = serde_json::from_slice::<Report>(body) else {
            return;
        };
        for (name, value, label) in report.m.into_iter().take(MOST_REPORTED) {
            if let Some(instrument) = Instrument::reported(&name) {
                self.record(instrument, value, &[client, &label]);
            }
        }
    }

    /// Sends what is left and stops the reader's thread. It waits for the
    /// send, so it is not for a thread that others wait on.
    pub fn stop(&self) {
        if let Some(sending) = &self.0
            && let Err(error) = sending.provider.shutdown()
        {
            tracing::warn!("coxd's last telemetry was not sent: {error}");
        }
    }
}

/// The endpoint `setting`, what `settings.json` holds as `telemetry`,
/// names, without a `/` at its end; none when it holds nothing, or names
/// none.
fn endpoint_in(setting: Option<&Value>) -> Result<Option<String>, String> {
    let Some(setting) = setting.filter(|setting| !setting.is_null()) else {
        return Ok(None);
    };
    let Some(setting) = setting.as_object() else {
        return Err(format!("{setting} is not {{\"endpoint\": ...}}"));
    };
    match setting.get("endpoint") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(endpoint))
            if endpoint.starts_with("http://") || endpoint.starts_with("https://") =>
        {
            Ok(Some(endpoint.trim_end_matches('/').to_owned()))
        }
        Some(endpoint) => Err(format!(
            "{endpoint} is no endpoint, as \"http://127.0.0.1:4318\""
        )),
    }
}

/// Sends the exporter's requests with ureq, waiting on the thread that
/// asks, which is the periodic reader's own. It says in the log when the
/// endpoint stops taking them, and when it takes them again, once each.
#[derive(Debug)]
struct Ureq {
    agent: ureq::Agent,
    failing: AtomicBool,
}

impl Ureq {
    fn send(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let (parts, body) = request.into_parts();
        let response = self.agent.run(Request::from_parts(parts, body.to_vec()))?;
        let (parts, mut body) = response.into_parts();
        let bytes = body.read_to_vec()?;
        Ok(Response::from_parts(parts, Bytes::from(bytes)))
    }
}

#[async_trait::async_trait]
impl HttpClient for Ureq {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let endpoint = request.uri().to_string();
        let sent = self.send(request);
        let refused = match &sent {
            Ok(response) if response.status().is_success() => None,
            Ok(response) => Some(format!("it answered {}", response.status())),
            Err(error) => Some(error.to_string()),
        };
        let failed = self.failing.swap(refused.is_some(), Ordering::Relaxed);
        match refused {
            Some(why) if !failed => {
                tracing::warn!("coxd's telemetry is not taken at {endpoint}: {why}");
            }
            None if failed => tracing::info!("coxd's telemetry is taken at {endpoint} again"),
            _ => {}
        }
        sent
    }
}

/// An OTLP endpoint of a test's own, which keeps what is sent to it.
#[cfg(test)]
pub(crate) mod testing {
    use std::collections::BTreeMap;
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::mpsc::{Receiver, channel};

    use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
    use opentelemetry_proto::tonic::common::v1::any_value;
    use opentelemetry_proto::tonic::metrics::v1::metric::Data;
    use prost::Message as _;
    use serde_json::json;

    use super::Telemetry;

    /// What one series of an instrument holds as it was sent: its name
    /// and unit, its labels, and how many measurements it counts.
    #[derive(Debug, PartialEq)]
    pub(crate) struct Series {
        pub(crate) name: String,
        pub(crate) unit: String,
        pub(crate) labels: BTreeMap<String, String>,
        pub(crate) count: u64,
    }

    /// The endpoint: what was posted to it, as the path and the body of
    /// each request.
    pub(crate) struct Collector {
        port: u16,
        posted: Receiver<(String, Vec<u8>)>,
    }

    impl Collector {
        /// Listens on a port of its own, on a thread that ends with the
        /// test.
        pub(crate) fn start() -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let (to, posted) = channel();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut head = String::new();
                    let mut length = 0;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = line.split_once(':')
                            && name.eq_ignore_ascii_case("content-length")
                        {
                            length = value.trim().parse().unwrap();
                        }
                        head.push_str(&line);
                    }
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    let path = head.split(' ').nth(1).unwrap_or_default().to_owned();
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    if to.send((path, body)).is_err() {
                        break;
                    }
                }
            });
            Self { port, posted }
        }

        /// A telemetry that sends here, set up as `settings.json` would
        /// have it.
        pub(crate) fn telemetry(&self) -> Telemetry {
            let endpoint = format!("http://127.0.0.1:{}/", self.port);
            Telemetry::set_up(Some(&json!({ "endpoint": endpoint })))
        }

        /// How many requests came that were not read yet.
        pub(crate) fn requests(&self) -> usize {
            self.posted.try_iter().count()
        }

        /// The series `telemetry` sent here by the time it stopped, which
        /// stops it.
        pub(crate) fn sent_by(&self, telemetry: &Telemetry) -> Vec<Series> {
            telemetry.stop();
            let mut series = Vec::new();
            for (path, body) in self.posted.try_iter() {
                assert_eq!(path, "/v1/metrics");
                let request = ExportMetricsServiceRequest::decode(body.as_slice()).unwrap();
                let metrics = request
                    .resource_metrics
                    .into_iter()
                    .flat_map(|resource| resource.scope_metrics)
                    .flat_map(|scope| scope.metrics);
                for metric in metrics {
                    let Some(Data::Histogram(histogram)) = metric.data else {
                        panic!("{} is no histogram", metric.name);
                    };
                    for point in histogram.data_points {
                        let labels = point
                            .attributes
                            .into_iter()
                            .map(|label| match label.value.and_then(|value| value.value) {
                                Some(any_value::Value::StringValue(value)) => (label.key, value),
                                other => panic!("{} is {other:?}", label.key),
                            })
                            .collect();
                        series.push(Series {
                            name: metric.name.clone(),
                            unit: metric.unit.clone(),
                            labels,
                            count: point.count,
                        });
                    }
                }
            }
            series
        }
    }

    /// The labels of a series, as a test writes them.
    pub(crate) fn labels(labels: &[(&str, &str)]) -> BTreeMap<String, String> {
        labels
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::testing::{Collector, Series, labels};
    use super::*;

    #[test]
    fn sends_what_is_recorded_to_the_endpoint_the_settings_name() {
        let collector = Collector::start();
        let telemetry = collector.telemetry();
        assert!(telemetry.on());

        telemetry.record(Instrument::HttpDuration, 0.02, &["/media/*", "200"]);
        telemetry.record(Instrument::HttpDuration, 0.03, &["/media/*", "200"]);
        telemetry.record(Instrument::HttpDuration, 0.5, &["/nowhere", "418"]);
        telemetry.record(Instrument::SettleDuration, f64::NAN, &[]);

        let mut sent = collector.sent_by(&telemetry);
        sent.sort_by_key(|series| series.count);
        assert_eq!(
            sent,
            [
                Series {
                    name: "cox.phone.http.duration".to_owned(),
                    unit: "s".to_owned(),
                    labels: labels(&[("route", "other"), ("status", "other")]),
                    count: 1,
                },
                Series {
                    name: "cox.phone.http.duration".to_owned(),
                    unit: "s".to_owned(),
                    labels: labels(&[("route", "/media/*"), ("status", "200")]),
                    count: 2,
                },
            ]
        );
    }

    #[test]
    fn takes_of_a_pages_report_what_the_table_lists() {
        let collector = Collector::start();
        let telemetry = collector.telemetry();
        let report = json!({"m": [
            ["cold", 0.8, "drawn"],
            ["cold", 0.9, "made-up"],
            ["draw", 0.01, "chat"],
            ["lag", 0.004, ""],
            ["cox.daemon.settle.duration", 1.0, ""],
            ["nothing", 1.0, ""],
            ["draw", -1.0, "chat"],
            ["draw", 1e300, "chat"],
        ]});

        telemetry.report(report.to_string().as_bytes(), "ios");
        telemetry.report(br#"{"m": [["draw"]]}"#, "ios");
        telemetry.report(b"not json", "ios");

        let mut sent: Vec<_> = collector
            .sent_by(&telemetry)
            .into_iter()
            .map(|series| (series.name, series.labels, series.count))
            .collect();
        sent.sort();
        assert_eq!(
            sent,
            [
                (
                    "cox.phone.page.cold_start".to_owned(),
                    labels(&[("client", "ios"), ("phase", "drawn")]),
                    1
                ),
                (
                    "cox.phone.page.cold_start".to_owned(),
                    labels(&[("client", "ios"), ("phase", "other")]),
                    1
                ),
                (
                    "cox.phone.page.draw.duration".to_owned(),
                    labels(&[("client", "ios"), ("view", "chat")]),
                    1
                ),
                (
                    "cox.phone.page.lag".to_owned(),
                    labels(&[("client", "ios")]),
                    1
                ),
            ]
        );
    }

    #[test]
    fn reads_no_more_of_a_report_than_a_page_would_send() {
        let collector = Collector::start();
        let telemetry = collector.telemetry();
        let many: Vec<_> = (0..MOST_REPORTED + 100)
            .map(|_| json!(["draw", 0.01, "home"]))
            .collect();

        telemetry.report(json!({ "m": many }).to_string().as_bytes(), "android");

        let sent = collector.sent_by(&telemetry);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].count, MOST_REPORTED as u64);
    }

    #[test]
    fn sends_nothing_unless_the_settings_name_an_endpoint() {
        let collector = Collector::start();
        for setting in [None, Some(json!({})), Some(json!({"endpoint": null}))] {
            let telemetry = Telemetry::set_up(setting.as_ref());
            assert!(!telemetry.on(), "{setting:?}");
            telemetry.record(Instrument::SettleDuration, 0.01, &[]);
            telemetry.report(br#"{"m": [["draw", 0.01, "home"]]}"#, "ios");
            telemetry.stop();
        }
        assert_eq!(collector.requests(), 0);
    }

    #[test]
    fn reads_the_endpoint_from_the_settings() {
        assert_eq!(endpoint_in(None), Ok(None));
        assert_eq!(
            endpoint_in(Some(&json!({"endpoint": "http://127.0.0.1:4318/"}))),
            Ok(Some("http://127.0.0.1:4318".to_owned()))
        );
        assert_eq!(
            endpoint_in(Some(&json!({"endpoint": "https://otlp.example"}))),
            Ok(Some("https://otlp.example".to_owned()))
        );
        assert!(endpoint_in(Some(&json!({"endpoint": "127.0.0.1:4318"}))).is_err());
        assert!(endpoint_in(Some(&json!({"endpoint": 4318}))).is_err());
        assert!(endpoint_in(Some(&json!(true))).is_err());
    }
}
