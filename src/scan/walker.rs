use super::arena::{flags, TreeArena, NONE};
use super::{
    category_for_name, ErrorSample, ScanError, ScanProgress, ScanStats, ScanTree,
    ERROR_SAMPLE_LIMIT, TOP_FILES_LIMIT, TOP_FOLDERS_LIMIT,
};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::fs::{self, ReadDir};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Instant;

struct Frame {
    node: u32,
    path: PathBuf,
    read_dir: Option<ReadDir>,
    small_agg: u32,
}

/// Portable filesystem metadata accessors.
///
/// The scan needs the allocated (on-disk) size, the device, the inode and the
/// link count. Only Unix exposes all four directly; Windows reports none of
/// them through `std`, so it approximates allocated size with the logical size
/// and treats every file as having a single link (no dedup).
mod meta {
    use std::fs;

    #[cfg(unix)]
    pub fn blocks(metadata: &fs::Metadata) -> u64 {
        use std::os::unix::fs::MetadataExt;
        metadata.blocks().saturating_mul(512)
    }

    #[cfg(not(unix))]
    pub fn blocks(metadata: &fs::Metadata) -> u64 {
        // NTFS rounds up to cluster size, but exact cluster size is not
        // exposed portably; the logical size is a good-faith approximation.
        metadata.len()
    }

    #[cfg(unix)]
    pub fn device(metadata: &fs::Metadata) -> u64 {
        use std::os::unix::fs::MetadataExt;
        metadata.dev()
    }

    /// Windows has no portable device id; report one constant so the
    /// "crosses a mount point" check never fires there.
    #[cfg(not(unix))]
    pub fn device(_metadata: &fs::Metadata) -> u64 {
        0
    }

    #[cfg(unix)]
    pub fn hardlink_key(metadata: &fs::Metadata) -> Option<(u64, u64)> {
        use std::os::unix::fs::MetadataExt;
        (metadata.nlink() > 1).then(|| (metadata.dev(), metadata.ino()))
    }

    /// NTFS can hardlink, but `std` exposes no link count or file index, so
    /// dedup is skipped rather than guessed.
    #[cfg(not(unix))]
    pub fn hardlink_key(_metadata: &fs::Metadata) -> Option<(u64, u64)> {
        None
    }
}

fn blocks(metadata: &fs::Metadata) -> u64 {
    meta::blocks(metadata)
}

/// A directory earns a category tint only when one category clearly leads.
/// Below this share the folder stays neutral: a near-tie between, say, video
/// and documents would paint a confidently wrong colour.
const DOMINANT_MIN_SHARE_PCT: u128 = 55;

/// Set `dominant_category` on every directory from its descendants' sizes.
///
/// A directory's tally is the sum of its direct file categories plus each
/// subdirectory's contribution, where a subdirectory contributes as its own
/// dominant category (or nothing when it is neutral). That keeps the tally to
/// a nine-slot array per directory, held for one pass and then dropped,
/// instead of nine extra bytes on every node. `nodes` lists parents before
/// children, so iterating backwards means every child is resolved before its
/// parent reads it.
fn resolve_dominant_categories(arena: &mut TreeArena) {
    let len = arena.nodes.len();
    let mut tallies: Vec<[u64; 9]> = vec![[0; 9]; len];

    // Nodes are appended parents-first, so descending indices visit every child
    // before its parent. Each child reports one contribution upward: files as
    // their own category, folders as their resolved dominant category. A
    // folder that has no clear majority stays neutral and contributes nothing.
    for id in (0..len).rev() {
        let node = &arena.nodes[id];
        let parent = node.parent;
        if parent == NONE {
            continue;
        }
        let parent = parent as usize;

        if !node.is_dir() {
            let category = (node.category() as usize).min(8);
            tallies[parent][category] = tallies[parent][category].saturating_add(node.alloc);
            continue;
        }

        if let Some(index) = dominant_index(&tallies[id]) {
            tallies[parent][index] = tallies[parent][index].saturating_add(node.alloc);
        }
    }

    for (node, tally) in arena.nodes.iter_mut().zip(tallies.iter()) {
        if !node.is_dir() || node.is_small_agg() {
            continue;
        }
        let value = dominant_index(tally)
            .map(|index| index as u8)
            .unwrap_or(flags::DOMINANT_NONE);
        node.set_dominant_category(value);
    }
}

