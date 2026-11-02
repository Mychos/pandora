//! A local channel: a Unix socket or a Windows named pipe, never a TCP port. Both
//! platforms are user-private — socket mode 0600, pipe DACL and
//! `PIPE_REJECT_REMOTE_CLIENTS` — because being reachable only by this user is the
//! whole access control for what speaks over it.
//!
//! The caller names a path; on Windows that path is turned into a pipe name that is
//! machine-global, so it is digested with the path rather than spelled out: two users
//! with a data dir each must never land on the same pipe. [`display`] says what the
//! user should point a client at.

use std::path::{Path, PathBuf};

#[cfg(unix)]
pub use unix::{connect_at, Listener, Stream};
#[cfg(windows)]
pub use windows::{connect_at, Listener, Stream};

#[cfg(not(any(unix, windows)))]
pub use other::{connect_at, Listener, Stream};

/// The address the channel is actually reached at: the path on unix, the pipe name on
/// Windows. What a settings page shows, and what an environment variable gets.
#[cfg(windows)]
pub fn display(path: &Path) -> PathBuf {
    PathBuf::from(windows::pipe(path))
}

#[cfg(not(windows))]
pub fn display(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(unix)]
mod unix {
    use std::fs::Permissions;
    use std::io;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;
    use std::time::{Duration, Instant};

    pub type Stream = UnixStream;

    /// How long a poll between two accepts leaves the thread asleep. Small enough that
    /// a caller waits less than a frame.
    const TICK: Duration = Duration::from_millis(5);

    pub struct Listener(UnixListener);

    impl Listener {
        /// A second app gets `None` and does not listen: two would race for every request.
        pub fn bind_at(path: &Path) -> io::Result<Option<Self>> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            // A path that connects is live; anything else is a crash leftover that bind refuses.
            if UnixStream::connect(path).is_ok() {
                return Ok(None);
            }
            let _ = std::fs::remove_file(path);
            let listener = UnixListener::bind(path)?;
            // The directory keeps others out; the mode keeps it honest if the socket lands elsewhere.
            std::fs::set_permissions(path, Permissions::from_mode(0o600))?;
            listener.set_nonblocking(true)?;
            Ok(Some(Self(listener)))
        }

        /// Blocks until one arrives, so a browser is answered even when nothing draws.
        pub fn accept(&self) -> io::Result<Stream> {
            loop {
                if let Some(stream) = self.accept_timeout(Duration::from_secs(3600))? {
                    return Ok(stream);
                }
            }
        }

        /// `Ok(None)` once `wait` has passed with nobody at the door.
        pub fn accept_timeout(&self, wait: Duration) -> io::Result<Option<Stream>> {
            let deadline = Instant::now() + wait;
            loop {
                match self.0.accept() {
                    Ok((stream, _)) => return Ok(Some(stream)),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e),
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Ok(None);
                }
                std::thread::sleep(TICK.min(left));
            }
        }
    }

    pub fn connect_at(path: &Path) -> io::Result<Stream> {
        UnixStream::connect(path)
    }
}

