//! Keeping the screen awake while you ride. Pedalling a trainer is not
//! keyboard or mouse input, so without this the system thinks nobody is
//! there and dims, then blanks, the screen mid-ride.
//!
//! [`Awake::set`] is called every frame with whether the screen should be
//! kept awake ([`wanted`]). Only a change is passed on, to a thread of its
//! own that talks to the system, so the frame never waits on it:
//!
//! The computer is kept from sleeping for want of input too, since a
//! laptop set to sleep after some minutes idle would otherwise do so.
//!
//! - Linux (works the same on Wayland and X11): on the session bus,
//!   `org.freedesktop.ScreenSaver.Inhibit` for the display (no dimming,
//!   blanking or locking) and `org.freedesktop.PowerManagement.Inhibit`
//!   against sleep, which KDE Plasma's PowerDevil keeps apart (Xfce, LXQt
//!   and others have one or both). Where there is no screen saver service,
//!   GNOME's `org.gnome.SessionManager.Inhibit` with the idle flag does
//!   both. Each request is tied to our bus connection, which the thread
//!   keeps open, so it also ends if VeloForge crashes.
//! - Windows: `SetThreadExecutionState` (display and system), which holds
//!   only for the thread that called it and only while that thread lives.
//!   The thread lives as long as the app, which is why the calls are made
//!   there.
//! - macOS: IOKit power assertions against idle display and system sleep.
//!
//! If the system refuses, that is logged once and the ride carries on.

use crate::ride::State;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

/// Stopped this long in a running ride (auto-paused, or the clock left
/// running), the screen may sleep again: the rider has most likely left.
pub const IDLE_LIMIT_SECS: f64 = 10.0 * 60.0;

/// Whether the screen should be kept awake: only while a ride (a workout
/// too) is running, not before it starts, while paused or once finished,
/// and not after the rider has stopped pedalling for [`IDLE_LIMIT_SECS`].
pub fn wanted(enabled: bool, state: State, idle_secs: f64) -> bool {
    enabled && state == State::Running && idle_secs < IDLE_LIMIT_SECS
}

/// Asks the system to keep the screen awake, or lets it go.
trait Backend {
    /// Returns how it is held (for the log), or why it could not be.
    fn hold(&mut self) -> Result<String, String>;
    fn release(&mut self);
}

/// Turns wanted states into hold and release calls: only on a change, and
/// with a refusal logged the first time only.
struct Keeper<B: Backend> {
    backend: B,
    held: bool,
    /// The last state asked for, so a refused hold is not retried every
    /// frame (only when the screen is wanted awake again later).
    want: bool,
    told_failure: bool,
    told_how: bool,
}

impl<B: Backend> Keeper<B> {
    fn new(backend: B) -> Self {
        Keeper {
            backend,
            held: false,
            want: false,
            told_failure: false,
            told_how: false,
        }
    }

    /// Returns a line for the log, if there is something to say.
    fn apply(&mut self, want: bool) -> Option<String> {
        if want == self.want {
            return None;
        }
        self.want = want;
        if !want {
            if self.held {
                self.backend.release();
                self.held = false;
            }
            return None;
        }
        match self.backend.hold() {
            Ok(how) => {
                self.held = true;
                (!std::mem::replace(&mut self.told_how, true))
                    .then(|| format!("Keeping the screen awake while riding ({how})"))
            }
            Err(why) => (!std::mem::replace(&mut self.told_failure, true)).then(|| {
                format!(
                    "Couldn't keep the screen awake while riding: {why}. The ride \
                     carries on; the screen may dim as usual."
                )
            }),
        }
    }
}

/// The app's handle: cheap to call every frame.
#[derive(Default)]
pub struct Awake {
    want: bool,
    tx: Option<Sender<bool>>,
    log_rx: Option<Receiver<String>>,
    worker: Option<JoinHandle<()>>,
}

