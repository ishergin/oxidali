use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use dali2rust_platform::slice_store::{SliceKey, SliceStore, SliceWriteSession, StoreError};

const SLICE_EXTENSION: &str = "bin";

#[derive(Debug, Default)]
struct WriteClaims(Mutex<HashSet<SliceKey>>);

impl WriteClaims {
    fn take(&self, key: SliceKey) -> Result<WriteClaim<'_>, StoreError> {
        if !self.0.lock().unwrap_or_else(PoisonError::into_inner).insert(key) {
            return Err(StoreError::Deferred);
        }
        Ok(WriteClaim { claims: self, key })
    }
}

struct WriteClaim<'a> {
    claims: &'a WriteClaims,
    key: SliceKey,
}

impl Drop for WriteClaim<'_> {
    fn drop(&mut self) {
        self.claims.0.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.key);
    }
}

#[derive(Debug)]
pub struct FileSliceStore {
    base_dir: PathBuf,
    claims: WriteClaims,
}

impl FileSliceStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            claims: WriteClaims::default(),
        }
    }

    fn path(&self, key: SliceKey) -> PathBuf {
        self.base_dir
            .join(key.label())
            .with_extension(SLICE_EXTENSION)
    }
}

fn backend(op: &str, e: std::io::Error) -> StoreError {
    StoreError::Backend(format!("{op}: {e}"))
}

impl SliceStore for FileSliceStore {
    fn load(&self, key: SliceKey) -> Result<Vec<u8>, StoreError> {
        std::fs::read(self.path(key)).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StoreError::Missing,
            _ => backend("read", e),
        })
    }

    fn begin_write(&self, key: SliceKey) -> Result<Box<dyn SliceWriteSession + '_>, StoreError> {
        let claim = self.claims.take(key)?;
        let target = self.path(key);
        if let Some(parent) = target.parent().filter(|parent| !parent.exists()) {
            std::fs::create_dir_all(parent).map_err(|e| backend("create_dir_all", e))?;
        }
        let tmp = target.with_extension("tmp");
        let file = std::fs::File::create(&tmp).map_err(|e| backend("create", e))?;
        Ok(Box::new(FileWriteSession { _claim: claim, target, tmp, file }))
    }
}

struct FileWriteSession<'a> {
    _claim: WriteClaim<'a>,
    target: PathBuf,
    tmp: PathBuf,
    file: std::fs::File,
}

impl SliceWriteSession for FileWriteSession<'_> {
    fn append(&mut self, chunk: &[u8]) -> Result<(), StoreError> {
        self.file
            .write_all(chunk)
            .map_err(|e| backend("write", e))
    }

    fn commit(mut self: Box<Self>) -> Result<(), StoreError> {
        self.file.flush().map_err(|e| backend("flush", e))?;
        drop(self.file);
        std::fs::rename(&self.tmp, &self.target).map_err(|e| {
            let _ = std::fs::remove_file(&self.tmp);
            backend("rename", e)
        })
    }

    fn abort(self: Box<Self>) {
        drop(self.file);
        let _ = std::fs::remove_file(&self.tmp);
    }
}

#[derive(Debug)]
pub struct InMemorySliceStore {
    slices: std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
    claims: WriteClaims,
}

impl Default for InMemorySliceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemorySliceStore {
    pub fn new() -> Self {
        Self {
            slices: std::sync::Mutex::new(std::collections::HashMap::new()),
            claims: WriteClaims::default(),
        }
    }

    pub fn labels(&self) -> Vec<String> {
        let mut labels: Vec<String> = self.lock().keys().cloned().collect();
        labels.sort();
        labels
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, Vec<u8>>> {
        self.slices.lock().expect("slice store lock")
    }
}

impl SliceStore for InMemorySliceStore {
    fn load(&self, key: SliceKey) -> Result<Vec<u8>, StoreError> {
        self.lock()
            .get(&key.label())
            .cloned()
            .ok_or(StoreError::Missing)
    }

    fn begin_write(&self, key: SliceKey) -> Result<Box<dyn SliceWriteSession + '_>, StoreError> {
        Ok(Box::new(MemoryWriteSession {
            _claim: self.claims.take(key)?,
            store: self,
            label: key.label(),
            buf: Vec::new(),
        }))
    }
}

struct MemoryWriteSession<'a> {
    _claim: WriteClaim<'a>,
    store: &'a InMemorySliceStore,
    label: String,
    buf: Vec<u8>,
}

impl SliceWriteSession for MemoryWriteSession<'_> {
    fn append(&mut self, chunk: &[u8]) -> Result<(), StoreError> {
        self.buf.extend_from_slice(chunk);
        Ok(())
    }

    fn commit(self: Box<Self>) -> Result<(), StoreError> {
        self.store.lock().insert(self.label, self.buf);
        Ok(())
    }

    fn abort(self: Box<Self>) {}
}

pub fn slice_file_name(key: SliceKey) -> String {
    format!("{}.{SLICE_EXTENSION}", key.label())
}

pub fn default_base_dir(root: &Path) -> PathBuf {
    root.join("dali/persistence")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("slice-store-{}", std::process::id()));
        let store = FileSliceStore::new(&dir);
        let key = SliceKey::Scene {
            adapter_id: 0,
            scene_id: 3,
        };

        let mut session = store.begin_write(key).expect("begin");
        session.append(b"hello ").expect("append");
        session.append(b"world").expect("append");
        session.commit().expect("commit");

        assert_eq!(store.load(key).expect("load"), b"hello world");
        assert!(dir.join("a0/scene3.bin").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_slice_reads_as_missing() {
        let store = InMemorySliceStore::new();
        assert!(matches!(
            store.load(SliceKey::Adapters),
            Err(StoreError::Missing)
        ));
    }

    #[test]
    fn an_aborted_write_leaves_the_previous_content() {
        let store = InMemorySliceStore::new();
        let mut first = store.begin_write(SliceKey::Adapters).expect("begin");
        first.append(b"first").expect("append");
        first.commit().expect("commit");

        let mut second = store.begin_write(SliceKey::Adapters).expect("begin");
        second.append(b"second").expect("append");
        second.abort();

        assert_eq!(store.load(SliceKey::Adapters).expect("load"), b"first");
    }

    #[test]
    fn a_host_store_defers_a_second_writer_of_one_slice_like_the_device() {
        let store = InMemorySliceStore::new();
        let key = SliceKey::PollerSettings;
        let first = store.begin_write(key).expect("first writer");
        assert!(matches!(store.begin_write(key).map(|_| ()), Err(StoreError::Deferred)));
        first.abort();
        store.begin_write(key).expect("the slot is free once the first let go").abort();
    }
}
