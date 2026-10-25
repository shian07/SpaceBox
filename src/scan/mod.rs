pub mod arena;
pub mod walker;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

pub use arena::{Node, TreeArena};
pub use walker::scan;

pub const SMALL_FILE_THRESHOLD: u64 = 1024 * 1024;
pub const TOP_FILES_LIMIT: usize = 200;
pub const TOP_FOLDERS_LIMIT: usize = 200;
pub const ERROR_SAMPLE_LIMIT: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Category {
    Other = 0,
    Video = 1,
    Image = 2,
    Audio = 3,
    Archive = 4,
    Code = 5,
    Document = 6,
    Data = 7,
    System = 8,
}

impl Category {
    pub const ALL: [Category; 9] = [
        Category::Video,
        Category::Image,
        Category::Audio,
        Category::Archive,
        Category::Code,
        Category::Document,
        Category::Data,
        Category::System,
        Category::Other,
    ];

    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Category::Video,
            2 => Category::Image,
            3 => Category::Audio,
            4 => Category::Archive,
            5 => Category::Code,
            6 => Category::Document,
            7 => Category::Data,
            8 => Category::System,
            _ => Category::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::Video => "Video",
            Category::Image => "Images",
            Category::Audio => "Audio",
            Category::Archive => "Archives",
            Category::Code => "Code",
            Category::Document => "Documents",
            Category::Data => "Data",
            Category::System => "System",
            Category::Other => "Other",
        }
    }
}

fn ext_is(ext: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| ext.eq_ignore_ascii_case(candidate))
}

pub fn category_for_name(name: &str) -> Category {
    let Some((_, ext)) = name.rsplit_once('.') else {
        return Category::Other;
    };

    if ext_is(
        ext,
        &[
            "mp4", "mkv", "avi", "mov", "webm", "m4v", "flv", "wmv", "mpg", "mpeg", "ts",
        ],
    ) {
        Category::Video
    } else if ext_is(
        ext,
        &[
            "jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "tiff", "tif", "heic", "raw", "dng",
            "psd", "xcf",
        ],
    ) {
        Category::Image
    } else if ext_is(
        ext,
        &[
            "mp3", "flac", "wav", "ogg", "m4a", "aac", "opus", "wma", "aiff",
        ],
    ) {
        Category::Audio
    } else if ext_is(
        ext,
        &[
            "zip", "tar", "gz", "bz2", "xz", "zst", "7z", "rar", "iso", "deb", "rpm", "pkg",
            "appimage", "jar",
        ],
    ) {
        Category::Archive
    } else if ext_is(
        ext,
        &[
            "rs", "c", "cc", "cpp", "h", "hpp", "py", "js", "mjs", "ts", "tsx", "jsx", "go",
            "java", "kt", "swift", "rb", "php", "sh", "bash", "zsh", "fish", "html", "css", "scss",
            "vue", "svelte", "lua", "pl", "r",
        ],
    ) {
        Category::Code
    } else if ext_is(
        ext,
        &[
            "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "odt", "ods", "odp",
            "epub", "rtf",
        ],
    ) {
        Category::Document
    } else if ext_is(
        ext,
        &[
            "json", "xml", "csv", "yml", "yaml", "toml", "sqlite", "db", "sql", "log", "dat",
            "bin", "ini", "cfg", "conf",
        ],
    ) {
        Category::Data
    } else if ext_is(
        ext,
        &[
            "so", "dylib", "dll", "a", "o", "lib", "ko", "exe", "sys", "dylib", "node",
        ],
    ) {
        Category::System
    } else {
        Category::Other
    }
}

