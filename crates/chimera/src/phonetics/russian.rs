//! Russian text → segments, by rule. Russian spelling gives the consonants and their softness
//! almost exactly; what it does not give is the stress, and the unstressed vowels depend on it.
//!
//! Stress, in order: a mark in the text (`+` before the vowel, as in `молок+о`, or a combining
//! acute after it, `молоко́`), the letter ё, a lexicon of common words and inflected forms,
//! Russian morphological suffix/ending rules (adjectival `-ый/-ий/-ая/-ое/-ые/-ого/-ых`, verbal
//! `-ешь/-ет/-ем/-ете/-ут/-ют/-овать`, nominal `-ация/-ение/-ость/-тель`), and last a stem fallback.
//!
//! Standard Contemporary Russian (*литературное произношение*) rules modelled here:
//! - Palatalization before `е ё и ю я ь`, plus regressive softness assimilation in dental clusters
//!   (`стена`, `снег`, `жизнь`, `вместе`, `кончик`, `женщина`).
//! - Stressed vowel allophony: `/e/` is close-mid `[e]` before a soft consonant (`день`, `петь`,
//!   `теперь`) and open-mid `[ɛ]` before a hard consonant or word-finally (`привет`, `нет`, `лето`,
//!   `дело`); `/a/` and `/o/` front to `[æ]` and `[œ]` between two soft consonants (`пять`, `мягкий`,
//!   `тётя`).
//! - Two-degree unstressed vowel reduction (*аканье* / *иканье*): 1st-pretonic, word-initial and
//!   open word-final `а/о` open to unrounded `[ɐ]`, medial/closed non-pretonic `а/о` reduce to `[ə]`,
//!   unstressed `е/я` after soft consonants reduce to `[ɪ]` (`[ɨ]` after hard `ж ш ц`). In singing
//!   (`reduce == false`), unstressed `а/о` stay open `[ɐ]` so held notes never sustain a closed `[ə]`.
//! - Final devoicing, regressive obstruent voicing assimilation (`в` transparent before sonorants/vowels),
//!   geminate fusion (`русский`, `медленно`), unaspirated voiceless stops *and* affricates (`п т к ц ч`),
//!   and single-contact apical tap `[ɾ]`-timed `/r/` in clusters/intervocalically vs rolled initial `/r/`.

use super::english::{Phrase, js_is_space};
use super::{Intonation, Segment};
use std::collections::HashMap;
use std::sync::OnceLock;

const ACUTE: char = '\u{301}';

