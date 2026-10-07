# shrewd 🦊

`shrewd` stores a `Vec<i64>` in 2–8× less memory and still lets you read any element by index. It looks at your data once, picks the most compact of three encodings, and keeps every value at a fixed position, so a lookup is a single memory read.

The trade-off: each read costs a little more CPU work than reading a `Vec<i64>`, in exchange for much less memory. When the memory saving changes where your data lives (in a faster cache level, or in RAM instead of on disk), `shrewd` is faster than a `Vec<i64>` as well as smaller. When it doesn't, a `Vec<i64>` is faster.

```rust
use shrewd_rs::Shrewd;

let packed = Shrewd::pack(vec![1_000_000, 1_000_005, 1_000_010]);

assert_eq!(packed.get(1), Some(1_000_005));
assert_eq!(packed.len(), 3);
println!("{} bytes, stored as {}", packed.memory_size(), packed.compressor_type());
```

No dependencies, no `unsafe` code.

---

## 🎯 Is shrewd Right for You?

**A good fit:**

* **Large read-only lookup tables** read by index, such as ID mappings, per-entity attributes, or the integer columns of a data snapshot. With 262,144 values, a lookup takes 4.2–4.7 ns instead of 10.3 ns, because the smaller table still fits in the CPU cache.
* **Caches and datasets limited by RAM.** 1 GiB holds 537 million to 1.07 billion values instead of 134 million.
* **Chunks and partitions in columnar storage.** A column that needs 64 bits across a whole table is often narrow inside one partition: timestamps in milliseconds in a 1-minute partition take 2 bytes instead of 8.
* **Bulk reads of data that does not fit in the cache.** With 67 million values, copying values out takes 0.04–0.05 ns per value instead of 0.20 ns (when built for the target CPU).
* **Storing and sending data.** The serialized form is as small as the data in memory, and the same on every platform.

**Not a good fit:**

* **Data that fits in the cache and is mostly scanned.** A `Vec<i64>` loop uses SIMD and is faster (0.09 ns per value against 0.18–0.42 ns).
* **Data that changes.** `shrewd` is read-only once packed.
* **Ranges you know when writing the code.** If all values fit in an `i16`, a `Vec<i16>` gives the same saving with no decoding at all.

The full measurements behind these numbers are in Performance below.

---

## 🚀 Getting Started

Add `shrewd-rs` to your `Cargo.toml`:

```toml
[dependencies]
shrewd-rs = "0.1.0"
```

Everything is available from the crate root: `Shrewd`, `CompressorType`, `DecodeError`, `Iter` and `DEFAULT_BLOCK_SIZE`.

```rust
use shrewd_rs::{CompressorType, Shrewd};

fn main() {
    // A small, highly repetitive dataset of large values.
    let data: Vec<i64> = vec![
        i32::MIN as i64, i32::MIN as i64, i32::MIN as i64, i32::MIN as i64,
        i32::MAX as i64, i32::MAX as i64, i32::MAX as i64, i32::MAX as i64,
    ];

    // `pack` consumes the vector and picks the best encoding.
    let packed = Shrewd::pack(data);
    assert_eq!(packed.compressor_type(), CompressorType::Dictionary);

    // Random access. `get` returns None past the end instead of panicking.
    assert_eq!(packed.get(0), Some(i32::MIN as i64));
    assert_eq!(packed.get(4), Some(i32::MAX as i64));
    assert_eq!(packed.get(8), None);

    assert_eq!(packed.len(), 8);
    println!("Footprint: {} bytes", packed.memory_size()); // heap + the Shrewd value itself
}
```

---

## 🏗 Building a Shrewd

There are three ways to build one. All of them take ownership of the input, because the encoding needs every value before it can be chosen.