#[derive(Debug, Clone)]
pub enum ScanError {
    Cancelled,
    NotDirectory(PathBuf),
    Io { path: PathBuf, message: String },
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanError::Cancelled => write!(f, "Scan cancelled"),
            ScanError::NotDirectory(path) => {
                write!(f, "{} is not a directory", path.display())
            }
            ScanError::Io { path, message } => {
                write!(f, "{}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for ScanError {}

#[derive(Debug, Default, Clone, Copy)]
pub struct ScanStats {
    pub allocated: u64,
    pub logical: u64,
    pub files: u64,
    pub dirs: u64,
    pub errors: u64,
    pub hardlinks_skipped: u64,
    pub elapsed_secs: f64,
}

#[derive(Debug, Clone)]
pub struct ErrorSample {
    pub parent: u32,
    pub name: String,
    pub message: String,
}

pub struct ProgressSnapshot {
    pub entries: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub current: String,
}

pub struct ScanProgress {
    pub entries: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub cancel: AtomicBool,
    current: Mutex<String>,
}

impl Default for ScanProgress {
    fn default() -> Self {
        Self {
            entries: AtomicU64::new(0),
            dirs: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            cancel: AtomicBool::new(false),
            current: Mutex::new(String::new()),
        }
    }
}

impl ScanProgress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn set_current(&self, path: &Path) {
        if let Ok(mut current) = self.current.lock() {
            current.clear();
            current.push_str(&path.to_string_lossy());
        }
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        let current = self
            .current
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default();
        ProgressSnapshot {
            entries: self.entries.load(Ordering::Relaxed),
            dirs: self.dirs.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            current,
        }
    }
}

pub struct ScanTree {
    pub arena: TreeArena,
    pub root: PathBuf,
    pub stats: ScanStats,
    pub errors: Vec<ErrorSample>,
    pub top_files: Vec<u32>,
    pub top_folders: Vec<u32>,
}

impl ScanTree {
    #[cfg(test)]
    pub fn root_id(&self) -> u32 {
        0
    }

    pub fn name(&self, id: u32) -> &str {
        self.arena.name(id)
    }

    pub fn node(&self, id: u32) -> &Node {
        &self.arena.nodes[id as usize]
    }

    pub fn children(&self, id: u32) -> impl Iterator<Item = u32> + '_ {
        self.arena.children(id)
    }

    pub fn is_dir(&self, id: u32) -> bool {
        self.node(id).is_dir()
    }

    pub fn is_small_agg(&self, id: u32) -> bool {
        self.node(id).is_small_agg()
    }

    pub fn is_other_fs(&self, id: u32) -> bool {
        self.node(id).is_other_fs()
    }

    pub fn is_symlink(&self, id: u32) -> bool {
        self.node(id).is_symlink()
    }

    pub fn category(&self, id: u32) -> Category {
        Category::from_u8(self.node(id).category())
    }

    /// The category holding most of a directory's bytes, when one clearly leads.
    /// Files return their own category.
    pub fn dominant_category(&self, id: u32) -> Option<Category> {
        let node = self.node(id);
        if node.is_dir() {
            node.dominant_category().map(Category::from_u8)
        } else {
            Some(Category::from_u8(node.category()))
        }
    }

    pub fn size(&self, id: u32, use_alloc: bool) -> u64 {
        let node = self.node(id);
        if use_alloc {
            node.alloc
        } else {
            node.logical
        }
    }

    pub fn parent(&self, id: u32) -> Option<u32> {
        self.arena.parent(id)
    }

    pub fn path_of(&self, id: u32) -> PathBuf {
        let ids = self.arena.ancestor_ids(id);
        let mut path = self.root.clone();
        for ancestor in ids.into_iter().skip(1) {
            path.push(self.name(ancestor));
        }
        path
    }

    pub fn relative_path_of(&self, id: u32) -> String {
        let ids = self.arena.ancestor_ids(id);
        let mut out = String::new();
        for (index, ancestor) in ids.into_iter().skip(1).enumerate() {
            if index > 0 {
                out.push('/');
            }
            out.push_str(self.name(ancestor));
        }
        out
    }

    pub fn segments(&self, id: u32) -> Vec<(u32, String)> {
        self.arena
            .ancestor_ids(id)
            .into_iter()
            .map(|ancestor| (ancestor, self.name(ancestor).to_string()))
            .collect()
    }

    pub fn contains_ci(name: &str, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        let hay = name.as_bytes();
        let needle = needle.as_bytes();
        if needle.len() > hay.len() {
            return false;
        }
        hay.windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<u32> {
        if query.is_empty() {
            return Vec::new();
        }
        let mut found = Vec::new();
        for (id, node) in self.arena.nodes.iter().enumerate() {
            if id == 0 || node.is_small_agg() {
                continue;
            }
            if Self::contains_ci(self.name(id as u32), query) {
                found.push(id as u32);
            }
        }
        found.sort_by_key(|id| std::cmp::Reverse(self.size(*id, true)));
        found.truncate(limit);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorizes_extensions_case_insensitively() {
        assert_eq!(category_for_name("movie.MP4"), Category::Video);
        assert_eq!(category_for_name("photo.jpeg"), Category::Image);
        assert_eq!(category_for_name("song.flac"), Category::Audio);
        assert_eq!(category_for_name("src.RS"), Category::Code);
        assert_eq!(category_for_name("no-extension"), Category::Other);
    }

    #[test]
    fn ci_search() {
        assert!(ScanTree::contains_ci("HelloWorld.txt", "world"));
        assert!(!ScanTree::contains_ci("short", "longer-than-short"));
    }
}
