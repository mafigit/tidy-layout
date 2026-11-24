# tidy-layout

A layout engine for diagrams of cards and connections, in Rust, compiled to WebAssembly.

It is an **unofficial Rust port of TALA**, the layout engine of [D2](https://github.com/d2lang/d2),
with a small API of its own on top. It is not affiliated with or endorsed by Terrastruct.

## What it does

- `arrange`: places for all cards, as TALA arranges a diagram (the best of its three seeds).
- `route`: connectors around the cards, at right angles, with places for their labels.
- `snap`: the free place nearest to where a card is dropped.

For the same diagram and seed the port gives the drawing TALA gives: the tests replay answers
recorded from TALA's own code (`tests/fixtures/`, see [docs/reference.md](docs/reference.md)).
Not ported: containers, labels of cards, connections from a card to itself.

## Use

- Rust: the crate `tidy-layout` (`src/api.rs` holds the three calls).
- JavaScript: `js/index.ts` loads the `.wasm` file and offers the same three calls.

## Build and test

Needs Rust with the `wasm32-unknown-unknown` target (see `rust-toolchain.toml`).

- `bin/build` compiles the engine into `pkg/tidy_layout.wasm`.
- `bin/build test` runs the Rust tests first.
- `pnpm install && pnpm test` also tests the JavaScript wrapper (needs Node and pnpm).

## Licence

[MPL-2.0](LICENSE), as TALA's source, of which this is a translation. `src/gorand.rs` and
`src/gosort.rs` are ports of parts of Go's standard library, under Go's BSD-style licence
([LICENSE-GO](LICENSE-GO)). [NOTICE.md](NOTICE.md) says where each part comes from.

## AI disclosure

This port was written with AI assistance: the Rust code, the tests, the tools and the documents
were written by Claude Code (Anthropic's Claude models), directed by the maintainer.

- How it was checked: not by reading every line, but by comparing it with TALA. Every ported
  stage replays answers recorded from TALA's Go code and must match them, on the recorded
  fixtures and on some 137,000 generated diagrams; deliberately broken versions of the code
  must fail those tests (`tools/breaks/`).
- What that does not show: behaviour on diagrams unlike the tested ones. Use it at your own
  risk; the licence gives no warranty.
- The code is a translation of MPL-2.0 source, published under the same licence.
