// Settings page for the TeamSpeak 3 Deck plugin (Stream Deck property
// inspector API: the host calls connectElgatoStreamDeckSocket).
(function () {
  'use strict';
  const PREFIX = 'com.avonderau.ts3deck.';
  const PER_KEY = ['channel', 'password', 'label', 'message'];
  const STATUS_TEXT = {
    nokey: ['Enter your ClientQuery API key below.', 'bad'],
    badkey: ['TeamSpeak rejected the API key.', 'bad'],
    connecting: ['Connecting to TeamSpeak…', ''],
    offline: ['TeamSpeak 3 is not running, or ClientQuery is turned off.', 'bad'],
    disconnected: ['TeamSpeak is running but not connected to a server.', ''],
  };

  let ws = null;
  let uuid = '';
  let action = '';
  let settings = {};
  let waiting = null;
  const $ = (id) => document.getElementById(id);

  function send(event, payload) {
    if (ws && ws.readyState === 1) ws.send(JSON.stringify({ event, context: uuid, action, payload }));
  }

  // expectReply warns when the plugin does not answer, which usually means
  // its process is not running (see the key's title and the plugin log).
  function expectReply(text) {
    const el = $('status');
    el.textContent = text;
    el.className = 'status';
    clearTimeout(waiting);
    waiting = setTimeout(function () {
      el.textContent = 'The plugin is not responding. If the key says "Plugin crashed" or "Plugin host offline", check the plugin log in ~/.local/state/OpenLinkHub/deck-plugins/logs/.';
      el.className = 'status bad';
    }, 4000);
  }

  function showStatus(p) {
    clearTimeout(waiting);
    const el = $('status');
    let text;
    let cls;
    if (p.status === 'connected') {
      text = `Connected as ${p.nickname || '?'} on ${p.serverName || 'server'}${p.channelPath ? ' · ' + p.channelPath : ''}`;
      cls = 'ok';
      if (action === PREFIX + 'channel') send('sendToPlugin', { type: 'channels' });
    } else {
      [text, cls] = STATUS_TEXT[p.status] || ['Unknown state', ''];
    }
    el.textContent = text;
    el.className = 'status ' + cls;
    if (p.connection) {
      if (!$('host').value && p.connection.host !== '127.0.0.1') $('host').value = p.connection.host;
      if (!$('port').value && p.connection.port !== 25639) $('port').value = p.connection.port;
      $('apiKey').placeholder = p.connection.hasKey ? 'Saved (enter a new key to replace it)' : 'XXXX-XXXX-…';
    }
  }

  function fillChannels(list) {
    const dl = $('channel-list');
    dl.textContent = '';
    for (const path of list) {
      const opt = document.createElement('option');
      opt.value = path;
      dl.appendChild(opt);
    }
  }

  function saveKeySettings() {
    for (const id of PER_KEY) settings[id] = $(id).value;
    send('setSettings', settings);
  }

  window.connectElgatoStreamDeckSocket = function (port, inUUID, registerEvent, info, actionInfo) {
    uuid = inUUID;
    try {
      const a = JSON.parse(actionInfo || '{}');
      action = a.action || '';
      settings = (a.payload && a.payload.settings) || {};
    } catch (e) { /* keep defaults */ }
    const section = $('action-' + action.replace(PREFIX, ''));
    if (section) section.hidden = false;
    for (const id of PER_KEY) {
      $(id).value = settings[id] || '';
      $(id).addEventListener('change', saveKeySettings);
    }
    ws = new WebSocket('ws://127.0.0.1:' + port);
    ws.onopen = function () {
      ws.send(JSON.stringify({ event: registerEvent, uuid }));
      send('sendToPlugin', { type: 'status' });
      expectReply('Checking…');
    };
    ws.onmessage = function (evt) {
      let msg;
      try { msg = JSON.parse(evt.data); } catch (e) { return; }
      if (msg.event === 'sendToPropertyInspector' && msg.payload) {
        if (msg.payload.type === 'status') showStatus(msg.payload);
        else if (msg.payload.type === 'channels') fillChannels(msg.payload.channels || []);
      } else if (msg.event === 'didReceiveSettings' && msg.payload) {
        settings = msg.payload.settings || {};
      }
    };
  };
  window.connectSocket = window.connectElgatoStreamDeckSocket;

  $('save').addEventListener('click', function () {
    send('sendToPlugin', {
      type: 'saveConnection',
      host: $('host').value,
      port: $('port').value,
      apiKey: $('apiKey').value || undefined,
    });
    $('apiKey').value = '';
    expectReply('Saving…');
  });
})();
