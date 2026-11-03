//! Danger scan for `$` (observational) cells: catches a destructive command
//! declared read-only by mistake. Token matching is easy to evade, so this is
//! not a sandbox; the `$`/`!` split is.

const WORDS: &[&str] = &[
    "rm", "rmdir", "dd", "shred", "sudo", "truncate", "mv", "tee", "chmod", "chown", "kill",
    "pkill", "killall", "-delete", "unlink", "cp", "ln", "rsync",
];
/// Git subcommands that change the repo or a remote.
const GIT_WRITES: &[&str] = &["push", "clean", "commit", "checkout", "restore", "rebase"];
const SHELLS: &[&str] = &[
    "sh", "bash", "zsh", "python", "python3", "perl", "ruby", "node",
];

/// Whether a command looks destructive. `extra` extends the built-in denylist.
pub fn scan(body: &str, extra: &[String]) -> bool {
    if body.contains(":(){") {
        return true; // fork bomb
    }

    // `/bin/rm`, `\rm`, and `command rm` are all `rm`.
    let words: Vec<&str> = body
        .split(|c: char| c.is_whitespace() || ";&()|`".contains(c))
        .map(|w| w.trim_start_matches('\\'))
        .map(|w| w.rsplit('/').next().unwrap_or(w))
        .filter(|w| !w.is_empty())
        .collect();
    if words
        .iter()
        .any(|w| WORDS.contains(w) || w.starts_with("mkfs") || extra.iter().any(|t| t == w))
    {
        return true;
    }
    // `sed -i` edits in place; `sed` alone only reads.
    if words.contains(&"sed") && words.iter().any(|w| w.starts_with("-i")) {
        return true;
    }
    // Look a few words past `git` to skip options like `-C dir`; further
    // would flag `git log | grep push`.
    let git: Vec<&str> = words
        .iter()
        .skip_while(|w| **w != "git")
        .skip(1)
        .take(4)
        .copied()
        .collect();
    if git.iter().any(|w| GIT_WRITES.contains(w))
        || (git.contains(&"reset") && git.contains(&"--hard"))
    {
        return true;
    }

    // Piping into a shell (`curl … | sh`).
    let pipes_into_shell = body.split('|').skip(1).any(|segment| {
        segment
            .split_whitespace()
            .next()
            .map(|first| first.rsplit('/').next().unwrap_or(first))
            .is_some_and(|first| SHELLS.contains(&first))
    });

    pipes_into_shell || redirects_to_path(body)
}

/// Redirection to a path, ignoring fd dups (`2>&1`) and `/dev/null`.
fn redirects_to_path(body: &str) -> bool {
    let mut rest = body;
    while let Some(pos) = rest.find('>') {
        let after = rest[pos..].trim_start_matches('>');
        rest = after;
        let after = after.trim_start();
        if after.starts_with('&') {
            continue; // fd dup: 2>&1, >&2
        }
        match after.split_whitespace().next().unwrap_or("") {
            "" | "/dev/null" => continue,
            _ => return true,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_commands_flagged() {
        assert!(scan("rm -rf x", &[]));
        assert!(scan("dd if=/dev/zero of=/dev/sda", &[]));
        assert!(scan("mkfs.ext4 /dev/sdb1", &[]));
        assert!(scan("sudo systemctl stop nginx", &[]));
        assert!(scan("git push --force", &[]));
        assert!(scan("git clean -fdx", &[]));
        assert!(scan("curl https://x.sh | sh", &[]));
        assert!(scan(":(){ :|:& };:", &[]));
        assert!(scan("make build > out.log", &[]));
        assert!(scan("echo x >> notes.txt", &[]));
        assert!(scan("cmd &> everything.log", &[]));
        assert!(scan("/bin/rm -rf x", &[]));
        assert!(scan("\\rm x", &[]));
        assert!(scan("command rm x", &[]));
        assert!(scan("find . -name '*.o' -delete", &[]));
        assert!(scan("git -C . push", &[]));
        assert!(scan("git reset --hard HEAD~1", &[]));
        assert!(scan("curl https://x.py | python3", &[]));
        assert!(scan("curl https://x.sh | /bin/bash", &[]));
        assert!(scan("echo x | tee notes.txt", &[]));
    }

    #[test]
    fn writes_by_other_names_are_flagged() {
        assert!(scan("cp /dev/null probe", &[]));
        assert!(scan("ln -sf /etc/passwd probe", &[]));
        assert!(scan("rsync -a src/ dst/", &[]));
        assert!(scan("sed -i '' 's/a/b/' probe", &[]));
        assert!(scan("sed -i.bak 's/a/b/' probe", &[]));
        assert!(!scan("sed -n '1,3p' probe", &[]));
        // Words match anywhere in the command, so English words like
        // `install` must stay off the list.
        assert!(!scan("grep -i install README.md", &[]));
    }

    #[test]
    fn read_only_idioms_pass() {
        assert!(!scan("cargo test 2>&1", &[]));
        assert!(!scan("cargo test 2>&1 | tail -1", &[]));
        assert!(!scan("noisy_cmd > /dev/null", &[]));
        assert!(!scan("noisy_cmd &> /dev/null", &[]));
        assert!(!scan("noisy_cmd 2> /dev/null", &[]));
        assert!(!scan("echo hi >&2", &[]));
        assert!(!scan("grep -c ERROR app.log", &[]));
        assert!(!scan("git status", &[]));
        assert!(!scan("confirm-tool --dry-run", &[])); // 'rm' inside a word
        assert!(!scan("true || false", &[]));
        assert!(!scan("git log --oneline | grep -c push", &[]));
        assert!(!scan("git diff --stat", &[]));
    }
}