| You have… | Use | Notes |
| :--- | :--- | :--- |
| A `Vec<i64>` | `Shrewd::pack(values)` or `values.into()` | The vector is moved in and freed while the result is built, so you never hold two full copies. |
| An iterator of `i64` | `iter.collect::<Shrewd>()` | The values are collected into a temporary `Vec<i64>` first, then packed. |
| A slice (`&[i64]`) | `Shrewd::pack(slice.to_vec())` | There is no direct conversion from a slice, so the copy is visible in your code. |

```rust
use shrewd_rs::Shrewd;

// From a vector: `pack` and `into` are the same.
let packed = Shrewd::pack(vec![10, 20, 30]);
let also_packed: Shrewd = vec![10, 20, 30].into();
assert_eq!(packed.get(2), also_packed.get(2));

// From an iterator.
let squares: Shrewd = (0..100).map(|x| x * x).collect();
assert_eq!(squares.get(9), Some(81));
```

* **Pack once, read many times.** `pack` does all the analysis up front, and the result cannot be changed. To change values, decode them, change them, and pack again.
* **Collecting from a borrowed vector costs a full extra copy.** `values.iter().copied().collect::<Shrewd>()` leaves `values` in place, so memory peaks at the original, a temporary copy, and the packed result. If you no longer need the vector, use `values.into_iter()`: its buffer is reused, so no extra copy is made.
* **Empty input is valid.** It gives a `Shrewd` where `is_empty()` is `true`.

---

## 📖 Reading Data

| You want to… | Use | Notes |
| :--- | :--- | :--- |
| Read one element | `get(i)` | `Some(value)` or `None` past the end. Never panics on any index. |
| Loop over everything | `for v in &packed` | Easiest. Decodes 64 values at a time behind the scenes. |
| Sum, fold or reduce over everything | `packed.iter().fold(...)`, `.sum()`, `.for_each(...)` | Faster than a `for` loop, because it decodes whole blocks without per-value overhead. |
| The same, as fast as possible on a desktop CPU | `packed.iter_buffered::<1024>().fold(...)` | Larger blocks. See Performance Tips. |
| Decode a range into your own buffer | `packed.decode_into(start, &mut buf)` | No allocation. Returns how many values were written. |
| Get a plain `Vec<i64>` back | `packed.iter().collect::<Vec<_>>()` | One exact-size allocation, but 8 bytes per value again. |

```rust
use shrewd_rs::Shrewd;

let packed = Shrewd::pack((0..1_000).map(|x| 5_000_000 + x).collect());

// Iteration: works with every iterator adapter; `len()` is exact.
let mut evens = 0;
for value in &packed {
    if value % 2 == 0 {
        evens += 1;
    }
}
assert_eq!(evens, 500);
let total: i64 = packed.iter().sum();
assert_eq!(packed.iter().len(), 1_000);

// Bulk decode of a range into a reusable buffer: elements 100..164.
let mut buf = [0i64; 64];
let n = packed.decode_into(100, &mut buf);
assert_eq!(n, 64);
assert_eq!(buf[0], 5_000_100);

// Decode near the end: the count tells you how much is valid.
let n = packed.decode_into(990, &mut buf);
assert_eq!(n, 10);

// Streaming over everything in fixed-size chunks, with no allocation:
let mut start = 0;
loop {
    let n = packed.decode_into(start, &mut buf);
    if n == 0 {
        break;
    }
    // process &buf[..n]
    start += n;
}
```

`decode_into` reads `out.len()` values starting at `start`, or fewer if the data ends first. A `start` at or past `len()` returns `0`.

---

## 💾 Saving and Loading

```rust
use shrewd_rs::{DecodeError, Shrewd};

fn round_trip() -> Result<(), DecodeError> {
    let packed = Shrewd::pack(vec![10, 20, 30]);
    let bytes: Vec<u8> = packed.to_bytes();

    let restored = Shrewd::from_bytes(&bytes)?;
    assert_eq!(restored.iter().collect::<Vec<_>>(), vec![10, 20, 30]);

    // Truncated or malformed input is reported, not panicked on.
    assert_eq!(Shrewd::from_bytes(&bytes[..3]).err(), Some(DecodeError::UnexpectedEOF));
    Ok(())
}
```

