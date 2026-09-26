use dali2rust_platform::fs::{FileMetadata, FileSystemHal, FsError, FsWriteSession};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct StdFileSystemHal {
    base_dir: PathBuf,
}

impl StdFileSystemHal {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    fn full_path(&self, rel: &str) -> PathBuf {
        let rel = rel.trim_start_matches('/');
        self.base_dir.join(rel)
    }
}

impl FileSystemHal for StdFileSystemHal {
    fn write_file_atomic(&self, path: &str, content: &[u8]) -> Result<(), FsError> {
        let target = self.full_path(path);
        let tmp = target.with_extension("tmp");
        if let Some(parent) = target.parent().filter(|parent| !parent.exists()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&tmp, content).map_err(FsError::from)?;
        std::fs::rename(&tmp, &target).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            FsError::Io(e)
        })
    }

    fn begin_atomic_write<'a>(
        &'a self,
        path: &str,
    ) -> Result<Box<dyn FsWriteSession + 'a>, FsError> {
        let target = self.full_path(path);
        let tmp = target.with_extension("tmp");
        if let Some(parent) = target.parent().filter(|parent| !parent.exists()) {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::File::create(&tmp).map_err(FsError::from)?;
        Ok(Box::new(StdWriteSession { target, tmp, file }))
    }

    fn metadata(&self, path: &str) -> Result<FileMetadata, FsError> {
        let full = self.full_path(path);
        let m = std::fs::metadata(&full).map_err(FsError::from)?;
        Ok(FileMetadata {
            is_dir: m.is_dir(),
            len: m.len(),
        })
    }

    fn read_file(&self, path: &str) -> Result<Vec<u8>, FsError> {
        let full = self.full_path(path);
        std::fs::read(&full).map_err(FsError::from)
    }

    fn exists(&self, path: &str) -> bool {
        self.full_path(path).exists()
    }

    fn create_dir_all(&self, path: &str) -> Result<(), FsError> {
        let full = self.full_path(path);
        std::fs::create_dir_all(&full)?;
        Ok(())
    }

    fn list_dir(&self, dir: &str) -> Result<Vec<String>, FsError> {
        let full = self.full_path(dir);
        let mut files = Vec::new();
        Self::walk_dir(&full, "", &mut files)?;
        Ok(files)
    }
}

struct StdWriteSession {
    target: PathBuf,
    tmp: PathBuf,
    file: std::fs::File,
}

impl FsWriteSession for StdWriteSession {
    fn append(&mut self, chunk: &[u8]) -> Result<(), FsError> {
        self.file.write_all(chunk).map_err(FsError::from)
    }

    fn commit(mut self: Box<Self>) -> Result<(), FsError> {
        self.file.flush().map_err(FsError::from)?;
        drop(self.file);
        std::fs::rename(&self.tmp, &self.target).map_err(|e| {
            let _ = std::fs::remove_file(&self.tmp);
            FsError::Io(e)
        })
    }

    fn abort(self: Box<Self>) {
        drop(self.file);
        let _ = std::fs::remove_file(&self.tmp);
    }
}

impl StdFileSystemHal {
    fn walk_dir(base: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), FsError> {
        let dir = if prefix.is_empty() {
            base.to_path_buf()
        } else {
            base.join(prefix)
        };
        for entry in std::fs::read_dir(&dir).map_err(FsError::from)? {
            let entry = entry.map_err(FsError::from)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let full_rel_path = base.join(&rel);
            let meta = std::fs::metadata(&full_rel_path).map_err(FsError::from)?;
            if meta.is_dir() {
                StdFileSystemHal::walk_dir(base, &rel, out)?;
            } else {
                out.push(rel);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_fs(label: &str) -> StdFileSystemHal {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        StdFileSystemHal::new(std::env::temp_dir().join(format!("d2r-bsp-fs-{label}-{nonce}")))
    }

    #[test]
    fn streaming_session_commits_chunks_atomically() {
        let fs = scratch_fs("commit");
        let mut session = fs.begin_atomic_write("/a0/data.bin").expect("begin");
        session.append(b"hello ").expect("append 1");
        session.append(b"world").expect("append 2");
        assert!(
            !fs.exists("/a0/data.bin"),
            "target must not appear before commit"
        );
        session.commit().expect("commit");
        assert_eq!(fs.read_file("/a0/data.bin").expect("read"), b"hello world");
        assert!(!fs.exists("/a0/data.tmp"), "tmp must be renamed away");
    }

    #[test]
    fn an_atomic_write_and_a_commit_replace_the_previous_content() {
        let fs = scratch_fs("replace");
        fs.write_file_atomic("/a0/data.bin", b"old").expect("seed");
        fs.write_file_atomic("/a0/data.bin", b"newer").expect("replace");
        assert_eq!(fs.read_file("/a0/data.bin").expect("read"), b"newer");
        let mut session = fs.begin_atomic_write("/a0/data.bin").expect("begin");
        session.append(b"newest").expect("append");
        session.commit().expect("commit");
        assert_eq!(fs.read_file("/a0/data.bin").expect("read"), b"newest");
    }

    #[test]
    fn streaming_session_abort_leaves_previous_content_intact() {
        let fs = scratch_fs("abort");
        fs.write_file_atomic("/a0/data.bin", b"old").expect("seed");
        let mut session = fs.begin_atomic_write("/a0/data.bin").expect("begin");
        session.append(b"partial new content").expect("append");
        session.abort();
        assert_eq!(fs.read_file("/a0/data.bin").expect("read"), b"old");
        assert!(!fs.exists("/a0/data.tmp"), "tmp must be cleaned up on abort");
    }
}
