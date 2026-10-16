// =============================================================================
// test_zs_restore.C — zs_restore.C の合成テスト(TODO/154 §2)。
//
//   root -l -b -q tools/scan/test_zs_restore.C        → exit code = 失敗数(0 = green)
//
// 合成 raw を一時ディレクトリに書き、zs_restore の出力が手計算どおりになることを見る。
// 基線 b(a,c) = 200 + 3c + 50a(a = AGET ブロック番号)。全セル 奇数 cell −2 / 偶数 cell +2
// (512 cells で平均ちょうど 0 → 表の mean = b)。ZS 版 = b + 10 未満のセルを 0 にしたもの
// (FPN ch は全セル b ± 2 < b + 10 → 全 0)。パルスのセル(cell 100..130 など)は b + H ± 2 で残る。
//   例: (a=0, c=3) b = 209、H = 400 → 偶数 cell 611、奇数 cell 607。抑制セルは復元後 209。
// =============================================================================
#include "zs_restore.C"

#include <TSystem.h>

#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

namespace test_zr {

int g_failures = 0;

void check(bool ok, const std::string &what)
{
  std::printf("  %s %s\n", ok ? "ok  " : "FAIL", what.c_str());
  if (!ok) ++g_failures;
}

void checkNear(double got, double want, const std::string &what, double tol = 1e-9)
{
  char buf[256];
  std::snprintf(buf, sizeof(buf), "%s: got %.10g want %.10g", what.c_str(), got, want);
  check(std::isfinite(got) && std::fabs(got - want) <= tol, buf);
}

int baseline(int block, int c) { return 200 + 3 * c + 50 * block; }

struct Pulse {
  int block, ch, lo, hi, height;
};
// イベント k は 1 + k % 3 本(先頭から)
const Pulse kPulses[3] = {{0, 3, 100, 130, 400}, {1, 7, 150, 180, 500}, {2, 20, 200, 230, 450}};
// 高さは run_metrics のヒット定義(ピーク ±5 cells の電荷 q ≥ 2000)を満たすように:
// ピークは平らな頂の最初の cell(偶数、b + H + 2)で、窓 ±5 に頂は 6 cells → q ≈ 6 × (H + 2) ≥ 2412。

// 1 イベント分(nBlocks × 68 × 512)を buf に作る。zs なら b + 10 未満を 0 に。
void fillEvent(std::vector<Short_t> &buf, int nBlocks, int k, bool zs)
{
  for (int b = 0; b < nBlocks; ++b)
    for (int c = 0; c < 68; ++c)
      for (int t = 0; t < 512; ++t)
        buf[(static_cast<size_t>(b) * 68 + c) * 512 + t] =
            static_cast<Short_t>(baseline(b, c) + ((t % 2 == 1) ? -2 : +2));
  const int n = 1 + k % 3;
  for (int p = 0; p < n; ++p) {
    const Pulse &P = kPulses[p];
    if (P.block >= nBlocks) continue;
    for (int t = P.lo; t <= P.hi; ++t)
      buf[(static_cast<size_t>(P.block) * 68 + P.ch) * 512 + t] += static_cast<Short_t>(P.height);
  }
  if (zs)
    for (int b = 0; b < nBlocks; ++b)
      for (int c = 0; c < 68; ++c)
        for (int t = 0; t < 512; ++t) {
          Short_t &v = buf[(static_cast<size_t>(b) * 68 + c) * 512 + t];
          if (v < baseline(b, c) + 10) v = 0;
        }
}

bool writeRaw(const std::string &path, int nBlocks, int nEvents, bool zs)
{
  TFile f(path.c_str(), "RECREATE");
  if (f.IsZombie()) return false;
  TTree tree("raw", "synthetic raw (test_zs_restore.C)");
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  std::vector<Short_t> buf(static_cast<size_t>(nBlocks) * 68 * 512);
  tree.Branch("eventId", &eventId);
  tree.Branch("eventTime", &eventTime);
  tree.Branch("adc", buf.data(), ("adc[" + std::to_string(nBlocks) + "][68][512]/S").c_str());
  for (int k = 0; k < nEvents; ++k) {
    fillEvent(buf, nBlocks, k, zs);
    eventId = 100U + static_cast<UInt_t>(k);
    eventTime = 1000ULL + static_cast<ULong64_t>(k) * 5000000ULL;
    tree.Fill();
  }
  tree.Write();
  return true;
}

// calib pedestal の JSON。block = asad*4 + aget。skipBlock/skipCh が非負ならその ch を省く。
bool writePed(const std::string &path, int nBlocks, int skipBlock, int skipCh)
{
  FILE *fp = std::fopen(path.c_str(), "w");
  if (!fp) return false;
  std::fputs("{\"meta\": {\"note\": \"synthetic\"}, \"channels\": [\n", fp);
  bool first = true;
  for (int b = 0; b < nBlocks; ++b)
    for (int c = 0; c < 68; ++c) {
      if (b == skipBlock && c == skipCh) continue;
      std::fprintf(fp,
                   "%s{\"asad\": %d, \"aget\": %d, \"ch\": %d, \"mean\": %d.0, \"rms\": 1.5, "
                   "\"runs\": 1, \"is_fpn\": false}",
                   first ? "" : ",\n", b / 4, b % 4, c, baseline(b, c));
      first = false;
    }
  std::fputs("\n]}\n", fp);
  return std::fclose(fp) == 0;
}

// 読み戻し(入力と出力を同じ形で比べる)
struct Loaded {
  bool ok = false;
  Long64_t entries = 0;
  int leafLen = 0;
  std::vector<UInt_t> id;
  std::vector<ULong64_t> time;
  std::vector<Short_t> adc;  // entries × leafLen
};

Loaded load(const std::string &path)
{
  Loaded L;
  TFile f(path.c_str(), "READ");
  if (f.IsZombie()) return L;
  TTree *t = f.Get<TTree>("raw");
  if (!t || !t->GetLeaf("adc") || !t->GetLeaf("eventId") || !t->GetLeaf("eventTime")) return L;
  L.leafLen = t->GetLeaf("adc")->GetLen();
  L.entries = t->GetEntries();
  std::vector<Short_t> buf(static_cast<size_t>(L.leafLen));
  UInt_t id = 0;
  ULong64_t tm = 0;
  t->SetBranchAddress("adc", buf.data());
  t->SetBranchAddress("eventId", &id);
  t->SetBranchAddress("eventTime", &tm);
  for (Long64_t i = 0; i < L.entries; ++i) {
    t->GetEntry(i);
    L.id.push_back(id);
    L.time.push_back(tm);
    L.adc.insert(L.adc.end(), buf.begin(), buf.end());
  }
  L.ok = true;
  return L;
}

Short_t cell(const Loaded &L, int ev, int block, int c, int t)
{
  return L.adc[static_cast<size_t>(ev) * L.leafLen + (static_cast<size_t>(block) * 68 + c) * 512 + t];
}

}  // namespace test_zr

