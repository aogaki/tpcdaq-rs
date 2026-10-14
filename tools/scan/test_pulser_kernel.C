// =============================================================================
// test_pulser_kernel.C — pulser_kernel.C の合成テスト(TODO/138 §2)。
//
//   root -l -b -q tools/scan/test_pulser_kernel.C     → exit code = 失敗数(0 = green)
//
// 合成 raw(mini = adc[4][68][512])を一時ディレクトリに書き、pulser_kernel.C の関数で求めた
// 応答核の指標が手計算どおりに戻ることを見る。期待値の出典は各 check の上のコメント(手計算)。
// 分位点は線形補間 h = (n−1)·q(run_metrics.C の quantile と同じ定義)。
//
// 形は区分線形(発注書の例は CR-RC^2 だが、補間した交点が手計算でちょうど決まる形にした):
//   s(k)、k = ピークからのセル数(ピーク = 1)
//     k < −8         0
//     −8 ≤ k ≤ 0     (k + 8) / 8                 立ち上がり(10 % は k = −7.2、50 % は −4、90 % は −0.8)
//     0 < k ≤ 25     (25 − k) / 25               立ち下がり(50 % は k = 12.5)
//     25 < k ≤ 30    −(k − 25) / 50              アンダーシュート: k = 30 で −0.1(最小)
//     30 < k ≤ 50    −(50 − k) / 200             k = 50 で 0 に戻る
//     100 ≤ k ≤ 149  +0.02                       尾の「揺れ」(正)
//     150 ≤ k ≤ 199  −0.01                       尾の「揺れ」(負 — 非対称にして RMS の定義を縛る)
//     それ以外        0
//   振幅 A が 200 の倍数なら A·s(k) は全セル整数(A/8、A/25、A/50、A/200、A/100 が整数)。
//   → 立ち上がり 10–90 % = −0.8 − (−7.2) = 6.4 cells、FWHM = 12.5 − (−4) = 16.5 cells、
//     アンダーシュートの最小 = −0.1 @ +30 cell、
//     t = +50…+300(251 cells)の RMS = √((50·0.02² + 50·0.01²) / 251) = √(0.025 / 251)、
//     ピーク間 = 0.02 − (−0.01) = 0.03。
//
// 全 ch(FPN 含む)の基線 = b(a,c) + cm(t):
//   b(a,c) = 200 + 3c + 50a、cm(t) = t / 13(整数除算。cell 0 で 0、cell 511 で 39 — run0076 の
//   common mode の模型、test_run_metrics.C と同じ)。信号 ch だけ ped 窓 cell 1..20 に ±2 の交互
//   (奇数 cell −2、偶数 +2 → 平均 0)。FPN を引けば v(t) = A·s(t − peak) ちょうど(ped = b − B(a))。
//   引かなければピーク付近(cell 110..457)に cm = 8..35 ADC が残り、形の指標がずれる。
// =============================================================================
#include "pulser_kernel.C"

#include <TSystem.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <string>

