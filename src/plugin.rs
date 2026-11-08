//! Key logic: turns Deck events into TeamSpeak requests and TeamSpeak state
//! into key titles, states and settings-page updates. It does no I/O; the
//! messages to send to the Deck host collect in [`Plugin::outbox`].

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use serde_json::{json, Value};

use crate::clientquery::{
    find_channel, Command, Config, Request, Status, TsEvent, TsState, DEFAULT_PORT,
};

pub const PLUGIN: &str = "com.avonderau.ts3deck";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Mic,
    Speakers,
    Away,
    Ptt,
    Channel,
    Status,
}

impl Action {
    pub fn from_uuid(uuid: &str) -> Option<Action> {
        Some(match uuid.strip_prefix(PLUGIN)?.strip_prefix('.')? {
            "mic" => Action::Mic,
            "speakers" => Action::Speakers,
            "away" => Action::Away,
            "ptt" => Action::Ptt,
            "channel" => Action::Channel,
            "status" => Action::Status,
            _ => return None,
        })
    }
}

const MAX_LINE: usize = 10;

/// Shortens text to `MAX_LINE` characters with an ellipsis.
pub fn fit(text: &str) -> String {
    if text.chars().count() <= MAX_LINE {
        return text.to_string();
    }
    let mut s: String = text.chars().take(MAX_LINE - 1).collect();
    s.push('…');
    s
}

fn setting<'a>(settings: &'a Value, key: &str) -> &'a str {
    settings.get(key).and_then(Value::as_str).unwrap_or("")
}

fn channel_label(settings: &Value) -> String {
    let label = setting(settings, "label");
    if !label.is_empty() {
        return label.to_string();
    }
    let channel = setting(settings, "channel");
    channel
        .split('/')
        .rfind(|p| !p.is_empty())
        .unwrap_or(channel)
        .to_string()
}

/// The title and state a key shows.
pub fn view(action: Action, ts: &TsState, settings: &Value, held: bool) -> (String, u8) {
    let state = |on: bool| u8::from(on);
    if ts.status != Status::Connected {
        if action == Action::Channel
            && ts.status == Status::Disconnected
            && !setting(settings, "channel").is_empty()
        {
            return (fit(&channel_label(settings)), 0);
        }
        let title = match ts.status {
            Status::NoKey => "Set API\nkey",
            Status::BadKey => "Bad API\nkey",
            Status::Connecting => "TS3\n…",
            Status::Offline => "TS3\noffline",
            Status::Disconnected => "No\nserver",
            Status::Connected => unreachable!(),
        };
        return (title.into(), 0);
    }
    match action {
        Action::Mic => (
            if ts.mic_muted { "Mic off" } else { "Mic on" }.into(),
            state(ts.mic_muted),
        ),
        Action::Speakers => (
            if ts.speakers_muted {
                "Sound off"
            } else {
                "Sound on"
            }
            .into(),
            state(ts.speakers_muted),
        ),
        Action::Away => (
            if ts.away { "Away" } else { "Online" }.into(),
            state(ts.away),
        ),
        Action::Ptt => {
            let live = held && ts.transmitting;
            (if live { "TALK" } else { "PTT" }.into(), state(live))
        }
        Action::Channel => {
            let wanted = setting(settings, "channel");
            if wanted.is_empty() {
                return ("Pick\nchannel".into(), 0);
            }
            let found = find_channel(&ts.channels, wanted);
            let name = match found {
                _ if !setting(settings, "label").is_empty() => {
                    setting(settings, "label").to_string()
                }
                Some(cid) => ts
                    .channels
                    .iter()
                    .find(|c| c.cid == cid)
                    .map(|c| c.name.clone())
                    .unwrap_or_default(),
                None => channel_label(settings),
            };
            (fit(&name), state(found == Some(ts.cid)))
        }
        Action::Status => {
            let users = if ts.users == 1 {
                "1 user".to_string()
            } else {
                format!("{} users", ts.users)
            };
            (
                format!("{}\n{users}", fit(&ts.channel_name)),
                state(ts.talking),
            )
        }
    }
}

