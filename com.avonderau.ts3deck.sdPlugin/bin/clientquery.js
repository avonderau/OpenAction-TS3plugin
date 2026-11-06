// A small client for the TeamSpeak 3 client's ClientQuery interface
// (Tools > Options > Addons > ClientQuery, TCP port 25639 by default).
//
// It keeps one connection open, authenticates with the API key, follows the
// selected server tab and emits a compact "state" object whenever something
// the Deck keys show changes.

import { EventEmitter } from 'node:events';
import net from 'node:net';
import { buildCommand, parseEntry, parseList } from './escape.js';

export const ERR_NOT_CONNECTED = 1794;

const COMMAND_TIMEOUT_MS = 5000;
const GREETING_WAIT_MS = 300;
const REFRESH_DEBOUNCE_MS = 80;
const KEEPALIVE_MS = 30000;
const RETRY_MIN_MS = 1000;
const RETRY_MAX_MS = 30000;

export class QueryError extends Error {
  constructor(id, msg) {
    super(`ClientQuery error ${id}: ${msg}`);
    this.id = id;
    this.msg = msg;
  }
}

// channelPaths turns a channellist reply into cid -> {name, path}.
export function channelPaths(entries) {
  const byId = new Map();
  for (const e of entries) {
    if (e.cid === undefined) continue;
    byId.set(e.cid, { name: e.channel_name ?? '', pid: e.pid ?? '0' });
  }
  const out = new Map();
  const pathOf = (cid, depth = 0) => {
    if (out.has(cid)) return out.get(cid).path;
    const ch = byId.get(cid);
    if (!ch || depth > 64) return '';
    const parent = ch.pid !== '0' && byId.has(ch.pid) ? pathOf(ch.pid, depth + 1) : '';
    const path = parent ? `${parent}/${ch.name}` : ch.name;
    out.set(cid, { name: ch.name, path });
    return path;
  };
  for (const cid of byId.keys()) pathOf(cid);
  return out;
}

// findChannel resolves "Parent/Child" or a plain channel name to a cid.
// A full path wins; a bare name must be unique on the server.
export function findChannel(paths, wanted) {
  const target = String(wanted ?? '').trim().replace(/^\/+|\/+$/g, '').toLowerCase();
  if (!target) return null;
  let byName = [];
  for (const [cid, ch] of paths) {
    if (ch.path.toLowerCase() === target) return cid;
    if (ch.name.toLowerCase() === target) byName.push(cid);
  }
  return byName.length === 1 ? byName[0] : null;
}

const flag = (v) => v === '1';

export class ClientQuery extends EventEmitter {
  constructor({ host = '127.0.0.1', port = 25639, apiKey = '' } = {}) {
    super();
    this.host = host;
    this.port = port;
    this.apiKey = apiKey;
    this.socket = null;
    this.buffer = '';
    this.queue = [];
    this.inflight = null;
    this.ready = false;
    this.stopped = true;
    this.retryMs = RETRY_MIN_MS;
    this.retryTimer = null;
    this.refreshTimer = null;
    this.keepaliveTimer = null;
    this.channels = new Map();
    this.talking = new Set();
    this.state = { status: apiKey ? 'connecting' : 'nokey' };
  }

  // configure changes the connection settings and reconnects when needed.
  configure({ host, port, apiKey }) {
    const next = {
      host: host || '127.0.0.1',
      port: Number(port) || 25639,
      apiKey: (apiKey ?? '').trim(),
    };
    if (next.host === this.host && next.port === this.port && next.apiKey === this.apiKey) return;
    Object.assign(this, next);
    if (!this.stopped) {
      this.disconnect();
      this.start();
    }
  }

  start() {
    this.stopped = false;
    if (!this.apiKey) {
      this.setState({ status: 'nokey' });
      return;
    }
    this.connect();
  }

  stop() {
    this.stopped = true;
    this.disconnect();
  }

  disconnect() {
    clearTimeout(this.retryTimer);
    clearTimeout(this.refreshTimer);
    clearInterval(this.keepaliveTimer);
    this.retryTimer = this.refreshTimer = this.keepaliveTimer = null;
    if (this.socket) {
      this.socket.removeAllListeners();
      this.socket.on('error', () => {});
      this.socket.destroy();
      this.socket = null;
    }
    this.failPending(new Error('disconnected'));
    this.ready = false;
    this.buffer = '';
  }

