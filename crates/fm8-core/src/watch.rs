//! Watching the open directory with raw Linux inotify (libc only).
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub struct DirWatch {
    fd: i32,
    pub dir: PathBuf,
}

const MASK: u32 = libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_MOVED_TO | libc::IN_MODIFY | libc::IN_ATTRIB | libc::IN_CLOSE_WRITE | libc::IN_DELETE_SELF | libc::IN_MOVE_SELF;

impl DirWatch {
    pub fn new(dir: &Path) -> Result<DirWatch, String> {
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 { return Err(std::io::Error::last_os_error().to_string()) }
        let c = CString::new(dir.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        if unsafe { libc::inotify_add_watch(fd, c.as_ptr(), MASK) } < 0 {
            let e = std::io::Error::last_os_error().to_string();
            unsafe { libc::close(fd) };
            return Err(e);
        }
        Ok(DirWatch { fd, dir: dir.to_path_buf() })
    }

    /// True if anything changed since the last call (events are drained and coalesced).
    pub fn changed(&self) -> bool {
        let mut any = false;
        let mut buf = [0u8; 4096];
        loop {
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 { break }
            any = true; // every watched event type is a change; access events are not subscribed
        }
        any
    }
}

impl Drop for DirWatch {
    fn drop(&mut self) { unsafe { libc::close(self.fd) }; }
}
