use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use super::*;
use crate::live::Stats;
use crate::live::hls::Live;
use crate::live::hls::ts::{PACKET, SYNC};
use crate::live::source::Source;
use crate::playback::Input;

/// `count` packets filled with `fill`, the first a program association
/// table.
fn filled(count: usize, fill: u8) -> Vec<u8> {
    let mut data = Vec::new();
    for _ in 0..count {
        data.push(SYNC);
        data.extend([fill; PACKET - 1]);
    }
    data[1..3].copy_from_slice(&[0x40, 0]);
    data
}

#[test]
fn the_start_of_an_answer_is_held_for_the_margin() {
    let mut margin = Margin::default();
    assert!(margin.pass(Vec::new(), false).is_empty());
    assert!(margin.pass(vec![1], false).is_empty());
    margin.since = Some(Instant::now() - START_MARGIN);
    assert_eq!(margin.pass(vec![2], false), [vec![1], vec![2]]);
    assert_eq!(margin.pass(vec![3], false), [vec![3]]);
    let mut cut_short = Margin::default();
    assert!(cut_short.pass(vec![1], false).is_empty());
    assert_eq!(cut_short.pass(vec![2], true), [vec![1], vec![2]]); // an answer breaking off
    // A burst as large as the queue goes on before the margin's time.
    let mut burst = Margin::default();
    for _ in 1..QUEUED_CHUNKS {
        assert!(burst.pass(vec![0; CHUNK], false).is_empty());
    }
    assert_eq!(burst.pass(vec![0; CHUNK], false).len(), QUEUED_CHUNKS);
}

#[test]
fn what_comes_before_the_tables_is_dropped() {
    let tables = filled(2, 7);
    let mut chunk = [vec![SYNC; 100], tables.clone()].concat();
    let mut first = TablesFirst::default();
    first.trim(&mut chunk);
    assert_eq!(chunk, tables);
    let mut later = vec![9; 50];
    first.trim(&mut later);
    assert_eq!(later, [9; 50]); // once found, everything goes on
    let mut none = TablesFirst::default();
    let mut junk = vec![1; 1000];
    none.trim(&mut junk);
    assert!(junk.is_empty());
}

#[test]
fn each_answer_is_a_section_and_the_first_is_not_asked_twice() {
    let answers = [filled(8, 1), filled(4, 2), filled(2, 3)];
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/live.ts", listener.local_addr().unwrap());
    let served = answers.clone();
    std::thread::spawn(move || {
        for body in served {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                line.clear();
            }
            let mut stream = reader.into_inner();
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(&body);
        }
    });
    let viewing = Arc::new(AtomicBool::new(false));
    let stats = Arc::new(Mutex::new(Stats::default()));
    let source = Source::direct(url, Vec::new());
    let live = Live::resolve(&source, &Stop::new(&viewing), stats, &Default::default()).unwrap().unwrap();
    let stop = Stop::new(&viewing);
    let mut stream = live.open(&stop).unwrap().main;
    for (section, answer) in answers[..2].iter().enumerate() {
        let mut read = Vec::new();
        stream.read_to_end(&mut read).unwrap();
        assert_eq!(stream.discontinuity_sequence(), section as u64);
        assert_eq!(&read, answer);
        assert!(stream.next_section());
    }
    stop.set();
}
