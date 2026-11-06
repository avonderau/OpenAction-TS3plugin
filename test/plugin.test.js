import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { test } from 'node:test';
import { ClientQuery } from '../com.avonderau.ts3deck.sdPlugin/bin/clientquery.js';
import { ACTIONS, Plugin, view } from '../com.avonderau.ts3deck.sdPlugin/bin/plugin.js';
import { startMockTs3 } from './mock-ts3.js';

// FakeDeck records what the plugin sends instead of talking WebSocket.
class FakeDeck extends EventEmitter {
  constructor() {
    super();
    this.sent = [];
    this.uuid = 'plugin-token';
  }
  record(event, context, payload) { this.sent.push({ event, context, payload }); }
  setTitle(context, title) { this.record('setTitle', context, title); }
  setState(context, state) { this.record('setState', context, state); }
  setSettings(context, s) { this.record('setSettings', context, s); }
  getGlobalSettings() { this.record('getGlobalSettings'); }
  setGlobalSettings(s) { this.record('setGlobalSettings', null, s); }
  sendToPropertyInspector(context, action, payload) { this.record('sendToPropertyInspector', context, payload); }
  showAlert(context) { this.record('showAlert', context); }
  showOk(context) { this.record('showOk', context); }
  log(m) { this.record('log', null, m); }
  last(event, context) {
    for (let i = this.sent.length - 1; i >= 0; i--) {
      const m = this.sent[i];
      if (m.event === event && m.context === context) return m.payload;
    }
    return undefined;
  }
}

const until = async (fn, ms = 3000) => {
  const end = Date.now() + ms;
  for (;;) {
    try {
      fn();
      return;
    } catch (err) {
      if (Date.now() > end) throw err;
      await new Promise((r) => setTimeout(r, 20));
    }
  }
};

test('view shows offline states on every key', () => {
  for (const action of Object.values(ACTIONS)) {
    assert.deepEqual(view(action, { status: 'nokey' }), { title: 'Set API\nkey', state: 0 });
  }
  assert.equal(view(ACTIONS.channel, { status: 'disconnected' }, { channel: 'Games/Squad 1' }).title, 'Squad 1');
});

test('view reflects connected state', () => {
  const ts = { status: 'connected', micMuted: true, speakersMuted: false, away: true, transmitting: true,
    cid: 3, channelName: 'Squad 1', channelPath: 'Games/Squad 1', users: 1, talking: true };
  assert.deepEqual(view(ACTIONS.mic, ts), { title: 'Mic off', state: 1 });
  assert.deepEqual(view(ACTIONS.speakers, ts), { title: 'Sound on', state: 0 });
  assert.deepEqual(view(ACTIONS.away, ts), { title: 'Away', state: 1 });
  assert.deepEqual(view(ACTIONS.ptt, ts, {}, { held: true }), { title: 'TALK', state: 1 });
  assert.deepEqual(view(ACTIONS.ptt, ts, {}, { held: false }), { title: 'PTT', state: 0 });
  const channels = new Map([['3', { name: 'Squad 1', path: 'Games/Squad 1' }], ['1', { name: 'Lobby', path: 'Lobby' }]]);
  assert.deepEqual(view(ACTIONS.channel, ts, { channel: 'games/squad 1' }, { channels }), { title: 'Squad 1', state: 1 });
  assert.deepEqual(view(ACTIONS.channel, ts, { channel: 'lobby' }, { channels }), { title: 'Lobby', state: 0 });
  assert.deepEqual(view(ACTIONS.channel, ts, { channel: 'Games/Squad 1' }), { title: 'Squad 1', state: 1 });
  assert.deepEqual(view(ACTIONS.channel, ts, { channel: 'Lobby', label: 'Home' }), { title: 'Home', state: 0 });
  assert.deepEqual(view(ACTIONS.status, ts), { title: 'Squad 1\n1 user', state: 1 });
  assert.equal(view(ACTIONS.status, { ...ts, channelName: 'A very long channel name' }).title.split('\n')[0], 'A very lo…');
});