/// Common Russian words and irregular forms with their stressed vowel in capitals.
const STRESS_WORDS: &str = "\
привЕт спасИбо пожАлуйста пожАлста хорошО плОхо здрАствуйте здрАствуй свидАния извинИте простИте \
севОдня сегОдня зАвтра вчерА сейчАс тепЕрь всегдА никогдА иногдА Очень мнОго мАло немнОго тОлько тОже тАкже ужЕ Или Если когдА \
потомУ почемУ зачЕм какОй какАя какОе какИе котОрый котОрая котОрое котОрые Этот Эта Это Эти Этим Этих Этому онА онО онИ менЯ тебЯ евО егО \
моЯ моИ моЁ твоЯ твоИ твоЁ себЯ штОбы чтОбы потОм тогдА тудА сюдА откУда кудА дОма домОй водА водЫ вОду землЯ зЕмлю нЕбо сОнце лунА звездА звЁзды мОре морЯ рекА рЕку \
гОрод городА странА человЕк лЮди женА дЕти ребЁнок мАма пАпа сестрА брАтья друзьЯ дрУга любОвь врЕмя Утро утром вЕчер вечерА недЕля мЕсяц рабОта дЕло словА слОво \
язЫк рУсский рУсская рУсское рУсские россИя россИи москвА москвЕ мУзыка пЕсня пЕсню пЕсни гОлос головА гОлову рукА рУку рУки ногА нОгу нОги глазА сЕрце душА дУшу \
хотЕть хочУ хОчешь хОчет хотИм хотИте хотЯт могУ мОжешь мОжет мОжем мОжете мОгут мОжно нУжно нАдо нельзЯ \
говорИть говорЮ говорИшь говорИт говорИм говорИте говорЯт сказАть скажУ скАжешь скАжет скАжут \
знАть знАю знАешь знАет знАем знАете знАют дУмать дУмаю дУмаешь дУмает дЕлать дЕлаю дЕлаешь дЕлает сдЕлать \
вИдеть вИжу вИдишь вИдит слЫшать слЫшу слЫшишь любИть люблЮ лЮбишь лЮбит лЮбим лЮбят \
идтИ идУ идЁшь идЁт идЁм идУт пойтИ пойдУ жИть живУ живЁшь живЁт живУт бЫть бУду бУдешь бУдет бУдем бУдут былА бЫло бЫли имЕть \
понимАть понимАю понимАешь понимАет красИвый красИвая красИвое красИвые большОй большАя большОе большИе мАленький мАленькая \
нОвый нОвая нОвое нОвые стАрый стАрая хорОший хорОшая хорОшее хорОшие плохОй плохАя дОбрый дОброе дОбрая \
бЕлый бЕлая чЁрный чЁрная крАсный крАсная сИний сИняя зелЁный зелЁная золотОй золотАя молодОй молодАя живОй живАя пустОй простОй \
одИн однА однО однУ двА двЕ трИ четЫре пЯть шЕсть сЕмь вОсемь дЕвять дЕсять двАдцать трИдцать сОрок стО тЫсяча \
пЕрвый вторОй трЕтий никОй никтО ништО ничегО ничевО всевО всегО самА самО сАми сАмый сАмая сАмое \
дорОга дорОгу окнО Окна машИна кнИга кнИгу шкОла учИтель студЕнт вопрОс отвЕт прАвда конЕшно конЕчно навЕрное \
дАже пОсле пЕред мЕжду чЕрез Около прОтив вмЕсте опЯть снОва давнО скОро бЫстро мЕдленно \
давАй давАйте спокОйной нОчи нОчью щАстье счАстье рАдость свобОда рОдина побЕда зимА веснА лЕто Осень вЕтер огОнь дерЕвня \
дЕрево дерЕвья цветЫ цветОк птИца птИцы кОшка собАка лОшадь молокО кОфе чАю чАй бУлок бУлка хлЕб вЫпей \
прекрАсно отлИчно синтезАтор компьЮтер телефОн интернЕт \
калИнка малИнка садУ Ягода Ягоду катЮша Яблони грУши тумАны рекОй бЕрег высОкий крутОй расцветАли поплылИ выходИла \
пОле берЁза берЁзка стоЯла кудрЯвая подмоскОвные вечерА дорогОй дорогАя милАя мИлый роднОй роднАя далекО высокО глубокО \
хОлодно теплО светлО темнО тИхо грОмко весЁлый грУстно однАжды вокрУг внутрИ назАд вперЁд навсегдА никудА нигдЕ \
кОмната квартИра Улица плОщадь магазИн ресторАн вокзАл самолЁт пОезд автОбус дЕньги ценА рублЕй минУта минУту секУнда \
погОда прирОда истОрия культУра наУка рабОтать игрАть читАть писАть слУшать смотрЕть смОтришь смОтрит помогАть \
семьЯ делА какИх такОй такАя такОе такИе другОй другАя другИе кАждый кАждая нАша нАши вАши вАша тебЕ себЕ мнЕ \
моегО твоегО своегО своЯ своИ свОй одногО никогО никакОй зовУт пришЁл ушЁл нашЁл \
бОльше мЕньше лУчше хУже рАньше пОзже дОлго дАльше блИзко далЁкий рЯдом тАм тУт вОт ещЁ покА едвА почтИ совсЕм \
весьмА тОчно прОсто вдрУг срАзу ужАсно чудЕсно красИво спокОйно интерЕсно вАжно трУдно легкО слОжно \
отвечАть спрАшивать начинАть продолжАть открЫть закрЫть купИть продАть гулЯть отдыхАть танцевАть пЕть поЁт поЮ поЁм \
ширОкая ширОкий Южных Южный губЕрний губЕрния толчОк хозЯйства хозЯйство мЯгких мЯгкий лЁгких лЁгкий";

fn stress_dict() -> &'static HashMap<String, usize> {
    static D: OnceLock<HashMap<String, usize>> = OnceLock::new();
    D.get_or_init(|| {
        let mut m = HashMap::new();
        for w in STRESS_WORDS.split_whitespace() {
            let mut n = 0;
            let mut at = None;
            for c in w.chars() {
                if is_vowel(lower(c)) {
                    if c.is_uppercase() {
                        at = Some(n);
                    }
                    n += 1;
                }
            }
            if let Some(at) = at {
                m.insert(w.to_lowercase(), at);
            }
        }
        m
    })
}

/// Parse the tables now (an `Engine` does this when it is created).
pub fn init() {
    stress_dict();
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

pub fn is_cyrillic(c: char) -> bool {
    matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё')
}

/// Does this text have any Russian in it?
pub fn has_cyrillic(text: &str) -> bool {
    text.chars().any(is_cyrillic)
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'а' | 'е' | 'ё' | 'и' | 'о' | 'у' | 'ы' | 'э' | 'ю' | 'я')
}

