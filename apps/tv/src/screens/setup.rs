//! The settings screen: the channel list's URL and state on the left, with
//! «Aggiorna lista» to download it again; on the right the QR code of the
//! page where the phone sends a new URL. The page is served while the
//! screen is open.

use mana_core::catalog::update::{Outcome, Ticket, Update, Updater};
use url::Url;

use super::gfx::Renderer;
use super::hits::Target;
use super::list_updates::ListState;
use super::qr::Qr;
use super::text::Text;
use super::{App, BACKGROUND, ERROR, FOCUS, MUTED, OK, ROW, TEXT};
use crate::phone::{Request, Server};
use crate::platform::keys::Action;

/// Longest texts shown, in characters: longer ones are shortened in the
/// middle, so that they stay left of the QR code.
const MAX_URL_CHARS: usize = 56;
const MAX_NOTICE_CHARS: usize = 64;

pub struct Setup {
    /// The page's server, or why there is none.
    server: Result<Server, String>,
    qr: Option<Qr>,
    /// The download running, of the URL the phone sent or of the list in
    /// use («Aggiorna lista»).
    waiting: Option<(Ticket, Option<Request>)>,
    /// What became of the last URL sent, and whether it was taken.
    notice: Option<(String, bool)>,
}

impl Setup {
    fn open() -> Self {
        let server = Server::start();
        let qr = server.as_ref().ok().and_then(|s| Qr::new(s.address()));
        Self { server, qr, waiting: None, notice: None }
    }

    /// The URL the phone sent, if any, while none is downloading.
    fn request(&mut self) -> Option<Request> {
        if self.waiting.is_some() {
            return None;
        }
        self.server.as_ref().ok()?.request()
    }

    fn wait(&mut self, ticket: Ticket, request: Option<Request>) {
        self.notice = Some(("Scarico la lista…".into(), true));
        self.waiting = Some((ticket, request));
    }

    fn refuse(&mut self, request: Request, reason: &str) {
        let text = format!("Non salvata: {reason}.");
        request.answer(false, text.clone());
        self.notice = Some((text, false));
    }

    /// Says what came of the download waited for, to the phone too when it
    /// sent the URL.
    fn outcome(&mut self, update: &Update) {
        if self.waiting.as_ref().is_none_or(|(ticket, _)| *ticket != update.ticket) {
            return;
        }
        let Some((_, request)) = self.waiting.take() else { return };
        let (text, saved) = match &update.outcome {
            Outcome::Channels { channels, kept: true } => (format!("Lista salvata: {} canali", channels.len()), true),
            Outcome::Channels { channels, kept: false } => {
                (format!("Lista caricata: {} canali, ma non resterà dopo lo spegnimento", channels.len()), true)
            }
            Outcome::Failed(reason) => (format!("Non salvata: {reason}. La TV tiene la lista di prima."), false),
        };
        if let Some(request) = request {
            request.answer(saved, text.clone());
        }
        self.notice = Some((text, saved));
    }

    fn set_current(&self, url: Option<&Url>) {
        if let Ok(server) = &self.server {
            server.set_current(url);
        }
    }

    /// The page's QR code and address, or why there is none.
    fn draw_page(&self, r: &mut Renderer, t: &mut Text, s: f32) {
        match (&self.server, &self.qr) {
            (Ok(server), Some(qr)) => {
                let side = qr.draw(r, 1160.0 * s, 200.0 * s, 560.0 * s);
                let below = 200.0 * s + side;
                t.draw(r, 1160.0 * s, below + 30.0 * s, 26.0 * s, MUTED, "Inquadra col telefono oppure apri");
                t.draw(r, 1160.0 * s, below + 76.0 * s, 32.0 * s, TEXT, server.address());
            }
            (Ok(server), None) => {
                t.draw(r, 1160.0 * s, 240.0 * s, 32.0 * s, TEXT, server.address());
            }
            (Err(reason), _) => {
                t.draw(r, 1160.0 * s, 240.0 * s, 32.0 * s, ERROR, reason);
            }
        }
    }
}

impl App {
    pub(super) fn open_setup(&mut self) {
        self.setup = Some(Setup::open());
    }

    /// Whether the settings screen is open, its page waiting for the phone.
    pub fn setup_open(&self) -> bool {
        self.setup.is_some()
    }

    pub(super) fn handle_setup(&mut self, action: Action) {
        match action {
            Action::Back => self.setup = None,
            Action::Select => self.refresh_list(),
            _ => {}
        }
    }

    /// «Aggiorna lista»: downloads the list in use again, unless a download
    /// is already waited for.
    fn refresh_list(&mut self) {
        let Some(setup) = &mut self.setup else { return };
        if setup.waiting.is_some() {
            return;
        }
        let refused = match self.updater.as_mut().map(Updater::refresh) {
            Some(Some(ticket)) => return setup.wait(ticket, None),
            Some(None) => "Nessun indirizzo: impostalo dal telefono",
            None => "La TV usa la lista di prova copiata in /tmp",
        };
        setup.notice = Some((refused.into(), false));
    }

