# Fuzzers

Four targets, one per parser that takes bytes from a stranger: the box's base64
(`codec`), the CSV reader (`csv`), the three importers plus the vault payload
(`import`), and the share format (`share`). None of them asserts a result — a parser
may refuse anything — only that nothing panics, overflows or allocates without bound.

```sh
cd fuzz
cargo fuzz run codec -- -max_total_time=60
cargo fuzz run csv   -- -max_total_time=60
cargo fuzz run import -- -max_total_time=60
cargo fuzz run share -- -max_total_time=60
```

Nightly and a C++ toolchain, because that is what `libfuzzer-sys` needs. The crate
builds the whole library under AddressSanitizer, so a run wants `CARGO_BUILD_JOBS=2`:
rustc compiling `ttf-parser` under the sanitizer flags segfaults when several of them
run at once.

No workflow runs these on a timer. Run them by hand and keep `fuzz/corpus`, which is
what carries a finding from one run to the next.
