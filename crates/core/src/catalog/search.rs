//! Finding channels and events by name: the text typed against each one's
//! title and group, regardless of case and accents, so that `serie a`
//! finds the matches of Serie A and `fior` Monza - Fiorentina.

use super::Channel;

/// At most this many channels found, the first in order.
pub const MAX_FOUND: usize = 200;

/// The channels of `channels` whose title or group contains every word of
/// `query`, in their order; none for a query without words.
pub fn search(channels: &[Channel], query: &str) -> Vec<Channel> {
    let words: Vec<String> = fold(query).split_whitespace().map(str::to_string).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let matches = |channel: &&Channel| {
        let text = fold(&format!("{} {}", channel.name.as_deref().unwrap_or(""), channel.group.as_deref().unwrap_or("")));
        words.iter().all(|word| text.contains(word.as_str()))
    };
    channels.iter().filter(matches).take(MAX_FOUND).cloned().collect()
}

/// `text` in lowercase, the accents of Latin letters taken off.
fn fold(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).map(unaccented).collect()
}

fn unaccented(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => 'e',
        'ğ' => 'g',
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ı' => 'i',
        'ł' | 'ľ' => 'l',
        'ñ' | 'ń' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => 'o',
        'ř' => 'r',
        'ś' | 'š' | 'ş' | 'ș' => 's',
        'ť' | 'ţ' | 'ț' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => 'u',
        'ý' | 'ÿ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ChannelError;

    fn channel(name: &str, group: &str) -> Channel {
        let group = (!group.is_empty()).then(|| group.to_string());
        Channel { name: Some(name.into()), logo: None, group, starts_at: None, play: Err(ChannelError::NoStream) }
    }

    fn names(found: &[Channel]) -> Vec<&str> {
        found.iter().map(|c| c.name.as_deref().unwrap()).collect()
    }

    #[test]
    fn every_word_is_found_in_the_title_or_the_group_regardless_of_case_and_accents() {
        let channels = [
            channel("Monza - Fiorentina", "Italian Serie A"),
            channel("Botafogo FR - São Paulo", "Brasile Serie A"),
            channel("Rai 1", "Rai"),
            channel("Malmö - Hammarby", ""),
        ];
        assert_eq!(names(&search(&channels, "fior")), ["Monza - Fiorentina"]);
        assert_eq!(names(&search(&channels, "SERIE a")), ["Monza - Fiorentina", "Botafogo FR - São Paulo"]);
        assert_eq!(names(&search(&channels, "sao serie")), ["Botafogo FR - São Paulo"]);
        assert_eq!(names(&search(&channels, "MALMO")), ["Malmö - Hammarby"]);
        assert_eq!(names(&search(&channels, "rai 1")), ["Rai 1"]);
        assert!(search(&channels, "  ").is_empty());
        assert!(search(&channels, "juventus").is_empty());
    }

    #[test]
    fn at_most_max_found_are_given() {
        let channels: Vec<Channel> = (0..MAX_FOUND + 5).map(|i| channel(&format!("Event {i}"), "")).collect();
        assert_eq!(search(&channels, "event").len(), MAX_FOUND);
    }
}