int test_zs_restore()
{
  using namespace test_zr;
  const std::string dir = std::string(gSystem->TempDirectory()) + "/test_zs_restore_" +
                          std::to_string(static_cast<long>(gSystem->GetPid()));
  gSystem->mkdir(dir.c_str(), true);
  const std::string zsRaw = dir + "/run0001_raw.root";
  const std::string plain = dir + "/run0002_raw.root";
  const std::string zs8 = dir + "/run0003_raw.root";
  const std::string ped = dir + "/ped.json";
  const std::string ped8 = dir + "/ped8.json";
  const std::string pedMiss = dir + "/ped_miss.json";
  const std::string outA = dir + "/outA_raw.root";
  const std::string outB = dir + "/outB_raw.root";
  const std::string outC = dir + "/outC_raw.root";
  const std::string outD = dir + "/outD_raw.root";
  const std::string outE = dir + "/outE_raw.root";

  check(writeRaw(zsRaw, 4, 3, true), "write ZS raw (3 events, 4 AGETs)");
  check(writeRaw(plain, 4, 3, false), "write non-ZS raw (3 events, 4 AGETs)");
  check(writeRaw(zs8, 8, 2, true), "write ZS raw (2 events, 8 AGETs = 2 AsAd)");
  check(writePed(ped, 4, -1, -1), "write pedestal table");
  check(writePed(ped8, 8, -1, -1), "write pedestal table (8 blocks)");
  check(writePed(pedMiss, 4, 1, 5), "write pedestal table without (block 1, ch 5)");

  // ---- (a) ZS 合成 → 各セルが元の値か round(mean)、パルスのセルは同一 ----------------------------
  // イベント k のパルス数 = 1 + k % 3 → ev0 = P0 のみ、ev1 = P0,P1、ev2 = P0,P1,P2。
  // 全セル 3 ev × 4 blk × 68 ch × 512 = 417792。残るのはパルスセルだけ:
  //   P0 (3 ev × 31 cells)、P1 (2 ev × 31)、P2 (1 ev × 31) = 186 cells → 埋めたセル = 417792 − 186。
  std::printf("[(a) ZS synthetic: suppressed cells -> round(mean), pulse cells untouched]\n");
  zsrestore::Stats st;
  std::string err;
  check(zsrestore::restore(zsRaw, ped, outA, 0, st, err), "restore ZS raw ok");
  const Loaded A = load(outA);
  const Loaded Z = load(zsRaw);
  check(A.ok && A.entries == 3 && A.leafLen == 4 * 68 * 512, "output tree: 3 entries, adc[4][68][512]");
  check(A.id == Z.id && A.time == Z.time, "eventId / eventTime copied as is");
  checkNear(static_cast<double>(st.events), 3, "events");
  checkNear(static_cast<double>(st.filled), 3 * 4 * 68 * 512 - 186, "filled cells");
  checkNear(static_cast<double>(st.unfilled), 0, "unfilled cells (all channels in the table)");
  // (a=0,c=3) b = 209:cell 50 は抑制 → 209、cell 100(偶数、パルス)= 209 + 400 + 2 = 611、cell 101 = 607
  checkNear(cell(A, 0, 0, 3, 50), 209, "suppressed cell (0,3,t50) -> 209");
  checkNear(cell(A, 0, 0, 3, 100), 611, "pulse cell even (0,3,t100) = 611");
  checkNear(cell(A, 0, 0, 3, 101), 607, "pulse cell odd (0,3,t101) = 607");
  // FPN ch(0,11) は全 0 → 表にあるので 200 + 33 = 233。ブロック 3 の ch 67:200 + 201 + 150 = 551
  checkNear(cell(A, 0, 0, 11, 7), 233, "FPN cell (0,11,t7) -> 233");
  checkNear(cell(A, 1, 3, 67, 511), 551, "block 3 ch 67 cell 511 -> 551");
  // ev0 には P1 が無い((1,7) は ev1 から): b = 200 + 21 + 50 = 271 に復元。ev1 では cell 150 = 271 + 500 + 2
  checkNear(cell(A, 0, 1, 7, 150), 271, "P1 absent in ev0: (1,7,t150) -> 271");
  checkNear(cell(A, 1, 1, 7, 150), 773, "P1 present in ev1: (1,7,t150) = 773");
  bool every = true;  // 全セル: ZS 入力が 0 なら b、非 0 なら同一
  for (int e = 0; e < 3; ++e)
    for (int b = 0; b < 4; ++b)
      for (int c = 0; c < 68; ++c)
        for (int t = 0; t < 512; ++t) {
          const Short_t z = cell(Z, e, b, c, t), o = cell(A, e, b, c, t);
          if (o != (z == 0 ? baseline(b, c) : z)) every = false;
        }
  check(every, "every cell: output == (input == 0 ? round(mean) : input)");

  // ---- (b) 非 ZS 入力 → 同一ツリー ---------------------------------------------------------------
  std::printf("[(b) non-ZS input: identical tree]\n");
  zsrestore::Stats sb;
  check(zsrestore::restore(plain, ped, outB, 0, sb, err), "restore non-ZS raw ok");
  const Loaded P = load(plain), B = load(outB);
  check(B.ok && B.entries == P.entries && B.leafLen == P.leafLen && B.id == P.id && B.time == P.time &&
            B.adc == P.adc,
        "non-ZS: entries, eventId, eventTime and every adc cell identical");
  checkNear(static_cast<double>(sb.filled), 0, "non-ZS: filled == 0");

  // ---- (c) 健全性: 復元 + 表なし == ZS + 表(run_metrics のヒット定義)---------------------------
  // ev0..2 のヒット ch 数 = 1,2,3 → median 2。電荷は両者とも同じ ped(= b)で同じセルを数える。
  std::printf("[(c) sanity: metrics(restored, no table) == metrics(ZS raw, table)]\n");
  runmetrics::RunMetrics mr, mz;
  check(runmetrics::computeRunMetrics(outA, mr, err), "metrics of restored (no table) ok");
  check(runmetrics::computeRunMetrics(zsRaw, mz, err, ped), "metrics of ZS raw (table) ok");
  check(mz.charge_median > 1000, "ZS + table: charge_median is a real signal (not 0)");
  checkNear(mr.charge_median, mz.charge_median, "restored == ZS+table: charge_median (1 %)",
            mz.charge_median * 0.01);
  checkNear(mr.hit_channels_median, mz.hit_channels_median, "restored == ZS+table: hit_channels_median");
  checkNear(mz.hit_channels_median, 2.0, "hit_channels_median == 2 (1,2,3 hit channels)");
  check(!mr.zs_suspected, "restored raw is no longer flagged zs_suspected");

  // ---- (d) 表に無い ch → 不変 + 計数 -----------------------------------------------------------------
  // (block 1, ch 5) は全イベントでパルス無し = 全 512 cells が 0 のまま → unfilled = 3 × 512 = 1536。
  // 表にある他の ch は通常どおり: filled = 417792 − 186 − 1536。
  std::printf("[(d) channel absent from the table: left as 0 and counted]\n");
  zsrestore::Stats sd;
  check(zsrestore::restore(zsRaw, pedMiss, outD, 0, sd, err), "restore with incomplete table ok");
  const Loaded D = load(outD);
  checkNear(static_cast<double>(sd.unfilled), 3 * 512, "unfilled == 1536");
  checkNear(static_cast<double>(sd.filled), 3 * 4 * 68 * 512 - 186 - 1536,
            "filled excludes the missing channel");
  checkNear(cell(D, 0, 1, 5, 300), 0, "missing channel (1,5,t300) stays 0");
  checkNear(cell(D, 0, 1, 6, 300), 200 + 18 + 50, "neighbour channel (1,6,t300) filled -> 268");

  // ---- (e) 同名拒否 -----------------------------------------------------------------------------
  std::printf("[(e) rawOut == rawIn is refused]\n");
  zsrestore::Stats se;
  check(!zsrestore::restore(zsRaw, ped, zsRaw, 0, se, err) && !err.empty(), "restore: same path refused");
  check(zs_restore(zsRaw.c_str(), ped.c_str(), zsRaw.c_str()) == 1, "zs_restore(): same path returns 1");
  const Loaded Z2 = load(zsRaw);
  check(Z2.ok && Z2.adc == Z.adc, "input file untouched after the refusal");
  check(zs_restore((dir + "/nope_raw.root").c_str(), ped.c_str(), outE.c_str()) == 1,
        "zs_restore(): missing input returns 1");
  check(zs_restore(zsRaw.c_str(), (dir + "/nope.json").c_str(), outE.c_str()) == 1,
        "zs_restore(): missing pedestal table returns 1");

  // ---- maxEvents と N(leaf の長さ)--------------------------------------------------------------
  std::printf("[maxEvents and N from the leaf length]\n");
  check(zs_restore(zsRaw.c_str(), ped.c_str(), outC.c_str(), 2) == 0,
        "zs_restore(maxEvents = 2) returns 0");
  const Loaded C = load(outC);
  check(C.ok && C.entries == 2 && C.id.size() == 2 && C.id[1] == 101U,
        "maxEvents = 2: two entries (eventId 100, 101)");
  // 8 AGET(2 AsAd):block 5 = asad 1, aget 1。(5, 4) b = 200 + 12 + 250 = 462
  zsrestore::Stats s8;
  check(zsrestore::restore(zs8, ped8, outE, 0, s8, err), "restore 8-AGET ZS raw ok");
  const Loaded E = load(outE);
  check(E.ok && E.leafLen == 8 * 68 * 512, "output leaf is adc[8][68][512]");
  checkNear(cell(E, 0, 5, 4, 20), 462, "8 AGETs: (block 5, ch 4, t20) -> 462");
  checkNear(static_cast<double>(s8.unfilled), 0, "8 AGETs: unfilled == 0");

  for (const std::string &q : {zsRaw, plain, zs8, ped, ped8, pedMiss, outA, outB, outC, outD, outE})
    gSystem->Unlink(q.c_str());
  gSystem->Unlink(dir.c_str());

  std::printf("%s: %d failure(s)\n", g_failures == 0 ? "GREEN" : "RED", g_failures);
  return g_failures;
}
