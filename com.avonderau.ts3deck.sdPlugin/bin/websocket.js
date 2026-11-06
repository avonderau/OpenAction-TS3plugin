// A small RFC 6455 WebSocket client for the plugin's local connection to the
// Deck host. Node only gained a built-in WebSocket client in version 22, and
// many Linux distributions still ship Node 18 or 20, so the plugin carries
// its own. It supports text messages, fragmentation, ping/pong and close,
// which is all the Stream Deck protocol needs.

import { createHash, randomBytes } from 'node:crypto';
import { EventEmitter } from 'node:events';
import net from 'node:net';

const GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';
const MAX_MESSAGE = 64 << 20;

export const CONNECTING = 0;
export const OPEN = 1;
export const CLOSED = 3;

export class WebSocketClient extends EventEmitter {
  constructor(host, port, path = '/') {
    super();
    this.readyState = CONNECTING;
    this.key = randomBytes(16).toString('base64');
    this.buffer = Buffer.alloc(0);
    this.handshaken = false;
    this.fragments = [];
    this.socket = net.createConnection({ host, port }, () => {
      this.socket.write(
        `GET ${path} HTTP/1.1\r\n` +
          `Host: ${host}:${port}\r\n` +
          'Upgrade: websocket\r\n' +
          'Connection: Upgrade\r\n' +
          `Sec-WebSocket-Key: ${this.key}\r\n` +
          'Sec-WebSocket-Version: 13\r\n\r\n',
      );
    });
    this.socket.on('data', (chunk) => this.onData(chunk));
    this.socket.on('error', (err) => this.emit('error', err));
    this.socket.on('close', () => this.finish());
  }

  finish() {
    if (this.readyState === CLOSED) return;
    this.readyState = CLOSED;
    this.emit('close');
  }

  fail(reason) {
    this.emit('error', new Error(reason));
    this.socket.destroy();
  }

  onData(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    if (!this.handshaken) {
      const end = this.buffer.indexOf('\r\n\r\n');
      if (end < 0) {
        if (this.buffer.length > 16384) this.fail('WebSocket handshake too large');
        return;
      }
      const head = this.buffer.subarray(0, end).toString('latin1').split('\r\n');
      this.buffer = this.buffer.subarray(end + 4);
      const expected = createHash('sha1').update(this.key + GUID).digest('base64');
      const accept = head.find((l) => /^sec-websocket-accept:/i.test(l))?.split(':')[1]?.trim();
      if (!/^HTTP\/1\.1 101/.test(head[0]) || accept !== expected) {
        this.fail(`WebSocket handshake refused: ${head[0]}`);
        return;
      }
      this.handshaken = true;
      this.readyState = OPEN;
      this.emit('open');
    }
    this.readFrames();
  }

  readFrames() {
    while (this.buffer.length >= 2) {
      const b0 = this.buffer[0];
      const b1 = this.buffer[1];
      const fin = (b0 & 0x80) !== 0;
      const opcode = b0 & 0x0f;
      const masked = (b1 & 0x80) !== 0;
      let len = b1 & 0x7f;
      let offset = 2;
      if (len === 126) {
        if (this.buffer.length < 4) return;
        len = this.buffer.readUInt16BE(2);
        offset = 4;
      } else if (len === 127) {
        if (this.buffer.length < 10) return;
        const big = this.buffer.readBigUInt64BE(2);
        if (big > BigInt(MAX_MESSAGE)) {
          this.fail('WebSocket message too large');
          return;
        }
        len = Number(big);
        offset = 10;
      }
      const maskOffset = offset;
      if (masked) offset += 4;
      if (this.buffer.length < offset + len) return;
      let payload = Buffer.from(this.buffer.subarray(offset, offset + len));
      if (masked) {
        const mask = this.buffer.subarray(maskOffset, maskOffset + 4);
        for (let i = 0; i < payload.length; i++) payload[i] ^= mask[i & 3];
      }
      this.buffer = this.buffer.subarray(offset + len);
      this.onFrame(fin, opcode, payload);
      if (this.readyState === CLOSED) return;
    }
  }

  onFrame(fin, opcode, payload) {
    switch (opcode) {
      case 0x0: // continuation
      case 0x1: // text
      case 0x2: // binary
        this.fragments.push(payload);
        if (this.fragments.reduce((n, f) => n + f.length, 0) > MAX_MESSAGE) {
          this.fail('WebSocket message too large');
          return;
        }
        if (fin) {
          const data = Buffer.concat(this.fragments).toString('utf8');
          this.fragments = [];
          this.emit('message', data);
        }
        return;
      case 0x8: // close
        this.writeFrame(0x8, payload.subarray(0, 2));
        this.socket.end();
        this.finish();
        return;
      case 0x9: // ping
        this.writeFrame(0xa, payload);
        return;
      default: // pong and reserved opcodes
    }
  }

  writeFrame(opcode, payload) {
    if (this.socket.destroyed) return;
    const mask = randomBytes(4);
    let header;
    if (payload.length < 126) {
      header = Buffer.from([0x80 | opcode, 0x80 | payload.length]);
    } else if (payload.length < 65536) {
      header = Buffer.alloc(4);
      header[0] = 0x80 | opcode;
      header[1] = 0x80 | 126;
      header.writeUInt16BE(payload.length, 2);
    } else {
      header = Buffer.alloc(10);
      header[0] = 0x80 | opcode;
      header[1] = 0x80 | 127;
      header.writeBigUInt64BE(BigInt(payload.length), 2);
    }
    const body = Buffer.from(payload);
    for (let i = 0; i < body.length; i++) body[i] ^= mask[i & 3];
    this.socket.write(Buffer.concat([header, mask, body]));
  }

  send(text) {
    if (this.readyState !== OPEN) return;
    this.writeFrame(0x1, Buffer.from(String(text), 'utf8'));
  }

  close() {
    if (this.readyState !== OPEN) {
      this.socket.destroy();
      return;
    }
    this.writeFrame(0x8, Buffer.from([0x03, 0xe8]));
    this.socket.end();
  }
}
