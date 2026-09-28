//! `{{ variables }}` in a workflow's text: finding them, and saying what is wrong with one.
//!
//! Deliberately small. A placeholder is a dotted name and nothing else — no filters, no
//! conditions, no expressions — so a file reads the same to a person as it does to the engine.
//! `\{{` is a literal `{{`, for a prompt that has to show one.

/// One `{{ … }}` found in a piece of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    /// The dotted name, split: `workspace.branch` is `["workspace", "branch"]`.
    pub path: Vec<String>,
    /// Byte offset of the opening `{{` in the text.
    pub start: usize,
    /// Exactly as written, braces included — what a person would search the file for.
    pub raw: String,
}

/// Something in the text that looks like a placeholder but is not one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Malformed {
    pub start: usize,
    pub raw: String,
    pub message: String,
}

/// A piece of template text, taken apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    Var(Placeholder),
}

/// Split `text` into literal runs and placeholders, or say where it goes wrong. Every problem in
/// the text is reported, not just the first.
pub fn parse(text: &str) -> Result<Vec<Part>, Vec<Malformed>> {
    let mut parts = Vec::new();
    let mut problems = Vec::new();
    let mut literal = String::new();
    let mut rest = text;
    let mut offset = 0;
    while let Some(at) = rest.find("{{") {
        // `\{{` is written for a literal `{{`: keep the braces, drop the backslash.
        if rest[..at].ends_with('\\') {
            literal.push_str(&rest[..at - 1]);
            literal.push_str("{{");
            rest = &rest[at + 2..];
            offset += at + 2;
            continue;
        }
        literal.push_str(&rest[..at]);
        let start = offset + at;
        let after = &rest[at + 2..];
        let Some(close) = after.find("}}") else {
            let raw: String = rest[at..].lines().next().unwrap_or("").to_owned();
            problems.push(Malformed {
                start,
                raw,
                message: "`{{` is never closed with `}}`. Write `\\{{` for a literal `{{`."
                    .to_owned(),
            });
            rest = "";
            break;
        };
        let inner = &after[..close];
        let raw = format!("{{{{{inner}}}}}");
        match name(inner) {
            Ok(path) => {
                if !literal.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut literal)));
                }
                parts.push(Part::Var(Placeholder { path, start, raw }));
            }
            Err(message) => problems.push(Malformed {
                start,
                raw,
                message,
            }),
        }
        let consumed = at + 2 + close + 2;
        rest = &rest[consumed..];
        offset += consumed;
    }
    literal.push_str(rest);
    if !literal.is_empty() {
        parts.push(Part::Text(literal));
    }
    if problems.is_empty() {
        Ok(parts)
    } else {
        Err(problems)
    }
}

/// The placeholders in `text`, ignoring anything malformed.
pub fn placeholders(text: &str) -> Vec<Placeholder> {
    match parse(text) {
        Ok(parts) => parts
            .into_iter()
            .filter_map(|part| match part {
                Part::Var(var) => Some(var),
                Part::Text(_) => None,
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// `text` with each placeholder replaced by what `lookup` gives for its path, or nothing when it
/// gives nothing. A file is checked before it runs, so a malformed `text` is only returned as it
/// is, never half-filled.
pub fn render(text: &str, lookup: impl Fn(&[String]) -> Option<String>) -> String {
    let Ok(parts) = parse(text) else {
        return text.to_owned();
    };
    let mut out = String::with_capacity(text.len());
    for part in parts {
        match part {
            Part::Text(literal) => out.push_str(&literal),
            Part::Var(var) => out.push_str(&lookup(&var.path).unwrap_or_default()),
        }
    }
    out
}

/// Whether one path segment is a name: letters, digits, `_` and `-`.
pub fn is_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn name(inner: &str) -> Result<Vec<String>, String> {
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return Err("`{{ }}` is empty. Put a name inside, like `{{ workspace.branch }}`.".into());
    }
    let path: Vec<&str> = trimmed.split('.').collect();
    if path.iter().all(|segment| is_segment(segment)) {
        Ok(path.into_iter().map(str::to_owned).collect())
    } else {
        Err(format!(
            "`{{{{{inner}}}}}` is not a variable. A variable is a dotted name, like \
             `{{{{ workspace.branch }}}}`, and nothing else."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(path: &[&str], start: usize, raw: &str) -> Part {
        Part::Var(Placeholder {
            path: path.iter().map(|s| (*s).to_owned()).collect(),
            start,
            raw: raw.to_owned(),
        })
    }

    #[test]
    fn text_and_placeholders_are_split_apart() {
        let parts = parse("PR #{{ pr.number }} on {{workspace.branch}}.").unwrap();
        assert_eq!(
            parts,
            vec![
                Part::Text("PR #".into()),
                var(&["pr", "number"], 4, "{{ pr.number }}"),
                Part::Text(" on ".into()),
                var(&["workspace", "branch"], 23, "{{workspace.branch}}"),
                Part::Text(".".into()),
            ]
        );
    }

    #[test]
    fn a_backslash_keeps_braces_literal() {
        let parts = parse(r"Jinja uses \{{ name }} and so do we: {{ memory }}").unwrap();
        assert_eq!(
            parts[0],
            Part::Text("Jinja uses {{ name }} and so do we: ".into())
        );
        assert_eq!(parts[1], var(&["memory"], 37, "{{ memory }}"));
    }

    #[test]
    fn an_unclosed_placeholder_is_reported_where_it_opens() {
        let problems = parse("one {{ pr.number } two").unwrap_err();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].start, 4);
        assert!(problems[0].message.contains("never closed"));
    }

    #[test]
    fn anything_but_a_dotted_name_is_refused() {
        for bad in [
            "{{ }}",
            "{{ pr.number | upper }}",
            "{{ a..b }}",
            "{{ a b }}",
            "{{ inputs.x() }}",
        ] {
            let problems = parse(bad).unwrap_err();
            assert_eq!(problems.len(), 1, "{bad}");
            assert_eq!(problems[0].start, 0, "{bad}");
        }
    }

    #[test]
    fn every_problem_in_the_text_is_reported() {
        let problems = parse("{{ a b }} fine {{ ok }} {{ c d }}").unwrap_err();
        assert_eq!(problems.len(), 2);
        assert_eq!(problems[1].start, 24);
    }

    #[test]
    fn rendering_fills_each_placeholder_and_keeps_escaped_braces() {
        let lookup = |path: &[String]| match path.join(".").as_str() {
            "pr.number" => Some("42".to_owned()),
            _ => None,
        };
        assert_eq!(
            render(
                r"PR #{{ pr.number }}, \{{ literal }}, [{{ inputs.missing }}]",
                lookup
            ),
            "PR #42, {{ literal }}, []"
        );
        assert_eq!(render("broken {{ a b }}", lookup), "broken {{ a b }}");
    }

    #[test]
    fn plain_text_has_no_placeholders() {
        assert_eq!(
            parse("nothing here").unwrap(),
            vec![Part::Text("nothing here".into())]
        );
        assert!(placeholders("single { braces } are fine").is_empty());
    }
}
