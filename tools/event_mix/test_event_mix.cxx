// test_event_mix.cxx — event_mix の単体テスト(TODO/128)。合成データのみ(実データはリポに入れない)。
// 既知の波形を持つ raw ツリーを作り、足し算の規則・mix ツリー・再現性・AsAd 数のリーフ読みを検証する。
// 期待値は手計算(各ケースのコメントに式を書く)。
#include <unistd.h>

#include <functional>

#include "../root_sink/check.hpp"
#include "event_mix.hpp"

using evmix::kCells;
using Wave = std::vector<int16_t>;

namespace {

const char* kTmp = "/tmp";  // 一時 ROOT ファイルの置き場(cwd 非依存にするため pid で名前を分ける)

std::string tmpPath(const std::string& name) {
  return std::string(kTmp) + "/event_mix_test_" + std::to_string(::getpid()) + "_" + name;
}

// 1 イベント分の波形(nCh*512)を f(ch, t) で作る。
Wave makeWave(size_t nCh, const std::function<int(size_t, int)>& f) {
  Wave w(nCh * kCells);
  for (size_t c = 0; c < nCh; ++c)
    for (int t = 0; t < kCells; ++t) w[c * kCells + t] = static_cast<int16_t>(f(c, t));
  return w;
}

// SPEC §6.7 の raw ツリーを書く。entry i の eventId = 100 + i、eventTime = 5000 + i。
void writeRaw(const std::string& path, int nAsad, const std::vector<Wave>& events) {
  TFile f(path.c_str(), "RECREATE");
  TTree t("raw", "synthetic raw");
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  Wave buf(static_cast<size_t>(4 * nAsad) * 68 * kCells);
  t.Branch("eventId", &eventId, "eventId/i");
  t.Branch("eventTime", &eventTime, "eventTime/l");
  const std::string spec = "adc[" + std::to_string(4 * nAsad) + "][68][512]/S";
  t.Branch("adc", buf.data(), spec.c_str());
  for (size_t i = 0; i < events.size(); ++i) {
    eventId = static_cast<UInt_t>(100 + i);
    eventTime = 5000 + i;
    buf = events[i];
    t.Fill();
  }
  t.Write();
  f.Close();
}

struct RawOut {
  std::vector<Wave> adc;
  std::vector<UInt_t> eventId;
  std::vector<ULong64_t> eventTime;
  std::string leafTitle;
};

RawOut readRaw(const std::string& path) {
  RawOut r;
  TFile f(path.c_str(), "READ");
  auto* t = dynamic_cast<TTree*>(f.Get("raw"));
  if (!t) return r;
  TLeaf* leaf = t->GetLeaf("adc");
  r.leafTitle = leaf->GetTitle();
  Wave buf(static_cast<size_t>(leaf->GetLenStatic()));
  UInt_t id = 0;
  ULong64_t tm = 0;
  t->SetBranchAddress("eventId", &id);
  t->SetBranchAddress("eventTime", &tm);
  t->SetBranchAddress("adc", buf.data());
  for (Long64_t i = 0; i < t->GetEntries(); ++i) {
    t->GetEntry(i);
    r.adc.push_back(buf);
    r.eventId.push_back(id);
    r.eventTime.push_back(tm);
  }
  return r;
}

constexpr size_t kCh1 = 4 * 68;  // 1 AsAd

// ---- 足し算の核(ROOT 不要の純粋関数) ----------------------------------------------

void test_pedestal_only_B_leaves_A_unchanged() {
  // B は全 cell 同じ値(ped と一致)→ B - ped = 0 → out = A(バイト一致)。
  Wave a = makeWave(kCh1, [](size_t c, int t) { return 300 + static_cast<int>((c * 7 + t * 3) % 50); });
  Wave b = makeWave(kCh1, [](size_t c, int) { return 100 + static_cast<int>(c % 9); });
  Wave out;
  evmix::mixEvent(a, {&b}, {0}, out);
  CHECK(out == a);
  evmix::mixEvent(a, {&b}, {17}, out);  // Δ があっても平坦な B は 0 を足すだけ
  CHECK(out == a);
}

void test_shift_moves_peak_and_out_of_range_adds_zero() {
  // A は全 cell 1000。B は cell 0..40 が 100(ped = 100)、cell 100 = 400(山, +300)、
  // cell 0 = 900(+800, ped 窓の外)、cell 511 = 700(+600)。
  Wave a = makeWave(kCh1, [](size_t, int) { return 1000; });
  Wave b = makeWave(kCh1, [](size_t, int t) {
    if (t == 100) return 400;
    if (t == 0) return 900;
    if (t == 511) return 700;
    return 100;
  });
  Wave out;
  // Δ = +7: 山は cell 107、元の位置 100 は B[93]-ped = 0 → 1000。
  evmix::mixEvent(a, {&b}, {7}, out);
  CHECK_EQ(out[107], 1000 + 300);
  CHECK_EQ(out[100], 1000);
  // cell 0 は t=7 へ移る(1000+800)。t < 7 は B[t-7] が範囲外 → 0 を足す → 1000。
  CHECK_EQ(out[7], 1000 + 800);
  CHECK_EQ(out[0], 1000);
  CHECK_EQ(out[6], 1000);
  // B[511] は Δ=+7 では範囲外へ落ちる(どの t にも現れない)。
  for (int t = 0; t < kCells; ++t) CHECK(out[t] != 1000 + 600);
  // Δ = -3: B[511] は t=508 へ。t = 509..511 は s = 512..514 が範囲外 → 1000。
  evmix::mixEvent(a, {&b}, {-3}, out);
  CHECK_EQ(out[508], 1000 + 600);
  CHECK_EQ(out[511], 1000);
  CHECK_EQ(out[97], 1000 + 300);
  // 全 ch に同じ規則(最後の ch でも)
  CHECK_EQ(out[(kCh1 - 1) * kCells + 97], 1000 + 300);
}

void test_clamp_and_rounding() {
  // 飽和: A = 4000、B の山 +500 → 4500 → 4095。
  Wave a = makeWave(kCh1, [](size_t, int) { return 4000; });
  Wave b = makeWave(kCh1, [](size_t, int t) { return t == 100 ? 600 : 100; });
  Wave out;
  evmix::mixEvent(a, {&b}, {0}, out);
  CHECK_EQ(out[100], 4095);
  CHECK_EQ(out[99], 4000);
  // 0 未満: A = 100、B の谷 (ped 1000 → 0) = -1000 → -900 → 0。
  Wave a2 = makeWave(kCh1, [](size_t, int) { return 100; });
  Wave b2 = makeWave(kCh1, [](size_t, int t) { return t == 100 ? 0 : 1000; });
  evmix::mixEvent(a2, {&b2}, {0}, out);
  CHECK_EQ(out[100], 0);
  CHECK_EQ(out[99], 100);
  // 四捨五入: ped 窓 cell 1..20 = 10 個が 10、10 個が 11 → ped = 10.5。
  // B[100] = 21 → +10.5 → A=1000 で 1010.5 → 1011。B[101] = 20 → +9.5 → 1009.5 → 1010。
  Wave a3 = makeWave(kCh1, [](size_t, int) { return 1000; });
  Wave b3 = makeWave(kCh1, [](size_t, int t) {
    if (t == 100) return 21;
    if (t == 101) return 20;
    if (t >= 1 && t <= 10) return 10;
    return 11;
  });
  evmix::mixEvent(a3, {&b3}, {0}, out);
  CHECK_EQ(out[100], 1011);
  CHECK_EQ(out[101], 1010);
}

void test_three_sources_add() {
  // A=1000 平坦、B(ped 10)山 +50 @ cell 50、C(ped 20)山 +70 @ cell 60 を Δ_C=-10 で → cell 50。
  Wave a = makeWave(kCh1, [](size_t, int) { return 1000; });
  Wave b = makeWave(kCh1, [](size_t, int t) { return t == 50 ? 60 : 10; });
  Wave c = makeWave(kCh1, [](size_t, int t) { return t == 60 ? 90 : 20; });
  Wave out;
  evmix::mixEvent(a, {&b, &c}, {0, -10}, out);
  CHECK_EQ(out[50], 1000 + 50 + 70);
  CHECK_EQ(out[60], 1000);
}

// ---- 乱数・組ファイル ---------------------------------------------------------------

void test_random_specs_and_parse() {
  const auto s1 = evmix::randomSpecs(10, 200, 42, 5);
  const auto s2 = evmix::randomSpecs(10, 200, 42, 5);
  const auto s3 = evmix::randomSpecs(10, 200, 43, 5);
  bool same12 = true, same13 = true, ok = true, sawNeg = false, sawPos = false;
  for (size_t i = 0; i < s1.size(); ++i) {
    same12 = same12 && s1[i].src == s2[i].src && s1[i].shift == s2[i].shift;
    same13 = same13 && s1[i].src == s3[i].src && s1[i].shift == s3[i].shift;
    ok = ok && s1[i].src[0] != s1[i].src[1] && s1[i].src[0] >= 0 && s1[i].src[1] < 10 && s1[i].shift[0] == 0 &&
         std::abs(s1[i].shift[1]) <= 5;
    sawNeg = sawNeg || s1[i].shift[1] < 0;
    sawPos = sawPos || s1[i].shift[1] > 0;
  }
  CHECK(same12);
  CHECK(!same13);
  CHECK(ok);
  CHECK(sawNeg && sawPos);
  for (const auto& m : evmix::randomSpecs(10, 20, 1, 0)) CHECK_EQ(m.shift[1], 0);  // 既定 D = 0

  std::istringstream in("# comment\n3 7\n\n1 2 -4  # trailing\n0 5 3 8 -2\n");
  const auto v = evmix::parseSpecs(in, 10);
  CHECK_EQ(v.size(), 3);
  CHECK(v[0].src == (std::vector<int>{3, 7}) && v[0].shift == (std::vector<int>{0, 0}));
  CHECK(v[1].src == (std::vector<int>{1, 2}) && v[1].shift == (std::vector<int>{0, -4}));
  CHECK(v[2].src == (std::vector<int>{0, 5, 8}) && v[2].shift == (std::vector<int>{0, 3, -2}));
  std::istringstream bad1("1 2 3 4\n"), bad2("1 99\n"), bad3("1\n"), bad4("1 x\n");
  for (auto* s : {&bad1, &bad2, &bad3, &bad4}) {
    bool threw = false;
    try {
      evmix::parseSpecs(*s, 10);
    } catch (const std::runtime_error&) {
      threw = true;
    }
    CHECK(threw);
  }
}

// ---- ROOT I/O --------------------------------------------------------------------

std::vector<Wave> distinctEvents(int nAsad, int n) {
  std::vector<Wave> ev;
  const size_t nCh = static_cast<size_t>(4 * nAsad) * 68;
  for (int e = 0; e < n; ++e)
    ev.push_back(makeWave(nCh, [e](size_t c, int t) { return 200 + e * 30 + static_cast<int>((c + t) % 11); }));
  return ev;
}

void test_mix_tree_contents() {
  const std::string in = tmpPath("in.root"), out = tmpPath("out.root");
  writeRaw(in, 1, distinctEvents(1, 6));
  std::vector<evmix::MixSpec> specs = {{{4, 1}, {0, 9}}, {{2, 5, 0}, {0, -3, 20}}};
  CHECK_EQ(evmix::mixFile(in, out, specs), 2);

  TFile f(out.c_str(), "READ");
  auto* mix = dynamic_cast<TTree*>(f.Get("mix"));
  CHECK(mix != nullptr);
  if (!mix) return;
  CHECK_EQ(mix->GetEntries(), 2);
  Int_t nSrc = 0, srcEntry[8] = {}, shift[8] = {};
  UInt_t eventId = 0, srcEventId[8] = {};
  mix->SetBranchAddress("eventId", &eventId);
  mix->SetBranchAddress("nSrc", &nSrc);
  mix->SetBranchAddress("srcEntry", srcEntry);
  mix->SetBranchAddress("srcEventId", srcEventId);
  mix->SetBranchAddress("shift", shift);
  mix->GetEntry(0);
  CHECK_EQ(eventId, 0);
  CHECK_EQ(nSrc, 2);
  CHECK_EQ(srcEntry[0], 4);
  CHECK_EQ(srcEntry[1], 1);
  CHECK_EQ(srcEventId[0], 104);  // 入力 entry i の eventId = 100 + i
  CHECK_EQ(srcEventId[1], 101);
  CHECK_EQ(shift[0], 0);
  CHECK_EQ(shift[1], 9);
  mix->GetEntry(1);
  CHECK_EQ(eventId, 1);
  CHECK_EQ(nSrc, 3);
  CHECK_EQ(srcEntry[2], 0);
  CHECK_EQ(srcEventId[1], 105);
  CHECK_EQ(shift[1], -3);
  CHECK_EQ(shift[2], 20);

  const RawOut r = readRaw(out);
  CHECK_EQ(r.adc.size(), 2);
  CHECK_EQ(r.eventId[0], 0);  // 0 からの連番
  CHECK_EQ(r.eventId[1], 1);
  CHECK_EQ(r.eventTime[0], 5004);  // A の eventTime(5000 + entry 4)
  CHECK_EQ(r.eventTime[1], 5002);
  CHECK(r.leafTitle == "adc[4][68][512]");
  std::remove(in.c_str());
  std::remove(out.c_str());
}

void test_same_seed_same_output() {
  const std::string in = tmpPath("in2.root"), o1 = tmpPath("o1.root"), o2 = tmpPath("o2.root"),
                    o3 = tmpPath("o3.root");
  writeRaw(in, 1, distinctEvents(1, 8));
  evmix::mixFile(in, o1, evmix::randomSpecs(8, 5, 99, 30));
  evmix::mixFile(in, o2, evmix::randomSpecs(8, 5, 99, 30));
  evmix::mixFile(in, o3, evmix::randomSpecs(8, 5, 100, 30));
  const RawOut r1 = readRaw(o1), r2 = readRaw(o2), r3 = readRaw(o3);
  CHECK_EQ(r1.adc.size(), 5);
  CHECK(r1.adc == r2.adc);
  CHECK(r1.eventTime == r2.eventTime);
  CHECK(r1.adc != r3.adc);
  for (const auto& p : {in, o1, o2, o3}) std::remove(p.c_str());
}

void test_nasad_read_from_leaf() {
  // 2 AsAd(adc[8][68][512])の合成入力。第 2 AsAd(ch 272..543)にも同じ規則が効く。
  const std::string in = tmpPath("in3.root"), out = tmpPath("out3.root");
  const size_t nCh = 8 * 68;
  std::vector<Wave> ev;
  ev.push_back(makeWave(nCh, [](size_t, int) { return 1000; }));
  ev.push_back(makeWave(nCh, [](size_t c, int t) { return t == 30 ? 150 + static_cast<int>(c % 5) : 100; }));
  writeRaw(in, 2, ev);
  CHECK_EQ(evmix::mixFile(in, out, {{{0, 1}, {0, 4}}}), 1);
  const RawOut r = readRaw(out);
  CHECK(r.leafTitle == "adc[8][68][512]");
  CHECK_EQ(r.adc.size(), 1);
  if (r.adc.empty()) return;
  CHECK_EQ(r.adc[0].size(), nCh * kCells);
  // 山は cell 34、振幅 = 50 + (c % 5)。ch 300 (第 2 AsAd): 50 + 0 → 1050。ch 543 (最後): 50 + 3 → 1053。
  CHECK_EQ(r.adc[0][300 * kCells + 34], 1050);
  CHECK_EQ(r.adc[0][543 * kCells + 34], 1053);
  CHECK_EQ(r.adc[0][543 * kCells + 30], 1000);
  std::remove(in.c_str());
  std::remove(out.c_str());
}

}  // namespace

int main() {
  test_pedestal_only_B_leaves_A_unchanged();
  test_shift_moves_peak_and_out_of_range_adds_zero();
  test_clamp_and_rounding();
  test_three_sources_add();
  test_random_specs_and_parse();
  test_mix_tree_contents();
  test_same_seed_same_output();
  test_nasad_read_from_leaf();
  return tpccheck::report("test_event_mix");
}