/// Vowel letters and signs that soften the consonant before them.
fn softens(c: char) -> bool {
    matches!(c, 'е' | 'ё' | 'и' | 'ю' | 'я' | 'ь')
}

/// Words and clusters read differently by Standard Russian orthoepy.
fn respell(w: &str) -> String {
    let whole = match w {
        "что" => "што",
        "чтоб" => "штоб",
        "чтобы" => "штобы",
        "ничто" => "ништо",
        "конечно" => "конешно",
        "скучно" => "скушно",
        "нарочно" => "нарошно",
        "его" => "ево",
        "него" => "нево",
        "чего" => "чево",
        "ничего" => "ничево",
        "всего" => "всево",
        "сегодня" => "севодня",
        "итого" => "итово",
        "бог" => "бох",
        "солнце" => "сонце",
        "сердце" => "серце",
        "сердца" => "серца",
        "здравствуйте" => "здраствуйте",
        "здравствуй" => "здраствуй",
        "пожалуйста" => "пожалста",
        "счастье" => "щастье",
        "счастливый" => "щасливый",
        "счастливая" => "щасливая",
        "счастливо" => "щасливо",
        "чувство" => "чуство",
        "чувства" => "чуства",
        "чувствовать" => "чуствовать",
        "праздник" => "празник",
        "лестница" => "лесница",
        "лёгкий" => "лёхкий",
        "лёгкая" => "лёхкая",
        "лёгкое" => "лёхкое",
        "лёгких" => "лёхких",
        "мягкий" => "мяхкий",
        "мягкая" => "мяхкая",
        "мягкое" => "мяхкое",
        "мягких" => "мяхких",
        _ => "",
    };
    if !whole.is_empty() {
        return whole.to_string();
    }
    let mut s = w.to_string();
    // genitive adjectival/pronominal endings -ого, -его are pronounced with [v]
    let n = s.chars().count();
    if n >= 4 && (s.ends_with("ого") || s.ends_with("его")) && !matches!(w, "много" | "немного" | "дорого" | "строго" | "убого" | "недорого" | "полого" | "лого") {
        let cut = s.char_indices().nth(n - 2).map(|(i, _)| i).unwrap();
        let tail: String = s[cut..].replacen('г', "в", 1);
        s.truncate(cut);
        s.push_str(&tail);
    }
    for (a, b) in [
        ("ться", "ца"),
        ("тся", "ца"),
        ("сч", "щ"),
        ("зч", "щ"),
        ("стн", "сн"),
        ("здн", "зн"),
        ("стл", "сл"),
        ("рдц", "рц"),
        ("лнц", "нц"),
        ("вств", "ств"),
        ("тц", "ц"),
        ("дц", "ц"),
        ("гк", "хк"),
        ("гч", "хч"),
    ] {
        if s.contains(a) {
            s = s.replace(a, b);
        }
    }
    s
}

/// The letters of a word and the index (among its vowels) of a stress marked in the text.
fn letters_and_mark(w: &str) -> (Vec<char>, Option<usize>) {
    let mut out = Vec::new();
    let mut mark = None;
    let mut vowels = 0;
    let mut plus = false;
    for c in w.chars() {
        if c == '+' {
            plus = true;
            continue;
        }
        if c == ACUTE {
            if vowels > 0 {
                mark = Some(vowels - 1);
            }
            continue;
        }
        let c = lower(c);
        if !is_cyrillic(c) {
            continue;
        }
        if is_vowel(c) {
            if plus {
                mark = Some(vowels);
            }
            vowels += 1;
        }
        plus = false;
        out.push(c);
    }
    (out, mark)
}

