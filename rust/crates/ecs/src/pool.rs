//! The thread pool, when the build has one (the `parallel` feature, not on WebAssembly). Without
//! it every job runs in turn on the calling thread.

#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
mod imp {
    use rayon::prelude::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};

    pub const AVAILABLE: bool = true;

    pub fn threads() -> usize {
        rayon::current_num_threads()
    }

    /// Runs `f` on every job, on the pool when `parallel` is set and there is more than one job.
    pub fn run<J: Send>(parallel: bool, jobs: Vec<J>, f: impl Fn(J) + Send + Sync) {
        if parallel && jobs.len() > 1 {
            jobs.into_par_iter().with_max_len(1).for_each(f);
        } else {
            jobs.into_iter().for_each(f);
        }
    }
}

#[cfg(not(all(feature = "parallel", not(target_arch = "wasm32"))))]
mod imp {
    pub const AVAILABLE: bool = false;

    pub fn threads() -> usize {
        1
    }

    pub fn run<J: Send>(_parallel: bool, jobs: Vec<J>, f: impl Fn(J) + Send + Sync) {
        jobs.into_iter().for_each(f);
    }
}

pub use imp::{AVAILABLE, run, threads};
