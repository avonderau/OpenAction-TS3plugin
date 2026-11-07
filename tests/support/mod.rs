//! A fake TeamSpeak 3 ClientQuery server for tests. It speaks the same line
//! protocol: "\n\r" line ends, "error id=… msg=…" trailers, notify events.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ts3deck::clientquery::{TsEvent, TsState};
use ts3deck::escape::{build_command, escape, parse_entry};

pub const API_KEY: &str = "KEY-123";

#[derive(Debug)]
pub struct Ts3 {
    pub connected: bool,
    pub clid: u64,
    pub cid: u64,
    pub vars: HashMap<&'static str, String>,
    pub channels: Vec<(u64, u64, &'static str)>,
    pub others: Vec<(u64, u64)>,
    pub passwords: HashMap<u64, &'static str>,
    pub log: Vec<String>,
}

impl Default for Ts3 {
    fn default() -> Self {
        let vars = [
            ("client_nickname", "Andrew"),
            ("client_input_muted", "0"),
            ("client_output_muted", "0"),
            ("client_away", "0"),
            ("client_away_message", ""),
            ("client_input_deactivated", "0"),
        ];
        Ts3 {
            connected: true,
            clid: 5,
            cid: 1,
            vars: vars.into_iter().map(|(k, v)| (k, v.to_string())).collect(),
            channels: vec![
                (1, 0, "Lobby"),
                (2, 0, "Games"),
                (3, 2, "Squad 1"),
                (4, 2, "Squad 2"),
                (5, 0, "AFK"),
            ],
            others: vec![(6, 1), (7, 3)],
            passwords: HashMap::from([(4, "secret")]),
            log: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct MockTs3 {
    pub port: u16,
    pub state: Arc<Mutex<Ts3>>,
    clients: Arc<Mutex<Vec<TcpStream>>>,
}

impl MockTs3 {
    pub fn start(state: Ts3) -> MockTs3 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mock = MockTs3 {
            port: listener.local_addr().unwrap().port(),
            state: Arc::new(Mutex::new(state)),
            clients: Arc::default(),
        };
        let m = mock.clone();
        thread::spawn(move || {
            for sock in listener.incoming().flatten() {
                let m = m.clone();
                m.clients.lock().unwrap().push(sock.try_clone().unwrap());
                thread::spawn(move || m.serve(sock));
            }
        });
        mock
    }

    /// Sends a line to every connected client.
    pub fn broadcast(&self, line: &str) {
        for c in self.clients.lock().unwrap().iter_mut() {
            let _ = c.write_all(format!("{line}\n\r").as_bytes());
        }
    }

    pub fn drop_clients(&self) {
        for c in self.clients.lock().unwrap().drain(..) {
            let _ = c.shutdown(std::net::Shutdown::Both);
        }
    }

    pub fn var(&self, k: &str) -> String {
        self.state.lock().unwrap().vars[k].clone()
    }

    fn serve(&self, sock: TcpStream) {
        let mut out = sock.try_clone().unwrap();
        let mut send = |line: &str| {
            let _ = out.write_all(format!("{line}\n\r").as_bytes());
        };
        send("TS3 Client");
        send("Welcome to the TeamSpeak 3 ClientQuery interface, type \"help\" for a list of commands.");
        send("Use the \"auth\" command to authenticate yourself. Your API key can be found in the client's ClientQuery settings.");
        send("selected schandlerid=1");
        let mut authed = false;
        for line in BufReader::new(sock).lines() {
            let Ok(line) = line else { return };
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }
            let (cmd, rest) = line.split_once(' ').unwrap_or((&line, ""));
            let args = parse_entry(rest);
            self.state.lock().unwrap().log.push(line.clone());
            if cmd == "auth" {
                if args.get("apikey").map(String::as_str) == Some(API_KEY) {
                    authed = true;
                    send("error id=0 msg=ok");
                } else {
                    send("error id=256 msg=invalid\\sapikey");
                }
                continue;
            }
            if !authed {
                send("error id=1796 msg=currently\\snot\\spossible");
                continue;
            }
            let (reply, notify) = self.handle(cmd, &args);
            for l in reply {
                send(&l);
            }
            if let Some(n) = notify {
                self.broadcast(&n);
            }
        }
    }

    fn handle(&self, cmd: &str, args: &HashMap<String, String>) -> (Vec<String>, Option<String>) {
        let ok = || "error id=0 msg=ok".to_string();
        let mut s = self.state.lock().unwrap();
        match cmd {
            "clientnotifyregister" | "use" => (vec![ok()], None),
            "currentschandlerid" => (vec!["schandlerid=1".into(), ok()], None),
            "whoami" if !s.connected => (vec!["error id=1794 msg=not\\sconnected".into()], None),
            "whoami" => (vec![format!("clid={} cid={}", s.clid, s.cid), ok()], None),
            "clientvariable" => {
                let mut line = format!("clid={}", s.clid);
                for (k, v) in &s.vars {
                    line.push_str(&format!(" {k}={}", escape(v)));
                }
                (vec![line, ok()], None)
            }
            "channellist" => {
                let list: Vec<String> = s
                    .channels
                    .iter()
                    .map(|(cid, pid, name)| {
                        format!(
                            "cid={cid} pid={pid} channel_order=0 channel_name={}",
                            escape(name)
                        )
                    })
                    .collect();
                (vec![list.join("|"), ok()], None)
            }
            "clientlist" => {
                let mut all = vec![(s.clid, s.cid)];
                all.extend(s.others.iter().copied());
                let list: Vec<String> = all
                    .iter()
                    .map(|(clid, cid)| format!("clid={clid} cid={cid} client_nickname=x"))
                    .collect();
                (vec![list.join("|"), ok()], None)
            }
            "servervariable" => (vec!["virtualserver_name=Test\\sServer".into(), ok()], None),
            "clientupdate" => {
                let mut changed = vec![
                    ("schandlerid", Some("1".to_string())),
                    ("clid", Some(s.clid.to_string())),
                ];
                for (k, v) in args {
                    let Some(key) = s.vars.keys().find(|x| **x == k.as_str()).copied() else {
                        return (vec!["error id=1538 msg=invalid\\sparameter".into()], None);
                    };
                    s.vars.insert(key, v.clone());
                    changed.push((key, Some(v.clone())));
                }
                (
                    vec![ok()],
                    Some(build_command("notifyclientupdated", &changed)),
                )
            }
            "clientmove" => {
                let cid: u64 = args.get("cid").and_then(|c| c.parse().ok()).unwrap_or(0);
                if !s.channels.iter().any(|c| c.0 == cid) {
                    return (vec!["error id=768 msg=invalid\\schannelID".into()], None);
                }
                if let Some(pw) = s.passwords.get(&cid) {
                    if args.get("cpw").map(String::as_str) != Some(*pw) {
                        return (vec!["error id=781 msg=invalid\\spassword".into()], None);
                    }
                }
                s.cid = cid;
                let clid = s.clid;
                (
                    vec![ok()],
                    Some(format!(
                        "notifyclientmoved schandlerid=1 ctid={cid} reasonid=0 clid={clid}"
                    )),
                )
            }
            _ => (vec!["error id=256 msg=command\\snot\\sfound".into()], None),
        }
    }
}

/// Waits for a state matching `pred`, skipping other events.
pub fn wait_state(events: &Receiver<TsEvent>, pred: impl Fn(&TsState) -> bool) -> TsState {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(TsEvent::State(s)) if pred(&s) => return s,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for state"),
            Err(RecvTimeoutError::Disconnected) => panic!("worker stopped"),
        }
    }
}

/// Waits for a Failed event and returns its error text.
pub fn wait_failed(events: &Receiver<TsEvent>) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(TsEvent::Failed { error, .. }) => return error,
            Ok(_) => {}
            Err(_) => panic!("no failure reported"),
        }
    }
}
