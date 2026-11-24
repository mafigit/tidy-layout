# Where this code comes from

This crate is a Rust port of parts of **TALA**, the layout engine of [D2](https://github.com/d2lang/d2)
(`d2layouts/d2talalayout`), Copyright Terrastruct Inc. and the D2 authors, licensed under the
Mozilla Public License 2.0.

It is a translation of that source, so it stays under the **MPL-2.0** (see `LICENSE`). Whoever
ships the built engine must make this source available.

`src/gorand.rs` is a port of Go's `math/rand` (the seeded generator and its table) and
`src/gosort.rs` of Go's `sort.Slice`, Copyright The Go Authors, under Go's BSD-style licence: see
`LICENSE-GO`. TALA draws its random numbers from the one and sorts with the other.

- Reference: `d2lang/d2` at commit `25e67b4` (2026-11-21).
- Each Rust file names the Go file and functions it was ported from.
- `tests/fixtures/*.tala.json` hold answers recorded from TALA's own code; the tests check that the
  port gives the same ones. How they were recorded: [docs/reference.md](docs/reference.md).
