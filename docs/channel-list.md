# Channel list

The channel list of the [TV app](tv-app.md): how it is set, its TOML format
and the recipe steps that resolve a channel. Mana contains no channels and
no list URL: each user provides their own.

## Setting it from the phone

The list is a TOML file at a URL, downloaded at every start: it is updated
by editing that file, without a new version of the app. The app immediately
shows the copy saved at the last update in `/media/developer/mana/lista/`,
and replaces it with the downloaded one only if it is valid TOML; with no
network and no copy, the home screen says so.

"Impostazioni" (Settings), at the top right of the home screen (right
arrow), shows a QR code and the address `http://<IP della TV>:8080/<codice>`,
with the next port if that one is taken and a new code at every opening; the
page, served as long as the screen stays open, shows the URL in use and,
with "Cambia indirizzo" (Change address), a field for the new one. The TV
downloads the list immediately and keeps it, saving the URL in
`/media/developer/mana/lista/scelta.url`, only if it is valid TOML;
otherwise the phone receives the reason and the previous list stays
([phone/](../apps/tv/src/phone/mod.rs),
[update.rs](../crates/core/src/catalog/update.rs)). A `lista` in the
[settings](tv-app.md#files-and-settings-on-the-tv) takes precedence over the
URL chosen from the phone, and "Impostazioni" says so. "Aggiorna lista"
(Update list), in "Impostazioni", downloads the list in use again without
waiting for the next start.

## Format

```toml
[ricette.esempio]
passi = [
  "get https://tv.invalid/dirette/{id}.json",
  "json video.content_url",
  "query formato=hls",
  "get",
  "json video.0",
]

[[canali]]
nome = "Canale 1"
logo = "https://tv.invalid/loghi/canale1.png"  # optional, like `gruppo`
ricetta = "esempio"
id = "canale1"

[[canali]]
nome = "Canale 2"
url = "https://tv.invalid/canale2/playlist.m3u8"
header = { "User-Agent" = "…" }  # optional

[[includi]]
m3u = "https://liste.invalid/canali.m3u"
```

Channels and included M3U lists appear in file order. A channel has `nome`
and exactly one of `url` and `ricetta`; its other fields are the recipe's
parameters, written `{nome}` in the steps, and `{dispositivo}` is a UUID
created when first needed and kept in `/media/developer/mana/dispositivo`.
An unknown field is ignored; a channel with no name, with both or neither of
`url` and `ricetta`, with a nonexistent recipe or an unknown step stays in
the list as unavailable, with the reason in the log
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
- `json campo.0`: extracts a value from the JSON;
- `query nome=valore`: sets a query parameter of the URL;
- `testo MODELLO`: builds a text, with `{json:campo.0}` taken from the JSON.

The same steps, written `resolve = …`, apply in
[`.stream`](tv-app.md#files-and-settings-on-the-tv) files.