namespace test_pk {

int g_failures = 0;

void check(bool ok, const std::string &what)
{
  if (ok) {
    std::printf("  ok   %s\n", what.c_str());
  } else {
    std::printf("  FAIL %s\n", what.c_str());
    ++g_failures;
  }
}

void checkNear(double got, double want, const std::string &what, double tol = 1e-9)
{
  const bool ok = std::isfinite(got) && std::fabs(got - want) <= tol;
  char buf[256];
  std::snprintf(buf, sizeof(buf), "%s: got %.10g want %.10g", what.c_str(), got, want);
  check(ok, buf);
}

void checkNull(double got, const std::string &what)
{
  char buf[256];
  std::snprintf(buf, sizeof(buf), "%s: got %.10g want null(NaN)", what.c_str(), got);
  check(std::isnan(got), buf);
}

void checkCount(long got, long want, const std::string &what)
{
  check(got == want, what + ": got " + std::to_string(got) + " want " + std::to_string(want));
}

bool contains(const std::string &hay, const std::string &needle)
{
  return hay.find(needle) != std::string::npos;
}

// ---- 合成 raw ---------------------------------------------------------------------------------
Short_t g_adc[4][68][512];

int baseline(int a, int c) { return 200 + 3 * c + 50 * a; }
int commonMode(int t) { return t / 13; }

// A·s(k)(冒頭の表)。A は 200 の倍数で全セル整数。
int shapeAdc(int A, int k)
{
  if (k < -8) return 0;
  if (k <= 0) return A * (k + 8) / 8;
  if (k <= 25) return A * (25 - k) / 25;
  if (k <= 30) return -A * (k - 25) / 50;
  if (k <= 50) return -A * (50 - k) / 200;
  if (k >= 100 && k <= 149) return A / 50;
  if (k >= 150 && k <= 199) return -A / 100;
  return 0;
}

void fillBaseline()
{
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c)
      for (int t = 0; t < 512; ++t) {
        int v = baseline(a, c) + commonMode(t);
        if (!runmetrics::isFpn(c) && t >= 1 && t <= 20) v += (t % 2 == 1) ? -2 : +2;
        g_adc[a][c][t] = static_cast<Short_t>(v);
      }
}

// 今の値に A·s(t − peak) を足し、12 bit のフルスケール 4095 で頭打ちにする(飽和の模型)。
void addPulse(int a, int c, int peak, int A)
{
  for (int t = 0; t < 512; ++t) {
    const int v = g_adc[a][c][t] + shapeAdc(A, t - peak);
    g_adc[a][c][t] = static_cast<Short_t>(std::min(v, 4095));
  }
}

// cell lo..hi に +h の矩形。
void addBox(int a, int c, int lo, int hi, int h)
{
  for (int t = lo; t <= hi; ++t) g_adc[a][c][t] = static_cast<Short_t>(g_adc[a][c][t] + h);
}

// イベント e(0..19)。P = 150 + e mod 5、偶奇 = e mod 2(20 イベントで (P, 偶奇) の組が各 2 回)。
//   AGET 0: ch 3 @ P、ch 30 @ P+1、A = 800
//   AGET 1: ch 5 @ P、ch 40 @ P+2(偶数)/ P(奇数)、ch 60 @ P+1、A = 1200
//           ch 61 は e mod 4 == 0(5 イベント)だけ A = 4000 → b(1,61) + cm + 4000 = 433 + 11 + 4000
//           > 4095 で頭打ち = 飽和(除いて数える)。他のイベントは基線だけ(閾値以下)
//   AGET 2: ch 12 @ P+3、A = 2400
//           ch 13 @ P、A = 4000 → 339 + 11 + 4000 > 4095 で毎イベント飽和
//           ch 14 は cell P..P+5 に +100 の矩形 → v(ピーク) = 100 ちょうど(「100 を超え」ない — 除く、
//           飽和には数えない)
//   AGET 3: 信号 ch にパルス無し。FPN ch 45 にだけ A = 1000 のパルス(おとり)→ FPN の平均に 1/4 入って
//           信号 ch は v = −250·s(負、最大 +25 < 100)。FPN を信号 ch として使うと ch 45 の
//           v = 1000·s − 250·s = 750·s がピーク 750 で通ってしまう
void fillEvent(int e)
{
  fillBaseline();
  const int P = 150 + e % 5;
  const bool even = (e % 2 == 0);
  addPulse(0, 3, P, 800);
  addPulse(0, 30, P + 1, 800);
  addPulse(1, 5, P, 1200);
  addPulse(1, 40, P + (even ? 2 : 0), 1200);
  addPulse(1, 60, P + 1, 1200);
  if (e % 4 == 0) addPulse(1, 61, P, 4000);
  addPulse(2, 12, P + 3, 2400);
  addPulse(2, 13, P, 4000);
  addBox(2, 14, P, P + 5, 100);
  addPulse(3, 45, P, 1000);
}