/// Index of the category holding at least `DOMINANT_MIN_SHARE_PCT` of the
/// bytes, or `None` when no category leads clearly or nothing was recorded.
fn dominant_index(tally: &[u64; 9]) -> Option<usize> {
    let total: u64 = tally.iter().sum();
    if total == 0 {
        return None;
    }
    let (index, &best) = tally.iter().enumerate().max_by_key(|(_, &size)| size)?;
    ((best as u128) * 100 >= (total as u128) * DOMINANT_MIN_SHARE_PCT).then_some(index)
}

fn keep_top(heap: &mut BinaryHeap<Reverse<(u64, u32)>>, size: u64, id: u32, limit: usize) {
    if heap.len() < limit {
        heap.push(Reverse((size, id)));
    } else if let Some(Reverse((smallest, _))) = heap.peek() {
        if size > *smallest {
            heap.pop();
            heap.push(Reverse((size, id)));
        }
    }
}

fn record_error(
    errors: &mut Vec<ErrorSample>,
    count: &mut u64,
    parent: u32,
    name: &str,
    error: &std::io::Error,
) {
    *count += 1;
    if errors.len() < ERROR_SAMPLE_LIMIT {
        errors.push(ErrorSample {
            parent,
            name: name.to_string(),
            message: error.to_string(),
        });
    }
}

