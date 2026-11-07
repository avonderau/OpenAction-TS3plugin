# TeamSpeak 3 for OpenDeck

Deck keys that control the TeamSpeak 3 client and show what it is doing.
The plugin is written for the Deck plugin host in
[OpenLinkHub Nexus](https://github.com/avonderau/OpenLinkHub-Nexus)
([docs](https://github.com/avonderau/OpenLinkHub-Nexus/blob/main/docs/deck-plugins.md)).
It is a standard Stream Deck SDK plugin, so it also runs in OpenDeck and in
the Elgato Stream Deck app on Windows and macOS.

| Key | Press | Shows |
| --- | --- | --- |
| Toggle microphone | Mute or unmute your mic | Mic on / Mic off |
| Toggle sound | Mute or unmute speakers/headphones | Sound on / Sound off |
| Toggle away | Set or clear away, with an optional message | Online / Away |
| Push to talk | Transmit while the key is held | PTT / TALK |
| Join channel | Move to a channel (with optional password) | The channel; lit while you are in it |
| Status | Refresh | Your channel and its user count; lit while you talk |

Keys show "TS3 offline" when TeamSpeak is not running, "No server" when
it is not connected, and "Set API key" until you enter the key.

## How it works

The plugin talks to the TeamSpeak 3 client through its built-in
**ClientQuery** interface on `127.0.0.1:25639`. Nothing has to be installed
into TeamSpeak. It follows the server tab you have selected and listens
for TeamSpeak's events, so the keys update when you mute or move from
inside TeamSpeak too.

## Requirements

- TeamSpeak 3 client 3.1 or later, with the ClientQuery addon enabled
  (it is on by default).
- One of:
  - Linux on x86_64 or aarch64, with OpenLinkHub Nexus or OpenDeck
  - Windows 10 or later (x86_64), with the Elgato Stream Deck app (6.4 or later) or OpenDeck
  - macOS 12 or later (Apple Silicon or Intel), with the Elgato Stream Deck app or OpenDeck

The plugin is a single native binary for each platform, so nothing else
needs to be installed.

## Install

1. Download `com.avonderau.ts3deck.streamDeckPlugin` from the
   [latest release](https://github.com/avonderau/openaction-ts3plugin/releases/latest),
   or build it yourself (see below).
2. In OpenLinkHub, open the Deck editor, go to the **Plugins** tab, choose
   the file and click **Install plugin**, then confirm on your desktop. Or,
   as your desktop user:

   ```sh
   /opt/OpenLinkHub/OpenLinkHub --install-deck-plugin dist/com.avonderau.ts3deck.streamDeckPlugin
   ```

   In the **Elgato Stream Deck app**, double-click the downloaded file. In
   **OpenDeck**, use **Plugins → Install from file**.

3. Put a TeamSpeak action on a key (**Plugin action (experimental)**),
   **Save & Apply**, then open **Plugin settings**.
4. In TeamSpeak 3, open **Tools → Options → Addons → ClientQuery →
   Settings**, copy the API key, paste it into the settings page and click
   **Save connection**. The key is shared by every TeamSpeak key.

For **Join channel**, enter the channel name, or its full path such as
`Games/Squad 1` when names repeat. The field suggests channels from the
server you are connected to.

### macOS: if the plugin does not start

The macOS binary is not signed with an Apple Developer ID yet, so macOS may
block it after a download. If the keys stay blank, clear the quarantine
flag on the installed plugin and restart the Stream Deck app:

```sh
xattr -dr com.apple.quarantine ~/Library/Application\ Support/com.elgato.StreamDeck/Plugins/com.avonderau.ts3deck.sdPlugin
```

### Push to talk

Set TeamSpeak's capture mode to **Continuous Transmission**. The key opens
your mic while it is held and closes it on release, and also when the key
leaves the page. Holding needs a Deck that reports key release (Elgato
Stream Deck, Mirabox HSV293SV3). On the original Mirabox HSV293S and the
Ajazz AKP153, a press opens and closes the mic at once.

## Development

The plugin is written in Rust (stable, 1.75 or later).

```sh
cargo test                      # unit tests, a fake ClientQuery server, and the
                                # binary driven through a fake Deck host
sh scripts/build.sh             # builds this OS's binary into the plugin folder
sh scripts/pack.sh              # writes dist/com.avonderau.ts3deck.streamDeckPlugin
python3 scripts/make-icons.py   # redraws the key icons (needs Pillow)
```

`scripts/build.sh` needs these Rust targets (`rustup target add …`):

| OS | Targets | Output |
| --- | --- | --- |
| Linux | `x86_64-unknown-linux-musl aarch64-unknown-linux-musl` | static binaries; aarch64 links with `rust-lld` |
| macOS | `aarch64-apple-darwin x86_64-apple-darwin` | one universal binary |
| Windows (Git Bash) | `x86_64-pc-windows-msvc` | `ts3deck.exe` with a static C runtime |

A local `pack.sh` includes only the binaries you built. CI builds all three
platforms and packs them into one archive (`.github/workflows/build.yml`),
which the release workflow publishes when a `vX.Y.Z` tag is pushed.

| Path | What it is |
| --- | --- |
| `src/clientquery.rs` | ClientQuery connection, state, commands, reconnects |
| `src/plugin.rs` | Key actions: what each key does and shows |
| `src/deck.rs` | WebSocket connection to the Deck host |
| `com.avonderau.ts3deck.sdPlugin/` | Manifest, icons and settings page |

## Notes

- The API key and channel passwords are stored in the Deck's plugin
  settings in plain text, like any other Stream Deck plugin setting.
- Plugin output goes to
  `~/.local/state/OpenLinkHub/deck-plugins/logs/com.avonderau.ts3deck.log`.

## License

Licensed under the [MIT License](LICENSE). Copyright (c) 2026 avonderau.
