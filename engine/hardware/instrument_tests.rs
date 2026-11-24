// Tests every PEQUOD Keys instrument shares (included into each crate's `tests` module).
// The crate provides `Dsp`, `defaults()`, `presets()`, and `tune_params()` - settings clean
// enough to hear the pitch (no vibrato, no Leslie...).

fn render(p: &[f32], events: &[(usize, Midi)], n: usize) -> (Vec<f32>, Vec<f32>) {
    let mut d = Dsp::new(48000.0, 2);
    d.set_params(p);
    d.reset();
    let mut m = vec![0.0f32; Dsp::METERS.max(1)];
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];
    let mut k = 0;
    let mut ei = 0;
    while k < n {
        while ei < events.len() && events[ei].0 <= k {
            d.midi(events[ei].1);
            ei += 1;
        }
        let next = events.get(ei).map(|e| e.0).unwrap_or(n).max(k + 1);
        let e = (k + 256).min(n).min(next);
        d.set_params(p);
        {
            let mut ch: Vec<&mut [f32]> = vec![&mut l[k..e], &mut r[k..e]];
            d.process(&mut ch, &mut m);
        }
        k = e;
    }
    (l, r)
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

fn with(mut p: Vec<f32>, set: &[(usize, f32)]) -> Vec<f32> {
    for (i, v) in set {
        p[*i] = *v;
    }
    p
}

/// Fundamental by autocorrelation (the first strong peak), in Hz.
fn pitch(x: &[f32]) -> f64 {
    let fs = 48000.0;
    let (lo, hi) = ((fs / 2000.0) as usize, (fs / 30.0) as usize);
    let w = 8192.min(x.len() - hi - 1);
    let e0: f64 = x[..w].iter().map(|v| (*v as f64).powi(2)).sum();
    let mut corr = vec![0.0f64; hi + 2];
    for lag in lo..=hi + 1 {
        let mut s = 0.0;
        for i in 0..w {
            s += x[i] as f64 * x[i + lag] as f64;
        }
        corr[lag] = s / e0.max(1e-12);
    }
    let best = corr[lo..=hi].iter().cloned().fold(f64::MIN, f64::max);
    // the first lag that gets within 10 % of the best peak (so not an octave down)
    let mut lag = lo;
    for t in lo + 1..=hi {
        if corr[t] >= 0.9 * best && corr[t] >= corr[t - 1] && corr[t] >= corr[t + 1] {
            lag = t;
            break;
        }
    }
    // parabolic interpolation
    let (a, b, c) = (corr[lag - 1], corr[lag], corr[lag + 1]);
    let den = a - 2.0 * b + c;
    let off = if den.abs() > 1e-12 { 0.5 * (a - c) / den } else { 0.0 };
    fs / (lag as f64 + off)
}

/// Every preset plays chords, soft and hard, finite and bounded, and makes a sound.
#[test]
fn presets_play_clean() {
    let base = defaults();
    let mut all = vec![("(defaults)", base.clone())];
    for pr in presets() {
        all.push((pr.name, with(base.clone(), pr.values)));
    }
    for (name, p) in all {
        let ev = [
            (0, Midi::On { note: 48, vel: 0.3 }),
            (100, Midi::On { note: 60, vel: 0.7 }),
            (200, Midi::On { note: 64, vel: 1.0 }),
            (300, Midi::On { note: 67, vel: 0.5 }),
            (24000, Midi::Off { note: 48 }),
            (24000, Midi::Off { note: 60 }),
            (30000, Midi::On { note: 84, vel: 0.9 }),
            (40000, Midi::Off { note: 64 }),
            (40000, Midi::Off { note: 67 }),
            (42000, Midi::Off { note: 84 }),
        ];
        let (l, r) = render(&p, &ev, 72000);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() < 8.0), "{name}: unbounded");
        let pk = l[..24000].iter().chain(r[..24000].iter()).fold(0.0f32, |a, v| a.max(v.abs()));
        println!("{name:28} chord peak {:6.1} dBFS, rms {:6.1} dBFS", 20.0 * (pk as f64).log10(), 20.0 * rms(&l[2000..24000]).log10());
        assert!(rms(&l[2000..24000]) > 1e-3, "{name}: silent ({})", rms(&l[2000..24000]));
        assert!(pk < 2.0, "{name}: far too loud ({pk})");
    }
}

/// A4 plays at 440 Hz (and A2 at 110).
#[test]
fn plays_in_tune() {
    for (note, want) in [(69u8, 440.0f64), (45, 110.0)] {
        let (l, _) = render(&tune_params(), &[(0, Midi::On { note, vel: 0.8 })], 48000);
        let f = pitch(&l[9600..]);
        println!("note {note}: {f:.2} Hz");
        assert!((f / want).log2().abs() * 1200.0 < 8.0, "note {note}: {f:.2} Hz, want {want}");
    }
}

/// Released notes stop (or fade) - nothing hangs.
#[test]
fn notes_end() {
    let (l, _) = render(&tune_params(), &[(0, Midi::On { note: 60, vel: 0.8 }), (12000, Midi::Off { note: 60 })], 288000);
    let tail = rms(&l[264000..]);
    println!("tail {tail:.2e}");
    assert!(tail < 1e-3, "still sounding: {tail}");
}

/// Every control sits on the panel. (`PANEL_DUMP=dir` also writes the panel's JSON there.)
#[test]
fn panel_fits() {
    let p = panel();
    for it in &p.items {
        let (x, y) = match it {
            Item::Knob { x, y, .. } | Item::Switch { x, y, .. } | Item::Selector { x, y, .. } | Item::Text { x, y, .. } | Item::Rotor { x, y, .. } | Item::Caption { x, y, .. } => (*x, *y),
            _ => continue,
        };
        assert!(x > 0.0 && x < p.size.0 as f32 && y > 0.0 && y < p.size.1 as f32, "item at {x},{y} is off the panel");
    }
    if let Ok(dir) = std::env::var("PANEL_DUMP") {
        std::fs::write(format!("{dir}/{}.json", p.title.replace(' ', "_")), vintage::native::panel_json(&p)).unwrap();
    }
}