  connect() {
    this.setState({ status: 'connecting' });
    const socket = net.createConnection({ host: this.host, port: this.port });
    this.socket = socket;
    socket.setEncoding('utf8');
    let greetingTimer = null;
    let greeted = false;
    const greet = () => {
      if (greeted) return;
      greeted = true;
      clearTimeout(greetingTimer);
      this.ready = true;
      this.pump();
      this.handshake();
    };
    socket.on('connect', () => {
      // Some versions never send the greeting's last line; don't hang.
      greetingTimer = setTimeout(greet, GREETING_WAIT_MS * 5);
    });
    socket.on('data', (chunk) => {
      this.buffer += chunk;
      const lines = this.buffer.split(/[\r\n]+/);
      this.buffer = lines.pop();
      for (const line of lines) {
        if (!line) continue;
        if (!greeted) {
          if (line.startsWith('TS3 Client')) {
            clearTimeout(greetingTimer);
            greetingTimer = setTimeout(greet, GREETING_WAIT_MS);
          } else if (line.startsWith('selected ')) {
            greet();
          }
          continue;
        }
        this.onLine(line);
      }
    });
    socket.on('error', (err) => this.emit('log', `ClientQuery socket error: ${err.message}`));
    socket.on('close', () => {
      clearTimeout(greetingTimer);
      if (this.socket !== socket) return;
      this.socket = null;
      this.ready = false;
      this.failPending(new Error('connection closed'));
      clearInterval(this.keepaliveTimer);
      if (this.state.status !== 'badkey') this.setState({ status: 'offline' });
      this.scheduleReconnect();
    });
  }

