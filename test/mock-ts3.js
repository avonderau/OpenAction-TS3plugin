// A fake TeamSpeak 3 ClientQuery server for tests. It speaks the same line
// protocol (\n\r line ends, "error id=… msg=…" trailers, notify events).

import net from 'node:net';
import { buildCommand, escape, parseEntry } from '../com.avonderau.ts3deck.sdPlugin/bin/escape.js';

export function startMockTs3({ apiKey = 'KEY-123', connected = true } = {}) {
  const state = {
    connected,
    clid: 5,
    cid: 1,
    nickname: 'Andrew',
    client_input_muted: 0,
    client_output_muted: 0,
    client_away: 0,
    client_away_message: '',
    client_input_deactivated: 0,
    channels: [
      { cid: 1, pid: 0, channel_name: 'Lobby' },
      { cid: 2, pid: 0, channel_name: 'Games' },
      { cid: 3, pid: 2, channel_name: 'Squad 1' },
      { cid: 4, pid: 2, channel_name: 'Squad 2' },
      { cid: 5, pid: 0, channel_name: 'AFK' },
    ],
    others: [
      { clid: 6, cid: 1, client_nickname: 'Bea' },
      { clid: 7, cid: 3, client_nickname: 'Cy' },
    ],
    passwords: { 4: 'secret' },
    log: [],
  };
  const sockets = new Set();

  const server = net.createServer((sock) => {
    sockets.add(sock);
    sock.on('close', () => sockets.delete(sock));
    sock.on('error', () => {});
    let authed = false;
    let buf = '';
    const out = (line) => sock.write(line + '\n\r');
    const ok = () => out('error id=0 msg=ok');
    out('TS3 Client');
    out('Welcome to the TeamSpeak 3 ClientQuery interface, type "help" for a list of commands and "help <command>" for information on a specific command.');
    out('Use the "auth" command to authenticate yourself. Your API key can be found in the client\'s ClientQuery settings.');
    out('selected schandlerid=1');
    sock.on('data', (chunk) => {
      buf += chunk.toString('utf8');
      const lines = buf.split('\n');
      buf = lines.pop();
      for (const raw of lines) {
        const line = raw.trim();
        if (!line) continue;
        state.log.push(line);
        const space = line.indexOf(' ');
        const cmd = space < 0 ? line : line.slice(0, space);
        const args = parseEntry(space < 0 ? '' : line.slice(space + 1));
        if (cmd === 'auth') {
          if (args.apikey === apiKey) {
            authed = true;
            ok();
          } else out('error id=256 msg=invalid\\sapikey');
          continue;
        }
        if (!authed) {
          out('error id=1796 msg=currently\\snot\\spossible');
          continue;
        }
        handle(cmd, args, out, ok);
      }
    });
  });

  function broadcast(line) {
    for (const s of sockets) s.write(line + '\n\r');
  }

  function handle(cmd, args, out, ok) {
    switch (cmd) {
      case 'clientnotifyregister':
      case 'use':
        return ok();
      case 'currentschandlerid':
        out('schandlerid=1');
        return ok();
      case 'whoami':
        if (!state.connected) return out('error id=1794 msg=not\\sconnected');
        out(`clid=${state.clid} cid=${state.cid}`);
        return ok();
      case 'clientvariable':
        out(`clid=${state.clid} client_nickname=${escape(state.nickname)} client_input_muted=${state.client_input_muted} client_output_muted=${state.client_output_muted} client_away=${state.client_away} client_away_message=${escape(state.client_away_message)} client_input_deactivated=${state.client_input_deactivated}`);
        return ok();
      case 'channellist':
        out(state.channels.map((c) => buildCommand('', c).trim()).join('|'));
        return ok();
      case 'clientlist': {
        const all = [{ clid: state.clid, cid: state.cid, client_nickname: state.nickname }, ...state.others];
        out(all.map((c) => buildCommand('', c).trim()).join('|'));
        return ok();
      }
      case 'servervariable':
        out('virtualserver_name=Test\\sServer');
        return ok();
      case 'clientupdate': {
        const changed = {};
        for (const [k, v] of Object.entries(args)) {
          if (!(k in state)) return out('error id=1538 msg=invalid\\sparameter');
          state[k] = typeof state[k] === 'number' ? Number(v) : v;
          changed[k] = v;
        }
        ok();
        return broadcast(buildCommand('notifyclientupdated', { schandlerid: 1, clid: state.clid, ...changed }));
      }
      case 'clientmove': {
        const cid = Number(args.cid);
        if (!state.channels.some((c) => c.cid === cid)) return out('error id=768 msg=invalid\\schannelID');
        if (state.passwords[cid] && args.cpw !== state.passwords[cid]) return out('error id=781 msg=invalid\\spassword');
        state.cid = cid;
        ok();
        return broadcast(`notifyclientmoved schandlerid=1 ctid=${cid} reasonid=0 clid=${state.clid}`);
      }
      default:
        return out('error id=256 msg=command\\snot\\sfound');
    }
  }

  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      resolve({
        port: server.address().port,
        state,
        broadcast,
        dropClients() {
          for (const s of sockets) s.destroy();
        },
        close() {
          for (const s of sockets) s.destroy();
          return new Promise((r) => server.close(r));
        },
      });
    });
  });
}
