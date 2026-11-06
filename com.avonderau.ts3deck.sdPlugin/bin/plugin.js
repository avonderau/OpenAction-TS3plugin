#!/usr/bin/env node
// TeamSpeak 3 Deck plugin: keys that control the local TeamSpeak 3 client
// through ClientQuery and show its state.

import { realpathSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { ClientQuery, findChannel } from './clientquery.js';
import { StreamDeck, parseArgs } from './streamdeck.js';

export const PLUGIN = 'com.avonderau.ts3deck';
export const ACTIONS = {
  mic: `${PLUGIN}.mic`,
  speakers: `${PLUGIN}.speakers`,
  away: `${PLUGIN}.away`,
  ptt: `${PLUGIN}.ptt`,
  channel: `${PLUGIN}.channel`,
  status: `${PLUGIN}.status`,
};

const OFFLINE_TITLES = {
  nokey: 'Set API\nkey',
  badkey: 'Bad API\nkey',
  connecting: 'TS3\n…',
  offline: 'TS3\noffline',
  disconnected: 'No\nserver',
};

const MAX_LINE = 10;

export function fit(text, max = MAX_LINE) {
  const s = String(text ?? '');
  return [...s].length > max ? [...s].slice(0, max - 1).join('') + '…' : s;
}

// view returns the {title, state} a key shows for the current TS3 state.
export function view(action, ts, settings = {}, local = {}) {
  if (ts.status !== 'connected') {
    // A channel key keeps its label so you can still tell keys apart.
    if (action === ACTIONS.channel && settings.channel && ts.status === 'disconnected') {
      return { title: fit(channelLabel(settings)), state: 0 };
    }
    return { title: OFFLINE_TITLES[ts.status] ?? 'TS3', state: 0 };
  }
  switch (action) {
    case ACTIONS.mic:
      return { title: ts.micMuted ? 'Mic off' : 'Mic on', state: ts.micMuted ? 1 : 0 };
    case ACTIONS.speakers:
      return { title: ts.speakersMuted ? 'Sound off' : 'Sound on', state: ts.speakersMuted ? 1 : 0 };
    case ACTIONS.away:
      return { title: ts.away ? 'Away' : 'Online', state: ts.away ? 1 : 0 };
    case ACTIONS.ptt: {
      const live = local.held && ts.transmitting;
      return { title: live ? 'TALK' : 'PTT', state: live ? 1 : 0 };
    }
    case ACTIONS.channel: {
      if (!settings.channel) return { title: 'Pick\nchannel', state: 0 };
      const cid = local.channels?.size ? findChannel(local.channels, settings.channel) : null;
      const here = cid !== null ? Number(cid) === ts.cid : inChannel(ts, settings.channel);
      const name = settings.label || (cid !== null ? local.channels.get(cid).name : channelLabel(settings));
      return { title: fit(name), state: here ? 1 : 0 };
    }
    case ACTIONS.status: {
      const users = ts.users === 1 ? '1 user' : `${ts.users} users`;
      return { title: `${fit(ts.channelName)}\n${users}`, state: ts.talking ? 1 : 0 };
    }
    default:
      return { title: '', state: 0 };
  }
}

function channelLabel(settings) {
  if (settings.label) return settings.label;
  const parts = String(settings.channel).split('/').filter(Boolean);
  return parts[parts.length - 1] ?? settings.channel;
}

function inChannel(ts, wanted) {
  const w = String(wanted).trim().toLowerCase();
  return w === String(ts.channelPath).toLowerCase() || w === String(ts.channelName).toLowerCase();
}

export class Plugin {
  constructor(deck, ts) {
    this.deck = deck;
    this.ts = ts;
    this.keys = new Map(); // context -> {action, settings, held, shown}
    this.inspectors = new Map(); // context -> action
    this.globals = null;

    deck.on('open', () => deck.getGlobalSettings());
    deck.on('close', () => {
      ts.stop();
      process.exit(0);
    });
    deck.on('didReceiveGlobalSettings', (msg) => this.onGlobals(msg.payload?.settings ?? {}));
    deck.on('willAppear', (msg) => {
      this.keys.set(msg.context, { action: msg.action, settings: msg.payload?.settings ?? {}, held: false });
      this.render(msg.context);
    });
    deck.on('willDisappear', (msg) => {
      const key = this.keys.get(msg.context);
      this.keys.delete(msg.context);
      // Never leave the mic open because a PTT key scrolled off the page.
      if (key?.held) this.ts.setTransmit(false).catch(() => {});
    });
    deck.on('didReceiveSettings', (msg) => {
      const key = this.keys.get(msg.context);
      if (!key) return;
      key.settings = msg.payload?.settings ?? {};
      this.render(msg.context);
    });
    deck.on('keyDown', (msg) => this.onKey(msg.context, true));
    deck.on('keyUp', (msg) => this.onKey(msg.context, false));
    deck.on('propertyInspectorDidAppear', (msg) => {
      this.inspectors.set(msg.context, msg.action);
      this.sendStatus(msg.context);
    });
    deck.on('propertyInspectorDidDisappear', (msg) => this.inspectors.delete(msg.context));
    deck.on('sendToPlugin', (msg) => this.onInspector(msg));

    ts.on('state', () => {
      for (const context of this.keys.keys()) this.render(context);
      for (const context of this.inspectors.keys()) this.sendStatus(context);
    });
    ts.on('log', (line) => deck.log(line));
  }

  onGlobals(settings) {
    const first = this.globals === null;
    this.globals = settings;
    this.ts.configure({ host: settings.host, port: settings.port, apiKey: settings.apiKey });
    if (first) this.ts.start();
  }

  onInspector(msg) {
    const p = msg.payload ?? {};
    if (p.type === 'saveConnection') {
      const settings = {
        host: String(p.host ?? '').trim() || '127.0.0.1',
        port: Number(p.port) || 25639,
        // An empty key field means "keep the saved key".
        apiKey: String(p.apiKey ?? '').trim() || (this.globals?.apiKey ?? ''),
      };
      this.deck.setGlobalSettings(settings);
      this.onGlobals(settings);
    } else if (p.type === 'status') {
      this.sendStatus(msg.context);
    } else if (p.type === 'channels') {
      const channels = [...this.ts.channels.values()].map((c) => c.path).sort((a, b) => a.localeCompare(b));
      this.deck.sendToPropertyInspector(msg.context, msg.action, { type: 'channels', channels });
    }
  }

  sendStatus(context) {
    const action = this.inspectors.get(context) ?? this.keys.get(context)?.action;
    const s = this.ts.state;
    this.deck.sendToPropertyInspector(context, action, {
      type: 'status',
      status: s.status,
      serverName: s.serverName ?? '',
      channelPath: s.channelPath ?? '',
      nickname: s.nickname ?? '',
      connection: { host: this.ts.host, port: this.ts.port, hasKey: Boolean(this.ts.apiKey) },
    });
  }

  render(context) {
    const key = this.keys.get(context);
    if (!key) return;
    // Until the saved connection settings arrive, don't flash "Set API key".
    const ts = this.globals === null ? { status: 'connecting' } : this.ts.state;
    const v = view(key.action, ts, key.settings, { held: key.held, channels: this.ts.channels });
    if (key.shown?.title !== v.title) this.deck.setTitle(context, v.title);
    if (key.shown?.state !== v.state) this.deck.setState(context, v.state);
    key.shown = v;
  }

  async onKey(context, down) {
    const key = this.keys.get(context);
    if (!key) return;
    try {
      switch (key.action) {
        case ACTIONS.ptt:
          key.held = down;
          this.render(context);
          await this.ts.setTransmit(down);
          return;
        case ACTIONS.mic:
          if (down) await this.ts.toggleMic();
          return;
        case ACTIONS.speakers:
          if (down) await this.ts.toggleSpeakers();
          return;
        case ACTIONS.away:
          if (down) await this.ts.toggleAway(key.settings.message ?? '');
          return;
        case ACTIONS.channel:
          if (!down) return;
          if (!key.settings.channel) throw new Error('no channel set for this key');
          await this.ts.joinChannel(key.settings.channel, key.settings.password ?? '');
          return;
        case ACTIONS.status:
          if (down) await this.ts.refresh();
          return;
        default:
      }
    } catch (err) {
      this.deck.log(`${key.action}: ${err.message}`);
      this.deck.showAlert(context);
    } finally {
      this.render(context);
    }
  }
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (!args.port || !args.uuid) {
    console.error('usage: plugin.js -port <port> -pluginUUID <uuid> -registerEvent <event> -info <json>');
    process.exit(2);
  }
  const deck = new StreamDeck(args);
  const ts = new ClientQuery();
  new Plugin(deck, ts);
  deck.connect();
}

function isEntryPoint() {
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
}

if (isEntryPoint()) main();
