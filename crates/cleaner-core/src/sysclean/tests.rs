use super::*;
use crate::test_support::TempDir;
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp/cleaner-test-home"))
}

#[test]
fn catalog_ids_are_unique() {
    let targets = catalog(&home());
    let mut seen = HashSet::new();
    for target in &targets {
        assert!(
            seen.insert(target.id.clone()),
            "duplicate catalog id: {}",
            target.id
        );
    }
}

#[test]
fn catalog_is_not_empty_and_is_described() {
    let targets = catalog(&home());
    assert!(
        !targets.is_empty(),
        "the catalog for this platform has no entries"
    );
    for target in &targets {
        assert!(!target.label.is_empty(), "{} has no label", target.id);
        assert!(!target.detail.is_empty(), "{} has no detail", target.id);
    }
}

/// The whole safety model in one assertion: nothing in the catalog may point
/// outside the roots the executor is willing to delete under.
#[test]
fn every_catalog_path_is_inside_an_allowed_root() {
    let home = home();
    let roots = allowed_roots(&home);

    for target in catalog(&home) {
        // `Empty` only ever removes children, so the directory it names has to
        // be a legal container rather than a legal deletion target.
        let allowed: fn(&Path, &[PathBuf]) -> bool = match target.action {
            Action::Empty(_) => is_container_allowed,
            _ => is_path_allowed,
        };
        for path in target.action.paths() {
            assert!(
                allowed(path, &roots),
                "{} would delete {} which is outside every allowed root",
                target.id,
                path.display()
            );
        }
    }
}

/// A catalog entry must never be able to take out a whole home directory or a
/// system root, however it was written.
#[test]
fn no_catalog_path_is_a_bare_root() {
    let home = home();
    let roots = allowed_roots(&home);

    for target in catalog(&home) {
        if matches!(target.action, Action::Empty(_)) {
            continue;
        }
        for path in target.action.paths() {
            assert!(
                !roots.iter().any(|root| path == root),
                "{} targets the root {} itself",
                target.id,
                path.display()
            );
        }
    }
}

/// Pure caches are pre-ticked, so they get an extra guard: no `Safe` entry may
/// reach into a directory that holds real user work.
#[test]
fn safe_targets_never_touch_user_documents() {
    let home = home();
    let forbidden = [
        "Documents",
        "Desktop",
        "Pictures",
        "Movies",
        "Music",
        "Library/Keychains",
        "Library/Mail",
        ".ssh",
        ".gnupg",
    ];

    for target in catalog(&home) {
        if target.tier != Tier::Safe {
            continue;
        }
        for path in target.action.paths() {
            for name in forbidden {
                assert!(
                    !path.starts_with(home.join(name)),
                    "safe target {} reaches into {}",
                    target.id,
                    path.display()
                );
            }
        }
    }
}

#[test]
fn report_only_targets_are_never_selectable() {
    for target in catalog(&home()) {
        if matches!(target.action, Action::ReportOnly) {
            assert!(
                !target.selectable(),
                "{} should not be selectable",
                target.id
            );
            assert!(
                !target.default_marked(),
                "{} should not be marked",
                target.id
            );
        }
    }
}

#[test]
fn only_safe_targets_are_marked_by_default() {
    for target in catalog(&home()) {
        assert_eq!(
            target.default_marked(),
            target.tier == Tier::Safe,
            "{} has the wrong default",
            target.id
        );
    }
}

#[test]
fn path_allowance_rejects_roots_and_traversal() {
    let root = PathBuf::from("/tmp/cleaner-root");
    let roots = vec![root.clone()];

    assert!(is_path_allowed(&root.join("cache"), &roots));
    assert!(
        !is_path_allowed(&root, &roots),
        "the root itself is not a target"
    );
    assert!(!is_path_allowed(&PathBuf::from("/tmp/elsewhere"), &roots));
    assert!(!is_path_allowed(&root.join("../escape"), &roots));
}

