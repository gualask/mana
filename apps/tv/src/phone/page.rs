//! The page the phone opens: the list's URL in use on the TV, and the form
//! that changes it, open only when there is nothing to show or the URL sent
//! was refused. Plain HTML, no JavaScript.

use mana_core::language::Language;
use mana_core::text;

use crate::palette::{BACKGROUND, ERROR, FOCUS, MUTED, OK, ROW, TEXT};

/// What the page says about the last URL sent.
pub enum Notice<'a> {
    None,
    Saved(&'a str),
    /// Why `sent` was not taken: the form keeps it, to be corrected.
    Refused { text: &'a str, sent: &'a str },
}

/// The page at `path`, with `in_use` the URL of the TV's list, if any, in
/// `language`.
pub fn page(path: &str, in_use: Option<&str>, notice: Notice, language: Language) -> String {
    let (notice, sent) = match notice {
        Notice::None => (String::new(), None),
        Notice::Saved(text) => {
            let done = text!(language, "phone-done");
            (format!("<p class=\"ok\">✓ {}</p><p class=\"muted\">{done}</p>", escape(text)), None)
        }
        Notice::Refused { text, sent } => (format!("<p class=\"no\">{}</p>", escape(text)), Some(sent)),
    };
    let shown = in_use.map_or(String::new(), |url| {
        let label = text!(language, "phone-in-use");
        format!("<div class=\"box\"><p class=\"muted\">{label}</p><p>{}</p></div>", escape(url))
    });
    let form = match (sent, in_use) {
        (None, Some(url)) => {
            let change = text!(language, "phone-change");
            format!("<details><summary>{change}</summary>{}</details>", form(path, url, language))
        }
        (sent, _) => form(path, sent.unwrap_or_default(), language),
    };
    let (tag, title, heading) =
        (language.tag(), text!(language, "phone-title"), text!(language, "phone-heading"));
    format!(
        "<!doctype html><html lang=\"{tag}\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{title}</title><style>\
body{{font-family:system-ui,sans-serif;margin:0 auto;max-width:560px;padding:24px 16px;background:#{BACKGROUND:06x};color:#{TEXT:06x}}}\
h1{{font-size:1.4em;margin:0 0 16px}}p{{line-height:1.4;overflow-wrap:anywhere;margin:0 0 12px}}\
.box{{padding:12px 14px 2px;border-radius:8px;background:#{ROW:06x};margin-bottom:16px}}\
.muted,label{{color:#{MUTED:06x}}}.ok{{color:#{OK:06x}}}.no{{color:#{ERROR:06x}}}label{{display:block;margin-bottom:8px}}\
textarea{{box-sizing:border-box;width:100%;min-height:7em;font:inherit;font-size:16px;padding:10px;border-radius:8px;border:1px solid #{FOCUS:06x};background:#{ROW:06x};color:inherit;word-break:break-all}}\
button,summary{{display:block;box-sizing:border-box;width:100%;padding:14px;font:inherit;font-size:1.1em;text-align:center;border-radius:8px;cursor:pointer}}\
button{{margin-top:16px;border:0;background:#{FOCUS:06x};color:#fff}}\
summary{{list-style:none;border:1px solid #{FOCUS:06x};color:#{TEXT:06x}}}summary::-webkit-details-marker{{display:none}}details[open] summary{{display:none}}\
</style></head><body><h1>{heading}</h1>{notice}{shown}{form}</body></html>"
    )
}

/// The form that sends a URL, `url` filled in.
fn form(path: &str, url: &str, language: Language) -> String {
    let (label, save) = (text!(language, "phone-address"), text!(language, "phone-save"));
    format!(
        "<form method=\"post\" action=\"{}\"><label for=\"url\">{label}</label>\
<textarea id=\"url\" name=\"url\" autocapitalize=\"off\" autocorrect=\"off\" spellcheck=\"false\" required>{}</textarea>\
<button type=\"submit\">{save}</button></form>",
        escape(path),
        escape(url),
    )
}

/// `text` safe inside HTML text and attribute values.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            c => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    const IN_USE: &str = "https://a.example/l?x=1&y=\"2\"";

    #[test]
    fn the_url_and_the_notice_are_escaped() {
        let page = page("/K7QM", Some(IN_USE), Notice::Refused { text: "<b>no</b>", sent: "ftp://x" }, Language::Italian);
        assert!(page.contains("<p>https://a.example/l?x=1&amp;y=&quot;2&quot;</p>"));
        assert!(page.contains("&lt;b&gt;no&lt;/b&gt;"));
        assert!(page.contains("action=\"/K7QM\""));
        assert!(!page.contains("<script"));
        // The colours are the TV's, as six hex digits.
        assert!(page.contains("background:#10131a") && page.contains(".ok{color:#8fe3a0}"));
    }

    /// With a list in use the form waits behind «Cambia indirizzo»; without
    /// one, or after a refusal, it is open, with what was sent to correct.
    #[test]
    fn the_form_is_open_only_when_needed() {
        let it = Language::Italian;
        let closed = page("/K7QM", Some(IN_USE), Notice::None, it);
        assert!(closed.contains("<details><summary>Cambia indirizzo</summary><form"));
        let saved = page("/K7QM", Some(IN_USE), Notice::Saved("Lista salvata: 8 canali"), it);
        assert!(saved.contains("Lista salvata: 8 canali") && saved.contains("puoi chiudere") && saved.contains("<details>"));
        let empty = page("/K7QM", None, Notice::None, it);
        assert!(!empty.contains("<details>") && empty.contains("required></textarea>"));
        let refused = page("/K7QM", Some(IN_USE), Notice::Refused { text: "Non salvata", sent: "ftp://x" }, it);
        assert!(!refused.contains("<details>") && refused.contains(">ftp://x</textarea>"));
    }

    #[test]
    fn the_page_is_in_the_app_s_language() {
        let english = page("/K7QM", Some(IN_USE), Notice::None, Language::English);
        assert!(english.contains("<html lang=\"en\">") && english.contains("<h1>Channel list</h1>"));
        assert!(english.contains("<summary>Change address</summary>") && english.contains(">Save</button>"));
        let italian = page("/K7QM", Some(IN_USE), Notice::None, Language::Italian);
        assert!(italian.contains("<html lang=\"it\">") && italian.contains(">Salva</button>"));
    }
}
