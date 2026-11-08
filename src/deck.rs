//! The plugin's WebSocket connection to the Deck host (Stream Deck SDK v2).
//! Hosts start the plugin with `-port`, `-pluginUUID`, `-registerEvent` and
//! `-info`.

use std::io;
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use serde_json::{json, Value};
use tungstenite::{Message, WebSocket};

use crate::clientquery::{self, Request, TsEvent};
use crate::plugin::Plugin;

const POLL: Duration = Duration::from_millis(25);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub port: u16,
    pub uuid: String,
    pub register_event: String,
}

pub fn parse_args<I: IntoIterator<Item = String>>(argv: I) -> Result<Args, String> {
    let mut args = Args {
        register_event: "registerPlugin".into(),
        ..Args::default()
    };
    let mut it = argv.into_iter();
    while let Some(flag) = it.next() {
        // Hosts spell the flags differently ("-pluginUUID", "-pluginuuid").
        let name = flag.to_ascii_lowercase();
        let value = match name.as_str() {
            "-port" | "-pluginuuid" | "-registerevent" | "-info" => {
                it.next().ok_or(format!("{flag} needs a value"))?
            }
            _ => continue,
        };
        match name.as_str() {
            "-port" => args.port = value.parse().map_err(|_| format!("bad -port {value}"))?,
            "-pluginuuid" => args.uuid = value,
            "-registerevent" => args.register_event = value,
            _ => {}
        }
    }
    if args.port == 0 || args.uuid.is_empty() {
        return Err(
            "usage: ts3deck -port <port> -pluginUUID <uuid> -registerEvent <event> -info <json>"
                .into(),
        );
    }
    Ok(args)
}

/// Connects to the host and runs the plugin until the host closes the
/// connection.
pub fn run(args: &Args) -> Result<(), String> {
    let stream = TcpStream::connect(("127.0.0.1", args.port))
        .map_err(|e| format!("connect to host: {e}"))?;
    stream.set_nodelay(true).ok();
    // No Origin header: hosts refuse browser connections that carry one.
    let url = format!("ws://127.0.0.1:{}", args.port);
    let (mut ws, _) = tungstenite::client(url.as_str(), stream)
        .map_err(|e| format!("WebSocket handshake: {e}"))?;
    ws.get_ref()
        .set_read_timeout(Some(POLL))
        .map_err(|e| e.to_string())?;
    send(
        &mut ws,
        &json!({"event": args.register_event, "uuid": args.uuid}),
    )?;

    let (req_tx, req_rx) = mpsc::channel::<Request>();
    let (ev_tx, ev_rx) = mpsc::channel::<TsEvent>();
    let worker = clientquery::spawn(req_rx, move |e| {
        let _ = ev_tx.send(e);
    });
    let mut plugin = Plugin::new(args.uuid.clone(), req_tx);
    plugin.on_open();
    let result = pump(&mut ws, &mut plugin, &ev_rx);
    drop(plugin); // closes the request channel, which stops the worker
    let _ = worker.join();
    result
}

fn pump(
    ws: &mut WebSocket<TcpStream>,
    plugin: &mut Plugin,
    events: &Receiver<TsEvent>,
) -> Result<(), String> {
    loop {
        for msg in plugin.outbox.drain(..) {
            send(ws, &msg)?;
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                if let Ok(msg) = serde_json::from_str::<Value>(&text) {
                    plugin.on_deck(&msg);
                }
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Ok(())
            }
            Err(e) => return Err(format!("host connection: {e}")),
        }
        loop {
            match events.try_recv() {
                Ok(e) => plugin.on_ts(e),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Err("TeamSpeak worker stopped".into()),
            }
        }
    }
}

fn send(ws: &mut WebSocket<TcpStream>, msg: &Value) -> Result<(), String> {
    ws.send(Message::Text(msg.to_string()))
        .map_err(|e| format!("send to host: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_host_arguments() {
        let a = parse_args(strings(&[
            "-port",
            "28196",
            "-pluginUUID",
            "abc",
            "-registerEvent",
            "registerPlugin",
            "-info",
            "{}",
        ]))
        .unwrap();
        assert_eq!(
            a,
            Args {
                port: 28196,
                uuid: "abc".into(),
                register_event: "registerPlugin".into()
            }
        );
        assert!(parse_args(strings(&["-port", "x", "-pluginUUID", "a"])).is_err());
        assert!(parse_args(strings(&["-pluginUUID", "a"])).is_err());
        let a = parse_args(strings(&[
            "-PORT",
            "1",
            "-pluginuuid",
            "u",
            "-registerEvent",
            "r",
        ]))
        .unwrap();
        assert_eq!((a.port, a.uuid.as_str()), (1, "u"));
    }
}
