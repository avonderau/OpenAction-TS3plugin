import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import net from 'node:net';
import { test } from 'node:test';
import { WebSocketClient } from '../com.avonderau.ts3deck.sdPlugin/bin/websocket.js';

const GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';

function frame(opcode, payload, fin = true) {
  const body = Buffer.from(payload);
  let head;
  if (body.length < 126) head = Buffer.from([(fin ? 0x80 : 0) | opcode, body.length]);
  else if (body.length < 65536) {
    head = Buffer.alloc(4);
    head[0] = (fin ? 0x80 : 0) | opcode;
    head[1] = 126;
    head.writeUInt16BE(body.length, 2);
  } else {
    head = Buffer.alloc(10);
    head[0] = (fin ? 0x80 : 0) | opcode;
    head[1] = 127;
    head.writeBigUInt64BE(BigInt(body.length), 2);
  }
  return Buffer.concat([head, body]);
}

// readClientFrames decodes the masked frames a client sends.
function readClientFrames(buf) {
  const out = [];
  while (buf.length >= 2) {
    let len = buf[1] & 0x7f;
    let off = 2;
    if (len === 126) { len = buf.readUInt16BE(2); off = 4; }
    else if (len === 127) { len = Number(buf.readBigUInt64BE(2)); off = 10; }
    assert.ok(buf[1] & 0x80, 'client frames must be masked');
    if (buf.length < off + 4 + len) break;
    const mask = buf.subarray(off, off + 4);
    const data = Buffer.from(buf.subarray(off + 4, off + 4 + len));
    for (let i = 0; i < data.length; i++) data[i] ^= mask[i & 3];
    out.push({ opcode: buf[0] & 0x0f, data });
    buf = buf.subarray(off + 4 + len);
  }
  return { frames: out, rest: buf };
}

function server(onClient) {
  return new Promise((resolve) => {
    const srv = net.createServer((sock) => {
      let buf = Buffer.alloc(0);
      let upgraded = false;
      const received = [];
      sock.on('data', (chunk) => {
        buf = Buffer.concat([buf, chunk]);
        if (!upgraded) {
          const end = buf.indexOf('\r\n\r\n');
          if (end < 0) return;
          const head = buf.subarray(0, end).toString();
          buf = buf.subarray(end + 4);
          assert.doesNotMatch(head, /^origin:/im, 'the host refuses connections with an Origin header');
          const key = /sec-websocket-key: (.+)/i.exec(head)[1].trim();
          const accept = createHash('sha1').update(key + GUID).digest('base64');
          sock.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
          upgraded = true;
          onClient(sock, received);
        }
        const { frames, rest } = readClientFrames(buf);
        buf = rest;
        for (const f of frames) received.push(f);
        sock.emit('frames', received);
      });
      sock.on('error', () => {});
    });
    srv.listen(0, '127.0.0.1', () => resolve(srv));
  });
}

test('WebSocket client talks the frames the Deck host uses', async () => {
  const big = 'x'.repeat(70000);
  const srv = await server((sock) => {
    sock.write(frame(0x1, '{"event":"small"}'));
    // A message split over fragments and TCP writes, with a ping in between.
    const f1 = frame(0x1, '{"event":"frag', false);
    sock.write(f1.subarray(0, 3));
    setTimeout(() => {
      sock.write(f1.subarray(3));
      sock.write(frame(0x9, 'hi'));
      sock.write(frame(0x0, 'mented"}'));
      sock.write(frame(0x1, JSON.stringify({ event: 'big', data: big })));
      sock.write(frame(0x1, 'ünïcödé'));
    }, 20);
  });
  const ws = new WebSocketClient('127.0.0.1', srv.address().port);
  const messages = [];
  const done = new Promise((resolve) => ws.on('message', (m) => {
    messages.push(m);
    if (messages.length === 4) resolve();
  }));
  await new Promise((r) => ws.on('open', r));
  ws.send('hello');
  ws.send('y'.repeat(300));
  await done;
  assert.deepEqual(messages.slice(0, 2), ['{"event":"small"}', '{"event":"fragmented"}']);
  assert.equal(JSON.parse(messages[2]).data.length, 70000);
  assert.equal(messages[3], 'ünïcödé');
  const closed = new Promise((r) => ws.on('close', r));
  ws.close();
  await closed;
  await new Promise((r) => srv.close(r));
});

test('WebSocket client answers pings and sends masked text', async () => {
  let received;
  const srv = await server((sock, got) => {
    received = got;
    sock.write(frame(0x9, 'ping!'));
  });
  const ws = new WebSocketClient('127.0.0.1', srv.address().port);
  await new Promise((r) => ws.on('open', r));
  ws.send('y'.repeat(300));
  const end = Date.now() + 2000;
  while ((!received || received.length < 2) && Date.now() < end) await new Promise((r) => setTimeout(r, 10));
  const pong = received.find((f) => f.opcode === 0xa);
  const text = received.find((f) => f.opcode === 0x1);
  assert.equal(pong.data.toString(), 'ping!');
  assert.equal(text.data.toString(), 'y'.repeat(300));
  ws.close();
  await new Promise((r) => srv.close(r));
});

test('WebSocket client refuses a bad handshake', async () => {
  const srv = net.createServer((sock) => {
    sock.once('data', () => sock.write('HTTP/1.1 403 Forbidden\r\n\r\n'));
  });
  await new Promise((r) => srv.listen(0, '127.0.0.1', r));
  const ws = new WebSocketClient('127.0.0.1', srv.address().port);
  const err = await new Promise((r) => ws.on('error', r));
  assert.match(err.message, /403/);
  await new Promise((r) => ws.on('close', r));
  await new Promise((r) => srv.close(r));
});
