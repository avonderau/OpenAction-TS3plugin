// TeamSpeak query escaping. Values in commands and replies use backslash
// escapes; "|" separates entries in a list and spaces separate parameters.

const ESCAPES = [
  ['\\', '\\\\'],
  ['/', '\\/'],
  [' ', '\\s'],
  ['|', '\\p'],
  ['\x07', '\\a'],
  ['\b', '\\b'],
  ['\f', '\\f'],
  ['\n', '\\n'],
  ['\r', '\\r'],
  ['\t', '\\t'],
  ['\v', '\\v'],
];

const UNESCAPE = new Map(ESCAPES.map(([raw, esc]) => [esc[1], raw]));

export function escape(value) {
  let out = String(value);
  for (const [raw, esc] of ESCAPES) out = out.split(raw).join(esc);
  return out;
}

export function unescape(value) {
  let out = '';
  for (let i = 0; i < value.length; i++) {
    const c = value[i];
    if (c === '\\' && i + 1 < value.length) {
      const next = value[++i];
      out += UNESCAPE.has(next) ? UNESCAPE.get(next) : next;
    } else {
      out += c;
    }
  }
  return out;
}

// parseEntry turns "a=1 b=x\sy flag" into {a: '1', b: 'x y', flag: ''}.
export function parseEntry(text) {
  const entry = {};
  for (const part of text.split(' ')) {
    if (!part) continue;
    const eq = part.indexOf('=');
    if (eq < 0) entry[part] = '';
    else entry[part.slice(0, eq)] = unescape(part.slice(eq + 1));
  }
  return entry;
}

// parseList splits a reply on "|" into entries.
export function parseList(text) {
  return text.split('|').map(parseEntry);
}

// buildCommand renders a command with escaped parameters, skipping
// undefined and null values.
export function buildCommand(name, params = {}) {
  const parts = [name];
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null) continue;
    parts.push(`${key}=${escape(value)}`);
  }
  return parts.join(' ');
}