pub fn scan(
    root: &Path,
    small_file_threshold: u64,
    progress: &ScanProgress,
) -> Result<ScanTree, ScanError> {
    let started = Instant::now();
    let root_metadata = fs::symlink_metadata(root).map_err(|error| ScanError::Io {
        path: root.to_path_buf(),
        message: error.to_string(),
    })?;
    if !root_metadata.is_dir() {
        return Err(ScanError::NotDirectory(root.to_path_buf()));
    }

    let root_dev = meta::device(&root_metadata);
    let root_name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());

    let mut arena = TreeArena::new();
    let root_id = arena.add_node(
        NONE,
        &root_name,
        flags::ROOT | flags::DIR,
        blocks(&root_metadata),
        root_metadata.len(),
    );

    let mut hardlink_set: HashSet<(u64, u64)> = HashSet::new();
    let mut top_files: BinaryHeap<Reverse<(u64, u32)>> = BinaryHeap::new();
    let mut errors = Vec::new();
    let mut error_count = 0u64;
    let mut hardlinks_skipped = 0u64;
    let mut file_count = 0u64;
    let mut dir_count = 1u64;
    let mut entries_seen = 0u64;

    progress.set_current(root);
    let mut stack = vec![Frame {
        node: root_id,
        path: root.to_path_buf(),
        read_dir: None,
        small_agg: NONE,
    }];

    while !stack.is_empty() {
        if progress.is_cancelled() {
            return Err(ScanError::Cancelled);
        }

        let mut finished = false;
        let mut next_frame = None;

        {
            let frame = stack.last_mut().expect("stack is not empty");

            if frame.read_dir.is_none() {
                match fs::read_dir(&frame.path) {
                    Ok(entries) => {
                        progress.set_current(&frame.path);
                        frame.read_dir = Some(entries);
                    }
                    Err(error) => {
                        record_error(&mut errors, &mut error_count, frame.node, "", &error);
                        finished = true;
                    }
                }
            }

            if !finished {
                let entry: Option<std::io::Result<fs::DirEntry>> = {
                    let iter = frame.read_dir.as_mut().expect("read_dir exists");
                    loop {
                        match iter.next() {
                            Some(Ok(entry)) => break Some(Ok(entry)),
                            Some(Err(error)) => {
                                record_error(&mut errors, &mut error_count, frame.node, "", &error);
                            }
                            None => break None,
                        }
                    }
                };

                match entry {
                    None => finished = true,
                    Some(Ok(entry)) => {
                        entries_seen += 1;
                        progress.entries.fetch_add(1, Ordering::Relaxed);
                        let name = entry.file_name().to_string_lossy().into_owned();

                        match entry.metadata() {
                            Err(error) => {
                                record_error(
                                    &mut errors,
                                    &mut error_count,
                                    frame.node,
                                    &name,
                                    &error,
                                );
                            }
                            Ok(metadata) => {
                                let is_symlink = metadata.file_type().is_symlink();
                                if metadata.is_dir() && !is_symlink {
                                    dir_count += 1;
                                    progress.dirs.fetch_add(1, Ordering::Relaxed);
                                    let mut node_flags = flags::DIR;
                                    if meta::device(&metadata) != root_dev {
                                        node_flags |= flags::OTHER_FS;
                                    }
                                    let child = arena.add_node(
                                        frame.node,
                                        &name,
                                        node_flags,
                                        blocks(&metadata),
                                        metadata.len(),
                                    );
                                    if meta::device(&metadata) == root_dev {
                                        next_frame = Some(Frame {
                                            node: child,
                                            path: entry.path(),
                                            read_dir: None,
                                            small_agg: NONE,
                                        });
                                    }
                                } else {
                                    let mut skip = false;
                                    if let Some(key) =
                                        meta::hardlink_key(&metadata).filter(|_| !is_symlink)
                                    {
                                        if !hardlink_set.insert(key) {
                                            hardlinks_skipped += 1;
                                            skip = true;
                                        }
                                    }

                                    if !skip {
                                        let alloc = blocks(&metadata);
                                        let logical = metadata.len();
                                        file_count += 1;
                                        progress.bytes.fetch_add(alloc, Ordering::Relaxed);

                                        if logical >= small_file_threshold
                                            || alloc >= small_file_threshold
                                        {
                                            let category = category_for_name(&name) as u16;
                                            let mut node_flags =
                                                (category << flags::CAT_SHIFT) & flags::CAT_MASK;
                                            if is_symlink {
                                                node_flags |= flags::SYMLINK;
                                            }
                                            let child = arena.add_node(
                                                frame.node, &name, node_flags, alloc, logical,
                                            );
                                            arena.nodes[child as usize].files = 1;
                                            keep_top(&mut top_files, alloc, child, TOP_FILES_LIMIT);
                                        } else {
                                            if frame.small_agg == NONE {
                                                let aggregate = arena.add_node(
                                                    frame.node,
                                                    "small files",
                                                    flags::SMALL_AGG,
                                                    0,
                                                    0,
                                                );
                                                frame.small_agg = aggregate;
                                            }
                                            let aggregate =
                                                &mut arena.nodes[frame.small_agg as usize];
                                            aggregate.alloc = aggregate.alloc.saturating_add(alloc);
                                            aggregate.logical =
                                                aggregate.logical.saturating_add(logical);
                                            aggregate.files = aggregate.files.saturating_add(1);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Some(Err(_)) => unreachable!("entry errors are drained above"),
                }
            }
        }

        if finished {
            stack.pop();
        }
        if let Some(frame) = next_frame {
            stack.push(frame);
        }
        if entries_seen.is_multiple_of(256) && progress.is_cancelled() {
            return Err(ScanError::Cancelled);
        }
    }

    for id in (1..arena.nodes.len()).rev() {
        let (parent, alloc, logical, files) = {
            let node = &arena.nodes[id];
            (node.parent, node.alloc, node.logical, node.files)
        };
        if parent != NONE {
            let parent_node = &mut arena.nodes[parent as usize];
            parent_node.alloc = parent_node.alloc.saturating_add(alloc);
            parent_node.logical = parent_node.logical.saturating_add(logical);
            parent_node.files = parent_node.files.saturating_add(files);
        }
    }

    // Record each directory's dominant content category so the treemap can
    // tint folders instead of drawing them all the same grey. Children come
    // after their parent in `nodes`, so walking backwards settles a child's
    // dominance before the parent reads it.
    resolve_dominant_categories(&mut arena);

    let mut folder_heap: BinaryHeap<Reverse<(u64, u32)>> = BinaryHeap::new();
    for (id, node) in arena.nodes.iter().enumerate().skip(1) {
        if node.is_dir() && !node.is_small_agg() {
            keep_top(&mut folder_heap, node.alloc, id as u32, TOP_FOLDERS_LIMIT);
        }
    }

    let mut top_folders: Vec<(u64, u32)> = folder_heap
        .into_iter()
        .map(|Reverse(entry)| entry)
        .collect();
    top_folders.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let top_folders = top_folders.into_iter().map(|(_, id)| id).collect();

    let mut top_files: Vec<(u64, u32)> =
        top_files.into_iter().map(|Reverse(entry)| entry).collect();
    top_files.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let top_files = top_files.into_iter().map(|(_, id)| id).collect();

    let stats = ScanStats {
        allocated: arena.nodes[root_id as usize].alloc,
        logical: arena.nodes[root_id as usize].logical,
        files: file_count,
        dirs: dir_count,
        errors: error_count,
        hardlinks_skipped,
        elapsed_secs: started.elapsed().as_secs_f64(),
    };

    Ok(ScanTree {
        arena,
        root: root.to_path_buf(),
        stats,
        errors,
        top_files,
        top_folders,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{Category, SMALL_FILE_THRESHOLD};
    use std::fs;
    #[cfg(unix)]
    use std::fs::File;
    #[cfg(unix)]
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn temp_root(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("spacebox-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn progress() -> ScanProgress {
        ScanProgress::new()
    }

    #[test]
    fn dominant_category_follows_the_bulk_of_a_folder() {
        let root = temp_root("dominant");
        let video_dir = root.join("media");
        let mixed = root.join("mixed");
        fs::create_dir_all(&video_dir).unwrap();
        fs::create_dir_all(&mixed).unwrap();
        fs::write(video_dir.join("a.mkv"), vec![1u8; 2 * 1024 * 1024]).unwrap();
        fs::write(video_dir.join("b.mp4"), vec![1u8; 2 * 1024 * 1024]).unwrap();
        // 50/50 split: no category clearly leads, so the folder stays neutral.
        fs::write(mixed.join("c.mkv"), vec![1u8; 2 * 1024 * 1024]).unwrap();
        fs::write(mixed.join("d.txt"), vec![1u8; 2 * 1024 * 1024]).unwrap();

        let tree = scan(&root, SMALL_FILE_THRESHOLD, &progress()).unwrap();
        let media = tree
            .children(tree.root_id())
            .find(|id| tree.name(*id) == "media")
            .expect("media dir");
        let mixed_id = tree
            .children(tree.root_id())
            .find(|id| tree.name(*id) == "mixed")
            .expect("mixed dir");

        assert_eq!(tree.dominant_category(media), Some(Category::Video));
        assert_eq!(tree.dominant_category(mixed_id), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn nested_folder_inherits_descendant_category() {
        let root = temp_root("nested-dominant");
        let nested = root.join("outer").join("inner");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("song.flac"), vec![1u8; 4 * 1024 * 1024]).unwrap();

        let tree = scan(&root, SMALL_FILE_THRESHOLD, &progress()).unwrap();
        let outer = tree
            .children(tree.root_id())
            .find(|id| tree.name(*id) == "outer")
            .expect("outer dir");

        assert_eq!(tree.dominant_category(outer), Some(Category::Audio));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn scans_totals_hardlinks_and_small_aggregate() {
        let root = temp_root("totals");
        let sub = root.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(root.join("large.bin"), vec![7u8; 2 * 1024 * 1024]).unwrap();
        for index in 0..10 {
            fs::write(root.join(format!("small-{index}.txt")), b"hello").unwrap();
        }
        fs::hard_link(root.join("large.bin"), sub.join("hardlink.bin")).unwrap();

        let tree = scan(&root, SMALL_FILE_THRESHOLD, &progress()).unwrap();
        assert!(tree.stats.files >= 11);
        assert_eq!(tree.stats.hardlinks_skipped, 1);
        assert!(tree.stats.logical < 4 * 1024 * 1024);
        assert!(tree.stats.allocated >= 2 * 1024 * 1024);

        let aggregate = tree
            .children(tree.root_id())
            .find(|id| tree.is_small_agg(*id))
            .expect("small aggregate exists");
        assert_eq!(tree.node(aggregate).files, 10);
        assert!(tree.node(aggregate).logical >= 50);
        assert!(tree.arena.memory_bytes() > 0);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(unix)] // symlinks, sparse allocation and mode bits are Unix concepts
    fn symlink_is_not_followed() {
        let root = temp_root("symlink-root");
        let outside = temp_root("symlink-outside");
        fs::write(outside.join("big.bin"), vec![1u8; 5 * 1024 * 1024]).unwrap();
        symlink(outside.join("big.bin"), root.join("link.bin")).unwrap();
        symlink(&outside, root.join("link-dir")).unwrap();

        let tree = scan(&root, SMALL_FILE_THRESHOLD, &progress()).unwrap();
        assert!(tree.stats.logical < 3 * 1024 * 1024);
        assert_eq!(tree.stats.files, 2);

        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    #[cfg(unix)] // symlinks, sparse allocation and mode bits are Unix concepts
    fn sparse_file_reports_allocated_and_logical_separately() {
        let root = temp_root("sparse");
        let file = File::create(root.join("sparse.img")).unwrap();
        file.set_len(1024 * 1024 * 1024).unwrap();

        let tree = scan(&root, SMALL_FILE_THRESHOLD, &progress()).unwrap();
        assert!(tree.stats.logical >= 1024 * 1024 * 1024);
        assert!(tree.stats.allocated < 64 * 1024 * 1024);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(unix)] // symlinks, sparse allocation and mode bits are Unix concepts
    fn records_permission_errors() {
        let root = temp_root("permissions");
        let denied = root.join("denied");
        fs::create_dir(&denied).unwrap();
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o000)).unwrap();

        let tree = scan(&root, SMALL_FILE_THRESHOLD, &progress()).unwrap();
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(tree.stats.errors >= 1);
        assert!(!tree.errors.is_empty());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn respects_cancellation() {
        let root = temp_root("cancel");
        fs::write(root.join("file.bin"), b"data").unwrap();
        let progress = progress();
        progress.request_cancel();
        let result = scan(&root, SMALL_FILE_THRESHOLD, &progress);
        assert!(matches!(result, Err(ScanError::Cancelled)));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reconstructs_paths() {
        let root = temp_root("paths");
        let deep = root.join("a/b");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("big.bin"), vec![2u8; 2 * 1024 * 1024]).unwrap();

        let tree = scan(&root, SMALL_FILE_THRESHOLD, &progress()).unwrap();
        let file = tree
            .arena
            .nodes
            .iter()
            .enumerate()
            .find(|(_, node)| !node.is_dir() && !node.is_small_agg())
            .map(|(id, _)| id as u32)
            .unwrap();
        assert_eq!(tree.path_of(file), deep.join("big.bin"));
        assert_eq!(tree.relative_path_of(file), "a/b/big.bin");
        assert_eq!(tree.segments(file).len(), 4);

        fs::remove_dir_all(&root).unwrap();
    }
}
