// Minimal Stream Deck SDK (v2) plugin connection. Works with the Elgato app,
// OpenDeck and the OpenLinkHub Nexus Deck plugin host, which all launch the
// plugin with -port, -pluginUUID, -registerEvent and -info.

import { EventEmitter } from 'node:events';
import { OPEN, WebSocketClient } from './websocket.js';

export function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i++) {
    if (argv[i].startsWith('-') && i + 1 < argv.length) args[argv[i].slice(1)] = argv[++i];
  }
  let info = {};
  try {
    info = JSON.parse(args.info ?? '{}');
  } catch {
    info = {};
  }
  return { port: args.port, uuid: args.pluginUUID, registerEvent: args.registerEvent ?? 'registerPlugin', info };
}

export class StreamDeck extends EventEmitter {
  constructor({ port, uuid, registerEvent }) {
    super();
    this.port = port;
    this.uuid = uuid;
    this.registerEvent = registerEvent;
    this.ws = null;
  }

  connect() {
    const ws = new WebSocketClient('127.0.0.1', Number(this.port));
    this.ws = ws;
    ws.on('open', () => {
      ws.send(JSON.stringify({ event: this.registerEvent, uuid: this.uuid }));
      this.emit('open');
    });
    ws.on('message', (text) => {
      let data;
      try {
        data = JSON.parse(text);
      } catch {
        return;
      }
      if (data && typeof data.event === 'string') this.emit(data.event, data);
    });
    ws.on('close', () => this.emit('close'));
    ws.on('error', (err) => console.error(`Deck connection: ${err.message}`));
  }

  send(payload) {
    if (this.ws?.readyState === OPEN) this.ws.send(JSON.stringify(payload));
  }

  setTitle(context, title) {
    this.send({ event: 'setTitle', context, payload: { title, target: 0 } });
  }

  setState(context, state) {
    this.send({ event: 'setState', context, payload: { state } });
  }

  setSettings(context, settings) {
    this.send({ event: 'setSettings', context, payload: settings });
  }

  getGlobalSettings() {
    this.send({ event: 'getGlobalSettings', context: this.uuid });
  }

  setGlobalSettings(settings) {
    this.send({ event: 'setGlobalSettings', context: this.uuid, payload: settings });
  }

  sendToPropertyInspector(context, action, payload) {
    this.send({ event: 'sendToPropertyInspector', context, action, payload });
  }

  showAlert(context) {
    this.send({ event: 'showAlert', context });
  }

  showOk(context) {
    this.send({ event: 'showOk', context });
  }

  log(message) {
    this.send({ event: 'logMessage', payload: { message: String(message) } });
  }
}