impl Awake {
    /// Keep the screen awake (true) or let it sleep as usual (false).
    /// Does nothing unless this differs from the last call.
    pub fn set(&mut self, on: bool) {
        if on == self.want {
            return;
        }
        self.want = on;
        if self.tx.is_none() {
            if !on {
                return;
            }
            self.start();
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(on);
        }
    }

    /// Lines for the app's log from the thread, if any.
    pub fn take_log(&mut self) -> Option<String> {
        self.log_rx.as_ref()?.try_recv().ok()
    }

    /// Let the screen sleep and end the thread, waiting for it to finish.
    pub fn stop(&mut self) {
        self.want = false;
        self.tx = None;
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }

    fn start(&mut self) {
        let (tx, rx) = mpsc::channel::<bool>();
        let (log_tx, log_rx) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("keep-awake".into())
            .spawn(move || {
                let mut keeper = Keeper::new(platform::System::default());
                // Ends when the app drops its sender: then release.
                for want in rx.iter().chain([false]) {
                    if let Some(line) = keeper.apply(want) {
                        let _ = log_tx.send(line);
                    }
                }
            });
        match worker {
            Ok(w) => {
                self.tx = Some(tx);
                self.log_rx = Some(log_rx);
                self.worker = Some(w);
            }
            Err(e) => {
                // Only when the screen is next wanted awake is this tried
                // again (`set` passes on changes only).
                let (log_tx, log_rx) = mpsc::channel();
                let _ = log_tx.send(format!(
                    "Couldn't keep the screen awake while riding: {e}. The ride carries on."
                ));
                self.log_rx = Some(log_rx);
            }
        }
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use dbus::blocking::Connection;
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(3);
    const APP: &str = "VeloForge";
    /// GNOME wants the desktop file's name.
    const APP_ID: &str = "veloforge";
    const REASON: &str = "Riding";
    /// `org.gnome.SessionManager.Inhibit` flag: the session is not idle
    /// (no dimming, blanking, locking or sleeping for want of input).
    const GNOME_IDLE: u32 = 8;

    /// The services asked, each on its own well-known name.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Via {
        /// The display: no dimming, blanking or locking (KDE, Xfce, LXQt,
        /// Cinnamon and others).
        ScreenSaver,
        /// No automatic sleep. On KDE Plasma (PowerDevil) this is apart
        /// from the screen saver request; Xfce's power manager has it too.
        PowerManagement,
        /// GNOME: both at once, through the idle flag.
        Gnome,
    }

    impl Via {
        /// (bus name, object path, interface, release method)
        fn at(self) -> (&'static str, &'static str, &'static str, &'static str) {
            match self {
                Via::ScreenSaver => (
                    "org.freedesktop.ScreenSaver",
                    "/org/freedesktop/ScreenSaver",
                    "org.freedesktop.ScreenSaver",
                    "UnInhibit",
                ),
                Via::PowerManagement => (
                    "org.freedesktop.PowerManagement",
                    "/org/freedesktop/PowerManagement/Inhibit",
                    "org.freedesktop.PowerManagement.Inhibit",
                    "UnInhibit",
                ),
                Via::Gnome => (
                    "org.gnome.SessionManager",
                    "/org/gnome/SessionManager",
                    "org.gnome.SessionManager",
                    "Uninhibit",
                ),
            }
        }
    }

    /// Holds the session bus connection: these services drop a request
    /// when the connection that made it closes.
    #[derive(Default)]
    pub struct System {
        conn: Option<Connection>,
        cookies: Vec<(Via, u32)>,
    }

    impl System {
        fn conn(&mut self) -> Result<&Connection, String> {
            // The bus may have gone away since (a logout, a restart).
            if self
                .conn
                .as_ref()
                .is_some_and(|c| !c.channel().is_connected())
            {
                self.conn = None;
            }
            if self.conn.is_none() {
                let c = Connection::new_session()
                    .map_err(|e| format!("no session bus ({})", plain(&e)))?;
                self.conn = Some(c);
            }
            Ok(self.conn.as_ref().expect("just set"))
        }

