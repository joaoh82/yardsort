//! What the workflow parser and the smoke test parser have in common: a problem with a line and
//! a column on it, serde's errors tidied, positions in the source, durations, names, and "did
//! you mean".
//!
//! Both parsers read a YAML file with `serde-saphyr`, whose `Spanned<T>` keeps where each value
//! was, and both report every mistake they find rather than the first. The pieces here are the
//! ones that know nothing about what either file means.

use serde::{Deserialize, Serialize};
use serde_saphyr::Location;
use specta::Type;

/// The longest a timeout may be. A run waiting a week is a run somebody forgot.
pub const MAX_TIMEOUT_SECS: u64 = 7 * 24 * 60 * 60;

/// One thing wrong with a file, where it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// 1-based. `None` when the problem is the file as a whole.
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
}

impl Problem {
    pub fn at(line: usize, column: usize, message: impl Into<String>) -> Self {
        Self {
            line: Some(line as u32),
            column: Some(column as u32),
            message: message.into(),
        }
    }

    pub fn whole(message: impl Into<String>) -> Self {
        Self {
            line: None,
            column: None,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => write!(f, "{line}:{column}: {}", self.message),
            (Some(line), None) => write!(f, "{line}: {}", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

/// A file that did not check out. Whatever could be read of it comes along, so it can still be
/// listed by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    pub id: Option<String>,
    pub name: Option<String>,
    pub problems: Vec<Problem>,
}

/// Just enough to name a file that does not parse, and to see that it is from a later version
/// before its unknown keys are blamed on it.
#[derive(Deserialize)]
struct Head {
    #[serde(default)]
    id: Option<serde_saphyr::Spanned<HeadValue>>,
    #[serde(default)]
    name: Option<serde_saphyr::Spanned<HeadValue>>,
    #[serde(default)]
    version: Option<serde_saphyr::Spanned<HeadValue>>,
}

/// Anything at all; only a string or a number is kept.
#[derive(Deserialize)]
#[serde(untagged)]
enum HeadValue {
    Number(u64),
    Text(String),
    #[allow(dead_code)]
    Other(serde::de::IgnoredAny),
}

impl HeadValue {
    fn text(&self) -> Option<String> {
        match self {
            HeadValue::Text(text) => Some(text.clone()),
            HeadValue::Number(n) => Some(n.to_string()),
            HeadValue::Other(_) => None,
        }
    }
}

/// The top of a file, read leniently: its `id` and `name` when it has them, and whether its
/// `version` is one a later Yardsort reads. `kind` names the file in that message, "workflow"
/// or "smoke test".
pub struct Heading {
    pub id: Option<String>,
    pub name: Option<String>,
    /// The file asks for a later schema than `current`; this is the problem to report instead
    /// of blaming its new keys.
    pub later_version: Option<Problem>,
}

pub fn heading(text: &str, kind: &str, current: u32) -> Heading {
    let head = serde_saphyr::from_str::<Head>(text).ok();
    let id = head
        .as_ref()
        .and_then(|h| h.id.as_ref())
        .and_then(|v| v.value.text());
    let name = head
        .as_ref()
        .and_then(|h| h.name.as_ref())
        .and_then(|v| v.value.text());
    let later_version = head
        .as_ref()
        .and_then(|h| h.version.as_ref())
        .and_then(|version| match version.value {
            HeadValue::Number(n) if n > u64::from(current) => Some(at(
                &version.referenced,
                format!(
                    "This file is for a later Yardsort ({kind} version {n}); this one reads \
                     version {current}. Update Yardsort to run it."
                ),
            )),
            _ => None,
        });
    Heading {
        id,
        name,
        later_version,
    }
}

/// serde's message, without its trailing "at line N, column M" — the position is ours to show.
pub fn from_serde(error: &serde_saphyr::Error) -> Problem {
    let plain = error.without_snippet().to_string();
    let message = match plain.rfind(" at line ") {
        Some(cut) => plain[..cut].to_owned(),
        None => plain,
    };
    let message = match message.strip_prefix("duplicate mapping key: ") {
        Some(rest) => {
            let key = rest.split(',').next().unwrap_or(rest).trim();
            format!("`{key}` is given twice.")
        }
        None => capitalise(&message),
    };
    match error.location() {
        Some(location) => at(&location, message),
        None => Problem::whole(message),
    }
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A problem at a spanned value's position.
pub fn at(location: &Location, message: impl Into<String>) -> Problem {
    Problem::at(
        location.line() as usize,
        location.column() as usize,
        message,
    )
}

/// A name for an input or a step: what a `{{ … }}` path segment can hold, starting with a letter.
pub fn is_name(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A file's id: what the file is called.
pub fn is_id(text: &str) -> bool {
    text.len() <= 64
        && text.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `45m`, `30s`, `2h`: a whole number and one unit, in seconds. `what` is the word for it in
/// the message: "timeout", "interval".
pub fn duration(text: &str, what: &str) -> Result<u64, String> {
    let text = text.trim();
    let usage = format!("Write {what} as a whole number and s, m or h, like `45m`.");
    let not_a_duration = || format!("`{text}` is not {what}. {usage}");
    // Matched as suffixes, never sliced by byte count: the last character of a file's text can be
    // any width, and a slice through the middle of one panics.
    let (number, scale) = [("s", 1), ("m", 60), ("h", 60 * 60)]
        .into_iter()
        .find_map(|(unit, scale)| text.strip_suffix(unit).map(|number| (number, scale)))
        .ok_or_else(not_a_duration)?;
    // Digits only: `u64::from_str` would also take a leading `+`.
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return Err(not_a_duration());
    }
    let count: u64 = number.parse().map_err(|_| not_a_duration())?;
    let secs = count.saturating_mul(scale);
    if secs == 0 {
        return Err(format!(
            "{} of zero would end at once.",
            capitalise(what.trim_start_matches("a ").trim_start_matches("an "))
        ));
    }
    if secs > MAX_TIMEOUT_SECS {
        return Err(format!(
            "{} may be at most a week (`168h`).",
            capitalise(what)
        ));
    }
    Ok(secs)
}

/// Where in `source` the text `raw`, found at byte `start` of `value`, is written.
///
/// `field` is where `value` begins in the file. YAML may fold, indent or unescape a value, so an
/// offset in the value is not an offset in the file; the text itself is the same in both. The
/// same text can appear more than once, so this finds the same *occurrence*: the second
/// `{{ pr.nubmer }}` in the value is the second in the file. Escaped ones, `\{{`, are literals
/// and are counted in neither.
pub fn find(
    source: &str,
    field: &Location,
    value: &str,
    raw: &str,
    start: usize,
) -> (usize, usize) {
    let fallback = (field.line() as usize, field.column() as usize);
    let Some(from) = offset_of(source, field.line() as usize, field.column() as usize) else {
        return fallback;
    };
    let nth = unescaped(value, raw).take_while(|&at| at < start).count();
    match unescaped(&source[from..], raw).nth(nth) {
        Some(found) => position_of(source, from + found),
        None => fallback,
    }
}

/// Where `needle` occurs in `hay`, leaving out occurrences escaped with a backslash.
fn unescaped<'a>(hay: &'a str, needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    hay.match_indices(needle)
        .map(|(at, _)| at)
        .filter(move |&at| !hay[..at].ends_with('\\'))
}

/// The byte offset of a 1-based line and character column.
fn offset_of(source: &str, line: usize, column: usize) -> Option<usize> {
    let mut start = 0;
    for _ in 1..line {
        start += source[start..].find('\n')? + 1;
    }
    let text = &source[start..];
    let within = text
        .char_indices()
        .nth(column.saturating_sub(1))
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    Some(start + within)
}

/// The 1-based line and character column of a byte offset.
fn position_of(source: &str, offset: usize) -> (usize, usize) {
    let before = &source[..offset];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, source[line_start..offset].chars().count() + 1)
}

/// Each item in backticks, comma-separated.
pub fn list<T: std::fmt::Display>(items: impl IntoIterator<Item = T>) -> String {
    items
        .into_iter()
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// ". Did you mean `x`?" when one name is close, else ".".
pub fn suggestion<'a>(name: &str, known: impl Iterator<Item = &'a String>) -> String {
    let close = known
        .filter(|k| distance(name, k) <= 2)
        .min_by_key(|k| distance(name, k));
    match close {
        Some(k) => format!(". Did you mean `{k}`?"),
        None => ".".to_owned(),
    }
}

/// Edit distance, for "did you mean". Names are short.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let here = row[j + 1];
            row[j + 1] = if ca == *cb {
                prev
            } else {
                1 + prev.min(row[j]).min(here)
            };
            prev = here;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_take_one_unit() {
        assert_eq!(duration("45m", "a timeout"), Ok(45 * 60));
        assert_eq!(duration("30s", "a timeout"), Ok(30));
        assert_eq!(duration(" 2h ", "a timeout"), Ok(7200));
        for bad in [
            "45", "m", "1h30m", "-5m", "+5m", "4.5h", "10d", "", "1秒", "秒", "5 m",
        ] {
            assert!(duration(bad, "a timeout").is_err(), "{bad}");
        }
        assert!(duration("0m", "a timeout").unwrap_err().contains("zero"));
        assert!(duration("169h", "a timeout").unwrap_err().contains("week"));
        assert_eq!(
            duration("0s", "an interval").unwrap_err(),
            "Interval of zero would end at once."
        );
    }

    #[test]
    fn positions_count_characters_not_bytes() {
        let source = "a: é\nb: {{ x }}\n";
        let offset = offset_of(source, 2, 4).unwrap();
        assert_eq!(&source[offset..offset + 2], "{{");
        assert_eq!(position_of(source, offset), (2, 4));
        assert_eq!(offset_of("é{{", 1, 2), Some(2));
    }

    #[test]
    fn close_names_are_suggested() {
        let known = ["review".to_owned(), "notify".to_owned()];
        assert_eq!(
            suggestion("reveiw", known.iter()),
            ". Did you mean `review`?"
        );
        assert_eq!(suggestion("zzz", known.iter()), ".");
    }

    #[test]
    fn a_heading_is_read_from_a_file_that_will_not_check_out() {
        // Well-formed YAML with keys no file has: the head still reads. Broken YAML reads nothing,
        // and the caller falls back to the file name.
        let read = heading("id: x\nname: 4\nversion: 9\nnonsense: 1\n", "smoke test", 1);
        assert_eq!(read.id.as_deref(), Some("x"));
        assert_eq!(read.name.as_deref(), Some("4"));
        let later = read.later_version.unwrap();
        assert_eq!(later.line, Some(3));
        assert!(later.message.contains("smoke test version 9"), "{later}");
        assert!(heading("version: 1\n", "workflow", 1)
            .later_version
            .is_none());
        assert!(heading("id: [", "workflow", 1).id.is_none());
    }
}
