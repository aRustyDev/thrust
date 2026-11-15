// Native timing of zxcvbn (research/0015, section 4.1): the first call builds six dictionaries (lazy_static), later calls do not.
// Release build, ONE PROCESS PER RUN (run.sh) so every run's first call is cold. This is a LOWER BOUND for the Worker (wasm in V8 is
// slower than native), not the platform's figure: that is tools/measure-zxcvbn/cloudflare. A counting allocator reports the heap the
// first call leaves behind (the dictionaries, which live as long as the isolate does) and the peak while it builds them; on wasm32 a
// `&str` is 8 bytes, not 16, so the Worker's figure should be smaller (derived, not measured).
//
// One line is printed, in microseconds:
//   first_cold_us   the first strength check (two zxcvbn calls: the first 72 characters and the last 72), dictionaries included
//   second_us       a second strength check on another password
//   worst72_us      a 72-character password
//   warm_mean_us    the mean of 200 warm strength checks
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use zxcvbn_first_call::plane_score;

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            let now = LIVE.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = System.realloc(p, l, new);
        if !q.is_null() {
            if new >= l.size() {
                let now = LIVE.fetch_add(new - l.size(), Ordering::Relaxed) + (new - l.size());
                PEAK.fetch_max(now, Ordering::Relaxed);
            } else {
                LIVE.fetch_sub(l.size() - new, Ordering::Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static A: Counting = Counting;

fn main() {
    let weak = "Password123!";
    let strong = "correct-Horse-battery-staple-9X";
    let worst = "aB3$xY7!kQ".chars().cycle().take(72).collect::<String>();

    let live_before = LIVE.load(Ordering::Relaxed);
    let t0 = Instant::now();
    let s_weak = plane_score(weak);
    let first = t0.elapsed();
    let live_after = LIVE.load(Ordering::Relaxed);
    let peak = PEAK.load(Ordering::Relaxed);

    let t1 = Instant::now();
    let s_strong = plane_score(strong);
    let second = t1.elapsed();

    let t2 = Instant::now();
    let s_worst = plane_score(&worst);
    let worst_t = t2.elapsed();

    let n = 200;
    let t3 = Instant::now();
    for _ in 0..n {
        std::hint::black_box(plane_score(std::hint::black_box(strong)));
    }
    let warm = t3.elapsed() / n;

    println!(
        "first_cold_us={} second_us={} worst72_us={} warm_mean_us={} retained_heap_bytes={} peak_heap_bytes={} scores weak={:?} strong={:?} worst={:?}",
        first.as_micros(),
        second.as_micros(),
        worst_t.as_micros(),
        warm.as_micros(),
        live_after.saturating_sub(live_before),
        peak,
        s_weak,
        s_strong,
        s_worst
    );
}
