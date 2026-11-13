//! Work on worker threads, results taken in the original order.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use crate::{Error, Result};
use rolid_text::txt;

struct State<T> {
    /// Next job a worker looks at.
    next: usize,
    /// Jobs taken by the consumer so far.
    base: usize,
    /// Cost of the jobs claimed and not yet taken.
    used: u64,
    ready: HashMap<usize, std::thread::Result<T>>,
    stop: bool,
}

/// Run `work` for the jobs with a `cost` (`Some`) on `threads` workers and
/// hand every job to `consume` in order: with its result, or with `None` for a
/// job without a cost (the consumer does it itself). Workers stay at most a
/// few jobs and `budget` of cost ahead of the consumer. `work` gets a flag set
/// when the consumer gave up (error, cancel) to stop early. With one thread
/// everything runs on the calling thread.
pub(crate) fn ordered<J, T, K, W, C>(
    jobs: &[J],
    threads: usize,
    budget: u64,
    cost: K,
    work: W,
    cancel: &AtomicBool,
    mut consume: C,
) -> Result<()>
where
    J: Sync,
    T: Send,
    K: Fn(&J) -> Option<u64> + Sync,
    W: Fn(&J, &AtomicBool) -> T + Sync,
    C: FnMut(&J, Option<T>) -> Result<()>,
{
    let abort = AtomicBool::new(false);
    if threads <= 1 {
        for j in jobs {
            let r = cost(j).map(|_| work(j, &abort));
            consume(j, r)?;
        }
        return Ok(());
    }
    let ahead = threads * 4 + 8;
    let st = Mutex::new(State { next: 0, base: 0, used: 0, ready: HashMap::new(), stop: false });
    let cv = Condvar::new();
    let lock = || st.lock().unwrap_or_else(|e| e.into_inner());
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| loop {
                let (i, _c) = {
                    let mut g = lock();
                    loop {
                        if g.stop {
                            return;
                        }
                        while g.next < jobs.len() && cost(&jobs[g.next]).is_none() {
                            g.next += 1;
                        }
                        if g.next >= jobs.len() {
                            return;
                        }
                        let c = cost(&jobs[g.next]).unwrap_or(0);
                        if g.next < g.base + ahead && (g.used == 0 || g.used + c <= budget) {
                            let i = g.next;
                            g.next += 1;
                            g.used += c;
                            break (i, c);
                        }
                        g = cv.wait(g).unwrap_or_else(|e| e.into_inner());
                    }
                };
                let r = catch_unwind(AssertUnwindSafe(|| work(&jobs[i], &abort)));
                lock().ready.insert(i, r);
                cv.notify_all();
            });
        }
        let res = (|| {
            for (i, j) in jobs.iter().enumerate() {
                let r = match cost(j) {
                    None => None,
                    Some(c) => {
                        let mut g = lock();
                        loop {
                            if let Some(r) = g.ready.remove(&i) {
                                g.used -= c;
                                break Some(r);
                            }
                            if cancel.load(Ordering::Relaxed) {
                                return Err(Error::Canceled);
                            }
                            g = cv.wait_timeout(g, Duration::from_millis(100)).unwrap_or_else(|e| e.into_inner()).0;
                        }
                    }
                };
                lock().base = i + 1;
                cv.notify_all();
                let r = match r {
                    Some(Err(_)) => return Err(Error::Corrupt(txt!("internal error while compressing"))),
                    Some(Ok(x)) => Some(x),
                    None => None,
                };
                consume(j, r)?;
            }
            Ok(())
        })();
        if res.is_err() {
            abort.store(true, Ordering::Relaxed);
        }
        lock().stop = true;
        cv.notify_all();
        res
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn keeps_the_order_and_runs_inline_jobs_on_the_caller() {
        let jobs: Vec<u64> = (0..200).collect();
        let running = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let mut seen = Vec::new();
        let caller = std::thread::current().id();
        let no = AtomicBool::new(false);
        ordered(
            &jobs,
            4,
            1000,
            |j| (j % 7 != 0).then_some(10),
            |j, _| {
                let n = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                std::thread::sleep(Duration::from_micros(200 * (j % 5)));
                running.fetch_sub(1, Ordering::SeqCst);
                (j * 2, std::thread::current().id())
            },
            &no,
            |j, r| {
                match r {
                    Some((v, id)) => {
                        assert_eq!(v, j * 2);
                        assert_ne!(id, caller);
                    }
                    None => assert_eq!(j % 7, 0),
                }
                seen.push(*j);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(seen, jobs);
        assert!(peak.load(Ordering::SeqCst) <= 4);
    }

    #[test]
    fn an_error_stops_the_workers() {
        let jobs: Vec<u32> = (0..10_000).collect();
        let done = AtomicUsize::new(0);
        let no = AtomicBool::new(false);
        let r = ordered(
            &jobs,
            3,
            u64::MAX,
            |_| Some(1),
            |_, _| {
                done.fetch_add(1, Ordering::SeqCst);
            },
            &no,
            |j, _| if *j == 5 { Err(Error::Corrupt(rolid_text::Text::new("x"))) } else { Ok(()) },
        );
        assert!(r.is_err());
        assert!(done.load(Ordering::SeqCst) < 100);
    }

    #[test]
    fn one_thread_is_sequential() {
        let jobs = [1, 2, 3];
        let no = AtomicBool::new(false);
        let mut out = Vec::new();
        ordered(&jobs, 1, 0, |_| Some(1), |j, _| j * 10, &no, |_, r| {
            out.push(r.unwrap());
            Ok(())
        })
        .unwrap();
        assert_eq!(out, [10, 20, 30]);
    }
}
