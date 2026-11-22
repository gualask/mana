use super::*;

/// A ladder of 1080p at 4.2 Mbit/s, 720p at 2.2 and 360p at 1, as the
/// lab serves, fed 2 s segments of the variant followed, on a clock of
/// its own. It plays 1080p already, as after the first measures on a fast
/// network: `Ladder::new` alone starts lower.
struct Live {
    ladder: Ladder,
    choice: QualityChoice,
    stats: Arc<Mutex<Stats>>,
    start: Instant,
    /// Seconds since the start.
    t: f64,
}

impl Live {
    fn new() -> Self {
        let variant = |bandwidth, width: u32, height: u32| Variant {
            bandwidth,
            resolution: Some(format!("{width}x{height}")),
            url: Url::parse(&format!("https://lab.invalid/{height}.m3u8")).unwrap(),
        };
        let variants =
            vec![variant(1_000_000, 640, 360), variant(2_200_000, 1280, 720), variant(4_200_000, 1920, 1080)];
        let (choice, stats) = (QualityChoice::default(), Arc::new(Mutex::new(Stats::default())));
        let mut ladder = Ladder::new(variants, choice.clone(), stats.clone());
        ladder.current = ladder.variants.len() - 1;
        Self { ladder, choice, stats, start: Instant::now(), t: 0.0 }
    }

    /// Downloads `segments` segments at `rate` bits per second; the next
    /// one is out 2 s after the last, or at once when behind.
    fn download(&mut self, segments: u32, rate: f64) {
        for _ in 0..segments {
            let bytes = self.ladder.variants[self.ladder.current].bandwidth as f64 * 2.0 / 8.0;
            let took = bytes * 8.0 / rate;
            self.ladder.measure(bytes as usize, Duration::from_secs_f64(took));
            self.t += took.max(2.0);
        }
    }

    /// The path switched to now, if any.
    fn choose(&mut self) -> Option<String> {
        let now = self.start + Duration::from_secs_f64(self.t);
        self.ladder.choose(now).map(|url| url.path().to_string())
    }

    fn followed(&self) -> &str {
        self.ladder.current().path()
    }

    /// Segments at `rate(t)` until second `until`, choosing after each:
    /// the seconds and paths switched to.
    fn run(&mut self, until: f64, rate: impl Fn(f64) -> f64) -> Vec<(f64, String)> {
        let mut switches = Vec::new();
        while let Some(switch) = self.next_switch(until - self.t, &rate) {
            switches.push(switch);
        }
        switches
    }

    /// Segments at `rate(t)`, choosing after each, until a switch within
    /// `seconds`: its second and path.
    fn next_switch(&mut self, seconds: f64, rate: impl Fn(f64) -> f64) -> Option<(f64, String)> {
        let until = self.t + seconds;
        while self.t < until {
            self.download(1, rate(self.t));
            if let Some(path) = self.choose() {
                return Some((self.t, path));
            }
        }
        None
    }
}

fn paths(switches: &[(f64, String)]) -> Vec<&str> {
    switches.iter().map(|(_, path)| path.as_str()).collect()
}

/// 1 Mbit/s for `low` seconds every `period`, else `high` Mbit/s.
fn swinging(low: f64, high: f64, period: f64) -> impl Fn(f64) -> f64 {
    move |t| if t % period < period / 2.0 { low * 1e6 } else { high * 1e6 }
}

#[test]
fn it_keeps_the_variant_followed_without_measures() {
    let mut live = Live::new();
    assert_eq!(live.followed(), "/1080.m3u8");
    assert_eq!(live.choose(), None);
    assert_eq!(live.stats.lock().unwrap().heights, [360, 720, 1080]);
}

#[test]
fn the_quality_chosen_is_kept_whatever_the_bandwidth_until_auto() {
    let mut live = Live::new();
    live.choice.set(Quality::Lines(720));
    assert_eq!(live.choose().as_deref(), Some("/720.m3u8"), "at once, before any measure");
    live.download(10, 0.5e6);
    assert_eq!(live.choose(), None);
    // The best at most that high, else the lightest.
    live.choice.set(Quality::Lines(576));
    assert_eq!(live.choose().as_deref(), Some("/360.m3u8"));
    live.choice.set(Quality::Lines(240));
    assert_eq!(live.choose(), None);
    assert_eq!(live.followed(), "/360.m3u8");
    // Back to auto: up once the network allows it.
    live.choice.set(Quality::Auto);
    let switches = live.run(live.t + 120.0, |_| 50e6);
    assert_eq!(paths(&switches).last(), Some(&"/1080.m3u8"));
    // A new opening starts on the choice.
    live.choice.set(Quality::Lines(720));
    assert_eq!(Ladder::new(live.ladder.variants.clone(), live.choice.clone(), live.stats.clone()).current().path(), "/720.m3u8");
}

