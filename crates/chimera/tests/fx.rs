//! The effects rack: off is a wire, every effect stays bounded on speech at any device rate, and
//! the effects do what their names say (the cow's echoes slide in pitch, the golden reverb rings
//! for as long as its decay says, the phone narrows the band).
use chimera::engine::Engine;
use chimera::fx::{FX_INFO, Fx, FxChain, N_FX_PARAMS};
use chimera::phonetics::ipa::tokenize_ipa;
use chimera::phonetics::planner::{PlanOptions, plan_utterance};

fn speech(fs: f64) -> Vec<f32> {
    let mut e = Engine::new(fs);
    let p = plan_utterance(&tokenize_ipa("ˈhɛloʊ ˈwɝld"), &PlanOptions { body: "male".into(), t0: 0.05, f0: Some(110.0), ..Default::default() });
    e.schedule(&p.gestures, 0.0, true);
    let mut out = vec![0f32; ((p.end + 0.2) * fs) as usize];
    e.render(&mut out);
    out
}

fn run(c: &mut FxChain, x: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![0f32; x.len()], vec![0f32; x.len()]);
    // in uneven blocks, as a host would
    let mut i = 0;
    for n in [128usize, 1, 500, 64].iter().cycle() {
        if i >= x.len() {
            break;
        }
        let e = (i + n).min(x.len());
        c.process(&x[i..e], &mut l[i..e], &mut r[i..e]);
        i = e;
    }
    (l, r)
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// Level (linear) of `x` at `hz` (a single DFT bin, Hann window).
fn level_at(x: &[f32], fs: f64, hz: f64) -> f64 {
    let n = x.len() as f64;
    let (mut re, mut im) = (0.0, 0.0);
    for (i, v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n).cos();
        let ph = std::f64::consts::TAU * hz * i as f64 / fs;
        re += *v as f64 * w * ph.cos();
        im -= *v as f64 * w * ph.sin();
    }
    (re * re + im * im).sqrt() / n
}

#[test]
fn with_everything_off_the_rack_is_a_wire() {
    let x = speech(48000.0);
    let mut c = FxChain::new(48000.0);
    assert!(c.is_idle());
    let (l, r) = run(&mut c, &x);
    assert!(l == x && r == x);
}

#[test]
fn every_effect_stays_bounded_at_every_setting_and_rate() {
    for fs in [16000.0, 44100.0, 48000.0, 96000.0] {
        let x = speech(fs);
        for fx in Fx::ALL {
            for setting in [0.0, 0.5, 1.0] {
                let mut c = FxChain::new(fs);
                c.set_on(fx, true);
                for k in 0..N_FX_PARAMS {
                    c.set_param(fx, k, setting);
                }
                let (l, r) = run(&mut c, &x);
                let peak = l.iter().chain(&r).fold(0f32, |m, v| m.max(v.abs()));
                assert!(l.iter().chain(&r).all(|v| v.is_finite()) && peak <= 0.9, "{} at {setting} / {fs} Hz: peak {peak}", fx.info().id);
            }
            // and at its defaults it is audibly not the dry voice
            let mut c = FxChain::new(fs);
            c.set_on(fx, true);
            let (l, _) = run(&mut c, &x);
            let diff: Vec<f32> = l.iter().zip(&x).map(|(a, b)| a - b).collect();
            assert!(rms(&diff) > 0.02 * rms(&x), "{} at its defaults leaves the voice as it was", fx.info().id);
        }
    }
}

#[test]
fn everything_on_at_once_stays_bounded_and_switching_off_fades() {
    let x = speech(48000.0);
    let mut c = FxChain::new(48000.0);
    for fx in Fx::ALL {
        c.set_on(fx, true);
    }
    let (l, r) = run(&mut c, &x);
    assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() <= 0.9));
    for fx in Fx::ALL {
        c.set_on(fx, false);
    }
    let silence = vec![0f32; 9600];
    let (l, _) = run(&mut c, &silence);
    assert!(c.is_idle());
    assert!(l[9000..].iter().all(|v| v.abs() < 1e-3), "a tail survived switching everything off");
}

