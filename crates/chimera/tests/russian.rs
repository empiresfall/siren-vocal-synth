//! Russian through the whole chain: every sound the rules produce is one the voice can make,
//! Russian syllables follow rising-sonority onsets, and sentences/songs render in every engine.
use chimera::anatomy::human::BodyMix;
use chimera::engine::{Engine, EngineKind};
use chimera::phonetics::inventory::phoneme;
use chimera::phonetics::planner::{PhraseInput, PlanOptions, is_vowel, plan_phrases};
use chimera::phonetics::russian::RussianG2P;
use chimera::phonetics::sing::{SongOptions, SungNote, plan_song};
use chimera::phonetics::syllables::lyrics_to_syllables;
use chimera::phonetics::Segment;

const TEXT: &str = "Съешь же ещё этих мягких французских булок, да выпей чаю. Широкая электрификация южных губерний даст мощный толчок подъёму сельского хозяйства? Привет, мир!";

fn no_fallback(_: &str, _: bool) -> Vec<Segment> {
    Vec::new()
}

#[test]
fn every_russian_sound_is_in_the_inventory() {
    let phrases = RussianG2P.to_phrases(TEXT, &no_fallback);
    assert_eq!(phrases.len(), 5);
    let mut seen = std::collections::BTreeSet::new();
    for p in &phrases {
        for s in &p.segments {
            assert!(is_vowel(&s.ipa, BodyMix::MALE) || phoneme(&s.ipa, BodyMix::MALE).is_some(), "the voice has no /{}/", s.ipa);
            seen.insert(s.ipa.clone());
        }
        assert_eq!(p.segments.iter().filter(|s| s.word_end == Some(true)).count(), p.words.len());
    }
    println!("{} sounds: {}", seen.len(), seen.into_iter().collect::<Vec<_>>().join(" "));
}

#[test]
fn a_russian_sentence_speaks_in_every_engine() {
    let phrases = RussianG2P.to_phrases("Привет, мир! Как дела?", &no_fallback);
    let inputs: Vec<PhraseInput> = phrases.iter().map(|p| PhraseInput { segments: p.segments.clone(), intonation: Some(p.intonation), pause: Some(p.pause) }).collect();
    let plan = plan_phrases(&inputs, &PlanOptions { body: "male".into(), t0: 0.05, f0: Some(112.0), ..Default::default() });
    for kind in EngineKind::ALL {
        let mut e = Engine::new(48000.0);
        e.set_kind(kind);
        e.schedule(&plan.gestures, 0.0, true);
        let mut out = vec![0f32; ((plan.end + 0.2) * 48000.0) as usize];
        e.render(&mut out);
        let rms = (out.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / out.len() as f64).sqrt();
        let peak = out.iter().fold(0f32, |m, x| m.max(x.abs()));
        println!("{kind:?}: {:.2} s, rms {:.1} dB, peak {peak:.2}", plan.end, 20.0 * rms.log10());
        assert!(out.iter().all(|x| x.is_finite()) && rms > 0.01 && peak < 0.8, "{kind:?}");
    }
}

#[test]
fn russian_syllables_keep_onsets_and_hiatus_clean() {
    let g = RussianG2P;
    let syls_of = |s: &str| lyrics_to_syllables(s, &|w: &str, reduce: bool| g.word(w, reduce));

    // хорошо -> [xɐ] [rɐ] [ʂo]: /r/ and /ʂ/ lead their own syllables instead of trailing as codas
    let xor = syls_of("хорошо");
    assert_eq!(xor.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["хо", "ро", "шо"]);
    assert_eq!(xor[0].coda.len(), 0);
    assert_eq!(xor[1].onset[0].ipa, "r");
    assert_eq!(xor[2].onset[0].ipa, "ʂ");

    // молоко -> [mɐ] [ɫɐ] [ko]: velarised /ɫ/ leads the second syllable
    let mol = syls_of("молоко");
    assert_eq!(mol.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["мо", "ло", "ко"]);
    assert_eq!(mol[1].onset[0].ipa, "ɫ");

    // hiatus without '+' (`моя`, `красивая`) splits into real syllables rather than `~` melismas
    let moya = syls_of("моя красивая Москва");
    assert_eq!(moya.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["мо", "я", "кра", "си", "ва", "я", "Мо", "сква"]);
}

#[test]
fn russian_lyrics_become_syllables_and_sing() {
    let g = RussianG2P;
    let syl = lyrics_to_syllables("Калинка, малинка мо+я", &|w: &str, reduce: bool| g.word(w, reduce));
    let text: Vec<&str> = syl.iter().map(|s| s.text.as_str()).collect();
    println!("{text:?}");
    assert_eq!(text, ["Ка", "лин", "ка", "ма", "лин", "ка", "мо", "я"]);
    assert!(syl[2].phrase_end && syl[7].word_end);
    // sung unstressed `а` stays open `[ɐ]`, stressed `я` is `[a]`
    assert_eq!(syl[0].nucleus.ipa, "ɐ");
    assert_eq!(syl[2].nucleus.ipa, "ɐ");
    assert_eq!(syl[7].nucleus.ipa, "a");
    let notes: Vec<SungNote> = syl.iter().enumerate().map(|(i, s)| SungNote { midi: 57.0 + (i % 3) as f64 * 2.0, beats: 1.0, syl: s.clone() }).collect();
    let plan = plan_song(&notes, &SongOptions { body: "male".into(), bpm: 120.0, t0: 0.2, vibrato: 30.0 });
    let mut e = Engine::new(48000.0);
    e.schedule(&plan.gestures, 0.0, true);
    let mut out = vec![0f32; ((plan.end + 0.2) * 48000.0) as usize];
    e.render(&mut out);
    assert!(out.iter().all(|x| x.is_finite()));
}
