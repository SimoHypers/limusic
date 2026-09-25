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

use crate::lyrics::Lyrics;

/// Fill `romanized` on every line that needs one and set `lyrics.script`, the key the UI remembers
/// the toggle under. Lines already in Latin script get nothing, so an English chorus in a K-pop
/// song is not printed twice.
pub fn fill(lyrics: &mut Lyrics) {
    let all: String = lyrics.lines.iter().map(|l| l.text.as_str()).collect();
    let Some(script) = dominant_script(&all) else {
        return;
    };
    lyrics.script = Some(script.into());
    // Apple's reading or ours, never both in one song: they spell differently, and the mix reads
    // as a bug. Apple leaves its Latin lines out, which is also what this does.
    if lyrics.lines.iter().any(|l| l.romanized.is_some()) {
        return;
    }
    for line in &mut lyrics.lines {
        // A Japanese line in a mostly Korean song (a J-version verse) is still Japanese.
        let japanese =
            script == "ja" || line.text.chars().any(|c| matches!(c, '\u{3040}'..='\u{30FF}'));
        let r = romanize_line(&line.text, japanese);
        if r != line.text {
            line.romanized = Some(r);
        }
    }
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

/// The script the song is mostly written in, as the key the UI stores its toggle under. `None`
/// when there is nothing to romanize.
fn dominant_script(text: &str) -> Option<&'static str> {
    let (mut kana, mut han, mut hangul) = (0usize, 0usize, 0usize);
    let mut other: Vec<(&'static str, usize)> = Vec::new();
    for c in text.chars() {
        let key = match c as u32 {
            0x3040..=0x30FF | 0x31F0..=0x31FF => {
                kana += 1;
                continue;
            }
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => {
                han += 1;
                continue;
            }
            0xAC00..=0xD7A3 => {
                hangul += 1;
                continue;
            }
            0x0370..=0x03FF | 0x1F00..=0x1FFF => "grek",
            0x0400..=0x052F => "cyrl",
            0x0530..=0x058F => "armn",
            0x10A0..=0x10FF => "geor",
            _ => continue,
        };
        match other.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => other.push((key, 1)),
        }
    }
    // Kanji alone cannot tell Japanese from Chinese, kana can. Japanese prose runs well over half
    // kana, so a tenth is a safe floor, and it keeps a Chinese song with one stylised の Chinese.
    let japanese = kana > 0 && kana * 10 >= han;
    let cjk = if japanese { ("ja", kana + han) } else { ("zh", han) };
    [cjk, ("ko", hangul)]
        .into_iter()
        .chain(other)
        .filter(|(_, n)| *n > 0)
        .max_by_key(|(_, n)| *n)
        .map(|(k, _)| k)
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
    reading: String,
}

fn japanese_run(run: &str) -> String {
    let Some(tokenizer) = TOKENIZER.as_ref() else {
        return run.to_owned();
    };
    let Ok(mut tokens) = tokenizer.tokenize(run) else {
        return run.to_owned();
    };
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

    // Words as Apple writes them: an inflection stays on its stem ("yokatta", "aishiteru"), the
    // copula and particles stand alone ("yume nara ba", "koto o").
    let mut words: Vec<String> = Vec::new();
    let mut prev_copula = false;
    for m in &morphs {
        if m.reading.is_empty() {
            continue;
        }
        let reading = match (m.pos.as_str(), m.surface.as_str()) {
            ("助詞", "は") => "ワ",
            ("助詞", "へ") => "エ",
            ("助詞", "を") => "オ",
            _ => m.reading.as_str(),
        };
        let copula = m.pos == "助動詞" && matches!(m.base.as_str(), "だ" | "です");
        let glue = (m.pos == "助動詞" && (!copula || prev_copula))
            || (m.sub == "接続助詞" && matches!(m.surface.as_str(), "て" | "で"))
            || (m.sub == "非自立"
                && matches!(m.base.as_str(), "てる" | "でる" | "とく" | "ちゃう" | "じゃう"))
            || m.sub == "接尾";
        match words.last_mut() {
            Some(w) if glue => w.push_str(reading),
            _ => words.push(reading.to_owned()),
        }
        prev_copula = copula;
    }
    // Converted per word, not per morpheme, so a trailing っ doubles the next consonant.
    words.iter().map(|w| w.to_romaji()).collect::<Vec<_>>().join(" ")
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
    fn picks_the_song_language_from_all_the_lines() {
        // A kanji-only line in a Japanese song is still Japanese.
        assert_eq!(dominant_script("東京の空\n東京"), Some("ja"));
        assert_eq!(dominant_script("还记得你说家是唯一的城堡の"), Some("zh"));
        assert_eq!(dominant_script("보고 싶다 Baby"), Some("ko"));
        assert_eq!(dominant_script("Я тебя люблю"), Some("cyrl"));
        assert_eq!(dominant_script("Only English here"), None);
        assert_eq!(dominant_script("तुम ही हो"), None);
    }

    #[test]
    fn fill_skips_latin_lines_and_defers_to_apple() {
        use crate::lyrics::LyricLine;
        let mut l = Lyrics {
            source: "t".into(),
            synced: false,
            instrumental: false,
            script: None,
            lines: vec![
                LyricLine::simple(None, "Я тебя люблю".into()),
                LyricLine::simple(None, "Hey".into()),
            ],
        };
        fill(&mut l);
        assert_eq!(l.script.as_deref(), Some("cyrl"));
        assert_eq!(l.lines[0].romanized.as_deref(), Some("Ya tebya lyublyu"));
        assert_eq!(l.lines[1].romanized, None);

        l.lines[0].romanized = Some("from apple".into());
        l.lines.push(LyricLine::simple(None, "Привет".into()));
        fill(&mut l);
        assert_eq!(l.lines[2].romanized, None);
    }
}
