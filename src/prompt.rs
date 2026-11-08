//! The prompt sent to the model, and cleaning up what it answers.

use openidea_plugin::{CommitChange, CommitDraft};

/// What the user chose in Settings › Plugins › AI Commit Message › Message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub style: String,
    pub language: String,
    pub body: String,
    pub use_draft: bool,
    pub instructions: String,
    pub max_diff_bytes: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            style: "conventional".into(),
            language: "English".into(),
            body: "auto".into(),
            use_draft: true,
            instructions: String::new(),
            max_diff_bytes: 64 * 1024,
        }
    }
}

pub const SYSTEM: &str = "You write git commit messages. Reply with the commit message only: \
no preamble, no explanation, no quotes and no code fences.";

/// The path inside its repository, with forward slashes.
fn relative(change: &CommitChange, path: &str) -> String {
    let repo = change.repository.trim_end_matches(['/', '\\']);
    let rel = path
        .strip_prefix(repo)
        .map(|p| p.trim_start_matches(['/', '\\']))
        .unwrap_or(path);
    rel.replace('\\', "/")
}

fn status_letter(status: &str) -> &'static str {
    match status {
        "added" => "A",
        "modified" => "M",
        "deleted" => "D",
        "renamed" => "R",
        "copied" => "C",
        "type-changed" => "T",
        _ => "?",
    }
}

/// Cuts `text` to at most `max` bytes at a character boundary.
fn cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The user message: rules, the author's hint, the file list and the diff.
pub fn build(draft: &CommitDraft, opts: &Options) -> String {
    let mut p = String::new();
    p.push_str("Write the commit message for the changes below.\n\nRules:\n");
    let language = if opts.language.trim().is_empty() {
        "English"
    } else {
        opts.language.trim()
    };
    p.push_str(&format!("- Write it in {language}.\n"));
    match opts.style.as_str() {
        "plain" => p.push_str("- Subject line: a short summary in the imperative mood (\"Add\", not \"Added\").\n"),
        "gitmoji" => p.push_str(
            "- Subject line: start with the one gitmoji (as the emoji character) that fits best, then a space and a short summary in the imperative mood.\n",
        ),
        _ => p.push_str(
            "- Follow Conventional Commits: `type(scope): subject`, type one of feat, fix, docs, style, refactor, perf, test, build, ci, chore, revert; \
the scope is optional and short; add `!` after the type or scope for a breaking change. The subject is in the imperative mood and starts lowercase.\n",
        ),
    }
    p.push_str("- The subject line has at most 72 characters and no period at the end.\n");
    match opts.body.as_str() {
        "never" => p.push_str("- Write the subject line only, no body.\n"),
        "always" => p.push_str("- After a blank line, add a body that explains what changed and why, wrapped at 72 columns.\n"),
        _ => p.push_str(
            "- Add a body (after a blank line, wrapped at 72 columns, a few lines or bullet points) only when the change is not obvious from the subject.\n",
        ),
    }
    p.push_str("- Base it only on the diff. Do not invent issue numbers, names or reasons.\n");
    if draft.amend {
        p.push_str("- This commit amends the previous one; the diff shows only the changes added on top of it.\n");
    }
    let hint = draft.message.trim();
    if opts.use_draft && !hint.is_empty() {
        p.push_str(&format!(
            "\nThe author already wrote this in the message box. Treat it as notes on the intent and keep what it says (rewrite it to follow the rules):\n<<<\n{hint}\n>>>\n"
        ));
    }
    let extra = opts.instructions.trim();
    if !extra.is_empty() {
        p.push_str(&format!("\nMore instructions from the author:\n{extra}\n"));
    }

    p.push_str("\nChanged files:\n");
    for c in &draft.changes {
        let path = relative(c, &c.path);
        match &c.old_path {
            Some(old) => p.push_str(&format!(
                "{} {} -> {}\n",
                status_letter(&c.status),
                relative(c, old),
                path
            )),
            None => p.push_str(&format!("{} {}\n", status_letter(&c.status), path)),
        }
    }

    p.push_str("\nDiff:\n```diff\n");
    let mut left = opts.max_diff_bytes.max(1024);
    let mut skipped = 0usize;
    let mut cut_any = draft.changes.iter().any(|c| c.truncated);
    for c in &draft.changes {
        let Some(diff) = c.diff.as_deref() else {
            p.push_str(&format!("# {}: binary file\n", relative(c, &c.path)));
            continue;
        };
        if left == 0 {
            skipped += 1;
            continue;
        }
        let part = cut(diff, left);
        if part.len() < diff.len() {
            cut_any = true;
        }
        p.push_str(part);
        if !part.ends_with('\n') {
            p.push('\n');
        }
        left -= part.len();
    }
    p.push_str("```\n");
    if cut_any || skipped > 0 {
        p.push_str("(The diff was cut to fit");
        if skipped > 0 {
            p.push_str(&format!(
                "; {skipped} more file(s) are only in the list above"
            ));
        }
        p.push_str(".)\n");
    }
    p
}

