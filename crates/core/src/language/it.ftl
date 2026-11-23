# The texts the apps show, in Italian: the same messages as en.ftl, with
# the same variables.

## Settings: the language

language-automatic = Automatica
language-italian = Italiano
language-english = English

## Channel list: its address and its download

list-url-empty = indirizzo vuoto
list-url-too-long = indirizzo di { $length } byte, oltre i { $max }
list-url-control = caratteri di controllo nell'indirizzo
list-url-invalid = indirizzo non valido: { $detail }
list-url-scheme = schema { $scheme } non accettato: serve http o https
list-not-utf8 = non è testo UTF-8
list-update-not-started = aggiornamento della lista non avviato
list-not-toml-at-line = non è TOML valido: riga { $line }: { $message }
list-not-toml = non è TOML valido: { $message }
list-italian-keys = la lista usa le vecchie chiavi italiane: il formato ora ha chiavi inglesi, `{ $channels }` al posto di `{ $italian }`

## Channel list: a channel that cannot be played

channel-unnamed = Canale senza nome
channel-no-stream = la lista non dà il suo indirizzo
channel-no-name = canale senza `{ $key }`
channel-not-text = `{ $key }` non è un testo
channel-bad-url = `{ $key }` non valido: { $detail }
channel-url-and-recipe = canale con sia `{ $url }` sia `{ $recipe }`
channel-no-url-or-recipe = canale senza `{ $url }` né `{ $recipe }`
channel-unknown-recipe = ricetta { $name } inesistente
channel-recipe = ricetta { $name }: { $problem }
channel-header-not-text = header { $name } non è un testo
channel-header-not-table = `{ $key }` non è una tabella
channel-recipe-not-table = non è una tabella
channel-recipe-steps-not-list = `{ $key }` non è un elenco
channel-recipe-no-steps = senza `{ $key }`
channel-recipe-step-not-text = un passo non è un testo

## Network

net-status = HTTP { $code } per { $path }
net-oversized = { $path }: risposta oltre { $limit } byte
net-empty = { $path }: risposta vuota

## Recipes and stream files

recipe-invalid-step = passo non valido: { $step }
recipe-missing-parameter = parametro { $name } mancante
recipe-too-many-steps = ricetta di { $count } passi, al massimo { $max }
recipe-step = passo { $number } ({ $step }): { $problem }
recipe-not-url = risultato non è un URL: { $detail }
recipe-no-url-at-end = la ricetta non termina con un URL
recipe-no-url = nessun URL da scaricare
recipe-neither-json-nor-text = risposta né JSON né testo
recipe-missing-field = campo { $path } assente
recipe-field-not-text = campo { $path } non è un testo
recipe-unclosed = `{ "{" }json:` senza `{ "}" }`
recipe-not-json = il valore corrente non è JSON
recipe-unexpected-value = il valore corrente non è del tipo atteso
stream-header = { $file }: header senza `Nome: valore`
stream-unknown-key = chiave sconosciuta in { $file }: { $key }
stream-url-or-recipe = { $file }: serve `url` oppure una ricetta `resolve`, non entrambi

## Live streams

live-unsupported = non supportato: { $cause }
live-decryption-length = decifratura AES-128 fallita: lunghezza non multipla di 16 byte
live-decryption-padding = decifratura AES-128 fallita: padding PKCS#7 non valido
live-key-length = chiave AES-128 non di 16 byte
live-stall = playlist ferma da { $seconds } s (ultimo segmento { $last }, atteso { $expected })
live-refused-again = fonte rifiutata anche dopo { $count } nuove risoluzioni: { $cause }
playlist-invalid = playlist non valida
playlist-not-utf8 = playlist non UTF-8
playlist-map-without-uri = EXT-X-MAP senza URI
playlist-key-without-uri = chiave senza URI
playlist-sequence = sequenza non valida: { $value }
playlist-duration = durata non valida: { $value }
playlist-iv = IV non valido: { $value }
playlist-byte-range = segmenti con BYTERANGE
playlist-map-byte-range = sezione di inizializzazione con BYTERANGE
playlist-dash = manifest DASH
playlist-encryption = cifratura { $method }
playlist-other-format = formato diverso da HLS e MPEG-TS
packed-audio-no-timestamp = audio packed senza timestamp ID3
packed-audio-not-aac = audio packed non AAC