        fn inhibit(&mut self, via: Via) -> Result<(), String> {
            let (name, path, iface, _) = via.at();
            let proxy = self.conn()?.with_proxy(name, path, TIMEOUT);
            let got = if via == Via::Gnome {
                proxy.method_call::<(u32,), _, _, _>(
                    iface,
                    "Inhibit",
                    (APP_ID, 0u32, REASON, GNOME_IDLE),
                )
            } else {
                proxy.method_call::<(u32,), _, _, _>(iface, "Inhibit", (APP, REASON))
            };
            let (cookie,) = got.map_err(|e| plain(&e))?;
            self.cookies.push((via, cookie));
            Ok(())
        }
    }

    impl super::Backend for System {
        fn hold(&mut self) -> Result<String, String> {
            match self.inhibit(Via::ScreenSaver) {
                Ok(()) => {
                    // Sleep is a separate request where the desktop has
                    // one; without it the display request is still kept.
                    Ok(match self.inhibit(Via::PowerManagement) {
                        Ok(()) => "org.freedesktop.ScreenSaver and org.freedesktop.PowerManagement",
                        Err(_) => "org.freedesktop.ScreenSaver",
                    }
                    .into())
                }
                Err(first) => match self.inhibit(Via::Gnome) {
                    Ok(()) => Ok("org.gnome.SessionManager".into()),
                    Err(e) => Err(format!(
                        "the desktop answered neither org.freedesktop.ScreenSaver ({first}) \
                         nor org.gnome.SessionManager ({e})"
                    )),
                },
            }
        }

        fn release(&mut self) {
            let mut failed = false;
            for (via, cookie) in std::mem::take(&mut self.cookies) {
                let Some(conn) = &self.conn else { break };
                let (name, path, iface, method) = via.at();
                failed |= conn
                    .with_proxy(name, path, TIMEOUT)
                    .method_call::<(), _, _, _>(iface, method, (cookie,))
                    .is_err();
            }
            // If that failed, closing the connection releases it anyway.
            if failed {
                self.conn = None;
            }
        }
    }

    fn plain(e: &dbus::Error) -> String {
        e.message()
            .or(e.name())
            .unwrap_or("unknown error")
            .to_string()
    }
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::System::Power::{
        SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
    };

    /// The state set here belongs to the calling thread: the keep-awake
    /// thread, which lives as long as the app.
    #[derive(Default)]
    pub struct System {
        held: bool,
    }

    impl super::Backend for System {
        fn hold(&mut self) -> Result<String, String> {
            // SAFETY: plain flags; no pointers.
            let prev = unsafe {
                SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED)
            };
            if prev == 0 {
                Err("Windows refused SetThreadExecutionState".into())
            } else {
                self.held = true;
                Ok("SetThreadExecutionState".into())
            }
        }