* **The format is portable and versioned.** All numbers are little-endian and all lengths are 8-byte `u64`s, so bytes move freely between platforms, including 32-bit and 64-bit ones. The first byte is a format version (currently 1). If a future release changes the layout, it will change the version, so bytes from a different release are rejected instead of misread. The full layout is in the `to_bytes` documentation.
* **Decoding never panics.** Anything `from_bytes` accepts can be read without a panic, and lengths are checked against the real input before anything is allocated, so a damaged length can't trigger a huge allocation. `from_bytes` can't detect damage that still forms valid data, such as a changed value, so add a checksum if the bytes travel over an unreliable channel.
* **For the smallest files,** run a general-purpose compressor such as zstd or LZ4 over the bytes (see Design Choices).

`from_bytes` returns a `DecodeError`:

| Variant | Meaning |
| :--- | :--- |
| `UnexpectedEOF` | The input ended before the encoded data did. |
| `UnsupportedVersion(u8)` | The bytes were written in a format version this release can't read. |
| `InvalidCompressorType(u8)` | The encoding byte is not a known encoding, or bytes from one encoding were read as another. |
| `InvalidDataSize(u8)` | A width byte is not 1, 2, 4 or 8. |
| `InvalidDictionaryIndex(u64)` | A dictionary index points outside the dictionary, so the input is damaged. |

`DecodeError` implements `std::error::Error`, so `?` works with `Box<dyn Error>` and similar error types. New variants may be added in later releases, so a `match` on it needs a `_` arm.

---

## 🛠 How It Works

`Shrewd::pack` makes one pass over the input to find the minimum, the maximum and an estimate of the number of distinct values (using HyperLogLog), then picks the encoding that uses the least memory:

| Encoding | How it works | Best for | Stored as |
| :--- | :--- | :--- | :--- |
| **`Size`** | Truncation | Numbers that are simply small (ages, months, small IDs). | The narrowest of `i8`, `i16`, `i32`, `i64` that fits every value. |
| **`Offset`** | Midpoint offset (frame of reference) | Values in a narrow band anywhere on the number line (sensor readings, timestamps, IDs near a large base). | Each value minus the midpoint of the minimum and maximum, in the narrowest signed width that fits the range. |
| **`Dictionary`** | Palette encoding | Few distinct values, however large (status codes, categories). | A table of the distinct values, plus a `u8`/`u16`/`u32`/`u64` index per element. |

`compressor_type()` tells you which one was chosen. It prints as its name, so it works directly in logs. The choice cannot be set by the caller.

**Guarantees:**

* **Every `i64` value round-trips exactly,** from `i64::MIN` to `i64::MAX`.
* **No hidden copies.** Peak memory while packing is the input, plus the output, plus a 4 KiB estimator. `memory_size()` reports the exact footprint, and the benchmark suite checks it against an allocator counter.
* **No vtable on the read path.** `Shrewd` wraps a private `enum` instead of a `Box<dyn Trait>`.
* **No `unsafe` code and no dependencies.** Third-party crates are used only for tests and benchmarks.

### 🧭 Design Choices

* **Whole-byte widths, on purpose.** Values are stored in 1, 2, 4 or 8 bytes, so every read is a single aligned load. Packing to the exact number of bits (as Parquet or FastLanes do) would save more space but make every read do more work.
* **Compression of the bytes is left to you.** `shrewd` does the lightweight, random-access part. For storage or transfer, run a general-purpose compressor such as zstd or LZ4 over `to_bytes()` if you want the smallest size. Byte-aligned data suits these compressors well, because unused high bytes and repeated patterns are what they remove.
* **Read-only.** A packed vector never changes, which keeps the encodings simple and every read free of locking or bookkeeping.

---

## ⚡ Performance

### Performance Tips

