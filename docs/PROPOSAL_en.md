# Proposal: New TPC DAQ — Rust Core + C++ Satellites (pre-specification)

- **status**: draft v0.4-en — English edition for circulation; translation of `PROPOSAL_ja.md` v0.4, which remains the working master
- **date**: 2026-08-12 (v0.2: 2026-08-10, incorporating impressions from the HIγS campaign / v0.3: review pass — oxyroot re-verified and reversed, root_sink reuse with its risks made explicit, P6 added, Q3 added, single-writer JSONL, quantitative P5 acceptance, ELITPC = 2 CoBos confirmed, minimum target raised to 100 Hz, R13 waveform view added / v0.4: pre-circulation review — collaboration note in D5, TPCReco statement turned into a confirmation request, signal-channel/FPN definition, "memo" legend, timeline note)

---

## 1. Background

The current GET-based DAQ has the following operational pain points (details in `ASSESSMENT_移植検討.md`, internal document in Japanese):

1. **Time lost to offline conversion** — graw → ROOT (graw2root) runs after the experiment, adding a waiting step.
2. **The online monitor reads after the fact** — TPCReco works by reading graw files already written to a directory, so it is not real-time. In addition, as far as we could confirm (2026-08), **our understanding is that TPCReco's online mode does not include track reconstruction — please correct us if this is wrong**. What is actually needed online is lightweight histogram displays, not a reconstruction-grade monitor.
3. A structural weakness: reception through recording runs single-threaded and synchronous.
   > Note: the "stops at ~1000 triggers" issue once suspected as a stability problem was later **traced to an incorrectly connected VETO input** (2026-08) — **it was not a software fault**. The monotonic consumption of the DataBloc 64 MiB budget found during that investigation (`ASSESSMENT_移植検討.md` §1-2) remains a latent code issue but is dropped from the motivation of this proposal.
4. **Operations are scattered across applications** — run control (GetController), power-supply operation and monitoring are separate programs, and every shift starts several of them. **Consolidating these launches into one is a key point of this project** (both the run-control integration and the R12 power-supply integration follow from it).

In 2026-06 a new C++ DAQ (`tpcdaq/`) was implemented up to M4 plus hardening (24/24 tests green; regression against a real 2025 .graw re-verified 2026-08-06). All tests are software tests — **no connection to real hardware (real CoBo / real ECC) has been made yet**. However, the three protocol-level blockers for real hardware — missing support for frameType 2 (compact) used by the real 2025 run (events=0 → 108 after the fix), the DataSender id format (`CoBo[0]`, uppercase flowType `TCP`), and pinning Ice encoding to 1.1 — have been fixed against real data and the real GET sources (recorded in the hardening section of `tpcdaq/README.md`).
This proposal updates the requirements based on impressions gathered while participating in the HIGS campaign and redefines the direction.

## 2. Requirements (2026-08; based on impressions from the HIGS campaign and earlier observation)

> These are **impressions** — from conversations on site and long-standing observation, not official Warsaw feedback — organized into requirement form. They will be updated with Warsaw's official input once the tpcdaq-rs repository is shared.
> In the Source column, "memo" = my own notes from the HIγS campaign.

