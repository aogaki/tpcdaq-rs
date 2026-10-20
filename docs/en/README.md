# English translations

These are LLM translations of the Japanese documents that the October 2026 scan report
([`docs/reports/2026-10_scan/scan_report_2026-10_en.pdf`](../reports/2026-10_scan/scan_report_2026-10_en.pdf)) refers to.
All were made from commit `20d31da` (2026-10-20).

**The Japanese files are authoritative.** If a translation and its original differ, the original wins.
Do not edit the translations by hand: fix the Japanese and regenerate. Each file names its original in its header.
Other documents in the repository are Japanese only; ask S. Aogaki if you need one of them in English.

| Translation | Original | Contents |
|---|---|---|
| [SCANS_en.md](SCANS_en.md) | [`docs/SCANS_ja.md`](../SCANS_ja.md) | Scan ledger S1–S19: one section per scan with settings, runs and the result table |
| [SPEC_en.md](SPEC_en.md) | [`docs/SPEC_ja.md`](../SPEC_ja.md) v1.49 | DAQ specification: components, messages, configuration, root-sink, logbook, WS protocol, acceptance, on-hardware checks |
| [RUNBOOK_en.md](RUNBOOK_en.md) | [`docs/RUNBOOK_ja.md`](../RUNBOOK_ja.md) | Operating procedures, including ZS operation |
| [tickets/](tickets/) | [`TODO/archive/135–161`](../../TODO/archive/) | Work tickets behind the scans: plan, work order, measured results, rulings |

## Tickets

| # | Title |
|---|---|
| [135](tickets/135_param_scan_driver.md) | Parameter scan driver (config list → PUT /api/params → run → wait → stop → record) |
| [136](tickets/136_run_metrics.md) | One run → metrics JSON (band position, clip rate and saturation rate from `_raw.root` + track statistics from the macros' tracks) |
| [137](tickets/137_peaking_time_scan.md) | peakingTime scan (70 / 232 / 502 / 1014 ns, 25 MHz, triggerDelay 1750, α source) |
| [138](tickets/138_pulser_response_kernels.md) | Response kernels of the shaper (AsAd built-in pulser, 25 MHz, 4 peaking points × 2 amplitudes) and the origin of the "wiggle" |
| [139](tickets/139_deconvolution_prototype.md) | Prototype of shaping deconvolution (restore the α waveforms with the 138 kernels) |
| [140](tickets/140_gain_scan.md) | Gain scan (502 ns × {120 fC, 240 fC, 1 pC}, 1014 ns × 1 pC, 25 MHz, triggerDelay 1750, α source) |
| [141](tickets/141_readout_threshold_scan.md) | Scan of readout mode / hit register threshold / ZS (real tracks, E setting 502 ns × 240 fC) |
| [142](tickets/142_gem_voltage_margin_scan.md) | Proposal for a scan of the GEM voltage margin (lowering direction only; voltage operation by the user, DAQ only runs and aggregates) |
| [143](tickets/143_scan_ledger.md) | Scan ledger (docs/SCANS_ja.md) and the table-generating tool (tools/scan/ledger.py) |
| [144](tickets/144_elitpc_wiki_migration.md) | Migrate the old TPC machine's PukiWiki (ELI-TPC Wiki) to ELITPC |
| [145](tickets/145_caen_geco_on_elitpc.md) | Connect CAEN GECO2020 (snap) to the DT1415ET on ELITPC (set the HV with this, with the DAQ shut down) |
| [146](tickets/146_hit_z_residual_vs_peaking.md) | Measure the z residual of strip hits (difference from the track line) for each peaking / HV (material for deciding whether to use the signal width as the error) |
| [147](tickets/147_drift_voltage_scan.md) | Drift cage voltage scan (1,000 / 1,200 / 1,400 V, on site, operating point B) |
| [148](tickets/148_uvw_closure_residual.md) | Measuring the accuracy of the 2D (strip-plane) centroid with U/V/W closure residuals (per peaking time / HV / drift voltage) |
| [149](tickets/149_zs_per_channel_and_zs_aware_metrics.md) | Per-channel ZS thresholds (generating exceptions from the pedestal table) and a run_metrics that supports ZS data |
| [150](tickets/150_maxrate_B_partial_zs.md) | Maximum-rate test 2: operating point B (full) / hit-register partial (g4) / ZS (per channel) — what sets the limit (FW readout time vs 1 GbE) |
| [151](tickets/151_root_sink_pedestal_table_zs.md) | root_sink: pedestal table (`[system] ped_table`) and a ZS-aware PEventTPC fill |
| [152](tickets/152_monitor_zs_restore.md) | monitor: ZS restoration of the 0x02 grid with `[system] ped_table` (fixing the live reconstruction and the baseline of the UI) |
| [153](tickets/153_uvw_closure_pedjson.md) | `pedJson` for uvw_closure.C (the same table path as run_metrics) — optional |
| [154](tickets/154_zs_restore_bridge.md) | `tools/scan/zs_restore.C` (ZS raw -> raw with suppressed cells filled by the ped) + `metrics_for_run.sh --zs-fill` + a patch proposal for analyzeUVW.C (wording only) |
| [155](tickets/155_zs_params_procedure.md) | `tools/scan/zs_params.sh` (pedestal run -> table -> params with ZS) + RUNBOOK "ZS operation" |
| [156](tickets/156_zs_validation_runs.md) | Validation alpha runs for ZS (B + ZS, B + hit partial g4 + ZS, with / without FPN) -> SCANS S18, deciding whether to keep the FPN |
| [157](tickets/157_spec_v149_zs.md) | SPEC v1.49 (ZS support) + WARSAW_PLAN §7 |
| [158](tickets/158_decoder_frametype_counters.md) | decoder: per-frameType counters + a size check of compact (frameType 2) |
| [161](tickets/161_gas_flow_runs_174_175.md) | Comparison of gas flow 100 -> 50 ml/min (runs 174 / 175, weekend run) |

159 and 160 are not included (still open when the translations were made).
