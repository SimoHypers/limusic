//! Romanized lyrics (issue #202): a Latin-script reading under each line of a song in a script the
//! listener may not read.
//!
//! Apple Music's own pronunciation lines come first. Boidu serves Apple's TTML, which carries a
//! human-written, word-timed `<transliteration>`; `lyrics::parse_ttml_aaml` keeps it, and a song
//! that has it is left alone here. Everything else goes through a local engine, offline and
//! deterministic:
//!
//! - **Japanese:** lindera + IPADIC. A dictionary is the only way to read kanji, and the part of
//!   speech is what turns the particles は/へ/を into "wa"/"e"/"o" and decides where words break.
//!   Spelled the way Apple spells it: Hepburn without macrons (`shou`, `ijou`).
//! - **Korean:** Revised Romanization, with the sound changes at syllable boundaries applied, so
//!   `좋아해` reads "joahae" and `한국말` "hangungmal" rather than letter by letter.
//! - **Chinese:** Hanyu Pinyin with tone marks, one syllable per character.
//! - **Cyrillic, Greek, Georgian, Armenian:** `any_ascii`, whose tables are good for alphabets.
//!
//! Deliberately not covered: the Indic scripts (`any_ascii` drops vowels that are pronounced,
//! "zindagi" comes out "imdgi"), Thai (no spaces between words, so one unreadable run per line) and
//! the abjads (Arabic, Hebrew), where a transliteration without the unwritten vowels reads nothing
//! like the song. A wrong reading is worse than none.
//!
//! Runs on every `get_lyrics` answer and is never cached, so a better engine applies to songs that
//! were cached before it.

use std::sync::LazyLock;

use lindera::{DictionaryKind, DictionaryLoader, Mode, Tokenizer};
use pinyin::ToPinyin;
use wana_kana::ConvertJapanese;

use crate::lyrics::{LyricWord, Lyrics};

/// Fill `romanized` on every line that needs one. Lines already in Latin script get nothing, so an
/// English chorus in a K-pop song is not printed twice.
pub fn fill(lyrics: &mut Lyrics) {
    // Apple's reading or ours, never both in one song: they spell differently, and the mix reads
    // as a bug. Apple leaves its Latin lines out, which is also what this does.
    if !lyrics.lines.iter().any(|l| l.romanized.is_some()) {
        let texts: Vec<&str> = lyrics.lines.iter().map(|l| l.text.as_str()).collect();
        let readings = readings(&texts);
        for (line, r) in lyrics.lines.iter_mut().zip(readings) {
            line.romanized = r;
        }
    }
    time_readings(lyrics);
}

/// A word-synced line's reading, timed by its words so it sweeps with them the way Apple's does.
/// Each word's own reading (the lyric editor's) says where that word ends in the line's. A reading
/// that is not the engine's, spelled differently, stays untimed.
fn time_readings(lyrics: &mut Lyrics) {
    let todo: Vec<usize> = (0..lyrics.lines.len())
        .filter(|&i| {
            let l = &lyrics.lines[i];
            l.romanized.is_some()
                && l.romanized_words.is_none()
                && l.words.as_ref().is_some_and(|w| !w.is_empty())
        })
        .collect();
    if todo.is_empty() {
        return;
    }
    let texts: Vec<Vec<String>> = todo
        .iter()
        .map(|&i| lyrics.lines[i].words.iter().flatten().map(|w| w.text.clone()).collect())
        .collect();
    // The whole song decides Japanese, as for the line readings, not just the timed lines.
    let all: String = lyrics.lines.iter().map(|l| l.text.as_str()).collect();
    for (i, parts) in todo.into_iter().zip(word_readings_in(&texts, is_japanese(&all))) {
        let line = &mut lyrics.lines[i];
        line.romanized_words = time_reading(
            line.romanized.as_deref().unwrap_or_default(),
            line.words.as_deref().unwrap_or_default(),
            &parts,
        );
    }
}