## Playback

playback-unavailable = riproduzione disponibile solo sulla TV
playback-stopped = fermato prima di aprire il decoder
playback-video-codec = codec video non supportato: { $video }
playback-no-video = il flusso non ha video
playback-demuxer-gone = demuxer interrotto
playback-decoder-refuses = il decoder rifiuta i dati da { $ms } ms
playback-decoder-frozen = il decoder non mostra nuovi fotogrammi da { $seconds } s
playback-decoder-wont-resume = il decoder rifiuta di riprendere
backend-open = il decoder non si apre: { $detail }
backend-no-window = la finestra del video non è stata creata
backend-load-refused = il decoder rifiuta i formati del flusso
backend-load-timeout = il decoder non ha finito di caricare
backend-play-refused = il decoder rifiuta di riprodurre
backend-reported = il decoder segnala: { $detail }
demux-open = impossibile aprire il flusso: { $detail }
demux-stream-info = impossibile leggere i formati del flusso: { $detail }
demux-annexb = impossibile convertire il video dell'MP4 per il decoder: { $detail }
demux-aac-config = AAC senza configurazione nell'MP4
demux-read = impossibile leggere il flusso: { $detail }
demux-rewind = impossibile tornare all'inizio del file: { $detail }

## Home screen

home-subtitle = Prova di fattibilità · canali della lista, poi clip e dirette di prova
home-position = { $focus } di { $count }
home-empty = Nessun canale, né clip in { $clips } o { $data }
home-footer = OK per riprodurre, poi OK per i canali, su o giù per il pannello, destra per la qualità · destra qui per Impostazioni · Indietro per uscire
home-list-downloading = Lista dei canali in arrivo…
home-list-unavailable = Lista dei canali non disponibile: { $reason }
home-list-not-set = Nessuna lista: impostala da Impostazioni →

## While watching

channels-header = Canali · { $focus } di { $count }
state-starting = Avvio
state-playing = In riproduzione
state-ending = Fine
state-stopped = Fermato
state-failed = Errore: { $failure }
info-no-audio = nessuno
info-quality = QUALITÀ
quality-title = Qualità
quality-in-use = in uso

## Settings

settings = Impostazioni
settings-footer = Su o giù per scegliere · OK per aggiornare la lista o cambiare lingua · Indietro per tornare ai canali
settings-channel-list = Lista dei canali
settings-list-not-set = non impostata
settings-list-from-file = La riga «{ $key }» in { $file } vince sull'URL del telefono
settings-update = Aggiorna lista
settings-updating = Aggiorno…
settings-language = Lingua
settings-scan = Inquadra col telefono oppure apri
settings-test-list = lista di prova: { $count ->
    [one] { $count } canale
   *[other] { $count } canali
    }
settings-channels = { $count ->
    [one] { $count } canale
   *[other] { $count } canali
    }
settings-list-downloading = in arrivo…
settings-list-unavailable = non disponibile: { $reason }
settings-no-channels = nessun canale
notice-downloading = Scarico la lista…
notice-saved = Lista salvata: { $count ->
    [one] { $count } canale
   *[other] { $count } canali
    }
notice-loaded = Lista caricata: { $count ->
    [one] { $count } canale
   *[other] { $count } canali
    }, ma non resterà dopo lo spegnimento
notice-not-saved = Non salvata: { $reason }. La TV tiene la lista di prima.
notice-no-address = Nessun indirizzo: impostalo dal telefono
notice-test-list = La TV usa la lista di prova { $file }
notice-test-list-refused = Non salvata: la TV usa la lista di prova { $file }.

## Phone page

phone-no-network = Nessuna rete: collega la TV
phone-no-free-port = nessuna porta libera tra { $first } e { $last }
phone-title = Mana · lista dei canali
phone-heading = Lista dei canali
phone-in-use = In uso sulla TV
phone-change = Cambia indirizzo
phone-address = Indirizzo della lista (TOML)
phone-save = Salva
phone-done = Fatto: puoi chiudere questa pagina.
phone-settings-closed = Impostazioni chiuse sulla TV.
phone-still-downloading = La TV sta ancora scaricando la lista: guarda la TV.
phone-closed-meanwhile = Impostazioni chiuse sulla TV: guarda la TV per l'esito.