struct Key {
    action: Action,
    uuid: String,
    settings: Value,
    held: bool,
    shown: Option<(String, u8)>,
}

pub struct Plugin {
    /// The registration token, used as the context for global settings.
    uuid: String,
    requests: Sender<Request>,
    keys: HashMap<String, Key>,
    /// Open settings pages: context -> action UUID.
    inspectors: HashMap<String, String>,
    globals: Option<Config>,
    ts: TsState,
    pub outbox: Vec<Value>,
}

fn parse_config(settings: &Value) -> Config {
    let host = setting(settings, "host").trim();
    let port = match settings.get("port") {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    };
    Config {
        host: if host.is_empty() {
            "127.0.0.1".into()
        } else {
            host.into()
        },
        port: port
            .and_then(|p| u16::try_from(p).ok())
            .filter(|p| *p != 0)
            .unwrap_or(DEFAULT_PORT),
        api_key: setting(settings, "apiKey").trim().to_string(),
    }
}

impl Plugin {
    pub fn new(uuid: String, requests: Sender<Request>) -> Plugin {
        Plugin {
            uuid,
            requests,
            keys: HashMap::new(),
            inspectors: HashMap::new(),
            globals: None,
            ts: TsState::default(),
            outbox: Vec::new(),
        }
    }

    fn request(&self, command: Command, context: Option<&str>) {
        let _ = self.requests.send(Request {
            command,
            context: context.map(str::to_string),
        });
    }

    /// Called once the Deck connection is open.
    pub fn on_open(&mut self) {
        self.outbox
            .push(json!({"event": "getGlobalSettings", "context": self.uuid}));
    }

    pub fn on_deck(&mut self, msg: &Value) {
        let event = msg.get("event").and_then(Value::as_str).unwrap_or("");
        let context = msg
            .get("context")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let action_uuid = msg
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let payload = msg.get("payload").cloned().unwrap_or(Value::Null);
        let settings = || {
            payload
                .get("settings")
                .cloned()
                .unwrap_or_else(|| json!({}))
        };
        match event {
            "didReceiveGlobalSettings" => self.on_globals(&settings()),
            "willAppear" => {
                let Some(action) = Action::from_uuid(&action_uuid) else {
                    return;
                };
                let key = Key {
                    action,
                    uuid: action_uuid,
                    settings: settings(),
                    held: false,
                    shown: None,
                };
                self.keys.insert(context.clone(), key);
                self.render(&context);
            }
            "willDisappear" => {
                // Never leave the mic open because a PTT key left the page.
                if self.keys.remove(&context).is_some_and(|k| k.held) {
                    self.request(Command::Transmit(false), None);
                }
            }
            "didReceiveSettings" => {
                if let Some(key) = self.keys.get_mut(&context) {
                    key.settings = settings();
                    self.render(&context);
                }
            }
            "keyDown" => self.on_key(&context, true),
            "keyUp" => self.on_key(&context, false),
            "propertyInspectorDidAppear" => {
                self.inspectors.insert(context.clone(), action_uuid);
                self.send_status(&context);
            }
            "propertyInspectorDidDisappear" => {
                self.inspectors.remove(&context);
            }
            "sendToPlugin" => self.on_inspector(&context, &action_uuid, &payload),
            // After sleep the ClientQuery socket may be dead without an error;
            // refresh now so the keys don't show stale state.
            "systemDidWakeUp" => self.request(Command::Refresh, None),
            _ => {}
        }
    }

    fn on_globals(&mut self, settings: &Value) {
        let cfg = parse_config(settings);
        let first = self.globals.is_none();
        self.globals = Some(cfg.clone());
        self.request(Command::Configure(cfg), None);
        if first {
            self.render_all();
        }
    }

