//! A client for the TeamSpeak 3 client's ClientQuery interface (Tools >
//! Options > Addons > ClientQuery, TCP port 25639 by default).
//!
//! [`spawn`] runs one worker thread that owns the connection. It
//! authenticates with the API key, follows the selected server tab, and
//! reports a [`TsState`] whenever something the Deck keys show changes. The
//! plugin sends it [`Request`]s; failures come back as [`TsEvent::Failed`]
//! so the key that asked can show an alert.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::escape::{build_command, parse_entry, parse_list, Entry};

pub const DEFAULT_PORT: u16 = 25639;
pub const ERR_NOT_CONNECTED: u32 = 1794;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const GREETING_WAIT: Duration = Duration::from_millis(300);
const GREETING_MAX: Duration = Duration::from_millis(1500);
const POLL: Duration = Duration::from_millis(40);
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(80);
const KEEPALIVE: Duration = Duration::from_secs(30);
const RETRY_MIN: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub api_key: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            host: "127.0.0.1".into(),
            port: DEFAULT_PORT,
            api_key: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// No API key configured yet.
    NoKey,
    /// TeamSpeak refused the API key.
    BadKey,
    #[default]
    Connecting,
    /// TeamSpeak is not running or ClientQuery is off.
    Offline,
    /// TeamSpeak is running but the selected tab has no server.
    Disconnected,
    Connected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    pub cid: u64,
    pub name: String,
    /// Full path from the top, levels joined with "/".
    pub path: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TsState {
    pub status: Status,
    pub clid: u64,
    pub cid: u64,
    pub nickname: String,
    pub server_name: String,
    pub mic_muted: bool,
    pub speakers_muted: bool,
    pub away: bool,
    pub transmitting: bool,
    pub channel_name: String,
    pub channel_path: String,
    pub users: usize,
    pub talking: bool,
    pub channels: Vec<Channel>,
}