    /// Hands the URL the phone sent to the list's updater.
    pub(super) fn take_phone_request(&mut self) {
        let Some(setup) = &mut self.setup else { return };
        let Some(request) = setup.request() else { return };
        match &mut self.updater {
            Some(updater) => {
                let ticket = updater.change(request.url.clone());
                setup.wait(ticket, Some(request));
            }
            None => setup.refuse(request, "la TV usa la lista di prova copiata in /tmp"),
        }
    }

    /// Answers the phone, when `update` is the download of the URL it sent.
    pub(super) fn answer_phone(&mut self, update: &Update) {
        if let Some(setup) = &mut self.setup {
            setup.outcome(update);
        }
    }

    /// Gives the page the URL in use, for its form.
    pub(super) fn show_url_on_page(&self) {
        if let Some(setup) = &self.setup {
            setup.set_current(self.updater.as_ref().and_then(Updater::url));
        }
    }

    /// The list's state, as the settings screen says it.
    fn list_summary(&self) -> String {
        match self.list_state() {
            ListState::Tests(count) => format!("lista di prova: {count} canali"),
            ListState::Channels(count) => format!("{count} canali"),
            ListState::Downloading => "in arrivo…".into(),
            ListState::Unavailable(reason) => format!("non disponibile: {reason}"),
            ListState::NotSet | ListState::Empty => "nessun canale".into(),
        }
    }

    pub(super) fn draw_setup(&mut self, r: &mut Renderer, t: &mut Text, s: f32, surface: (i32, i32)) {
        r.begin(surface.0, surface.1, BACKGROUND);
        t.draw(r, 120.0 * s, 90.0 * s, 72.0 * s, TEXT, "Impostazioni");
        self.draw_refresh_button(r, t, s);
        let Some(setup) = &self.setup else { return };
        self.draw_list_state(setup, r, t, s);
        setup.draw_page(r, t, s);
        t.draw(r, 120.0 * s, 980.0 * s, 24.0 * s, MUTED, "OK per aggiornare la lista · Indietro per tornare ai canali");
    }

    /// «Aggiorna lista», the screen's only button, so always focused;
    /// dimmed while a download is waited for.
    fn draw_refresh_button(&mut self, r: &mut Renderer, t: &mut Text, s: f32) {
        let busy = self.setup.as_ref().is_some_and(|setup| setup.waiting.is_some());
        let (x, y, w, h) = (120.0 * s, 600.0 * s, 420.0 * s, 72.0 * s);
        r.rect(x, y, w, h, if busy { ROW } else { FOCUS });
        self.hits.add(x, y, w, h, Target::Refresh);
        let label = if busy { "Aggiorno…" } else { "Aggiorna lista" };
        t.draw(r, x + 36.0 * s, y + 16.0 * s, 32.0 * s, if busy { MUTED } else { TEXT }, label);
    }

    /// The list's URL and state, whether a settings file wins over the
    /// phone, and what became of the last URL sent.
    fn draw_list_state(&self, setup: &Setup, r: &mut Renderer, t: &mut Text, s: f32) {
        t.draw(r, 120.0 * s, 240.0 * s, 40.0 * s, TEXT, "Lista dei canali");
        let url = self.updater.as_ref().and_then(Updater::url).map(Url::as_str);
        t.draw(r, 120.0 * s, 310.0 * s, 28.0 * s, if url.is_some() { TEXT } else { MUTED }, &shortened(url.unwrap_or("non impostata"), MAX_URL_CHARS));
        t.draw(r, 120.0 * s, 360.0 * s, 28.0 * s, MUTED, &shortened(&self.list_summary(), MAX_NOTICE_CHARS));
        if let Some(file) = &self.list_file {
            let text = format!("La riga «lista» in {} vince sull'URL del telefono", file.display());
            t.draw(r, 120.0 * s, 430.0 * s, 24.0 * s, ERROR, &shortened(&text, MAX_NOTICE_CHARS));
        }
        if let Some((notice, taken)) = &setup.notice {
            t.draw(r, 120.0 * s, 500.0 * s, 28.0 * s, if *taken { OK } else { ERROR }, &shortened(notice, MAX_NOTICE_CHARS));
        }
    }
}

/// `text` within `max` characters: its start and end around «…».
fn shortened(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let end = max / 4;
    let start = max - end - 1;
    let head: String = text.chars().take(start).collect();
    let tail: String = text.chars().skip(count - end).collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_texts_keep_their_start_and_end() {
        assert_eq!(shortened("breve", 10), "breve");
        let long = "https://liste.example/mana-lista/0123456789abcdef/lista.toml";
        let short = shortened(long, 24);
        assert_eq!(short.chars().count(), 24);
        assert!(short.starts_with("https://liste.e") && short.ends_with("a.toml") && short.contains('…'));
    }
}
