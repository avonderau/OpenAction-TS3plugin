mod support;

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use support::{MockTs3, Ts3, API_KEY};
use ts3deck::clientquery::{spawn, TsEvent};
use ts3deck::plugin::Plugin;

const MIC: &str = "com.avonderau.ts3deck.mic";
const PTT: &str = "com.avonderau.ts3deck.ptt";
const CHANNEL: &str = "com.avonderau.ts3deck.channel";
const STATUS: &str = "com.avonderau.ts3deck.status";
const AWAY: &str = "com.avonderau.ts3deck.away";

/// Runs a Plugin against the real worker and records what it sends.
struct Harness {
    plugin: Plugin,
    events: Receiver<TsEvent>,
    sent: Vec<Value>,
}

impl Harness {
    fn new() -> Harness {
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        spawn(rx, move |e| {
            let _ = etx.send(e);
        });
        let mut plugin = Plugin::new("token".into(), tx);
        plugin.on_open();
        let mut h = Harness {
            plugin,
            events: erx,
            sent: Vec::new(),
        };
        h.flush();
        h
    }

    fn flush(&mut self) {
        self.sent.append(&mut self.plugin.outbox);
    }

    fn deck(&mut self, msg: Value) {
        self.plugin.on_deck(&msg);
        self.flush();
    }

    fn appear(&mut self, action: &str, context: &str, settings: Value) {
        self.deck(json!({"event": "willAppear", "action": action, "context": context, "payload": {"settings": settings}}));
    }

    fn key(&mut self, event: &str, context: &str) {
        self.deck(json!({"event": event, "context": context}));
    }

    fn last(&self, event: &str, context: &str) -> Option<&Value> {
        self.sent
            .iter()
            .rev()
            .find(|m| m["event"] == event && m["context"] == context)
            .map(|m| &m["payload"])
    }

    fn title(&self, context: &str) -> String {
        self.last("setTitle", context)
            .and_then(|p| p["title"].as_str())
            .unwrap_or("")
            .to_string()
    }

    fn state(&self, context: &str) -> i64 {
        self.last("setState", context)
            .and_then(|p| p["state"].as_i64())
            .unwrap_or(-1)
    }

    /// Feeds TeamSpeak events until `pred` holds.
    fn until(&mut self, what: &str, pred: impl Fn(&Harness) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !pred(self) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; sent: {:#?}",
                self.sent
            );
            if let Ok(e) = self.events.recv_timeout(Duration::from_millis(50)) {
                self.plugin.on_ts(e);
                self.flush();
            }
        }
    }
}

#[test]
fn keys_drive_teamspeak_and_follow_its_state() {
    let ts3 = MockTs3::start(Ts3::default());
    let mut h = Harness::new();
    assert!(h
        .sent
        .iter()
        .any(|m| m["event"] == "getGlobalSettings" && m["context"] == "token"));

    h.appear(MIC, "k-mic", json!({}));
    assert_eq!(
        h.title("k-mic"),
        "TS3\n…",
        "no 'Set API key' flash before settings arrive"
    );

    h.deck(json!({"event": "didReceiveGlobalSettings", "payload": {"settings": {"port": ts3.port, "apiKey": API_KEY}}}));
    h.until("mic title", |h| h.title("k-mic") == "Mic on");

    h.appear(CHANNEL, "k-ch", json!({"channel": "Games/Squad 1"}));
    h.appear(STATUS, "k-st", json!({}));
    h.appear(PTT, "k-ptt", json!({}));
    h.appear(AWAY, "k-away", json!({"message": "lunch"}));
    assert_eq!(h.title("k-st"), "Lobby\n2 users");
    assert_eq!(h.title("k-ch"), "Squad 1");
    assert_eq!(h.state("k-ch"), 0);

    h.key("keyDown", "k-mic");
    h.key("keyUp", "k-mic");
    h.until("mic muted", |h| h.state("k-mic") == 1);
    assert_eq!(ts3.var("client_input_muted"), "1");

    h.key("keyDown", "k-ch");
    h.until("joined channel", |h| h.state("k-ch") == 1);
    assert_eq!(h.title("k-st"), "Squad 1\n2 users");

    h.key("keyDown", "k-away");
    h.until("away", |h| h.state("k-away") == 1);
    assert_eq!(ts3.var("client_away_message"), "lunch");

    h.key("keyDown", "k-ptt");
    assert_eq!(h.title("k-ptt"), "TALK");
    h.key("keyUp", "k-ptt");
    h.until("ptt released", |_| {
        ts3.var("client_input_deactivated") == "1"
    });
    h.until("ptt title", |h| h.title("k-ptt") == "PTT");

    // A held PTT key that leaves the page closes the mic.
    h.key("keyDown", "k-ptt");
    h.until("ptt down", |_| ts3.var("client_input_deactivated") == "0");
    h.deck(json!({"event": "willDisappear", "context": "k-ptt"}));
    h.until("ptt closed", |_| ts3.var("client_input_deactivated") == "1");

    // A failing action shows the alert mark on its key.
    h.deck(json!({"event": "didReceiveSettings", "context": "k-ch", "payload": {"settings": {"channel": "Nowhere"}}}));
    h.key("keyDown", "k-ch");
    h.until("alert", |h| {
        h.sent
            .iter()
            .any(|m| m["event"] == "showAlert" && m["context"] == "k-ch")
    });
}

#[test]
fn settings_page_saves_the_connection_without_losing_the_key() {
    let ts3 = MockTs3::start(Ts3::default());
    let mut h = Harness::new();
    h.deck(json!({"event": "didReceiveGlobalSettings", "payload": {"settings": {}}}));
    h.appear(MIC, "k", json!({}));
    h.deck(json!({"event": "propertyInspectorDidAppear", "action": MIC, "context": "k"}));
    h.until("nokey", |h| {
        h.last("sendToPropertyInspector", "k")
            .is_some_and(|p| p["status"] == "nokey")
    });

    let port = ts3.port.to_string();
    h.deck(
        json!({"event": "sendToPlugin", "action": MIC, "context": "k",
        "payload": {"type": "saveConnection", "host": "", "port": port, "apiKey": API_KEY}}),
    );
    let saved = h
        .sent
        .iter()
        .rev()
        .find(|m| m["event"] == "setGlobalSettings")
        .unwrap()["payload"]
        .clone();
    assert_eq!(
        saved,
        json!({"host": "127.0.0.1", "port": ts3.port, "apiKey": API_KEY})
    );
    h.until("connected", |h| {
        h.last("sendToPropertyInspector", "k")
            .is_some_and(|p| p["status"] == "connected")
    });

    // Saving with an empty key field keeps the saved key.
    h.deck(
        json!({"event": "sendToPlugin", "action": MIC, "context": "k",
        "payload": {"type": "saveConnection", "host": "127.0.0.1", "port": port}}),
    );
    let saved = h
        .sent
        .iter()
        .rev()
        .find(|m| m["event"] == "setGlobalSettings")
        .unwrap()["payload"]
        .clone();
    assert_eq!(saved["apiKey"], API_KEY);

    h.deck(json!({"event": "sendToPlugin", "action": MIC, "context": "k", "payload": {"type": "channels"}}));
    let channels = h.last("sendToPropertyInspector", "k").unwrap()["channels"].clone();
    assert!(channels
        .as_array()
        .unwrap()
        .contains(&json!("Games/Squad 2")));
}
