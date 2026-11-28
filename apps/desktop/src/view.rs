//! The window: the video over the whole of it, the list on the left while
//! open, the status at the top while there is something to say.

use mana_core::catalog::logos::{BOX, Placeholder};
use mana_core::session::State as Viewing;
use mana_core::text;
use rune::tok::{self, TextStyle};
use rune::{Color, Cx, El, GrowthSpec, ScrollSpec, SurfaceFit, col, el, oklch, row, text};

use crate::app::{App, Kind, Note, Row};

const PANEL_WIDTH: f32 = 520.0;
const ROW_HEIGHT: f32 = 60.0;
/// A logo's box in a row: the core's, at the height of a row's names.
const LOGO_HEIGHT: f32 = 40.0;
const LOGO_SIZE: (f32, f32) = (BOX.0 * LOGO_HEIGHT / BOX.1, LOGO_HEIGHT);
const NAME: TextStyle = TextStyle::new(17.0, 600.0);
const DETAIL: TextStyle = TextStyle::new(13.0, 400.0);
const HEADING: TextStyle = TextStyle::new(22.0, 700.0);
const INITIAL: TextStyle = TextStyle::new(18.0, 600.0);

pub fn view(app: &App, _: &mut Cx<App>) -> El {
    let video = el().key("video").w_full().h_full().native_surface(&app.surface, SurfaceFit::Fit);
    let mut root = el().w_full().h_full().fill(Color::BLACK).child(video);
    if app.list_open {
        root = root.child(list(app));
    }
    if let Some(status) = status(app) {
        root = root.child(status);
    }
    root
}

fn list(app: &App) -> El {
    let t = tok::theme(true);
    let level = app.top();
    let (focus, count) = ((level.focus + 1).min(level.rows.len()), level.rows.len());
    let title = match &level.kind {
        Kind::List => text!(app.language, "channels-header", focus = focus, count = count),
        Kind::Folder(_) => text!(app.language, "channels-folder-header", folder = level.title.as_str(), focus = focus, count = count),
        Kind::Signals(_) => format!("{} · {focus}/{count}", level.title),
    };
    let heading = row().w_full().px(16.0).pb(12.0).child(text(title).font(HEADING).color(t.text).single_line());
    let rows = col()
        .key(format!("rows-{}", app.levels.len()))
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(level.rows.len(), ROW_HEIGHT, |app: &App, i| app.top().rows[i].key.as_str().into(), list_row)
        .virtual_rows_version(app.version);
    let scroll = col().key(format!("list-{}", app.levels.len())).w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(rows);
    let hint = text(text!(app.language, "desktop-hint")).font(DETAIL).color(t.text_secondary);
    col()
        .key("panel")
        .absolute(0.0, 0.0)
        .w(PANEL_WIDTH)
        .h_full()
        .min_h(0.0)
        .pt(24.0)
        .px(12.0)
        .pb(16.0)
        .gap(8.0)
        .fill(oklch(0.17, 0.02, 265.0).alpha(0.9))
        .child(heading)
        .child(scroll)
        .child(el().px(16.0).child(hint))
}

fn list_row(app: &App, i: usize) -> El {
    let t = tok::theme(true);
    let level = app.top();
    let (item, focused) = (&level.rows[i], i == level.focus);
    let secondary = if focused { t.text } else { t.text_secondary };
    let mut title = row().gap(12.0).items_center();
    if let Some(time) = &item.time {
        title = title.child(text(time.clone()).font(NAME).color(secondary));
    }
    title = title.child(text(item.title.clone()).font(NAME).color(t.text).single_line());
    let names = col().gap(2.0).child(title).child(text(item.detail.clone()).font(DETAIL).color(secondary).single_line());
    let mut line = row()
        .h(ROW_HEIGHT)
        .w_full()
        .px(16.0)
        .gap(14.0)
        .items_center()
        .radius(10.0)
        .fill(if focused { oklch(0.52, 0.19, 262.0) } else { Color::TRANSPARENT });
    if level.with_logos() {
        line = line.child(logo(app, item));
    }
    line.child(names)
}

/// The row's logo fitted in its box; while it has none, or it has not
/// come, the box with its placeholder.
fn logo(app: &App, item: &Row) -> El {
    let frame = el().w(LOGO_SIZE.0).h(LOGO_SIZE.1).radius(4.0);
    let Some(channel) = item.channel() else { return frame };
    if let Some(pixels) = channel.logo.as_deref().and_then(|url| app.logos.get(url)) {
        return frame.pixels_fit(pixels, SurfaceFit::Fit);
    }
    let muted = tok::theme(true).text_secondary;
    let mark = match channel.placeholder(app.language) {
        Placeholder::Folder => col().child(el().w(12.0).h(5.0).fill(muted)).child(el().w(28.0).h(18.0).fill(muted)),
        Placeholder::Initial(initial) => text(initial).font(INITIAL).color(muted),
    };
    frame.fill(oklch(0.29, 0.03, 265.0)).center().child(mark)
}

/// The name and the state of what plays, with the list open or while there
/// is something to say: hidden over a video that plays, as a static panel
/// can mark an OLED screen.
fn status(app: &App) -> Option<El> {
    let t = tok::theme(true);
    let playing = app.session.stats().is_some_and(|stats| stats.state == Viewing::Playing);
    if !app.list_open && playing && matches!(app.note, Note::None) && !app.volume_shown() {
        return None;
    }
    let (name, status) = (app.playing(), app.status());
    if name.is_none() && status.is_none() {
        return None;
    }
    let mut panel = col()
        .key("status")
        .absolute(if app.list_open { PANEL_WIDTH + 24.0 } else { 24.0 }, 24.0)
        .px(14.0)
        .py(10.0)
        .gap(4.0)
        .radius(10.0)
        .fill(Color::BLACK.alpha(0.6));
    if let Some(name) = name {
        panel = panel.child(text(name).font(NAME).color(t.text));
    }
    if let Some(status) = status {
        panel = panel.child(text(status).font(DETAIL).color(t.text_secondary));
    }
    Some(panel)
}