* **Choose the read method by access pattern.** Use `get` for a few elements, `iter().fold` / `sum` / `for_each` for full scans, and `decode_into` for streaming with a fixed buffer. A plain `for` loop is convenient but slower, because it pulls values one at a time.
* **Pick the iterator block size for your environment.**
  * `iter()` decodes 64 values at a time (512 bytes of stack), which is safe on small embedded stacks and cheap to start.
  * On desktop and server CPUs built for the target CPU (below), `iter_buffered::<1024>()` makes `fold`-style scans up to about 5× faster. It takes 8 KiB of stack and is slower to start, so keep `iter()` for short reads such as `take` or `find`.
  * A block size of 0 does not compile.
* **Build for your CPU when scan speed matters.** The decode loops auto-vectorize. With `-C target-cpu=native` (or a specific `-C target-cpu=...` for your deployment hardware), bulk decoding runs 3–4× faster than the baseline x86-64 build:
  ```sh
  RUSTFLAGS="-C target-cpu=native" cargo build --release
  ```
  Only use `native` for binaries that run on the machine they're built on.
* **Use a release profile like this one** in your root `Cargo.toml` for the best code generation:
  ```toml
  [profile.release]
  opt-level = 3
  lto = true
  codegen-units = 1
  panic = "abort"
  ```

### Results by Use Case

Measured on one machine (AMD Ryzen 9 9900X: 48 KiB L1, 1 MiB L2, 32 MiB L3, rustc 1.98). Treat them as a guide and run the benchmarks on your own hardware. Where a cell shows a range, it covers the three benchmark datasets (see Benchmark Details).

**Large read-only lookup tables.** Because the `shrewd` copy is smaller, it stays in a faster cache level for longer, so lookups are faster once the table outgrows the cache. Time per lookup, default release build:

| Values | `Vec<i64>` memory | `shrewd` memory | `Vec<i64>` single lookup | `shrewd` single lookup | `Vec<i64>` independent reads | `shrewd` independent reads |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| 4,096 | 32 KiB | 4–8 KiB | 2.5 ns | 2.7–3.2 ns | 0.7 ns | 1.3–1.6 ns |
| 262,144 | 2 MiB | 256–512 KiB | 10.3 ns | 4.2–4.7 ns | 2.6 ns | 1.4–1.6 ns |
| 4.2 M | 32 MiB | 4–8 MiB | 62 ns | 14–31 ns | 8.5 ns | 5.5–9.1 ns |
| 67 M | 512 MiB | 64–128 MiB | 96 ns | 85–90 ns | 10.5 ns | 13.8–17.4 ns |

A *single lookup* waits for each read before starting the next one, which is how most code reads a value and uses it. *Independent reads* let the CPU run many reads at the same time. Both include the cost of computing a random index.

**Caches and datasets limited by RAM.** Every value that stays in memory saves a trip to a database, a disk or the network, which costs far more than decoding. Number of values that fit in each amount of memory (exact allocator counts):

| | `Vec<i64>` | Small numbers | Narrow band | Few distinct values |
| :--- | ---: | ---: | ---: | ---: |
| Bytes per value | 8 | 2 | 1 | 1 |
| Number of elements in 48 KiB (L1) | 6,141 | 24,548 | 49,096 | 48,968 |
| Number of elements in 1 MiB (L2) | 131,069 | 524,260 | 1,048,520 | 1,048,392 |
| Number of elements in 1 GiB | 134 M | 537 M | 1.07 B | 1.07 B |

How many bytes a value needs depends on your data: it is the smallest whole-byte width that fits the range (or the number of distinct values). Data gets no saving only when it has many distinct values and a range wider than about 4.3 billion (2³²), such as hashes.

**Chunks and partitions in columnar storage.** Columnar file formats and databases load and spill data in chunks, usually per partition. `shrewd` stores each chunk relative to the middle of its own range, so a column that is wide across the table but narrow inside a partition packs well:

