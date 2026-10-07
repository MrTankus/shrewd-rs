
mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::mem::size_of;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{Dataset, Dictionary, Offset, Size, LENS};
use shrewd_rs::Shrewd;

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
    assert_eq!(packed.compressor_type().as_str(), D::STRATEGY, "dataset routed to the wrong strategy (len {len})");
    let packed_heap = pack.retained as usize;
    let reported_heap = packed.memory_size() - size_of::<Shrewd>();

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

/// Total bytes a container of `len` values occupies: its heap plus the value itself.
fn footprint<D: Dataset>(len: usize) -> (usize, usize) {
    let vec = measure(|| D::generate(len));
    let vec_total = vec.retained as usize + size_of::<Vec<i64>>();
    let shrewd_total = common::packed::<D>(len).memory_size();
    (vec_total, shrewd_total)
}

/// Data and unified cache sizes of CPU 0, from Linux sysfs; empty elsewhere.
fn cache_sizes() -> Vec<(String, usize)> {
    let mut caches = Vec::new();
    for index in 0.. {
        let dir = format!("/sys/devices/system/cpu/cpu0/cache/index{index}");
        let read = |name: &str| std::fs::read_to_string(format!("{dir}/{name}")).map(|s| s.trim().to_string());
        let (Ok(level), Ok(kind), Ok(size)) = (read("level"), read("type"), read("size")) else { break };
        if kind == "Instruction" {
            continue;
        }
        let bytes = match size.strip_suffix('K') {
            Some(kib) => kib.parse::<usize>().ok().map(|k| k << 10),
            None => size.strip_suffix('M').and_then(|mib| mib.parse::<usize>().ok()).map(|m| m << 20),
        };
        if let Some(bytes) = bytes {
            caches.push((format!("L{level} ({})", human_readable(bytes)), bytes));
        }
    }
    caches
}

/// Footprint per element at equal element counts, and how many elements fit in each memory budget.
fn compare_with_vec() {
    let (small, large) = (LENS[0], LENS[LENS.len() - 1]);
    println!("\nFootprint vs Vec<i64> with the same values (heap + the value itself)");
    println!("{:<10} {:>9} | {:>11} {:>11} {:>7} | {:>10} {:>10}", "strategy", "len", "Vec<i64>", "Shrewd", "ratio", "Vec B/el", "Shrewd B/el");
    println!("{}", "-".repeat(82));
    let mut costs = Vec::new();
    for (strategy, sizes) in [
        (Size::STRATEGY, [footprint::<Size>(small), footprint::<Size>(large)]),
        (Offset::STRATEGY, [footprint::<Offset>(small), footprint::<Offset>(large)]),
        (Dictionary::STRATEGY, [footprint::<Dictionary>(small), footprint::<Dictionary>(large)]),
    ] {
        for (len, (vec_total, shrewd_total)) in [small, large].into_iter().zip(sizes) {
            println!(
                "{:<10} {:>9} | {:>11} {:>11} {:>6.1}% | {:>10.3} {:>10.3}",
                strategy, len, human_readable(vec_total), human_readable(shrewd_total),
                shrewd_total as f64 / vec_total as f64 * 100.0,
                vec_total as f64 / len as f64, shrewd_total as f64 / len as f64,
            );
        }
        // Linear fit from the two lengths: cost per element, plus fixed overhead (struct, dictionary).
        let per_element = (sizes[1].1 - sizes[0].1) as f64 / (large - small) as f64;
        let fixed = sizes[0].1 as f64 - per_element * small as f64;
        costs.push((strategy, per_element, fixed));
    }

    let mut budgets = cache_sizes();
    if budgets.is_empty() {
        println!("\n(cache sizes are only read on Linux; showing main memory only)");
    }
    budgets.push(("1 GiB of RAM".to_string(), 1 << 30));
    let fits = |per_element: f64, fixed: f64, budget: usize| ((budget as f64 - fixed) / per_element).max(0.0) as usize;

    println!("\nElements that fit in each memory budget (Vec<i64>: 8 B/element + 24 B)");
    print!("{:<18} {:>14}", "budget", "Vec<i64>");
    for (strategy, _, _) in &costs {
        print!(" {:>20}", strategy);
    }
    println!();
    println!("{}", "-".repeat(33 + 21 * costs.len()));
    for (label, budget) in budgets {
        let vec_fits = fits(size_of::<i64>() as f64, size_of::<Vec<i64>>() as f64, budget);
        print!("{:<18} {:>14}", label, vec_fits);
        for &(_, per_element, fixed) in &costs {
            let shrewd_fits = fits(per_element, fixed, budget);
            print!(" {:>13} ({:>4.1}x)", shrewd_fits, shrewd_fits as f64 / vec_fits as f64);
        }
        println!();
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
    compare_with_vec();
}
