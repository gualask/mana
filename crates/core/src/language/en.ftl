# The texts the apps show, in English: the screens, the phone's page and
# the messages of the core. Every message is also in it.ftl, with the same
# variables; `cargo test` checks both against the sources (language/tests.rs).
# Variables are named as in the `text!` calls that fill them.

## Settings: the language

language-automatic = Automatic
language-italian = Italiano
language-english = English

## Channel list: its address and its download

list-url-empty = empty address
list-url-too-long = address of { $length } bytes, over { $max }
list-url-control = control characters in the address
list-url-invalid = invalid address: { $detail }
list-url-scheme = scheme { $scheme } not accepted: http or https is needed
list-not-utf8 = not UTF-8 text
list-update-not-started = list update not started
list-not-toml-at-line = not valid TOML: line { $line }: { $message }
list-not-toml = not valid TOML: { $message }
list-italian-keys = the list uses the old Italian keys: the format now has English keys, `{ $channels }` instead of `{ $italian }`

## Channel list: a channel that cannot be played

channel-unnamed = Unnamed channel
channel-no-stream = the list gives no stream for it
channel-no-name = channel without `{ $key }`
channel-not-text = `{ $key }` is not a text
channel-bad-url = invalid `{ $key }`: { $detail }
channel-url-and-source = channel with both `{ $url }` and `{ $source }`
channel-no-url-or-source = channel without `{ $url }` or `{ $source }`
channel-unknown-source = source { $name } does not exist
channel-source = source { $name }: { $problem }
channel-header-with-source = `{ $key }` on a channel with a source: the source gives the headers
channel-header-not-text = header { $name } is not a text
channel-header-not-table = `{ $key }` is not a table

## Network

net-status = HTTP { $code } for { $path }
net-oversized = { $path }: answer over { $limit } bytes
net-empty = { $path }: empty answer

## Stream files

stream-header = { $file }: header without `Name: value`
stream-unknown-key = unknown key in { $file }: { $key }
stream-no-url = { $file }: needs `url`

## Source files

source-format-at-line = not valid TOML: line { $line }: { $message }
source-format = not valid TOML: { $message }
source-unknown-field = { $at }: unknown field `{ $field }`
source-missing-field = { $at }: `{ $field }` missing
source-invalid-value = { $at }: `{ $field }` not valid: { $detail }
source-unsupported-operation = { $at }: operation `{ $op }` unknown to this version of Mana
source-one-of = { $at }: exactly one of { $choices } is needed
source-unknown-reference = { $at }: `{ $reference }` names nothing that comes before it
source-type-mismatch = { $at }: { $expected } needed, `{ $reference }` gives { $found }
source-unknown-headers = { $at }: no header set `{ $name }`
source-unknown-resolver = { $at }: no resolver `{ $name }`
source-unknown-recipe = { $at }: no recipe `{ $name }`
source-call-inputs = { $at }: the inputs are not those recipe `{ $recipe }` names
source-duplicate-id = { $at }: a step `{ $id }` comes earlier
source-equal-routes = { $at }: an earlier route has the same `when`
source-call-cycle = recipe `{ $recipe }` calls itself
source-bad-pattern = { $at }: invalid pattern: { $detail }
source-bad-selector = { $at }: invalid selector: { $detail }
source-wrong-output-list = { $at }: the last step does not give a list
source-wrong-output-resolution = { $at }: the last step is not `stream`, `next` or `call`

source-network-error = { $at }: network error
source-access-denied = { $at }: access denied
source-rate-limited = { $at }: too many requests, the server asks to wait
source-changed = { $at }: the answers no longer fit the source
source-unsupported-host = { $at }: no route for this address
source-no-sources = { $at }: no streams for now
source-limit = { $at }: limit reached
source-cancelled = { $at }: cancelled

## Live streams

live-unsupported = unsupported: { $cause }
live-decryption-length = AES-128 decryption failed: length not a multiple of 16 bytes
live-decryption-padding = AES-128 decryption failed: invalid PKCS#7 padding
live-key-length = AES-128 key not 16 bytes long
live-stall = playlist still for { $seconds } s (last segment { $last }, expected { $expected })
live-refused-again = source refused even after { $count } new resolutions: { $cause }
playlist-invalid = invalid playlist
playlist-not-utf8 = playlist not in UTF-8
playlist-map-without-uri = EXT-X-MAP without URI
playlist-key-without-uri = key without URI
playlist-sequence = invalid sequence: { $value }
playlist-duration = invalid duration: { $value }
playlist-iv = invalid IV: { $value }
playlist-byte-range = segments with BYTERANGE
playlist-map-byte-range = initialization section with BYTERANGE
playlist-dash = DASH manifest
playlist-encryption = encryption { $method }
playlist-other-format = format other than HLS and MPEG-TS
packed-audio-no-timestamp = packed audio without ID3 timestamp
packed-audio-not-aac = packed audio not AAC