/// Morphological stress prediction when a word is neither marked nor in [`STRESS_WORDS`].
fn guess_stress(word: &str, letters: &[char], n: usize) -> usize {
    // 1. Productive derivational suffixes with fixed stress
    for (suf, back) in [
        ("ировать", 3usize),
        ("ировали", 4),
        ("ация", 3),
        ("яция", 3),
        ("ации", 3),
        ("яции", 3),
        ("ацию", 3),
        ("яцию", 3),
        ("ение", 3),
        ("ание", 3),
        ("ения", 3),
        ("ания", 3),
        ("ению", 3),
        ("анию", 3),
        ("овать", 1),
        ("евать", 1),
        ("ость", 2),
        ("ости", 3),
        ("остью", 2),
        ("тель", 2),
        ("теля", 3),
        ("тели", 3),
        ("енький", 3),
        ("онький", 3),
        ("енькая", 4),
        ("онькая", 4),
    ] {
        if word.ends_with(suf) && n >= back {
            return n - back;
        }
    }
    // 2. Adjectival endings: `-ый` and `-ий` are never stressed in multi-syllable words!
    if word.ends_with("ый") || word.ends_with("ий") {
        return n - 2;
    }
    if word.ends_with("ой") {
        // `-ой` is stressed in nominative masculine (`большОй`, `молодОй`), stem-stressed in short gen/ins fem if 3+ syl
        return if n == 2 { 1 } else { n - 2 };
    }
    // Two-vowel adjectival inflections (`ширОкая`, `сЕльского`, `нОвые`, `красИвую`): stress sits on the stem before the 2-vowel ending
    for suf in ["ая", "яя", "ое", "ее", "ые", "ие", "ую", "юю", "ого", "его", "ому", "ему", "ыми", "ими"] {
        if word.ends_with(suf) {
            return if n >= 3 { n - 3 } else { 0 };
        }
    }
    // One-vowel adjectival oblique endings (`Южных`, `францУзских`, `нОвым`) and noun plurals (`гУберний`, `бУлок`)
    for suf in ["ых", "их", "ым", "им", "ами", "ями", "ах", "ях", "ов", "ев", "ей"] {
        if word.ends_with(suf) {
            return if suf == "ей" && n == 2 { 1 } else { n - 2 };
        }
    }
    // Unstressed verb person/past endings (`читАешь`, `дУмает`, `знАем`, `пишУт` -> stem)
    for suf in ["ешь", "ет", "ем", "ете", "ут", "ют", "ишь", "ит", "им", "ите", "ал", "ала", "ало", "али", "ил", "ила", "или"] {
        if word.ends_with(suf) {
            return n - 2;
        }
    }
    // Imperative / diminutive `-ей` (`вЫпей` vs `ручЕй`): prefix `вы-` always takes stress!
    if word.starts_with("вы") {
        return 0;
    }
    // Fallback: closed final syllable (not `-ь`) tends toward final stress (`толчОк`, `вокзАл`), open final toward penult (`мАма`, `шкОла`)
    let last = *letters.last().unwrap();
    if !is_vowel(last) && last != 'ь' && last != 'й' { n - 1 } else { n - 2 }
}

/// Which vowel (counting from 0) carries the stress.
fn stress_of(word: &str, letters: &[char], mark: Option<usize>) -> usize {
    let vowels: Vec<char> = letters.iter().copied().filter(|c| is_vowel(*c)).collect();
    let n = vowels.len();
    if n <= 1 {
        return 0;
    }
    if let Some(m) = mark {
        return m.min(n - 1);
    }
    if let Some(i) = vowels.iter().position(|c| *c == 'ё') {
        return i;
    }
    if let Some(i) = stress_dict().get(word) {
        return (*i).min(n - 1);
    }
    guess_stress(word, letters, n)
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Vowel,
    /// an obstruent that has a voiced/voiceless partner
    Paired,
    /// voiceless with no voiced partner in the spelling (х ц ч щ)
    Voiceless,
    Sonorant,
}

struct Ph {
    ipa: &'static str,
    kind: Kind,
    soft: bool,
    geminate: bool,
    /// vowels: the letter's class (a o e i y u), stress, and position
    class: char,
    stressed: bool,
    vowel_index: usize,
}

fn voiced(ipa: &str) -> Option<&'static str> {
    Some(match ipa {
        "p" => "b",
        "f" => "v",
        "k" => "g",
        "t" => "d",
        "ʂ" => "ʐ",
        "s" => "z",
        _ => return None,
    })
}

fn voiceless(ipa: &str) -> Option<&'static str> {
    Some(match ipa {
        "b" => "p",
        "v" => "f",
        "g" => "k",
        "d" => "t",
        "ʐ" => "ʂ",
        "z" => "s",
        _ => return None,
    })
}

fn is_voiced_obstruent(ipa: &str) -> bool {
    voiceless(ipa).is_some()
}

