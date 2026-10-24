//! XDG directory resolution, session logging, and the advisory instance lock.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Resolved XDG locations used by the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub state_dir: PathBuf,
    pub config_dir: PathBuf,
    pub runtime_dir: PathBuf,
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

impl Paths {
    /// Resolve from the environment, honouring `XDG_*` overrides.
    pub fn from_env() -> Self {
        let home = home();
        let state_dir = env_path("XDG_STATE_HOME").unwrap_or_else(|| home.join(".local/state"));
        let config_dir = env_path("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        let runtime_dir = env_path("XDG_RUNTIME_DIR").unwrap_or_else(|| {
            // Fall back to a per-user directory in /tmp rather than writing into $HOME.
            PathBuf::from(format!("/tmp/redragon-gui-{}", unsafe { libc::getuid() }))
        });
        Paths {
            state_dir,
            config_dir,
            runtime_dir,
        }
    }

    pub fn app_state_dir(&self) -> PathBuf {
        self.state_dir.join("redragon-gui")
    }

    pub fn app_config_dir(&self) -> PathBuf {
        self.config_dir.join("redragon-gui")
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.app_state_dir().join("backups")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.app_state_dir().join("logs")
    }

    /// Directory holding the model descriptors. Installed builds read the data
    /// directory next to the binary; a source checkout uses `models/` at the root.
    pub fn models_dir(&self) -> PathBuf {
        if let Some(dir) = std::env::var_os("REDRAGON_GUI_MODELS").map(PathBuf::from) {
            return dir;
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let candidate = dir.join("models");
                if candidate.join("00-registry.toml").is_file() {
                    return candidate;
                }
                if let Some(share) = dir.parent() {
                    let candidate = share.join("share/redragon-gui/models");
                    if candidate.join("00-registry.toml").is_file() {
                        return candidate;
                    }
                }
            }
        }
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models")
    }

    pub fn lock_path(&self, vendor_id: u16, product_id: u16) -> PathBuf {
        self.runtime_dir
            .join("redragon-gui")
            .join(format!("{vendor_id:04x}-{product_id:04x}.lock"))
    }
}

/// Append-only log file shared with the tracing subscriber.
#[derive(Clone)]
pub struct LogFile {
    file: Arc<Mutex<fs::File>>,
    path: PathBuf,
}

impl LogFile {
    /// Create (or append to) the session log and record a startup line.
    pub fn create(paths: &Paths) -> io::Result<Self> {
        let dir = paths.logs_dir();
        fs::create_dir_all(&dir)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let path = dir.join(format!("session-{stamp}-{}.log", std::process::id()));
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let log = LogFile {
            file: Arc::new(Mutex::new(file)),
            path,
        };
        // Written directly so the startup record exists even if a global
        // subscriber is already installed (tests, embedded use).
        log.write_line(&format!(
            "session start pid={} version={}",
            std::process::id(),
            env!("CARGO_PKG_VERSION")
        ))?;
        Ok(log)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write_line(&self, line: &str) -> io::Result<()> {
        let mut file = self.file.lock().expect("log mutex poisoned");
        writeln!(file, "{line}")?;
        file.flush()
    }
}

impl Write for LogFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut file = self.file.lock().expect("log mutex poisoned");
        file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut file = self.file.lock().expect("log mutex poisoned");
        file.flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogFile {
    type Writer = LogFile;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// An advisory lock so two instances of this tool do not apply at the same time.
pub struct InstanceLock {
    file: fs::File,
    path: PathBuf,
}

impl InstanceLock {
    pub fn acquire(paths: &Paths, vendor_id: u16, product_id: u16) -> io::Result<Self> {
        let path = paths.lock_path(vendor_id, product_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        let fd = std::os::unix::io::AsRawFd::as_raw_fd(&file);
        let rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(InstanceLock { file, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let fd = std::os::unix::io::AsRawFd::as_raw_fd(&self.file);
        unsafe {
            libc::flock(fd, libc::LOCK_UN);
        }
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_file_is_created_under_state_home() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            state_dir: tmp.path().to_path_buf(),
            config_dir: tmp.path().join("config"),
            runtime_dir: tmp.path().join("run"),
        };
        let log = LogFile::create(&paths).expect("log created");
        assert!(log.path().is_file());
        let contents = fs::read_to_string(log.path()).expect("read log");
        assert!(contents.contains("session start"), "contents: {contents}");
    }
}