/// Cuts `reading` at each word's end, matching letters only (spaces and punctuation fall to the
/// word before). `None` when the words don't spell the reading.
fn time_reading(
    reading: &str,
    words: &[LyricWord],
    parts: &[Option<String>],
) -> Option<Vec<LyricWord>> {
    let r: Vec<char> = reading.chars().collect();
    let skip = |mut at: usize| {
        while at < r.len() && !r[at].is_alphanumeric() {
            at += 1;
        }
        at
    };
    let mut at = 0;
    let mut out = Vec::new();
    for (w, part) in words.iter().zip(parts) {
        let from = at;
        for c in part.as_deref().unwrap_or(w.text.trim()).chars().filter(|c| c.is_alphanumeric()) {
            at = skip(at);
            if at == r.len() || !r[at].to_lowercase().eq(c.to_lowercase()) {
                return None;
            }
            at += 1;
        }
        at = skip(at);
        if at > from {
            out.push(LyricWord {
                text: r[from..at].iter().collect(),
                start_ms: w.start_ms,
                end_ms: w.end_ms,
                bg: w.bg,
            });
        }
    }
    (at == r.len() && !out.is_empty()).then_some(out)
}

/// Each line's reading, `None` where it needs none. The whole song decides Japanese or Chinese, so
/// the lines go in together.
fn readings<S: AsRef<str>>(texts: &[S]) -> Vec<Option<String>> {
    let all: String = texts.iter().map(AsRef::as_ref).collect();
    let song_japanese = is_japanese(&all);
    texts
        .iter()
        .map(|t| {
            let t = t.as_ref();
            // A Japanese line in a mostly Korean song (a J-version verse) is still Japanese, and a
            // Chinese line with one stylised の is still Chinese.
            let r = romanize_line(t, song_japanese || is_japanese(t));
            (r != t).then_some(r)
        })
        .collect()
}

fn is_kana(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}')
}

/// Kanji alone cannot tell Japanese from Chinese, kana can, so the whole song decides and a
/// kanji-only line in a Japanese song stays Japanese. Japanese prose runs well over half kana, so a
/// tenth is a safe floor, and it keeps a Chinese song with one stylised の Chinese.
fn is_japanese(text: &str) -> bool {
    let kana = text.chars().filter(|&c| is_kana(c)).count();
    let han = text.chars().filter(|&c| class(c) == Class::Cjk && !is_kana(c)).count();
    kana > 0 && kana * 10 >= han
}

#[derive(Clone, Copy, PartialEq)]
enum Class {
    /// Latin, digits, spaces, anything this does not touch.
    Keep,
    /// Kana and Han. Japanese or Chinese depending on the song.
    Cjk,
    Hangul,
    /// Scripts `any_ascii` transliterates well.
    Alphabet,
    /// CJK and full-width punctuation: `、` → `,`.
    Punct,
}

fn class(c: char) -> Class {
    match c as u32 {
        0x3040..=0x30FA | 0x30FC..=0x30FF | 0x31F0..=0x31FF | 0x3005..=0x3006 => Class::Cjk,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => Class::Cjk,
        0xAC00..=0xD7A3 => Class::Hangul,
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Class::Alphabet, // Greek
        0x0400..=0x052F => Class::Alphabet,                   // Cyrillic
        0x0530..=0x058F => Class::Alphabet,                   // Armenian
        0x10A0..=0x10FF => Class::Alphabet,                   // Georgian
        0x3000..=0x303F | 0x30FB | 0xFF00..=0xFFEF => Class::Punct,
        _ => Class::Keep,
    }
}