#[test]
fn probe_measures_a_tree_and_drops_absent_targets() {
    let temp = TempDir::new("sysclean-probe");
    temp.write("cache/a.bin", &[7u8; 4096]);
    temp.write("cache/nested/b.bin", &[7u8; 8192]);

    let present = Target {
        id: "present".into(),
        group: Group::DevCache,
        label: "Present".into(),
        detail: "d".into(),
        tier: Tier::Safe,
        action: Action::Remove(vec![temp.join("cache")]),
        probe: vec![temp.join("cache")],
        requires: None,
    };
    let absent = Target {
        id: "absent".into(),
        group: Group::DevCache,
        label: "Absent".into(),
        detail: "d".into(),
        tier: Tier::Safe,
        action: Action::Remove(vec![temp.join("nope")]),
        probe: vec![temp.join("nope")],
        requires: None,
    };

    let progress = crate::tree::ScanProgress::new();
    let cancelled = Arc::new(AtomicBool::new(false));
    let candidates = probe(vec![present, absent], &progress, &cancelled);

    assert_eq!(candidates.len(), 1, "absent targets should be dropped");
    assert_eq!(candidates[0].target.id, "present");
    assert!(
        candidates[0].size >= 12288,
        "expected at least 12 KiB, got {}",
        candidates[0].size
    );
}

#[test]
fn probe_keeps_command_targets_with_nothing_to_measure() {
    let target = Target {
        id: "cmd".into(),
        group: Group::Container,
        label: "Prune".into(),
        detail: "d".into(),
        tier: Tier::Safe,
        action: Action::Command {
            program: "true".into(),
            args: vec![],
        },
        probe: vec![],
        requires: None,
    };

    let progress = crate::tree::ScanProgress::new();
    let cancelled = Arc::new(AtomicBool::new(false));
    let candidates = probe(vec![target], &progress, &cancelled);

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].size, 0);
}

#[test]
fn dry_run_reports_bytes_without_deleting() {
    let temp = TempDir::new("sysclean-dry");
    temp.write("cache/a.bin", &[1u8; 4096]);

    let target = Target {
        id: "dry".into(),
        group: Group::DevCache,
        label: "Dry".into(),
        detail: "d".into(),
        tier: Tier::Safe,
        action: Action::Remove(vec![temp.join("cache")]),
        probe: vec![temp.join("cache")],
        requires: None,
    };

    let sink = Arc::new(Mutex::new(Vec::new()));
    let report = run(vec![target], temp.path(), true, sink);

    assert_eq!(report.done.len(), 1);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(
        report.freed >= 4096,
        "expected bytes counted, got {}",
        report.freed
    );
    assert!(temp.join("cache/a.bin").exists(), "dry run must not delete");
}

#[test]
fn live_run_removes_paths_and_empty_keeps_the_directory() {
    let temp = TempDir::new("sysclean-live");
    temp.write("gone/a.bin", &[1u8; 2048]);
    temp.write("kept/a.bin", &[1u8; 2048]);
    temp.write("kept/nested/b.bin", &[1u8; 2048]);

    let remove = Target {
        id: "remove".into(),
        group: Group::DevCache,
        label: "Remove".into(),
        detail: "d".into(),
        tier: Tier::Safe,
        action: Action::Remove(vec![temp.join("gone")]),
        probe: vec![],
        requires: None,
    };
    let empty = Target {
        id: "empty".into(),
        group: Group::DevCache,
        label: "Empty".into(),
        detail: "d".into(),
        tier: Tier::Safe,
        action: Action::Empty(vec![temp.join("kept")]),
        probe: vec![],
        requires: None,
    };

    let sink = Arc::new(Mutex::new(Vec::new()));
    let report = run(vec![remove, empty], temp.path(), false, sink);

    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(
        !temp.join("gone").exists(),
        "Remove should delete the directory"
    );
    assert!(
        temp.join("kept").exists(),
        "Empty should keep the directory"
    );
    assert!(
        !temp.join("kept/a.bin").exists(),
        "Empty should clear contents"
    );
    assert!(
        !temp.join("kept/nested").exists(),
        "Empty should clear contents"
    );
}

/// Targets needing root are handed back, never attempted in-process.
#[test]
fn elevated_targets_are_deferred_not_executed() {
    let temp = TempDir::new("sysclean-root");
    temp.write("system/a.bin", &[1u8; 1024]);

    let target = Target {
        id: "root".into(),
        group: Group::SystemJunk,
        label: "Root".into(),
        detail: "d".into(),
        tier: Tier::NeedsRoot,
        action: Action::Remove(vec![temp.join("system")]),
        probe: vec![],
        requires: None,
    };

    let sink = Arc::new(Mutex::new(Vec::new()));
    let report = run(vec![target], temp.path(), false, sink);

    assert_eq!(report.deferred.len(), 1);
    assert!(report.done.is_empty());
    assert!(
        temp.join("system/a.bin").exists(),
        "a deferred target must not have been executed"
    );
}

