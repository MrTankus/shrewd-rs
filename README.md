# shrewd 🦊

`shrewd` is a fast, allocation-conscious compression vector engine written in pure Rust. It transforms loose, oversized `Vec<i64>` series into tightly fitted, random-access byte structures using data-driven heuristic routing.

It is designed specifically for **low-memory random-access scenarios** (like read-only database column stores, timeseries caches, or state engines) where inflating an entire vector to read single elements would cause critical allocation spikes.

## ✨ Core Features

* **Static Enum Dispatch:** `Shrewd` wraps a private Rust `enum` that is matched at each call site, rather than dynamic `Box<dyn Trait>` pointers — no vtable indirection on the read path. The individual compressors are internal, so strategies can evolve without breaking your code.
* **Single-Pass Heuristic Routing (O(N)):** The router profiles your dataset in one linear scan (min/max and an estimated cardinality), then sizes every strategy in O(1) from those figures before allocating the optimal storage layout.
* **Streamlined Memory Footprint:** The framework avoids duplicating your dataset. It borrows the input stream via references to calculate routing metrics, only consuming the vector by value at the final step to populate the target byte buffer.
* **Symmetrical Wrapping Bounds:** Employs explicit bit-width wrapping arithmetic to ensure that heavily polarized ranges (including `i64::MIN` and `i64::MAX`) remain perfectly safe and reversible.

---

## 🛠 The Three Packing Engines

`shrewd` automatically selects from three distinct compression layouts based on your vector's mathematical distribution:

| Strategy | Mode of Action | Perfect For | Storage Target |
| :--- | :--- | :--- | :--- |
| **`Size`** | Truncation Downscaling | Raw numbers that simply happen to be small (e.g., ages, months, entity IDs). | Packs elements into the tightest matching uniform primitive (`i8`, `i16`, `i32`, `i64`). |
| **`Offset`** | Midpoint Offset Encoding | Values packed into a narrow band, wherever it sits on the number line (e.g., sensor metrics, tracking timestamps, IDs near a large base). | Subtracts the midpoint of the min/max range and stores each offset in the narrowest signed primitive (`i8`, `i16`, `i32`, `i64`) that fits the range. |
| **`Dictionary`**| Palette Encoding | Low-cardinality sequences featuring repeated occurrences of massive numbers (e.g., status codes, sparse arrays). | Maps raw items into compact, zero-indexed offset windows (`u8`, `u16`, `u32`, `u64`). |

---

## 🚀 Quick Start

Add `shrewd-rs` to your `Cargo.toml`:

```toml
[dependencies]
shrewd-rs = "0.1.0"
```

### Basic Example

Using the crate is incredibly straightforward. You pass a standard vector to `Shrewd::pack`, and interact with the resulting wrapper like an immutable native array. The `Compressor` trait must be in scope to call its methods:

```rust
use shrewd_rs::shrewd::{Shrewd, Compressor};

fn main() {
  // 1. Imagine a highly repetitive, sparse dataset
  let original_data: Vec<i64> = vec![
    i32::MIN as i64, i32::MIN as i64, i32::MIN as i64, i32::MIN as i64,
    i32::MAX as i64, i32::MAX as i64, i32::MAX as i64, i32::MAX as i64,
  ];

  // 2. Pack the data. Shrewd automatically analyzes boundaries 
  // and selects the Dictionary engine.
  let packed_vec = Shrewd::pack(original_data);

  println!("Strategy: {}", packed_vec.compressor_type()); // Output: Dictionary

  // 3. Query elements instantly in O(1) time. `get` returns an Option:
  //    Some(value) for a valid index, None when the index is out of range.
  println!("Value at index 0: {:?}", packed_vec.get(0)); // Output: Some(-2147483648)
  println!("Value at index 8: {:?}", packed_vec.get(8)); // Output: None
  if let Some(value) = packed_vec.get(4) {
    println!("Value at index 4: {}", value); // Output: 2147483647
  }
  println!("Total vector length: {}", packed_vec.length()); // Output: 8

  // 4. Inspect your precise heap + stack memory usage
  println!("Total memory footprint: {} bytes", packed_vec.size());
}
```

`Shrewd` is opaque: you cannot match on the chosen strategy. Use `compressor_type()` to see which one was selected.

Packing an empty vector is valid and returns an empty `Shrewd` with `length() == 0`.

> **Bounds checking:** `get()` never panics. It returns `Some(value)` for an index below `length()` and `None` otherwise, including for every index on an empty `Shrewd`. Use `?`, `if let`, or `unwrap_or` to handle the out-of-range case, as you would with `slice::get`.

---

## 📊 Performance Configuration

To unlock the absolute ceiling of the LLVM compiler's vectorization and instruction-pipelining optimizations, always compile your final binary using maximum release profiles. Add the following to your root `Cargo.toml`:

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
```

## 📜 License
Licensed under [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0)