fn romanize_line(line: &str, japanese: bool) -> String {
    let mut out = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let cls = class(chars[i]);
        let start = i;
        while i < chars.len() && class(chars[i]) == cls {
            i += 1;
        }
        let run: String = chars[start..i].iter().collect();
        let piece = match cls {
            Class::Keep => run,
            Class::Punct | Class::Alphabet => any_ascii::any_ascii(&run),
            Class::Hangul => korean(&run),
            Class::Cjk if japanese => japanese_run(&run),
            Class::Cjk => chinese(&run),
        };
        // A script change with no space in the source ("Baby愛してる", "你好，世界") still needs one
        // in Latin, or the words run together.
        let joins = out.chars().last().is_some_and(|c| c.is_alphanumeric() || ",.!?;:".contains(c))
            && piece.chars().next().is_some_and(char::is_alphanumeric);
        if joins {
            out.push(' ');
        }
        out.push_str(&piece);
    }
    out
}

// --- Chinese ------------------------------------------------------------------------------------

/// ponytail: one reading per character, the most common one. A polyphone inside a word takes the
/// wrong reading (了解 "le jiě", not "liǎo jiě"; 音乐 "yīn lè"). A phrase dictionary fixes that;
/// the crates that have one were either buggy on single characters or 20 MB.
fn chinese(run: &str) -> String {
    let syllables: Vec<String> = run
        .chars()
        .map(|c| c.to_pinyin().map_or_else(|| c.to_string(), |p| p.with_tone().to_owned()))
        .collect();
    syllables.join(" ")
}

// --- Japanese -----------------------------------------------------------------------------------

/// IPADIC, embedded. The dictionary is ~45 MB of read-only data in the binary (~6 MB compressed in
/// the installers), paged in only when a Japanese song is shown.
static TOKENIZER: LazyLock<Option<Tokenizer>> = LazyLock::new(|| {
    DictionaryLoader::load_dictionary_from_kind(DictionaryKind::IPADIC)
        .map(|d| Tokenizer::new(d, None, Mode::Normal))
        .inspect_err(|e| tracing::warn!(error = %e, "romanize: IPADIC failed to load"))
        .ok()
});

struct Morph {
    surface: String,
    pos: String,
    sub: String,
    base: String,
    /// Katakana, the particles は/へ/を as they are said. Empty: read with the morph before it.
    reading: String,
    /// Written onto the word before it, the way Apple writes them: an inflection stays on its stem
    /// ("yokatta", "aishiteru"), the copula and particles stand alone ("yume nara ba", "koto o").
    glue: bool,
}

/// A run of kana and kanji in IPADIC's morphemes, read. `None` without the dictionary.
fn morphs(run: &str) -> Option<Vec<Morph>> {
    let mut tokens = TOKENIZER.as_ref()?.tokenize(run).ok()?;
    let mut morphs: Vec<Morph> = tokens
        .iter_mut()
        .map(|t| {
            let surface = t.text.to_owned();
            let d = t.get_details().unwrap_or_default();
            let field = |i: usize| d.get(i).filter(|s| **s != "*").map(|s| s.to_string());
            Morph {
                // An unknown word has no reading. Kana reads as itself; an unknown kanji stays.
                reading: field(7).unwrap_or_else(|| surface.clone()),
                pos: field(0).unwrap_or_default(),
                sub: field(1).unwrap_or_default(),
                base: field(6).unwrap_or_default(),
                surface,
                glue: false,
            }
        })
        .collect();

    // IPADIC reads a number and its counter apart: 一人 "ichi nin". These two are everywhere in
    // lyrics and are never read that way.
    for i in 1..morphs.len() {
        if morphs[i].surface == "人" && morphs[i - 1].sub == "数" {
            let fused = match morphs[i - 1].surface.as_str() {
                "一" => "ヒトリ",
                "二" => "フタリ",
                _ => continue,
            };
            morphs[i - 1].reading = fused.into();
            morphs[i].reading.clear();
        }
    }

    let mut prev_copula = false;
    for m in &mut morphs {
        if m.reading.is_empty() {
            continue;
        }
        match (m.pos.as_str(), m.surface.as_str()) {
            ("助詞", "は") => m.reading = "ワ".into(),
            ("助詞", "へ") => m.reading = "エ".into(),
            ("助詞", "を") => m.reading = "オ".into(),
            _ => {}
        }
        let copula = m.pos == "助動詞" && matches!(m.base.as_str(), "だ" | "です");
        m.glue = (m.pos == "助動詞" && (!copula || prev_copula))
            || (m.sub == "接続助詞" && matches!(m.surface.as_str(), "て" | "で"))
            || (m.sub == "非自立"
                && matches!(m.base.as_str(), "てる" | "でる" | "とく" | "ちゃう" | "じゃう"))
            || m.sub == "接尾";
        prev_copula = copula;
    }
    Some(morphs)
}