#[test]
fn report_only_targets_are_refused() {
    let temp = TempDir::new("sysclean-report");
    temp.write("vm/disk.img", &[1u8; 1024]);

    let target = Target {
        id: "vm".into(),
        group: Group::Container,
        label: "VM".into(),
        detail: "d".into(),
        tier: Tier::Reclaimable,
        action: Action::ReportOnly,
        probe: vec![temp.join("vm")],
        requires: None,
    };

    let sink = Arc::new(Mutex::new(Vec::new()));
    let report = run(vec![target], temp.path(), false, sink);

    assert_eq!(report.failed.len(), 1);
    assert!(temp.join("vm/disk.img").exists());
}

/// A path outside the allowed roots is refused even when the caller asks for it.
#[test]
fn paths_outside_the_allowed_roots_are_refused() {
    let temp = TempDir::new("sysclean-escape");
    let outside = TempDir::new("sysclean-outside");
    outside.write("victim.bin", &[1u8; 1024]);

    let target = Target {
        id: "escape".into(),
        group: Group::DevCache,
        label: "Escape".into(),
        detail: "d".into(),
        tier: Tier::Safe,
        action: Action::Remove(vec![outside.join("victim.bin")]),
        probe: vec![],
        requires: None,
    };

    let sink = Arc::new(Mutex::new(Vec::new()));
    let report = run(vec![target], temp.path(), false, sink);

    assert_eq!(report.failed.len(), 1);
    assert!(report.failed[0].1.contains("outside the allowed roots"));
    assert!(
        outside.join("victim.bin").exists(),
        "refused path must survive"
    );
}

#[test]
fn elevation_preview_shows_the_exact_commands() {
    let target = Target {
        id: "logs".into(),
        group: Group::SystemJunk,
        label: "Logs".into(),
        detail: "d".into(),
        tier: Tier::NeedsRoot,
        action: Action::Empty(vec![PathBuf::from("/private/var/db/diagnostics")]),
        probe: vec![],
        requires: None,
    };

    let preview = elevate::preview(std::slice::from_ref(&target));
    assert_eq!(preview.len(), 1);
    assert!(
        preview[0].contains("/private/var/db/diagnostics"),
        "preview should name the path: {}",
        preview[0]
    );
    #[cfg(not(windows))]
    assert!(preview[0].starts_with("sudo "));
}

#[test]
fn deleter_sink_collects_errors_instead_of_printing() {
    let sink: crate::deleter::MessageSink = Arc::new(Mutex::new(Vec::new()));
    let stats = Arc::new(crate::stats::Stats::new());
    let deleter = crate::deleter::Deleter::with_sink(
        stats,
        false,
        false,
        Arc::clone(&crate::pool::SCAN_POOL),
        Arc::clone(&sink),
    );

    let (tx, rx) = crossbeam_channel::bounded(4);
    tx.send(crate::scanner::ScanResult {
        path: PathBuf::from("/definitely/not/a/real/path/xyzzy"),
        is_dir: false,
        size: 0,
    })
    .unwrap();
    drop(tx);
    deleter.process(rx);

    let collected = sink.lock().unwrap();
    assert!(
        !collected.is_empty(),
        "the failure should have been collected"
    );
}

/// Dump the real catalog for this machine.
///
/// Ignored by default because it walks the actual home directory; run it with
/// `cargo test -p cleaner-core -- --ignored --nocapture real_catalog` to check
/// the numbers against `du`.
#[test]
#[ignore = "manual: measures the real machine"]
fn real_catalog_probe_dump() {
    let home = dirs::home_dir().expect("home directory");
    let targets = catalog(&home);
    let total = targets.len();

    let progress = crate::tree::ScanProgress::new();
    let cancelled = Arc::new(AtomicBool::new(false));
    let candidates = probe(targets, &progress, &cancelled);

    let sum: u64 = candidates.iter().map(|c| c.size).sum();
    let safe: u64 = candidates
        .iter()
        .filter(|c| c.target.tier == Tier::Safe)
        .map(|c| c.size)
        .sum();

    println!("catalog: {total} entries, {} present", candidates.len());
    println!("total {sum} bytes, safe-tier {safe} bytes");
    for candidate in &candidates {
        println!(
            "{:>14}  {:<12} {:<38} {}",
            candidate.size,
            candidate.target.tier.label(),
            candidate.target.label,
            candidate.section()
        );
    }
}
