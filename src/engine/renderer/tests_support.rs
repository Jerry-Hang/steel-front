// Source-scanning tests must see ALL production code, not just the file they were written in.
//
// Before the module split these tests used `include_str!("renderer.rs")`, which happened to
// cover everything. Once `impl Renderer` is spread over `renderer/*.rs`, that scan silently
// covers less and less -- a judge that keeps saying OK while no longer looking at the code it
// claims to check (repo lesson 46). So: enumerate the subtree at run time, and **fail closed**
// when the scan surface looks wrong instead of passing on an empty string.
//
// Run-time (not `include_str!`) on purpose: the file list must follow the module split without
// anybody remembering to update it. Tests run with the crate root as the working directory.
//
// NOTE: comments and messages here are ASCII on purpose -- the CJK glyph gate cannot be
// rebuilt (see AGENTS.md), so new test scaffolding avoids introducing new CJK codepoints.

use std::path::{Path, PathBuf};

const ROOT_FILE: &str = "src/engine/renderer.rs";
const MODULE_DIR: &str = "src/engine/renderer";

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("source scan cannot read {}: {e}", path.display()))
}

/// Every production source of the renderer module: the root file plus its non-test children.
///
/// `tests_*.rs` are skipped because these scanners assert about production code (an assertion
/// that "no `.ok()` swallows a failure" would be satisfied by the test file itself otherwise).
pub fn renderer_production_sources() -> String {
    let root = Path::new(ROOT_FILE);
    assert!(
        root.is_file(),
        "source scan: {ROOT_FILE} not found -- tests must run from the crate root"
    );
    let mut out = read(root);

    // Cross-check against the module declarations in the root file: every declared sibling
    // module must either be collected as production code or exist as a `tests_*.rs` (test-only).
    // Without this, a split that adds a module the enumeration skips would shrink the scan
    // surface silently -- the judge would keep saying OK while no longer reading the code.
    let declared: Vec<String> = out
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            t.strip_prefix("mod ")
                .and_then(|r| r.strip_suffix(';'))
                .map(|name| name.trim().to_string())
        })
        .collect();
    assert!(
        declared.len() >= 3,
        "source scan: only {} `mod NAME;` declaration(s) parsed from {ROOT_FILE} -- parser is wrong",
        declared.len()
    );

    let dir = Path::new(MODULE_DIR);
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("source scan cannot list {}: {e}", dir.display()));
    let mut children: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "rs").unwrap_or(false))
        .filter(|p| {
            !p.file_name()
                .map(|n| n.to_string_lossy().starts_with("tests_"))
                .unwrap_or(false)
        })
        .collect();
    children.sort();
    for child in &children {
        out.push_str(&read(child));
    }

    for name in &declared {
        let prod = children
            .iter()
            .any(|p| p.file_stem().map(|s| s == name.as_str()).unwrap_or(false));
        let exists = dir.join(format!("{name}.rs")).is_file() || name == "tests_support";
        assert!(
            prod || exists,
            "source scan: `mod {name};` is declared in {ROOT_FILE} but its source was not collected \
             ({} production child module(s) collected) -- scan surface is wrong",
            children.len()
        );
    }

    // Fail closed: an empty or implausibly small scan surface means the enumeration broke,
    // not that the production code became clean.
    assert!(
        out.len() >= 100_000,
        "source scan: only {} bytes collected from {ROOT_FILE} + {} child module(s) -- scan surface is wrong",
        out.len(),
        children.len()
    );
    assert!(
        out.contains("impl Renderer"),
        "source scan: collected text has no `impl Renderer` -- scan surface is wrong"
    );
    out
}
