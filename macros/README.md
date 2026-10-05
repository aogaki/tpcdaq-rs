# mini-eTPC standalone ROOT macros

Self-contained analysis chain for mini-eTPC GRAW data. Needs **only ROOT**
(6.x) - no GET software, no TPCReco. Copy this folder anywhere and run.

```
root -l -b -q 'graw2root.C("CoBo_xxx_0000.graw", -1)'   # 1. graw -> ONE raw tree
root -l -b -q 'analyzeUVW.C("CoBo_xxx_raw.root")'       # 2. UVW plots, charge, width
root -l -b -q 'makeTracks.C("CoBo_xxx_uvw.root")'       # 3. 3D tracks
```

graw2root.C merges a whole run into one ROOT file: give it the first chunk
and the following `_0001, _0002, ...` chunks are picked up automatically,
or give it a text file (e.g. `rawFileList.dat`, one GRAW path per line,
`#` comments) for arbitrary file sets. Uncleaned files are fine - topology
frames are skipped wherever they appear.

**Step 1 is optional when the data come from the tpcdaq-rs DAQ**: its
root-sink started with `--raw-tree` writes `run{N}_raw.root` next to the
run's `run{N}.root` in exactly this format (same tree `raw`, same branches,
verified entry-by-entry against graw2root.C on a real mini run - SPEC §6.7).
Feed that file straight into analyzeUVW.C. graw2root.C stays the tool for
.graw files from any other source (GetController, old runs, other DAQs).

| step | input | output | content |
|---|---|---|---|
| rateVsTime.C | .graw       | `*_rate.root/.png` | header-only scan (no waveforms): event rate vs time (bin width in s), inter-event Δt (linear + log10), DAQ restarts marked. CoBo timestamp = 10 ns ticks (checked against wall clock). Fast: 10k frames in seconds |
| chargeVsTime.C | `*_uvw.root` + `*_raw.root` | `*_charge_vs_time.root/.png` | joins per-event charge (analyzeUVW) with the CoBo timestamp (graw2root): median charge / hits per time bin, early-vs-late charge spectra, rate. Tells a counting effect (decaying source) from a signal effect (attachment, gain) |
| graw2root.C  | .graw       | `*_raw.root`    | tree `raw`: PURE lossless conversion - full waveforms `adc[4][68][512]`, eventId, 48-bit eventTime. No pedestal / threshold / geometry. Reads frameType 1 (2018, 4-byte items) and frameType 2 (rev 5 compact, 2-byte items — what tpcdaq-rs/zCoBo write today) |
| analyzeUVW.C | `*_raw.root` | `*_uvw.root`   | ALL analysis: pedestal, ONE hit per strip (peak time -> z, integrated charge q, halo cut `STRIP_Q_MIN`), channel map, z scale; tree `uvw`: per-view charge, signal width, band-selected strip hits; histograms `hCharge` (strip-hit charge), `hWidth_U/V/W`; example UVW canvases |
| makeTracks.C | `*_uvw.root` | `*_tracks.root` | tree `tracks`: straight-line 3D track per event (start/end = 2%-98% charge quantiles, direction, length, charge, dEdx = charge/length; `xStartF`..`zEndF`, `lengthF` = outermost hits, for range measurements). xy direction from the charge-weighted variances of the U/V/W strip positions (no z), dip from a z-vs-path-length fit |
| sourceTracks.C | `*_tracks.root` | `*_src.root` | event filter: tracks re-oriented to +x, `|phi| <= PHI_MAX_DEG` and the extended line within `SRC_R_MM` of (`SRC_X`, `SRC_Y`); same `tracks` tree, so analyzeTracks.C / analyzeAngles.C run on it unchanged |
| stoppedAlphas.C | `*_tracks.root` | (canvas) | alphas that stop inside the cage (source-like, enter at the left wall, end away from every boundary): residual range (outermost hit) -> energy with the NIST ASTAR projected range in CO2 at the run density (3rd argument [mbar]); E histogram, stop point, charge vs E, energy at emission for a source at x = 2nd argument; fits the 228Th lines in the stop-point histogram and solves gas density + source distance from them |

## The `uvw` strip-hit branches (the analyzeUVW -> makeTracks interface)

analyzeUVW.C makes ONE hit per strip (TODO/094): the cell of the largest
pedestal-subtracted sample is the peak; the hit gets z = charge-weighted time
of the above-threshold cells within +-`PEAK_HALF_CELLS` (5) of the peak and
q = their summed charge. The pulse length (~4.6 us above threshold, ~20 mm of
z) therefore no longer smears z. Strips with q < `STRIP_Q_MIN` (2000) are
dropped: nearly every strip carries a small halo pulse at the track time
(not seen by the FPN channels, so not AGET common mode). Per view, a
charge-weighted line is fitted through the strip hits and only hits within
`BAND_MM` of it are kept. For every kept hit ONE entry is appended to four
parallel vectors (same length, same order); the branch names are those of
the former 3 mm drift-slice centroids:

| branch   | type            | meaning |
|---|---|---|
| `slView` | `vector<int>`   | view of this strip hit: 0=U, 1=V, 2=W |
| `slZ`    | `vector<float>` | drift z of the pulse peak [mm] (charge-weighted, +-5 cells) |
| `slS`    | `vector<float>` | strip position in that view [mm] |
| `slQ`    | `vector<float>` | integrated charge of the peak window [ADC], >= `STRIP_Q_MIN` |

makeTracks.C fits one straight line to the hits of all three views (details
in its header): the charge-weighted variances of s in U, V and W are the
projections of the 2x2 (x,y) covariance of the track charge, which they
determine; its major axis is the xy direction (no z involved, so a flat
track keeps its in-plane angle, and a smearing common to all views - strip
pitch, diffusion - does not bias it). The dip comes from a charge-weighted
fit of z against the path length along that direction, the endpoints from
the 2%-98% charge quantiles of that path length. All three views need >= 3
hits, else `ok == 0`.

**Scale change**: `nhits` now counts strips (not time cells above threshold)
and `qview` sums peak-window charges of halo-cut strips, so both are NOT
comparable with `*_uvw.root` files written before TODO/094.

All tunable constants (thresholds, pedestal window, peak window, halo cut,
drift velocity, sampling rate, band width) sit at the top of analyzeUVW.C -
re-tuning never requires re-reading the GRAW file. Raw files are ~130 KB/event.

**Shell note**: always single-quote the macro call - unquoted parentheses
are shell syntax in zsh/bash: `root -l 'graw2root.C+O("file.graw")'`.

**macOS SDKROOT note**: on this Mac (CLT 27), ACLiC/cling loses track of the
libc++ headers unless `SDKROOT` points at a real SDK - prefix every `root`
invocation with `SDKROOT=$(xcrun --sdk macosx --show-sdk-path)`, e.g.
`SDKROOT=$(xcrun --sdk macosx --show-sdk-path) root -l -b -q 'pulserScan.C(...)'`.

| step | input | output | content |
|---|---|---|---|
| pulserScan.C | 2 CSV from `calib scan --csv-channels ... --csv-waveform ...` | `pulserScan.png` | D-3 pulser amplitude scan (TODO/091 §1): per-channel response vs amplitude (FPN vs signal), residual-fraction distribution vs amplitude, average waveform per amplitude (signal ch mean), rise(10-90%)/peak/FWHM table (stdout + on-canvas, units = ADC sample buckets) |

## Assumptions / caveats

- Data format: CoBo full readout, single AsAd; **frameType 1 (4-byte items) and
  frameType 2 "compact" (2-byte items, per-AGET channel/cell cursors — same rule as
  the tpcdaq-rs decoder)** are both decoded. 12-byte topology frames are skipped.
- `channel_map.csv` maps (AGET, raw channel) to (view, strip position). It was
  generated from TPCReco's `geometry_mini_eTPC.dat` (tools/eventdisplay/mapDump);
  regenerate it if the pad plane or cabling changes.
- The z scale uses a drift velocity **computed from the run conditions**
  (`DRIFT_HV_V`, `DRIFT_LEN_CM` at the top of analyzeUVW.C; the pressure is the
  2nd argument, default `PRESSURE_MBAR` = 100 - pass the real run pressure, and
  the same value to stoppedAlphas.C)
  via the CO2 mobility law v = 0.7233 x (E/p) [cm/us per V/(cm*mbar)],
  interpolated from the HIGS/ELITPC operating-point table (valid for
  E/p = 0.54-1.05). It is table-derived, NOT measured: confirm with the
  z-edge method before trusting absolute lengths and angles involving z.
- **Head/tail is a coin flip**: mini-eTPC alphas cross the detector without
  stopping, so there is no Bragg peak and the charge-asymmetry orientation in
  makeTracks.C carries no real information. Use `|direction|` or fix the sign
  from your source geometry.
- Every event is fitted as ONE straight track; events where U, V or W has
  fewer than 3 strip hits get `ok == 0`.

Validated against the full TPCReco/simplefit pipeline on run
2026-09-02T14:30:48.869: median axis difference 0.8 deg, median length
difference 0.4 mm, at ~125 events/s for the conversion step (this was the
drift-slice chain before TODO/094; not re-validated since).
