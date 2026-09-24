#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub is_dir: bool,
    pub len: u64,
}

#[derive(Debug)]
pub enum FsError {
    Io(std::io::Error),
    NotFound,
    NoSpace,
    Other(String),
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsError::Io(e) => write!(f, "io: {e}"),
            FsError::NotFound => write!(f, "not found"),
            FsError::NoSpace => write!(f, "no space"),
            FsError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl From<std::io::Error> for FsError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound {
            FsError::NotFound
        } else {
            FsError::Io(e)
        }
    }
}

pub trait FsWriteSession {
    fn append(&mut self, chunk: &[u8]) -> Result<(), FsError>;

    fn commit(self: Box<Self>) -> Result<(), FsError>;

    fn abort(self: Box<Self>);
}

struct BufferedWriteSession<'a, F: FileSystemHal + ?Sized> {
    fs: &'a F,
    path: String,
    buf: Vec<u8>,
}

impl<F: FileSystemHal + ?Sized> FsWriteSession for BufferedWriteSession<'_, F> {
    fn append(&mut self, chunk: &[u8]) -> Result<(), FsError> {
        self.buf.extend_from_slice(chunk);
        Ok(())
    }

    fn commit(self: Box<Self>) -> Result<(), FsError> {
        self.fs.write_file_atomic(&self.path, &self.buf)
    }

    fn abort(self: Box<Self>) {}
}

pub trait FileSystemHal: Send + Sync {
    fn write_file_atomic(&self, path: &str, content: &[u8]) -> Result<(), FsError>;

    fn begin_atomic_write<'a>(&'a self, path: &str) -> Result<Box<dyn FsWriteSession + 'a>, FsError> {
        Ok(Box::new(BufferedWriteSession {
            fs: self,
            path: path.to_string(),
            buf: Vec::new(),
        }))
    }

    fn metadata(&self, path: &str) -> Result<FileMetadata, FsError>;

    fn read_file(&self, path: &str) -> Result<Vec<u8>, FsError>;

    fn exists(&self, path: &str) -> bool;

    fn create_dir_all(&self, path: &str) -> Result<(), FsError>;

    fn list_dir(&self, dir: &str) -> Result<Vec<String>, FsError>;
}
