//! Names for workspaces: a slug of what the user asked for, or a railway station when they
//! asked for nothing.

/// Longest slug we generate. Branch and folder names this long still read well in a sidebar,
/// and keep Windows paths short.
const MAX_LEN: usize = 32;
const MAX_WORDS: usize = 4;

/// Words that carry no meaning in a task description.
const FILLER: &[&str] = &[
    "a", "an", "the", "to", "of", "in", "on", "for", "and", "or", "with", "please", "can", "you",
    "could", "would", "we", "i", "me", "my", "our", "it", "is", "are", "be", "that", "this",
];

const STATIONS: &[&str] = &[
    "paddington",
    "shinjuku",
    "grand-central",
    "gare-du-nord",
    "atocha",
    "zurich-hb",
    "penn",
    "kings-cross",
    "union",
    "termini",
    "centraal",
    "hauptbahnhof",
    "luz",
    "flinders",
    "waverley",
    "st-pancras",
    "kyoto",
    "anhalter",
    "santa-justa",
    "sirkeci",
    "keleti",
    "oriente",
    "haymarket",
];

/// Turn free text into a branch- and folder-safe slug: lowercase ASCII words joined by dashes.
/// Returns `None` when nothing usable is left (empty prompt, only symbols, non-Latin script).
pub fn slugify(text: &str) -> Option<String> {
    slug(&task_text(text), true)
}

/// Like [`slugify`], for names rather than sentences: every word counts ("My App" → `my-app`).
pub fn slugify_name(text: &str) -> Option<String> {
    slug(text, false)
}

/// Prefer an explicit request anywhere in the prose over introductory context. This is
/// deliberately local extraction, not a semantic summary: unknown wording keeps the fallback.
fn task_text(text: &str) -> String {
    const ACTIONS: &[&str] = &[
        "add",
        "fix",
        "implement",
        "improve",
        "refactor",
        "remove",
        "rename",
        "update",
        "support",
        "create",
        "replace",
        "resolve",
        "investigate",
        "debug",
        "optimize",
        "migrate",
        "document",
        "write",
        "test",
        "enable",
        "disable",
        "simplify",
    ];
    let mut fenced = false;
    let mut prose = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fenced = !fenced;
        } else if !fenced {
            prose.push(line);
        }
    }
    let prose = prose.join("\n");
    for clause in prose
        .split(['!', '?', '\n', ';'])
        .flat_map(|line| line.split(". "))
    {
        let words: Vec<&str> = clause
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        for (index, word) in words.iter().enumerate() {
            if !ACTIONS.contains(&word.to_ascii_lowercase().as_str()) {
                continue;
            }
            let lead = words[..index].join(" ").to_ascii_lowercase();
            // Do not mistake a verb mentioned in background prose (or a negative request)
            // for the task. Only strip known conversational request prefixes.
            if matches!(
                lead.as_str(),
                "" | "please"
                    | "can you"
                    | "can you please"
                    | "could you"
                    | "could you please"
                    | "would you"
                    | "would you please"
                    | "i want to"
                    | "i need to"
                    | "we need to"
                    | "we should"
                    | "let s"
                    | "help me"
                    | "help me to"
                    | "i would like to"
                    | "i d like to"
                    | "task"
                    | "goal"
                    | "request"
            ) && words[index + 1..]
                .iter()
                .any(|word| !FILLER.contains(&word.to_ascii_lowercase().as_str()))
            {
                return words[index..].join(" ");
            }
        }
    }
    prose
}

fn slug(text: &str, drop_filler: bool) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let meaningful: Vec<&String> = words
        .iter()
        .filter(|w| !drop_filler || !FILLER.contains(&w.as_str()))
        .collect();
    // A prompt made only of filler ("can you please") still beats a random name.
    let chosen: Vec<&String> = if meaningful.is_empty() {
        words.iter().collect()
    } else {
        meaningful
    };

    let mut slug = String::new();
    for word in chosen.into_iter().take(MAX_WORDS) {
        let extra = if slug.is_empty() { 0 } else { 1 };
        if slug.len() + extra + word.len() > MAX_LEN {
            if slug.is_empty() {
                slug.push_str(&word[..MAX_LEN]);
            }
            break;
        }
        if extra == 1 {
            slug.push('-');
        }
        slug.push_str(word);
    }
    (!slug.is_empty()).then_some(slug)
}

