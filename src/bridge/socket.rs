//! The local channel the app listens on and the host relays through.
//!
//! A Unix domain socket on unix and a named pipe on Windows, never a TCP port —
//! nothing here is reachable off the machine. On unix it lives in the session's
//! runtime dir: per-user, off the disk, and cleared by the system at logout, with
//! the app's data dir as the fallback on a system without one. On Windows it is
//! `\\.\pipe\pandora-bridge-<digest>`, created with `PIPE_REJECT_REMOTE_CLIENTS`
//! and the creator's own DACL, so the same-user rule holds there too: a crashed
//! process takes its pipe with it, which is why only unix has to sweep a leftover.
//!
//! Both ends of both platforms are named by one path — [`path`] — and each platform
//! turns that path into whatever it actually listens on. That is what lets a test
//! point both peers at a channel of its own with `PANDORA_BRIDGE_SOCKET`.

use std::io;
use std::path::PathBuf;

#[cfg(unix)]
pub use unix::{connect_at, Listener, Stream};
#[cfg(windows)]
pub use windows::{connect_at, Listener, Stream};

#[cfg(not(any(unix, windows)))]
pub use other::{connect_at, Listener, Stream};

/// Where the app listens. `PANDORA_BRIDGE_SOCKET` names it outright, which is how a
/// test — or a second install — points both peers at a channel of its own instead of
/// the session's.
///
/// Otherwise the session's runtime dir: per-user, off the disk, and cleared by the
/// system at logout. The app's data dir is the fallback on a system without one.
pub fn path() -> PathBuf {
    if let Some(path) = std::env::var_os("PANDORA_BRIDGE_SOCKET").filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(crate::vault::store::data_dir);
    base.join("pandora-bridge.sock")
}

/// Reach the app on the default channel. Fails when nothing is listening, which
/// every caller reads as "no bridge here, answer it yourself".
pub fn connect() -> io::Result<Stream> {
    connect_at(&path())
}

#[cfg(unix)]
mod unix {
    use std::fs::Permissions;
    use std::io;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;

    pub type Stream = UnixStream;

    pub struct Listener(UnixListener);

    impl Listener {
        /// Bind at one named socket. Two apps on one vault would race for every
        /// request, so the second one gets `None` and does not listen at all.
        pub fn bind_at(path: &Path) -> io::Result<Option<Self>> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            // Something already listening is a live instance; anything else at that
            // path is a leftover from a crash, and bind would refuse it.
            if UnixStream::connect(path).is_ok() {
                return Ok(None);
            }
            let _ = std::fs::remove_file(path);
            let listener = UnixListener::bind(path)?;
            // The directory already keeps other users out; this keeps it honest if
            // the socket ever lands somewhere shareable.
            std::fs::set_permissions(path, Permissions::from_mode(0o600))?;
            Ok(Some(Self(listener)))
        }

        /// The next connection. Blocks until one arrives: the app keeps a thread
        /// parked here, so a browser is answered even when nothing is drawing.
        pub fn accept(&self) -> io::Result<Stream> {
            self.0.accept().map(|(stream, _)| stream)
        }
    }

    pub fn connect_at(path: &Path) -> io::Result<Stream> {
        UnixStream::connect(path)
    }
}

/// Windows has no socket in `std`, so the channel is a named pipe driven through
/// `windows-sys`. The pipe name is derived from [`super::path`], never from the
/// user name, so `PANDORA_BRIDGE_SOCKET` isolates one test or one install exactly
/// as it does on unix.
#[cfg(windows)]
mod windows {
    use std::io::{self, Read, Write};
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::Duration;
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING,
        ERROR_NO_DATA, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE,
        INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, WaitNamedPipeW, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

    /// Room on each side of one message. The protocol caps a message at 1 MiB, and
    /// the buffer only has to hold what is in flight, not the whole of one.
    const BUFFER: u32 = 64 * 1024;

    pub struct Listener {
        name: Vec<u16>,
        /// The instance waiting for the next caller, handed out by `accept`. Behind
        /// a lock because `accept` borrows the listener shared — the shape the unix
        /// side has — and because the pipe instance it leaves behind is the only
        /// state either side carries.
        next: Mutex<Option<HANDLE>>,
    }

    impl Listener {
        /// Bind one named pipe. A second app finds the first one's instance waiting
        /// and gets `None`, so only one of them ever answers.
        pub fn bind_at(path: &Path) -> io::Result<Option<Self>> {
            let name = name(path)?;
            // An instance that nobody has connected to yet is a live app waiting for
            // a browser; every instance busy is one with a request in flight. Either
            // way the name is taken.
            if unsafe { WaitNamedPipeW(name.as_ptr(), 0) } != 0 {
                return Ok(None);
            }
            if unsafe { GetLastError() } != ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            let handle = create(&name)?;
            Ok(Some(Self {
                name,
                next: Mutex::new(Some(handle)),
            }))
        }

