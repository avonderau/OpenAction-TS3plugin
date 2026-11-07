//! Runs the built plugin binary against a fake Deck host and a fake
//! ClientQuery server, the way a host launches it.

mod support;

use std::net::TcpListener;
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use support::{MockTs3, Ts3, API_KEY};
use tungstenite::Message;

#[test]
// The handshake callback's error type is tungstenite's, not ours.
#[allow(clippy::result_large_err)]
fn binary_registers_and_drives_a_key() {
    let ts3 = MockTs3::start(Ts3::default());
    let host = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = host.local_addr().unwrap().port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ts3deck"))
        .args([
            "-port",
            &port.to_string(),
            "-pluginUUID",
            "tok123",
            "-registerEvent",
            "registerPlugin",
            "-info",
            "{}",
        ])
        .spawn()
        .unwrap();

    let (stream, _) = host.accept().unwrap();
    let mut origin = None;
    let mut ws = tungstenite::accept_hdr(
        stream,
        |req: &tungstenite::handshake::server::Request, resp| {
            origin = req.headers().get("origin").cloned();
            Ok(resp)
        },
    )
    .unwrap();
    assert!(
        origin.is_none(),
        "the plugin must not send an Origin header"
    );
    ws.get_ref()
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();

    let mut seen: Vec<Value> = Vec::new();
    let mut wait =
        |ws: &mut tungstenite::WebSocket<_>, what: &str, pred: &dyn Fn(&Value) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(m) = seen.iter().find(|m| pred(m)) {
                    return m.clone();
                }
                assert!(Instant::now() < deadline, "no {what}; got {seen:#?}");
                if let Ok(Message::Text(t)) = ws.read() {
                    seen.push(serde_json::from_str(&t).unwrap());
                }
            }
        };
    let send = |ws: &mut tungstenite::WebSocket<_>, v: Value| {
        ws.send(Message::Text(v.to_string())).unwrap()
    };

    wait(&mut ws, "registration", &|m| {
        m["event"] == "registerPlugin" && m["uuid"] == "tok123"
    });
    wait(&mut ws, "getGlobalSettings", &|m| {
        m["event"] == "getGlobalSettings" && m["context"] == "tok123"
    });
    send(
        &mut ws,
        json!({"event": "didReceiveGlobalSettings", "payload": {"settings": {"port": ts3.port, "apiKey": API_KEY}}}),
    );
    send(
        &mut ws,
        json!({"event": "willAppear", "action": "com.avonderau.ts3deck.mic", "context": "c1", "payload": {"settings": {}}}),
    );
    let title = |want: &'static str| {
        move |m: &Value| {
            m["event"] == "setTitle" && m["context"] == "c1" && m["payload"]["title"] == want
        }
    };
    wait(&mut ws, "Mic on", &title("Mic on"));
    send(&mut ws, json!({"event": "keyDown", "context": "c1"}));
    wait(&mut ws, "Mic off", &title("Mic off"));

    ws.close(None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let _ = ws.read();
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "plugin exited with {status}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "plugin did not exit after the host closed"
        );
    }
}