/// Segments of one word. `prefix`: a vowelless proclitic said as part of this word (`в`, `к`, `с`).
/// `sung`: when true (`reduce == false`), vowel durations stay unset for `sing.rs` and unstressed
/// vowels keep open singing diction (`[ɐ]` rather than closed `[ə]`).
fn word_segments(prefix: &str, word: &str, stressed_word: bool, sung: bool) -> Vec<Segment> {
    let (letters0, mark) = letters_and_mark(word);
    if letters0.is_empty() {
        return Vec::new();
    }
    let plain: String = letters0.iter().collect();
    let re = respell(&plain);
    let n_vowels_written = letters0.iter().filter(|c| is_vowel(**c)).count();
    let stress_written = stress_of(if stress_dict().contains_key(&re) { &re } else { &plain }, &letters0, mark);
    let mut letters: Vec<char> = prefix.chars().map(lower).filter(|c| is_cyrillic(*c)).collect();
    letters.extend(re.chars());
    let n_vowels = letters.iter().filter(|c| is_vowel(**c)).count();
    // a respelling can drop a vowel (пожалуйста): keep the stress the same distance from the end
    let stress = if n_vowels < n_vowels_written { stress_written.saturating_sub(n_vowels_written - n_vowels) } else { stress_written };

    let mut raw: Vec<Ph> = Vec::new();
    let mut vi = 0;
    for (i, &c) in letters.iter().enumerate() {
        let prev = if i > 0 { Some(letters[i - 1]) } else { None };
        let next = letters.get(i + 1).copied();
        if is_vowel(c) {
            let iotated = matches!(c, 'е' | 'ё' | 'ю' | 'я') && prev.is_none_or(|p| is_vowel(p) || p == 'ъ' || p == 'ь');
            if iotated || (c == 'и' && prev == Some('ь')) {
                raw.push(Ph { ipa: "j", kind: Kind::Sonorant, soft: true, geminate: false, class: ' ', stressed: false, vowel_index: 0 });
            }
            let class = match c {
                'а' | 'я' => 'a',
                'о' | 'ё' => 'o',
                'э' | 'е' => 'e',
                'и' => 'i',
                'ы' => 'y',
                _ => 'u',
            };
            // и after always-hard ж ш ц retracts to [ɨ]
            let class = if class == 'i' && matches!(prev, Some('ж' | 'ш' | 'ц')) { 'y' } else { class };
            raw.push(Ph { ipa: "", kind: Kind::Vowel, soft: false, geminate: false, class, stressed: vi == stress && stressed_word, vowel_index: vi });
            vi += 1;
            continue;
        }
        let soft_next = next.is_some_and(softens);
        let (ipa, kind, soft) = match c {
            'б' => ("b", Kind::Paired, soft_next),
            'в' => ("v", Kind::Paired, soft_next),
            'г' => ("g", Kind::Paired, soft_next),
            'д' => ("d", Kind::Paired, soft_next),
            'ж' => ("ʐ", Kind::Paired, false),
            'з' => ("z", Kind::Paired, soft_next),
            'й' => ("j", Kind::Sonorant, true),
            'к' => ("k", Kind::Paired, soft_next),
            'л' => ("l", Kind::Sonorant, soft_next),
            'м' => ("m", Kind::Sonorant, soft_next),
            'н' => ("n", Kind::Sonorant, soft_next),
            'п' => ("p", Kind::Paired, soft_next),
            'р' => ("r", Kind::Sonorant, soft_next),
            'с' => ("s", Kind::Paired, soft_next),
            'т' => ("t", Kind::Paired, soft_next),
            'ф' => ("f", Kind::Paired, soft_next),
            'х' => ("x", Kind::Voiceless, soft_next),
            'ц' => ("ts", Kind::Voiceless, false),
            'ч' => ("tʃ", Kind::Voiceless, true),
            'ш' => ("ʂ", Kind::Paired, false),
            'щ' => ("ʃ", Kind::Voiceless, true),
            _ => continue, // ъ ь
        };
        raw.push(Ph { ipa, kind, soft, geminate: false, class: ' ', stressed: false, vowel_index: 0 });
    }

    // ---- voicing: the last obstruent of a cluster decides, and a word ends voiceless
    let mut next_obstruent: Option<bool> = Some(false);
    for i in (0..raw.len()).rev() {
        match raw[i].kind {
            Kind::Vowel | Kind::Sonorant => next_obstruent = None,
            Kind::Voiceless => next_obstruent = Some(false),
            Kind::Paired => {
                match next_obstruent {
                    Some(false) => {
                        if let Some(v) = voiceless(raw[i].ipa) {
                            raw[i].ipa = v;
                        }
                    }
                    Some(true) => {
                        if let Some(v) = voiced(raw[i].ipa) {
                            raw[i].ipa = v;
                        }
                    }
                    None => {}
                }
                // в does not voice what stands before it (свет, твой)
                next_obstruent = if raw[i].ipa == "v" { None } else { Some(is_voiced_obstruent(raw[i].ipa)) };
            }
        }
    }

    // ---- regressive softness assimilation in dental clusters (стена, снег, жизнь, вместе, кончик)
    for i in (0..raw.len().saturating_sub(1)).rev() {
        if raw[i].kind == Kind::Vowel || raw[i].soft {
            continue;
        }
        let nxt = &raw[i + 1];
        if nxt.kind != Kind::Vowel && nxt.soft {
            let dental_before_dental = matches!(raw[i].ipa, "s" | "z") && matches!(nxt.ipa, "t" | "d" | "n" | "l" | "s" | "z" | "v" | "m" | "p" | "b");
            let n_before_palatal = raw[i].ipa == "n" && matches!(nxt.ipa, "tʃ" | "ʃ" | "t" | "d");
            if dental_before_dental || n_before_palatal {
                raw[i].soft = true;
            }
        }
    }

    // ---- fuse adjacent identical consonants into one smooth geminate (русский, медленно, группа)
    let mut ph: Vec<Ph> = Vec::with_capacity(raw.len());
    for p in raw {
        if p.kind != Kind::Vowel
            && let Some(prev) = ph.last_mut()
            && prev.kind != Kind::Vowel
            && prev.ipa == p.ipa
        {
            prev.soft |= p.soft;
            prev.geminate = true;
            continue;
        }
        ph.push(p);
    }

    // ---- emit segments with authentic Russian allophony and syllable-tier timing
    let last_index = ph.len().saturating_sub(1);
    let mut out: Vec<Segment> = Vec::with_capacity(ph.len());
    for i in 0..ph.len() {
        let p = &ph[i];
        if p.kind != Kind::Vowel {
            let ipa = if p.ipa == "l" && !p.soft { "ɫ" } else { p.ipa };
            let mut s = Segment::new(ipa);
            if p.soft && p.ipa != "j" {
                s.palatalized = Some(true);
            }
            // Russian voiceless stops AND affricates (ц, ч) are unaspirated
            if matches!(p.ipa, "p" | "t" | "k" | "ts" | "tʃ") {
                s.aspirated = Some(false);
            }
            let prev_c = i > 0 && ph[i - 1].kind != Kind::Vowel;
            let next_c = i + 1 < ph.len() && ph[i + 1].kind != Kind::Vowel;
            let next_stressed = i + 1 < ph.len() && ph[i + 1].kind == Kind::Vowel && ph[i + 1].stressed;
            if p.ipa == "ʃ" {
                // щ is long soft [ɕː]
                s.dur = Some(if sung { 0.12 } else { 0.135 });
            } else if p.ipa == "r" {
                // Russian /r/: geminate -> 85 ms roll; word-initial before stressed vowel -> 62 ms 2-tap roll;
                // everywhere else (clusters `привет`, intervocalic `хорошо`, coda `мир`) -> 38 ms single apical tap!
                s.dur = Some(if p.geminate {
                    0.085
                } else if i == 0 && next_stressed && !sung {
                    0.062
                } else {
                    0.038
                });
            } else if !sung {
                let base = match ipa {
                    "p" | "t" | "k" => 0.072,
                    "b" | "d" | "g" => 0.062,
                    "ts" | "tʃ" => 0.105,
                    "s" | "ʂ" | "f" | "x" => 0.088,
                    "z" | "ʐ" | "v" => 0.068,
                    "m" | "n" | "l" | "ɫ" => 0.068,
                    "j" => if i == last_index || next_c { 0.050 } else { 0.056 },
                    _ => 0.070,
                };
                let cluster = if prev_c && next_c {
                    0.76
                } else if prev_c || next_c {
                    0.86
                } else {
                    1.0
                };
                let gem = if p.geminate { 1.45 } else { 1.0 };
                s.dur = Some(base * cluster * gem);
            } else if p.geminate {
                s.dur = Some(0.115);
            }
            out.push(s);
            continue;
        }

        let after_soft = i > 0 && ph[i - 1].kind != Kind::Vowel && ph[i - 1].soft;
        let before_soft = i + 1 < ph.len() && ph[i + 1].kind != Kind::Vowel && ph[i + 1].soft;
        let after_cons = i > 0 && ph[i - 1].kind != Kind::Vowel;
        let open_final = i == last_index;
        let word_final_syl = p.vowel_index + 1 == n_vowels;

        let ipa = if p.stressed {
            match p.class {
                // between two soft consonants (`пять`, `опять`, `часть`, `мягкий`), /a/ fronts to [æ]
                'a' => if after_soft && before_soft && !sung { "æ" } else { "a" },
                // between two soft consonants (`тётя`), /o/ fronts toward [œ]
                'o' => if after_soft && before_soft && !sung { "œ" } else { "o" },
                // Russian /e/ is close-mid [e] before a soft consonant (`день`, `петь`, `теперь`)
                // and open-mid [ɛ] before a hard consonant or word-finally (`привет`, `нет`, `лето`, `дело`)
                'e' => if before_soft { "e" } else { "ɛ" },
                'i' => "i",
                'y' => "ɨ",
                _ => "u",
            }
        } else if sung {
            // Russian vocal diction: sustained notes keep open [ɐ] on unstressed а/о (never muffled [ə])
            match p.class {
                'a' | 'o' if after_soft => if open_final { "ɐ" } else { "ɪ" },
                'a' | 'o' => "ɐ",
                'e' => if after_soft || !after_cons { if open_final { "e" } else { "ɪ" } } else { "ɨ" },
                'i' => "ɪ",
                'y' => "ɨ",
                _ => "ʊ",
            }
        } else {
            let pretonic = stressed_word && p.vowel_index + 1 == stress;
            match p.class {
                // open final `-а/-о/-я` (`спасибо`, `калинка`, `музыка`, `сегодня`) is unrounded [ɐ]
                'a' | 'o' if after_soft => if open_final { "ɐ" } else { "ɪ" },
                'a' | 'o' => if pretonic || !after_cons || open_final { "ɐ" } else { "ə" },
                'e' => if after_soft || !after_cons { "ɪ" } else { "ɨ" },
                'i' => "ɪ",
                'y' => "ɨ",
                _ => "ʊ",
            }
        };

        let mut s = Segment::new(ipa);
        s.stress = Some(if p.stressed { 1 } else { 0 });
        if !sung {
            // Three-tier Russian syllable rhythm: stressed >> 1st-pretonic / open-final > other unstressed
            let pretonic = stressed_word && p.vowel_index + 1 == stress;
            let high = matches!(ipa, "i" | "ɨ" | "u" | "ɪ" | "ʊ");
            let d = if p.stressed {
                if word_final_syl {
                    if high { 0.165 } else { 0.188 }
                } else if high {
                    0.138
                } else {
                    0.158
                }
            } else if pretonic || !after_cons {
                if high { 0.095 } else { 0.114 }
            } else if open_final {
                0.096
            } else {
                0.068
            };
            s.dur = Some(d);
        }
        out.push(s);
    }
    out
}