        /// The next connection. Blocks until one arrives, exactly as unix does, and
        /// leaves a fresh instance behind so the app goes on answering.
        pub fn accept(&self) -> io::Result<Stream> {
            let Some(handle) = self.next.lock().expect("pipe lock").take() else {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "the bridge has no pipe instance left",
                ));
            };

            let mut wait = Wait::new()?;
            let started = unsafe { ConnectNamedPipe(handle, wait.overlapped()) };
            // A caller that arrived between the two calls connected already: the API
            // reports that as a failure, and it is the outcome we wanted.
            if started == 0 && unsafe { GetLastError() } == ERROR_PIPE_CONNECTED {
                self.renew();
                return Ok(Stream::new(handle));
            }
            if let Err(e) = wait.finish(handle, started, None) {
                unsafe { CloseHandle(handle) };
                self.renew();
                return Err(e);
            }
            self.renew();
            Ok(Stream::new(handle))
        }

        /// The instance the next `accept` takes. A failure here is not fatal to the
        /// connection already accepted: the listener simply stops answering after it.
        fn renew(&self) {
            if let Some(mut slot) = self.next.lock().ok() {
                if slot.is_none() {
                    *slot = create(&self.name).ok();
                }
            }
        }
    }

    pub struct Stream {
        handle: HANDLE,
        timeout: Mutex<Option<Duration>>,
    }

    impl Stream {
        fn new(handle: HANDLE) -> Self {
            Self {
                handle,
                timeout: Mutex::new(None),
            }
        }

        /// One overlapped read or write, waiting at most `timeout` for it. A read
        /// that runs out of time is cancelled, not abandoned, so the handle is not
        /// left with an operation still pointing at a buffer we are about to drop.
        fn begin(
            &self,
            buf: *mut u8,
            len: usize,
            write: bool,
            timeout: Option<Duration>,
        ) -> io::Result<usize> {
            if len == 0 {
                return Ok(0);
            }
            let len = u32::try_from(len)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "message too long"))?;
            let mut wait = Wait::new()?;
            let started = unsafe {
                if write {
                    WriteFile(
                        self.handle,
                        buf,
                        len,
                        std::ptr::null_mut(),
                        wait.overlapped(),
                    )
                } else {
                    ReadFile(
                        self.handle,
                        buf,
                        len,
                        std::ptr::null_mut(),
                        wait.overlapped(),
                    )
                }
            };
            wait.finish(self.handle, started, timeout)
        }
    }

    impl Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let timeout = *self.timeout.lock().expect("timeout lock");
            match self.begin(buf.as_mut_ptr(), buf.len(), false, timeout) {
                Ok(n) => Ok(n),
                // The other end hung up, or the pipe was closed under us. `read_exact`
                // turns this into the UnexpectedEof that ends the protocol cleanly.
                Err(e) if matches!(e.raw_os_error(), Some(code) if code == ERROR_BROKEN_PIPE as i32 || code == ERROR_NO_DATA as i32) => {
                    Ok(0)
                }
                Err(e) => Err(e),
            }
        }
    }

    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.begin(buf.as_ptr().cast_mut(), buf.len(), true, None)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Stream {
        pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            *self.timeout.lock().expect("timeout lock") = timeout;
            Ok(())
        }
    }

    impl Drop for Stream {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
        }
    }

    // The handle is owned by exactly one `Stream`, which moves between threads and is
    // never shared: `&Stream` is not usable for I/O on its own, so moving it is safe.
    unsafe impl Send for Stream {}

    pub fn connect_at(path: &Path) -> io::Result<Stream> {
        let name = name(path)?;
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(Stream::new(handle))
    }

    /// One instance of the pipe, waiting to be connected.
    fn create(name: &[u16]) -> io::Result<HANDLE> {
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER,
                BUFFER,
                0,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(handle)
    }

    /// The pipe name that stands for one path. A digest rather than the path itself:
    /// a pipe name is machine-global, and two users with a data dir each must never
    /// land on each other's pipe.
    fn name(path: &Path) -> io::Result<Vec<u16>> {
        let key = crate::vault::crypto::digest(path.to_string_lossy().as_bytes());
        let full = format!("\\\\.\\pipe\\pandora-bridge-{key}");
        Ok(full.encode_utf16().chain(std::iter::once(0)).collect())
    }

    /// One overlapped operation: the structure the API fills in, and the event that
    /// says it finished. Owned per call so a timed-out one cannot outlive its buffer.
    struct Wait {
        ov: OVERLAPPED,
        event: HANDLE,
    }

    impl Wait {
        fn new() -> io::Result<Self> {
            let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
            if event.is_null() {
                return Err(io::Error::last_os_error());
            }
            // `hEvent` is the one field we own; the API writes the rest.
            let mut ov = OVERLAPPED::default();
            ov.hEvent = event;
            Ok(Self { ov, event })
        }

        fn overlapped(&mut self) -> *mut OVERLAPPED {
            &mut self.ov
        }

        /// Wait for `started`'s operation and hand back how many bytes it moved.
        /// `None` waits for as long as the other end takes.
        fn finish(
            &mut self,
            handle: HANDLE,
            started: BOOL,
            timeout: Option<Duration>,
        ) -> io::Result<usize> {
            if started == 0 {
                let failed = io::Error::last_os_error();
                if failed.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                    return Err(failed);
                }
                let millis = match timeout {
                    Some(d) => d.as_millis().min(u32::MAX as u128) as u32,
                    None => u32::MAX, // INFINITE
                };
                match unsafe { WaitForSingleObject(self.event, millis) } {
                    WAIT_OBJECT_0 => {}
                    WAIT_TIMEOUT => {
                        // Cancel and let the cancellation land: the OVERLAPPED cannot
                        // be dropped while the kernel may still write to it.
                        unsafe {
                            CancelIoEx(handle, &self.ov);
                            WaitForSingleObject(self.event, u32::MAX);
                        }
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "the other end did not answer in time",
                        ));
                    }
                    _ => return Err(io::Error::last_os_error()),
                }
            }
            let mut moved = 0u32;
            if unsafe { GetOverlappedResult(handle, &self.ov, &mut moved, 0) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(moved as usize)
        }
    }

    impl Drop for Wait {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.event) };
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::{Read, Write};

        /// The whole channel, both ends, over the real kernel object: what the unix
        /// side gets from `UnixStream` and this side has to build.
        #[test]
        fn a_pipe_carries_a_message_both_ways_and_hangs_up_cleanly() {
            let dir = std::env::temp_dir().join(format!("pandora-pipe-{}", std::process::id()));
            let path = dir.join("bridge.sock");
            let listener = Listener::bind_at(&path)
                .expect("bind")
                .expect("nothing is listening yet");

            let here = path.clone();
            let peer = std::thread::spawn(move || {
                let mut stream = connect_at(&here).expect("connect");
                stream.write_all(b"ping").unwrap();
                let mut back = [0u8; 4];
                stream.read_exact(&mut back).unwrap();
                assert_eq!(&back, b"pong");
            });

            let mut stream = listener.accept().expect("accept");
            let mut got = [0u8; 4];
            stream.read_exact(&mut got).unwrap();
            assert_eq!(&got, b"ping");
            stream.write_all(b"pong").unwrap();
            // The other end hangs up: this must read as a clean end, not an error.
            drop(stream);
            peer.join().unwrap();
        }

        /// A second app finds the first one's pipe and does not try to share it.
        #[test]
        fn a_second_listener_declines() {
            let dir = std::env::temp_dir().join(format!("pandora-pipe-2-{}", std::process::id()));
            let path = dir.join("bridge.sock");
            let _first = Listener::bind_at(&path).expect("bind").expect("first");
            assert!(
                Listener::bind_at(&path).expect("bind").is_none(),
                "the second app must not listen on the same box"
            );
        }

        /// A read that nobody answers gives up on its own, and the connection can
        /// still be dropped without leaving an operation pointing at a dead buffer.
        #[test]
        fn a_silent_caller_times_out() {
            let dir = std::env::temp_dir().join(format!("pandora-pipe-3-{}", std::process::id()));
            let path = dir.join("bridge.sock");
            let listener = Listener::bind_at(&path).expect("bind").expect("first");

            let here = path.clone();
            let peer = std::thread::spawn(move || connect_at(&here).expect("connect"));

            let mut stream = listener.accept().expect("accept");
            stream
                .set_read_timeout(Some(Duration::from_millis(50)))
                .unwrap();
            let mut buf = [0u8; 8];
            let refused = stream.read(&mut buf);
            assert_eq!(
                refused.unwrap_err().kind(),
                io::ErrorKind::TimedOut,
                "a caller that says nothing must not park the app forever"
            );
            drop(peer.join().unwrap());
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod other {
    use std::io::{self, Read, Write};
    use std::path::Path;

    /// Never constructed: this platform has no channel yet, so `connect_at` always
    /// fails and `bind_at` always says no. It exists so the relay's code type-checks
    /// where there is nothing to listen on.
    pub struct Stream;

    impl Stream {
        pub fn set_read_timeout(&self, _: Option<std::time::Duration>) -> io::Result<()> {
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    impl Read for Stream {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    impl Write for Stream {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::Unsupported.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    pub struct Listener;

    impl Listener {
        pub fn bind_at(_: &Path) -> io::Result<Option<Self>> {
            Ok(None)
        }

        /// Never called: `bind_at` always says no, so nothing ever listens here.
        pub fn accept(&self) -> io::Result<Stream> {
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    pub fn connect_at(_: &Path) -> io::Result<Stream> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