/// A slug for `prompt`, or a station name picked by `seed` when the prompt yields none.
pub fn base_name(prompt: &str, seed: usize) -> String {
    slugify(prompt).unwrap_or_else(|| STATIONS[seed % STATIONS.len()].to_owned())
}

/// `base`, or `base-2`, `base-3`, … — the first candidate `taken` does not object to.
pub fn unique(base: &str, mut taken: impl FnMut(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !taken(candidate))
        .expect("an unbounded range always yields a free name")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_become_short_readable_slugs() {
        assert_eq!(
            slugify("Fix the login bug in auth.ts!").unwrap(),
            "fix-login-bug-auth"
        );
        assert_eq!(
            slugify("  Can you please add dark mode?  ").unwrap(),
            "add-dark-mode"
        );
        assert_eq!(
            slugify("Refactor: API v2 → v3").unwrap(),
            "refactor-api-v2-v3"
        );
    }

    #[test]
    fn task_requests_win_over_introductory_context_and_code() {
        for prompt in [
            "I've been looking at the settings screen. Could you please add dark mode?",
            "Background: the app is too bright.\nTask: add dark mode",
            "```text\nremove all settings\n```\nI would like to add dark mode.",
            "Do not remove the theme picker. Add dark mode instead.",
        ] {
            assert_eq!(
                slugify(prompt).unwrap(),
                if prompt.contains("instead") {
                    "add-dark-mode-instead"
                } else {
                    "add-dark-mode"
                }
            );
        }
        assert_eq!(
            slugify("Please fix auth.ts crashes").unwrap(),
            "fix-auth-ts-crashes"
        );
        assert_eq!(
            slugify("I want to fix login crashes").unwrap(),
            "fix-login-crashes"
        );
        assert_eq!(
            slugify("The fix failed. Please investigate login crashes").unwrap(),
            "investigate-login-crashes"
        );
        assert_eq!(
            slugify("Login crashes after logout").unwrap(),
            "login-crashes-after-logout"
        );
        assert_eq!(
            slugify_name("I want to fix login").unwrap(),
            "i-want-to-fix"
        );
    }

    #[test]
    fn names_keep_every_word() {
        assert_eq!(slugify_name("My App").unwrap(), "my-app");
        assert_eq!(slugify_name("the-thing_v2").unwrap(), "the-thing-v2");
    }

    #[test]
    fn slugs_are_capped_in_words_and_length() {
        let slug =
            slugify("implement comprehensive internationalization infrastructure everywhere now")
                .unwrap();
        assert!(slug.len() <= MAX_LEN, "{slug}");
        assert_eq!(slug, "implement-comprehensive");
        assert_eq!(slugify(&"x".repeat(80)).unwrap().len(), MAX_LEN);
    }

    #[test]
    fn slugs_are_safe_for_branches_and_folders_everywhere() {
        let slug = slugify("../../etc/passwd; rm -rf ~ && echo \"hi\" | CON.txt").unwrap();
        assert!(slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        assert!(!slug.starts_with('-') && !slug.ends_with('-') && !slug.contains("--"));
    }

    #[test]
    fn unusable_prompts_fall_back_to_a_station() {
        assert_eq!(slugify(""), None);
        assert_eq!(slugify("¿¡…!?"), None);
        assert_eq!(slugify("修复登录"), None);
        assert_eq!(base_name("", 0), "paddington");
        assert_eq!(base_name("", STATIONS.len() + 1), "shinjuku");
        assert_eq!(slugify("can you please").unwrap(), "can-you-please");
    }

    #[test]
    fn taken_names_get_a_number() {
        let taken = ["fix-bug", "fix-bug-2"];
        assert_eq!(unique("fix-bug", |n| taken.contains(&n)), "fix-bug-3");
        assert_eq!(unique("fresh", |n| taken.contains(&n)), "fresh");
    }
}