#[test]
fn below_a_good_picture_going_up_is_tried_again_at_least_once_a_minute() {
    let mut live = Live::new();
    let (mut down, path) = live.next_switch(30.0, |_| 1.5e6).unwrap();
    assert_eq!(path, "/360.m3u8");
    // From 360p up to 720p and back, again and again: each wrong one
    // doubles the hold, but from 360p it lasts at most a minute.
    for _ in 0..4 {
        let (up, path) = live.next_switch(120.0, |_| 4e6).unwrap();
        assert_eq!(path, "/720.m3u8");
        assert!(up - down <= MAX_UP_HOLD_BELOW_GOOD.as_secs_f64() + 10.0, "{up} {down}");
        let path;
        (down, path) = live.next_switch(30.0, |_| 1.5e6).unwrap();
        assert_eq!(path, "/360.m3u8");
    }
    assert_eq!(live.ladder.up_hold, MAX_UP_HOLD);
}

#[test]
fn a_slower_network_moves_down_to_the_variant_that_fits_then_up_again() {
    let mut live = Live::new();
    live.download(3, 50e6);
    assert_eq!(live.choose(), None);
    // 2.5 Mbit/s: 720p needs 2.2, within 95%; 1080p does not.
    live.download(3, 2.5e6);
    assert_eq!(live.choose().as_deref(), Some("/720.m3u8"));
    live.download(10, 2.5e6);
    assert_eq!(live.choose(), None, "720p fits in 95%, 1080p not in 70%");
    // Slower still: down to the lightest, even if it does not fit.
    live.download(2, 0.3e6);
    assert_eq!(live.choose().as_deref(), Some("/360.m3u8"));
    assert_eq!(live.choose(), None);
    // Back to a fast network: up a step at a time, once the slow
    // average has caught up.
    let back = live.t;
    let switches = live.run(back + 200.0, |_| 50e6);
    assert_eq!(paths(&switches), ["/720.m3u8", "/1080.m3u8"]);
    assert!(switches[1].0 - back < 120.0, "{switches:?}");
}

#[test]
fn a_short_burst_does_not_move_up() {
    let mut live = Live::new();
    live.download(10, 2e6);
    assert_eq!(live.choose().as_deref(), Some("/360.m3u8"));
    // One fast download: the slow average stays low.
    live.download(1, 50e6);
    assert_eq!(live.choose(), None);
}

#[test]
fn going_up_waits_after_going_down_and_longer_each_time_it_proves_wrong() {
    let mut live = Live::new();
    let (down, path) = live.next_switch(30.0, |_| 3e6).unwrap();
    assert_eq!((path.as_str(), live.ladder.up_hold), ("/720.m3u8", UP_HOLD));
    // Fast at once: up only once the hold is over.
    let (up, path) = live.next_switch(60.0, |_| 50e6).unwrap();
    assert_eq!(path, "/1080.m3u8");
    assert!(up - down >= UP_HOLD.as_secs_f64());
    // Down again within UP_PROVEN: the next hold is UP_PROVEN, then it
    // doubles at each wrong one, up to MAX_UP_HOLD.
    for hold in [UP_PROVEN, UP_PROVEN * 2, MAX_UP_HOLD, MAX_UP_HOLD] {
        let (down, path) = live.next_switch(30.0, |_| 2.6e6).unwrap();
        assert_eq!((path.as_str(), live.ladder.up_hold), ("/720.m3u8", hold));
        let (up, path) = live.next_switch(hold.as_secs_f64() + 60.0, |_| 50e6).unwrap();
        assert_eq!(path, "/1080.m3u8");
        assert!(up - down >= hold.as_secs_f64(), "{up} {down}");
    }
    // Up and kept for UP_PROVEN: back to UP_HOLD.
    assert_eq!(live.next_switch(UP_PROVEN.as_secs_f64(), |_| 50e6), None);
    let (_, path) = live.next_switch(30.0, |_| 2.6e6).unwrap();
    assert_eq!((path.as_str(), live.ladder.up_hold), ("/720.m3u8", UP_HOLD));
}