fn japanese_run(run: &str) -> String {
    let Some(morphs) = morphs(run) else {
        return run.to_owned();
    };
    let mut words: Vec<String> = Vec::new();
    for m in morphs.iter().filter(|m| !m.reading.is_empty()) {
        match words.last_mut() {
            Some(w) if m.glue => w.push_str(&m.reading),
            _ => words.push(m.reading.clone()),
        }
    }
    // Converted per word, not per morpheme, so a trailing っ doubles the next consonant.
    words.iter().map(|w| w.to_romaji()).collect::<Vec<_>>().join(" ")
}

// --- The lyric editor's words (#676) ------------------------------------------------------------

/// Each word's reading, one list per line as the lyric editor split it, `None` where a word needs
/// none. A Japanese line is read whole and each word gets its share of the reading: a kanji reads
/// by its neighbours, 並 alone is "nami" and in 並べた "nara".
pub fn word_readings(lines: &[Vec<String>]) -> Vec<Vec<Option<String>>> {
    let all: String = lines.iter().flatten().map(String::as_str).collect();
    word_readings_in(lines, is_japanese(&all))
}

fn word_readings_in(lines: &[Vec<String>], song_japanese: bool) -> Vec<Vec<Option<String>>> {
    lines
        .iter()
        .map(|words| {
            let pieces = if song_japanese || is_japanese(&words.concat()) {
                japanese_pieces(words)
            } else {
                words.iter().map(|w| vec![Piece::Text(romanize_line(w, false))]).collect()
            };
            // The kana a word opens with, for the っ that ends the word before it.
            let opens: Vec<Option<String>> = pieces
                .iter()
                .map(|p| match p.first() {
                    Some(Piece::Kana(k, _)) => Some(k.clone()),
                    _ => None,
                })
                .collect();
            pieces
                .into_iter()
                .zip(words)
                .enumerate()
                .map(|(k, (p, w))| {
                    let next = opens.get(k + 1).and_then(Option::as_deref);
                    join(p, next).filter(|r| r != w.trim())
                })
                .collect()
        })
        .collect()
}

enum Piece {
    /// Katakana, and whether it joins the kana before it.
    Kana(String, bool),
    /// Already Latin.
    Text(String),
}

/// A Japanese line's reading, cut at the editor's word edges.
fn japanese_pieces(words: &[String]) -> Vec<Vec<Piece>> {
    let owner: Vec<usize> = words
        .iter()
        .enumerate()
        .flat_map(|(k, w)| std::iter::repeat_n(k, w.chars().count()))
        .collect();
    let chars: Vec<char> = words.concat().chars().collect();
    let mut out: Vec<Vec<Piece>> = words.iter().map(|_| Vec::new()).collect();
    // Calls `f(word, from, to)` for each word's share of `from..to`.
    let split = |from: usize, to: usize, f: &mut dyn FnMut(usize, usize, usize)| {
        let mut a = from;
        while a < to {
            let b = (a..to).find(|&i| owner[i] != owner[a]).unwrap_or(to);
            f(owner[a], a, b);
            a = b;
        }
    };
    let text = |out: &mut Vec<Vec<Piece>>, from: usize, to: usize| {
        split(from, to, &mut |k, a, b| {
            let part: String = chars[a..b].iter().collect();
            out[k].push(Piece::Text(romanize_line(&part, true)));
        });
    };
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        let cls = class(chars[i]);
        while i < chars.len() && class(chars[i]) == cls {
            i += 1;
        }
        let run: String = chars[start..i].iter().collect();
        let morphs = (cls == Class::Cjk).then(|| morphs(&run)).flatten();
        // Morphemes cover the run exactly; anything else is read as the line reader would.
        let Some(morphs) = morphs
            .filter(|m| m.iter().map(|m| m.surface.chars().count()).sum::<usize>() == i - start)
        else {
            text(&mut out, start, i);
            continue;
        };
        let mut at = start;
        for m in morphs {
            let surface: Vec<char> = m.surface.chars().collect();
            let end = at + surface.len();
            if !m.reading.is_empty() {
                let reading: Vec<char> = m.reading.chars().collect();
                // Across an edge: each char's own reading, or all of it on the first char.
                let per_char = align(&surface, &reading).unwrap_or_else(|| {
                    let mut v = vec![String::new(); surface.len()];
                    v[0] = m.reading.clone();
                    v
                });
                let mut glue = m.glue;
                split(at, end, &mut |k, a, b| {
                    let kana: String = per_char[a - at..b - at].concat();
                    if !kana.is_empty() {
                        out[k].push(Piece::Kana(kana, glue));
                    }
                    glue = false;
                });
            }
            at = end;
        }
    }
    out
}

