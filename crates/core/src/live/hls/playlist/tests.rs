use super::*;

#[test]
fn a_playlist_may_start_with_a_bom() {
    let base = Url::parse("https://lab.invalid/live/index.m3u8").unwrap();
    let text = "\u{feff}\n#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\na.ts\n";
    assert_eq!(MediaPlaylist::parse(text, &base).unwrap().segments.len(), 1);
    assert!(!is_playlist(b"G\x47G#EXTM3U"));
}

#[test]
fn attributes_keep_quoted_commas() {
    let attributes = parse_attributes(r#"BANDWIDTH=500000,CODECS="avc1.64001f,mp4a.40.2",RESOLUTION=1280x720"#);
    assert_eq!(attributes["BANDWIDTH"], "500000");
    assert_eq!(attributes["CODECS"], "avc1.64001f,mp4a.40.2");
    assert_eq!(attributes["RESOLUTION"], "1280x720");
}

#[test]
fn live_playlist_numbers_segments_and_resolves_keys() {
    let base = Url::parse("https://lab.invalid/live/index.m3u8").unwrap();
    let text = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:41\n\
                #EXT-X-KEY:METHOD=AES-128,URI=\"key\",IV=0x0000000000000000000000000000002A\n\
                #EXTINF:2.0,\nsegment-41.ts\n#EXT-X-DISCONTINUITY\n#EXT-X-KEY:METHOD=AES-128,URI=\"../k2\"\n#EXTINF:2.0,\nsegment-42.ts\n";
    let playlist = MediaPlaylist::parse(text, &base).unwrap();
    assert!(!playlist.ended);
    assert_eq!(playlist.target_duration, Duration::from_secs(2));
    let sequences: Vec<u64> = playlist.segments.iter().map(|s| s.sequence).collect();
    assert_eq!(sequences, [41, 42]);
    assert!(!playlist.segments[0].discontinuity && playlist.segments[1].discontinuity);
    assert_eq!(playlist.segments[0].discontinuity_sequence, 0);
    assert_eq!(playlist.segments[1].discontinuity_sequence, 1);
    assert_eq!(playlist.segments[0].url.as_str(), "https://lab.invalid/live/segment-41.ts");
    let first = playlist.segments[0].key.as_ref().unwrap();
    assert_eq!(first.url.as_str(), "https://lab.invalid/live/key");
    assert_eq!(first.iv, Some(42u128.to_be_bytes()));
    let second = playlist.segments[1].key.as_ref().unwrap();
    assert_eq!(second.url.as_str(), "https://lab.invalid/k2");
    assert_eq!(second.iv, None);
}

#[test]
fn ended_playlist_and_cleared_key() {
    let base = Url::parse("http://lab.invalid/vod/index.m3u8").unwrap();
    let text = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n#EXTINF:1,\na.ts\n#EXT-X-KEY:METHOD=NONE\n#EXTINF:1,\nb.ts\n#EXT-X-ENDLIST\n";
    let playlist = MediaPlaylist::parse(text, &base).unwrap();
    assert!(playlist.ended);
    assert!(!playlist.segments[0].discontinuity);
    assert!(playlist.segments[0].key.is_some());
    assert!(playlist.segments[1].key.is_none());
}

#[test]
fn rejects_unsupported_encryption_and_non_playlists() {
    let base = Url::parse("http://lab.invalid/index.m3u8").unwrap();
    assert!(matches!(MediaPlaylist::parse("<html>", &base), Err(Error::Invalid(_))));
    let dash = "<?xml version=\"1.0\"?>\n<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"dynamic\">";
    assert!(matches!(MediaPlaylist::parse(dash, &base), Err(Error::Unsupported(_))));
    let sample_aes = MediaPlaylist::parse("#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"k\"\n", &base);
    assert!(matches!(sample_aes, Err(Error::Unsupported(_))));
    let ranged = MediaPlaylist::parse("#EXTM3U\n#EXT-X-MAP:URI=\"i.mp4\",BYTERANGE=\"720@0\"\n", &base);
    assert!(matches!(ranged, Err(Error::Unsupported(_))));
}

#[test]
fn fmp4_segments_carry_their_initialization_section() {
    let base = Url::parse("https://lab.invalid/live3/stream_0.m3u8").unwrap();
    let text = "#EXTM3U\n#EXT-X-MAP:URI=\"init_0.mp4\"\n#EXTINF:2,\na.m4s\n#EXTINF:2,\nb.m4s\n";
    let playlist = MediaPlaylist::parse(text, &base).unwrap();
    let init = Url::parse("https://lab.invalid/live3/init_0.mp4").unwrap();
    assert!(playlist.segments.iter().all(|s| s.init.as_ref() == Some(&init)));
}

#[test]
fn live_start_is_three_target_durations_from_the_end() {
    let base = Url::parse("https://lab.invalid/index.m3u8").unwrap();
    let start = |target: u32, duration: &str, count: usize| {
        let segments = (0..count).map(|i| format!("#EXTINF:{duration},\ns{i}.ts\n")).collect::<String>();
        let text = format!("#EXTM3U\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:10\n{segments}");
        MediaPlaylist::parse(&text, &base).unwrap().live_start()
    };
    assert_eq!(start(2, "2.0", 6), 13); // 6 s: 3 segments of 2 s
    assert_eq!(start(10, "10.0", 6), 13); // 30 s, as Rai 1
    assert_eq!(start(6, "4.0", 8), 13); // 18 s: 5 segments shorter than the target
    assert_eq!(start(10, "10.0", 2), 10); // a short playlist: its first segment
}

#[test]
fn segments_are_dated_from_the_last_program_date_time() {
    assert_eq!(parse_date("1970-01-01T00:00:00Z"), Some(0.0));
    assert_eq!(parse_date("2026-11-21T10:00:00.500Z"), Some(1_795_255_200.5));
    assert_eq!(parse_date("2026-11-21T11:00:00.500+01:00"), Some(1_795_255_200.5));
    assert_eq!(parse_date("2026-11-21T09:30:00.500-0030"), Some(1_795_255_200.5));
    assert_eq!(parse_date("2000-02-29T00:00:00Z"), Some(951_782_400.0));
    assert_eq!(parse_date("2026-13-01T00:00:00Z"), None);
    assert_eq!(parse_date("yesterday"), None);
    let base = Url::parse("https://lab.invalid/index.m3u8").unwrap();
    let text = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\na.ts\n\
        #EXT-X-PROGRAM-DATE-TIME:1970-01-01T00:01:00Z\n#EXTINF:2,\nb.ts\n#EXTINF:1.5,\nc.ts\n#EXTINF:2,\nd.ts\n";
    let dates: Vec<_> = MediaPlaylist::parse(text, &base).unwrap().segments.iter().map(|s| s.date).collect();
    assert_eq!(dates, [None, Some(60.0), Some(62.0), Some(63.5)]);
}

#[test]
fn durations_are_checked_and_the_target_is_at_least_a_second() {
    let base = Url::parse("https://lab.invalid/index.m3u8").unwrap();
    let parse = |target: &str, duration: &str| {
        MediaPlaylist::parse(&format!("#EXTM3U\n#EXT-X-TARGETDURATION:{target}\n#EXTINF:{duration},\na.ts\n"), &base)
    };
    let fractional = parse("4", "3.84").unwrap();
    assert_eq!(fractional.segments[0].duration, Duration::from_secs_f64(3.84));
    assert_eq!(parse("0", "0").unwrap().target_duration, MIN_TARGET_DURATION);
    for (target, duration) in [("-2", "2"), ("inf", "2"), ("NaN", "2"), ("2", "-1"), ("2", "1e300"), ("2", "inf"), ("2", "abc")] {
        assert!(matches!(parse(target, duration), Err(Error::Invalid(_))), "{target} {duration}");
    }
}
