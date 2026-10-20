use std::sync::mpsc::Receiver;
use std::time::Duration;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::sync::{mpsc, Arc};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::time::{Instant, SystemTime};

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
const POLL: Duration = Duration::from_secs(1);
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
const SUSPEND_GAP: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub idle: Option<Duration>,
    pub locked: bool,
    pub suspended: bool,
}

#[derive(Default)]
pub struct Monitor {
    rx: Option<Receiver<State>>,
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    active: Option<Arc<AtomicBool>>,
    idle: Option<Duration>,
}

impl Monitor {
    pub fn start() -> Self {
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        {
            Self::watch()
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            Self::default()
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn watch() -> Self {
        let mut monitor = Self::default();
        let (tx, rx) = mpsc::channel();
        let active = Arc::new(AtomicBool::new(false));
        let watching = Arc::clone(&active);
        if std::thread::Builder::new()
            .name("pandora-idle".to_owned())
            .spawn(move || watch(tx, watching))
            .is_ok()
        {
            monitor.rx = Some(rx);
            monitor.active = Some(active);
        }
        monitor
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    pub fn set_active(&self, active: bool) {
        if let Some(watching) = &self.active {
            watching.store(active, Ordering::Relaxed);
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    pub fn set_active(&self, _: bool) {}

    pub fn poll(&mut self) -> State {
        let mut state = State {
            idle: self.idle,
            ..State::default()
        };
        if let Some(rx) = &self.rx {
            for next in rx.try_iter() {
                self.idle = next.idle;
                state.locked |= next.locked;
                state.suspended |= next.suspended;
            }
        }
        state.idle = self.idle;
        #[cfg(target_os = "android")]
        {
            state.locked |= crate::shared::android::take_system_lock();
        }
        state
    }
}

pub fn should_lock(state: State, timeout: Duration, activity_idle: Duration) -> bool {
    state.locked
        || state.suspended
        || (timeout > Duration::ZERO
            && activity_idle >= timeout
            && state.idle.is_none_or(|idle| idle >= timeout))
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn suspended(sleep: Duration, work: Duration) -> bool {
    sleep.saturating_sub(work) >= SUSPEND_GAP
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn watch(tx: mpsc::Sender<State>, active: Arc<AtomicBool>) {
    let mut wall = SystemTime::now();
    let mut monotonic = Instant::now();
    let mut clocks = platform::clocks();
    loop {
        std::thread::sleep(POLL);
        if !active.load(Ordering::Relaxed) {
            wall = SystemTime::now();
            monotonic = Instant::now();
            clocks = platform::clocks();
            continue;
        }
        let now_wall = SystemTime::now();
        let now_monotonic = Instant::now();
        let elapsed_wall = now_wall.duration_since(wall).unwrap_or_default();
        let elapsed_monotonic = now_monotonic.duration_since(monotonic);
        let next_clocks = platform::clocks();
        let suspended = match (clocks, next_clocks) {
            (Some((sleep_before, work_before)), Some((sleep_now, work_now))) => {
                let sleep = sleep_now.saturating_sub(sleep_before);
                let work = work_now.saturating_sub(work_before);
                suspended(sleep, work)
            }
            _ => elapsed_wall.saturating_sub(elapsed_monotonic) >= SUSPEND_GAP,
        };
        wall = now_wall;
        monotonic = now_monotonic;
        clocks = next_clocks;

        let state = State {
            idle: system_idle_time::get_idle_time().ok(),
            locked: platform::locked(),
            suspended,
        };
        if tx.send(state).is_err() {
            return;
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::time::Duration;
    use zbus::blocking::{Connection, Proxy};

    pub fn clocks() -> Option<(Duration, Duration)> {
        fn read(clock: libc::clockid_t) -> Option<Duration> {
            let mut value = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            if unsafe { libc::clock_gettime(clock, &mut value) } != 0 || value.tv_sec < 0 {
                return None;
            }
            Some(Duration::new(value.tv_sec as u64, value.tv_nsec as u32))
        }

        Some((read(libc::CLOCK_BOOTTIME)?, read(libc::CLOCK_MONOTONIC)?))
    }

    pub fn locked() -> bool {
        (|| {
            let connection = Connection::system().ok()?;
            let manager = Proxy::new(
                &connection,
                "org.freedesktop.login1",
                "/org/freedesktop/login1",
                "org.freedesktop.login1.Manager",
            )
            .ok()?;
            let path: zbus::zvariant::OwnedObjectPath = manager
                .call("GetSessionByPID", &(std::process::id(),))
                .ok()?;
            let session = Proxy::new(
                &connection,
                "org.freedesktop.login1",
                path.as_str(),
                "org.freedesktop.login1.Session",
            )
            .ok()?;
            session.get_property("LockedHint").ok()
        })()
        .unwrap_or(false)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::time::Duration;
    use windows_sys::Win32::System::StationsAndDesktops::{
        CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, DESKTOP_READOBJECTS, UOI_NAME,
    };
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    use windows_sys::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime;

    pub fn clocks() -> Option<(Duration, Duration)> {
        let mut working = 0;
        (unsafe { QueryUnbiasedInterruptTime(&mut working) } != 0).then(|| {
            (
                Duration::from_millis(unsafe { GetTickCount64() }),
                Duration::from_nanos(working.saturating_mul(100)),
            )
        })
    }

    pub fn locked() -> bool {
        unsafe {
            let desktop = OpenInputDesktop(0, 0, DESKTOP_READOBJECTS);
            if desktop.is_null() {
                return true;
            }
            let mut name = [0u16; 64];
            let mut needed = 0;
            let ok = GetUserObjectInformationW(
                desktop,
                UOI_NAME,
                name.as_mut_ptr().cast(),
                std::mem::size_of_val(&name) as u32,
                &mut needed,
            );
            CloseDesktop(desktop);
            if ok == 0 {
                return true;
            }
            let end = name.iter().position(|c| *c == 0).unwrap_or(name.len());
            String::from_utf16_lossy(&name[..end]) != "Default"
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{c_char, c_void};
    use std::sync::OnceLock;
    use std::time::Duration;

    type Clock = unsafe extern "C" fn() -> u64;

    static CONTINUOUS: OnceLock<Option<Clock>> = OnceLock::new();

    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }

    extern "C" {
        fn mach_absolute_time() -> u64;
        fn mach_timebase_info(info: *mut Timebase) -> i32;
    }

    #[link(name = "System")]
    extern "C" {
        fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    fn continuous() -> Option<u64> {
        let clock = CONTINUOUS.get_or_init(|| unsafe {
            let handle = dlopen(std::ptr::null(), 1);
            if handle.is_null() {
                return None;
            }
            let symbol = dlsym(handle, c"mach_continuous_time".as_ptr());
            (!symbol.is_null()).then(|| std::mem::transmute::<*mut c_void, Clock>(symbol))
        });
        clock.map(|clock| unsafe { clock() })
    }

    pub fn clocks() -> Option<(Duration, Duration)> {
        unsafe {
            let mut info = Timebase { numer: 0, denom: 0 };
            let continuous = continuous()?;
            if mach_timebase_info(&mut info) != 0 || info.denom == 0 {
                return None;
            }
            let duration = |ticks: u64| {
                let nanos = (ticks as u128 * info.numer as u128) / info.denom as u128;
                Duration::from_nanos(nanos.min(u64::MAX as u128) as u64)
            };
            Some((duration(continuous), duration(mach_absolute_time())))
        }
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            string: *const c_char,
            encoding: u32,
        ) -> *const c_void;
        fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        fn CFBooleanGetValue(value: *const c_void) -> u8;
        fn CFRelease(value: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IORegistryGetRootEntry(port: u32) -> u32;
        fn IORegistryEntryCreateCFProperty(
            entry: u32,
            key: *const c_void,
            allocator: *const c_void,
            options: u32,
        ) -> *const c_void;
        fn IOObjectRelease(object: u32) -> i32;
    }

    pub fn locked() -> bool {
        unsafe {
            let root = IORegistryGetRootEntry(0);
            if root == 0 {
                return false;
            }
            let users_key =
                CFStringCreateWithCString(std::ptr::null(), c"IOConsoleUsers".as_ptr(), 0x08000100);
            if users_key.is_null() {
                IOObjectRelease(root);
                return false;
            }
            let users = IORegistryEntryCreateCFProperty(root, users_key, std::ptr::null(), 0);
            IOObjectRelease(root);
            CFRelease(users_key);
            if users.is_null() || CFArrayGetCount(users) == 0 {
                if !users.is_null() {
                    CFRelease(users);
                }
                return false;
            }
            let user = CFArrayGetValueAtIndex(users, 0);
            let lock_key = CFStringCreateWithCString(
                std::ptr::null(),
                c"CGSSessionScreenIsLocked".as_ptr(),
                0x08000100,
            );
            if user.is_null() || lock_key.is_null() {
                if !lock_key.is_null() {
                    CFRelease(lock_key);
                }
                CFRelease(users);
                return false;
            }
            let value = CFDictionaryGetValue(user, lock_key);
            let locked = !value.is_null() && CFBooleanGetValue(value) != 0;
            CFRelease(lock_key);
            CFRelease(users);
            locked
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_activity_prevents_lock_while_the_app_is_away() {
        let timeout = Duration::from_secs(900);
        assert!(!should_lock(
            State {
                idle: Some(Duration::from_secs(1)),
                ..State::default()
            },
            timeout,
            timeout,
        ));
    }

    #[test]
    fn app_activity_extends_the_system_idle_deadline() {
        let timeout = Duration::from_secs(900);
        assert!(!should_lock(
            State {
                idle: Some(timeout),
                ..State::default()
            },
            timeout,
            Duration::ZERO,
        ));
    }

    #[test]
    fn lock_and_sleep_events_override_never_lock() {
        let never = Duration::ZERO;
        assert!(should_lock(
            State {
                locked: true,
                ..State::default()
            },
            never,
            never,
        ));
        assert!(should_lock(
            State {
                suspended: true,
                ..State::default()
            },
            never,
            never,
        ));
    }

    #[test]
    fn local_activity_is_the_fallback_when_system_idle_is_unavailable() {
        let timeout = Duration::from_secs(900);
        assert!(should_lock(State::default(), timeout, timeout,));
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn suspend_clock_detects_only_time_outside_the_working_state() {
        assert!(suspended(Duration::from_secs(8), Duration::from_secs(1)));
        assert!(!suspended(Duration::from_secs(1), Duration::from_secs(1)));
    }
}
