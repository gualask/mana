# Channel list

The channel list of the [TV app](tv-app.md): how it is set, its TOML format
and how a channel with a source is played. Mana contains no channels and
no list URL: each user provides their own.

## Setting it from the phone

The list is a TOML file at a URL, downloaded at every start: it is updated
by editing that file, without a new version of the app. The app immediately
shows the copy saved at the last update in `/media/developer/mana/list/`,
and replaces it with the downloaded one only if it is valid TOML; with no
network and no copy, the home screen says so.

"Settings" ("Impostazioni"), at the top right of the home screen (right
arrow), shows a QR code and the address `http://<TV address>:8080/<code>`,
with the next port if that one is taken and a new code at every opening; the
page, served as long as the screen stays open in the app's language, shows
the URL in use and, with "Change address", a field for the new one. The TV
downloads the list immediately and keeps it, saving the URL in
`/media/developer/mana/list/chosen.url`, only if it is valid TOML;
otherwise the phone receives the reason and the previous list stays
([phone/](../apps/tv/src/phone/mod.rs),
[update.rs](../crates/core/src/catalog/update.rs)). A `list` in the
[settings](tv-app.md#files-and-settings-on-the-tv) takes precedence over the
URL chosen from the phone, and "Settings" says so. "Update list", in
"Settings", downloads the list in use again without waiting for the next
start.

## Format

```toml
[sources.example]                  # the key is the source's id
[[sources.example.resolve.steps]]
id = "live"
op = "http"
url = "https://tv.invalid/live/{input.item.id|component}.json"
[[sources.example.resolve.steps]]
id = "stream"
op = "stream"
url = "{live.json.video.content_url}"

[[channels]]
name = "Channel 1"
logo = "https://tv.invalid/logos/channel1.png"  # optional, like `group`
source = "example"
id = "channel1"

[[channels]]
name = "Channel 2"
url = "https://tv.invalid/channel2/playlist.m3u8"
header = { "User-Agent" = "…" }  # optional

[[include]]
m3u = "https://lists.invalid/channels.m3u"

[[include]]
source = "example"                 # its catalogue's items, as channels
```

Channels and included M3U lists appear in file order. A channel has `name`
and exactly one of `url` and `source`. A source is a
[source file](configuration.md#source-files) written under
`[sources.NAME]`: NAME is its id, and its name when it gives none. A
channel with a source is an item of it: `title` is the channel's name, and
its other fields (`id` above) are the item's, read as `{input.item.id}`;
`{input.device}` is a UUID created when a channel with a source is first
read and kept in `/media/developer/mana/device`. The source gives the
headers of its requests and streams, so such a channel has no `header`.

An `[[include]]` with `source` puts there the items of that source's
catalogue, each a channel of the source: events with a start time first,
by time, which the screens show in the TV's local time, then the others in
the catalogue's order. The catalogue is read with the list, and again when
the channel list or the home screen opens if it is more than 5 minutes
old ([list_updates.rs](../apps/tv/src/screens/list_updates.rs)); its copy
is kept, as an included M3U list's, for a start without network.

The keys are in English: a list written with the Italian keys of the first
versions (`canali` at the root) is refused, with an error that names
`channels`; the other old keys are unknown fields like any other. An
unknown field is ignored; a channel with no name, with both or neither of
`url` and `source`, with a source the list does not define or one that
breaks the contract stays in the list as unavailable, with the reason in
the log ([list.rs](../crates/core/src/catalog/list.rs)). An included list
that cannot be downloaded uses its copy, otherwise it is skipped. The
entries of an M3U list can declare headers (`#EXTVLCOPT:http-user-agent=`,
`#EXTVLCOPT:http-referrer=`, `url|User-Agent=…&Referer=…`), sent with every
request of the channel ([m3u.rs](../crates/core/src/catalog/m3u.rs)).

## Playing a channel with a source

A channel whose source has no `sources` flow is its only candidate and
plays at once. Otherwise OK asks the source for the item's candidates and
shows them in a menu, by their `label` or else their `id`, while what was
playing goes on; with a single candidate, it plays at once. The candidate
chosen is resolved and played; Left opens the menu again while it plays,
with the one in use marked, to choose another
([signals_menu.rs](../apps/tv/src/screens/signals_menu.rs)).

Mana does not pass to another candidate by itself: one that does not
resolve, or whose stream cannot be opened or plays no more, ends playback,
and the menu opens again with that candidate marked and the reason. Its
stream gives up after 3 network failures a second apart, as Media3 gives up
a playlist, so that an unreachable host does not hold the screen; a channel
with a URL, or a source's only candidate, is tried until the network is
back. A token refused while watching has the same candidate resolved
again, within the budget of reopenings
([reopen.rs](../crates/core/src/session/reopen.rs)).
