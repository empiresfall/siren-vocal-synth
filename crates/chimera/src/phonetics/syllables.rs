//! Lyrics → sung syllables. Words come from the pronouncing dictionary (unreduced: sung function
//! words keep full vowels), are split into syllables by the maximal-onset principle with English
//! onset clusters (or Russian rising-sonority rules for Cyrillic), and the spelling is split to
//! match for display ("twin-kle", "Ка-лин-ка").
//! Typed hyphens force syllable breaks; a lone "-" (or "~") holds the previous vowel onto the
//! next note (melisma), as in vocaloid editors.

use crate::anatomy::human::BodyMix;
use super::english::js_is_space;
use super::planner::is_vowel;
use super::russian::{has_cyrillic, is_cyrillic};
use super::{Segment, Syllable};
use serde::{Deserialize, Serialize};

// English onsets (IPA), plus single non-English IPA consonants so isolated non-Latin segments
// still start their own syllable rather than falling into the previous coda.
#[rustfmt::skip]
const ONSETS: &[&str] = &[
    "p", "b", "t", "d", "k", "g", "f", "v", "θ", "ð", "s", "z", "ʃ", "ʒ", "h", "m", "n", "l", "ɹ", "w", "j", "tʃ", "dʒ",
    "r", "ɾ", "ɫ", "ʂ", "ʐ", "x", "ts",
    "pl", "bl", "kl", "gl", "fl", "sl", "pɹ", "bɹ", "tɹ", "dɹ", "kɹ", "gɹ", "fɹ", "θɹ", "ʃɹ",
    "sp", "st", "sk", "sm", "sn", "sw", "sf", "tw", "kw", "dw", "gw", "θw",
    "pj", "bj", "kj", "gj", "mj", "nj", "fj", "vj", "hj", "lj",
    "spl", "spɹ", "stɹ", "skɹ", "skw", "skj", "spj",
];

/// One syllable of a word, before it gets its text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyllableParts {
    pub onset: Vec<Segment>,
    pub nucleus: Segment,
    pub coda: Vec<Segment>,
}

/// Split one word's segments into onset / nucleus / coda syllables (English maximal-onset).
pub fn syllabify_segments(segs: &[Segment]) -> Vec<SyllableParts> {
    let vi: Vec<usize> = segs.iter().enumerate().filter(|(_, s)| is_vowel(&s.ipa, BodyMix::MALE)).map(|(i, _)| i).collect();
    if vi.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0;
    for (n, &v) in vi.iter().enumerate() {
        let mut end = segs.len(); // exclusive end of this syllable
        if let Some(&next) = vi.get(n + 1) {
            // consonants between this vowel and the next: the longest legal onset goes right
            let between = &segs[v + 1..next];
            let mut k = 0;
            for len in (1..=3.min(between.len())).rev() {
                let cluster: String = between[between.len() - len..].iter().map(|s| s.ipa.as_str()).collect();
                if ONSETS.contains(&cluster.as_str()) {
                    k = len;
                    break;
                }
            }
            end = next - k;
        }
        out.push(SyllableParts { onset: segs[start..v].to_vec(), nucleus: segs[v].clone(), coda: segs[v + 1..end].to_vec() });
        start = end;
    }
    out
}

/// Russian sonority rank (Avanesov's open-syllable / rising-sonority principle):
/// 3 = palatal glide `j`, 2 = sonorants (`r, ɾ, l, ɫ, m, n`), 1 = obstruents.
fn ru_sonority(ipa: &str) -> u8 {
    match ipa {
        "j" => 3,
        "r" | "ɾ" | "l" | "ɫ" | "m" | "n" => 2,
        _ => 1,
    }
}

