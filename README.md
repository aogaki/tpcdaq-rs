# tpcdaq-rs

A new DAQ and online-monitoring system for GET-based TPC readout — the mini eTPC at ELI-NP
and the ELITPC at the Faculty of Physics, Warsaw University. Rust core + C++ satellites
(ROOT, ZeroC Ice), connected as ZeroMQ components in the style of
[delila-rs](https://github.com/ELI-NP/delila-rs).

**Status: receive → decode → dual storage complete (P0–P2), run-control groundwork in place.**
The implementation master is the specification [docs/SPEC_ja.md](docs/SPEC_ja.md) (v1.8, Japanese);
the founding proposal is [docs/PROPOSAL_en.md](docs/PROPOSAL_en.md)
(master copy in Japanese: [docs/PROPOSAL_ja.md](docs/PROPOSAL_ja.md), v0.4).

Implemented so far: TOML config, TPCReco `.dat` geometry (FPN reorder, channel roles),
ZeroMQ message core with a schema-drift guard, MFM framer + frameType 1/2 decoder,
a pacing-capable multi-file `graw_replay` tool, the per-CoBo receiver (never-stop drain,
listen-before-start, end-of-stream semantics), the per-AsAd graw writer (byte- and
naming-identical to the real DataRouter, rotation included), the eventIdx event builder +
ROOT sink writing **TPCReco-compatible PEventTPC files**, a JSONL logbook, and the
ecc-bridge (Ice client to the unmodified GET control plane, with a fake ECC for tests).
Everything is validated against real-machine data: the graw output is a byte-exact
lossless split of a real run, and on a real ELITPC run the ROOT output matched the
production offline conversion **event for event, bit for bit** (3852 events, 0 differences).
Next: online monitoring (histogram aggregation, WebSocket streaming, web UI).

## What it replaces

The stock GET software chain (dataRouter + offline `grawToEventTPC` conversion +
file-tailing monitors). tpcdaq-rs writes graw (byte-identical raw) and
**TPCReco-compatible ROOT (PEventTPC — the same format the offline converter produces,
so the conversion step disappears)** online, simultaneously, streams UVW monitoring
histograms and raw waveforms live to any browser, and consolidates run control, logbook
and power-supply operation into one web application. Minimum target trigger rate: 100 Hz;
the control plane (ECC / getHwServer / firmware) is not modified.

## Layout

- `docs/` — proposal (ja/en), specification ([SPEC_ja.md](docs/SPEC_ja.md)), deployment plan
- `TODO/` — numbered task units + `CURRENT.md` (session entry point)
- `src/` — Rust components (receiver / decoder / graw-writer / monitor / controller / psu)
- `tools/` — C++ satellites (root-sink, ecc-bridge)
- `third_party/` — isolated vendored code with its own licenses (CeCILL etc.), when needed
- `tests/fixtures/` — synthetic fixtures only (no real run data in this repository)

## License

BSD 3-Clause. See [LICENSE](LICENSE).