  scheduleReconnect(delay = this.retryMs) {
    if (this.stopped || this.retryTimer) return;
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      if (!this.stopped && !this.socket) this.connect();
    }, delay);
    this.retryMs = Math.min(this.retryMs * 2, RETRY_MAX_MS);
  }

  async handshake() {
    try {
      await this.command('auth', { apikey: this.apiKey });
    } catch (err) {
      if (err instanceof QueryError) {
        this.emit('log', `ClientQuery rejected the API key (${err.msg})`);
        this.setState({ status: 'badkey' });
        // Retry slowly in case the key was changed on the TeamSpeak side.
        this.retryMs = RETRY_MAX_MS;
        this.socket?.destroy();
      }
      return;
    }
    try {
      await this.command('clientnotifyregister', { schandlerid: 0, event: 'any' });
      this.retryMs = RETRY_MIN_MS;
      this.keepaliveTimer = setInterval(() => this.refreshSoon(), KEEPALIVE_MS);
      await this.refresh();
    } catch (err) {
      this.emit('log', `ClientQuery setup failed: ${err.message}`);
      this.socket?.destroy();
    }
  }

  // command sends one query command and resolves with its data entries.
  command(name, params, options) {
    return this.raw(buildCommand(name, params), options);
  }

  // raw sends a pre-built command line (for commands with bare flags).
  raw(line, { list = false } = {}) {
    return new Promise((resolve, reject) => {
      if (!this.socket) {
        reject(new Error('TeamSpeak is not reachable'));
        return;
      }
      this.queue.push({ line, list, resolve, reject, data: [] });
      this.pump();
    });
  }

  pump() {
    if (!this.ready || this.inflight || !this.queue.length || !this.socket) return;
    const cmd = this.queue.shift();
    this.inflight = cmd;
    cmd.timer = setTimeout(() => {
      this.emit('log', `ClientQuery timed out on "${cmd.line.split(' ')[0]}"`);
      this.socket?.destroy();
    }, COMMAND_TIMEOUT_MS);
    this.socket.write(cmd.line + '\n');
  }

  failPending(err) {
    const pending = this.inflight ? [this.inflight, ...this.queue] : [...this.queue];
    this.inflight = null;
    this.queue = [];
    for (const cmd of pending) {
      clearTimeout(cmd.timer);
      cmd.reject(err);
    }
  }

  onLine(line) {
    if (line.startsWith('notify')) {
      const space = line.indexOf(' ');
      const name = space < 0 ? line : line.slice(0, space);
      this.onNotify(name, parseEntry(space < 0 ? '' : line.slice(space + 1)));
      return;
    }
    const cmd = this.inflight;
    if (!cmd) return;
    if (line.startsWith('error ')) {
      const result = parseEntry(line.slice(6));
      const id = Number(result.id);
      clearTimeout(cmd.timer);
      this.inflight = null;
      if (id === 0) cmd.resolve(cmd.list ? cmd.data.flatMap(parseList) : Object.assign({}, ...cmd.data.map(parseEntry)));
      else cmd.reject(new QueryError(id, result.msg ?? ''));
      this.pump();
      return;
    }
    cmd.data.push(line);
  }

  onNotify(name, data) {
    const current = String(this.state.schid ?? '');
    switch (name) {
      case 'notifytalkstatuschange':
        if (data.schandlerid !== current) return;
        if (data.status === '1') this.talking.add(data.clid);
        else this.talking.delete(data.clid);
        if (data.clid === String(this.state.clid)) this.setState({ talking: data.status === '1' });
        return;
      case 'notifycurrentserverconnectionchanged':
        this.refreshSoon();
        return;
      case 'notifyclientupdated':
      case 'notifyclientmoved':
      case 'notifycliententerview':
      case 'notifyclientleftview':
      case 'notifyconnectstatuschange':
      case 'notifychanneledited':
      case 'notifychannelcreated':
      case 'notifychanneldeleted':
      case 'notifychannelmoved':
        if (data.schandlerid === undefined || data.schandlerid === current) this.refreshSoon();
        return;
      default:
    }
  }

  refreshSoon() {
    if (this.refreshTimer) return;
    this.refreshTimer = setTimeout(() => {
      this.refreshTimer = null;
      this.refresh().catch((err) => this.emit('log', `refresh failed: ${err.message}`));
    }, REFRESH_DEBOUNCE_MS);
  }

  // refresh reads everything the keys show from the selected server tab.
  async refresh() {
    const { schandlerid } = await this.command('currentschandlerid');
    const schid = Number(schandlerid);
    if (schid !== this.state.schid) this.talking.clear();
    await this.command('use', { schandlerid: schid });
    let me;
    try {
      me = await this.command('whoami');
    } catch (err) {
      if (err instanceof QueryError && err.id === ERR_NOT_CONNECTED) {
        this.channels = new Map();
        this.setState({ status: 'disconnected', schid, clid: null, cid: null, channelName: '',
          channelPath: '', serverName: '', users: 0, talking: false });
        return;
      }
      throw err;
    }
    const clid = Number(me.clid);
    const cid = Number(me.cid);
    // clientvariable takes bare property names, so send it as a raw line.
    const vars = await this.raw(`clientvariable clid=${clid} client_nickname client_input_muted client_output_muted client_away client_away_message client_input_deactivated`);
    const channels = channelPaths(await this.command('channellist', undefined, { list: true }));
    this.channels = channels;
    const clients = await this.command('clientlist', undefined, { list: true });
    const users = clients.filter((c) => Number(c.cid) === cid).length;
    const server = await this.raw('servervariable virtualserver_name').catch(() => ({}));
    const channel = channels.get(String(cid)) ?? { name: '', path: '' };
    this.setState({
      status: 'connected',
      schid,
      clid,
      cid,
      nickname: vars.client_nickname ?? '',
      micMuted: flag(vars.client_input_muted),
      speakersMuted: flag(vars.client_output_muted),
      away: flag(vars.client_away),
      awayMessage: vars.client_away_message ?? '',
      transmitting: vars.client_input_deactivated !== '1',
      channelName: channel.name,
      channelPath: channel.path,
      serverName: server.virtualserver_name ?? '',
      users,
      talking: this.talking.has(String(clid)),
    });
  }

  setState(patch) {
    const next = { ...this.state, ...patch };
    if (JSON.stringify(next) === JSON.stringify(this.state)) return;
    this.state = next;
    this.emit('state', next);
  }

  requireConnected() {
    if (this.state.status !== 'connected') {
      throw new Error(this.state.status === 'disconnected' ? 'not connected to a server' : 'TeamSpeak is not reachable');
    }
  }

  async update(params) {
    this.requireConnected();
    await this.command('clientupdate', params);
    await this.refresh();
  }

  setMicMuted(muted) {
    return this.update({ client_input_muted: muted ? 1 : 0 });
  }

  toggleMic() {
    return this.setMicMuted(!this.state.micMuted);
  }

  setSpeakersMuted(muted) {
    return this.update({ client_output_muted: muted ? 1 : 0 });
  }

  toggleSpeakers() {
    return this.setSpeakersMuted(!this.state.speakersMuted);
  }

  setAway(away, message = '') {
    return this.update(away ? { client_away: 1, client_away_message: message } : { client_away: 0 });
  }

  toggleAway(message) {
    return this.setAway(!this.state.away, message);
  }

  // setTransmit opens or closes the mic for push-to-talk. It works when
  // TeamSpeak's capture mode is "Continuous Transmission".
  setTransmit(on) {
    return this.update({ client_input_deactivated: on ? 0 : 1 });
  }

  async joinChannel(channel, password = '') {
    this.requireConnected();
    if (!this.channels.size) await this.refresh();
    const cid = findChannel(this.channels, channel);
    if (!cid) throw new Error(`channel "${channel}" not found`);
    if (Number(cid) === this.state.cid) return;
    await this.command('clientmove', { cid, clid: this.state.clid, cpw: password || undefined });
    await this.refresh();
  }
}