/// Split Russian segments into syllables by Avanesov's rising-sonority rule:
/// - A single intervocalic consonant always begins the next syllable (`мо-ло-ко`, `хо-ро-шо`).
/// - In a cluster, any leading glide (`тай-на`) or sonorant before an obstruent/sonorant
///   (`ка-лин-ка`, `кар-та`, `вол-на`) closes the left syllable, while obstruent+sonorant and
///   sibilant/stop clusters (`Мо-сква`, `про-сто`, `о-кно`, `се-мья`) open the right syllable.
pub fn syllabify_russian_segments(segs: &[Segment]) -> Vec<SyllableParts> {
    let vi: Vec<usize> = segs.iter().enumerate().filter(|(_, s)| is_vowel(&s.ipa, BodyMix::MALE)).map(|(i, _)| i).collect();
    if vi.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0;
    for (n, &v) in vi.iter().enumerate() {
        let mut end = segs.len();
        if let Some(&next) = vi.get(n + 1) {
            let between = &segs[v + 1..next];
            let m = between.len();
            let coda_len = if m <= 1 {
                0
            } else {
                let mut c = 0;
                while c + 1 < m {
                    let s0 = ru_sonority(&between[c].ipa);
                    let s1 = ru_sonority(&between[c + 1].ipa);
                    // glide before anything, sonorant before obstruent, or two non-glide sonorants
                    if s0 > s1 || (s0 == 2 && s1 == 2) || between[c].ipa == between[c + 1].ipa {
                        c += 1;
                    } else {
                        break;
                    }
                }
                // keep onset at most 3 consonants in huge 4-consonant clusters
                c.max(m.saturating_sub(3))
            };
            end = v + 1 + coda_len;
        }
        out.push(SyllableParts { onset: segs[start..v].to_vec(), nucleus: segs[v].clone(), coda: segs[v + 1..end].to_vec() });
        start = end;
    }
    out
}

// The spelling is indexed in UTF-16 code units, as the JS string was.
fn is_spell_vowel(c: u16) -> bool {
    "aeiouy".bytes().any(|v| v as u16 == c) || is_cyrillic_vowel(c)
}
fn is_spell_vowel_ci(c: u16) -> bool {
    "aeiouyAEIOUY".bytes().any(|v| v as u16 == c) || is_cyrillic_vowel(c)
}
/// а е и о у ы э ю я ё, either case (each is one UTF-16 unit)
fn is_cyrillic_vowel(c: u16) -> bool {
    "аеиоуыэюяёАЕИОУЫЭЮЯЁ".encode_utf16().any(|v| v == c)
}
fn is_ru_sonorant(c: char) -> bool {
    matches!(c, 'р' | 'л' | 'м' | 'н' | 'й')
}
fn eq_ci(c: u16, lower: u8) -> bool {
    c == lower as u16 || c == lower.to_ascii_uppercase() as u16
}
/// `word.slice(a, b)` (indices clamped to the word)
fn slice(w: &[u16], a: usize, b: usize) -> String {
    let (a, b) = (a.min(w.len()), b.min(w.len()));
    if a >= b { String::new() } else { String::from_utf16_lossy(&w[a..b]) }
}

