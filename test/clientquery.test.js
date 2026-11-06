import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ClientQuery, channelPaths, findChannel } from '../com.avonderau.ts3deck.sdPlugin/bin/clientquery.js';
import { buildCommand, escape, parseList, unescape } from '../com.avonderau.ts3deck.sdPlugin/bin/escape.js';
import { startMockTs3 } from './mock-ts3.js';

function waitFor(cq, pred, ms = 3000) {
  if (pred(cq.state)) return Promise.resolve(cq.state);
  return new Promise((resolve, reject) => {
    const t = setTimeout(() => {
      cq.off('state', on);
      reject(new Error(`timed out; last state ${JSON.stringify(cq.state)}`));
    }, ms);
    const on = (s) => {
      if (pred(s)) {
        clearTimeout(t);
        cq.off('state', on);
        resolve(s);
      }
    };
    cq.on('state', on);
  });
}

test('escape round-trips query special characters', () => {
  const raw = 'a b/c|d\\e\nf\tg';
  assert.equal(escape(raw), 'a\\sb\\/c\\pd\\\\e\\nf\\tg');
  assert.equal(unescape(escape(raw)), raw);
  assert.equal(buildCommand('clientupdate', { client_away_message: 'brb 5 min', skip: undefined }),
    'clientupdate client_away_message=brb\\s5\\smin');
  assert.deepEqual(parseList('cid=1 channel_name=A\\sB|cid=2 flag'), [
    { cid: '1', channel_name: 'A B' },
    { cid: '2', flag: '' },
  ]);
});

test('channel paths and lookup', () => {
  const paths = channelPaths(parseList(
    'cid=1 pid=0 channel_name=Lobby|cid=2 pid=0 channel_name=Games|cid=3 pid=2 channel_name=Squad\\s1|cid=4 pid=0 channel_name=Squad\\s1'));
  assert.equal(paths.get('3').path, 'Games/Squad 1');
  assert.equal(findChannel(paths, 'games/squad 1'), '3');
  assert.equal(findChannel(paths, '/Lobby/'), '1');
  assert.equal(findChannel(paths, 'Squad 1'), '4', 'a top-level path match wins over the duplicate name');
  assert.equal(findChannel(paths, 'nope'), null);
});

test('connects, reads state, and controls the client', async (t) => {
  const ts3 = await startMockTs3();
  const cq = new ClientQuery({ port: ts3.port, apiKey: 'KEY-123' });
  t.after(async () => {
    cq.stop();
    await ts3.close();
  });
  cq.start();
  const s = await waitFor(cq, (st) => st.status === 'connected');
  assert.equal(s.nickname, 'Andrew');
  assert.equal(s.channelName, 'Lobby');
  assert.equal(s.serverName, 'Test Server');
  assert.equal(s.users, 2);
  assert.equal(s.micMuted, false);

  await cq.toggleMic();
  assert.equal(ts3.state.client_input_muted, 1);
  assert.equal(cq.state.micMuted, true);
  await cq.toggleSpeakers();
  assert.equal(cq.state.speakersMuted, true);

  await cq.toggleAway('back in 5');
  assert.equal(ts3.state.client_away, 1);
  assert.equal(ts3.state.client_away_message, 'back in 5');
  assert.equal(cq.state.away, true);
  await cq.toggleAway();
  assert.equal(cq.state.away, false);

  await cq.setTransmit(false);
  assert.equal(ts3.state.client_input_deactivated, 1);
  assert.equal(cq.state.transmitting, false);
  await cq.setTransmit(true);
  assert.equal(cq.state.transmitting, true);

  await cq.joinChannel('Games/Squad 1');
  assert.equal(cq.state.channelPath, 'Games/Squad 1');
  assert.equal(cq.state.users, 2);
  await assert.rejects(cq.joinChannel('Squad 2', 'wrong'), /invalid password/);
  await cq.joinChannel('Squad 2', 'secret');
  assert.equal(cq.state.cid, 4);
  await assert.rejects(cq.joinChannel('Nowhere'), /not found/);
});

test('follows changes made in TeamSpeak itself', async (t) => {
  const ts3 = await startMockTs3();
  const cq = new ClientQuery({ port: ts3.port, apiKey: 'KEY-123' });
  t.after(async () => {
    cq.stop();
    await ts3.close();
  });
  cq.start();
  await waitFor(cq, (st) => st.status === 'connected');

  ts3.state.client_input_muted = 1;
  ts3.broadcast('notifyclientupdated schandlerid=1 clid=5 client_input_muted=1');
  await waitFor(cq, (st) => st.micMuted);

  ts3.broadcast('notifytalkstatuschange schandlerid=1 status=1 isreceivedwhisper=0 clid=5');
  await waitFor(cq, (st) => st.talking);
  ts3.broadcast('notifytalkstatuschange schandlerid=1 status=0 isreceivedwhisper=0 clid=5');
  await waitFor(cq, (st) => !st.talking);
});

test('reports a rejected API key and a client without a server', async (t) => {
  const ts3 = await startMockTs3({ connected: false });
  const bad = new ClientQuery({ port: ts3.port, apiKey: 'WRONG' });
  const good = new ClientQuery({ port: ts3.port, apiKey: 'KEY-123' });
  t.after(async () => {
    bad.stop();
    good.stop();
    await ts3.close();
  });
  bad.start();
  await waitFor(bad, (st) => st.status === 'badkey');
  good.start();
  await waitFor(good, (st) => st.status === 'disconnected');
  await assert.rejects(good.toggleMic(), /not connected to a server/);
});

test('reconnects after TeamSpeak restarts', async (t) => {
  const ts3 = await startMockTs3();
  const cq = new ClientQuery({ port: ts3.port, apiKey: 'KEY-123' });
  t.after(async () => {
    cq.stop();
    await ts3.close();
  });
  cq.start();
  await waitFor(cq, (st) => st.status === 'connected');
  ts3.dropClients();
  await waitFor(cq, (st) => st.status === 'offline');
  await waitFor(cq, (st) => st.status === 'connected', 5000);
});

test('waits for an API key before connecting', async () => {
  const cq = new ClientQuery({ port: 1, apiKey: '' });
  cq.start();
  assert.equal(cq.state.status, 'nokey');
  cq.stop();
});