| Column inside one partition | Range | Bytes per value (`Vec<i64>`: 8) |
| :--- | ---: | ---: |
| Timestamps in milliseconds, 1-minute partition | 60,000 | 2 |
| Timestamps in seconds, 1-hour partition | 3,600 | 2 |
| Timestamps in milliseconds, 1-day partition | 86.4 M | 4 |
| IDs in a range partition of 200 IDs | 200 | 1 |
| The partition key column itself | 1 distinct value | 1 |

These widths follow directly from the encoding rules. The benchmark's "narrow band" data is this kind of column: 64-bit values around 10¹² within a range of 251. A chunk of 4,096 of these values takes 4.05 KiB in `shrewd` and 32.02 KiB in a `Vec<i64>`. The fixed cost per chunk is about 50 bytes in memory and about 20 bytes when serialized, plus 8 bytes per distinct value for data with few distinct values. The exception is hash partitioning: hashed IDs are spread over the whole 64-bit range in every partition and are almost all distinct, so they get no saving.

**Bulk reads of large data.** `shrewd` reads 1–2 bytes per value from memory instead of 8, which outweighs the decoding work once the data does not fit in the cache. This needs the bulk methods (`decode_into` or `iter_buffered::<1024>().fold`) and a build for the target CPU. Time per value, native build:

| Values | `Vec<i64>` copy out | `shrewd` copy out (`decode_into`) | `Vec<i64>` sum | `shrewd` sum (`iter_buffered::<1024>`) |
| :--- | ---: | ---: | ---: | ---: |
| 4,096 | 0.03 ns | 0.04 ns (0.23 ns\*) | 0.02 ns | 0.07–0.08 ns (0.26 ns\*) |
| 262,144 | 0.06 ns | 0.04 ns (0.22 ns\*) | 0.05 ns | 0.06–0.07 ns (0.25 ns\*) |
| 4.2 M | 0.13 ns | 0.04 ns (0.21 ns\*) | 0.08 ns | 0.06–0.07 ns (0.25 ns\*) |
| 67 M | 0.20 ns | 0.04–0.05 ns (0.22 ns\*) | 0.14 ns | 0.07–0.08 ns (0.25 ns\*) |

\* Data with few distinct values. Each value needs a lookup in the table of distinct values, so it reads more slowly.

**Scans of data that fits in the cache, where a `Vec<i64>` is faster.** Element-by-element loops cannot use SIMD on `shrewd` the way they can on a `Vec<i64>`. Time per value for 262,144 values, default release build:

| Scan | `Vec<i64>` | `shrewd` |
| :--- | ---: | ---: |
| `for` loop | 0.09 ns | 0.31–0.42 ns |
| `iter().fold` | 0.09 ns | 0.18–0.29 ns |
| `get(i)` for every index | 0.09 ns | 0.37–1.07 ns |
| `collect` into a new `Vec<i64>` | 0.11 ns | 0.50–0.77 ns |

### Benchmark Details

The benchmarks measure what it costs to keep data in `shrewd` instead of a `Vec<i64>`. Every read operation runs on both, in two comparisons:

* **Equal number of elements:** both hold the same values. This is the choice you actually make ("I have N values: which container?"). The results by use case above come from this comparison.
* **Equal memory:** both are filled to the same amount of memory (32 KiB, 512 KiB, 16 MiB and 128 MiB, sized to sit in L1, L2, L3 and main memory). Both then sit in the same cache level, so this shows the cost of decoding alone.

Each operation uses each container's natural API: `get(i)` for random reads and `get(i)` loops, `for` loops, `iter().fold` (plus `iter_buffered::<1024>().fold` for `shrewd`), copying values out in chunks of 1,024 (`decode_into` and `copy_from_slice`), and `collect`. `pack`, `to_bytes` and `from_bytes` are measured on their own, because a `Vec<i64>` has nothing equivalent.