/// Split a Cyrillic word into `n` chunks: each Cyrillic vowel letter is its own syllable (so
/// hiatus words like `моя`, `знаю`, `красивая`, `Россия` split cleanly instead of merging into
/// `~`), `й`/`ь`/`ъ` never begin a syllable (`боль-шой`, `тай-на`), and sonorants close before
/// following consonants (`Ка-лин-ка`).
fn split_russian_spelling(word: &str, n: usize) -> Vec<String> {
    if n <= 1 {
        return vec![word.to_string()];
    }
    let chars: Vec<char> = word.chars().collect();
    let v_idx: Vec<usize> = chars
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let mut buf = [0u16; 2];
            let enc = c.encode_utf16(&mut buf);
            enc.len() == 1 && is_cyrillic_vowel(enc[0])
        })
        .map(|(i, _)| i)
        .collect();
    if v_idx.len() < n {
        let mut out = vec![word.to_string()];
        out.extend(std::iter::repeat_n("~".to_string(), n - 1));
        return out;
    }
    let v_idx = &v_idx[..n];
    let mut cuts: Vec<usize> = vec![0];
    for k in 0..v_idx.len() - 1 {
        let mut a = v_idx[k] + 1;
        // keep trailing combining acute with the left vowel
        if a < chars.len() && chars[a] == '\u{301}' {
            a += 1;
        }
        let mut b = v_idx[k + 1];
        // keep leading '+' stress mark with the right vowel
        if b > a && chars[b - 1] == '+' {
            b -= 1;
        }
        // inspect Cyrillic letters in chars[a..b]
        let gap: Vec<(usize, char)> = (a..b)
            .filter_map(|idx| {
                let lc = chars[idx].to_lowercase().next().unwrap_or(chars[idx]);
                is_cyrillic(lc).then_some((idx, lc))
            })
            .collect();
        let cut = if gap.is_empty() {
            b
        } else if gap.len() == 1 {
            // a lone 'й', 'ь' or 'ъ' between vowels (e.g. `майор`, `судья`? no: `район`) stays left if sign/й
            if matches!(gap[0].1, 'й' | 'ь' | 'ъ') { b } else { gap[0].0 }
        } else {
            // count how many letters stay in the left coda:
            // - leading й / ь / ъ
            // - geminate pair (`рус-ский`, `мед-лен-но`)
            // - sonorant (+ optional ь/ъ) followed by another consonant (`Ка-лин-ка`, `боль-шой`)
            let mut left = 0usize;
            while left < gap.len() && matches!(gap[left].1, 'й' | 'ь' | 'ъ') {
                left += 1;
            }
            if left == 0 && gap.len() >= 2 {
                if gap[0].1 == gap[1].1 {
                    left = 1;
                } else if is_ru_sonorant(gap[0].1) {
                    left = 1;
                    if left < gap.len() && matches!(gap[left].1, 'ь' | 'ъ') {
                        left += 1;
                    }
                    // if `сь`/`зь` etc. took all letters (`се-мья`, `дру-зья`), leave consonant+ь together on the right
                    if left >= gap.len() {
                        left = 0;
                    }
                } else if gap.len() >= 3 && matches!(gap[1].1, 'ь' | 'ъ') {
                    // consonant + ь + consonant (`пись-мо`, `день-ги`)
                    left = 2;
                }
            }
            if left >= gap.len() { b } else { gap[left].0 }
        };
        cuts.push(cut);
    }
    cuts.push(chars.len());
    (0..cuts.len() - 1).map(|i| chars[cuts[i]..cuts[i + 1]].iter().collect()).collect()
}

/// Split a written word into n chunks at its vowel groups (approximate hyphenation).
pub fn split_spelling(word: &str, n: usize) -> Vec<String> {
    if has_cyrillic(word) {
        return split_russian_spelling(word, n);
    }
    if n <= 1 {
        return vec![word.to_string()];
    }
    let w: Vec<u16> = word.encode_utf16().collect();
    let lower: Vec<u16> = word.to_lowercase().encode_utf16().collect();
    // /[aeiouy]+/g on the lowercased word
    let mut groups: Vec<[usize; 2]> = Vec::new();
    let mut i = 0;
    while i < lower.len() {
        if is_spell_vowel(lower[i]) {
            let s = i;
            while i < lower.len() && is_spell_vowel(lower[i]) {
                i += 1;
            }
            groups.push([s, i]);
        } else {
            i += 1;
        }
    }
    let len = w.len();
    // /[^aeiouy]e$/i and /[^aeiouyl]le$/i
    let silent_e = len >= 2 && eq_ci(w[len - 1], b'e') && !is_spell_vowel_ci(w[len - 2]);
    let le = len >= 3 && eq_ci(w[len - 1], b'e') && eq_ci(w[len - 2], b'l') && !is_spell_vowel_ci(w[len - 3]) && !eq_ci(w[len - 3], b'l');
    // a final silent e ("like") is not a syllable, but consonant + "le" is ("twin-kle", "lit-tle")
    if groups.len() > 1 && silent_e && !le && groups[groups.len() - 1][0] + 1 == len {
        groups.pop();
    }
    if groups.len() < n {
        // fewer written vowels than sung syllables (e.g. "fire"): hold the word, then continue marks
        let mut out = vec![word.to_string()];
        out.extend(std::iter::repeat_n("~".to_string(), n - 1));
        return out;
    }
    while groups.len() > n {
        let last = groups.pop().unwrap();
        let k = groups.len() - 1;
        groups[k][1] = last[1];
    }
    let mut cuts: Vec<usize> = vec![0];
    for i in 0..groups.len() - 1 {
        let (a, b) = (groups[i][1], groups[i + 1][0]); // consonant letters a..b between vowel groups
        // a digraph stays together on the right ("any-where", "fa-ther"); otherwise one consonant
        // goes right and longer runs split in the middle ("lit-tle", "won-der")
        let gap = b - a;
        let dig = gap >= 2 && {
            let d = [w.get(b - 2).copied(), w.get(b - 1).copied()];
            ["th", "sh", "ch", "ph", "wh", "ck", "gh"].iter().any(|p| {
                let p = p.as_bytes();
                d[0].is_some_and(|c| eq_ci(c, p[0])) && d[1].is_some_and(|c| eq_ci(c, p[1]))
            })
        };
        cuts.push(if dig { b - 2 } else { a + if gap <= 1 { 0 } else { gap / 2 } });
    }
    cuts.push(len);
    (0..cuts.len() - 1).map(|i| slice(&w, cuts[i], cuts[i + 1])).collect()
}