#[test]
fn the_cows_echo_comes_back_lower_each_time() {
    let fs = 48000.0;
    let mut c = FxChain::new(fs);
    c.set_on(Fx::Cow, true);
    // 0.2 s echo, strong feedback, shift down by 120·0.6² = 43.2 Hz per pass, echo only
    let time = ((0.2f64 - 0.03) / 0.67).sqrt();
    c.set_param(Fx::Cow, 0, time);
    c.set_param(Fx::Cow, 1, 0.9);
    c.set_param(Fx::Cow, 2, 0.2);
    c.set_param(Fx::Cow, 3, 1.0);
    // let the controls settle (a moving delay time bends the pitch, like tape)
    run(&mut c, &vec![0f32; 48000]);
    // a 0.1 s burst of 1 kHz
    let n = (1.2 * fs) as usize;
    let x: Vec<f32> = (0..n).map(|i| if i < 4800 { 0.3 * (std::f64::consts::TAU * 1000.0 * i as f64 / fs).sin() as f32 } else { 0.0 }).collect();
    let (l, _) = run(&mut c, &x);
    for (pass, want) in [(1usize, 1000.0 - 43.2), (2, 1000.0 - 86.4), (3, 1000.0 - 129.6)] {
        let a = (pass as f64 * 0.2 * fs) as usize;
        let seg = &l[a..a + 4800];
        let at = level_at(seg, fs, want);
        let dry = level_at(seg, fs, 1000.0);
        println!("echo {pass}: {:.4} at {want:.0} Hz, {:.4} at 1000 Hz, {:.4} mirrored", at, dry, level_at(seg, fs, 2000.0 - want));
        assert!(at > 0.01 && at > 4.0 * dry, "echo {pass} is not at {want:.0} Hz");
    }
}

#[test]
fn the_golden_reverb_rings_as_long_as_its_decay_says() {
    let fs = 48000.0;
    for (setting, seconds) in [(0.25, 0.3 * 40f64.powf(0.25)), (0.6, 0.3 * 40f64.powf(0.6))] {
        let mut c = FxChain::new(fs);
        c.set_on(Fx::Golden, true);
        c.set_param(Fx::Golden, 1, setting);
        c.set_param(Fx::Golden, 2, 0.0);
        c.set_param(Fx::Golden, 3, 1.0);
        let n = (fs * (seconds + 1.0)) as usize;
        let mut x = vec![0f32; n];
        x[..480].iter_mut().enumerate().for_each(|(i, v)| *v = if i % 7 < 3 { 0.5 } else { -0.5 });
        let (l, _) = run(&mut c, &x);
        let win = |t: f64| 20.0 * rms(&l[(t * fs) as usize..(t * fs) as usize + 2400]).log10();
        // the level falls 60 dB in `seconds`: measure the slope between 20 % and 70 % of it
        let (a, b) = (win(0.2 * seconds), win(0.7 * seconds));
        let t60 = 60.0 * 0.5 * seconds / (a - b);
        println!("decay {setting}: wanted {seconds:.2} s, measured {t60:.2} s");
        assert!((t60 / seconds - 1.0).abs() < 0.3, "wanted {seconds:.2} s, measured {t60:.2} s");
    }
}

#[test]
fn the_phone_keeps_only_the_middle_of_the_band() {
    let fs = 48000.0;
    let level = |hz: f64| {
        let mut c = FxChain::new(fs);
        c.set_on(Fx::Phone, true);
        c.set_param(Fx::Phone, 1, 0.0);
        c.set_param(Fx::Phone, 2, 0.0);
        let x: Vec<f32> = (0..24000).map(|i| 0.2 * (std::f64::consts::TAU * hz * i as f64 / fs).sin() as f32).collect();
        let (l, _) = run(&mut c, &x);
        rms(&l[12000..])
    };
    let (low, mid, high) = (level(80.0), level(1200.0), level(9000.0));
    println!("phone: 80 Hz {low:.4}, 1.2 kHz {mid:.4}, 9 kHz {high:.4}");
    assert!(mid > 5.0 * low && mid > 5.0 * high);
}

#[test]
fn the_catalogue_names_every_effect_and_control() {
    assert_eq!(FX_INFO.len(), Fx::ALL.len());
    for fx in Fx::ALL {
        assert_eq!(Fx::parse(fx.info().id), Some(fx));
        assert!(fx.info().params.iter().all(|p| !p.is_empty()));
    }
}