#[test]
fn a_network_swinging_around_what_a_variant_needs_does_not_swing_the_picture() {
    // Ten minutes, swinging every 20 to 120 s around what 1080p needs,
    // or between 2 Mbit/s and enough for 1080p. Without the hold, up to
    // a switch per swing (33 at 2 and 8 Mbit/s every 20 s). With it,
    // from 720p a wrong way up after each hold (10, 60, 120 and 240 s),
    // then at most one every MAX_UP_HOLD; from 360p, below a good
    // picture, at most one a minute.
    for (low, high) in [(4.0, 8.0), (2.0, 8.0), (4.3, 6.5)] {
        for period in [20.0, 40.0, 60.0, 90.0, 120.0] {
            let switches = Live::new().run(600.0, swinging(low, high, period));
            let wrong = wrong_ways_up(&switches);
            let (from_good, from_low): (Vec<_>, Vec<_>) = wrong.iter().partition(|(_, from)| from != "/360.m3u8");
            let late = from_good.iter().filter(|(t, _)| *t >= 300.0).count();
            let case = format!("{low}-{high} Mbit/s ogni {period} s: {switches:?}");
            assert!(from_good.len() <= 4 && late <= 1, "{case}");
            assert!(from_low.len() <= 10, "{case}");
        }
    }
}

/// Before any measure: the best at most 576 lines high, by bandwidth when
/// the heights are unknown, else the lightest; up at the first measure of
/// a fast network.
#[test]
fn it_starts_on_european_sd_and_goes_up_at_the_first_fast_measure() {
    let variant = |bandwidth, resolution: Option<&str>| Variant {
        bandwidth,
        resolution: resolution.map(Into::into),
        url: Url::parse(&format!("https://lab.invalid/{bandwidth}.m3u8")).unwrap(),
    };
    let rai = vec![variant(1_365_000, Some("768x432")), variant(2_137_000, Some("1024x576")), variant(2_793_000, Some("1280x720"))];
    let pluto: Vec<Variant> = [640_000, 1_042_000, 1_577_000, 2_294_000, 3_321_000].map(|b| variant(b, None)).into();
    let high = vec![variant(2_200_000, Some("1280x720")), variant(4_200_000, Some("1920x1080"))];
    let started = |variants: &[Variant]| variants[start(variants)].bandwidth;
    assert_eq!((started(&rai), started(&pluto), started(&high)), (2_137_000, 2_294_000, 2_200_000));

    let mut live = Live::new();
    live.ladder = Ladder::new(live.ladder.variants.clone(), live.choice.clone(), live.stats.clone());
    assert_eq!(live.followed(), "/360.m3u8");
    live.download(1, 50e6);
    assert_eq!(live.choose().as_deref(), Some("/1080.m3u8"));
}

#[test]
fn heights_come_from_the_resolution() {
    let variant = |resolution: Option<&str>| Variant {
        bandwidth: 1,
        resolution: resolution.map(Into::into),
        url: Url::parse("https://lab.invalid/v.m3u8").unwrap(),
    };
    assert_eq!(variant(Some("1024x576")).height(), Some(576));
    assert_eq!(variant(Some("wide")).height(), None);
    assert_eq!(variant(None).height(), None);
}

/// When the ladder went up to a variant it left for a lighter one within
/// UP_PROVEN, and from which.
fn wrong_ways_up(switches: &[(f64, String)]) -> Vec<(f64, String)> {
    let rank = |path: &str| ["/360.m3u8", "/720.m3u8", "/1080.m3u8"].iter().position(|p| *p == path).unwrap();
    let mut wrong = Vec::new();
    for (i, (t, path)) in switches.iter().enumerate().skip(1) {
        let went_up = rank(path) > rank(&switches[i - 1].1);
        if let Some((next, to)) = switches.get(i + 1)
            && went_up
            && rank(to) < rank(path)
            && next - t < UP_PROVEN.as_secs_f64()
        {
            wrong.push((*t, switches[i - 1].1.clone()));
        }
    }
    wrong
}

#[test]
fn a_lasting_change_is_still_followed() {
    // Four minutes slow, four fast: down within seconds, back up.
    let switches = Live::new().run(480.0, swinging(2.0, 50.0, 480.0));
    assert_eq!(paths(&switches), ["/360.m3u8", "/720.m3u8", "/1080.m3u8"], "{switches:?}");
    assert!(switches[0].0 < 15.0, "{switches:?}");
}