/// Where lyrics get their pronunciations: `EnglishG2P`, or any `Fn(&str, bool) -> Vec<Segment>`.
pub trait WordSource {
    fn word(&self, w: &str, reduce: bool) -> Vec<Segment>;
}

impl<F: Fn(&str, bool) -> Vec<Segment>> WordSource for F {
    fn word(&self, w: &str, reduce: bool) -> Vec<Segment> {
        self(w, reduce)
    }
}

fn is_lyric_punct(c: char) -> bool {
    matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '\n')
}
fn is_word_start(c: char) -> bool {
    // Russian words too, with their stress marks (`+` before the vowel, or a combining acute)
    c.is_ascii_alphabetic() || c == '\'' || is_cyrillic(c) || c == '+' || c == '\u{301}'
}

/// `text.match(/[A-Za-z'][A-Za-z'\-]*|(?<=\s|^)[~-](?=\s|$)|[.,;:!?\n]/g)`
fn lyric_tokens(text: &str) -> Vec<String> {
    let s: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if is_word_start(c) {
            let start = i;
            i += 1;
            while i < s.len() && (is_word_start(s[i]) || s[i] == '-') {
                i += 1;
            }
            out.push(s[start..i].iter().collect());
            continue;
        }
        let melisma = (c == '~' || c == '-') && (i == 0 || js_is_space(s[i - 1])) && (i + 1 == s.len() || js_is_space(s[i + 1]));
        if melisma || is_lyric_punct(c) {
            out.push(c.to_string());
        }
        i += 1;
    }
    out
}

/// Lyrics text → syllables (needs the pronouncing dictionary for real words).
pub fn lyrics_to_syllables(text: &str, g2p: &impl WordSource) -> Vec<Syllable> {
    let mut out: Vec<Syllable> = Vec::new();
    // words, melisma marks and phrase punctuation, in order
    let tokens = lyric_tokens(&text.replace('\u{2019}', "'"));
    for tok in &tokens {
        if tok.chars().count() == 1 && tok.chars().all(is_lyric_punct) {
            if let Some(last) = out.last_mut() {
                last.phrase_end = true;
            }
            continue;
        }
        if tok == "-" || tok == "~" {
            if let Some(prev) = out.last() {
                let s = Syllable { text: "~".into(), onset: vec![], nucleus: prev.nucleus.clone(), coda: vec![], melisma: Some(true), word_end: prev.word_end, phrase_end: false };
                out.push(s);
            }
            continue;
        }
        // typed hyphens split syllables explicitly ("ti-ger"); otherwise the dictionary decides
        let parts: Vec<&str> = tok.split('-').filter(|p| !p.is_empty()).collect();
        let mut syls: Vec<Syllable> = Vec::new();
        for part in &parts {
            let word_segs = g2p.word(&part.to_lowercase(), false);
            let sy = if has_cyrillic(part) { syllabify_russian_segments(&word_segs) } else { syllabify_segments(&word_segs) };
            let spell: Vec<String> = if parts.len() > 1 {
                let mut v = vec![part.to_string()];
                v.extend(std::iter::repeat_n("~".to_string(), sy.len().saturating_sub(1)));
                v
            } else {
                split_spelling(part, sy.len())
            };
            for (i, s) in sy.into_iter().enumerate() {
                // stress marks are for the pronunciation, not the page
                let text = spell.get(i).map_or_else(|| "~".to_string(), |t| t.replace(['+', '\u{301}'], ""));
                syls.push(Syllable { text, onset: s.onset, nucleus: s.nucleus, coda: s.coda, melisma: None, word_end: false, phrase_end: false });
            }
        }
        if let Some(last) = syls.last_mut() {
            last.word_end = true;
        }
        out.append(&mut syls);
    }
    out
}
