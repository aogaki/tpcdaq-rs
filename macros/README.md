# mini-eTPC standalone ROOT macros

Self-contained analysis chain for mini-eTPC GRAW data. Needs **only ROOT**
(6.x) - no GET software, no TPCReco. Copy this folder anywhere and run.

```
root -l -b -q 'graw2root.C("CoBo_xxx_0000.graw", -1)'   # 1. graw -> ONE raw tree
root -l -b -q 'analyzeUVW.C("CoBo_xxx_raw.root", 91, 25, 1200)'   # 2. UVW plots, charge, width (args: pressure [mbar], sampling [MHz], drift HV [V]; all required)
root -l -b -q 'makeTracks.C("CoBo_xxx_uvw.root")'       # 3. 3D tracks
```

Step 2 in parallel (events are independent; N processes on entry ranges,
merged with `hadd`, same output name as the serial run; TODO/169):

```
./par_analyzeUVW.sh -p 91 -s 25 -d 1200 [-j N] [-o out_uvw.root] [--tracks] CoBo_xxx_raw.root
```

N defaults to the number of cores - 2; `--tracks` runs makeTracks.C on the
merged file. run 175 (1.06 M events): 90 s with N = 12 vs ~14 min serial.
`./test_par_analyzeUVW.sh <raw.root> <workdir>` checks serial == parallel
(`compareUVW.C`, every entry / branch / bin).

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
drift velocity, band width) sit at the top of analyzeUVW.C -
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

## Alpha lines with stoppedAlphas.C (228Th source)

```
root -l -b -q 'stoppedAlphas.C("run0175_tracks.root", -386, 91)'   # source x [mm], pressure [mbar]
```

- Both arguments are required. Pass the **real run pressure**: with 100 mbar
  instead of 91 the energies still look plausible, but only with the source
  moved to about -350 (TODO/168).
- Current setup (source outside the entrance wall, x ~ -370, CO2 91 mbar):
  three lines stop inside the cage - **212Bi** (shallowest; its short side is
  cut by the trigger threshold, ~0.33 MeV of charge), **220Rn** and **216Po**;
  212Po crosses. Expect 220Rn : 216Po ~ 1 and 212Bi / 220Rn a bit below the
  branching ratio 0.359 (run 175: -28.2 / -8.2 / +37.8 mm, 220Rn : 216Po = 1.02,
  212Bi / 220Rn = 0.30).
- Each line is fitted within +-9 mm of its ASTAR prediction. At the nominal
  density (p, 20 C) the prediction for x = -370 misses the lines by ~9 mm, so
  start from **-386** (TODO/100). The macro then solves the gas density and the
  source distance from the lines, e.g. `density fit (2 lines): rho = ... nominal
  x 1.023 ... source distance d = 323 mm (x_src = -376)`; the residual of the
  line not used in the fit is an independent check (+0.4 mm for 212Bi).
  Re-run with the fitted values (e.g. `-376, 93.1`) for the energy at emission
  (run 175: table + 0.02 MeV, FWHM 1.5 % for all three lines).
- **Density and source distance are degenerate** in the energies (the energy
  lost before the wall depends on density x distance). The spacing between
  lines (220Rn -> 216Po ~ 46 mm) fixes the density; a wrong density shows up as
  an energy offset that grows with the line energy. The density excess left
  over (x 1.02 of nominal) is temperature or pressure-gauge offset - a
  thermometer would separate them.

## Assumptions / caveats

- Data format: CoBo full readout, single AsAd; **frameType 1 (4-byte items) and
  frameType 2 "compact" (2-byte items, per-AGET channel/cell cursors — same rule as
  the tpcdaq-rs decoder)** are both decoded. 12-byte topology frames are skipped.
- `channel_map.csv` maps (AGET, raw channel) to (view, strip position). It was
  generated from TPCReco's `geometry_mini_eTPC.dat` (tools/eventdisplay/mapDump);
  regenerate it if the pad plane or cabling changes.
- The z scale uses a drift velocity **computed from the run conditions**
  (`DRIFT_LEN_CM` at the top of analyzeUVW.C; the pressure, sampling rate and
  drift voltage are the 2nd-4th arguments, all required - pass the real run
  values, and the same pressure to stoppedAlphas.C, whose source x and
  pressure are also required)
  via the CO2 mobility law v = 0.7233 x (E/p) [cm/us per V/(cm*mbar)],
  interpolated from the HIGS/ELITPC operating-point table (valid for
  E/p = 0.54-1.05). It is table-derived, NOT measured: confirm with the
  z-edge method before trusting absolute lengths and angles involving z.
- **Sampling rate and drift voltage are arguments** of analyzeUVW.C (TODO/178),
  not read from the data and with no default: sampling in {100, 50, 25, 12.5}
  MHz, drift HV > 0 V (else an error and no output). Runs up to 56 were taken
  at 100 MHz (z comes out 4x too long if you pass 25); run 180 on is 12.5 MHz
  at 1600 V. The run's constants are recorded in the output as
  `TParameter<double>`: `pressure_mbar`, `sampling_MHz`, `drift_hv_V`,
  `drift_v_cm_us`, `mm_per_cell`, `z_window_mm` (= 512 x mm_per_cell) - in
  `*_uvw.root`, copied by makeTracks.C into `*_tracks.root` (an old uvw file
  without them gives a WARNING). stoppedAlphas.C takes its z window from
  `z_window_mm` and stops with an error on an older tracks file (re-run
  analyzeUVW -> makeTracks); it warns if its pressure argument differs from
  the file's `pressure_mbar` by 0.5 mbar or more.
- **Head/tail is not determined**: the charge-asymmetry orientation in
  makeTracks.C carries no reliable information. Alphas that cross the cage
  have no Bragg peak inside; alphas that stop inside do (see below), but no
  Bragg-based orientation is implemented. Use `|direction|` or fix the sign
  from your source geometry (stoppedAlphas.C aligns tracks to +x).
- Every event is fitted as ONE straight track; events where U, V or W has
  fewer than 3 strip hits get `ok == 0`.

Validated against the full TPCReco/simplefit pipeline on run
2026-09-02T14:30:48.869: median axis difference 0.8 deg, median length
difference 0.4 mm, at ~125 events/s for the conversion step (this was the
drift-slice chain before TODO/094; not re-validated since).