/// What each char of a morpheme reads as, its kana as anchors: 流れ込ん read ナガレコン gives
/// 流 ナガ, れ レ, 込 コ, ん ン. A run of kanji reads as a whole, on its first char. `None` when the
/// kana don't line up with the reading (は said ワ).
fn align(surface: &[char], reading: &[char]) -> Option<Vec<String>> {
    fn kata(c: char) -> char {
        match c {
            '\u{3041}'..='\u{3096}' => char::from_u32(c as u32 + 0x60).unwrap_or(c),
            _ => c,
        }
    }
    fn go(s: &[char], r: &[char], out: &mut Vec<String>) -> bool {
        let Some(&c) = s.first() else {
            return r.is_empty();
        };
        if is_kana(c) {
            if r.first() != Some(&kata(c)) {
                return false;
            }
            out.push(r[0].to_string());
            if go(&s[1..], &r[1..], out) {
                return true;
            }
            out.pop();
            return false;
        }
        let n = s.iter().take_while(|c| !is_kana(**c)).count();
        for len in 1..=r.len() {
            out.push(r[..len].iter().collect());
            out.extend(std::iter::repeat_n(String::new(), n - 1));
            if go(&s[n..], &r[len..], out) {
                return true;
            }
            out.truncate(out.len() - n);
        }
        false
    }
    let mut out = Vec::new();
    go(surface, reading, &mut out).then_some(out)
}

/// One word's pieces as Latin: kana glued the way `japanese_run` glues it, punctuation against the
/// word before. A っ at the end doubles the consonant `next` opens with: 待っ|て reads "mat" "te".
/// `None` when nothing is left.
fn join(pieces: Vec<Piece>, next: Option<&str>) -> Option<String> {
    let mut words: Vec<(bool, String)> = Vec::new();
    for p in pieces {
        match p {
            Piece::Kana(k, glue) => match words.last_mut() {
                Some((true, w)) if glue => w.push_str(&k),
                _ => words.push((true, k)),
            },
            Piece::Text(t) if !t.trim().is_empty() => words.push((false, t.trim().to_owned())),
            Piece::Text(_) => {}
        }
    }
    let last = words.len().saturating_sub(1);
    let mut out = String::new();
    for (i, (kana, w)) in words.into_iter().enumerate() {
        let w = match next {
            Some(n) if kana && i == last && w.ends_with('ッ') => {
                let (both, after) = (format!("{w}{n}").to_romaji(), n.to_romaji());
                both.strip_suffix(after.as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| w.to_romaji())
            }
            _ if kana => w.to_romaji(),
            _ => w,
        };
        if !out.is_empty() && w.chars().next().is_some_and(char::is_alphanumeric) {
            out.push(' ');
        }
        out.push_str(&w);
    }
    (!out.is_empty()).then_some(out)
}

