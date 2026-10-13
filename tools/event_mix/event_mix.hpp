// event_mix.hpp — 実イベントの生波形を足して複数飛跡イベントを作る(TODO/128)。
//
// 入出力は SPEC §6.7 の raw ツリー契約そのまま(macros の analyzeUVW.C が無改造で読める)。
// 足し方:  out[t] = A[t] + Σ_B (B[t − Δ_B] − ped_B)   (全 ch・FPN 込み)
//   ped_B = B のその ch の cell 1..20 の平均(analyzeUVW.C の PED_LO..PED_HI と同じ窓)
//   t − Δ_B が範囲外の cell は 0 を足す。結果は四捨五入して 0..4095 に頭打ち。
// ROOT だけに依存(TPCReco 不要)。ROOT 部分は I/O 関数に閉じ、足し算の核は純粋関数。
#ifndef TPCDAQ_EVENT_MIX_HPP
#define TPCDAQ_EVENT_MIX_HPP

#include <TFile.h>
#include <TLeaf.h>
#include <TTree.h>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <random>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

namespace evmix {

constexpr int kCells = 512;
constexpr int kPedLo = 1, kPedHi = 20;  // analyzeUVW.C の PED_LO / PED_HI
constexpr int kAdcMax = 4095;           // 12 bit

// 出力 1 イベントの組み方。src[0] = A(土台、shift[0] は常に 0)、src[1..] = B, C, ...
struct MixSpec {
  std::vector<int> src;    // 入力のエントリ番号
  std::vector<int> shift;  // time cell のずれ Δ(src と同じ長さ)
};

// ch 毎(= kCells 個ずつ)に、out = clamp(round(A + Σ (B[t−Δ] − ped_B)))。
// a と bs[k] は nCh*kCells 要素、shifts[k] は bs[k] の Δ。out は a と同じ長さに resize される。
inline void mixEvent(const std::vector<int16_t>& a, const std::vector<const std::vector<int16_t>*>& bs,
                     const std::vector<int>& shifts, std::vector<int16_t>& out) {
  const size_t n = a.size();
  const size_t nCh = n / kCells;
  out.resize(n);
  std::vector<double> acc(kCells);
  for (size_t c = 0; c < nCh; ++c) {
    const size_t o = c * kCells;
    for (int t = 0; t < kCells; ++t) acc[t] = a[o + t];
    for (size_t k = 0; k < bs.size(); ++k) {
      const int16_t* b = bs[k]->data() + o;
      double ped = 0;
      for (int t = kPedLo; t <= kPedHi; ++t) ped += b[t];
      ped /= (kPedHi - kPedLo + 1);
      for (int t = 0; t < kCells; ++t) {
        const int s = t - shifts[k];
        if (s < 0 || s >= kCells) continue;  // 範囲外は 0 を足す
        acc[t] += b[s] - ped;
      }
    }
    for (int t = 0; t < kCells; ++t) {
      double v = std::floor(acc[t] + 0.5);  // 四捨五入(.5 は上へ)
      if (v < 0) v = 0;
      if (v > kAdcMax) v = kAdcMax;
      out[o + t] = static_cast<int16_t>(v);
    }
  }
}

// ---- 組の指定 -----------------------------------------------------------------

// 乱数: A != B の組を n 個。Δ は [-maxShift, maxShift] の一様乱数(maxShift = 0 なら常に 0)。
inline std::vector<MixSpec> randomSpecs(int nEntries, int n, uint64_t seed, int maxShift) {
  if (nEntries < 2) throw std::runtime_error("random pairs need at least 2 input entries");
  std::mt19937_64 rng(seed);
  std::vector<MixSpec> v;
  for (int i = 0; i < n; ++i) {
    int a = static_cast<int>(rng() % nEntries);
    int b = static_cast<int>(rng() % (nEntries - 1));
    if (b >= a) ++b;  // a を除いた一様
    int d = 0;
    if (maxShift > 0) d = static_cast<int>(rng() % (2 * static_cast<uint64_t>(maxShift) + 1)) - maxShift;
    v.push_back({{a, b}, {0, d}});
  }
  return v;
}

// 1 行 = `srcA srcB [ΔB]` または `srcA srcB ΔB srcC ΔC ...`。'#' コメント可。
// トークン数 2 = Δ なし、奇数 >= 3 = A の後に (src, Δ) の組。偶数 >= 4 は曖昧なのでエラー。
inline std::vector<MixSpec> parseSpecs(std::istream& in, int nEntries) {
  std::vector<MixSpec> v;
  std::string line;
  int lineNo = 0;
  while (std::getline(in, line)) {
    ++lineNo;
    const auto h = line.find('#');
    if (h != std::string::npos) line.resize(h);
    std::istringstream ss(line);
    std::vector<long> tok;
    std::string w;
    bool bad = false;
    while (ss >> w) {
      char* end = nullptr;
      const long x = std::strtol(w.c_str(), &end, 10);
      if (*end != '\0') bad = true;
      tok.push_back(x);
    }
    if (tok.empty() && !bad) continue;
    const std::string where = "pairs file line " + std::to_string(lineNo) + ": ";
    if (bad) throw std::runtime_error(where + "non-integer token");
    if (tok.size() == 1 || (tok.size() >= 4 && tok.size() % 2 == 0))
      throw std::runtime_error(where + "expected `srcA srcB [dB]` or `srcA srcB dB srcC dC ...`");
    MixSpec m;
    m.src.push_back(static_cast<int>(tok[0]));
    m.shift.push_back(0);
    if (tok.size() == 2) {
      m.src.push_back(static_cast<int>(tok[1]));
      m.shift.push_back(0);
    } else {
      for (size_t i = 1; i + 1 < tok.size(); i += 2) {
        m.src.push_back(static_cast<int>(tok[i]));
        m.shift.push_back(static_cast<int>(tok[i + 1]));
      }
    }
    for (int s : m.src)
      if (s < 0 || s >= nEntries) throw std::runtime_error(where + "entry " + std::to_string(s) + " out of range");
    v.push_back(m);
  }
  return v;
}

// ---- ROOT I/O -----------------------------------------------------------------

// in の raw ツリーから specs の通りに混ぜ、out に raw / mix ツリーを書く。
// adc のリーフ記述(例 "adc[4][68][512]")と型は入力から読む(AsAd 数を焼き込まない)。
// 戻り値 = 書いたイベント数。
inline int mixFile(const std::string& inPath, const std::string& outPath, const std::vector<MixSpec>& specs) {
  TFile fin(inPath.c_str(), "READ");
  if (fin.IsZombie()) throw std::runtime_error("cannot open " + inPath);
  auto* tin = dynamic_cast<TTree*>(fin.Get("raw"));
  if (!tin) throw std::runtime_error(inPath + ": no TTree `raw`");
  TLeaf* leaf = tin->GetLeaf("adc");
  if (!leaf) throw std::runtime_error(inPath + ": no leaf `adc`");
  if (std::string(leaf->GetTypeName()) != "Short_t") throw std::runtime_error("leaf adc is not Short_t (/S)");
  const std::string leafDesc = leaf->GetTitle();  // "adc[4][68][512]"
  const size_t total = static_cast<size_t>(leaf->GetLenStatic());
  if (total == 0 || total % kCells != 0) throw std::runtime_error("adc leaf size is not a multiple of 512");
  const Long64_t nIn = tin->GetEntries();

  UInt_t eventIdIn = 0;
  ULong64_t eventTimeIn = 0;
  std::vector<int16_t> buf(total);
  tin->SetBranchAddress("eventId", &eventIdIn);
  tin->SetBranchAddress("eventTime", &eventTimeIn);
  tin->SetBranchAddress("adc", buf.data());

  TFile fout(outPath.c_str(), "RECREATE");
  if (fout.IsZombie()) throw std::runtime_error("cannot create " + outPath);
  TTree traw("raw", "mixed raw events");
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  std::vector<int16_t> outAdc(total);
  traw.Branch("eventId", &eventId, "eventId/i");
  traw.Branch("eventTime", &eventTime, "eventTime/l");
  traw.Branch("adc", outAdc.data(), (leafDesc + "/S").c_str());

  size_t maxSrc = 1;
  for (const auto& s : specs) maxSrc = std::max(maxSrc, s.src.size());
  TTree tmix("mix", "mix truth: sources of each output event");
  Int_t nSrc = 0;
  std::vector<Int_t> srcEntry(maxSrc), shift(maxSrc);
  std::vector<UInt_t> srcEventId(maxSrc);
  tmix.Branch("eventId", &eventId, "eventId/i");
  tmix.Branch("nSrc", &nSrc, "nSrc/I");
  tmix.Branch("srcEntry", srcEntry.data(), "srcEntry[nSrc]/I");
  tmix.Branch("srcEventId", srcEventId.data(), "srcEventId[nSrc]/i");
  tmix.Branch("shift", shift.data(), "shift[nSrc]/I");

  std::vector<std::vector<int16_t>> srcAdc(maxSrc, std::vector<int16_t>(total));
  for (size_t i = 0; i < specs.size(); ++i) {
    const MixSpec& m = specs[i];
    nSrc = static_cast<Int_t>(m.src.size());
    std::vector<const std::vector<int16_t>*> bs;
    std::vector<int> bShift;
    for (size_t k = 0; k < m.src.size(); ++k) {
      if (m.src[k] < 0 || m.src[k] >= nIn) throw std::runtime_error("source entry out of range");
      tin->GetEntry(m.src[k]);
      srcAdc[k] = buf;
      srcEntry[k] = m.src[k];
      srcEventId[k] = eventIdIn;
      shift[k] = m.shift[k];
      if (k == 0) eventTime = eventTimeIn;
      if (k > 0) {
        bs.push_back(&srcAdc[k]);
        bShift.push_back(m.shift[k]);
      }
    }
    mixEvent(srcAdc[0], bs, bShift, outAdc);
    eventId = static_cast<UInt_t>(i);
    traw.Fill();
    tmix.Fill();
  }
  fout.cd();
  traw.Write();
  tmix.Write();
  fout.Close();
  return static_cast<int>(specs.size());
}

}  // namespace evmix

#endif