// 窓の端が読み出し窓(cell 0..511)からはみ出す run: 2 イベント、AGET 0 の ch 3 @ 150 と
// ch 30 @ 450(どちらも A = 800)。ch 30 は k ≤ 511 − 450 = 61 までしか無い。
//   → n(k) = 4(k ≤ 61)、2(k ≥ 62)。平均は在るものだけ = 形はそのまま s(k)
void fillLate(int)
{
  fillBaseline();
  addPulse(0, 3, 150, 800);
  addPulse(0, 30, 450, 800);
}

bool writeRaw(const std::string &path, int nEvents, void (*fill)(int))
{
  TFile f(path.c_str(), "RECREATE");
  if (f.IsZombie()) return false;
  TTree tree("raw", "synthetic raw (test_pulser_kernel.C)");
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  tree.Branch("eventId", &eventId);
  tree.Branch("eventTime", &eventTime);
  tree.Branch("adc", g_adc, "adc[4][68][512]/S");
  for (int k = 0; k < nEvents; ++k) {
    fill(k);
    eventId = static_cast<UInt_t>(k);
    eventTime = 1000ULL + static_cast<ULong64_t>(k) * 5000000ULL;  // 50 ms 間隔(10 ns 刻み)
    tree.Fill();
  }
  tree.Write();
  return true;
}

std::string readFile(const std::string &path)
{
  std::string s;
  FILE *fp = std::fopen(path.c_str(), "r");
  if (!fp) return s;
  char buf[4096];
  size_t n;
  while ((n = std::fread(buf, 1, sizeof(buf), fp)) > 0) s.append(buf, n);
  std::fclose(fp);
  return s;
}

// 窓 [−40, +300] の offset k の配列 index(pulser_kernel.C の kWinLo = −40)。
size_t at(int k) { return static_cast<size_t>(k + 40); }

// 形の指標(全 AGET 共通 — 形が同じで振幅だけ違う)。A = その group の平均振幅(生の平均の倍率)。
void checkShape(const pulserkernel::Group &g, double A, const std::string &tag)
{
  // 冒頭の表: 立ち上がり 10–90 % = 6.4、FWHM = 16.5
  checkNear(g.rise_10_90_cells, 6.4, tag + " rise_10_90_cells");
  checkNear(g.fwhm_cells, 16.5, tag + " fwhm_cells");
  // アンダーシュート: 最小 −0.1 @ +30、生の平均では −0.1·A
  checkNear(g.undershoot_min_ratio, -0.1, tag + " undershoot_min_ratio");
  checkNear(g.undershoot_min_adc, -0.1 * A, tag + " undershoot_min_adc", 1e-6);
  checkNear(g.undershoot_cell, 30, tag + " undershoot_cell");
  // 尾 t = +50…+300: RMS = √(0.025/251)(0 を基準 — 区間平均を引く定義なら違う値)、ピーク間 0.03
  const double rms = std::sqrt(0.025 / 251.0);
  checkNear(g.tail_rms_ratio, rms, tag + " tail_rms_ratio");
  checkNear(g.tail_p2p_ratio, 0.03, tag + " tail_p2p_ratio");
  checkNear(g.tail_rms_adc, A * rms, tag + " tail_rms_adc", 1e-6);
  checkNear(g.tail_p2p_adc, 0.03 * A, tag + " tail_p2p_adc", 1e-6);
  // 正規化した平均の値そのもの(s(k)): −40 → 0、−4 → 0.5、0 → 1、+30 → −0.1、+120 → 0.02、
  // +160 → −0.01、+300 → 0(FPN を引かないと cm(450) − ped ≈ 34 ADC が残る)
  if (g.mean_norm.size() == pulserkernel::kWinLen && g.mean_raw.size() == pulserkernel::kWinLen) {
    checkNear(g.mean_norm[at(-40)], 0.0, tag + " mean_norm(-40)");
    checkNear(g.mean_norm[at(-4)], 0.5, tag + " mean_norm(-4)");
    checkNear(g.mean_norm[at(0)], 1.0, tag + " mean_norm(0)");
    checkNear(g.mean_norm[at(30)], -0.1, tag + " mean_norm(+30)");
    checkNear(g.mean_norm[at(120)], 0.02, tag + " mean_norm(+120)");
    checkNear(g.mean_norm[at(160)], -0.01, tag + " mean_norm(+160)");
    checkNear(g.mean_norm[at(300)], 0.0, tag + " mean_norm(+300)");
    checkNear(g.mean_raw[at(0)], A, tag + " mean_raw(0)", 1e-6);
    checkNear(g.mean_raw[at(300)], 0.0, tag + " mean_raw(+300) (common mode subtracted)", 1e-6);
  } else {
    check(false, tag + " mean_norm / mean_raw have kWinLen entries");
  }
}

}  // namespace test_pk