```sh
cargo bench --bench runtime                           # all timings (several minutes)
cargo bench --bench runtime -- equal_elements         # one comparison
cargo bench --bench runtime -- equal_memory::get_latency
cargo bench --bench memory                            # exact memory use, compared with Vec<i64>
```

**Datasets.** Each encoding is measured on its own dataset, shaped so that `pack` picks it. The datasets use fixed random seeds, and every benchmark checks that the expected encoding was chosen. A `Vec<i64>` uses 8 bytes per value and runs at the same speed whatever the values are, so one dataset is enough for it.

| Dataset | Values | Encoding | Bytes per value |
| :--- | :--- | :--- | ---: |
| Small numbers | uniform in ±30,000 (the `i16` range) | `Size` | 2 |
| Narrow band | `i64` values within a range of 251 around 10¹² | `Offset` | 1 |
| Few distinct values | 16 distinct, widely spread `i64` values | `Dictionary` | 1 |

**Equal number of elements, remaining operations.** Time per value for 4.2 M values:

| Operation | `Vec<i64>`, default build | `shrewd`, default build | `Vec<i64>`, native build | `shrewd`, native build |
| :--- | ---: | ---: | ---: | ---: |
| `for` loop | 0.10 ns | 0.31–0.42 ns | 0.07 ns | 0.38–0.43 ns |
| `iter().fold` (blocks of 64) | 0.10 ns | 0.19–0.29 ns | 0.07 ns | 0.23–0.29 ns |
| `get(i)` for every index | 0.10 ns | 0.37–1.08 ns | 0.08 ns | 0.38–1.10 ns |
| Copy out in chunks | 0.14 ns | 0.13–0.23 ns | 0.13 ns | 0.04–0.21 ns |
| `collect` into a new `Vec<i64>` | 1.11 ns | 1.28–1.40 ns | 1.09 ns | 1.30–1.42 ns |

**Equal memory.** Both containers in the same cache level, so this is the decoding cost alone. Time per value, default build:

| Operation | Level | `Vec<i64>` | `shrewd` |
| :--- | :--- | ---: | ---: |
| Single lookup | L1 | 2.5 ns | 2.7–3.2 ns |
| | L2 | 4.5 ns | 4.7–5.3 ns |
| | L3 | 23 ns | 36–74 ns¹ |
| | Main memory | 89 ns | 89–90 ns |
| Independent reads | L1 | 0.7 ns | 1.4–1.6 ns |
| | L2 | 0.7 ns | 1.5–1.8 ns |
| | Main memory | 9.1 ns | 13.3–17.6 ns |
| Copy out in chunks (native build) | L1 | 0.03 ns | 0.04–0.22 ns |
| | L3 | 0.07 ns | 0.04–0.21 ns |
| | Main memory | 0.17 ns | 0.04–0.22 ns |

¹ Noisy and not yet explained: `shrewd` was consistently slower here, but the size of the gap varied widely between runs.

**Encoding and serialization,** default build:

| Operation | Result |
| :--- | :--- |
| `pack`, small numbers and narrow band | 660–740 M values/s |
| `pack`, few distinct values | about 100 M values/s (hash-table bound; see Limitations) |
| `to_bytes` and `from_bytes` | 70–160 GB/s while the data is in cache; about 7 GB/s for very large data, where getting fresh memory pages from the OS dominates |
| Peak memory during `pack` | the input, plus the output, plus 4 KiB |

**Notes:**

* **Noise:** results for data around the L3 size can vary by 20–40% between runs. Compare runs on the same machine, and save the output before and after a change (`cargo bench > before.txt`).
* **Running as tests:** `cargo test --benches` checks that every benchmark runs, but it is slow, because it still builds the largest datasets (up to 128 M values) without optimizations.

---

## ⚠️ Limitations

* **Packing data with few distinct values is the slowest path** (about 100 M values/s), because it uses the standard library's DoS-resistant hasher.
* **`i64` only.**
* **Requires `std`** (`no_std` support is planned).

## 📜 License

Licensed under the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).