/// What the model answered, without fences, labels, quotes or extra blank lines.
pub fn clean(answer: &str) -> String {
    let mut text = answer.trim().replace("\r\n", "\n");
    // A whole answer in a code fence.
    if text.starts_with("```") {
        let mut lines: Vec<&str> = text.lines().collect();
        lines.remove(0);
        if lines
            .last()
            .is_some_and(|l| l.trim_start().starts_with("```"))
        {
            lines.pop();
        }
        text = lines.join("\n").trim().to_owned();
    }
    for label in [
        "Commit message:",
        "commit message:",
        "Commit Message:",
        "Message:",
    ] {
        if let Some(rest) = text.strip_prefix(label) {
            text = rest.trim().to_owned();
        }
    }
    for (open, close) in [("\"", "\""), ("'", "'"), ("`", "`")] {
        if text.len() >= 2
            && text.starts_with(open)
            && text.ends_with(close)
            && !text[1..text.len() - 1].contains(close)
        {
            text = text[1..text.len() - 1].trim().to_owned();
        }
    }
    // Trailing spaces, and at most one blank line in a row.
    let mut out: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim_end) {
        if line.is_empty() && out.last().is_some_and(|l| l.is_empty()) {
            continue;
        }
        out.push(line);
    }
    out.join("\n").trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &str, diff: Option<&str>) -> CommitChange {
        CommitChange {
            repository: "/repo".into(),
            path: format!("/repo/{path}"),
            old_path: None,
            status: "modified".into(),
            diff: diff.map(Into::into),
            truncated: false,
        }
    }

    fn draft(message: &str, changes: Vec<CommitChange>) -> CommitDraft {
        CommitDraft {
            revision: 1,
            message: message.into(),
            amend: false,
            changes,
        }
    }

    #[test]
    fn lists_files_relative_and_includes_the_diff() {
        let d = draft(
            "",
            vec![
                change("src/a.rs", Some("@@ -1 +1 @@\n-a\n+b\n")),
                change("img.png", None),
            ],
        );
        let p = build(&d, &Options::default());
        assert!(p.contains("M src/a.rs\n"));
        assert!(p.contains("+b\n"));
        assert!(p.contains("# img.png: binary file"));
        assert!(p.contains("Conventional Commits"));
        assert!(!p.contains("message box"));
    }

    #[test]
    fn uses_the_draft_as_a_hint_only_when_asked() {
        let d = draft("fix login crash", vec![change("a", Some("+x\n"))]);
        assert!(build(&d, &Options::default()).contains("fix login crash"));
        let off = Options {
            use_draft: false,
            ..Options::default()
        };
        assert!(!build(&d, &off).contains("fix login crash"));
    }

    #[test]
    fn cuts_long_diffs_and_says_so() {
        let big = "+".repeat(5000) + "\n";
        let d = draft(
            "",
            vec![change("a", Some(&big)), change("b", Some("+tail\n"))],
        );
        let p = build(
            &d,
            &Options {
                max_diff_bytes: 1024,
                ..Options::default()
            },
        );
        assert!(p.contains("1 more file(s)"));
        assert!(!p.contains("+tail"));
        assert!(p.len() < 3000);
    }

    #[test]
    fn windows_paths_become_relative() {
        let c = CommitChange {
            repository: "C:\\work\\repo".into(),
            path: "C:\\work\\repo\\src\\main.rs".into(),
            old_path: Some("C:\\work\\repo\\src\\old.rs".into()),
            status: "renamed".into(),
            diff: Some(String::new()),
            truncated: false,
        };
        let p = build(&draft("", vec![c]), &Options::default());
        assert!(p.contains("R src/old.rs -> src/main.rs"));
    }

    #[test]
    fn cleans_fences_labels_and_quotes() {
        assert_eq!(clean("```\nfeat: add x\n```"), "feat: add x");
        assert_eq!(
            clean("```text\nfeat: add x\n\n\n\nbody  \n```\n"),
            "feat: add x\n\nbody"
        );
        assert_eq!(clean("Commit message: fix: y"), "fix: y");
        assert_eq!(clean("\"docs: z\""), "docs: z");
        assert_eq!(clean("  chore: bump\r\n"), "chore: bump");
    }
}
