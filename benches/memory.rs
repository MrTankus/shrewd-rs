
mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::mem::size_of;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{Dataset, Dictionary, Offset, Size, LENS};
use shrewd_rs::shrewd::{Compressor, Shrewd};

struct Counting;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static GLOBAL: Counting = Counting;

fn grow(bytes: usize) {
    let current = CURRENT.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(current, Ordering::Relaxed);
}

fn shrink(bytes: usize) {
    CURRENT.fetch_sub(bytes, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            grow(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        shrink(layout.size());
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            grow(layout.size());
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size > layout.size() {
                grow(new_size - layout.size());
            } else {
                shrink(layout.size() - new_size);
            }
        }
        new_ptr
    }
}

struct Measured<T> {
    value: T,
    peak: usize,
    retained: isize,
}

fn measure<T>(f: impl FnOnce() -> T) -> Measured<T> {
    let start = CURRENT.load(Ordering::Relaxed);
    PEAK.store(start, Ordering::Relaxed);
    let value = f();
    Measured {
        value,
        peak: PEAK.load(Ordering::Relaxed) - start,
        retained: CURRENT.load(Ordering::Relaxed) as isize - start as isize,
    }
}

fn human_readable(bytes: usize) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.2} MiB", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.2} KiB", b as f64 / (1 << 10) as f64),
        b => format!("{b} B"),
    }
}

fn report<D: Dataset>(len: usize) {
    let data = D::generate(len);
    let input = data.len() * size_of::<i64>();

    // The input clone is made inside the measurement: `pack` consumes and frees it.
    let pack = measure(|| Shrewd::pack(data.clone()));
    let packed = pack.value;
    assert_eq!(packed.compressor_type(), D::STRATEGY, "dataset routed to the wrong strategy (len {len})");
    let packed_heap = pack.retained as usize;
    let reported_heap = packed.size() - size_of::<Shrewd>();

    let to_bytes = measure(|| packed.to_bytes());
    let bytes = to_bytes.value;
    let from_bytes = measure(|| Shrewd::from_bytes(&bytes).unwrap());

    println!(
        "{:<10} {:>9} | {:>11} {:>11} {:>11} {:>11} {:>6.1}% | {:>11} {:>11}",
        D::STRATEGY,
        len,
        human_readable(input),
        human_readable(pack.peak),
        human_readable(packed_heap),
        human_readable(reported_heap),
        packed_heap as f64 / input as f64 * 100.0,
        human_readable(to_bytes.peak),
        human_readable(from_bytes.peak),
    );
    if packed_heap != reported_heap {
        println!("  ^ size() disagrees with the measured heap by {} bytes", packed_heap as isize - reported_heap as isize);
    }
}

fn main() {
    println!(
        "{:<10} {:>9} | {:>11} {:>11} {:>11} {:>11} {:>7} | {:>11} {:>11}",
        "strategy", "len", "input", "pack peak", "packed", "size()", "ratio", "to_bytes pk", "from_bytes pk"
    );
    println!("{}", "-".repeat(118));
    for len in LENS {
        report::<Size>(len);
        report::<Offset>(len);
        report::<Dictionary>(len);
    }
}
