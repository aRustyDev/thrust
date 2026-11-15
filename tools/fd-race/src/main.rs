//! Does a pipe's EOF get delayed by a child that was spawned at the same moment by another thread?
//!
//! nextest runs every test as a child process with piped stdout and stderr, from many tasks at once, and calls a test LEAK when the
//! pipes are still open 200 ms after the process exited. A test that spawns nothing can only be held by ANOTHER process that inherited
//! its pipe's write end. On macOS std makes a pipe with `pipe()` and then `fcntl(FD_CLOEXEC)`, two steps (there is no `pipe2`); a
//! `fork` by another thread between them copies the write end without the flag, and the child keeps it across `exec` for as long as it
//! lives. This program is that, alone: `THREADS` threads each spawn `PER_THREAD` children, one in `LONG_EVERY` of them lives
//! `LONG_MS` ms (the holder) and the others exit at once; for each short one it measures how long after `wait()` its stdout takes to
//! reach EOF. A delay is a pipe held by someone else. Std only, no test of ours in it.
//!
//!   fdrace <threads> <per-thread> <long-every> <long-ms>

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    let a: Vec<u64> = std::env::args().skip(1).filter_map(|s| s.parse().ok()).collect();
    let (threads, per_thread, long_every, long_ms) = match a.as_slice() {
        [t, p, e, m] => (*t as usize, *p as usize, *e as usize, *m),
        _ => {
            eprintln!("usage: fdrace <threads> <per-thread> <long-every> <long-ms>");
            std::process::exit(2);
        }
    };
    let short = Arc::new(AtomicUsize::new(0));
    let delayed = Arc::new(AtomicUsize::new(0));
    let worst_ms = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicUsize::new(0));
    let start = Instant::now();
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let (short, delayed, worst_ms, failed) = (short.clone(), delayed.clone(), worst_ms.clone(), failed.clone());
            thread::spawn(move || {
                for i in 0..per_thread {
                    let long = i % long_every == 0;
                    let script = if long { format!("sleep {}", long_ms as f64 / 1000.0) } else { "exit 0".to_string() };
                    let spawned = Command::new("/bin/sh")
                        .arg("-c")
                        .arg(script)
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn();
                    let mut child = match spawned {
                        Ok(c) => c,
                        Err(_) => {
                            failed.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    };
                    if child.wait().is_err() {
                        failed.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    let exited = Instant::now();
                    let mut sink = Vec::new();
                    if let Some(mut out) = child.stdout.take() {
                        let _ = out.read_to_end(&mut sink);
                    }
                    if long {
                        continue;
                    }
                    short.fetch_add(1, Ordering::Relaxed);
                    let ms = exited.elapsed().as_millis() as usize;
                    if ms >= 100 {
                        delayed.fetch_add(1, Ordering::Relaxed);
                        worst_ms.fetch_max(ms, Ordering::Relaxed);
                    }
                }
            })
        })
        .collect();
    for h in handles {
        let _ = h.join();
    }
    println!(
        "threads={threads} per_thread={per_thread} long_every={long_every} long_ms={long_ms}: short children={} spawn/wait failures={} \
         with stdout EOF delayed >=100ms after exit={} (worst {} ms), {:.1}s",
        short.load(Ordering::Relaxed),
        failed.load(Ordering::Relaxed),
        delayed.load(Ordering::Relaxed),
        worst_ms.load(Ordering::Relaxed),
        start.elapsed().as_secs_f64()
    );
    let _ = Duration::from_secs(0);
}
