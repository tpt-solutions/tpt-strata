//! A small thread pool for the `parallel` feature, built on the standard library
//! only. Its worker threads live for the life of the process, so a query does
//! not start threads of its own. A job owns its input, because it may run after
//! the caller has returned to the pool's queue.

use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

/// A unit of work on the pool.
type Job = Box<dyn FnOnce() + Send + 'static>;

struct Pool {
    jobs: Mutex<mpsc::Sender<Job>>,
    threads: usize,
}

static POOL: OnceLock<Pool> = OnceLock::new();

fn pool() -> &'static Pool {
    POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..threads {
            let rx = Arc::clone(&rx);
            std::thread::spawn(move || loop {
                // Take one job at a time. The lock is released before it runs.
                let job = match rx.lock() {
                    Ok(guard) => guard.recv(),
                    Err(_) => break,
                };
                match job {
                    Ok(job) => job(),
                    Err(_) => break,
                }
            });
        }
        Pool {
            jobs: Mutex::new(tx),
            threads,
        }
    })
}

/// The number of worker threads, which is also the number of useful jobs.
pub(crate) fn threads() -> usize {
    pool().threads
}

/// Run each job on the pool and return the results in job order. A job that
/// panics has its panic re-raised on the caller, after the others finish. Jobs
/// must not call `run` themselves, or a full pool would wait on itself.
pub(crate) fn run<T: Send + 'static>(jobs: Vec<Box<dyn FnOnce() -> T + Send>>) -> Vec<T> {
    let n = jobs.len();
    let (done_tx, done_rx) = mpsc::channel::<(usize, std::thread::Result<T>)>();
    {
        let pool = pool();
        let tx = pool.jobs.lock().unwrap_or_else(|e| e.into_inner());
        for (i, job) in jobs.into_iter().enumerate() {
            let done = done_tx.clone();
            let _ = tx.send(Box::new(move || {
                let result = catch_unwind(AssertUnwindSafe(job));
                let _ = done.send((i, result));
            }));
        }
    }
    drop(done_tx);
    let mut slots: Vec<Option<T>> = (0..n).map(|_| None).collect();
    for _ in 0..n {
        let (i, result) = done_rx.recv().expect("pool workers stopped");
        match result {
            Ok(value) => slots[i] = Some(value),
            Err(panic) => resume_unwind(panic),
        }
    }
    slots
        .into_iter()
        .map(|slot| slot.expect("every job reports once"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::run;

    #[test]
    fn results_come_back_in_job_order() {
        let jobs: Vec<Box<dyn FnOnce() -> usize + Send>> = (0..64)
            .map(|i: usize| Box::new(move || i * 3) as Box<dyn FnOnce() -> usize + Send>)
            .collect();
        let out = run(jobs);
        assert_eq!(out, (0..64usize).map(|i| i * 3).collect::<Vec<_>>());
    }

    #[test]
    fn a_panicking_job_is_reraised_and_the_pool_keeps_working() {
        let jobs: Vec<Box<dyn FnOnce() -> usize + Send>> =
            vec![Box::new(|| 1), Box::new(|| panic!("job failed on purpose"))];
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(jobs)));
        assert!(caught.is_err());
        let again: Vec<Box<dyn FnOnce() -> usize + Send>> = vec![Box::new(|| 7usize)];
        assert_eq!(run(again), vec![7usize]);
    }
}