/// Words said without a stress of their own in running speech.
fn is_clitic(w: &str) -> bool {
    matches!(w, "не" | "на" | "по" | "за" | "и" | "а" | "но" | "да" | "о" | "об" | "у" | "из" | "от" | "до" | "для" | "без" | "под" | "над" | "при" | "про" | "же" | "ли" | "бы" | "ни" | "во" | "ко" | "со" | "то")
}

/// A vowelless preposition, said as the start of the next word.
fn is_proclitic(w: &str) -> bool {
    matches!(w, "в" | "к" | "с" | "б" | "ж" | "ль")
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RussianG2P;

impl RussianG2P {
    /// One word → segments with stress. `reduce`: clitics lose their stress and spoken durations/reduction apply;
    /// `reduce == false` (singing) uses open Russian vocal diction without pre-baked vowel durations.
    pub fn word(&self, w: &str, reduce: bool) -> Vec<Segment> {
        let plain: String = letters_and_mark(w).0.iter().collect();
        word_segments("", w, !(reduce && is_clitic(&plain)), !reduce)
    }

    /// Text → phrases. Words in other scripts go to `fallback` (word, reduce).
    pub fn to_phrases(&self, text: &str, fallback: &dyn Fn(&str, bool) -> Vec<Segment>) -> Vec<Phrase> {
        let mut phrases: Vec<Phrase> = Vec::new();
        let mut words: Vec<String> = Vec::new();
        let mut cur = String::new();
        let mut flush = |delim: &str, words: &mut Vec<String>| {
            let mut segs: Vec<Segment> = Vec::new();
            let mut said: Vec<String> = Vec::new();
            let mut prefix = String::new();
            for w in words.iter() {
                let plain: String = letters_and_mark(w).0.iter().collect();
                if has_cyrillic(w) && is_proclitic(&plain) {
                    prefix.push_str(&plain);
                    said.push(w.clone());
                    continue;
                }
                let mut ws = if has_cyrillic(w) { word_segments(&prefix, w, !is_clitic(&plain), false) } else { fallback(&w.to_lowercase(), true) };
                let joined = !prefix.is_empty() && has_cyrillic(w);
                prefix.clear();
                let Some(last) = ws.last_mut() else { continue };
                last.word_end = Some(true);
                segs.append(&mut ws);
                if joined {
                    // the preposition and its word are one spoken word
                    let p = said.pop().unwrap_or_default();
                    said.push(format!("{p} {w}"));
                } else {
                    said.push(w.clone());
                }
            }
            words.clear();
            if segs.is_empty() {
                return;
            }
            let q = delim.contains('?');
            let pause = if delim.contains(['.', '!', '?']) {
                0.35
            } else if !delim.is_empty() {
                0.18
            } else {
                0.3
            };
            phrases.push(Phrase { segments: segs, intonation: if q { Intonation::Question } else { Intonation::Statement }, pause, words: said });
        };
        let mut delim = String::new();
        for c in text.replace('’', "'").chars() {
            let is_punct = matches!(c, '.' | '!' | '?' | ';' | ':' | ',' | '…' | '\n');
            if !is_punct && !delim.is_empty() {
                flush(&delim, &mut words);
                delim.clear();
            }
            if c.is_alphabetic() || c == '+' || c == ACUTE || c == '\'' {
                cur.push(c);
                continue;
            }
            if !cur.is_empty() {
                words.push(core::mem::take(&mut cur));
            }
            if is_punct {
                delim.push(c);
            } else if !js_is_space(c) && c != '-' && c != '—' && c != '–' {
                // anything else separates words
            }
        }
        if !cur.is_empty() {
            words.push(cur);
        }
        flush(&delim, &mut words);
        phrases
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn say_spoken(w: &str) -> String {
        word_segments("", w, true, false)
            .iter()
            .map(|s| format!("{}{}{}", if s.stress == Some(1) { "ˈ" } else { "" }, s.ipa, if s.palatalized == Some(true) { "ʲ" } else { "" }))
            .collect()
    }

    #[test]
    fn common_words_and_allophony() {
        for (w, ipa) in [
            ("привет", "prʲɪvʲˈɛt"),
            ("день", "dʲˈenʲ"),
            ("пять", "pʲˈætʲ"),
            ("спасибо", "spɐsʲˈibɐ"),
            ("молоко", "məɫɐkˈo"),
            ("хорошо", "xərɐʂˈo"),
            ("что", "ʂtˈo"),
            ("его", "jɪvˈo"),
            ("сегодня", "sʲɪvˈodnʲɐ"),
            ("друг", "drˈuk"),
            ("водка", "vˈotkɐ"),
            ("вокзал", "vɐgzˈaɫ"),
            ("ёлка", "jˈoɫkɐ"),
            ("язык", "jɪzˈɨk"),
            ("жизнь", "ʐˈɨzʲnʲ"),
            ("семья", "sʲɪmʲjˈa"),
            ("счастье", "ʃʲˈæsʲtʲjɪ"),
            ("солнце", "sˈontsɨ"),
            ("кр+асного", "krˈasnəvɐ"),
            ("учиться", "ʊtʃʲˈitsɐ"),
            ("объект", "ɐbjˈɛkt"),
            ("Москва", "mɐskvˈa"),
            ("музыка", "mˈuzɨkɐ"),
            ("голова", "ɡəɫɐvˈa"),
            ("русский", "rˈuskʲɪj"),
        ] {
            assert_eq!(say_spoken(w).replace('g', "ɡ"), ipa.replace('g', "ɡ"), "{w}");
        }
    }

    #[test]
    fn morphological_stress_avoids_ending_traps() {
        assert_eq!(say_spoken("широкая"), "ʂɨrˈokəjɐ");
        assert_eq!(say_spoken("электрификация"), "ɪlʲɪktrʲɪfʲɪkˈatsɨjɐ");
        assert_eq!(say_spoken("умный"), "ˈumnɨj");
        assert_eq!(say_spoken("южных"), "jˈuʐnɨx");
        assert_eq!(say_spoken("сельского"), "sʲˈɛlʲskəvɐ");
    }

    #[test]
    fn a_marked_stress_wins() {
        assert_eq!(say_spoken("зам+ок"), "zɐmˈok");
        assert_eq!(say_spoken("з+амок"), "zˈamək");
        assert_eq!(say_spoken("замо\u{301}к"), "zɐmˈok");
    }

    #[test]
    fn phrases_join_prepositions_and_fall_back_for_latin_words() {
        let en = |w: &str, _: bool| vec![Segment::new("x"), Segment::new(&format!("<{w}>"))];
        let p = RussianG2P.to_phrases("Я в доме, а ты? Hello мир.", &en);
        assert_eq!(p.len(), 3);
        let ipa = |p: &Phrase| p.segments.iter().map(|s| s.ipa.as_str()).collect::<String>();
        assert_eq!(ipa(&p[0]), "javdomɪ");
        assert_eq!(p[0].words, ["Я", "в доме"]);
        assert_eq!(p[1].intonation, Intonation::Question);
        assert_eq!(ipa(&p[2]), "x<hello>mir");
        assert_eq!(p[0].segments.iter().filter(|s| s.word_end == Some(true)).count(), p[0].words.len());
    }
}
