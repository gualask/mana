//! The signals menu: the candidates of a channel or event whose source
//! gives several, which the viewer chooses among with the remote. Mana
//! does not pass to another by itself: when the one chosen fails, the menu
//! opens again with the reason, and asks the source again, as the signals
//! may have changed.

use std::collections::BTreeMap;

use mana_core::language::{Language, Localized};
use mana_core::session::{Candidates, Failure, Program, State, Stats, chosen_at_once};
use mana_core::source::Selection;
use mana_core::source::run::{self, Candidate};
use mana_core::text;

use super::App;
use super::gfx::Renderer;
use super::hits::{Hits, Target};
use super::text::Text;
use super::{BACKGROUND as PANEL, FOCUS, MUTED, TEXT};
use mana_core::clock;

/// Rows shown at once; the list scrolls past them.
const ROWS: usize = 12;

pub struct SignalsMenu {
    /// The channel's name, as the viewing is called.
    pub name: String,
    /// The channel's item, without a candidate chosen.
    pub selection: Selection,
    signals: Signals,
    focus: usize,
    /// The id of the candidate playing, once one was chosen.
    in_use: Option<String>,
    /// Why candidates chosen before failed, by id.
    failed: BTreeMap<String, Failure>,
    /// The source asked again, the signals found still shown meanwhile.
    again: Option<Candidates>,
}

enum Signals {
    Asking(Candidates),
    Found(Vec<Candidate>),
    /// The source gave none, or could not be asked.
    None(run::Failure),
}

impl SignalsMenu {
    /// Asks the source of `selection` for its item's candidates.
    pub fn ask(name: String, selection: Selection) -> Self {
        let signals = Signals::Asking(Candidates::ask(&selection));
        Self { name, selection, signals, focus: 0, in_use: None, failed: BTreeMap::new(), again: None }
    }

    /// Takes the source's first answer, once it came, true when it just
    /// did; or the answer asked again, which replaces the signals shown
    /// unless it failed.
    pub fn refresh(&mut self) -> bool {
        self.take_again();
        let Signals::Asking(asking) = &self.signals else { return false };
        let Some(answer) = asking.poll() else { return false };
        self.signals = match answer {
            Ok(found) => Signals::Found(found),
            Err(failure) => Signals::None(failure),
        };
        true
    }

    /// Asks the source again for the signals, once some were found.
    pub fn ask_again(&mut self) {
        if matches!(self.signals, Signals::Found(_)) {
            self.again = Some(Candidates::ask(&self.selection));
        }
    }

    /// The signals asked again, once they came, the focus kept on the same
    /// one if still offered.
    fn take_again(&mut self) {
        let Some(answer) = self.again.as_ref().and_then(Candidates::poll) else { return };
        self.again = None;
        let Ok(found) = answer else { return };
        let focused = self.candidates().get(self.focus).and_then(Candidate::id).map(str::to_string);
        self.focus = focused.and_then(|id| found.iter().position(|c| c.id() == Some(&id))).unwrap_or(0);
        self.signals = Signals::Found(found);
    }

    pub fn asking(&self) -> bool {
        matches!(self.signals, Signals::Asking(_))
    }

    /// The candidates found, none while asking.
    pub fn candidates(&self) -> &[Candidate] {
        match &self.signals {
            Signals::Found(found) => found,
            _ => &[],
        }
    }

    /// Moves the focus by `delta` rows, stopping at either end.
    pub fn step(&mut self, delta: isize) {
        let last = self.candidates().len().saturating_sub(1) as isize;
        self.focus = (self.focus as isize + delta).clamp(0, last) as usize;
    }

    /// Moves the focus to the row `index`, under the pointer.
    pub fn set_focus(&mut self, index: usize) {
        self.focus = index.min(self.candidates().len().saturating_sub(1));
    }

    /// The channel's item with the candidate at `index` chosen, which the
    /// menu marks as in use from now on.
    pub fn choose(&mut self, index: usize) -> Option<Selection> {
        let candidate = self.candidates().get(index)?.clone();
        self.in_use = candidate.id().map(str::to_string);
        self.focus = index;
        Some(Selection { candidate: Some(candidate), ..self.selection.clone() })
    }

    /// The row with the focus.
    pub fn focused(&self) -> usize {
        self.focus
    }

    /// The candidate in use failed for `failure`: it shows by its row, and
    /// it is no longer in use.
    pub fn failed(&mut self, failure: Failure) {
        if let Some(id) = self.in_use.take() {
            self.failed.insert(id, failure);
        }
    }

    /// Whether a candidate chosen is playing.
    pub fn in_use(&self) -> bool {
        self.in_use.is_some()
    }

