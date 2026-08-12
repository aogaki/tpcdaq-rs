# tpcdaq-rs

A new DAQ and online-monitoring system for GET-based TPC readout — the mini eTPC at ELI-NP
and the ELITPC at the Faculty of Physics, Warsaw University. Rust core + C++ satellites
(ROOT, ZeroC Ice), connected as ZeroMQ components in the style of
[delila-rs](https://github.com/ELI-NP/delila-rs).

**Status: specification frozen, receive/decode chain implemented (P0–P1).**
The implementation master is the specification [docs/SPEC_ja.md](docs/SPEC_ja.md) (v1.0, Japanese);
the founding proposal is [docs/PROPOSAL_en.md](docs/PROPOSAL_en.md)
(master copy in Japanese: [docs/PROPOSAL_ja.md](docs/PROPOSAL_ja.md), v0.4).

Implemented so far: TOML config, TPCReco `.dat` geometry (FPN reorder, channel roles),
ZeroMQ message core with a schema-drift guard, MFM framer + frameType 1/2 decoder
(validated against the real-data oracle: events=108 / items=15,040,512 / malformed=0),
a pacing-capable `graw_replay` tool, and the per-CoBo receiver (never-stop drain,
listen-before-start, end-of-stream semantics). 153 tests; a full-speed replay of a real
29 MB `.graw` arrives byte-identical on both downstream links with zero drops.
Next: storage (per-CoBo graw writer, ROOT sink with event builder).

## What it replaces

The stock GET software chain (dataRouter + offline graw2root + file-tailing monitors).
tpcdaq-rs writes graw (byte-identical raw) and graw2root-compatible ROOT **online,
simultaneously**, streams UVW monitoring histograms and raw waveforms live to any browser,
and consolidates run control, logbook and power-supply operation into one web application.
Minimum target trigger rate: 100 Hz; the control plane (ECC / getHwServer / firmware) is
not modified.

## Layout

- `docs/` — proposal (ja/en), specification (to come)
- `TODO/` — numbered task units + `CURRENT.md` (session entry point)
- `src/` — Rust components (receiver / decoder / graw-writer / monitor / controller / psu)
- `tools/` — C++ satellites (root-sink, ecc-bridge)
- `third_party/` — isolated vendored code with its own licenses (CeCILL etc.), when needed
- `tests/fixtures/` — synthetic fixtures only (no real run data in this repository)

## License

BSD 3-Clause. See [LICENSE](LICENSE).