int test_pulser_kernel()
{
  using namespace test_pk;
  using pulserkernel::Group;
  const std::string dir = std::string(gSystem->TempDirectory()) + "/test_pulser_kernel_" +
                          std::to_string(gSystem->GetPid());
  gSystem->mkdir(dir.c_str(), kTRUE);
  const std::string raw = dir + "/run0090_raw.root";
  const std::string lateRaw = dir + "/run0091_raw.root";
  const std::string json = dir + "/run0090_pulser.json";
  const std::string png = dir + "/run0090_pulser.png";
  check(writeRaw(raw, 20, fillEvent), "write synthetic pulser raw (20 events)");
  check(writeRaw(lateRaw, 2, fillLate), "write late-peak raw (2 events)");

  // ---- 20 イベント、全部 ---------------------------------------------------------------------
  std::printf("[synthetic pulser raw, common-mode slope on all channels]\n");
  pulserkernel::Result r;
  std::string err;
  check(pulserkernel::computePulserKernel(raw, 0, r, err), "computePulserKernel ok (" + err + ")");
  checkCount(r.events, 20, "events");
  checkCount(r.run, 90, "run (from run0090_raw.root)");
  checkCount(r.agets_without_fpn, 0, "agets_without_fpn");
  checkCount(static_cast<long>(r.agets.size()), 4, "agets.size() (adc[4][68][512] → 4 AGET)");

  if (r.agets.size() == 4) {
    const Group &g0 = r.agets[0], &g1 = r.agets[1], &g2 = r.agets[2], &g3 = r.agets[3];

    // AGET 0: 2 ch × 20 = 40 波形、飽和なし、振幅 800
    checkCount(g0.waveforms, 40, "aget0 waveforms");
    checkCount(g0.channels, 2, "aget0 channels");
    checkCount(g0.saturated_waveforms, 0, "aget0 saturated_waveforms");
    checkCount(g0.saturated_channels, 0, "aget0 saturated_channels");
    checkNear(g0.amplitude_median_adc, 800, "aget0 amplitude_median_adc");
    checkShape(g0, 800, "aget0");
    // ピーク cell: P(150..154 各 4 回)と P+1(151..155 各 4 回)の 40 個
    //   → 150:4、151:8、152:8、153:8、154:8、155:4(昇順 index 150 = 0–3、151 = 4–11、152 = 12–19、
    //     153 = 20–27、154 = 28–35、155 = 36–39)
    //   median h = 19.5 → (152 + 153)/2 = 152.5、p10 h = 3.9 → 150 + 0.9 = 150.9、
    //   p90 h = 35.1 → 154 + 0.1 = 154.1
    checkNear(g0.peak_cell.median, 152.5, "aget0 peak_cell.median");
    checkNear(g0.peak_cell.p10, 150.9, "aget0 peak_cell.p10");
    checkNear(g0.peak_cell.p90, 154.1, "aget0 peak_cell.p90");
    // イベント内の ch 間ばらつき(max − min)= (P+1) − P = 1 が毎イベント
    checkNear(g0.peak_cell_range_in_event.median, 1, "aget0 peak_cell_range_in_event.median");
    checkNear(g0.peak_cell_range_in_event.p90, 1, "aget0 peak_cell_range_in_event.p90");

    // AGET 1: 3 ch × 20 = 60 波形、ch 61 の 5 イベントが飽和、振幅 1200
    checkCount(g1.waveforms, 60, "aget1 waveforms");
    checkCount(g1.channels, 3, "aget1 channels");
    checkCount(g1.saturated_waveforms, 5, "aget1 saturated_waveforms (ch 61, e mod 4 == 0)");
    checkCount(g1.saturated_channels, 1, "aget1 saturated_channels");
    checkNear(g1.amplitude_median_adc, 1200, "aget1 amplitude_median_adc");
    checkShape(g1, 1200, "aget1");
    // ピーク cell: 各 P につき偶数 2 イベント P+{0,2,1}、奇数 2 イベント P+{0,0,1}
    //   → P+0: 6、P+1: 4、P+2: 2(P ごと 12 個、計 60)
    //   値ごと: 150:6、151:4+6 = 10、152:2+4+6 = 12、153:12、154:12、155:2+4 = 6、156:2
    //   (昇順 index 150 = 0–5、151 = 6–15、152 = 16–27、153 = 28–39、154 = 40–51、155 = 52–57、
    //    156 = 58–59)
    //   median h = 29.5 → 153、p10 h = 5.9 → 150 + 0.9 = 150.9、p90 h = 53.1 → 155
    checkNear(g1.peak_cell.median, 153, "aget1 peak_cell.median");
    checkNear(g1.peak_cell.p10, 150.9, "aget1 peak_cell.p10");
    checkNear(g1.peak_cell.p90, 155, "aget1 peak_cell.p90");
    // イベント内ばらつき: 偶数 = (P+2) − P = 2、奇数 = (P+1) − P = 1 → 10 個ずつ
    //   昇順 [1×10, 2×10]: median h = 9.5 → 1.5、p10 h = 1.9 → 1、p90 h = 17.1 → 2
    checkNear(g1.peak_cell_range_in_event.median, 1.5, "aget1 peak_cell_range_in_event.median");
    checkNear(g1.peak_cell_range_in_event.p10, 1, "aget1 peak_cell_range_in_event.p10");
    checkNear(g1.peak_cell_range_in_event.p90, 2, "aget1 peak_cell_range_in_event.p90");

    // AGET 2: ch 12 だけ(ch 13 は毎イベント飽和、ch 14 は v = 100 ちょうどで除外)
    checkCount(g2.waveforms, 20, "aget2 waveforms (ch 14 at exactly 100 ADC excluded)");
    checkCount(g2.channels, 1, "aget2 channels");
    checkCount(g2.saturated_waveforms, 20, "aget2 saturated_waveforms (ch 13 every event)");
    checkCount(g2.saturated_channels, 1, "aget2 saturated_channels");
    checkNear(g2.amplitude_median_adc, 2400, "aget2 amplitude_median_adc");
    checkShape(g2, 2400, "aget2");
    // ピーク cell = P+3 = 153..157 各 4 個: median h = 9.5 → 155、p10 h = 1.9 → 153、p90 h = 17.1 → 157
    checkNear(g2.peak_cell.median, 155, "aget2 peak_cell.median");
    checkNear(g2.peak_cell.p10, 153, "aget2 peak_cell.p10");
    checkNear(g2.peak_cell.p90, 157, "aget2 peak_cell.p90");
    // 1 ch だけ → ばらつき 0
    checkNear(g2.peak_cell_range_in_event.median, 0, "aget2 peak_cell_range_in_event.median");

    // AGET 3: 使える ch なし(FPN 45 のおとりは信号として使わない)→ 指標は null
    checkCount(g3.waveforms, 0, "aget3 waveforms (FPN decoy not used as signal)");
    checkCount(g3.channels, 0, "aget3 channels");
    checkCount(g3.saturated_waveforms, 0, "aget3 saturated_waveforms");
    checkNull(g3.amplitude_median_adc, "aget3 amplitude_median_adc");
    checkNull(g3.rise_10_90_cells, "aget3 rise_10_90_cells");
    checkNull(g3.fwhm_cells, "aget3 fwhm_cells");
    checkNull(g3.undershoot_min_ratio, "aget3 undershoot_min_ratio");
    checkNull(g3.tail_rms_ratio, "aget3 tail_rms_ratio");
    checkNull(g3.peak_cell.median, "aget3 peak_cell.median");
    if (g3.mean_norm.size() == pulserkernel::kWinLen)
      checkNull(g3.mean_norm[at(0)], "aget3 mean_norm(0)");
  }

  // 全体: 6 ch × 20 = 120 波形。飽和 5 + 20 = 25 波形、2 ch
  const Group &all = r.all;
  checkCount(all.waveforms, 120, "all waveforms");
  checkCount(all.channels, 6, "all channels");
  checkCount(all.saturated_waveforms, 25, "all saturated_waveforms");
  checkCount(all.saturated_channels, 2, "all saturated_channels");
  // 振幅: 800×40、1200×60、2400×20 → 昇順 800 = 0–39、1200 = 40–99、2400 = 100–119 → median h = 59.5 → 1200
  checkNear(all.amplitude_median_adc, 1200, "all amplitude_median_adc");
  // 生の平均の倍率 = 平均振幅 = (800·40 + 1200·60 + 2400·20) / 120 = 152000 / 120 = 1266.67
  checkShape(all, 152000.0 / 120.0, "all");
  // ピーク cell: offset は偶数 {0,1,0,2,1,3}、奇数 {0,1,0,0,1,3}(AGET 0 の 3/30、AGET 1 の 5/40/60、
  //   AGET 2 の 12)。各 P につき偶数 2・奇数 2 → P+0: 10、P+1: 8、P+2: 2、P+3: 4(P ごと 24)
  //   値ごと: 150:10、151:18、152:20、153:24、154:24、155:14、156:6、157:4(計 120)
  //   昇順 index 150 = 0–9、151 = 10–27、152 = 28–47、153 = 48–71、154 = 72–95、155 = 96–109、…
  //   median h = 59.5 → 153、p10 h = 11.9 → 151、p90 h = 107.1 → 155
  checkNear(all.peak_cell.median, 153, "all peak_cell.median");
  checkNear(all.peak_cell.p10, 151, "all peak_cell.p10");
  checkNear(all.peak_cell.p90, 155, "all peak_cell.p90");
  // イベント内(全 AGET の使った ch)のばらつき = 3 − 0 = 3 が毎イベント
  checkNear(all.peak_cell_range_in_event.median, 3, "all peak_cell_range_in_event.median");
  checkNear(all.peak_cell_range_in_event.p10, 3, "all peak_cell_range_in_event.p10");
  checkNear(all.peak_cell_range_in_event.p90, 3, "all peak_cell_range_in_event.p90");
  // 窓は全部読み出し窓の中(P − 40 ≥ 110、P + 3 + 300 ≤ 457)→ どの offset も 120
  if (all.n.size() == pulserkernel::kWinLen) {
    checkCount(all.n[at(-40)], 120, "all n(-40)");
    checkCount(all.n[at(300)], 120, "all n(+300)");
  } else {
    check(false, "all n has kWinLen entries");
  }

  // ---- JSON ------------------------------------------------------------------------------------
  const std::string js = pulserkernel::toJson(r);
  check(contains(js, "\"run\": 90,"), "JSON \"run\": 90");
  check(contains(js, "\"events\": 20,"), "JSON \"events\": 20");
  check(contains(js, "\"window_cells\": [-40, 300]"), "JSON window_cells");
  check(contains(js, "\"min_amplitude_adc\": 100"), "JSON min_amplitude_adc");
  check(contains(js, "\"aget\": 0,") && contains(js, "\"aget\": 3,"), "JSON aget 0..3 entries");
  check(contains(js, "\"undershoot\": {\"min_adc\": -80, \"min_ratio\": -0.1, \"cell\": 30}"),
        "JSON aget0 undershoot object");
  check(contains(js, "\"rise_10_90_cells\": 6.4, \"fwhm_cells\": 16.5"), "JSON rise / fwhm");
  check(contains(js, "\"amplitude_median_adc\": null"), "JSON null amplitude (aget3)");
  check(contains(js, "\"peak_cell\": {\"median\": 152.5, \"p10\": 150.9, \"p90\": 154.1}"),
        "JSON aget0 peak_cell");
  for (const char *key :
       {"\"raw_file\"", "\"agets_without_fpn\"", "\"all\"", "\"agets\"", "\"waveforms\"",
        "\"channels\"", "\"saturated_waveforms\"", "\"saturated_channels\"",
        "\"tail_50_300\"", "\"rms_adc\"", "\"rms_ratio\"", "\"p2p_adc\"", "\"p2p_ratio\"",
        "\"peak_cell_range_in_event\"", "\"mean_norm\"", "\"mean_raw\"", "\"n\""})
    check(contains(js, key), std::string("JSON key ") + key);

  // ---- 入口: JSON と PNG を書く ------------------------------------------------------------------
  check(pulser_kernel(raw.c_str(), json.c_str(), png.c_str(), 0) == 0, "pulser_kernel() returns 0");
  check(readFile(json) == js, "pulser_kernel() wrote the same JSON");
  FileStat_t st;
  check(gSystem->GetPathInfo(png.c_str(), st) == 0 && st.fSize > 0, "pulser_kernel() wrote the PNG");
  check(pulser_kernel((dir + "/missing_raw.root").c_str(), json.c_str()) == 1,
        "pulser_kernel() returns 1 for a missing raw file");

  // ---- maxEvents -----------------------------------------------------------------------------
  // 先頭 5 イベント(e = 0..4、P = 150..154、e mod 4 == 0 は e = 0, 4 の 2 回)
  std::printf("[maxEvents = 5]\n");
  pulserkernel::Result m;
  check(pulserkernel::computePulserKernel(raw, 5, m, err), "computePulserKernel ok (maxEvents 5)");
  checkCount(m.events, 5, "events (maxEvents 5)");
  checkCount(m.all.waveforms, 30, "all waveforms (maxEvents 5): 6 × 5");
  checkCount(m.all.saturated_waveforms, 7, "all saturated_waveforms (maxEvents 5): 2 + 5");

  // ---- 窓が読み出し窓からはみ出す ----------------------------------------------------------------
  std::printf("[late peak: window partly outside cells 0..511]\n");
  pulserkernel::Result l;
  check(pulserkernel::computePulserKernel(lateRaw, 0, l, err), "computePulserKernel ok (late)");
  checkCount(l.all.waveforms, 4, "late all waveforms");
  if (l.all.n.size() == pulserkernel::kWinLen && l.all.mean_norm.size() == pulserkernel::kWinLen) {
    // ch 30 @ 450 は k ≤ 61 まで → n(61) = 4、n(62) = 2
    checkCount(l.all.n[at(61)], 4, "late n(+61)");
    checkCount(l.all.n[at(62)], 2, "late n(+62)");
    // 在るものだけの平均 = s(k) のまま: +120 → 0.02(ch 3 だけ)
    checkNear(l.all.mean_norm[at(120)], 0.02, "late mean_norm(+120) from the samples present");
  } else {
    check(false, "late n / mean_norm have kWinLen entries");
  }
  checkNear(l.all.tail_p2p_ratio, 0.03, "late tail_p2p_ratio");

  // 後片付け
  for (const std::string &q : {raw, lateRaw, json, png}) gSystem->Unlink(q.c_str());
  gSystem->Unlink(dir.c_str());

  std::printf("%s: %d failure(s)\n", g_failures == 0 ? "GREEN" : "RED", g_failures);
  return g_failures;
}