    /// Draws the menu as a translucent panel on the left of the screen: the
    /// candidates by their label, the one in use and those that failed
    /// marked, the reason of the focused one that failed below; `s` scales
    /// the 1080p layout. Records the rows in `hits`.
    pub fn draw(&self, r: &mut Renderer, t: &mut Text, s: f32, hits: &mut Hits, language: Language) {
        let (x, y, w, row) = (60.0 * s, 120.0 * s, 720.0 * s, 64.0 * s);
        let candidates = self.candidates();
        let first = self.focus.saturating_sub(ROWS - 1);
        let shown = candidates.len().clamp(1, ROWS);
        r.rect(x, y, w, (shown as f32 * 64.0 + 200.0) * s, PANEL.with_alpha(0.85));
        t.draw(r, x + 30.0 * s, y + 24.0 * s, 32.0 * s, MUTED, &format!("{} · {}", text!(language, "signals-title"), self.name));
        let now = clock::now();
        if let Some(at) = self.selection.item.starts_at().filter(|&at| at > now) {
            let note = text!(language, "signals-not-started", time = clock::time(at, now));
            t.draw(r, x + 30.0 * s, y + 62.0 * s, 22.0 * s, MUTED, &note);
        }
        let note = match &self.signals {
            Signals::Asking(_) => Some(text!(language, "signals-asking")),
            Signals::None(failure) => Some(failure.text(language)),
            Signals::Found(_) => None,
        };
        if let Some(note) = note {
            t.draw(r, x + 36.0 * s, y + 102.0 * s, 28.0 * s, TEXT, &note);
            return;
        }
        for (i, candidate) in candidates.iter().enumerate().skip(first).take(ROWS) {
            let top = y + 90.0 * s + (i - first) as f32 * row;
            let (bx, bw, bh) = (x + 16.0 * s, w - 32.0 * s, row - 6.0 * s);
            hits.add(bx, top, bw, bh, Target::Signal(i));
            if i == self.focus {
                r.rect(bx, top, bw, bh, FOCUS);
            }
            let colour = if i == self.focus { TEXT } else { MUTED };
            let width = t.draw(r, x + 36.0 * s, top + 12.0 * s, 32.0 * s, TEXT, &candidate.label(i));
            let id = candidate.id();
            let mark = if id.is_some() && id == self.in_use.as_deref() {
                Some(text!(language, "signals-in-use"))
            } else {
                id.filter(|id| self.failed.contains_key(*id)).map(|_| text!(language, "signals-failed"))
            };
            if let Some(mark) = mark {
                t.draw(r, x + 56.0 * s + width, top + 16.0 * s, 26.0 * s, colour, &mark);
            }
        }
        let focused = candidates.get(self.focus).and_then(Candidate::id).and_then(|id| self.failed.get(id));
        if let Some(failure) = focused {
            let top = y + 110.0 * s + shown as f32 * row;
            t.draw(r, x + 36.0 * s, top, 24.0 * s, MUTED, &failure.text(language));
        }
    }
}


impl App {
    /// Plays the signal at `index` of the menu, which stays for Left.
    pub(super) fn play_signal(&mut self, index: usize) {
        let Some(mut menu) = self.signals.take() else { return };
        let chosen = menu.choose(index);
        let name = menu.name.clone();
        if let Some(selection) = chosen {
            self.start(name, Program::Source(selection));
        }
        self.signals = Some(menu);
        self.signals_open = false;
    }

    /// Takes the signals once they come, playing the only one at once, and
    /// opens the menu again when the signal in use failed.
    pub(super) fn refresh_signals(&mut self) {
        let Some(menu) = &mut self.signals else { return };
        if menu.refresh() && chosen_at_once(menu.candidates()).is_some() {
            self.play_signal(0);
            return;
        }
        if menu.in_use()
            && let Some(Stats { state: State::Failed(failure), .. }) = self.session.stats()
        {
            menu.failed(failure);
            menu.ask_again();
            self.signals_open = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mana_core::source::{Item, Source};

    use super::*;

    fn candidate(fields: &[(&str, &str)]) -> Candidate {
        Candidate { fields: fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    /// A menu with the signals found already, as the source answered.
    fn menu() -> SignalsMenu {
        let text = "id = \"e\"\nname = \"E\"\n[[resolve.steps]]\nid = \"s\"\nop = \"stream\"\nurl = \"https://a.invalid/x.m3u8\"\n";
        let source = Arc::new(Source::from_toml(text).unwrap());
        let selection = Selection { source, item: Item { fields: BTreeMap::new() }, device: String::new(), candidate: None };
        let found = vec![candidate(&[("id", "a"), ("label", "Main")]), candidate(&[("id", "b")]), candidate(&[("label", "")])];
        SignalsMenu {
            name: "Event".into(),
            selection,
            signals: Signals::Found(found),
            focus: 0,
            in_use: None,
            failed: BTreeMap::new(),
            again: None,
        }
    }

    #[test]
    fn a_signal_reads_as_its_label_else_its_id_else_its_number() {
        let menu = menu();
        let labels: Vec<String> = menu.candidates().iter().enumerate().map(|(i, c)| c.label(i)).collect();
        assert_eq!(labels, ["Main", "b", "3"]);
    }

    #[test]
    fn the_focus_stops_at_either_end() {
        let mut menu = menu();
        menu.step(-1);
        assert_eq!(menu.focused(), 0);
        menu.step(5);
        assert_eq!(menu.focused(), 2);
        menu.set_focus(9);
        assert_eq!(menu.focused(), 2);
    }

    #[test]
    fn a_signal_chosen_is_in_use_until_it_fails() {
        let mut menu = menu();
        let chosen = menu.choose(1).unwrap();
        assert_eq!(chosen.candidate.unwrap().fields["id"], "b");
        assert!(menu.in_use() && menu.focused() == 1);
        menu.failed(Failure::Thread("gone".into()));
        assert!(!menu.in_use());
        assert!(menu.failed.contains_key("b"));
        assert!(menu.choose(7).is_none());
    }
}
