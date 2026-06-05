// src/lib.rs
//
// Library facade for the POIS ESAM server. Exposes the framework-agnostic
// SESAME core (ANSI/SCTE 130-9) so that it can be exercised by Criterion
// benchmarks (`benches/sesame_overhead.rs`) and integration tests independently
// of the Axum HTTP stack, and reused by any ESAM client or server.
//
// The binary (`src/main.rs`) consumes this crate as `pois_esam_server::sesame`
// and wires the Axum adapter (`src/sesame_axum.rs`) into the ESAM exchange.
//
// SESAME now lives in its own crate (`sesame`, https://github.com/bokelleher/sesame-sdk,
// extracted byte-for-byte from this repo's former `src/sesame/`). We re-export it
// here so every existing `pois_esam_server::sesame::…` path resolves to the crate
// unchanged — the rest of rust-pois (axum adapter, benches) compiles as-is.

pub use ::sesame as sesame;