    fn on_inspector(&mut self, context: &str, action: &str, payload: &Value) {
        match payload.get("type").and_then(Value::as_str) {
            Some("saveConnection") => {
                let mut cfg = parse_config(payload);
                // An empty key field means "keep the saved key".
                if cfg.api_key.is_empty() {
                    cfg.api_key = self
                        .globals
                        .as_ref()
                        .map(|g| g.api_key.clone())
                        .unwrap_or_default();
                }
                let settings = json!({"host": cfg.host, "port": cfg.port, "apiKey": cfg.api_key});
                self.outbox.push(json!({"event": "setGlobalSettings", "context": self.uuid, "payload": settings}));
                self.on_globals(&settings);
                self.send_status(context);
            }
            Some("status") => self.send_status(context),
            Some("channels") => {
                let mut paths: Vec<&str> =
                    self.ts.channels.iter().map(|c| c.path.as_str()).collect();
                paths.sort_by_key(|p| p.to_lowercase());
                self.outbox.push(
                    json!({"event": "sendToPropertyInspector", "context": context, "action": action,
                    "payload": {"type": "channels", "channels": paths}}),
                );
            }
            _ => {}
        }
    }

    fn send_status(&mut self, context: &str) {
        let action = self
            .inspectors
            .get(context)
            .cloned()
            .or_else(|| self.keys.get(context).map(|k| k.uuid.clone()))
            .unwrap_or_default();
        let cfg = self.globals.clone().unwrap_or_default();
        let status = if self.globals.is_none() {
            Status::Connecting
        } else {
            self.ts.status
        };
        self.outbox.push(json!({"event": "sendToPropertyInspector", "context": context, "action": action, "payload": {
            "type": "status",
            "status": status,
            "serverName": self.ts.server_name,
            "channelPath": self.ts.channel_path,
            "nickname": self.ts.nickname,
            "connection": {"host": cfg.host, "port": cfg.port, "hasKey": !cfg.api_key.is_empty()},
        }}));
    }

    pub fn on_ts(&mut self, event: TsEvent) {
        match event {
            TsEvent::State(state) => {
                self.ts = state;
                self.render_all();
                let open: Vec<String> = self.inspectors.keys().cloned().collect();
                for context in open {
                    self.send_status(&context);
                }
            }
            TsEvent::Failed { context, error } => {
                self.log(&format!("action failed: {error}"));
                if let Some(context) = context {
                    self.outbox
                        .push(json!({"event": "showAlert", "context": context}));
                    if let Some(key) = self.keys.get_mut(&context) {
                        key.held = false;
                    }
                    self.render(&context);
                }
            }
            TsEvent::Log(line) => self.log(&line),
        }
    }

    /// Logs to stderr (the host's plugin log) and to the host's own log.
    pub fn log(&mut self, message: &str) {
        eprintln!("ts3deck: {message}");
        self.outbox
            .push(json!({"event": "logMessage", "payload": {"message": message}}));
    }

    fn on_key(&mut self, context: &str, down: bool) {
        let Some(key) = self.keys.get_mut(context) else {
            return;
        };
        let command = match (key.action, down) {
            (Action::Ptt, _) => {
                key.held = down;
                Command::Transmit(down)
            }
            (_, false) => return,
            (Action::Mic, true) => Command::ToggleMic,
            (Action::Speakers, true) => Command::ToggleSpeakers,
            (Action::Away, true) => Command::ToggleAway {
                message: setting(&key.settings, "message").to_string(),
            },
            (Action::Channel, true) => {
                let channel = setting(&key.settings, "channel").to_string();
                if channel.is_empty() {
                    self.outbox
                        .push(json!({"event": "showAlert", "context": context}));
                    return;
                }
                Command::Join {
                    channel,
                    password: setting(&key.settings, "password").to_string(),
                }
            }
            (Action::Status, true) => Command::Refresh,
        };
        self.request(command, Some(context));
        self.render(context);
    }

    fn render_all(&mut self) {
        let contexts: Vec<String> = self.keys.keys().cloned().collect();
        for context in contexts {
            self.render(&context);
        }
    }

