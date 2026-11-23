use super::draw::shortened;
use super::*;

#[test]
fn long_texts_keep_their_start_and_end() {
    assert_eq!(shortened("short", 10), "short");
    let long = "https://lists.example/mana-list/0123456789abcdef/list.toml";
    let short = shortened(long, 24);
    assert_eq!(short.chars().count(), 24);
    assert_eq!(short, "https://lists.exa…t.toml");
}

#[test]
fn a_notice_reads_in_either_language_and_says_whether_the_list_was_taken() {
    assert_eq!(Notice::Saved(8).text(Language::Italian), "Lista salvata: 8 canali");
    assert_eq!(Notice::Saved(8).text(Language::English), "List saved: 8 channels");
    let failed = Notice::Failed(update::Failure::NotUtf8);
    assert_eq!(failed.text(Language::Italian), "Non salvata: non è testo UTF-8. La TV tiene la lista di prima.");
    assert!(Notice::Loaded(1).taken() && !failed.taken() && !Notice::TestListRefused.taken());
}
