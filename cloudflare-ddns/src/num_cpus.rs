use std::num::NonZero;
use std::sync::OnceLock;
use std::thread;

#[macro_export]
macro_rules! non_zero {
    ($x: expr) => {{
        const {
            match ::std::num::NonZero::new($x) {
                Some(x) => x,
                None => panic!("non zero can't be 0"),
            }
        }
    }};
}

#[inline]
pub fn num_cpus() -> NonZero<usize> {
    static NUM_CPUS: OnceLock<NonZero<usize>> = OnceLock::new();

    #[cold]
    #[inline(never)]
    fn num_cpus_uncached() -> NonZero<usize> {
        thread::available_parallelism().unwrap_or(non_zero!(1))
    }

    *NUM_CPUS.get_or_init(num_cpus_uncached)
}
