# Channel list

The channel list of the [TV app](tv-app.md): how it is set, its TOML format
and the recipe steps that resolve a channel. Mana contains no channels and
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
[recipes.example]
steps = [
  "get https://tv.invalid/live/{id}.json",
  "json video.content_url",
  "query format=hls",
  "get",
  "json video.0",
]

[[channels]]
name = "Channel 1"
logo = "https://tv.invalid/logos/channel1.png"  # optional, like `group`
recipe = "example"
id = "channel1"

[[channels]]
name = "Channel 2"
url = "https://tv.invalid/channel2/playlist.m3u8"
header = { "User-Agent" = "…" }  # optional

[[include]]
m3u = "https://lists.invalid/channels.m3u"
```

Channels and included M3U lists appear in file order. A channel has `name`
and exactly one of `url` and `recipe`; its other fields are the recipe's
parameters, written `{field}` in the steps (`{id}` above), and `{device}` is a UUID created
when first needed and kept in `/media/developer/mana/device`. The keys
are in English: a list written with the Italian keys of the first versions
(`canali` at the root) is refused, with an error that names `channels`; the
other old keys are unknown fields like any other. An unknown field is
ignored; a channel with no name, with both or neither of `url` and `recipe`,
with a nonexistent recipe or an unknown step stays in the list as
unavailable, with the reason in the log
([list.rs](../crates/core/src/catalog/list.rs)). An included list that
cannot be downloaded uses its copy, otherwise it is skipped. The entries of
an M3U list can declare headers (`#EXTVLCOPT:http-user-agent=`,
`#EXTVLCOPT:http-referrer=`, `url|User-Agent=…&Referer=…`), sent with every
request of the channel ([m3u.rs](../crates/core/src/catalog/m3u.rs)).

## Recipe steps

A recipe is a provisional form, which the
[recipe contract](configuration.md#gap-from-the-current-draft) will
replace. The steps are run on the TV at every opening, each on the result of
the previous one ([recipe.rs](../crates/core/src/live/recipe.rs)):

- `get [URL]`: downloads the URL, or the one produced by the previous step;
- `json field.0`: extracts a value from the JSON;
- `query name=value`: sets a query parameter of the URL;
- `text TEMPLATE`: builds a text, with `{json:field.0}` taken from the JSON.

The same steps, written `resolve = …`, apply in
[`.stream`](tv-app.md#files-and-settings-on-the-tv) files.
