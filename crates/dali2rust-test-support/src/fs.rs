use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use dali2rust_bsp::fs::StdFileSystemHal;
use dali2rust_platform::fs::{FileMetadata, FileSystemHal, FsError};

#[derive(Debug, Default)]
struct FsState {
    dirs: BTreeSet<String>,
    files: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Default)]
pub struct InMemoryFileSystemHal {
    state: Mutex<FsState>,
}

impl InMemoryFileSystemHal {
    pub fn new() -> Self {
        Self::default()
    }

    fn normalize(path: &str) -> String {
        path.trim_matches('/').to_string()
    }

    fn insert_parent_dirs(state: &mut FsState, path: &str) {
        let mut current = String::new();
        let mut parts = path.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                break;
            }
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(part);
            state.dirs.insert(current.clone());
        }
    }

    fn has_dir(state: &FsState, path: &str) -> bool {
        state.dirs.contains(path)
            || state
                .files
                .keys()
                .any(|candidate| candidate.starts_with(&format!("{path}/")))
    }
}

impl FileSystemHal for InMemoryFileSystemHal {
    fn write_file_atomic(&self, path: &str, content: &[u8]) -> Result<(), FsError> {
        let path = Self::normalize(path);
        if path.is_empty() {
            return Err(FsError::Other("empty path".into()));
        }
        let mut state = self.state.lock().expect("fs lock");
        Self::insert_parent_dirs(&mut state, &path);
        state.files.insert(path.clone(), content.to_vec());
        state.dirs.remove(&path);
        Ok(())
    }

    fn metadata(&self, path: &str) -> Result<FileMetadata, FsError> {
        let path = Self::normalize(path);
        let state = self.state.lock().expect("fs lock");
        if let Some(content) = state.files.get(&path) {
            return Ok(FileMetadata {
                is_dir: false,
                len: content.len() as u64,
            });
        }
        if Self::has_dir(&state, &path) {
            return Ok(FileMetadata {
                is_dir: true,
                len: 0,
            });
        }
        Err(FsError::NotFound)
    }

    fn read_file(&self, path: &str) -> Result<Vec<u8>, FsError> {
        let path = Self::normalize(path);
        self.state
            .lock()
            .expect("fs lock")
            .files
            .get(&path)
            .cloned()
            .ok_or(FsError::NotFound)
    }

    fn exists(&self, path: &str) -> bool {
        let path = Self::normalize(path);
        let state = self.state.lock().expect("fs lock");
        state.files.contains_key(&path) || Self::has_dir(&state, &path)
    }

    fn create_dir_all(&self, path: &str) -> Result<(), FsError> {
        let path = Self::normalize(path);
        let mut state = self.state.lock().expect("fs lock");
        if !path.is_empty() {
            let dir_path = format!("{path}/_dir");
            Self::insert_parent_dirs(&mut state, &dir_path);
            state.dirs.insert(path);
        }
        Ok(())
    }

    fn list_dir(&self, dir: &str) -> Result<Vec<String>, FsError> {
        let dir = Self::normalize(dir);
        let state = self.state.lock().expect("fs lock");
        if !dir.is_empty() && !Self::has_dir(&state, &dir) {
            return Err(FsError::NotFound);
        }
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        Ok(state
            .files
            .keys()
            .filter_map(|path| path.strip_prefix(&prefix).map(ToOwned::to_owned))
            .collect())
    }
}

pub fn temp_fs(label: &str) -> StdFileSystemHal {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    StdFileSystemHal::new(std::env::temp_dir().join(format!("d2r-test-{label}-{nonce}")))
}

pub fn temp_slice_store(label: &str) -> dali2rust_bsp::slice_store_files::FileSliceStore {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    dali2rust_bsp::slice_store_files::FileSliceStore::new(
        std::env::temp_dir().join(format!("d2r-test-{label}-{nonce}")),
    )
}

pub fn write_slice(
    store: &dyn dali2rust_platform::slice_store::SliceStore,
    key: dali2rust_platform::slice_store::SliceKey,
    bytes: &[u8],
) {
    let mut session = store.begin_write(key).expect("begin slice write");
    session.append(bytes).expect("append slice");
    session.commit().expect("commit slice");
}

#[cfg(test)]
mod tests {
    use super::{FileSystemHal, InMemoryFileSystemHal};

    #[test]
    fn in_memory_fs_writes_and_reads_files() {
        let fs = InMemoryFileSystemHal::new();
        fs.write_file_atomic("/dali/persistence/adapters.json", br#"{"ok":true}"#)
            .expect("write");
        assert!(fs.exists("/dali/persistence"));
        assert_eq!(
            fs.read_file("/dali/persistence/adapters.json").expect("read"),
            br#"{"ok":true}"#
        );
    }

    #[test]
    fn in_memory_fs_lists_files_relative_to_requested_dir() {
        let fs = InMemoryFileSystemHal::new();
        fs.write_file_atomic("/dali/persistence/adapters.json", b"a")
            .expect("write adapters");
        fs.write_file_atomic("/dali/persistence/a0/virtual_lamps.json", b"b")
            .expect("write lamps");
        assert_eq!(
            fs.list_dir("/dali/persistence").expect("list"),
            vec!["a0/virtual_lamps.json".to_string(), "adapters.json".to_string()]
        );
    }

    #[test]
    fn in_memory_fs_reports_missing_files() {
        let fs = InMemoryFileSystemHal::new();
        assert!(fs.read_file("/missing.json").is_err());
        assert!(fs.list_dir("/missing").is_err());
    }
}
