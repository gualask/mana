//! The settings screen: the channel list's URL and state on the left, with
//! «Update list» to download it again and «Language» to choose the
//! language; on the right the QR code of the page where the phone sends a
//! new URL. The page is served while the screen is open.

use mana_core::catalog::update::{self, Outcome, Ticket, Update, Updater};
use mana_core::language::{Language, Localized};
use mana_core::text;
use url::Url;

use super::App;
use super::list_updates::CHANNELS_FILE;
use super::qr::Qr;
use crate::phone::{Request, Server, StartError};
use crate::platform::keys::Action;

mod draw;

pub struct Setup {
    /// The page's server, or why there is none.
    server: Result<Server, StartError>,
    qr: Option<Qr>,
    /// The download running, of the URL the phone sent or of the list in
    /// use («Update list»).
    waiting: Option<(Ticket, Option<Request>)>,
    /// What became of the last URL sent or the last refresh asked.
    notice: Option<Notice>,
    /// The row with the focus.
    focus: Row,
}

/// The screen's rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Refresh,
    Language,
}

impl Row {
    /// The rows from the top down.
    const ORDER: [Self; 2] = [Self::Refresh, Self::Language];

    fn index(self) -> usize {
        Self::ORDER.iter().position(|&row| row == self).unwrap_or(0)
    }

    /// The row above; the first stays.
    fn up(self) -> Self {
        Self::ORDER[self.index().saturating_sub(1)]
    }

    /// The row below; the last stays.
    fn down(self) -> Self {
        Self::ORDER[(self.index() + 1).min(Self::ORDER.len() - 1)]
    }

    /// Where the row starts, in 1080p units.
    fn top(self) -> f32 {
        600.0 + 96.0 * self.index() as f32
    }
}

/// What the screen says of the last download asked, kept as it happened:
/// it reads in the language of each frame.
enum Notice {
    Downloading,
    /// Saved with this many channels.
    Saved(usize),
    /// Taken with this many channels, but not kept across power-offs.
    Loaded(usize),
    Failed(update::Failure),
    /// A refresh asked without a URL.
    NoAddress,
    /// A refresh asked while the tests' list is in use.
    TestList,
    /// A URL from the phone refused while the tests' list is in use.
    TestListRefused,
}

impl Notice {
    /// Whether the list was taken.
    fn taken(&self) -> bool {
        matches!(self, Self::Downloading | Self::Saved(_) | Self::Loaded(_))
    }
}

impl Localized for Notice {
    fn text(&self, language: Language) -> String {
        match self {
            Self::Downloading => text!(language, "notice-downloading"),
            Self::Saved(count) => text!(language, "notice-saved", count = count),
            Self::Loaded(count) => text!(language, "notice-loaded", count = count),
            Self::Failed(failure) => {
                let reason = failure.text(language);
                text!(language, "notice-not-saved", reason = reason)
            }
            Self::NoAddress => text!(language, "notice-no-address"),
            Self::TestList => text!(language, "notice-test-list", file = CHANNELS_FILE),
            Self::TestListRefused => text!(language, "notice-test-list-refused", file = CHANNELS_FILE),
        }
    }
}

impl Setup {
    fn open(language: Language) -> Self {
        let server = Server::start(language);
        let qr = server.as_ref().ok().and_then(|s| Qr::new(s.address()));
        Self { server, qr, waiting: None, notice: None, focus: Row::Refresh }
    }

    /// The URL the phone sent, if any, while none is downloading.
    fn request(&mut self) -> Option<Request> {
        if self.waiting.is_some() {
            return None;
        }
        self.server.as_ref().ok()?.request()
    }

    fn wait(&mut self, ticket: Ticket, request: Option<Request>) {
        self.notice = Some(Notice::Downloading);
        self.waiting = Some((ticket, request));
    }

    /// Says what came of the download waited for, to the phone too, in
    /// `language`, when it sent the URL.
    fn outcome(&mut self, update: &Update, language: Language) {
        if self.waiting.as_ref().is_none_or(|(ticket, _)| *ticket != update.ticket) {
            return;
        }
        let Some((_, request)) = self.waiting.take() else { return };
        let notice = match &update.outcome {
            Outcome::Channels { channels, kept: true } => Notice::Saved(channels.len()),
            Outcome::Channels { channels, kept: false } => Notice::Loaded(channels.len()),
            Outcome::Failed(failure) => Notice::Failed(failure.clone()),
        };
        if let Some(request) = request {
            request.answer(notice.taken(), notice.text(language));
        }
        self.notice = Some(notice);
    }

    /// Gives the page the URL in use, for its form, and the language.
    fn sync_page(&self, url: Option<&Url>, language: Language) {
        if let Ok(server) = &self.server {
            server.set_current(url);
            server.set_language(language);
        }
    }
}

impl App {
    pub(super) fn open_setup(&mut self) {
        self.setup = Some(Setup::open(self.languages.current()));
    }

    /// Whether the settings screen is open, its page waiting for the phone.
    pub fn setup_open(&self) -> bool {
        self.setup.is_some()
    }

    /// Up and down move between the rows; OK refreshes the list, or, as
    /// left and right, changes the language.
    pub(super) fn handle_setup(&mut self, action: Action) {
        let Some(focus) = self.setup.as_ref().map(|setup| setup.focus) else { return };
        match (action, focus) {
            (Action::Back, _) => self.setup = None,
            (Action::Up, _) => self.point_setup(focus.up()),
            (Action::Down, _) => self.point_setup(focus.down()),
            (Action::Select, Row::Refresh) => self.refresh_list(),
            (Action::Select | Action::Right, Row::Language) => self.languages.choose(self.languages.choice().next()),
            (Action::Left, Row::Language) => self.languages.choose(self.languages.choice().previous()),
            (Action::Left | Action::Right, Row::Refresh) => {}
        }
    }

    /// Gives the focus to `row`, under the pointer.
    pub(super) fn point_setup(&mut self, row: Row) {
        if let Some(setup) = &mut self.setup {
            setup.focus = row;
        }
    }

    /// «Update list»: downloads the list in use again, unless a download
    /// is already waited for.
    fn refresh_list(&mut self) {
        let Some(setup) = &mut self.setup else { return };
        if setup.waiting.is_some() {
            return;
        }
        setup.notice = Some(match self.updater.as_mut().map(Updater::refresh) {
            Some(Some(ticket)) => return setup.wait(ticket, None),
            Some(None) => Notice::NoAddress,
            None => Notice::TestList,
        });
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
            None => {
                let notice = Notice::TestListRefused;
                request.answer(false, notice.text(self.languages.current()));
                setup.notice = Some(notice);
            }
        }
    }

    /// Answers the phone, when `update` is the download of the URL it sent.
    pub(super) fn answer_phone(&mut self, update: &Update) {
        if let Some(setup) = &mut self.setup {
            setup.outcome(update, self.languages.current());
        }
    }

    /// Keeps the phone's page in step with the TV: the URL in use, for its
    /// form, and the language.
    pub(super) fn sync_page(&self) {
        if let Some(setup) = &self.setup {
            setup.sync_page(self.updater.as_ref().and_then(Updater::url), self.languages.current());
        }
    }

}

#[cfg(test)]
mod tests;