// --- Korean -------------------------------------------------------------------------------------

const INITIAL: [&str; 19] = [
    "g", "kk", "n", "d", "tt", "r", "m", "b", "pp", "s", "ss", "", "j", "jj", "ch", "k", "t", "p",
    "h",
];
const MEDIAL: [&str; 21] = [
    "a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa", "wae", "oe", "yo", "u", "wo", "we",
    "wi", "yu", "eu", "ui", "i",
];

// Final consonant indices (0 = none), in Unicode's order.
const F_G: usize = 1;
const F_NH: usize = 6;
const F_D: usize = 7;
const F_LG: usize = 9;
const F_LH: usize = 15;
const F_B: usize = 17;
const F_J: usize = 22;
const F_T: usize = 25;
const F_H: usize = 27;
// Initial indices.
const I_G: usize = 0;
const I_N: usize = 2;
const I_D: usize = 3;
const I_R: usize = 5;
const I_M: usize = 6;
const I_SILENT: usize = 11;
const I_J: usize = 12;
const I_H: usize = 18;

/// What a final sounds like before a consonant or at the end of a word. `ㄺ` before `ㄱ` is the
/// one final whose sound depends on what follows; it is handled in `link`.
const CODA: [&str; 28] = [
    "", "k", "k", "k", "n", "n", "n", "t", "l", "k", "m", "l", "l", "l", "p", "l", "m", "p", "p",
    "t", "t", "ng", "t", "t", "k", "t", "p", "t",
];

/// A final carried over onto a following vowel: (what stays, what moves). `ㄶ`/`ㅀ`/`ㅎ` lose the
/// `ㅎ` there (않아 "ana", 좋아 "joa").
const LIAISON: [(&str, &str); 28] = [
    ("", ""),
    ("", "g"),
    ("", "kk"),
    ("k", "s"),
    ("", "n"),
    ("n", "j"),
    ("", "n"),
    ("", "d"),
    ("", "r"),
    ("l", "g"),
    ("l", "m"),
    ("l", "b"),
    ("l", "s"),
    ("l", "t"),
    ("l", "p"),
    ("", "r"),
    ("", "m"),
    ("", "b"),
    ("p", "s"),
    ("", "s"),
    ("", "ss"),
    ("ng", ""),
    ("", "j"),
    ("", "ch"),
    ("", "k"),
    ("", "t"),
    ("", "p"),
    ("", ""),
];

/// The sounds at one syllable boundary: the romanized final of the first syllable and the initial
/// of the second, after liaison, aspiration, nasalisation and the ㄹ rules. Tensing is not written
/// in Revised Romanization, so it is not modelled.
fn link(fin: usize, ini: usize, next_medial: usize) -> (&'static str, &'static str) {
    if ini == I_SILENT {
        // ㄷ/ㅌ before 이 palatalise: 같이 "gachi", 굳이 "guji".
        return match fin {
            F_D if next_medial == 20 => ("", "j"),
            F_T if next_medial == 20 => ("", "ch"),
            _ => LIAISON[fin],
        };
    }
    // ㅎ next to ㄱ/ㄷ/ㅈ/ㅂ aspirates it: 이렇게 "ireoke", 막혀 "makyeo".
    match (fin, ini) {
        (F_H | F_NH | F_LH, I_G | I_D | I_J) => {
            let rest = match fin {
                F_NH => "n",
                F_LH => "l",
                _ => "",
            };
            let aspirated = match ini {
                I_G => "k",
                I_D => "t",
                _ => "ch",
            };
            return (rest, aspirated);
        }
        (F_H, I_N) => return ("n", "n"),
        (F_G | F_LG, I_H) => return ("", "k"),
        (F_D, I_H) => return ("", "t"),
        (F_B, I_H) => return ("", "p"),
        (F_J, I_H) => return ("", "ch"),
        _ => {}
    }
    let coda = if fin == F_LG && ini == I_G { "l" } else { CODA[fin] };
    match (coda, ini) {
        ("k", I_N | I_M) => ("ng", INITIAL[ini]),
        ("t", I_N | I_M) => ("n", INITIAL[ini]),
        ("p", I_N | I_M) => ("m", INITIAL[ini]),
        ("n" | "l", I_R) | ("l", I_N) => ("l", "l"),
        ("m" | "ng", I_R) => (coda, "n"),
        ("k", I_R) => ("ng", "n"),
        ("p", I_R) => ("m", "n"),
        ("t", I_R) => ("n", "n"),
        _ => (coda, INITIAL[ini]),
    }
}

