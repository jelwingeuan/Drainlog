//! drainlogd, the user agent (no root).
//!   drainlogd           run: battery sample on will-sleep and on wake; every 5 min while
//!                       awake: sample, ingest `pmset -g log`, snapshot assertions and
//!                       scheduled wakes, rebuild the ledger and reports. CFRunLoop drives it.
//!   drainlogd --once    sample, ingest, rebuild, print the latest report, exit
//!   drainlogd --report  print the latest night, exit
//! The DB path is $DRAINLOG_DB or ~/Library/Application Support/Drainlog/drainlog.db.

use chrono::Local;
use drainlog::ledger::Ledger;
use drainlog::{battery, parse, report, store};
use rusqlite::Connection;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn pmset(args: &[&str]) -> String {
    Command::new("/usr/bin/pmset")
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn sample(conn: &Connection, source: &str) -> Option<battery::Sample> {
    let s = battery::read(now())?;
    if let Err(e) = store::insert_sample(conn, &s, source) {
        eprintln!("drainlogd: sample: {e}");
    }
    Some(s)
}

/// What is holding the Mac awake right now, and which wakes are scheduled.
fn snapshot(conn: &Connection) -> rusqlite::Result<()> {
    store::insert_assertions(
        conn,
        &parse::parse_assertions_snapshot(&pmset(&["-g", "assertions"]), now()),
    )?;
    store::insert_events(conn, &parse::parse_sched(&pmset(&["-g", "sched"]), &Local))?;
    Ok(())
}

/// shortcut: the unified log (`log show --predicate 'subsystem == "com.apple.powerd"'`) is not
/// queried in Stage 1; wake reasons come from pmset's log only.
/// shortcut: `pmset -g log` prints the whole week every time (~2 s CPU on this M1), so each
/// 5-min tick costs that; ingest less often if it shows up in the awake drain.
fn refresh(conn: &Connection) -> rusqlite::Result<(usize, usize, Ledger)> {
    let (events, asserts) = store::ingest_log(conn, &pmset(&["-g", "log"]))?;
    snapshot(conn)?;
    Ok((events, asserts, store::rebuild(conn, now())?))
}

fn print_latest(conn: &Connection) {
    match report::latest(conn) {
        Ok(Some(text)) => print!("{text}"),
        Ok(None) => println!(
            "No battery night yet. A night is a lid-close or idle sleep on battery, 2 h or longer, ending when you wake the Mac."
        ),
        Err(e) => eprintln!("drainlogd: report: {e}"),
    }
}

fn once(conn: &Connection, path: &Path) {
    match sample(conn, "once") {
        Some(s) => println!(
            "Battery now: {}% ({} of {} mAh, measured), {}.",
            s.pct,
            s.mah,
            s.max_mah,
            if s.on_ac { "on AC" } else { "on battery" }
        ),
        None => println!("Battery now: no AppleSmartBattery found."),
    }
    match refresh(conn) {
        Ok((events, asserts, l)) => {
            let b = &l.baseline;
            println!(
                "Ingested {events} new log events and {asserts} new assertion rows into {}.",
                path.display()
            );
            println!(
                "Battery nights: {}. Night-length sleeps skipped because they were on AC: {}.",
                l.nights.len(),
                l.skipped_ac
            );
            println!(
                "Baseline {:.1} mAh/h ({}{}); DarkWake {:.3} mAh/s ({}).",
                b.mah_per_h,
                b.source,
                if b.calibrated {
                    ", calibrated"
                } else {
                    ", not calibrated"
                },
                b.dark_mah_per_s,
                b.dark_source
            );
        }
        Err(e) => {
            eprintln!("drainlogd: refresh: {e}");
            std::process::exit(1);
        }
    }
    print_latest(conn);
}

fn main() {
    let path = store::default_path();
    let conn = match store::open(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("drainlogd: cannot open {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    match std::env::args().nth(1).as_deref() {
        None => agent::run(conn),
        Some("--once") => once(&conn, &path),
        Some("--report") => print_latest(&conn),
        Some(_) => {
            eprintln!("usage: drainlogd [--once | --report]");
            std::process::exit(2);
        }
    }
}

#[cfg(target_os = "macos")]
mod agent {
    use super::{Connection, refresh, sample, snapshot};
    use core_foundation::base::TCFType;
    use core_foundation::date::CFDate;
    use core_foundation::runloop::{
        CFRunLoop, CFRunLoopSource, CFRunLoopTimer, CFRunLoopTimerRef, kCFRunLoopCommonModes,
    };
    use io_kit_sys::types::{io_connect_t, io_object_t, io_service_t};
    use io_kit_sys::{
        IONotificationPortGetRunLoopSource, IONotificationPortRef, IOServiceInterestCallback,
    };
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;
    use std::time::{Duration, Instant, SystemTime};

    const TICK_S: f64 = 300.0;
    // IOMessage.h: iokit_common_msg(x) = 0xe0000000 | x.
    const CAN_SYSTEM_SLEEP: u32 = 0xe000_0270;
    const SYSTEM_WILL_SLEEP: u32 = 0xe000_0280;
    const SYSTEM_HAS_POWERED_ON: u32 = 0xe000_0300;

    // Not bound by io-kit-sys. IOKit is linked by io-kit-sys's build script.
    unsafe extern "C" {
        fn IORegisterForSystemPower(
            refcon: *mut c_void,
            port: *mut IONotificationPortRef,
            callback: IOServiceInterestCallback,
            notifier: *mut io_object_t,
        ) -> io_connect_t;
        fn IOAllowPowerChange(kernel_port: io_connect_t, notification_id: isize) -> i32;
    }

    // Callbacks all run on the main thread's run loop.
    thread_local! {
        static DB: RefCell<Option<Connection>> = const { RefCell::new(None) };
        static ROOT: Cell<io_connect_t> = const { Cell::new(0) };
        static LAST_TICK: Cell<(Instant, SystemTime)> = Cell::new((Instant::now(), SystemTime::now()));
    }

    fn with_db(f: impl FnOnce(&Connection)) {
        DB.with(|d| {
            if let Some(c) = d.borrow().as_ref() {
                f(c)
            }
        })
    }

    fn allow(arg: *mut c_void) {
        // SAFETY: the root port came from IORegisterForSystemPower; arg is the message's id.
        unsafe { IOAllowPowerChange(ROOT.get(), arg as isize) };
    }

    unsafe extern "C" fn on_power(_: *mut c_void, _: io_service_t, msg: u32, arg: *mut c_void) {
        match msg {
            CAN_SYSTEM_SLEEP => allow(arg),
            SYSTEM_WILL_SLEEP => {
                // Must stay quick: sleep waits (up to 30 s) for IOAllowPowerChange.
                with_db(|c| {
                    sample(c, "sleep");
                    if let Err(e) = snapshot(c) {
                        eprintln!("drainlogd: snapshot: {e}");
                    }
                });
                allow(arg);
            }
            // shortcut: unverified whether this also arrives for DarkWakes; when it does not,
            // DarkWakes are priced by rate (estimated), never guessed as measured.
            SYSTEM_HAS_POWERED_ON => with_db(|c| {
                sample(c, "wake");
            }),
            _ => {}
        }
    }

    extern "C" fn on_timer(_: CFRunLoopTimerRef, _: *mut c_void) {
        // A tick that came due during sleep fires right after the next wake, dark or full.
        // If the wall clock ran ahead of the uptime clock since the last tick, the Mac slept:
        // only sample, so the agent does not stretch DarkWakes. The next tick does the work.
        let (mono, wall) = LAST_TICK.get();
        LAST_TICK.set((Instant::now(), SystemTime::now()));
        let slept = wall
            .elapsed()
            .unwrap_or_default()
            .saturating_sub(mono.elapsed())
            > Duration::from_secs(60);
        with_db(|c| {
            sample(c, "timer");
            if !slept && let Err(e) = refresh(c) {
                eprintln!("drainlogd: refresh: {e}");
            }
        });
    }

    pub fn run(conn: Connection) {
        sample(&conn, "timer");
        if let Err(e) = refresh(&conn) {
            eprintln!("drainlogd: refresh: {e}");
        }
        DB.with(|d| *d.borrow_mut() = Some(conn));
        let mut port: IONotificationPortRef = std::ptr::null_mut();
        let mut notifier: io_object_t = 0;
        // SAFETY: out-pointers are valid locals; the callback matches IOServiceInterestCallback.
        let root = unsafe {
            IORegisterForSystemPower(std::ptr::null_mut(), &mut port, on_power, &mut notifier)
        };
        if root == 0 {
            eprintln!("drainlogd: IORegisterForSystemPower failed");
            std::process::exit(1);
        }
        ROOT.set(root);
        let rl = CFRunLoop::get_current();
        // SAFETY: the port's run loop source is owned by the port (get rule); the mode is a
        // CoreFoundation constant.
        unsafe {
            let src =
                CFRunLoopSource::wrap_under_get_rule(IONotificationPortGetRunLoopSource(port));
            rl.add_source(&src, kCFRunLoopCommonModes);
            let timer = CFRunLoopTimer::new(
                CFDate::now().abs_time() + TICK_S,
                TICK_S,
                0,
                0,
                on_timer,
                std::ptr::null_mut(),
            );
            rl.add_timer(&timer, kCFRunLoopCommonModes);
        }
        CFRunLoop::run_current();
    }
}

#[cfg(not(target_os = "macos"))]
mod agent {
    pub fn run(_: super::Connection) {
        eprintln!("drainlogd: the agent needs macOS");
        std::process::exit(1);
    }
}
