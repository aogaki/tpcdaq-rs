// =============================================================================
// analyzeUVW.C - step 2: hit building, UVW plots, charge / signal-width.
//
// Takes the PURE raw-waveform tree from graw2root.C and does all analysis:
//   0. pedestal subtraction (per channel, quiet-window mean), ADC threshold,
//      channel -> (view, strip position) mapping from channel_map.csv,
//      time cell -> drift z [mm] with the nominal drift velocity
//   1. per view: seed a straight line on the high-charge hits (weighted PCA)
//   2. keep only hits inside a band around that line (kills noise speckle)
//   3. signal width = charge-weighted RMS of the perpendicular residuals
//   4. slice the drift axis on a global grid and store the charge centroid of
//      every slice (these centroids are the input for makeTracks.C)
//
// Output: <input>_uvw.root
//   TTree "uvw": eventId, qview[3], width[3], nhits[3]
//                + slice vectors slView/slZ/slS/slQ
//   histograms:  hCharge (event charge), hWidth_U/V/W
//   canvases:    UVW hit maps of the first NDISPLAY events
//
// Usage:  root -l -b -q 'analyzeUVW.C("CoBo_xxx_raw.root")'
// =============================================================================
#include <TCanvas.h>
#include <TFile.h>
#include <TH1F.h>
#include <TH2F.h>
#include <TString.h>
#include <TTree.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <iostream>
#include <map>
#include <sstream>
#include <string>
#include <vector>

// ---- analysis constants -----------------------------------------------------
const double DRIFT_HV_V = 1200.0;    // drift cage voltage [V]
const double DRIFT_LEN_CM = 20.0;    // drift length [cm]
const double PRESSURE_MBAR = 100.0;  // CO2 pressure [mbar]
const double SAMPLING_MHZ = 25.0;    // AGET sampling rate
const double ADC_THRESHOLD = 35.0;   // keep hits above pedestal + this
const int PED_LO = 1, PED_HI = 20;   // pedestal window [time cells]
const double SEED_ADC = 100.0;       // hits above this seed the line fit
const double BAND_MM = 10.0;         // half-width of the accepted band
const double SLICE_MM = 3.0;         // drift-axis slice for track building
const int NDISPLAY = 5;              // events to draw as example hit maps
// -----------------------------------------------------------------------------

// CO2 drift velocity from the reduced field E/p. All seven rows of the
// HIGS/ELITPC operating-point table (Magboltz-derived; 130/190/250 mbar,
// ~293 K) collapse onto v = 0.7233 * (E/p) within +-0.2% for
// E/p = 0.54-1.05 V/(cm*mbar); the mini-eTPC working point (0.80) is an
// interpolation inside that range. Still table-derived, not measured -
// confirm with the z-edge method before trusting absolute 3D lengths.
double driftVelocityCmUs(double hvVolt, double lenCm, double pMbar)
{
  const double MOBILITY = 0.7233;  // (cm/us) per (V/cm/mbar)
  const double eOverP = hvVolt / lenCm / pMbar;
  if (eOverP < 0.54 || eOverP > 1.05)
    std::cout << "WARNING: E/p = " << eOverP
              << " V/(cm*mbar) is outside the 0.54-1.05 range of the "
                 "operating-point table; v_drift is an extrapolation here."
              << std::endl;
  return MOBILITY * eOverP;
}

