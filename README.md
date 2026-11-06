# TeamSpeak 3 for Deck

Deck keys that control the TeamSpeak 3 client and show what it is doing.
The plugin is written for the Deck plugin host in
[OpenLinkHub Nexus](https://github.com/avonderau/OpenLinkHub-Nexus)
([docs](https://github.com/avonderau/OpenLinkHub-Nexus/blob/main/docs/deck-plugins.md)).
It is a standard Stream Deck SDK plugin, so it should also run in OpenDeck
and the Elgato Stream Deck app.

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
- Node.js 22 or later on your `PATH`. The plugin uses Node's built-in
  WebSocket client and has no other dependencies.

## Install

1. Build `dist/com.avonderau.ts3deck.streamDeckPlugin` with `npm run pack`.
2. In OpenLinkHub, open the Deck editor, go to the **Plugins** tab, choose
   the file and click **Install plugin**, then confirm on your desktop. Or,
   as your desktop user:

   ```sh
   /opt/OpenLinkHub/OpenLinkHub --install-deck-plugin dist/com.avonderau.ts3deck.streamDeckPlugin
   ```

3. Put a TeamSpeak action on a key (**Plugin action (experimental)**),
   **Save & Apply**, then open **Plugin settings**.
4. In TeamSpeak 3, open **Tools → Options → Addons → ClientQuery →
   Settings**, copy the API key, paste it into the settings page and click
   **Save connection**. The key is shared by every TeamSpeak key.

For **Join channel**, enter the channel name, or its full path such as
`Games/Squad 1` when names repeat. The field suggests channels from the
server you are connected to.

### Push to talk

Set TeamSpeak's capture mode to **Continuous Transmission**. The key opens
your mic while it is held and closes it on release, and also when the key
leaves the page. Holding needs a Deck that reports key release (Elgato
Stream Deck, Mirabox HSV293SV3). On the original Mirabox HSV293S and the
Ajazz AKP153, a press opens and closes the mic at once.

## Development

```sh
npm test         # unit tests against a fake ClientQuery server
npm run pack     # writes dist/com.avonderau.ts3deck.streamDeckPlugin
npm run icons    # redraws the key icons (needs Python 3 with Pillow)
```

`node test/run-mock.js` starts the fake ClientQuery server on a random port
(API key `KEY-123`), so you can try the keys without TeamSpeak.

## Notes

- The API key and channel passwords are stored in the Deck's plugin
  settings in plain text, like any other Stream Deck plugin setting.
- Plugin output goes to
  `~/.local/state/OpenLinkHub/deck-plugins/logs/com.avonderau.ts3deck.log`.