| # | Requirement | Source |
|---|-------------|--------|
| R1 | A **web-based UI** | memo |
| R2 | **ELITPC in scope too** (1018 strips, 1024 signal channels [1088 including FPN] = 4 AsAds = 2 CoBos — one CoBo [FPGA board] reads out 2 AsAds). Do not specialize to the mini eTPC (256 signal channels [272 including FPN] = 1 AsAd, 1 CoBo) | memo |
| R3 | Online monitor: **"strip vs. time" 2D histogram for each of the U/V/W planes** | memo |
| R4 | Online monitor: **two charge (pulse-height) histograms per U/V/W plane**: ① **pulse-height spectrum** — per event, fill the pulse height (maximum over time samples) of **every strip in the plane**; ② **event maximum pulse height** — per event, fill **only the single maximum value in the plane**. Both are per plane, not per strip (2 kinds × 3 planes = 6 histograms) | memo + discussion |
| R5 | **No online 3D/track display** (to our understanding a feature TPCReco's online mode never had — see the confirmation request in §1-2; TPCReco stays as-is as the offline tool) | memo |
| R6 | **Run control feels essentially the same as GET controller** (describe→prepare→configure→start→stop) | memo |
| R7 | **On start/stop, record run metadata to a file (JSONL; fixed in R11)** and notify the monitor as needed | memo |
| R8 | Recording is **two online streams: graw (mandatory, raw data) + graw2root-compatible ROOT** (eliminating offline conversion) | memo |
| R9 | Event display shows **the latest event at a configurable time interval** (interval 0 s = true real time), plus a **freeze** function to hold the display on an interesting event. **The event ID (run number / event number) is always shown**, so a frozen event can be pinpointed later in offline analysis (built event data targets a single file per run — §5 — so these two values identify it uniquely) | discussion 2026-08-10 |
| R10 | On run stop, the monitor histograms are **promptly written to a separate ROOT file** — "separate" = a ROOT file containing only histograms, apart from the event-data file (`run<N>_monitor.root`) | discussion 2026-08-10 |
| R11 | Run metadata is **recorded as JSONL (fixed)**, and the UI has a **logbook view (reader)** for chronological reading: run start/stop records + operation audit log + **free-text shift comments**, all browsable and appendable on a single timeline | discussion 2026-08-10 |
| R12 | **(Bonus) HV/LV power-supply monitoring + control**: integrate the CAEN HiVolta (DT1415ET) and R&S HMP2020 into the web UI (V/I monitoring plus ON/OFF, VSET, etc.). **ELI-NP currently has no control application**, and this fits the app-consolidation key point (§1-4). Operations are protected by the same control-token + passphrase + audit-log scheme as run control. TRIPs etc. are auto-recorded into the R11 logbook | discussion 2026-08-10 |
| R13 | Online monitor **waveform view** — display **the raw waveforms of all strips of the current event at once** (overlay or grid, by plane/AsAd/AGET; FPN channels included, no subtraction). Useful for **baseline checks when taking pedestals and for spotting noisy/dead channels at a glance** | discussion 2026-08-12 |

> R4 definition (fixed 2026-08-10): x-axis = charge (ADC pulse height). Each strip's signal arrives as a time distribution of integrated charge; its **maximum over time samples** is the "pulse height". ① fills N_strip entries per event (noise peaks get recorded too — cut the low side at display/analysis time); ② fills one entry per event. **Reset at run start**, accumulate over the whole run; write-out at stop is R10.
>
> **The primary purpose is saturation monitoring** — first of all, whether maxima pile up at the ADC ceiling (12 bit = 4095). For that purpose ① and ② are nearly equivalent (both are cheap, so implement both). The x-range is therefore **fixed to the full ADC scale (0–4096)** so the ceiling bin is always visible (no auto-ranging that could hide it). As an aid, a numeric **saturation fraction** per plane (fraction of strips at full scale) is cheap and direct — to be considered at specification time.

## 3. Scope / Non-scope

**Scope**: the entire data plane (reception → decoding → two recording streams → monitoring) + run control + web UI.
**Full DAQ replacement from the start** (the CoBo data stream is pointed at the new application instead of dataRouter; intermediate forms such as file-tailing monitors are not the primary goal).

**Non-scope**:
- Online 3D/XY reconstruction display (R5)
- Changes to TPCReco or the offline analysis chain (goal: ROOT-output compatibility so they connect unmodified)
- Changes inside the control plane (ECC / getHwServer / firmware) — we only talk to it as a client
- Mac/Windows support (operation is on Linux; development on Mac is fine)

## 4. Policy decisions (2026-08-10)

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | **Independent new Rust project**, tentatively named **`tpcdaq-rs`** | As a DAQ it differs too much in character from delila-rs; the name shows the lineage from the C++ `tpcdaq` and follows the same naming convention as delila-rs |
| D2 | **Rust core; C++ satellites only where the ecosystem demands it** | ROOT (writing both the event TTrees and the monitor histograms requires C++ — oxyroot re-verified 2026-08-12, §5) and ZeroC Ice (ECC control) effectively require C++. Everything else in Rust |
| D3 | **Aim at full DAQ replacement from the start** | The core value (eliminating offline conversion; real-time monitoring) only materializes with replacement |
| D4 | **Adopt the delila-rs component system; inter-component transport is ZeroMQ** | Reuses a proven architecture and implementation know-how; C++ satellites join naturally as "components that speak ZMQ" |
| D5 | **Develop solo first; verify thoroughly on real hardware with the mini TPC at ELI-NP; hand to Warsaw for testing once it is usable** | Fast progress without coordination overhead; hardware verification (mini TPC) can be done in-house at ELI-NP; Warsaw receives something already verified. The ELITPC is at Warsaw. **Development work proceeds solo, but input on the requirements and the specification is welcome from this stage** — circulating this document is the first step |

### Implication of D2 — "Ice also stays on the C++ side"

The 2026-06 pause happened because "doing Ice in Rust is a pain". This policy avoids that structurally: **the Rust core knows nothing about Ice**. Ice communication with the ECC is carved out of the existing `tpcdaq::control::EccController` (a state-machine wrapper with Ice hidden behind a pimpl) into a small C++ bridge, driven from Rust over a plain protocol (JSON over stdio/socket). ROOT writing reuses the delila-rs C++ component **root_sink** (proven ZMQ subscription + eb_core event builder + TTree/TH1/TH2 writing) as the skeleton, with the TTree content ported from the existing `RootWriter` (GDataFrame; graw2root compatibility already verified).

## 5. Architecture — delila-rs-style component system (ZeroMQ)

We adopt the component system proven in delila-rs (D4): independent single-purpose components connected by ZeroMQ, with the C++ satellites participating as "components that speak the same ZMQ protocol".

```
CoBo ──TCP:46005──▶ [receiver] ──ZMQ──▶ [graw-writer]                 ← lossless (byte-identical append)
                        │
                        └──ZMQ──▶ [decoder] ──ZMQ──┬──▶ [root-sink (C++)] ← lossless (event builder + graw2root-compatible TTree + monitor.root)
                                                    └──▶ [monitor] ──▶ WS ──▶ web UI (browser)
                                                          ▲                 · per-plane strip×time 2D ×3
                                                          │ run events      · per-plane charge hists 2 kinds ×3
[controller] ──ZMQ──▶ [ecc-bridge (C++)] ──Ice──▶ ECC:46002 ──▶ CoBo (getHwServer)
     └── run metadata records (JSONL)
```

- **Never-block principle**, same as the C++ version: the receiver only drains. The recording chain (graw-writer / root-sink) is lossless (protected by PUSH/PULL backpressure); the monitoring chain is latest-first (PUB/SUB + decimation). "Separation of recording and visualization" (the lesson of C++ hardening item C3) is designed in from the start as a component boundary.
- **listen-before-start** (listen on 46005 before `ecc start`) and the other operational invariants established in M4 carry over.
- ELITPC/mini are switched via geometry configuration (TPCReco's `.dat` format). **Channel counts are never hard-coded** (the C++ version bakes in mini=256ch assumptions; this part is redesigned from scratch).
- **Multiple CoBos are designed in from the start** (the ELITPC is a 4-AsAd = **2-CoBo** system — R2 — so this is a production requirement, not future-proofing. Only a single CoBo [mini = 1 AsAd] can be verified in-house, but the configuration scales to N): one receiver component per CoBo. **Raw graw goes to one file per CoBo** — per-stream order is guaranteed by TCP, so in-file order is structurally always correct. Only the outputs that need cross-CoBo integration (ROOT / monitor) pass through an **event builder** (per-source FIFO → merge by eventIdx; missing fragments marked incomplete on timeout; pass-through in single-CoBo setups). The implementation reuses **delila-rs's C++ root_sink (eb_core)** (§9). **Built event data targets a single ROOT file per run** (the builder guarantees run-number/event-number consistency); only raw graw is split per CoBo.
  > Reference: the problem Warsaw currently faces with stock GET — "per-CoBo/AsAd files are correctly ordered, but merging everything into one file scrambles the order" — has, as its **leading hypothesis**, the concatenation of independent TCP streams **in arrival order** (inter-stream arrival order is nondeterministic; without re-ordering by event number, i.e. an event builder, mixing is the expected behavior). This is **an unverified hypothesis**, so we do not state it as fact — it will be checked together with the failure-unit question (Q3). Either way, the design above avoids this class of ordering problem structurally.
  > Since the ELITPC = 2 CoBos (R2), the event builder is **mandatory** for the ELITPC (pass-through suffices only for the mini [1 CoBo]). As the only hardware in-house is the mini, **multi-source merging is pinned down before Warsaw via offline tests running two graw_replay instances in parallel** (§7). Confirming the failure unit of the current problem (per-CoBo files or per-AsAd) is Q3 (§10) — presumed per-CoBo, since 1 CoBo = 1 independent TCP stream.
- A by-product of componentization: a future online filter is just one more component inserted between the decoder and the writers/monitor. Distribution across hosts also comes naturally with ZMQ.
- The message format on each link (raw frames / decoded-event serialization) will be defined at specification time, aligned with the delila-rs formats. The question of how to attach the C++ satellites (process separation vs. FFI) is automatically resolved by ZMQ componentization (everything is an independent process).
- **Monitor write-out at run stop** (R10): on stop, the histograms are promptly written to `run<N>_monitor.root` (histograms only; separate from event data), inheriting the C++ M3 monitor.root scheme. The write is done **on the C++ side, reusing root_sink's TH1D/TH2D writing machinery**. The original decision "directly from Rust via oxyroot (fixed)" was **reversed after re-verification (2026-08-12)**: oxyroot (0.1.25) has no histogram type in its public API (writing = `WriterTree`, i.e. TTrees of primitive-type branches, uncompressed only); a minimal test succeeded at TTree writing and failed to compile `TH1D`. In delila-rs, too, what Rust/oxyroot writes is TTrees only — histograms are written by the C++ root_sink.
  **Who owns histogram accumulation is the single most important design decision to settle first in the specification**: accumulating in both the Rust monitor and root_sink risks "the file does not match what the UI showed" (double management of binning and reset timing); centralizing in root_sink co-locates visualization load with the recording process, in tension with the C3 lesson of separating recording and visualization. Candidate: root_sink accumulates and publishes histogram snapshots, and monitor/UI only display them (the mismatch disappears structurally, conditional on verifying load isolation inside the recording process, e.g. a dedicated thread).
- **Event-display pacing is client-side** (R9): the server always distributes the latest event (PUB/SUB, latest-first); each browser independently applies its display interval (0 s = true real time) and freeze. → Achievable with no core changes, and one viewer's freeze never stops another's live view. **Freeze stops only the display** — DAQ, accumulated histograms and recording never stop (the UI clearly distinguishes it from run Stop). Distributed events always carry the **event ID (run number / event number)**, shown on screen — the event number is just the MFM-header eventIdx carried through by the decoder, so the cost is near zero, and the workflow "interesting event → note the ID → offline scrutiny" connects.
- **Waveform view** (R13): a UI view over **the same event payload** as the R9 event display (addable with no core changes). Shows the raw waveforms of all strips of the current event (FPN channels included, no subtraction — for pedestal work it is essential that FPN and baselines are visible as-is), overlaid or gridded by plane/AsAd/AGET. Interval, freeze and event ID (R9) apply unchanged. A full ELITPC overlay is a lot of points (~1088 ch × 512 buckets), so view narrowing and client-side decimation are defined in the UI specification.
- **Two-tier access control**: viewing (monitoring) is open without authentication (anyone being able to watch during a shift is a feature). Operations (configure/start/stop) require ① the control token (exactly one client holds control at a time; inherited from the C++ M4 design), ② a shared passphrase (one per configuration, requested when taking control), and ③ an **audit log** carrying the operator's name (all operations recorded into run metadata, R7). The threat model is **accidents and conflicting operations**, not attackers; no user accounts or role management. Remote access goes through SSH tunnels (no home-grown TLS/authentication). Should the collaboration require real authentication, a reverse proxy (basic auth etc.) in front covers it without code changes.
- **(Bonus) psu component (monitoring + control)** (R12): a device list (model, address) in the configuration file; 1 Hz polling → ZMQ publish → a "Power" panel in the UI (per-channel V/I trends + status badges + red TRIP/ALARM banner + ON/OFF and VSET controls). TRIP/INTLK transitions are auto-recorded into the R11 logbook (same timeline as runs).
  - **HiVolta**: ASCII protocol on TCP:1470 (`$CMD:MON,CH:8,PAR:VMON` etc. read all 8 channels at once; manual DT1415_rev16.pdf pp. 23–26). **Monitoring is allowed in LOCAL mode** (only SET returns `#LOC:ERR`) → read-only monitoring does not interfere with on-site operation. The TCP port serves one client at a time → a single poller owns it and fans out to browsers (consistent with this architecture). The official tool is still "coming soon" and EPICS support was discontinued, so building this has real value.
  - **HMP2020**: standard SCPI (LAN = raw socket 5025 [HO730/HO732 option] or USB); the usual `MEAS:VOLT?` / `MEAS:CURR?`. The front-panel lock in remote mode is expected to be avoided with `SYST:MIX` (mixed mode).
  - **Control included (decided 2026-08-10)**: since ELI-NP has no power-control application today, implement ON/OFF, VSET, ramps etc. in addition to monitoring (a consequence of the §1-4 consolidation policy). HV operations ride on the §5 access-control scheme (token + passphrase + audit log) with destructive-action confirmation patterns in the UI. SWVMAX (software voltage limit) in the configuration mechanically caps mis-settings. HiVolta SET requires REMOTE mode (switched via `BDCTR`; returning to LOCAL restores front-panel operation; monitoring works in either mode).
  - **Warsaw already has a desktop application with control** (learned 2026-08-10) → **only when deployed at Warsaw** is there contention for HiVolta's single TCP connection (no contention at ELI-NP; early deployment possible there). Coexistence options (we take the USB side / an export added to the Warsaw app / Warsaw keeps the desktop app) to be discussed at deployment time (Q2). Early consultation on Warsaw's implementation know-how (protocol quirks) would save investigation effort.
- **Logbook view** (R11): run metadata (start/stop, configId, output files, event counts, …) and the operation audit log flow into **one JSONL timeline**, so a "Log" tab that renders it already is a logbook: who did what when, and what happened in which run. **Free-text shift comments can be appended to the same timeline (a lightweight electronic logbook; fixed)** — there is no downside to having it. There will be four writers (the controller's run records and audit log, UI shift comments, psu TRIPs), so **appends to the JSONL are funneled through a single writer, the controller**; other components post via ZMQ (prevents interleaved lines and order inversion; to be defined in the specification).

## 6. Throughput and data-volume estimates (minimum target = 100 Hz trigger)

The current trigger rate is 4 Hz (~20 Hz even with an upgraded beam), but the **minimum target is 100 Hz** (updated from 80 Hz on 2026-08-12). Background: an idea is emerging to place a **solid target inside the detector** (instead of the gas target) and detect the outgoing particles; Prof. dr hab. Wojciech Dominik (Institute of Experimental Physics, Faculty of Physics, Warsaw University) is considering 100 Hz. 100 Hz is an acceptance criterion, not a ceiling — **no hard cap in software; the ceiling should be set by the physics of links, disks and the GET front-end** (go as far as they allow).

| Item | mini eTPC (256 ch) | ELITPC (~1024 ch) |
|------|--------------------|-------------------|
| Event size (full readout, 512 buckets × 2 B) | ~0.28 MB | ~1.1 MB |
| Trigger rate (minimum target) | 100 Hz | 100 Hz |
| Peak throughput | ~28 MB/s | **~111 MB/s (≈0.9 Gbit/s, 2 CoBos combined)** |
| Disk (sustained at target rate) | ~2.4 TB/day | **~9.6 TB/day** |

CPU remains comfortable. On the network, the ELITPC's ~111 MB/s splits into **2 CoBos × ~56 MB/s (~0.45 Gbit/s per CoBo link)**, so the CoBo-side links are fine, but the **receiving host's NIC sees ~0.9 Gbit/s combined — on the edge of saturating 1 GbE (TCP goodput ~0.94 Gbit/s)**. Therefore, **full-readout ELITPC at 100 Hz effectively requires either 10 GbE on the receiving side or one NIC per CoBo** (the receiver is an independent component per CoBo, so the architecture supports this as-is). Data links assume MTU 9000. Zero suppression / partial readout stay on the study list as well. Disk needs a real plan at ~9.6 TB/day sustained — a future online filter plus prescaled raw recording (out of scope, but addable later via §5 component insertion) helps exactly here.
The design focus is unchanged: never stall, never lose data, visible in real time.

> Reference (to be measured): full-readout AGET readout is roughly ~1.4 ms/event (68 ch × 512 cells ÷ 25 MS/s), i.e. ~14% dead time at 100 Hz. The practical ceiling of "as far as it goes" will likely be set by the GET front-end rather than by software; measuring the hardware side is part of the P5 hardware verification. Beyond that, the real lever is partial readout / zero suppression (making events smaller) — also addable via §5 component insertion.

## 7. Verification strategy — the existing C++ as oracle

The correctness of the new Rust implementation is checked mechanically against the verified C++ assets:

1. **Decoder**: reproduce `events=108 / items=15,040,512 / malformed=0` on the real 2025-run graw (29 MB) — the answer pinned by the C++ version. Support both frameType 1 (2018) and frameType 2 (2025; blkSize256/big-endian).
2. **graw writer**: byte-exact match with the replay input (same criterion as C++ M2).
3. **ROOT output**: compare the TTree definition ported into root_sink against the C++ tpcdaq output on identical input, at TTree level (guarantees graw2root compatibility — the port comes from `RootWriter`, so the check is mechanical).
4. **WS protocol**: the wire protocol is redefined in the specification along with the new UI; the existing cross-language conformance-test method (generate samples → machine-verify on both ends) carries over to the new protocol as-is.
5. **Run control**: e2e against the existing fake-ECC servant (test harness); real ECC first in a container, then on hardware. The real ECC is built from **the same version used in the experiment** (`~/WorkSpace/HIGS/2026/20190315_patched`, obtained 2026-08-10). Compared against the vendored version: all six .ice files, MDaq, GetController, MultiFrame sources and port definitions are identical (differences only in four MuTanT configuration files; no client-side impact).
6. **Fully verifiable offline**: `graw_replay` + fake-ECC exercise everything without a detector (same approach as the C++ version); hardware is for final acceptance only. The multi-CoBo path (event-builder multi-source merge) is also verified with **two graw_replay instances running in parallel** — no 2-CoBo hardware is available before Warsaw (ELITPC), so this is pinned down offline.

## 8. Roadmap (numbered TODO units are opened when work starts)

| Phase | Content | Exit criteria |
|-------|---------|---------------|
| P0 | Component skeleton (ZMQ wiring), configuration loading, geometry abstraction (mini/ELITPC) | ELITPC .dat loads; ch→(plane,strip) lookup works |
| P1 | Reception (one receiver per CoBo, N via configuration) + framer + decoder | Oracle match (§7-1); replay with 0 dropped (including load equivalent to 100 Hz) |
| P2 | graw writer (per-CoBo files) + root-sink attachment (event builder [pass-through for single CoBo] + TTree) | Byte-exact + TTree compatibility (§7-2,3); single ROOT file per run; two-source build verified with two parallel graw_replay instances (pre-verification of ELITPC = 2 CoBos) |
| P3 | Monitor accumulation + WS + new UI (the 9 histograms of R3/R4 + R9 event display + R13 waveform view) + R10 write-out at stop | Live display in the new UI; conformance tests green; `run<N>_monitor.root` written on stop |
| P4 | ecc-bridge + run control + run records + logbook view (R6/R7/R11) | fake-ECC e2e green; run metadata, audit log and comments persist in JSONL and are readable in the UI |
| P5 | Real-ECC container verification → hardware acceptance (**thorough verification on ELI-NP's mini TPC** → Warsaw's ELITPC) | Full-replacement operation on hardware. Acceptance is quantitative (e.g. replay match + N hours continuous with 0 drops at 100 Hz-equivalent load; N defined in the specification) — side-by-side comparison with dataRouter is impossible since a CoBo sends to a single target |
| P6 | (Bonus) psu component (R12: HiVolta + HMP2020 monitoring + control) + Power panel UI | Real V/I visible in the UI; ON/OFF and VSET operable; TRIPs auto-recorded in the R11 logbook |

Decoding, recording and run control (P1/P2/P4) retrace a road already traveled in the C++ version (specifications, tests and answers all exist); the event builder is reused from delila-rs root_sink (eb_core). Genuinely new design: the P0 geometry generalization, the R4 charge histograms, the new UI + new WS protocol in P3, and the P6 psu. P6 is independent of P0–P5 and can be deployed early at ELI-NP, where no competing application exists (Q2). Note, however, that the root_sink "reuse" is a skeleton, not the body (the TTree part is a port plus a lossless-transport conversion — see the caution in §9), so P2 must not be estimated as "cheap because it is reuse".

No calendar estimate is given, deliberately — we do not commit to dates before the specification is fixed. A ranged target will be shared once the specification is settled.

## 9. Existing assets

- `tpcdaq/` (C++): kept alive as **reference + oracle + satellite source**, not frozen.
- delila-rs: the source of the component system, the ZMQ design and the Angular UI patterns. **`tools/root_sink` (C++)** is the reuse source for the event builder (eb_core) + ROOT writing (ZSTD-compressed TTrees / TH1D/TH2D). DELILA-specific parts (event format, Δt monitor) are rewritten for the TPC; the reuse is the skeleton (ZMQ subscription, run state, builder, histogram registry/writing). TTree content is ported from `RootWriter` (§4 D2).
  **Caution (the largest surgery in the reuse)**: root_sink was designed as a **non-authoritative** consumer that additionally subscribes to the merger's PUB ("never gates the pipeline; dropping is acceptable" — per its source header). In tpcdaq-rs the ROOT stream is **lossless** (§5: PUSH/PULL backpressure), so the subscription-side transport semantics must be inverted.
- `reuse/rust_reference/` (geometry.rs / recon.rs): seeds for the P0 geometry and FPN reordering. W-consistency ghost suppression is not needed for now, since online reconstruction is out of scope (R5).
- webui (plain JS): **not reused (the UI is built new — too many planned additions)**. Only the cross-language conformance-test method for the WS protocol carries over to the new protocol design (§7-4).

## 10. Open items (discussion / questions for Warsaw)

Nearly everything was settled in the 2026-08-10 discussion. Three items remain open:

| # | Item | Working assumption |
|---|------|--------------------|
| Q1 | Detailed **field list** of the run metadata (JSONL; configId, geometry, file list, counts, …) | Field table defined in the specification |
| Q2 | (Bonus R12) **Only for Warsaw deployment**: coexistence with the existing desktop application (contention for HiVolta's single TCP connection; none at ELI-NP) + consultation on Warsaw's implementation know-how. On-site hardware checks: HMP2020 LAN option presence / `SYST:MIX` behavior | Can start at ELI-NP first; discuss at Warsaw deployment time |
| Q3 | Confirm the **failure unit** of the current GET "merging scrambles order" problem (per-CoBo files or per-AsAd) and **verify the cause hypothesis (arrival-order concatenation — §5)** — together with the Q2 consultation | Failure unit presumed per-CoBo; cause presumed arrival-order concatenation (both unverified). Impact is limited since the builder merges per CoBo |

**Settled (2026-08-10 discussion)**:
- Satellite attachment → resolved by D4 ZMQ componentization. Project name → D1 `tpcdaq-rs`.
- Warsaw contribution model → D5 "solo development → hardware-test request once usable".
- Charge histogram definition → §2 R4 (two kinds per plane; primary purpose saturation monitoring; reset at run start; R10 write-out at stop).
- Run metadata location → JSONL, browsed via the R11 logbook view.
- **UI → built new** (existing webui not reused — too many planned additions). **Stack = Angular + Angular Material + ECharts, the same as the delila-rs operator UI** (aligning the UI with delila-rs like the architecture; brings proven run-control screens, configuration forms and live-plot patterns; also matches the C++ version's Angular assumption). Design-decision reference = the **Atlassian Design guidelines** (semantic-color discipline, destructive-action confirmation, status-display conventions); visual model for the monitor pages = **Grafana** (dark theme, control-room legibility).
- **Repository → `tpcdaq-rs` only** (the existing test/get assets are not uploaded — it would never end; kept as local reference).
- **Multiple CoBos designed in from the start** (§5: per-CoBo receivers + per-CoBo raw graw files + event builder; in-house verification is single-CoBo only). Event builder reuses root_sink (eb_core); built event data targets a single file per run (2026-08-12).
- **ELITPC CoBo configuration → 2 CoBos, confirmed (2026-08-12)**: one CoBo (FPGA board) reads out 2 AsAds, so the 4-AsAd ELITPC is 2 CoBos. **Multi-CoBo is a production requirement, not future-proofing**; the event builder is mandatory for the ELITPC (§5; verified with two parallel graw_replay instances — §7-6).
- **Authentication → the two-tier scheme of §5.**
- **monitor.root → written by C++ (root_sink reuse); changed 2026-08-12**: oxyroot re-verification showed its public API has no histogram type and writing covers TTrees only (§5; reverses the 2026-08-10 "directly from Rust/oxyroot" decision).
- **Free-text logbook comments → included** (part of R11).

---
*Related documents (internal, in Japanese — key points can be translated into English on request): `ASSESSMENT_移植検討.md` (problem analysis of the current DAQ) / `DESIGN_newdaq.md` (C++ version design; many decisions carry over) / `brain_stoem_memo` (input notes for this proposal)*
