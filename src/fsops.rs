use fltk::app;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

fn spawn(program: &str, args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("{program}: {error}"))
}

pub fn copy_path(path: &Path) {
    app::copy(&path.to_string_lossy());
}

/// Open a file or folder with the platform's default handler.
pub fn open(path: &Path) -> Result<(), String> {
    imp::open(path)
}

/// Reveal a path in the platform's file manager, selecting it when possible.
pub fn reveal(path: &Path, is_dir: bool) -> Result<(), String> {
    imp::reveal(path, is_dir)
}

pub fn trash(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|error| error.to_string())
}

pub fn delete_permanently(path: &Path, is_dir: bool) -> Result<(), String> {
    if path.parent().is_none() {
        return Err("Refusing to delete a filesystem root".to_string());
    }
    if is_dir {
        fs::remove_dir_all(path).map_err(|error| error.to_string())
    } else {
        fs::remove_file(path).map_err(|error| error.to_string())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::*;
    use std::path::PathBuf;

    fn parent_of(path: &Path) -> PathBuf {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf())
    }

    pub fn open(path: &Path) -> Result<(), String> {
        spawn("xdg-open", [path.as_os_str().to_os_string()])
    }

    pub fn reveal(path: &Path, is_dir: bool) -> Result<(), String> {
        let attempts: [(&str, Vec<OsString>); 3] = [
            (
                "dolphin",
                if is_dir {
                    vec![path.as_os_str().to_os_string()]
                } else {
                    vec![OsString::from("--select"), path.as_os_str().to_os_string()]
                },
            ),
            (
                "nautilus",
                if is_dir {
                    vec![path.as_os_str().to_os_string()]
                } else {
                    vec![OsString::from("--select"), path.as_os_str().to_os_string()]
                },
            ),
            ("thunar", vec![path.as_os_str().to_os_string()]),
        ];

        for (program, args) in attempts {
            if spawn(program, args).is_ok() {
                return Ok(());
            }
        }

        let parent = parent_of(path);
        spawn("xdg-open", [parent.as_os_str().to_os_string()])
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    pub fn open(path: &Path) -> Result<(), String> {
        spawn("open", [path.as_os_str().to_os_string()])
    }

    pub fn reveal(path: &Path, is_dir: bool) -> Result<(), String> {
        if is_dir {
            spawn("open", [path.as_os_str().to_os_string()])
        } else {
            spawn(
                "open",
                [OsString::from("-R"), path.as_os_str().to_os_string()],
            )
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;

    pub fn open(path: &Path) -> Result<(), String> {
        // `cmd /C start "" "path"` uses the shell's file association. The empty
        // title argument is required so a quoted path is not read as the title.
        spawn(
            "cmd",
            [
                OsString::from("/C"),
                OsString::from("start"),
                OsString::from(""),
                path.as_os_str().to_os_string(),
            ],
        )
    }

    pub fn reveal(path: &Path, is_dir: bool) -> Result<(), String> {
        if is_dir {
            return spawn("explorer", [path.as_os_str().to_os_string()]);
        }
        // `/select,"file"` must be one argument including the quotes.
        let select = format!("/select,\"{}\"", path.to_string_lossy());
        spawn("explorer", [OsString::from(select)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("spacebox-{label}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn permanent_delete_removes_file() {
        let dir = scratch_dir("file");
        let file = dir.join("payload.txt");
        fs::write(&file, b"hello").unwrap();

        delete_permanently(&file, false).unwrap();
        assert!(!file.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn permanent_delete_removes_tree() {
        let dir = scratch_dir("tree");
        let tree = dir.join("nested");
        fs::create_dir_all(tree.join("deeper")).unwrap();
        fs::write(tree.join("deeper").join("file.txt"), b"data").unwrap();

        delete_permanently(&tree, true).unwrap();
        assert!(!tree.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn permanent_delete_refuses_filesystem_root() {
        let root: &Path = std::path::MAIN_SEPARATOR_STR.as_ref();
        let error = delete_permanently(Path::new(root), true).unwrap_err();
        assert!(error.contains("root"));
    }

    #[test]
    fn trash_moves_file_out_of_place() {
        let dir = scratch_dir("trash");
        let file = dir.join("trash-me.txt");
        fs::write(&file, b"bye").unwrap();

        trash(&file).unwrap();
        assert!(!file.exists());

        let removed: Vec<_> = trash::os_limited::list()
            .unwrap()
            .into_iter()
            .filter(|item| item.original_path() == file)
            .collect();
        assert!(!removed.is_empty(), "trashed file not found in trash list");
        trash::os_limited::purge_all(removed).unwrap();

        fs::remove_dir_all(&dir).unwrap();
    }
}