## Playback

playback-unavailable = playback available only on the TV
playback-stopped = stopped before opening the decoder
playback-video-codec = unsupported video codec: { $video }
playback-no-video = the stream has no video
playback-demuxer-gone = demuxer interrupted
playback-decoder-refuses = the decoder refuses data for { $ms } ms
playback-decoder-frozen = the decoder shows no new frames for { $seconds } s
playback-decoder-wont-resume = the decoder refuses to resume
backend-open = the decoder does not open: { $detail }
backend-no-window = the video window was not created
backend-load-refused = the decoder refuses the stream's formats
backend-load-timeout = the decoder did not finish loading
backend-play-refused = the decoder refuses to play
backend-reported = the decoder reports: { $detail }
demux-open = cannot open the stream: { $detail }
demux-stream-info = cannot read the stream's formats: { $detail }
demux-annexb = cannot convert the MP4's video for the decoder: { $detail }
demux-aac-config = AAC without configuration in the MP4
demux-read = cannot read the stream: { $detail }
demux-rewind = cannot go back to the start of the file: { $detail }

## Signals

signals-title = Signals
signals-asking = Looking for the signals…
signals-in-use = in use
signals-failed = does not play
signals-not-started = Starts { $time } · the signals may not be on air yet

## Home screen

home-subtitle = Feasibility test · the list's channels, then test clips and live streams
home-position = { $focus } of { $count }
home-empty = No channels, and no clips in { $clips } or { $data }
home-footer = OK to play, then OK for the channels, up or down for the panel, right for the quality · right here for Search and Settings · Back to exit
home-list-downloading = Channel list on its way…
home-list-unavailable = Channel list unavailable: { $reason }
home-list-not-set = No list: set it in Settings →
home-folder-footer = OK to play · Back to the list

## Folders

folder-loading = Looking for the events…
folder-unavailable = Events unavailable: { $reason }
folder-saved = Events of the saved copy: { $reason }
folder-empty = No events now

## Search

search = Search
search-title = Search
search-space = space
search-delete = delete
search-hint = Type to search the list's channels and the events of the folders already opened
search-found = { $count ->
    [one] 1 found
   *[other] { $count } found
}
search-footer = OK to type · right from the keyboard to the results, left back · OK on a result to play it · Back to close

## While watching

channels-header = Channels · { $focus } of { $count }
channels-folder-header = { $folder } · { $focus } of { $count }
state-starting = Starting
state-playing = Playing
state-ending = Ending
state-stopped = Stopped
state-failed = Error: { $failure }
info-no-audio = none
info-quality = QUALITY
quality-title = Quality
quality-in-use = in use

## Settings

settings = Settings
settings-footer = Up or down to choose · OK to update the list or change the language · Back to return to the channels
settings-channel-list = Channel list
settings-list-not-set = not set
settings-list-from-file = The «{ $key }» line in { $file } wins over the phone's URL
settings-update = Update list
settings-updating = Updating…
settings-language = Language
settings-scan = Scan it with the phone or open
settings-test-list = test list: { $count ->
    [one] { $count } channel
   *[other] { $count } channels
    }
settings-channels = { $count ->
    [one] { $count } channel
   *[other] { $count } channels
    }
settings-list-downloading = on its way…
settings-list-unavailable = unavailable: { $reason }
settings-no-channels = no channels
notice-downloading = Downloading the list…
notice-saved = List saved: { $count ->
    [one] { $count } channel
   *[other] { $count } channels
    }
notice-loaded = List loaded: { $count ->
    [one] { $count } channel
   *[other] { $count } channels
    }, but it will not stay after power-off
notice-not-saved = Not saved: { $reason }. The TV keeps the previous list.
notice-no-address = No address: set it from the phone
notice-test-list = The TV uses the test list { $file }
notice-test-list-refused = Not saved: the TV uses the test list { $file }.

## Phone page

phone-no-network = No network: connect the TV
phone-no-free-port = no free port between { $first } and { $last }
phone-title = Mana · channel list
phone-heading = Channel list
phone-in-use = In use on the TV
phone-change = Change address
phone-address = Address of the list (TOML)
phone-save = Save
phone-done = Done: you can close this page.
phone-settings-closed = Settings closed on the TV.
phone-still-downloading = The TV is still downloading the list: look at the TV.
phone-closed-meanwhile = Settings closed on the TV: look at the TV for the outcome.

## Desktop

desktop-list-not-set = No list: start Mana with --list and the list's address
desktop-hint = ↑ ↓ row · ← → page · Enter chooses · Esc back
desktop-folder = folder