        fn release(&mut self) {
            if std::mem::take(&mut self.held) {
                // SAFETY: as above.
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{c_char, c_void};

    type CFStringRef = *const c_void;
    /// `kCFStringEncodingUTF8`
    const UTF8: u32 = 0x0800_0100;
    /// `kIOPMAssertionLevelOn`
    const LEVEL_ON: u32 = 255;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            s: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(
            kind: CFStringRef,
            level: u32,
            name: CFStringRef,
            id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    #[derive(Default)]
    pub struct System {
        assertions: Vec<u32>,
    }

    fn cf(s: &std::ffi::CStr) -> CFStringRef {
        // SAFETY: a valid NUL-terminated string; the default allocator.
        unsafe { CFStringCreateWithCString(std::ptr::null(), s.as_ptr(), UTF8) }
    }

    /// One IOKit power assertion of the given type, or its error code.
    fn assert(kind: &std::ffi::CStr) -> Result<u32, i32> {
        let kind = cf(kind);
        let name = cf(c"VeloForge: riding");
        let mut id = 0u32;
        let ret = if kind.is_null() || name.is_null() {
            -1
        } else {
            // SAFETY: valid strings and a valid pointer to write the id to.
            unsafe { IOPMAssertionCreateWithName(kind, LEVEL_ON, name, &mut id) }
        };
        for s in [kind, name] {
            if !s.is_null() {
                // SAFETY: created above and not used again; IOKit keeps
                // its own copies.
                unsafe { CFRelease(s) };
            }
        }
        if ret == 0 {
            Ok(id)
        } else {
            Err(ret)
        }
    }

    impl super::Backend for System {
        fn hold(&mut self) -> Result<String, String> {
            // `kIOPMAssertionTypePreventUserIdleDisplaySleep`
            let id = assert(c"PreventUserIdleDisplaySleep")
                .map_err(|e| format!("IOKit refused the power assertion (error {e:#x})"))?;
            self.assertions.push(id);
            // `kIOPMAssertionTypePreventUserIdleSystemSleep`: the display
            // one should imply it, but ask plainly.
            if let Ok(id) = assert(c"PreventUserIdleSystemSleep") {
                self.assertions.push(id);
            }
            Ok("IOKit power assertions".into())
        }

        fn release(&mut self) {
            for id in self.assertions.drain(..) {
                // SAFETY: an assertion we created and have not released.
                unsafe { IOPMAssertionRelease(id) };
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod platform {
    #[derive(Default)]
    pub struct System {}

    impl super::Backend for System {
        fn hold(&mut self) -> Result<String, String> {
            Err("not supported on this system".into())
        }
        fn release(&mut self) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn only_a_running_ride_keeps_the_screen_awake() {
        assert!(wanted(true, State::Running, 0.0));
        assert!(wanted(true, State::Running, IDLE_LIMIT_SECS - 1.0));
        assert!(!wanted(true, State::Ready, 0.0), "not before the start");
        assert!(!wanted(true, State::Paused, 0.0), "not while paused");
        assert!(!wanted(false, State::Running, 0.0), "not when switched off");
        assert!(
            !wanted(true, State::Running, IDLE_LIMIT_SECS),
            "not once the rider has long stopped"
        );
    }

    #[derive(Clone, Default)]
    struct Fake {
        calls: Rc<RefCell<Vec<&'static str>>>,
        refuse: bool,
    }

    impl Backend for Fake {
        fn hold(&mut self) -> Result<String, String> {
            self.calls.borrow_mut().push("hold");
            if self.refuse {
                Err("no".into())
            } else {
                Ok("fake".into())
            }
        }
        fn release(&mut self) {
            self.calls.borrow_mut().push("release");
        }
    }

    #[test]
    fn holds_and_releases_only_on_a_change() {
        let fake = Fake::default();
        let mut k = Keeper::new(fake.clone());
        assert!(k.apply(false).is_none(), "nothing held: nothing to release");
        assert!(k.apply(true).is_some(), "says how, the first time");
        assert!(k.apply(true).is_none());
        k.apply(true);
        k.apply(false);
        k.apply(false);
        assert!(k.apply(true).is_none(), "says how only once");
        k.apply(false);
        assert_eq!(*fake.calls.borrow(), ["hold", "release", "hold", "release"]);
    }

    #[test]
    fn a_refusal_is_logged_once_and_never_released() {
        let fake = Fake {
            refuse: true,
            ..Default::default()
        };
        let mut k = Keeper::new(fake.clone());
        assert!(k.apply(true).is_some());
        assert!(k.apply(true).is_none(), "not retried every frame");
        assert!(k.apply(false).is_none());
        assert!(k.apply(true).is_none(), "retried, but not logged again");
        k.apply(false);
        assert_eq!(*fake.calls.borrow(), ["hold", "hold"]);
    }

    #[test]
    fn the_handle_only_starts_a_thread_when_wanted() {
        let mut a = Awake::default();
        a.set(false);
        assert!(a.worker.is_none());
        a.stop();
    }
}
