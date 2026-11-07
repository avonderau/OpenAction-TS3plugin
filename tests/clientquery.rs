mod support;

use std::sync::mpsc::{self, Receiver, Sender};

use support::{wait_failed, wait_state, MockTs3, Ts3, API_KEY};
use ts3deck::clientquery::{spawn, Command, Config, Request, Status, TsEvent};

fn worker(port: u16, key: &str) -> (Sender<Request>, Receiver<TsEvent>) {
    let (tx, rx) = mpsc::channel();
    let (etx, erx) = mpsc::channel();
    spawn(rx, move |e| {
        let _ = etx.send(e);
    });
    tx.send(req(Command::Configure(Config {
        host: "127.0.0.1".into(),
        port,
        api_key: key.into(),
    })))
    .unwrap();
    (tx, erx)
}

fn req(command: Command) -> Request {
    Request {
        command,
        context: Some("ctx".into()),
    }
}

#[test]
fn connects_reads_state_and_controls_the_client() {
    let ts3 = MockTs3::start(Ts3::default());
    let (tx, ev) = worker(ts3.port, API_KEY);
    let s = wait_state(&ev, |s| s.status == Status::Connected);
    assert_eq!(s.nickname, "Andrew");
    assert_eq!(s.channel_name, "Lobby");
    assert_eq!(s.server_name, "Test Server");
    assert_eq!(s.users, 2);
    assert!(!s.mic_muted);

    tx.send(req(Command::ToggleMic)).unwrap();
    wait_state(&ev, |s| s.mic_muted);
    assert_eq!(ts3.var("client_input_muted"), "1");

    tx.send(req(Command::ToggleSpeakers)).unwrap();
    wait_state(&ev, |s| s.speakers_muted);

    tx.send(req(Command::ToggleAway {
        message: "back in 5".into(),
    }))
    .unwrap();
    wait_state(&ev, |s| s.away);
    assert_eq!(ts3.var("client_away_message"), "back in 5");
    tx.send(req(Command::ToggleAway {
        message: String::new(),
    }))
    .unwrap();
    wait_state(&ev, |s| !s.away);

    tx.send(req(Command::Transmit(false))).unwrap();
    wait_state(&ev, |s| !s.transmitting);
    assert_eq!(ts3.var("client_input_deactivated"), "1");
    tx.send(req(Command::Transmit(true))).unwrap();
    wait_state(&ev, |s| s.transmitting);

    tx.send(req(Command::Join {
        channel: "Games/Squad 1".into(),
        password: String::new(),
    }))
    .unwrap();
    let s = wait_state(&ev, |s| s.cid == 3);
    assert_eq!(s.channel_path, "Games/Squad 1");
    assert_eq!(s.users, 2);

    tx.send(req(Command::Join {
        channel: "Squad 2".into(),
        password: "wrong".into(),
    }))
    .unwrap();
    assert!(wait_failed(&ev).contains("invalid password"));
    tx.send(req(Command::Join {
        channel: "Squad 2".into(),
        password: "secret".into(),
    }))
    .unwrap();
    wait_state(&ev, |s| s.cid == 4);
    tx.send(req(Command::Join {
        channel: "Nowhere".into(),
        password: String::new(),
    }))
    .unwrap();
    assert!(wait_failed(&ev).contains("not found"));
}

#[test]
fn follows_changes_made_in_teamspeak() {
    let ts3 = MockTs3::start(Ts3::default());
    let (_tx, ev) = worker(ts3.port, API_KEY);
    wait_state(&ev, |s| s.status == Status::Connected);

    ts3.state
        .lock()
        .unwrap()
        .vars
        .insert("client_input_muted", "1".into());
    ts3.broadcast("notifyclientupdated schandlerid=1 clid=5 client_input_muted=1");
    wait_state(&ev, |s| s.mic_muted);

    ts3.broadcast("notifytalkstatuschange schandlerid=1 status=1 isreceivedwhisper=0 clid=5");
    wait_state(&ev, |s| s.talking);
    ts3.broadcast("notifytalkstatuschange schandlerid=1 status=0 isreceivedwhisper=0 clid=5");
    wait_state(&ev, |s| !s.talking);
}

#[test]
fn reports_bad_key_and_no_server() {
    let ts3 = MockTs3::start(Ts3 {
        connected: false,
        ..Ts3::default()
    });
    let (_bad, bad_ev) = worker(ts3.port, "WRONG");
    wait_state(&bad_ev, |s| s.status == Status::BadKey);

    let (tx, ev) = worker(ts3.port, API_KEY);
    wait_state(&ev, |s| s.status == Status::Disconnected);
    tx.send(req(Command::ToggleMic)).unwrap();
    assert!(wait_failed(&ev).contains("not connected to a server"));
}

#[test]
fn reconnects_after_teamspeak_restarts() {
    let ts3 = MockTs3::start(Ts3::default());
    let (_tx, ev) = worker(ts3.port, API_KEY);
    wait_state(&ev, |s| s.status == Status::Connected);
    ts3.drop_clients();
    wait_state(&ev, |s| s.status == Status::Offline);
    wait_state(&ev, |s| s.status == Status::Connected);
}

#[test]
fn waits_for_a_key_and_applies_new_settings() {
    let ts3 = MockTs3::start(Ts3::default());
    let (tx, ev) = worker(ts3.port, "");
    wait_state(&ev, |s| s.status == Status::NoKey);
    tx.send(req(Command::ToggleMic)).unwrap();
    assert!(wait_failed(&ev).contains("no ClientQuery API key"));
    tx.send(req(Command::Configure(Config {
        host: "127.0.0.1".into(),
        port: ts3.port,
        api_key: API_KEY.into(),
    })))
    .unwrap();
    wait_state(&ev, |s| s.status == Status::Connected);
}

#[test]
fn reports_offline_when_nothing_listens() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (_tx, ev) = worker(port, API_KEY);
    wait_state(&ev, |s| s.status == Status::Offline);
}
