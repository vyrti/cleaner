use super::entry::DirEntry;
use super::progress::ScanProgress;
use super::sizing::apply_directory_sizes;
use super::DirTree;
use crate::fastwalk;
use crate::patterns::PatternMatcher;
use crate::pool::SCAN_POOL;
use crate::protected::protected_paths_for_root;
use foldhash::{HashMap, HashMapExt};
use rayon::prelude::*;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

impl DirTree {
    /// Build tree with SINGLE WalkDir pass - maximum performance
    pub fn build_with_progress(
        root: &Path,
        matcher: &PatternMatcher,
        progress: Arc<ScanProgress>,
        cancelled: Arc<AtomicBool>,
        force: bool,
    ) -> Self {
        #[cfg(test)]
        let profile_started = std::time::Instant::now();

        #[cfg(target_os = "macos")]
        let docker_path: Option<PathBuf> = {
            if let Some(home) = std::env::var_os("HOME") {
                let docker_container =
                    PathBuf::from(home).join("Library/Containers/com.docker.docker");
                if docker_container.exists() {
                    Some(docker_container)
                } else {
                    None
                }
            } else {
                None
            }
        };
        #[cfg(not(target_os = "macos"))]
        let docker_path: Option<PathBuf> = None;

        #[cfg(target_os = "macos")]
        let root_clone = root.to_path_buf();

        // Protected directories (NEVER auto-clean inside these, but allow scanning and manual TUI deletion)
        let protected_paths = protected_paths_for_root(root, force);

        let skip_check = Arc::new(move |path: &Path| -> bool {
            if let Some(ref docker) = docker_path {
                if path.starts_with(docker) {
                    return true;
                }
            }
            #[cfg(target_os = "macos")]
            {
                if (path.starts_with("/System/Volumes") || path == Path::new("/System/Volumes"))
                    && !root_clone.starts_with("/System/Volumes")
                {
                    return true;
                }
                if (path.starts_with("/Volumes") || path == Path::new("/Volumes"))
                    && !root_clone.starts_with("/Volumes")
                {
                    return true;
                }
            }
            false
        });

        let progress_clone = Arc::clone(&progress);
        let progress_cb = Arc::new(move |dirs: usize, files: usize, bytes: u64| {
            progress_clone.dirs.fetch_add(dirs, Ordering::Relaxed);
            progress_clone.files.fetch_add(files, Ordering::Relaxed);
            progress_clone.bytes.fetch_add(bytes, Ordering::Relaxed);
        });

        // Walk and index each directory in one pass so RawEntry collections are
        // consumed before the next directory is retained.
        let walk = fastwalk::walk_parallel_mapped(
            root.to_path_buf(),
            &SCAN_POOL,
            skip_check,
            Some(progress_cb),
            &|dir_path, entries| {
                let dir_is_protected = protected_paths
                    .iter()
                    .any(|protected| dir_path.starts_with(protected));
                Arc::new(
                    entries
                        .into_iter()
                        .filter(|entry| !entry.is_symlink)
                        .map(|entry| {
                            let entry_is_protected = dir_is_protected
                                || (entry.is_dir
                                    && protected_paths.iter().any(|protected| {
                                        dir_path.join(&entry.name).starts_with(protected)
                                    }));
                            let is_temp = if entry_is_protected {
                                false
                            } else if entry.is_dir {
                                matcher.is_temp_directory(&entry.name)
                            } else {
                                matcher.is_temp_file(&entry.name)
                            };
                            DirEntry {
                                name: entry.name,
                                size: entry.size,
                                is_dir: entry.is_dir,
                                is_temp,
                            }
                        })
                        .collect(),
                )
            },
        );
        #[cfg(test)]
        let scan_elapsed = profile_started.elapsed();
        progress.errors.fetch_add(walk.errors, Ordering::Relaxed);
        let mut children: HashMap<PathBuf, Arc<Vec<DirEntry>>> = walk.entries;

        if cancelled.load(Ordering::Relaxed) {
            progress.done.store(true, Ordering::Release);
            return Self::from_shared_children(HashMap::new());
        }

        progress.begin_stage(1, children.len());
        progress
            .stage_current
            .store(children.len(), Ordering::Relaxed);

        if cancelled.load(Ordering::Relaxed) {
            progress.done.store(true, Ordering::Release);
            return Self::from_shared_children(children);
        }

        // 3. Compute sizes in place.
        progress.begin_stage(2, children.len());
        apply_directory_sizes(root, &mut children, &progress, &cancelled);
        #[cfg(test)]
        let sizing_elapsed = profile_started.elapsed().saturating_sub(scan_elapsed);

        if cancelled.load(Ordering::Relaxed) {
            progress.done.store(true, Ordering::Release);
            return Self::from_shared_children(children);
        }

        // Add navigation in parallel. Entry sorting is deferred until a
        // directory is opened, avoiding work for directories never viewed.
        progress.begin_stage(3, children.len());
        let root_clone = root.to_path_buf();
        children.par_iter_mut().for_each(|(dir_path, entries)| {
            if cancelled.load(Ordering::Relaxed) {
                return;
            }
            // Add navigation
            if dir_path != &root_clone && dir_path.parent().is_some() {
                Arc::make_mut(entries).insert(
                    0,
                    DirEntry {
                        name: OsString::from(".."),
                        size: 0,
                        is_dir: true,
                        is_temp: false,
                    },
                );
            }
        });
        progress
            .stage_current
            .store(children.len(), Ordering::Relaxed);

        progress.done.store(true, Ordering::Release);
        #[cfg(test)]
        if std::env::var_os("CLEANER_PROFILE_ROOT").is_some() {
            println!(
                "tui phases: scan/index={scan_elapsed:?} sizing={sizing_elapsed:?} finalizing={:?}",
                profile_started
                    .elapsed()
                    .saturating_sub(scan_elapsed)
                    .saturating_sub(sizing_elapsed)
            );
        }
        Self::from_shared_children(children)
    }
}