/// One run of Hangul syllables, which is one word: the source puts spaces between words, and
/// Revised Romanization applies the sound changes inside a word only.
fn korean(run: &str) -> String {
    let syl: Vec<(usize, usize, usize)> = run
        .chars()
        .map(|c| {
            let s = c as usize - 0xAC00;
            (s / 588, (s % 588) / 28, s % 28)
        })
        .collect();
    let mut out = String::new();
    let mut onset = INITIAL[syl[0].0];
    for (i, &(_, medial, fin)) in syl.iter().enumerate() {
        out.push_str(onset);
        out.push_str(MEDIAL[medial]);
        let (coda, next) = match syl.get(i + 1) {
            Some(&(ini, next_medial, _)) => link(fin, ini, next_medial),
            None => (CODA[fin], ""),
        };
        out.push_str(coda);
        onset = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn korean_applies_the_sound_changes() {
        for (hangul, want) in [
            ("좋아해", "joahae"),
            ("같이", "gachi"),
            ("있고", "itgo"),
            ("싶어", "sipeo"),
            ("않아", "ana"),
            ("읽다", "ikda"),
            ("숨이", "sumi"),
            ("막혀", "makyeo"),
            ("이렇게", "ireoke"),
            ("한국말", "hangungmal"),
            ("몰라", "molla"),
            ("신라", "silla"),
            ("입니다", "imnida"),
            ("있는", "inneun"),
            ("없어", "eopseo"),
            ("괜찮아", "gwaenchana"),
            ("꽃이", "kkochi"),
            ("밖에", "bakke"),
            ("앉아", "anja"),
            ("닭", "dak"),
            ("사랑해요", "saranghaeyo"),
            ("웃었지만", "useotjiman"),
            ("라면", "ramyeon"),
            ("심리", "simni"),
        ] {
            assert_eq!(korean(hangul), want, "{hangul}");
        }
    }

    #[test]
    fn korean_line_keeps_the_english() {
        assert_eq!(romanize_line("말해 줘 Say it back", false), "malhae jwo Say it back");
        assert_eq!(romanize_line("Walk in this 미로", false), "Walk in this miro");
    }

    #[test]
    fn japanese_reads_particles_and_breaks_words() {
        for (ja, want) in [
            ("夢ならばどれほどよかったでしょう", "yume nara ba dorehodo yokatta deshou"),
            ("君の名は", "kimi no na wa"),
            ("未だにあなたのことを夢にみる", "imadani anata no koto o yume ni miru"),
            ("一人で歩いた道", "hitori de aruita michi"),
            ("言えずに隠してた", "iezu ni kakushiteta"),
            ("明日へ", "ashita e"),
        ] {
            assert_eq!(romanize_line(ja, true), want, "{ja}");
        }
    }

    #[test]
    fn mixed_scripts_get_a_space_at_the_seam() {
        assert_eq!(romanize_line("Baby愛してる", true), "Baby aishiteru");
        assert_eq!(romanize_line("你好，世界", false), "nǐ hǎo, shì jiè");
        assert_eq!(romanize_line("僕は、君を", true), "boku wa, kimi o");
    }

    #[test]
    fn the_whole_song_decides_japanese_or_chinese() {
        assert!(is_japanese("東京の空\n東京"));
        assert!(!is_japanese("还记得你说家是唯一的城堡の"));
        assert!(!is_japanese("보고 싶다 Baby"));
        // One stylised の leaves a Chinese line Chinese; a J-verse in a Korean song is Japanese.
        let zh = readings(&["还记得你说家是唯一的城堡の"]);
        assert!(zh[0].as_deref().unwrap().starts_with("hái jì"), "{zh:?}");
        let ko = readings(&["보고 싶다", "보고 싶어", "東京の空"]);
        assert_eq!(ko[2].as_deref(), Some("toukyou no sora"));
    }

    #[test]
    fn editor_words_share_the_line_reading() {
        let words = |ws: &[&str]| ws.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        let r = word_readings(&[
            words(&["月明かり ", "流れ", "込", "んで ", "影 ", "を ", "並", "べた"]),
            words(&["Baby ", "愛してる、", "待っ", "て", "待", "っ", "て"]),
        ]);
        let got = |l: &Vec<Option<String>>| {
            l.iter().map(|w| w.clone().unwrap_or("-".into())).collect::<Vec<_>>()
        };
        assert_eq!(got(&r[0]), ["tsukiakari", "nagare", "ko", "nde", "kage", "o", "nara", "beta"]);
        assert_eq!(got(&r[1]), ["-", "aishiteru,", "mat", "te", "ma", "t", "te"]);
        assert_eq!(
            align(&['流', 'れ', '込', 'ん'], &['ナ', 'ガ', 'レ', 'コ', 'ン']).unwrap(),
            ["ナガ", "レ", "コ", "ン"]
        );
        assert_eq!(align(&['は'], &['ワ']), None);
        let ko = word_readings(&[words(&["보고 ", "싶다 ", "Baby"])]);
        assert_eq!(got(&ko[0]), ["bogo", "sipda", "-"]);
    }

    #[test]
    fn fill_times_a_word_synced_reading_by_its_words() {
        use crate::lyrics::LyricLine;
        let ws = ["月明かり ", "流れ", "込", "んで ", "影 ", "を ", "並", "べた"];
        let words: Vec<LyricWord> = ws
            .iter()
            .enumerate()
            .map(|(i, w)| LyricWord {
                text: w.to_string(),
                start_ms: i as u64 * 100,
                end_ms: i as u64 * 100 + 100,
                bg: false,
            })
            .collect();
        let line = LyricLine {
            time_ms: Some(0),
            text: ws.concat(),
            words: Some(words),
            ..Default::default()
        };
        let mut l = Lyrics { lines: vec![line.clone()], synced: true, ..Default::default() };
        fill(&mut l);
        let timed = l.lines[0].romanized_words.as_ref().unwrap();
        let texts: Vec<&str> = timed.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["tsukiakari ", "nagare", "ko", "nde ", "kage ", "o ", "nara", "beta"]);
        assert_eq!((timed[2].start_ms, timed[2].end_ms), (200, 300));

        // A stored reading spelled another way stays as it was, untimed.
        let mut l = Lyrics {
            lines: vec![LyricLine {
                romanized: Some("tsukiakari nagarekonde kage wo narabeta".into()),
                ..line
            }],
            synced: true,
            ..Default::default()
        };
        fill(&mut l);
        assert_eq!(l.lines[0].romanized_words, None);
    }

    #[test]
    fn fill_skips_latin_lines_and_defers_to_apple() {
        use crate::lyrics::LyricLine;
        let mut l = Lyrics {
            source: "t".into(),
            lines: vec![
                LyricLine::simple(None, "Я тебя люблю".into()),
                LyricLine::simple(None, "Hey".into()),
            ],
            ..Default::default()
        };
        fill(&mut l);
        assert_eq!(l.lines[0].romanized.as_deref(), Some("Ya tebya lyublyu"));
        assert_eq!(l.lines[1].romanized, None);

        l.lines[0].romanized = Some("from apple".into());
        l.lines.push(LyricLine::simple(None, "Привет".into()));
        fill(&mut l);
        assert_eq!(l.lines[2].romanized, None);
    }
}
