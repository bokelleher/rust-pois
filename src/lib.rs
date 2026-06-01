// src/lib.rs
//
// Library facade for the POIS ESAM server. Exposes the framework-agnostic
// SESAME core (ANSI/SCTE 130-9) so that it can be exercised by Criterion
// benchmarks (`benches/sesame_overhead.rs`) and integration tests independently
// of the Axum HTTP stack, and reused by any ESAM client or server.
//
// The binary (`src/main.rs`) consumes this crate as `pois_esam_server::sesame`
// and wires the Axum adapter (`src/sesame_axum.rs`) into the ESAM exchange.

pub mod sesame;
