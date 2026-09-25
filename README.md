# shrewd 🦔

`shrewd` is a fast, allocation-conscious compression vector engine written in pure Rust. It transforms loose, oversized `Vec<i64>` series into tightly fitted, random-access byte structures using data-driven heuristic routing.

It is designed specifically for **low-memory random-access scenarios** (like read-only database column stores, timeseries caches, or state engines) where inflating an entire vector to read single elements would cause critical allocation spikes.

## ✨ Core Features

* **Static Enum Dispatch:** Polymorphism is handled via a plain Rust `enum` matched at each call site, rather than dynamic `Box<dyn Trait>` pointers — no vtable indirection on the read path.
* **Two-Pass Heuristic Routing (O(N)):** The router profiles your dataset in exactly two linear scans (Phase 1 establishes mean/cardinality, Phase 2 checks exact wrapping deltas) before allocating the optimal storage layout.
* **Streamlined Memory Footprint:** The framework avoids duplicating your dataset. It borrows the input stream via references to calculate routing metrics, only consuming the vector by value at the final step to populate the target byte buffer.
* **Symmetrical Wrapping Bounds:** Employs explicit bit-width wrapping arithmetic to ensure that heavily polarized ranges (including `i64::MIN` and `i64::MAX`) remain perfectly safe and reversible.

---

## 🛠 The Three Packing Engines

`shrewd` automatically selects from three distinct compression layouts based on your vector's mathematical distribution:

| Strategy | Mode of Action | Perfect For | Storage Target |
| :--- | :--- | :--- | :--- |
| **`Size`** | Truncation Downscaling | Raw numbers that simply happen to be small (e.g., ages, months, entity IDs). | Packs elements into the tightest matching uniform primitive (`i8`, `i16`, `i32`, `i64`). |
| **`Average`** | Delta/Mean Encoding | Sequences that cluster closely around a central mean baseline (e.g., sensor metrics, tracking timestamps). | Squeezes negative and positive data fluctuations around a statistical center. |
| **`Dictionary`**| Palette Encoding | Low-cardinality sequences featuring repeated occurrences of massive numbers (e.g., status codes, sparse arrays). | Maps raw items into compact, zero-indexed offset windows (`u8`, `u16`, `u32`, `u64`). |

---

## 🚀 Quick Start

Add `shrewd` to your `Cargo.toml`:

```toml
[dependencies]
shrewd = "0.1.0"
```

### Basic Example

Using the crate is incredibly straightforward. You pass a standard vector to the `pack` function, and interact with the resulting wrapper exactly like an immutable native array:

```rust
use shrewd::{Shrewd, Compressor};

fn main() {
  // 1. Imagine a highly repetitive, sparse dataset
  let original_data: Vec<i64> = vec![
    i32::MIN as i64, i32::MIN as i64,
    i32::MAX as i64, i32::MAX as i64
  ];

  // 2. Pack the data. Shrewd automatically analyzes boundaries 
  // and selects the Dictionary engine.
  let packed_vec = Shrewd::pack(original_data);

  // 3. Query elements instantly in O(1) time
  println!("Value at index 0: {}", packed_vec.get(0)); // Output: -2147483648
  println!("Total vector length: {}", packed_vec.length()); // Output: 4

  // 4. Inspect your precise heap + stack memory usage
  println!("Total memory footprint: {} bytes", packed_vec.size());
}
```

> **⚠️ A note on panics:** `get()` does not perform bounds checking. Calling it with an out-of-range index will panic. If you build with `panic = "abort"` (see below), an out-of-range access will terminate the process rather than unwind, so validate indices against `length()` if that matters for your use case.

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