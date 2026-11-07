//! TeamSpeak query escaping. Values in commands and replies use backslash
//! escapes; `|` separates entries in a list and spaces separate parameters.

use std::collections::HashMap;

/// One reply entry: `a=1 b=x\sy flag` becomes {a: "1", b: "x y", flag: ""}.
pub type Entry = HashMap<String, String>;

const ESCAPES: [(char, &str); 11] = [
    ('\\', "\\\\"),
    ('/', "\\/"),
    (' ', "\\s"),
    ('|', "\\p"),
    ('\x07', "\\a"),
    ('\x08', "\\b"),
    ('\x0c', "\\f"),
    ('\n', "\\n"),
    ('\r', "\\r"),
    ('\t', "\\t"),
    ('\x0b', "\\v"),
];

pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match ESCAPES.iter().find(|(raw, _)| *raw == c) {
            Some((_, esc)) => out.push_str(esc),
            None => out.push(c),
        }
    }
    out
}

pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(next) => {
                let raw = ESCAPES
                    .iter()
                    .find(|(_, esc)| esc.as_bytes()[1] as char == next)
                    .map(|(raw, _)| *raw);
                out.push(raw.unwrap_or(next));
            }
            None => out.push('\\'),
        }
    }
    out
}

pub fn parse_entry(text: &str) -> Entry {
    let mut entry = Entry::new();
    for part in text.split(' ').filter(|p| !p.is_empty()) {
        match part.split_once('=') {
            Some((k, v)) => entry.insert(k.to_string(), unescape(v)),
            None => entry.insert(part.to_string(), String::new()),
        };
    }
    entry
}

/// Splits a reply on `|` into entries.
pub fn parse_list(text: &str) -> Vec<Entry> {
    text.split('|').map(parse_entry).collect()
}

/// Renders a command with escaped parameters; `None` values are skipped.
pub fn build_command(name: &str, params: &[(&str, Option<String>)]) -> String {
    let mut line = name.to_string();
    for (key, value) in params {
        if let Some(v) = value {
            line.push(' ');
            line.push_str(key);
            line.push('=');
            line.push_str(&escape(v));
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_special_characters() {
        let raw = "a b/c|d\\e\nf\tg";
        assert_eq!(escape(raw), "a\\sb\\/c\\pd\\\\e\\nf\\tg");
        assert_eq!(unescape(&escape(raw)), raw);
    }

    #[test]
    fn builds_and_parses() {
        assert_eq!(
            build_command(
                "clientupdate",
                &[
                    ("client_away_message", Some("brb 5 min".into())),
                    ("skip", None)
                ]
            ),
            "clientupdate client_away_message=brb\\s5\\smin"
        );
        let list = parse_list("cid=1 channel_name=A\\sB|cid=2 flag");
        assert_eq!(list[0]["channel_name"], "A B");
        assert_eq!(list[1]["flag"], "");
    }
}
