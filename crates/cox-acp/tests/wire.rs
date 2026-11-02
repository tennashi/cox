//! What Cox and the agent say to each other is said for the logs, in the
//! span the agent was launched in.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

/// A line said: its target, whose it was, what it said, and the spans it
/// was said in.
#[derive(Debug, Default)]
struct Said {
    target: String,
    from: String,
    message: String,
    spans: Vec<String>,
}

impl Visit for Said {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.message = format!("{value:?}"),
            "from" => self.from = format!("{value:?}"),
            _ => {}
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "from" {
            value.clone_into(&mut self.from);
        }
    }
}

/// Notes each line said.
struct Heard(Arc<Mutex<Vec<Said>>>);

impl<S: Subscriber + for<'span> LookupSpan<'span>> Layer<S> for Heard {
    fn on_event(&self, event: &Event<'_>, cx: Context<'_, S>) {
        let mut said = Said {
            target: event.metadata().target().to_owned(),
            ..Said::default()
        };
        event.record(&mut said);
        said.spans = cx
            .event_scope(event)
            .into_iter()
            .flatten()
            .map(|span| span.name().to_owned())
            .collect();
        self.0.lock().unwrap().push(said);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn what_goes_between_cox_and_the_agent_is_said_in_its_span() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    // The agent's side runs on other threads, so this is for all of them.
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry().with(Heard(Arc::clone(&heard))),
    )
    .unwrap();
    // An agent that says something on its standard error, answers Cox's
    // first request, and goes.
    let script = r#"echo starting >&2
read request
id=$(printf '%s' "$request" | sed 's/.*"id":\([0-9]*\).*/\1/')
echo '{"jsonrpc":"2.0","id":'"$id"',"result":{"protocolVersion":1,"agentCapabilities":{}}}'
sleep 1"#;

    let agent = {
        let span = tracing::info_span!("chat", id = 3);
        let _entered = span.enter();
        cox_acp::launch(["sh".to_owned(), "-c".to_owned(), script.to_owned()]).unwrap()
    };
    let connected = cox_acp::connect(
        agent,
        std::env::temp_dir(),
        Vec::new(),
        None,
        |_| {},
        async |_| Ok(()),
    );
    let _ = tokio::time::timeout(Duration::from_secs(5), connected).await;

    let heard = heard.lock().unwrap();
    let find = |target: &str, from: &str, says: &str| {
        heard
            .iter()
            .find(|said| said.target == target && said.from == from && said.message.contains(says))
            .unwrap_or_else(|| panic!("no {target} {from} line with {says}: {heard:#?}"))
    };
    let sent = find(cox_acp::WIRE, "cox", "\"method\":\"initialize\"");
    assert_eq!(sent.spans, ["chat"]);
    let answered = find(cox_acp::WIRE, "agent", "\"protocolVersion\":1");
    assert_eq!(answered.spans, ["chat"]);
    let stderr = find(cox_acp::STDERR, "", "starting");
    assert_eq!(stderr.spans, ["chat"]);
}