void analyzeUVW(const char *rawFile)
{
  // --- channel map (aget, raw channel -> view, strip position) ---------------
  int chView[4][68];
  float chPos[4][68];
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c) chView[a][c] = -1;
  std::ifstream fm("channel_map.csv");
  if (!fm) {
    std::cout << "channel_map.csv not found (run from the macros folder)"
              << std::endl;
    return;
  }
  for (std::string line; std::getline(fm, line);) {
    if (line.empty() || line[0] == '#') continue;
    std::istringstream ss(line);
    int a, c, v, strip;
    float pos;
    char comma;
    if (ss >> a >> comma >> c >> comma >> v >> comma >> strip >> comma >> pos) {
      chView[a][c] = v;
      chPos[a][c] = pos;
    }
  }
  // per-view strip axis: ONE bin per strip (bin width == 1.5 mm pitch), so
  // the hit maps cannot alias against the strip pitch (white-line moire) and
  // the outermost strips are never clipped by a fixed axis range
  double sMin[3] = {1e9, 1e9, 1e9}, sMax[3] = {-1e9, -1e9, -1e9};
  int nStrips[3] = {0, 0, 0};
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c) {
      const int v = chView[a][c];
      if (v < 0) continue;
      sMin[v] = std::min(sMin[v], (double)chPos[a][c]);
      sMax[v] = std::max(sMax[v], (double)chPos[a][c]);
      ++nStrips[v];
    }
  const double PITCH = 1.5;  // mm, uniform in channel_map.csv (all views)
  const double driftV =
      driftVelocityCmUs(DRIFT_HV_V, DRIFT_LEN_CM, PRESSURE_MBAR);
  std::cout << "drift velocity " << driftV
            << " cm/us  (E = " << DRIFT_HV_V / DRIFT_LEN_CM
            << " V/cm, E/p = " << DRIFT_HV_V / DRIFT_LEN_CM / PRESSURE_MBAR
            << " V/(cm*mbar))" << std::endl;
  const double mmPerCell = driftV / SAMPLING_MHZ * 10.0;

  // --- input / output ---------------------------------------------------------
  TFile in(rawFile);
  TTree *tree = (TTree *)in.Get("raw");
  UInt_t eventId;
  static Short_t adc[4][68][512];
  tree->SetBranchAddress("eventId", &eventId);
  tree->SetBranchAddress("adc", adc);

  TString outName(rawFile);
  outName.ReplaceAll("_raw.root", "_uvw.root");
  TFile out(outName, "RECREATE");
  TTree utree("uvw", "per-view analysis + slice centroids");
  float qview[3], width[3];
  int nhits[3];
  std::vector<int> slView;
  std::vector<float> slZ, slS, slQ;
  utree.Branch("eventId", &eventId);
  utree.Branch("qview", qview, "qview[3]/F");
  utree.Branch("width", width, "width[3]/F");
  utree.Branch("nhits", nhits, "nhits[3]/I");
  utree.Branch("slView", &slView);
  utree.Branch("slZ", &slZ);
  utree.Branch("slS", &slS);
  utree.Branch("slQ", &slQ);

  TH1F hCharge("hCharge", "event charge (all views);charge [ADC];events", 4097,
               -0.5, 4096.5);
  TH1F *hWidth[3];
  const char *vn[3] = {"U", "V", "W"};
  for (int v = 0; v < 3; ++v)
    hWidth[v] =
        new TH1F(Form("hWidth_%s", vn[v]),
                 Form("signal width %s;width [mm];events", vn[v]), 60, 0, 15);

  for (Long64_t i = 0; i < tree->GetEntries(); ++i) {
    tree->GetEntry(i);

    // 0. build hits: pedestal subtraction + threshold + mapping
    std::vector<double> vz[3], vs[3], vq[3];
    for (int a = 0; a < 4; ++a) {
      for (int c = 0; c < 68; ++c) {
        int v = chView[a][c];
        if (v < 0) continue;  // FPN / unmapped channel
        double ped = 0;
        for (int t = PED_LO; t <= PED_HI; ++t) ped += adc[a][c][t];
        ped /= (PED_HI - PED_LO + 1);
        for (int t = 0; t < 512; ++t) {
          double val = adc[a][c][t] - ped;
          if (val <= ADC_THRESHOLD) continue;
          vz[v].push_back(t * mmPerCell);
          vs[v].push_back(chPos[a][c]);
          vq[v].push_back(val);
        }
      }
    }

    slView.clear();
    slZ.clear();
    slS.clear();
    slQ.clear();
    double qEvent = 0;

    for (int v = 0; v < 3; ++v) {
      qview[v] = 0;
      width[v] = 0;
      nhits[v] = 0;
      size_t n = vz[v].size();
      if (n < 10) continue;

      // 1. charge-weighted PCA line on the high-charge (seed) hits
      double sw = 0, mz = 0, ms = 0;
      for (size_t k = 0; k < n; ++k)
        if (vq[v][k] > SEED_ADC) {
          sw += vq[v][k];
          mz += vq[v][k] * vz[v][k];
          ms += vq[v][k] * vs[v][k];
        }
      if (sw <= 0) continue;
      mz /= sw;
      ms /= sw;
      double czz = 0, css = 0, czs = 0;
      for (size_t k = 0; k < n; ++k)
        if (vq[v][k] > SEED_ADC) {
          double dz = vz[v][k] - mz, ds = vs[v][k] - ms;
          czz += vq[v][k] * dz * dz;
          css += vq[v][k] * ds * ds;
          czs += vq[v][k] * dz * ds;
        }
      // principal axis of the 2x2 covariance matrix (largest eigenvalue)
      double tr = czz + css, det = czz * css - czs * czs;
      double lam = 0.5 * tr + std::sqrt(std::max(0.25 * tr * tr - det, 0.0));
      double dz0 = czs, ds0 = lam - czz;  // eigenvector
      double nn = std::hypot(dz0, ds0);
      if (nn < 1e-9) {
        dz0 = 1;
        ds0 = 0;
        nn = 1;
      }
      dz0 /= nn;
      ds0 /= nn;

      // 2.+3. band selection around the line; width from kept hits;
      // 4. slice the drift axis on a GLOBAL grid (bin = floor(z/SLICE_MM)) so
      //    the U/V/W centroids of the same slice line up for makeTracks.C
      double wsum = 0, perp2 = 0;
      std::map<int, std::pair<double, double>> slices;  // bin -> (sum q, q*s)
      for (size_t k = 0; k < n; ++k) {
        double perp = -(vz[v][k] - mz) * ds0 + (vs[v][k] - ms) * dz0;
        if (std::fabs(perp) > BAND_MM) continue;
        hCharge.Fill(vq[v][k]);
        wsum += vq[v][k];
        perp2 += vq[v][k] * perp * perp;
        ++nhits[v];
        auto &acc = slices[(int)std::floor(vz[v][k] / SLICE_MM)];
        acc.first += vq[v][k];
        acc.second += vq[v][k] * vs[v][k];
      }
      if (wsum <= 0) continue;
      qview[v] = wsum;
      width[v] = std::sqrt(perp2 / wsum);
      qEvent += wsum;
      hWidth[v]->Fill(width[v]);
      for (auto &kv : slices) {
        slView.push_back(v);
        slZ.push_back((kv.first + 0.5) * SLICE_MM);
        slS.push_back(kv.second.second / kv.second.first);
        slQ.push_back(kv.second.first);
      }
    }
    // hCharge.Fill(qEvent);
    utree.Fill();

    // example UVW hit maps for the first few events
    if (i < NDISPLAY) {
      TCanvas c(Form("evt_%u", eventId), Form("event %u", eventId), 1500, 500);
      c.Divide(3, 1);
      for (int v = 0; v < 3; ++v) {
        c.cd(v + 1);
        TH2F *h = new TH2F(
            Form("h%u_%s", eventId, vn[v]),
            Form("event %u  %s;drift z [mm];%s [mm]", eventId, vn[v], vn[v]),
            150, 0, 150, nStrips[v], sMin[v] - PITCH / 2, sMax[v] + PITCH / 2);
        for (size_t k = 0; k < vz[v].size(); ++k)
          h->Fill(vz[v][k], vs[v][k], vq[v][k]);
        h->SetStats(0);
        h->Draw("colz");
      }
      out.cd();
      c.Write();
    }
  }
  out.cd();
  utree.Write();
  hCharge.Write();
  for (int v = 0; v < 3; ++v) hWidth[v]->Write();
  out.Close();
  printf("wrote %s\n", outName.Data());
}