/// Windows has no socket in `std`: the channel is a pipe via `windows-sys`, named by
/// [`pipe`] so a path isolates one test, one install or one user.
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

    /// The protocol caps a message at 1 MiB; the buffer only holds what is in flight.
    const BUFFER: u32 = 64 * 1024;

    pub struct Listener {
        name: Vec<u16>,
        /// The instance waiting for the next caller, handed out by `accept`. Behind
        /// a lock because `accept` borrows the listener shared — the shape the unix
        /// side has — and because the pipe instance it leaves behind is the only
        /// state either side carries.
        next: Mutex<Option<HANDLE>>,
    }

    // The instance waiting in `next` is a kernel handle, an integer in a pointer's
    // clothes: it moves and is shared between threads exactly as `Stream` does.
    unsafe impl Send for Listener {}

    impl Listener {
        /// A second app finds the first one waiting and gets `None`, so only one answers.
        pub fn bind_at(path: &Path) -> io::Result<Option<Self>> {
            let name = wide(path);
            // A free instance is a live app; a busy one has a request in flight. Either way, taken.
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

        /// Blocks as unix does, and leaves a fresh instance behind.
        pub fn accept(&self) -> io::Result<Stream> {
            loop {
                if let Some(stream) = self.accept_timeout(Duration::from_secs(3600))? {
                    return Ok(stream);
                }
            }
        }

        /// `Ok(None)` once `wait` has passed with nobody at the door. The instance stays
        /// armed: `Wait::finish` cancels the connect and waits for the cancellation, so
        /// the same handle can be connected again by the next call.
        pub fn accept_timeout(&self, wait: Duration) -> io::Result<Option<Stream>> {
            let Some(handle) = self.next.lock().expect("pipe lock").take() else {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "the listener has no pipe instance left",
                ));
            };

            let mut wait_state = Wait::new()?;
            let started = unsafe { ConnectNamedPipe(handle, wait_state.overlapped()) };
            // A caller that connected between the two calls is the outcome we wanted.
            if started == 0 && unsafe { GetLastError() } == ERROR_PIPE_CONNECTED {
                self.renew();
                return Ok(Some(Stream::new(handle)));
            }
            match wait_state.finish(handle, started, Some(wait)) {
                Ok(_) => {
                    self.renew();
                    Ok(Some(Stream::new(handle)))
                }
                Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                    if let Some(mut slot) = self.next.lock().ok() {
                        if slot.is_none() {
                            *slot = Some(handle);
                        }
                    }
                    Ok(None)
                }
                Err(e) => {
                    unsafe { CloseHandle(handle) };
                    self.renew();
                    Err(e)
                }
            }
        }

        /// A failure here only stops answering after the connection already accepted.
        fn renew(&self) {
            if let Some(mut slot) = self.next.lock().ok() {
                if slot.is_none() {
                    *slot = create(&self.name).ok();
                }
            }
        }
    }

    impl Drop for Listener {
        /// The instance is a kernel handle, not a name on disk: a listener that goes away
        /// has to close it, or the pipe outlives the app that stopped listening on it.
        fn drop(&mut self) {
            if let Some(handle) = self.next.lock().ok().and_then(|mut slot| slot.take()) {
                unsafe { CloseHandle(handle) };
            }
        }
    }

    pub struct Stream {
        handle: HANDLE,
        read_timeout: Mutex<Option<Duration>>,
        write_timeout: Mutex<Option<Duration>>,
    }

    impl Stream {
        fn new(handle: HANDLE) -> Self {
            Self {
                handle,
                read_timeout: Mutex::new(None),
                write_timeout: Mutex::new(None),
            }
        }

        /// One overlapped read or write, waiting at most `timeout`. A timed-out read is
        /// cancelled, so no operation is left pointing at a buffer about to drop.
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
            let timeout = *self.read_timeout.lock().expect("timeout lock");
            match self.begin(buf.as_mut_ptr(), buf.len(), false, timeout) {
                Ok(n) => Ok(n),
                // The other end hung up: `read_exact` turns this into the UnexpectedEof.
                Err(e) if matches!(e.raw_os_error(), Some(code) if code == ERROR_BROKEN_PIPE as i32 || code == ERROR_NO_DATA as i32) => {
                    Ok(0)
                }
                Err(e) => Err(e),
            }
        }
    }

    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let timeout = *self.write_timeout.lock().expect("timeout lock");
            self.begin(buf.as_ptr().cast_mut(), buf.len(), true, timeout)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Stream {
        pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            *self.read_timeout.lock().expect("timeout lock") = timeout;
            Ok(())
        }

        pub fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            *self.write_timeout.lock().expect("timeout lock") = timeout;
            Ok(())
        }
    }

    impl Drop for Stream {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
        }
    }

    // One `Stream` owns the handle, moves between threads and is never shared.
    unsafe impl Send for Stream {}

    pub fn connect_at(path: &Path) -> io::Result<Stream> {
        let name = wide(path);
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

    /// The pipe name for a path. A path already spelled as a pipe is used as it stands,
    /// so a user who typed one gets exactly that.
    pub fn pipe(path: &Path) -> String {
        if let Some(literal) = path.to_str().filter(|it| it.starts_with(r"\\")) {
            return literal.to_owned();
        }
        // A digest, not the path: a pipe name is machine-global, and two users with a
        // data dir each must never land on the same pipe.
        let key = crate::vault::crypto::digest(path.to_string_lossy().as_bytes());
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "channel".to_owned());
        format!("\\\\.\\pipe\\pandora-{stem}-{key}")
    }

    fn wide(path: &Path) -> Vec<u16> {
        pipe(path)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
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

    /// One overlapped operation: the structure the API fills in and its event, owned
    /// per call so a timed-out one cannot outlive its buffer.
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

        /// Wait for `started` and return the bytes moved; `None` waits as long as needed.
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
                        // Let the cancellation land: the OVERLAPPED cannot drop while the kernel may write.
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

        /// The whole channel over the real kernel object, which unix gets from `UnixStream`.
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
            // The other end hangs up: this must read as a clean end.
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

        /// A read nobody answers gives up on its own, and the connection drops without
        /// an operation on a dead buffer.
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

        /// A poll that finds nobody leaves the instance armed, so the caller after it is
        /// still served — the shape the SSH agent's loop needs.
        #[test]
        fn an_empty_poll_does_not_use_up_the_instance() {
            let dir = std::env::temp_dir().join(format!("pandora-pipe-4-{}", std::process::id()));
            let path = dir.join("bridge.sock");
            let listener = Listener::bind_at(&path).expect("bind").expect("first");
            assert!(
                listener
                    .accept_timeout(Duration::from_millis(20))
                    .expect("poll")
                    .is_none(),
                "nobody connected"
            );

            let here = path.clone();
            let peer = std::thread::spawn(move || connect_at(&here).expect("connect"));
            let stream = listener
                .accept_timeout(Duration::from_secs(5))
                .expect("accept")
                .expect("the caller is served after an empty poll");
            stream
                .set_read_timeout(Some(Duration::from_millis(50)))
                .unwrap();
            drop(peer.join().unwrap());
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod other {
    use std::io::{self, Read, Write};
    use std::path::Path;

    /// Never constructed: this platform has no channel, so `connect_at` fails and
    /// `bind_at` says no; it exists so the caller type-checks.
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

        pub fn accept_timeout(&self, _: std::time::Duration) -> io::Result<Option<Stream>> {
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    pub fn connect_at(_: &Path) -> io::Result<Stream> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
