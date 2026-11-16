//! Every test a comment cites must be a test fn, so a rule that names its test
//! fails CI when the test is renamed or deleted. rustdoc can't check these:
//! `#[test]` fns are compiled out of the docs. See docs/comment-style.md.

use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn every_cited_test_exists() {
    let mut broken = Vec::new();
    for file in rust_files(Path::new("src")) {
        let text = fs::read_to_string(&file).unwrap();
        for (line_no, line) in text.lines().enumerate() {
            if !line.trim_start().starts_with("//") {
                continue;
            }
            for cited in cited_tests(line) {
                if !resolves(&text, cited) {
                    broken.push(format!("{}:{}: `{cited}`", file.display(), line_no + 1));
                }
            }
        }
    }
    assert!(broken.is_empty(), "no such test:\n{}", broken.join("\n"));
}

#[test]
fn the_check_finds_a_cited_test_and_rejects_a_missing_one() {
    let text = "/// (pinned by `tests::real`)\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn real() {}\n    fn helper() {}\n}\n";
    assert_eq!(cited_tests(text.lines().next().unwrap()), ["tests::real"]);
    assert!(resolves(text, "tests::real"));
    assert!(!resolves(text, "tests::helper"), "not a #[test]");
    assert!(!resolves(text, "tests::gone"));
}

#[test]
fn the_check_resolves_harness_tests() {
    let line = "/// (pinned by `harness::flags::version_flag_prints_the_package_version`)";
    let cited = cited_tests(line);
    assert_eq!(
        cited,
        ["harness::flags::version_flag_prints_the_package_version"]
    );
    assert!(resolves("", cited[0]));
    assert!(!resolves("", "harness::flags::gone"));
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files
}

/// Code spans like `` `tests::name` ``, `` `module::tests::name` `` or
/// `` `harness::module::name` ``.
fn cited_tests(line: &str) -> Vec<&str> {
    line.split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| {
            (span.contains("tests::") || span.starts_with("harness::"))
                && span
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
        })
        .collect()
}

/// `tests::name` is in `text`, `module::tests::name` in `src/module.rs`, and
/// `harness::module::name` in `tests/harness/module.rs`.
fn resolves(text: &str, cited: &str) -> bool {
    if let Some(rest) = cited.strip_prefix("harness::") {
        let Some((module, name)) = rest.rsplit_once("::") else {
            return false;
        };
        let file = Path::new("tests/harness")
            .join(module.replace("::", "/"))
            .with_extension("rs");
        return fs::read_to_string(file).is_ok_and(|text| is_test_fn(&text, name));
    }
    let (module, name) = cited.rsplit_once("tests::").unwrap();
    let module = module.trim_end_matches("::");
    if module.is_empty() {
        return is_test_fn(text, name);
    }
    let other = Path::new("src")
        .join(module.replace("::", "/"))
        .with_extension("rs");
    fs::read_to_string(other).is_ok_and(|text| is_test_fn(&text, name))
}

/// A `fn name(` (or `async fn name(`) whose attributes, above it, include
/// `#[test]` or `#[tokio::test]`.
fn is_test_fn(text: &str, name: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let signature = format!("fn {name}(");
    let async_signature = format!("async fn {name}(");
    lines.iter().enumerate().any(|(i, line)| {
        let line = line.trim_start();
        (line.starts_with(&signature) || line.starts_with(&async_signature))
            && lines[..i]
                .iter()
                .rev()
                .map(|l| l.trim())
                .take_while(|l| l.starts_with("#[") || l.starts_with("///"))
                .any(|l| l == "#[test]" || l == "#[tokio::test]")
    })
}
