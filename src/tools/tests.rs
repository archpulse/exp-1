//! Tool implementations: read, write, edit, shell, ast.
//!
//! All paths are resolved against the session working directory. Nothing is
//! sandboxed: commands run as the user.

use super::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::{Mutex, oneshot};

/// A foreground command still running after this long is promoted to a
/// background task, so the agent is never blocked on something slow. A
/// background task has no clock on it: builds and test suites run for minutes,
/// and the old 120s axe was cutting them off mid-flight.

#[cfg(test)]
mod tests {
    use super::*;

    /// Escape sequences are stripped: colour codes are noise in a model's
    /// context, and some tools emit them even with TERM=dumb.
    #[test]
    fn escape_sequences_are_stripped() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m plain"), "red plain");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}after"), "after");
        assert_eq!(strip_ansi("no escapes"), "no escapes");
    }

    /// A capped output keeps both ends: the head says what ran, the tail says
    /// how it ended, and the middle is what goes.
    #[test]
    fn a_capped_output_keeps_both_ends() {
        let text = format!("first line\n{}\nlast line", "x".repeat(MAX_OUTPUT * 2));
        let clamped = clamp(&text, None);
        assert!(clamped.starts_with("first line"), "{}", &clamped[..40.min(clamped.len())]);
        assert!(
            clamped.ends_with("last line"),
            "{}",
            &clamped[clamped.len().saturating_sub(40)..]
        );
        assert!(clamped.contains("omitted"), "the gap is marked: {}", clamped.len());
        assert!(clamped.len() <= MAX_OUTPUT + 64, "{}", clamped.len());
    }

    /// `include` narrows a search to matching file names.
    #[test]
    fn grep_can_limit_itself_to_matching_files() {
        let dir = std::env::temp_dir().join(format!("exp1-grep-include-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).expect("temp dir");
        std::fs::write(dir.join("src/lib.rs"), "needle\n").expect("write");
        std::fs::write(dir.join("src/notes.md"), "needle\n").expect("write");

        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let outcome = runtime.block_on(grep(
            &dir,
            &json!({ "pattern": "needle", "include": "*.rs" }),
        ));
        assert!(outcome.output.contains("lib.rs:1: needle"), "{}", outcome.output);
        assert!(!outcome.output.contains("notes.md"), "{}", outcome.output);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Several replacements in one call, and nothing written when one of them
    /// cannot be made.
    #[test]
    fn an_edit_can_change_several_places_at_once() {
        let dir = std::env::temp_dir().join(format!("exp1-multi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("code.rs");
        std::fs::write(&file, "alpha\nbeta\ngamma\n").expect("write");

        let undo = Arc::new(StdMutex::new(UndoLog::default()));
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let outcome = runtime.block_on(edit(
            &dir,
            &undo,
            &json!({
                "path": "code.rs",
                "edits": [
                    { "old": "alpha", "new": "ALPHA" },
                    { "old": "gamma", "new": "GAMMA" }
                ]
            }),
        ));
        assert!(outcome.output.contains("2 changes"), "{}", outcome.output);
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "ALPHA\nbeta\nGAMMA\n");

        // One bad pair leaves the file exactly as it was.
        let outcome = runtime.block_on(edit(
            &dir,
            &undo,
            &json!({
                "path": "code.rs",
                "edits": [
                    { "old": "beta", "new": "BETA" },
                    { "old": "delta", "new": "DELTA" }
                ]
            }),
        ));
        assert!(outcome.output.contains("not found"), "{}", outcome.output);
        assert_eq!(
            std::fs::read_to_string(&file).expect("read"),
            "ALPHA\nbeta\nGAMMA\n",
            "nothing was written"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A failed edit says enough to fix it without reading the file again.
    #[test]
    fn an_edit_failure_points_at_the_real_text() {
        let dir = std::env::temp_dir().join(format!("exp1-hint-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("code.rs");
        std::fs::write(&file, "fn main() {\n    let total = 1;\n    let other = 2;\n}\n")
            .expect("write");

        let undo = Arc::new(StdMutex::new(UndoLog::default()));
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        // The model got the indentation of the second line wrong, so the
        // two-line snippet is not in the file.
        let outcome = runtime.block_on(edit(
            &dir,
            &undo,
            &json!({
                "path": "code.rs",
                "old": "    let total = 1;\n   let other = 2;",
                "new": "    let total = 3;"
            }),
        ));
        assert!(outcome.output.contains("closest lines"), "{}", outcome.output);
        assert!(outcome.output.contains("2 |     let total = 1;"), "{}", outcome.output);

        // Two candidates: the answer says where they are.
        std::fs::write(&file, "let x = 1;\nlet x = 1;\n").expect("write");
        let outcome = runtime.block_on(edit(
            &dir,
            &undo,
            &json!({ "path": "code.rs", "old": "let x = 1;", "new": "let x = 2;" }),
        ));
        assert!(outcome.output.contains("lines 1, 2"), "{}", outcome.output);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A grep answers with `path:line: text`, skips build output, and stops at
    /// `max` with a note on how to ask for more.
    #[test]
    fn grep_finds_matches_and_skips_build_dirs() {
        let dir = std::env::temp_dir().join(format!("exp1-grep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).expect("temp dir");
        std::fs::create_dir_all(dir.join("target")).expect("temp dir");
        std::fs::write(dir.join("src/lib.rs"), "fn alpha() {}\nfn beta() {}\n").expect("write");
        std::fs::write(dir.join("target/junk.rs"), "fn alpha() {}\n").expect("write");

        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let outcome = runtime.block_on(grep(&dir, &json!({ "pattern": "fn \\w+" })));
        assert!(outcome.output.contains("src/lib.rs:1: fn alpha() {}"), "{}", outcome.output);
        assert!(outcome.output.contains("src/lib.rs:2: fn beta() {}"), "{}", outcome.output);
        assert!(!outcome.output.contains("junk.rs"), "target must be skipped: {}", outcome.output);
        assert!(outcome.output.contains("2 match(es) in 1 file(s)"), "{}", outcome.output);

        // A cap stops the list and says how to continue.
        let outcome = runtime.block_on(grep(&dir, &json!({ "pattern": "fn", "max": 1 })));
        assert!(outcome.output.contains("stopped at 1 matches"), "{}", outcome.output);

        // A bad pattern is refused rather than treated as literal text.
        let outcome = runtime.block_on(grep(&dir, &json!({ "pattern": "fn (" })));
        assert!(outcome.output.contains("invalid pattern"), "{}", outcome.output);

        // Nothing found is still an answer, with the number of files searched.
        let outcome = runtime.block_on(grep(&dir, &json!({ "pattern": "nothing-here" })));
        assert!(outcome.output.contains("no matches"), "{}", outcome.output);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A tree that descends into `target` costs thousands of lines of context
    /// for nothing. Build output is named once, not walked.
    #[test]
    fn the_tree_names_build_output_without_walking_it() {
        let root = std::env::temp_dir().join(format!("exp1-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("temp dir");
        std::fs::create_dir_all(root.join("target/debug/deps")).expect("temp dir");
        std::fs::create_dir_all(root.join("node_modules/leftpad")).expect("temp dir");
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("write");
        for i in 0..40 {
            std::fs::write(root.join("target/debug/deps").join(format!("obj{i}.o")), "x")
                .expect("write");
            std::fs::write(root.join("node_modules/leftpad").join(format!("f{i}.js")), "x")
                .expect("write");
        }

        let mut budget = TREE_MAX_ENTRIES;
        let tree = build_tree(&root, &mut budget);
        assert!(tree.contains("target/ (not walked)"), "{tree}");
        assert!(tree.contains("node_modules/ (not walked)"), "{tree}");
        assert!(tree.contains("main.rs"), "{tree}");
        assert!(!tree.contains("obj0.o"), "build output was walked: {tree}");
        assert!(!tree.contains("f0.js"), "node_modules was walked: {tree}");
        assert!(tree.lines().count() < 20, "the tree stayed small: {tree}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Rewinding the log hands back the turns after a point, newest last, and
    /// keeps the ones before it.
    #[test]
    fn the_undo_log_can_be_rewound_to_a_point() {
        let dir = std::env::temp_dir().join(format!("exp1-rewind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut log = UndoLog::default();

        for turn in 0..3 {
            log.begin();
            let file = dir.join(format!("file-{turn}.txt"));
            std::fs::write(&file, format!("turn {turn}")).expect("write");
            log.record(&file);
        }
        assert_eq!(log.turns(), 3);
        // Back to the start of turn 1: turns 1 and 2 are handed back.
        let taken = log.rewind_to(1);
        assert_eq!(taken.len(), 2);
        assert_eq!(log.turns(), 1);
        assert_eq!(taken[0].len(), 1, "one file per turn");
        assert!(taken[0][0].0.ends_with("file-1.txt"));
        assert!(taken[1][0].0.ends_with("file-2.txt"));
        // Nothing left to go back to returns nothing.
        assert!(log.rewind_to(1).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Truncation lands on a character boundary: cutting Cyrillic at a byte
    /// offset used to panic in the middle of a letter.
    #[test]
    fn truncation_never_splits_a_character() {
        let text = "привет, мир! ".repeat(4000);
        assert!(text.len() > MAX_OUTPUT, "the sample must be over the cap");
        let clamped = clamp(&text, None);
        assert!(clamped.len() <= MAX_OUTPUT + 64, "the byte cap held: {}", clamped.len());
        assert!(clamped.starts_with("привет"), "and nothing was mangled");

        // The helper itself never returns a slice inside a character.
        for at in 0..text.len().min(200) {
            let piece = cut(&text, at);
            assert!(text.starts_with(piece));
        }
    }

    /// A read with no limit must not swallow a large file: it returns a window
    /// and says how to continue.
    #[test]
    fn a_read_without_a_limit_is_bounded() {
        let dir = std::env::temp_dir().join(format!("exp1-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("big.txt");
        let body: String = (1..=1000).map(|n| format!("line {n}\n")).collect();
        std::fs::write(&file, body).expect("write");

        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let outcome = runtime.block_on(read(&dir, &json!({ "path": "big.txt" })));
        assert!(outcome.output.contains("big.txt (1000 lines)"), "{}", &outcome.output[..80.min(outcome.output.len())]);
        assert!(outcome.output.contains("showing 400 of 1000"), "{}", outcome.output);
        assert!(outcome.output.contains("line 400"), "the window should end at line 400");
        assert!(!outcome.output.contains("line 401"), "and not go past it");
        assert!(outcome.output.contains("pos to continue"), "{}", outcome.output);

        // An explicit limit still wins.
        let outcome = runtime.block_on(read(&dir, &json!({ "path": "big.txt", "limit": 10 })));
        assert!(outcome.output.contains("showing 10 of 1000"), "{}", outcome.output);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The edit result carries the changed lines, so the model does not read
    /// the file back just to see what it did.
    #[test]
    fn an_edit_shows_the_changed_lines() {
        let dir = std::env::temp_dir().join(format!("exp1-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("code.rs");
        std::fs::write(&file, "one\ntwo\nthree\nfour\n").expect("write");

        let undo = Arc::new(StdMutex::new(UndoLog::default()));
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let outcome = runtime.block_on(edit(
            &dir,
            &undo,
            &json!({ "path": "code.rs", "old": "three", "new": "THREE" }),
        ));
        assert!(outcome.output.contains("THREE"), "{}", outcome.output);
        assert!(outcome.output.contains("3 | THREE"), "the window is numbered: {}", outcome.output);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn counts_lines_and_recounts_when_the_file_changes() {
        let dir = std::env::temp_dir().join(format!("exp1-count-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("lines.txt");

        // No trailing newline: the last line still counts.
        std::fs::write(&file, "one\ntwo\nthree").expect("write");
        assert_eq!(count_lines(&file), Some(3));
        // A trailing newline does not add an empty line.
        std::fs::write(&file, "one\ntwo\n").expect("write");
        assert_eq!(count_lines(&file), Some(2));
        std::fs::write(&file, "").expect("write");
        assert_eq!(count_lines(&file), Some(0));

        // The cache answers the second call, and a changed file is recounted.
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        std::fs::write(&file, "a\nb\n").expect("write");
        let first = runtime.block_on(count(&dir, &json!({ "path": "lines.txt" })));
        assert!(first.output.contains("2 lines"), "{}", first.output);
        assert!(!first.output.contains("from .cache"), "{}", first.output);
        let second = runtime.block_on(count(&dir, &json!({ "path": "lines.txt" })));
        assert!(second.output.contains("from .cache"), "{}", second.output);
        // A different mtime must invalidate it, not report the old number.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&file, "a\nb\nc\nd\n").expect("write");
        let third = runtime.block_on(count(&dir, &json!({ "path": "lines.txt" })));
        assert!(third.output.contains("4 lines"), "{}", third.output);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_removes_a_file_and_refuses_a_directory() {
        let dir = std::env::temp_dir().join(format!("exp1-delete-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("gone.txt");
        std::fs::write(&file, "bye").expect("write");

        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let undo = Arc::new(StdMutex::new(UndoLog::default()));
        let outcome = runtime.block_on(delete(&dir, &undo, &json!({ "path": "gone.txt" })));
        assert!(!file.exists(), "the file should be gone");
        assert!(outcome.output.contains("deleted"), "{}", outcome.output);

        // A directory is refused, and a missing file is reported by name.
        let outcome = runtime.block_on(delete(&dir, &undo, &json!({ "path": "." })));
        assert!(outcome.output.contains("directory"), "{}", outcome.output);
        let outcome = runtime.block_on(delete(&dir, &undo, &json!({ "path": "nope.txt" })));
        assert!(outcome.output.contains("nope.txt"), "{}", outcome.output);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_a_plain_json_reply() {
        let body = r#"{"result":{"content":[{"type":"text","text":"hello"}]}}"#;
        assert_eq!(extract_search_text(body), Some(Ok("hello".to_string())));
    }

    #[test]
    fn reads_an_sse_reply() {
        let body = "event: message\ndata: {\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"found it\"}]}}\n\n";
        assert_eq!(extract_search_text(body), Some(Ok("found it".to_string())));
    }

    #[test]
    fn surfaces_a_jsonrpc_error() {
        let body = r#"{"error":{"code":-32602,"message":"invalid arguments"}}"#;
        assert_eq!(
            extract_search_text(body),
            Some(Err("invalid arguments".to_string()))
        );
    }

    #[test]
    fn reads_duckduckgo_results() {
        let html = r##"<a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fratatui.rs%2F&amp;rut=x">Ratatui <b>Rust</b></a>
                      <a class="result__snippet" href="#">Cook up <b>delicious</b> TUIs in Rust &amp; more</a>"##;
        let results = parse_ddg(html, 5);
        assert_eq!(results.len(), 1);
        assert!(results[0].contains("https://ratatui.rs/"), "{}", results[0]);
        assert!(results[0].contains("Cook up delicious TUIs in Rust & more"), "{}", results[0]);
    }

    #[test]
    fn skips_content_without_text() {
        let body = r#"{"result":{"content":[{"type":"image","data":"zzz"},{"type":"text","text":"second"}]}}"#;
        assert_eq!(extract_search_text(body), Some(Ok("second".to_string())));
    }

    #[test]
    fn unreadable_bodies_yield_nothing() {
        assert_eq!(extract_search_text("<html>nope</html>"), None);
        assert_eq!(extract_search_text(""), None);
    }

    #[test]
    fn encodes_query_values() {
        assert_eq!(encode_query_value("abc-123_X.y~z"), "abc-123_X.y~z");
        assert_eq!(encode_query_value("a b&c=d"), "a%20b%26c%3Dd");
    }

    /// Each mode asks for exactly its own tools: `always ask` for everything
    /// that changes files and the shell, `never ask` for nothing, and
    /// `ask only commands` for `shell` alone. Read-only tools never ask.
    #[test]
    fn every_permission_mode_asks_for_its_own_tools() {
        for tool in ["write", "edit", "delete", "shell"] {
            assert!(PermissionMode::AlwaysAsk.asks(tool), "always ask / {tool}");
            assert!(!PermissionMode::NeverAsk.asks(tool), "never ask / {tool}");
        }
        for tool in ["read", "grep", "count", "ast", "fetch", "websearch"] {
            assert!(!PermissionMode::AlwaysAsk.asks(tool), "always ask / {tool}");
            assert!(!PermissionMode::NeverAsk.asks(tool), "never ask / {tool}");
            assert!(!PermissionMode::AskOnlyCommands.asks(tool), "ask only commands / {tool}");
        }
        assert!(PermissionMode::AskOnlyCommands.asks("shell"));
        for tool in ["write", "edit", "delete"] {
            assert!(!PermissionMode::AskOnlyCommands.asks(tool), "ask only commands / {tool}");
        }
    }

    /// The labels are the ones the footer shows and the READMEs spell out, and
    /// the cycle walks all three and comes back.
    #[test]
    fn the_permission_mode_labels_and_cycle_are_the_documented_ones() {
        assert_eq!(PermissionMode::AlwaysAsk.label(), "always ask");
        assert_eq!(PermissionMode::NeverAsk.label(), "never ask");
        assert_eq!(PermissionMode::AskOnlyCommands.label(), "ask only commands");
        assert_eq!(PermissionMode::AlwaysAsk.cycle(), PermissionMode::NeverAsk);
        assert_eq!(PermissionMode::NeverAsk.cycle(), PermissionMode::AskOnlyCommands);
        assert_eq!(PermissionMode::AskOnlyCommands.cycle(), PermissionMode::AlwaysAsk);
        // A fresh install starts on the always-ask default.
        assert_eq!(PermissionMode::default(), PermissionMode::AlwaysAsk);
    }

    /// A deny rule keeps refusing in every mode, `never ask` included; the
    /// mode only decides calls the rules leave open.
    #[test]
    fn a_deny_rule_denies_in_never_ask() {
        let rules: Rules = serde_json::from_str(
            r#"{"rules": [{"action": "write", "resource": "*", "effect": "deny"}]}"#,
        )
        .expect("rules");
        let decision = rules.decide("write", "/etc/hosts").map(|rule| rule.effect.as_str());
        assert_eq!(decision, Some("deny"));
        assert_eq!(PermissionMode::NeverAsk.verdict(decision, "write"), Verdict::Deny);
        assert_eq!(PermissionMode::AlwaysAsk.verdict(decision, "write"), Verdict::Deny);
        assert_eq!(PermissionMode::AskOnlyCommands.verdict(decision, "write"), Verdict::Deny);

        // With nothing denied, the mode decides on its own.
        assert_eq!(PermissionMode::AlwaysAsk.verdict(None, "write"), Verdict::Ask);
        assert_eq!(PermissionMode::NeverAsk.verdict(None, "write"), Verdict::Run);
        assert_eq!(PermissionMode::AskOnlyCommands.verdict(None, "shell"), Verdict::Ask);
        assert_eq!(PermissionMode::AskOnlyCommands.verdict(None, "write"), Verdict::Run);
        // An explicit `ask` rule keeps its own question.
        assert_eq!(PermissionMode::NeverAsk.verdict(Some("ask"), "fetch"), Verdict::Ask);
    }
}