    fn render(&mut self, context: &str) {
        // Until the saved connection settings arrive, don't flash "Set API key".
        let connecting = TsState::default();
        let ts = if self.globals.is_none() {
            &connecting
        } else {
            &self.ts
        };
        let Some(key) = self.keys.get_mut(context) else {
            return;
        };
        let (title, state) = view(key.action, ts, &key.settings, key.held);
        let shown = key.shown.as_ref();
        if shown.map(|s| &s.0) != Some(&title) {
            self.outbox.push(json!({"event": "setTitle", "context": context, "payload": {"title": title, "target": 0}}));
        }
        if shown.map(|s| s.1) != Some(state) {
            self.outbox.push(
                json!({"event": "setState", "context": context, "payload": {"state": state}}),
            );
        }
        key.shown = Some((title, state));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clientquery::Channel;

    fn connected() -> TsState {
        TsState {
            status: Status::Connected,
            mic_muted: true,
            away: true,
            transmitting: true,
            cid: 3,
            channel_name: "Squad 1".into(),
            channel_path: "Games/Squad 1".into(),
            users: 1,
            talking: true,
            channels: vec![
                Channel {
                    cid: 1,
                    name: "Lobby".into(),
                    path: "Lobby".into(),
                },
                Channel {
                    cid: 3,
                    name: "Squad 1".into(),
                    path: "Games/Squad 1".into(),
                },
            ],
            ..TsState::default()
        }
    }

    #[test]
    fn offline_states_show_on_every_key() {
        let ts = TsState {
            status: Status::NoKey,
            ..TsState::default()
        };
        for a in [
            Action::Mic,
            Action::Speakers,
            Action::Away,
            Action::Ptt,
            Action::Channel,
            Action::Status,
        ] {
            assert_eq!(view(a, &ts, &json!({}), false), ("Set API\nkey".into(), 0));
        }
        let ts = TsState {
            status: Status::Disconnected,
            ..TsState::default()
        };
        assert_eq!(
            view(
                Action::Channel,
                &ts,
                &json!({"channel": "Games/Squad 1"}),
                false
            )
            .0,
            "Squad 1"
        );
    }

    #[test]
    fn connected_views() {
        let ts = connected();
        let none = json!({});
        assert_eq!(view(Action::Mic, &ts, &none, false), ("Mic off".into(), 1));
        assert_eq!(
            view(Action::Speakers, &ts, &none, false),
            ("Sound on".into(), 0)
        );
        assert_eq!(view(Action::Away, &ts, &none, false), ("Away".into(), 1));
        assert_eq!(view(Action::Ptt, &ts, &none, true), ("TALK".into(), 1));
        assert_eq!(view(Action::Ptt, &ts, &none, false), ("PTT".into(), 0));
        assert_eq!(
            view(
                Action::Channel,
                &ts,
                &json!({"channel": "games/squad 1"}),
                false
            ),
            ("Squad 1".into(), 1)
        );
        assert_eq!(
            view(Action::Channel, &ts, &json!({"channel": "lobby"}), false),
            ("Lobby".into(), 0)
        );
        assert_eq!(
            view(
                Action::Channel,
                &ts,
                &json!({"channel": "Lobby", "label": "Home"}),
                false
            ),
            ("Home".into(), 0)
        );
        assert_eq!(
            view(Action::Channel, &ts, &none, false),
            ("Pick\nchannel".into(), 0)
        );
        assert_eq!(
            view(Action::Status, &ts, &none, false),
            ("Squad 1\n1 user".into(), 1)
        );
        let long = TsState {
            channel_name: "A very long channel name".into(),
            ..connected()
        };
        assert_eq!(
            view(Action::Status, &long, &none, false).0,
            "A very lo…\n1 user"
        );
    }

    #[test]
    fn parses_action_uuids_and_config() {
        assert_eq!(
            Action::from_uuid("com.avonderau.ts3deck.ptt"),
            Some(Action::Ptt)
        );
        assert_eq!(Action::from_uuid("com.other.ptt"), None);
        let cfg = parse_config(&json!({"port": "1234", "apiKey": " k "}));
        assert_eq!(
            cfg,
            Config {
                host: "127.0.0.1".into(),
                port: 1234,
                api_key: "k".into()
            }
        );
        assert_eq!(parse_config(&json!({"port": 99999})).port, DEFAULT_PORT);
    }
}
