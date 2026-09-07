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

| step | input | output | content |
|---|---|---|---|
| graw2root.C  | .graw       | `*_raw.root`    | tree `raw`: PURE lossless conversion - full waveforms `adc[4][68][512]`, eventId, 48-bit eventTime. No pedestal / threshold / geometry |
| analyzeUVW.C | `*_raw.root` | `*_uvw.root`   | ALL analysis: pedestal, threshold, channel map, z scale; tree `uvw`: per-view charge, signal width, drift-slice charge centroids; histograms `hCharge`, `hWidth_U/V/W`; example UVW canvases |
| makeTracks.C | `*_uvw.root` | `*_tracks.root` | tree `tracks`: straight-line 3D track per event (start/end, direction, length, charge, dEdx = charge/length) |

## The `uvw` slice branches (the analyzeUVW -> makeTracks interface)

analyzeUVW.C cuts the drift axis into fixed 3 mm slices (`SLICE_MM`) on a
GLOBAL grid: slice index = `floor(z / 3 mm)`. The grid depends on neither the
event nor the view, so the U/V/W centroids of the same physical z interval
carry the same index - that alignment is the whole point: it is what lets
makeTracks.C combine the three 2D views into 3D points.

Only hits that survived the per-view band cut (within `BAND_MM` of the line
fit) contribute. For every (view, slice) that received any charge, ONE entry
is appended to four parallel vectors (same length, same order):

| branch   | type            | meaning |
|---|---|---|
| `slView` | `vector<int>`   | view of this centroid: 0=U, 1=V, 2=W |
| `slZ`    | `vector<float>` | slice CENTER in drift z [mm] = (index + 0.5) * 3 |
| `slS`    | `vector<float>` | charge-weighted centroid of the strip coordinate in that view [mm] |
| `slQ`    | `vector<float>` | summed band-selected charge of the slice [ADC] |

makeTracks.C groups the entries back by `floor(slZ / 3)` and, for every slice
seen by >= 2 views, solves `s_view = pitch_view . (x,y)` by least squares ->
one 3D point (x, y, z) with weight = summed charge, where z is the
charge-weighted mean of the slice's `slZ`. Events with fewer than
`MIN_SLICES` (3) such points get `ok == 0`.

Caveats:

- The 3 mm grid lives in TWO places: `SLICE_MM` in analyzeUVW.C and the
  literal `3.0` in makeTracks.C. Change one, change both.
- A slice seen by only one view cannot constrain (x,y); makeTracks.C drops it
  (this is why a track clipping a detector corner can lose its tip).

All tunable constants (thresholds, pedestal window, drift velocity, sampling
rate, band width, slice size) sit at the top of analyzeUVW.C - re-tuning never
requires re-reading the GRAW file. Raw files are ~130 KB/event.

**Shell note**: always single-quote the macro call - unquoted parentheses
are shell syntax in zsh/bash: `root -l 'graw2root.C+O("file.graw")'`.

## Assumptions / caveats

- Data format: CoBo **full readout**, frame revision 5, single AsAd
  (mini-eTPC DAQ default). 12-byte topology frames are skipped automatically.
- `channel_map.csv` maps (AGET, raw channel) to (view, strip position). It was
  generated from TPCReco's `geometry_mini_eTPC.dat` (tools/eventdisplay/mapDump);
  regenerate it if the pad plane or cabling changes.
- The z scale uses a drift velocity **computed from the run conditions**
  (`DRIFT_HV_V`, `DRIFT_LEN_CM`, `PRESSURE_MBAR` at the top of analyzeUVW.C)
  via the CO2 mobility law v = 0.7233 x (E/p) [cm/us per V/(cm*mbar)],
  interpolated from the HIGS/ELITPC operating-point table (valid for
  E/p = 0.54-1.05). It is table-derived, NOT measured: confirm with the
  z-edge method before trusting absolute lengths and angles involving z.
- **Head/tail is a coin flip**: mini-eTPC alphas cross the detector without
  stopping, so there is no Bragg peak and the charge-asymmetry orientation in
  makeTracks.C carries no real information. Use `|direction|` or fix the sign
  from your source geometry.
- Every event is fitted as ONE straight track; events with fewer than 3 usable
  drift slices get `ok == 0`.

Validated against the full TPCReco/simplefit pipeline on run
2026-09-02T14:30:48.869: median axis difference 0.8 deg, median length
difference 0.4 mm, at ~125 events/s for the conversion step.