test('keys drive TeamSpeak and follow its state', async (t) => {
  const ts3 = await startMockTs3();
  const deck = new FakeDeck();
  const cq = new ClientQuery();
  new Plugin(deck, cq);
  t.after(async () => {
    cq.stop();
    await ts3.close();
  });

  deck.emit('open');
  assert.ok(deck.sent.some((m) => m.event === 'getGlobalSettings'));
  deck.emit('willAppear', { action: ACTIONS.mic, context: 'k-mic', payload: { settings: {} } });
  assert.equal(deck.last('setTitle', 'k-mic'), 'TS3\n…');

  deck.emit('didReceiveGlobalSettings', { payload: { settings: { port: ts3.port, apiKey: 'KEY-123' } } });
  await until(() => assert.equal(deck.last('setTitle', 'k-mic'), 'Mic on'));

  deck.emit('willAppear', { action: ACTIONS.channel, context: 'k-ch', payload: { settings: { channel: 'Games/Squad 1' } } });
  deck.emit('willAppear', { action: ACTIONS.status, context: 'k-st', payload: { settings: {} } });
  deck.emit('willAppear', { action: ACTIONS.ptt, context: 'k-ptt', payload: { settings: {} } });
  deck.emit('willAppear', { action: ACTIONS.away, context: 'k-away', payload: { settings: { message: 'lunch' } } });
  assert.equal(deck.last('setTitle', 'k-st'), 'Lobby\n2 users');
  assert.equal(deck.last('setState', 'k-ch'), 0);

  deck.emit('keyDown', { action: ACTIONS.mic, context: 'k-mic' });
  await until(() => assert.equal(deck.last('setState', 'k-mic'), 1));
  assert.equal(ts3.state.client_input_muted, 1);

  deck.emit('keyDown', { action: ACTIONS.channel, context: 'k-ch' });
  await until(() => assert.equal(deck.last('setState', 'k-ch'), 1));
  assert.equal(deck.last('setTitle', 'k-st'), 'Squad 1\n2 users');

  deck.emit('keyDown', { action: ACTIONS.away, context: 'k-away' });
  await until(() => assert.equal(ts3.state.client_away_message, 'lunch'));

  deck.emit('keyDown', { action: ACTIONS.ptt, context: 'k-ptt' });
  await until(() => assert.equal(deck.last('setTitle', 'k-ptt'), 'TALK'));
  deck.emit('keyUp', { action: ACTIONS.ptt, context: 'k-ptt' });
  await until(() => assert.equal(ts3.state.client_input_deactivated, 1));
  assert.equal(deck.last('setTitle', 'k-ptt'), 'PTT');

  // A PTT key that leaves the page while held must close the mic.
  deck.emit('keyDown', { action: ACTIONS.ptt, context: 'k-ptt' });
  await until(() => assert.equal(ts3.state.client_input_deactivated, 0));
  deck.emit('willDisappear', { action: ACTIONS.ptt, context: 'k-ptt' });
  await until(() => assert.equal(ts3.state.client_input_deactivated, 1));

  // A failing action shows the alert mark.
  deck.emit('didReceiveSettings', { action: ACTIONS.channel, context: 'k-ch', payload: { settings: { channel: 'Nowhere' } } });
  deck.emit('keyDown', { action: ACTIONS.channel, context: 'k-ch' });
  await until(() => assert.ok(deck.sent.some((m) => m.event === 'showAlert' && m.context === 'k-ch')));
});

test('settings page saves the connection without losing the key', async (t) => {
  const ts3 = await startMockTs3();
  const deck = new FakeDeck();
  const cq = new ClientQuery();
  new Plugin(deck, cq);
  t.after(async () => {
    cq.stop();
    await ts3.close();
  });
  deck.emit('open');
  deck.emit('didReceiveGlobalSettings', { payload: { settings: {} } });
  deck.emit('willAppear', { action: ACTIONS.mic, context: 'k', payload: { settings: {} } });
  deck.emit('propertyInspectorDidAppear', { action: ACTIONS.mic, context: 'k' });
  assert.equal(deck.last('sendToPropertyInspector', 'k').status, 'nokey');

  deck.emit('sendToPlugin', { action: ACTIONS.mic, context: 'k',
    payload: { type: 'saveConnection', host: '', port: String(ts3.port), apiKey: 'KEY-123' } });
  await until(() => assert.equal(deck.last('sendToPropertyInspector', 'k').status, 'connected'));
  assert.deepEqual(deck.last('setGlobalSettings', null), { host: '127.0.0.1', port: ts3.port, apiKey: 'KEY-123' });

  // Saving again with an empty key field keeps the saved key.
  deck.emit('sendToPlugin', { action: ACTIONS.mic, context: 'k',
    payload: { type: 'saveConnection', host: '127.0.0.1', port: String(ts3.port) } });
  assert.equal(deck.last('setGlobalSettings', null).apiKey, 'KEY-123');

  deck.emit('sendToPlugin', { action: ACTIONS.mic, context: 'k', payload: { type: 'channels' } });
  assert.ok(deck.last('sendToPropertyInspector', 'k').channels.includes('Games/Squad 2'));
});