impl TsState {
    fn with_status(status: Status) -> Self {
        TsState {
            status,
            ..TsState::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Configure(Config),
    ToggleMic,
    ToggleSpeakers,
    ToggleAway {
        message: String,
    },
    /// Open (true) or close the mic for push-to-talk.
    Transmit(bool),
    Join {
        channel: String,
        password: String,
    },
    Refresh,
}

#[derive(Clone, Debug)]
pub struct Request {
    pub command: Command,
    /// The key that asked, so a failure can be shown on it.
    pub context: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TsEvent {
    State(TsState),
    Failed {
        context: Option<String>,
        error: String,
    },
    Log(String),
}

/// Builds cid -> channel with full paths from a `channellist` reply.
pub fn channel_paths(entries: &[Entry]) -> Vec<Channel> {
    let mut raw: HashMap<u64, (String, u64)> = HashMap::new();
    let mut order = Vec::new();
    for e in entries {
        let Some(cid) = e.get("cid").and_then(|c| c.parse().ok()) else {
            continue;
        };
        let pid = e.get("pid").and_then(|p| p.parse().ok()).unwrap_or(0);
        raw.insert(
            cid,
            (e.get("channel_name").cloned().unwrap_or_default(), pid),
        );
        order.push(cid);
    }
    order
        .into_iter()
        .map(|cid| {
            let mut parts = Vec::new();
            let mut at = cid;
            while let Some((name, pid)) = raw.get(&at) {
                parts.push(name.clone());
                if *pid == 0 || parts.len() > 64 {
                    break;
                }
                at = *pid;
            }
            parts.reverse();
            Channel {
                cid,
                name: raw[&cid].0.clone(),
                path: parts.join("/"),
            }
        })
        .collect()
}

/// Resolves "Parent/Child" or a plain channel name, ignoring case. A full
/// path wins; a bare name must be unique on the server.
pub fn find_channel(channels: &[Channel], wanted: &str) -> Option<u64> {
    let target = wanted.trim().trim_matches('/').to_lowercase();
    if target.is_empty() {
        return None;
    }
    if let Some(c) = channels.iter().find(|c| c.path.to_lowercase() == target) {
        return Some(c.cid);
    }
    let mut by_name = channels.iter().filter(|c| c.name.to_lowercase() == target);
    match (by_name.next(), by_name.next()) {
        (Some(c), None) => Some(c.cid),
        _ => None,
    }
}

#[derive(Debug)]
enum QueryError {
    Io(String),
    Query { id: u32, msg: String },
}

impl QueryError {
    fn text(&self) -> String {
        match self {
            QueryError::Io(e) => format!("TeamSpeak connection lost ({e})"),
            QueryError::Query { msg, .. } => msg.clone(),
        }
    }
}

impl From<io::Error> for QueryError {
    fn from(e: io::Error) -> Self {
        QueryError::Io(e.to_string())
    }
}

/// One ClientQuery connection with its own line buffer, so a read timeout
/// never loses a half-received line.
struct Conn {
    stream: TcpStream,
    pending: Vec<u8>,
    notifications: Vec<String>,
}

impl Conn {
    fn open(cfg: &Config) -> io::Result<Conn> {
        let addr = (cfg.host.as_str(), cfg.port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no address"))?;
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
        stream.set_nodelay(true)?;
        Ok(Conn {
            stream,
            pending: Vec::new(),
            notifications: Vec::new(),
        })
    }

    /// Returns the next line, or None when `wait` passes without one.
    fn next_line(&mut self, wait: Duration) -> io::Result<Option<String>> {
        let deadline = Instant::now() + wait;
        loop {
            if let Some(pos) = self.pending.iter().position(|&b| b == b'\n' || b == b'\r') {
                let line: Vec<u8> = self.pending.drain(..=pos).collect();
                let text = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
                if text.is_empty() {
                    continue;
                }
                return Ok(Some(text));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            self.stream
                .set_read_timeout(Some(left.max(Duration::from_millis(1))))?;
            let mut buf = [0u8; 8192];
            match self.stream.read(&mut buf) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed")),
                Ok(n) => self.pending.extend_from_slice(&buf[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None)
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Reads the greeting. Some versions never send its last line.
    fn greet(&mut self) -> io::Result<()> {
        let start = Instant::now();
        let mut seen = false;
        while start.elapsed() < GREETING_MAX {
            match self.next_line(if seen { GREETING_WAIT } else { GREETING_MAX })? {
                Some(l) if l.starts_with("selected ") => return Ok(()),
                Some(l) if l.starts_with("TS3 Client") => seen = true,
                Some(_) => {}
                None if seen => return Ok(()),
                None => {}
            }
        }
        Ok(())
    }

    /// Sends one command line and returns its data lines.
    fn raw(&mut self, line: &str) -> Result<Vec<String>, QueryError> {
        self.stream.write_all(line.as_bytes())?;
        self.stream.write_all(b"\n")?;
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        let mut data = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let Some(l) = self.next_line(left)? else {
                return Err(QueryError::Io(format!(
                    "timed out on {}",
                    line.split(' ').next().unwrap_or("")
                )));
            };
            if l.starts_with("notify") {
                self.notifications.push(l);
            } else if let Some(rest) = l.strip_prefix("error ") {
                let e = parse_entry(rest);
                let id = e.get("id").and_then(|i| i.parse().ok()).unwrap_or(u32::MAX);
                if id == 0 {
                    return Ok(data);
                }
                return Err(QueryError::Query {
                    id,
                    msg: e.get("msg").cloned().unwrap_or_default(),
                });
            } else {
                data.push(l);
            }
        }
    }

    fn command(
        &mut self,
        name: &str,
        params: &[(&str, Option<String>)],
    ) -> Result<Vec<String>, QueryError> {
        self.raw(&build_command(name, params))
    }

    /// Runs a command whose reply is one entry, merging all data lines.
    fn entry(&mut self, line: &str) -> Result<Entry, QueryError> {
        let mut out = Entry::new();
        for l in self.raw(line)? {
            out.extend(parse_entry(&l));
        }
        Ok(out)
    }

    fn list(&mut self, line: &str) -> Result<Vec<Entry>, QueryError> {
        Ok(self.raw(line)?.iter().flat_map(|l| parse_list(l)).collect())
    }
}

/// Starts the worker. Events go to `emit`; it stops when `requests` closes.
pub fn spawn<F>(requests: Receiver<Request>, emit: F) -> thread::JoinHandle<()>
where
    F: Fn(TsEvent) + Send + 'static,
{
    thread::spawn(move || Worker::new(requests, Box::new(emit)).run())
}

enum Next {
    /// Reconnect now (settings changed).
    Reconnect,
    /// The requests channel closed.
    Stop,
}

struct Worker {
    requests: Receiver<Request>,
    emit: Box<dyn Fn(TsEvent) + Send>,
    cfg: Config,
    state: TsState,
    talking: std::collections::HashSet<String>,
    schid: String,
}

impl Worker {
    fn new(requests: Receiver<Request>, emit: Box<dyn Fn(TsEvent) + Send>) -> Self {
        Worker {
            requests,
            emit,
            cfg: Config::default(),
            state: TsState::with_status(Status::Connecting),
            talking: Default::default(),
            schid: String::new(),
        }
    }

    fn set_state(&mut self, next: TsState) {
        if next != self.state {
            self.state = next.clone();
            (self.emit)(TsEvent::State(next));
        }
    }

    fn set_status(&mut self, status: Status) {
        self.set_state(TsState::with_status(status));
    }

    fn fail(&self, context: Option<String>, error: String) {
        (self.emit)(TsEvent::Failed { context, error });
    }

    fn log(&self, line: String) {
        (self.emit)(TsEvent::Log(line));
    }

    fn run(mut self) {
        let mut retry = RETRY_MIN;
        loop {
            if self.cfg.api_key.is_empty() {
                self.set_status(Status::NoKey);
                match self.idle(None) {
                    Next::Stop => return,
                    Next::Reconnect => continue,
                }
            }
            self.set_status(Status::Connecting);
            let wait = match Conn::open(&self.cfg) {
                Err(e) => {
                    self.log(format!(
                        "ClientQuery: cannot reach {}:{} ({e})",
                        self.cfg.host, self.cfg.port
                    ));
                    self.set_status(Status::Offline);
                    let w = retry;
                    retry = (retry * 2).min(RETRY_MAX);
                    w
                }
                Ok(mut conn) => match self.session(&mut conn) {
                    Ok(Next::Stop) => return,
                    Ok(Next::Reconnect) => {
                        retry = RETRY_MIN;
                        continue;
                    }
                    Err(QueryError::Query { msg, .. }) => {
                        self.log(format!("ClientQuery rejected the API key ({msg})"));
                        self.set_status(Status::BadKey);
                        // Retry slowly in case the key changes on the TeamSpeak side.
                        RETRY_MAX
                    }
                    Err(QueryError::Io(e)) => {
                        self.log(format!("ClientQuery connection lost: {e}"));
                        self.set_status(Status::Offline);
                        retry = RETRY_MIN;
                        retry
                    }
                },
            };
            if let Next::Stop = self.idle(Some(wait)) {
                return;
            }
        }
    }

    /// Waits while not connected: fails key actions, applies new settings.
    /// Returns Reconnect when the wait ends or settings change.
    fn idle(&mut self, wait: Option<Duration>) -> Next {
        let deadline = wait.map(|w| Instant::now() + w);
        loop {
            let req = match deadline {
                None => self
                    .requests
                    .recv()
                    .map_err(|_| RecvTimeoutError::Disconnected),
                Some(d) => self
                    .requests
                    .recv_timeout(d.saturating_duration_since(Instant::now())),
            };
            match req {
                Ok(Request {
                    command: Command::Configure(cfg),
                    ..
                }) => {
                    if cfg != self.cfg {
                        self.cfg = cfg;
                        return Next::Reconnect;
                    }
                }
                Ok(Request {
                    command: Command::Refresh,
                    ..
                }) => {}
                Ok(Request { context, .. }) => {
                    let why = match self.state.status {
                        Status::NoKey => "no ClientQuery API key set",
                        Status::BadKey => "TeamSpeak rejected the API key",
                        _ => "TeamSpeak is not reachable",
                    };
                    self.fail(context, why.into());
                }
                Err(RecvTimeoutError::Timeout) => return Next::Reconnect,
                Err(RecvTimeoutError::Disconnected) => return Next::Stop,
            }
        }
    }

    /// Runs one authenticated connection until it drops or settings change.
    /// An auth failure is returned as a Query error.
    fn session(&mut self, conn: &mut Conn) -> Result<Next, QueryError> {
        conn.greet()?;
        conn.command("auth", &[("apikey", Some(self.cfg.api_key.clone()))])?;
        conn.command(
            "clientnotifyregister",
            &[
                ("schandlerid", Some("0".into())),
                ("event", Some("any".into())),
            ],
        )
        .map_err(as_io)?;
        self.talking.clear();
        self.refresh(conn).map_err(as_io)?;
        let mut refresh_at: Option<Instant> = None;
        let mut last_refresh = Instant::now();
        loop {
            match self.requests.recv_timeout(POLL) {
                Ok(Request {
                    command: Command::Configure(cfg),
                    ..
                }) => {
                    if cfg != self.cfg {
                        self.cfg = cfg;
                        return Ok(Next::Reconnect);
                    }
                }
                Ok(req) => {
                    let result = self.execute(conn, &req.command);
                    if let Err(e) = result {
                        if let QueryError::Io(_) = e {
                            self.fail(req.context, e.text());
                            return Err(e);
                        }
                        self.fail(req.context, e.text());
                    }
                    // Commands refresh themselves; pick up anything else too.
                    last_refresh = Instant::now();
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(Next::Stop),
            }
            while let Some(line) = conn.next_line(Duration::from_millis(1))? {
                conn.notifications.push(line);
            }
            for line in std::mem::take(&mut conn.notifications) {
                if self.on_notify(&line) {
                    refresh_at.get_or_insert(Instant::now() + REFRESH_DEBOUNCE);
                }
            }
            let due = refresh_at.is_some_and(|t| Instant::now() >= t)
                || last_refresh.elapsed() >= KEEPALIVE;
            if due {
                refresh_at = None;
                last_refresh = Instant::now();
                self.refresh(conn).map_err(as_io)?;
            }
        }
    }

    /// Handles a notification; returns true when a refresh is needed.
    fn on_notify(&mut self, line: &str) -> bool {
        let (name, rest) = line.split_once(' ').unwrap_or((line, ""));
        let data = parse_entry(rest);
        let same_tab = data.get("schandlerid").map_or(true, |s| *s == self.schid);
        match name {
            "notifytalkstatuschange" => {
                if !same_tab {
                    return false;
                }
                let clid = data.get("clid").cloned().unwrap_or_default();
                let on = data.get("status").is_some_and(|s| s == "1");
                if on {
                    self.talking.insert(clid.clone());
                } else {
                    self.talking.remove(&clid);
                }
                if clid == self.state.clid.to_string() && self.state.status == Status::Connected {
                    let next = TsState {
                        talking: on,
                        ..self.state.clone()
                    };
                    self.set_state(next);
                }
                false
            }
            "notifycurrentserverconnectionchanged" => true,
            "notifyclientupdated"
            | "notifyclientmoved"
            | "notifycliententerview"
            | "notifyclientleftview"
            | "notifyconnectstatuschange"
            | "notifychanneledited"
            | "notifychannelcreated"
            | "notifychanneldeleted"
            | "notifychannelmoved" => same_tab,
            _ => false,
        }
    }

    /// Reads everything the keys show from the selected server tab.
    fn refresh(&mut self, conn: &mut Conn) -> Result<(), QueryError> {
        let tab = conn.entry("currentschandlerid")?;
        let schid = tab
            .get("schandlerid")
            .cloned()
            .unwrap_or_else(|| "1".into());
        if schid != self.schid {
            self.talking.clear();
            self.schid = schid.clone();
        }
        conn.command("use", &[("schandlerid", Some(schid))])?;
        let me = match conn.entry("whoami") {
            Ok(me) => me,
            Err(QueryError::Query {
                id: ERR_NOT_CONNECTED,
                ..
            }) => {
                self.set_status(Status::Disconnected);
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        let num = |e: &Entry, k: &str| e.get(k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        let clid = num(&me, "clid");
        let cid = num(&me, "cid");
        let vars = conn.entry(&format!(
            "clientvariable clid={clid} client_nickname client_input_muted client_output_muted client_away client_input_deactivated"
        ))?;
        let channels = channel_paths(&conn.list("channellist")?);
        let users = conn
            .list("clientlist")?
            .iter()
            .filter(|c| num(c, "cid") == cid)
            .count();
        let server = conn
            .entry("servervariable virtualserver_name")
            .unwrap_or_default();
        let flag = |k: &str| vars.get(k).is_some_and(|v| v == "1");
        let here = channels.iter().find(|c| c.cid == cid);
        let next = TsState {
            status: Status::Connected,
            clid,
            cid,
            nickname: vars.get("client_nickname").cloned().unwrap_or_default(),
            server_name: server
                .get("virtualserver_name")
                .cloned()
                .unwrap_or_default(),
            mic_muted: flag("client_input_muted"),
            speakers_muted: flag("client_output_muted"),
            away: flag("client_away"),
            transmitting: !flag("client_input_deactivated"),
            channel_name: here.map(|c| c.name.clone()).unwrap_or_default(),
            channel_path: here.map(|c| c.path.clone()).unwrap_or_default(),
            users,
            talking: self.talking.contains(&clid.to_string()),
            channels,
        };
        self.set_state(next);
        Ok(())
    }

    fn execute(&mut self, conn: &mut Conn, command: &Command) -> Result<(), QueryError> {
        if *command == Command::Refresh {
            return self.refresh(conn);
        }
        match self.state.status {
            Status::Connected => {}
            Status::Disconnected => {
                return Err(QueryError::Query {
                    id: ERR_NOT_CONNECTED,
                    msg: "not connected to a server".into(),
                })
            }
            _ => {
                return Err(QueryError::Query {
                    id: 0,
                    msg: "TeamSpeak is not reachable".into(),
                })
            }
        }
        let bit = |on: bool| Some(if on { "1" } else { "0" }.to_string());
        let s = &self.state;
        match command {
            Command::ToggleMic => {
                conn.command("clientupdate", &[("client_input_muted", bit(!s.mic_muted))])?
            }
            Command::ToggleSpeakers => conn.command(
                "clientupdate",
                &[("client_output_muted", bit(!s.speakers_muted))],
            )?,
            Command::ToggleAway { message } => {
                if s.away {
                    conn.command("clientupdate", &[("client_away", bit(false))])?
                } else {
                    conn.command(
                        "clientupdate",
                        &[
                            ("client_away", bit(true)),
                            ("client_away_message", Some(message.clone())),
                        ],
                    )?
                }
            }
            // Works when TeamSpeak's capture mode is Continuous Transmission.
            Command::Transmit(on) => {
                conn.command("clientupdate", &[("client_input_deactivated", bit(!on))])?
            }
            Command::Join { channel, password } => {
                let Some(cid) = find_channel(&s.channels, channel) else {
                    return Err(QueryError::Query {
                        id: 768,
                        msg: format!("channel \"{channel}\" not found"),
                    });
                };
                if cid == s.cid {
                    return Ok(());
                }
                let cpw = (!password.is_empty()).then(|| password.clone());
                conn.command(
                    "clientmove",
                    &[
                        ("cid", Some(cid.to_string())),
                        ("clid", Some(s.clid.to_string())),
                        ("cpw", cpw),
                    ],
                )?
            }
            Command::Configure(_) | Command::Refresh => unreachable!(),
        };
        self.refresh(conn)
    }
}

/// After authentication every query error is unexpected; treat it as a
/// broken connection so the worker reconnects.
fn as_io(e: QueryError) -> QueryError {
    match e {
        QueryError::Query { id, msg } => QueryError::Io(format!("error {id}: {msg}")),
        io => io,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channels() -> Vec<Channel> {
        channel_paths(&parse_list(
            "cid=1 pid=0 channel_name=Lobby|cid=2 pid=0 channel_name=Games|cid=3 pid=2 channel_name=Squad\\s1|cid=4 pid=0 channel_name=Squad\\s1",
        ))
    }

    #[test]
    fn builds_channel_paths() {
        let ch = channels();
        assert_eq!(
            ch.iter().find(|c| c.cid == 3).unwrap().path,
            "Games/Squad 1"
        );
        assert_eq!(ch.iter().find(|c| c.cid == 4).unwrap().path, "Squad 1");
    }

    #[test]
    fn finds_channels_by_path_or_unique_name() {
        let ch = channels();
        assert_eq!(find_channel(&ch, "games/squad 1"), Some(3));
        assert_eq!(find_channel(&ch, "/Lobby/"), Some(1));
        // The top-level "Squad 1" matches as a full path.
        assert_eq!(find_channel(&ch, "Squad 1"), Some(4));
        assert_eq!(find_channel(&ch, "Games"), Some(2));
        assert_eq!(find_channel(&ch, "nope"), None);
        assert_eq!(find_channel(&ch, ""), None);
    }
}
