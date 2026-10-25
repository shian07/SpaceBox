use crate::fsops;
use crate::scan::{self, ScanError, ScanProgress, ScanTree, SMALL_FILE_THRESHOLD};
use crate::sys;
use crate::treemap::layout::{self, Layout};
use fltk::app;
use fltk::dialog::{self, FileDialog, FileDialogAction, FileDialogType};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub enum Msg {
    ScanDone {
        generation: u64,
        result: Result<Arc<ScanTree>, ScanError>,
    },
    OpDone {
        generation: u64,
        result: Result<String, String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideMode {
    Files,
    Folders,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hover {
    Node(u32),
    Other { parent: u32, count: u32, size: u64 },
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct LayoutKey {
    focus: u32,
    width: i32,
    height: i32,
    use_alloc: bool,
}

pub struct UiState {
    pub tree: Option<Arc<ScanTree>>,
    pub root: PathBuf,
    pub focus: u32,
    pub selected: Option<u32>,
    pub hovered: Option<Hover>,
    pub layout: Layout,
    pub use_alloc: bool,
    pub scanning: bool,
    pub scan_generation: u64,
    pub op_generation: u64,
    pub progress: Arc<ScanProgress>,
    pub sender: app::Sender<Msg>,
    pub side_mode: SideMode,
    pub search_query: String,
    pub search_results: Vec<u32>,
    pub sidebar_nodes: Vec<u32>,
    pub sidebar_dirty: bool,
    pub dirty: bool,
    pub rss_kb: u64,
    pub toast: Option<(String, Instant)>,
    pub fatal: Option<String>,
    pub op_busy: bool,
    pub scan_started: Option<Instant>,
    layout_key: Option<LayoutKey>,
}

impl UiState {
    pub fn new(root: PathBuf, sender: app::Sender<Msg>) -> Self {
        Self {
            tree: None,
            root,
            focus: 0,
            selected: None,
            hovered: None,
            layout: Layout::empty(),
            use_alloc: true,
            scanning: false,
            scan_generation: 0,
            op_generation: 0,
            progress: Arc::new(ScanProgress::new()),
            sender,
            side_mode: SideMode::Files,
            search_query: String::new(),
            search_results: Vec::new(),
            sidebar_nodes: Vec::new(),
            sidebar_dirty: true,
            dirty: true,
            rss_kb: 0,
            toast: None,
            fatal: None,
            op_busy: false,
            scan_started: None,
            layout_key: None,
        }
    }

    pub fn ensure_layout(&mut self, width: i32, height: i32) {
        let key = LayoutKey {
            focus: self.focus,
            width,
            height,
            use_alloc: self.use_alloc,
        };
        if self.layout_key == Some(key) {
            return;
        }
        let new_layout = self
            .tree
            .as_ref()
            .map(|tree| layout::build(tree, self.focus, width, height, self.use_alloc));
        self.layout = new_layout.unwrap_or_else(Layout::empty);
        self.layout_key = Some(key);
    }

    pub fn invalidate_layout(&mut self) {
        self.layout_key = None;
        self.layout = Layout::empty();
    }

    pub fn refresh_sidebar_nodes(&mut self) {
        let Some(tree) = self.tree.as_ref() else {
            self.sidebar_nodes.clear();
            return;
        };
        self.sidebar_nodes = match self.side_mode {
            SideMode::Files => tree.top_files.clone(),
            SideMode::Folders => tree.top_folders.clone(),
            SideMode::Search => self.search_results.clone(),
        };
    }

    pub fn selected_path(&self) -> Option<PathBuf> {
        let tree = self.tree.as_ref()?;
        self.selected.map(|id| tree.path_of(id))
    }

    pub fn selected_is_dir(&self) -> bool {
        let Some(tree) = self.tree.as_ref() else {
            return false;
        };
        self.selected
            .map(|id| tree.is_dir(id) && !tree.is_small_agg(id))
            .unwrap_or(false)
    }

    pub fn set_toast(&mut self, message: impl Into<String>) {
        self.toast = Some((message.into(), Instant::now()));
        self.dirty = true;
    }
}

pub fn home_dir() -> PathBuf {
    // Unix uses HOME; Windows uses USERPROFILE, falling back to HOMEDRIVE+HOMEPATH.
    let primary: &[&str] = if cfg!(windows) {
        &["USERPROFILE", "HOME"]
    } else {
        &["HOME"]
    };

    for key in primary {
        if let Some(value) = std::env::var_os(key) {
            return PathBuf::from(value);
        }
    }

    if cfg!(windows) {
        if let Some(rest) = std::env::var_os("HOMEPATH") {
            let rest = rest.to_string_lossy();
            let drive = std::env::var_os("HOMEDRIVE").unwrap_or_else(|| "C:".into());
            return PathBuf::from(drive).join(rest.trim_start_matches('\\'));
        }
        return PathBuf::from("C:\\");
    }

    PathBuf::from("/")
}

pub fn start_scan(state: &Rc<RefCell<UiState>>, path: PathBuf) {
    let (sender, generation, progress) = {
        let mut st = state.borrow_mut();
        st.progress.request_cancel();
        st.scan_generation = st.scan_generation.wrapping_add(1);
        let generation = st.scan_generation;
        let progress = Arc::new(ScanProgress::new());
        st.progress = progress.clone();
        let sender = st.sender;
        st.root = path.clone();
        st.scanning = true;
        st.tree = None;
        st.focus = 0;
        st.selected = None;
        st.hovered = None;
        st.fatal = None;
        st.op_busy = false;
        st.sidebar_nodes.clear();
        st.search_results.clear();
        if st.side_mode == SideMode::Search {
            st.side_mode = SideMode::Files;
        }
        st.scan_started = Some(Instant::now());
        st.invalidate_layout();
        st.sidebar_dirty = true;
        st.dirty = true;
        (sender, generation, progress)
    };

    let thread_progress = progress.clone();
    let spawn_result = std::thread::Builder::new()
        .name("spacebox-scan".to_string())
        .stack_size(512 * 1024)
        .spawn(move || {
            let result = scan::scan(&path, SMALL_FILE_THRESHOLD, &thread_progress).map(Arc::new);
            sender.send(Msg::ScanDone { generation, result });
        });

    if let Err(error) = spawn_result {
        let mut st = state.borrow_mut();
        st.scanning = false;
        st.fatal = Some(format!("Could not start scan: {error}"));
        st.dirty = true;
        st.sidebar_dirty = true;
    }
}

pub fn rescan(state: &Rc<RefCell<UiState>>) {
    let root = state.borrow().root.clone();
    start_scan(state, root);
}

pub fn cancel_scan(state: &Rc<RefCell<UiState>>) {
    let progress = state.borrow().progress.clone();
    progress.request_cancel();
    state.borrow_mut().set_toast("Cancelling scan…");
}

pub fn toggle_scan(state: &Rc<RefCell<UiState>>) {
    if state.borrow().scanning {
        cancel_scan(state);
    } else {
        rescan(state);
    }
}

pub fn scan_home(state: &Rc<RefCell<UiState>>) {
    start_scan(state, home_dir());
}

pub fn choose_folder(state: &Rc<RefCell<UiState>>) {
    let current = state.borrow().root.clone();
    let mut chooser = FileDialog::new(FileDialogType::BrowseDir);
    chooser.set_title("Choose a folder to scan");
    let _ = chooser.set_directory(&current);
    match chooser.try_show() {
        Ok(FileDialogAction::Success) => {
            let path = chooser.filename();
            if !path.as_os_str().is_empty() {
                start_scan(state, path);
            }
        }
        Ok(_) => {}
        Err(error) => state
            .borrow_mut()
            .set_toast(format!("Folder chooser failed: {error}")),
    }
}

pub fn handle_message(state: &Rc<RefCell<UiState>>, message: Msg) -> Option<PathBuf> {
    match message {
        Msg::ScanDone { generation, result } => {
            let mut st = state.borrow_mut();
            if generation != st.scan_generation {
                return None;
            }
            st.scanning = false;
            match result {
                Ok(tree) => {
                    let elapsed = tree.stats.elapsed_secs;
                    let files = tree.stats.files;
                    st.root = tree.root.clone();
                    st.tree = Some(tree);
                    st.focus = 0;
                    st.selected = None;
                    st.hovered = None;
                    st.fatal = None;
                    st.invalidate_layout();
                    if st.search_query.is_empty() {
                        st.search_results.clear();
                    } else {
                        let query = st.search_query.clone();
                        if let Some(tree) = st.tree.as_ref() {
                            st.search_results = tree.search(&query, 200);
                        }
                        st.side_mode = SideMode::Search;
                    }
                    st.set_toast(format!(
                        "Scanned {} files in {}",
                        sys::format_count(files),
                        sys::format_duration(elapsed)
                    ));
                    st.sidebar_dirty = true;
                    st.dirty = true;
                }
                Err(error) => {
                    match error {
                        ScanError::Cancelled => {
                            st.fatal = None;
                            st.set_toast("Scan cancelled");
                        }
                        error => {
                            let message = error.to_string();
                            st.fatal = Some(message.clone());
                            st.set_toast(message);
                        }
                    }
                    st.sidebar_dirty = true;
                }
            }
            None
        }
        Msg::OpDone { generation, result } => {
            let mut st = state.borrow_mut();
            if generation != st.op_generation {
                return None;
            }
            st.op_busy = false;
            match result {
                Ok(message) => {
                    st.set_toast(message);
                    Some(st.root.clone())
                }
                Err(message) => {
                    st.set_toast(message);
                    None
                }
            }
        }
    }
}

pub fn navigate_to(state: &Rc<RefCell<UiState>>, id: u32) {
    let mut st = state.borrow_mut();
    let Some(tree) = st.tree.as_ref() else {
        return;
    };
    let is_dir = tree.is_dir(id) && !tree.is_small_agg(id);
    let parent = tree.parent(id).unwrap_or(0);
    st.focus = if is_dir { id } else { parent };
    st.selected = Some(id);
    st.hovered = None;
    st.invalidate_layout();
    st.dirty = true;
}

pub fn navigate_sidebar_row(state: &Rc<RefCell<UiState>>, index: usize) {
    let node = state.borrow().sidebar_nodes.get(index).copied();
    if let Some(node) = node {
        navigate_to(state, node);
    }
}

pub fn go_up(state: &Rc<RefCell<UiState>>) {
    let mut st = state.borrow_mut();
    if st.focus == 0 {
        return;
    }
    let Some(tree) = st.tree.as_ref() else {
        return;
    };
    let parent = tree.parent(st.focus).unwrap_or(0);
    let previous = st.focus;
    st.focus = parent;
    st.selected = Some(previous);
    st.hovered = None;
    st.invalidate_layout();
    st.dirty = true;
}

pub fn activate(state: &Rc<RefCell<UiState>>, id: u32) {
    let is_dir = {
        let st = state.borrow();
        st.tree
            .as_ref()
            .map(|tree| tree.is_dir(id) && !tree.is_small_agg(id))
            .unwrap_or(false)
    };
    if is_dir {
        navigate_to(state, id);
    } else {
        reveal_node(state, id);
    }
}

pub fn set_side_mode(state: &Rc<RefCell<UiState>>, mode: SideMode) {
    let mut st = state.borrow_mut();
    if st.side_mode == mode {
        return;
    }
    st.side_mode = mode;
    if mode == SideMode::Search && st.search_query.is_empty() {
        st.search_results.clear();
        st.set_toast("Type a name in the search box and press Enter");
    }
    st.sidebar_dirty = true;
    st.dirty = true;
}

pub fn set_search(state: &Rc<RefCell<UiState>>, query: String) {
    let mut st = state.borrow_mut();
    st.search_query = query;
    if st.search_query.trim().is_empty() {
        st.search_results.clear();
        if st.side_mode == SideMode::Search {
            st.side_mode = SideMode::Files;
        }
        st.set_toast("Search cleared");
    } else {
        let query = st.search_query.clone();
        if let Some(tree) = st.tree.as_ref() {
            st.search_results = tree.search(&query, 200);
        } else {
            st.search_results.clear();
        }
        st.side_mode = SideMode::Search;
    }
    st.sidebar_dirty = true;
    st.dirty = true;
}

pub fn set_size_mode(state: &Rc<RefCell<UiState>>, use_alloc: bool) {
    let mut st = state.borrow_mut();
    if st.use_alloc == use_alloc {
        return;
    }
    st.use_alloc = use_alloc;
    st.invalidate_layout();
    st.sidebar_dirty = true;
    st.set_toast(if use_alloc {
        "Treemap shows allocated disk space"
    } else {
        "Treemap shows logical file sizes"
    });
}

pub fn open_selected(state: &Rc<RefCell<UiState>>) {
    let Some(path) = state.borrow().selected_path() else {
        state.borrow_mut().set_toast("Select an item first");
        return;
    };
    match fsops::open(&path) {
        Ok(()) => {}
        Err(error) => state.borrow_mut().set_toast(error),
    }
}

pub fn reveal_selected(state: &Rc<RefCell<UiState>>) {
    let (path, is_dir) = {
        let st = state.borrow();
        (st.selected_path(), st.selected_is_dir())
    };
    let Some(path) = path else {
        state.borrow_mut().set_toast("Select an item first");
        return;
    };
    if let Err(error) = fsops::reveal(&path, is_dir) {
        state.borrow_mut().set_toast(error);
    }
}

pub fn copy_selected_path(state: &Rc<RefCell<UiState>>) {
    let Some(path) = state.borrow().selected_path() else {
        state.borrow_mut().set_toast("Select an item first");
        return;
    };
    fsops::copy_path(&path);
    state.borrow_mut().set_toast("Path copied to clipboard");
}

pub fn reveal_node(state: &Rc<RefCell<UiState>>, id: u32) {
    let value = {
        let st = state.borrow();
        let Some(tree) = st.tree.as_ref() else {
            return;
        };
        (tree.path_of(id), tree.is_dir(id) && !tree.is_small_agg(id))
    };
    if let Err(error) = fsops::reveal(&value.0, value.1) {
        state.borrow_mut().set_toast(error);
    }
}

fn selected_info(state: &Rc<RefCell<UiState>>) -> Option<(u32, PathBuf, bool, String, u64)> {
    let st = state.borrow();
    let id = st.selected?;
    if id == 0 {
        return None;
    }
    let tree = st.tree.as_ref()?;
    if tree.is_small_agg(id) {
        return None;
    }
    Some((
        id,
        tree.path_of(id),
        tree.is_dir(id) && !tree.is_small_agg(id),
        tree.name(id).to_string(),
        tree.size(id, st.use_alloc),
    ))
}

pub fn trash_selected(state: &Rc<RefCell<UiState>>) {
    if state.borrow().op_busy {
        return;
    }
    if state.borrow().scanning {
        state.borrow_mut().set_toast("Wait for the scan to finish");
        return;
    }
    let Some((_, path, is_dir, name, size)) = selected_info(state) else {
        state
            .borrow_mut()
            .set_toast("Select a file or folder first");
        return;
    };
    let text = format!(
        "Move \"{}\" ({}) to Trash?\n\n{}",
        name,
        sys::format_bytes(size),
        path.display()
    );
    let choice = dialog::choice2_default(&text, "Move to Trash", "Cancel", "");
    if choice == Some(0) {
        spawn_operation(state, path, is_dir, Operation::Trash);
    }
}

pub fn delete_selected_permanently(state: &Rc<RefCell<UiState>>) {
    if state.borrow().op_busy {
        return;
    }
    if state.borrow().scanning {
        state.borrow_mut().set_toast("Wait for the scan to finish");
        return;
    }
    let Some((_, path, is_dir, name, size)) = selected_info(state) else {
        state
            .borrow_mut()
            .set_toast("Select a file or folder first");
        return;
    };
    let text = format!(
        "Permanently delete \"{}\" ({})?\n\n{}\n\nThis cannot be undone.",
        name,
        sys::format_bytes(size),
        path.display()
    );
    let choice = dialog::choice2_default(&text, "Delete permanently", "Cancel", "");
    if choice == Some(0) {
        spawn_operation(state, path, is_dir, Operation::Delete);
    }
}

enum Operation {
    Trash,
    Delete,
}

fn spawn_operation(
    state: &Rc<RefCell<UiState>>,
    path: PathBuf,
    is_dir: bool,
    operation: Operation,
) {
    let (sender, generation) = {
        let mut st = state.borrow_mut();
        st.op_generation = st.op_generation.wrapping_add(1);
        st.op_busy = true;
        st.dirty = true;
        (st.sender, st.op_generation)
    };
    let result = std::thread::Builder::new()
        .name("spacebox-op".to_string())
        .stack_size(256 * 1024)
        .spawn(move || {
            let outcome = match operation {
                Operation::Trash => fsops::trash(&path).map(|_| "Moved to Trash".to_string()),
                Operation::Delete => fsops::delete_permanently(&path, is_dir)
                    .map(|_| "Deleted permanently".to_string()),
            };
            sender.send(Msg::OpDone {
                generation,
                result: outcome,
            });
        });
    if let Err(error) = result {
        let mut st = state.borrow_mut();
        st.op_busy = false;
        st.set_toast(format!("Could not start operation: {error}"));
    }
}

pub fn show_errors(state: &Rc<RefCell<UiState>>) {
    let text = {
        let st = state.borrow();
        let Some(tree) = st.tree.as_ref() else {
            return;
        };
        let mut text = format!(
            "{} item(s) could not be read.\n",
            sys::format_count(tree.stats.errors)
        );
        for error in &tree.errors {
            let mut path = tree.path_of(error.parent);
            if !error.name.is_empty() {
                path.push(&error.name);
            }
            text.push_str(&format!("\n{}\n  {}", path.display(), error.message));
        }
        if tree.stats.errors as usize > tree.errors.len() {
            text.push_str("\n\nOnly the first errors are shown.");
        }
        text
    };
    dialog::message_default(&text);
}
