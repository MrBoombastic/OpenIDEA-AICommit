//! The Conventional Commits check on the subject line.

pub const TYPES: &[&str] = &[
    "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert",
];

/// Why the first line is not `type(scope)!: subject`, or `None` when it is
/// (or when it is a message git writes itself: merges, reverts, fixups).
pub fn problem(message: &str) -> Option<String> {
    let subject = message
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))?;
    for generated in ["Merge ", "Revert \"", "fixup! ", "squash! ", "amend! "] {
        if subject.starts_with(generated) {
            return None;
        }
    }
    let Some((head, rest)) = subject.split_once(':') else {
        return Some(format!(
            "the subject should start with a type, like \"feat: …\" or \"fix(scope): …\" (types: {})",
            TYPES.join(", ")
        ));
    };
    let head = head.strip_suffix('!').unwrap_or(head);
    let kind = match head.split_once('(') {
        Some((kind, scope)) => match scope.strip_suffix(')') {
            Some(scope) if !scope.trim().is_empty() && !scope.contains(['(', ')']) => kind,
            _ => {
                return Some(format!(
                    "the scope in \"{head}\" should be a word in parentheses, like \"fix(api): …\""
                ));
            }
        },
        None => head,
    };
    if !TYPES.contains(&kind) {
        return Some(format!(
            "\"{kind}\" is not a Conventional Commits type (use one of {})",
            TYPES.join(", ")
        ));
    }
    if !rest.starts_with(' ') || rest.trim().is_empty() {
        return Some(
            "put a space and a description after the colon, like \"feat: add export\"".into(),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::problem;

    #[test]
    fn accepts_conventional_subjects() {
        for ok in [
            "feat: add export",
            "fix(api): handle 404\n\nbody",
            "refactor!: drop old config",
            "feat(ui)!: new layout",
            "\n# comment\nchore: bump deps",
            "Merge branch 'main' into dev",
            "Revert \"feat: x\"",
            "fixup! feat: x",
            "",
        ] {
            assert_eq!(problem(ok), None, "{ok:?}");
        }
    }

    #[test]
    fn rejects_the_rest() {
        assert!(
            problem("Add export")
                .unwrap()
                .contains("should start with a type")
        );
        assert!(
            problem("feature: add export")
                .unwrap()
                .contains("not a Conventional Commits type")
        );
        assert!(problem("fix(): x").unwrap().contains("scope"));
        assert!(problem("fix(api: x").unwrap().contains("scope"));
        assert!(problem("fix:x").unwrap().contains("space"));
        assert!(problem("fix: ").unwrap().contains("space"));
    }
}
