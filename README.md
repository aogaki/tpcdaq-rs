# tpcdaq-rs

A new DAQ and online-monitoring system for GET-based TPC readout — the mini eTPC at ELI-NP
and the ELITPC at the Faculty of Physics, Warsaw University. Rust core + C++ satellites
(ROOT, ZeroC Ice), connected as ZeroMQ components in the style of
[delila-rs](https://github.com/ELI-NP/delila-rs).

**Status: receive → decode → dual storage → online monitoring → web UI complete (P0–P3),
run-control groundwork in place.**
The implementation master is the specification [docs/SPEC_ja.md](docs/SPEC_ja.md) (v1.12, Japanese);
the founding proposal is [docs/PROPOSAL_en.md](docs/PROPOSAL_en.md)
(master copy in Japanese: [docs/PROPOSAL_ja.md](docs/PROPOSAL_ja.md), v0.4).

Implemented so far: TOML config, TPCReco `.dat` geometry (FPN reorder, channel roles),
ZeroMQ message core with a schema-drift guard, MFM framer + frameType 1/2 decoder,
a pacing-capable multi-file `graw_replay` tool, the per-CoBo receiver (never-stop drain,
listen-before-start, end-of-stream semantics), the per-AsAd graw writer (byte- and
naming-identical to the real DataRouter, rotation included), the eventIdx event builder +
ROOT sink writing **TPCReco-compatible PEventTPC files**, a JSONL logbook, the
ecc-bridge (Ice client to the unmodified GET control plane, with a fake ECC for tests),
and the online monitoring path — root-sink histogram aggregation published over PUB/SUB,
the `monitor` component fanning it out over WebSocket, and the Angular web UI (JSROOT for
the nine UVW histograms and event display, ECharts for raw waveforms, plus the logbook;
run control wiring to the REST API lands in P4).
Everything is validated against real-machine data: the graw output is a byte-exact
lossless split of a real run, and on a real ELITPC run the ROOT output matched the
production offline conversion **event for event, bit for bit** (3852 events, 0 differences).
Next: a 24-hour load/soak harness and real-site deployment at Warsaw
(see [docs/WARSAW_PLAN_ja.md](docs/WARSAW_PLAN_ja.md)).

## What it replaces

The stock GET software chain (dataRouter + offline `grawToEventTPC` conversion +
file-tailing monitors). tpcdaq-rs writes graw (byte-identical raw) and
**TPCReco-compatible ROOT (PEventTPC — the same format the offline converter produces,
so the conversion step disappears)** online, simultaneously, streams UVW monitoring
histograms and raw waveforms live to any browser, and consolidates run control, logbook
and power-supply operation into one web application. Minimum target trigger rate: 100 Hz;
the control plane (ECC / getHwServer / firmware) is not modified.

## Web UI

The operator UI lives in [`ui/`](ui/) (Angular + Angular Material; see
[ui/README.md](ui/README.md) for how to run and build it). It has five views — **Monitor**
(the nine UVW histograms and the event display, drawn with JSROOT), **Waveform** (raw ADC
traces, ECharts), **Logbook** (the JSONL timeline plus comment posting), **Run control**
and **Power**. It talks to two processes: the monitor's WebSocket (`:9000`) for live
histograms and status, and the controller's REST API (`:8080`) for the logbook and system
status; `ng build` output is served by the controller itself, so deployment stays
"Rust only". Everything it shows comes from the real pipeline — there are no mock data
sources — and it makes no outbound requests (fonts are bundled), so it works on an
offline DAQ machine. **Run control and Power are laid out but deliberately inert**: the
buttons are disabled until the REST wiring lands, so nothing on screen can be mistaken
for a control that works.

## Layout

- `docs/` — proposal (ja/en), specification ([SPEC_ja.md](docs/SPEC_ja.md)), deployment plan
- `TODO/` — numbered task units + `CURRENT.md` (session entry point)
- `src/` — Rust components (receiver / decoder / graw-writer / monitor / controller / psu)
- `ui/` — Angular operator UI (served by the controller from `ui_dir`)
- `tools/` — C++ satellites (root-sink, ecc-bridge)
- `third_party/` — isolated vendored code with its own licenses (CeCILL etc.), when needed
- `tests/fixtures/` — synthetic fixtures only (no real run data in this repository)

## License

BSD 3-Clause. See [LICENSE](LICENSE).
