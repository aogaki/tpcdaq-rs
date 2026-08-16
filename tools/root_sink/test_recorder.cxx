// test_recorder.cxx — Recorder(ファイルライフサイクル + monitor.root)の単体テスト
//                     (TODO/011 → 022 → 054)。
//
// **`make test` からは外してある**(`make test-root`)。tpc_wire / rs_core / eb_core /
// conformance の 4 本は ROOT 非依存のまま —— ヘッダテストに ROOT を混ぜない(発注書 §3)。
//
// 試験方式は既存 3 本と同じ「素の CHECK + main」(check.hpp)。やっていること:
//
//   合成 BuiltEvent を書く → **同プロセスで TFile を開き直して読み戻し**、
//   ファイルの命名・finalize・ロールオーバ・AutoSave・圧縮・monitor.root を機械照合する。
//
// **v1.17(054)で出力は PEventTPC 1 形式のみ**。TTree の中身(chargeMap の値)の照合は
// test_pevent.cxx の担当で、ここは**ファイルライフサイクル(SPEC §6.5)**が持ち場。
//
// 引数を 1 つ与えると **inspect モード**: その .root を読み戻して要約を印字する。
// E2E(graw_replay → receiver → decoder → root_sink)の entries=108 照合はこれを使う
// (発注書 §4「test_recorder 側の読み戻しユーティリティ」)。
//
//   ./test_recorder                      # 単体テスト
//   ./test_recorder /path/run0000.root   # 読み戻して要約(entries=... を印字)

#include <sys/stat.h>

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <utility>
#include <vector>

#include <TFile.h>
#include <TH1D.h>
#include <TH2D.h>
#include <TROOT.h>
#include <TTree.h>

#include "TPCReco/PEventTPC.h"
#include "check.hpp"
#include "eb_core.hpp"
#include "root_recorder.hpp"

namespace {

// Recorder は PEventTPC を書くのでジオメトリが要る。**合成フィクスチャ**を使う
// (このファイルの持ち場はファイルライフサイクルであって chargeMap の値ではない ——
// 値の照合は test_pevent.cxx。実 .dat はリポに入れない、CLAUDE.md)。
const char* kFixtureMiniReduced = "../../tests/fixtures/geometry_mini_reduced.dat";

// 1 回だけ読む(全テストが共有する読み取り専用の値)。
const tpcgeo::Geometry& fixture_geometry() {
  static const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  return geo;
}

// ---------------------------------------------------------------------------
// 読み戻しユーティリティ(inspect モードと全テストが共有)
// ---------------------------------------------------------------------------

// TTree の 1 エントリ = 1 ビルド済みイベント(SPEC §6.4)。chargeMap は読まない
// (実機ファイルは 1 エントリ 17 MB —— ライフサイクル試験には EventInfo で足りる)。
struct ReadEntry {
  unsigned event_id = 0;
  unsigned long timestamp = 0;
  long run_id = 0;
};

// `path` の TPCData を EventInfo だけ読み切る。開けなければ空(呼び手が CHECK で落ちる)。
std::vector<ReadEntry> read_root_file(const std::string& path) {
  std::vector<ReadEntry> out;
  TFile* in = TFile::Open(path.c_str(), "READ");
  if (in == nullptr || in->IsZombie()) {
    std::printf("read_root_file: cannot open %s\n", path.c_str());
    delete in;
    return out;
  }
  TTree* tree = dynamic_cast<TTree*>(in->Get(rootsink::kPEventTreeName));
  if (tree == nullptr) {
    std::printf("read_root_file: no TTree named \"%s\" in %s\n", rootsink::kPEventTreeName,
                path.c_str());
    in->Close();
    delete in;
    return out;
  }
  PEventTPC* ev = nullptr;
  // **上位ブランチ "Event" は有効のまま**にしないと GetEntry がオブジェクトを作らない。
  tree->SetBranchStatus("myChargeMap*", false);
  tree->SetBranchAddress(rootsink::kPEventBranchName, &ev);
  const Long64_t entries = tree->GetEntries();
  for (Long64_t i = 0; i < entries; ++i) {
    tree->GetEntry(i);
    if (ev == nullptr) continue;
    ReadEntry re;
    re.event_id = ev->GetEventInfo().GetEventId();
    re.timestamp = ev->GetEventInfo().GetEventTimestamp();
    re.run_id = ev->GetEventInfo().GetRunId();
    out.push_back(re);
  }
  in->Close();
  delete in;
  return out;
}

// AutoSave の観測専用の軽量読み手: ツリーの有無とエントリ数だけを見る(ブランチを
// 読まないので、まだ Recorder が書いている最中の inprogress にも使える)。
// キーが無い(AutoSave/Close がまだ一度も走っていない)ときは -1。
Long64_t peek_tree_entries(const std::string& path, const char* tree_name) {
  TFile* in = TFile::Open(path.c_str(), "READ");
  if (in == nullptr || in->IsZombie()) {
    delete in;
    return -1;
  }
  TTree* tree = dynamic_cast<TTree*>(in->Get(tree_name));
  const Long64_t entries = (tree == nullptr) ? -1 : tree->GetEntries();
  in->Close();
  delete in;
  return entries;
}

// ---------------------------------------------------------------------------
// テスト用の小道具
// ---------------------------------------------------------------------------

bool exists(const std::string& p) {
  struct stat st;
  return ::stat(p.c_str(), &st) == 0;
}

// dir 直下で prefix 始まりのものを名前順に返す(inprogress の残骸検出とロールオーバ確認用)。
std::vector<std::string> list_prefixed(const std::string& dir, const std::string& prefix) {
  std::vector<std::string> out = rootsink::list_directory(dir);
  std::vector<std::string> hits;
  for (const std::string& name : out) {
    if (name.size() >= prefix.size() && name.compare(0, prefix.size(), prefix) == 0) {
      hits.push_back(name);
    }
  }
  std::sort(hits.begin(), hits.end());
  return hits;
}

// テスト毎に使い捨てのディレクトリ(並列実行と再実行に耐える)。
std::string scratch_dir(const char* tag) {
  const char* tmp = std::getenv("TMPDIR");
  std::string base = (tmp != nullptr && tmp[0] != '\0') ? std::string(tmp) : std::string("/tmp");
  if (!base.empty() && base.back() == '/') base.pop_back();
  char suffix[64];
  std::snprintf(suffix, sizeof(suffix), "/tpcdaq_test_recorder_%d_%s",
                static_cast<int>(::getpid()), tag);
  const std::string dir = base + suffix;
  rootsink::mkdir_p(dir);
  return dir;
}

void remove_tree(const std::string& dir) {
  for (const std::string& name : rootsink::list_directory(dir)) {
    const std::string path = dir + "/" + name;
    struct stat st;
    if (::stat(path.c_str(), &st) == 0 && S_ISDIR(st.st_mode)) {
      remove_tree(path);
    } else {
      std::remove(path.c_str());
    }
  }
  ::rmdir(dir.c_str());
}

// SPEC §2.4 の item パック。[31:30] aget / [29:23] chan / [22:14] bucket / [11:0] ADC。
uint32_t pack_item(uint32_t aget, uint32_t chan, uint32_t bucket, uint32_t adc) {
  return (aget << 30) | (chan << 23) | (bucket << 14) | adc;
}

void push_item(std::vector<uint8_t>& items, uint32_t word) {
  items.push_back(static_cast<uint8_t>(word & 0xFF));
  items.push_back(static_cast<uint8_t>((word >> 8) & 0xFF));
  items.push_back(static_cast<uint8_t>((word >> 16) & 0xFF));
  items.push_back(static_cast<uint8_t>((word >> 24) & 0xFF));
}

// 非対称なヘッダを持つ合成フラグメント(取り違えが値で見えるように全部ずらす)。
rootsink::OwnedFragment make_fragment(uint32_t run, uint32_t event_idx, uint8_t cobo,
                                      uint8_t asad) {
  rootsink::OwnedFragment f;
  f.run_number = run;
  f.event_idx = event_idx;
  // 48bit 有効。フラグメント毎に違う値にして、エントリ間の混線を検出する。
  f.event_time = 0x0000'1234'5678'9abcULL + 0x100ULL * event_idx + cobo * 16 + asad;
  f.cobo = cobo;
  f.asad = asad;
  f.frame_type = 2;
  f.revision = 5;
  f.read_offset = 136 + cobo;
  f.status = 3 + asad;
  f.mult[0] = 68;
  f.mult[1] = 0;
  f.mult[2] = 17;
  f.mult[3] = 3 + event_idx;
  f.window_out = 512 + event_idx;
  f.last_cell[0] = 7;
  f.last_cell[1] = 9;
  f.last_cell[2] = 11;
  f.last_cell[3] = 13 + cobo;
  return f;
}

// ---------------------------------------------------------------------------
// 2. 異常終了は inprogress のまま残す(完全 run に化けない、SPEC §6.5)
// ---------------------------------------------------------------------------
void test_shutdown_without_eos_keeps_the_inprogress_name() {
  const std::string dir = scratch_dir("noeos");
  const uint32_t kRun = 12;
  std::vector<rootsink::RootFileRecord> files;
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &fixture_geometry();
    rootsink::Recorder rec(cfg);
    rootsink::BuiltEvent ev;
    ev.run_number = kRun;
    ev.event_idx = 1;
    rootsink::OwnedFragment f = make_fragment(kRun, 1, 0, 0);
    push_item(f.items, pack_item(0, 1, 2, 3));
    ev.fragments.push_back(std::move(f));
    rec.write(ev, 1000);
    rec.shutdown();  // EOS 無しで停止 = finalize しない
    files = rec.files_snapshot();
  }
  const std::string run_dir = dir + "/run0012";
  CHECK(!exists(run_dir + "/run0012.root"));  // 完全 run に化けない
  const std::vector<std::string> left = list_prefixed(run_dir, "run_inprogress_");
  CHECK_EQ(left.size(), 1);
  CHECK_EQ(files.size(), 1);
  if (files.size() == 1 && left.size() == 1) {
    CHECK(files[0].path == run_dir + "/" + left[0]);  // JSON に出るのも inprogress のパス
    CHECK_EQ(files[0].entries, 1);
  }
  // 中身は読める(途中まででも捨てていない)
  if (left.size() == 1) {
    const std::vector<ReadEntry> entries = read_root_file(run_dir + "/" + left[0]);
    CHECK_EQ(entries.size(), 1);
  }
  remove_tree(dir);
}

// ---------------------------------------------------------------------------
// 2b. 混在 run の防御は finalize しない(SPEC §6.5、TODO/024 R-P2-1)
// ---------------------------------------------------------------------------
//
// root_sink.cxx の consume() が run_number_mismatch の増分を検知して fatal(exit 6)
// にする経路が正になった(SPEC §6.2-5 v1.10)ので、ここに来るのは本来到達不能な
// 最後の砦。それでも Recorder::write() に直接違う run_number のイベントを渡すと、
// **旧 run は finalize されず inprogress のまま残る**こと(完成 run 名に化けない)を
// 単体で機械照合する。
void test_mixed_run_defense_keeps_the_old_run_inprogress() {
  const std::string dir = scratch_dir("mixedrun");
  const uint32_t kRunA = 5;
  const uint32_t kRunB = 6;
  std::vector<rootsink::RootFileRecord> files;
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &fixture_geometry();
    rootsink::Recorder rec(cfg);

    rootsink::BuiltEvent evA;
    evA.run_number = kRunA;
    evA.event_idx = 1;
    rootsink::OwnedFragment fA = make_fragment(kRunA, 1, 0, 0);
    push_item(fA.items, pack_item(0, 1, 2, 3));
    evA.fragments.push_back(std::move(fA));
    rec.write(evA, /*now_ms=*/1000);
    CHECK(rec.is_open());

    // run B のイベントが run A の EOS より先に届く(プロトコル違反 — 本来は上流の
    // consume() が fatal にする経路。Recorder 単体はその最後の砦)。
    rootsink::BuiltEvent evB;
    evB.run_number = kRunB;
    evB.event_idx = 1;
    rootsink::OwnedFragment fB = make_fragment(kRunB, 1, 0, 0);
    push_item(fB.items, pack_item(0, 1, 2, 4));
    evB.fragments.push_back(std::move(fB));
    rec.write(evB, /*now_ms=*/1010);

    rec.close_run(kRunB, /*now_ms=*/1020);
    CHECK(rec.fatal_reason() == nullptr);
    files = rec.files_snapshot();
  }
  const std::string run_dir_a = dir + "/run0005";
  const std::string run_dir_b = dir + "/run0006";

  // run A: **finalize されない**(完成 run 名に化けない、finalize=false に変更した点)。
  CHECK(!exists(run_dir_a + "/run0005.root"));
  const std::vector<std::string> left_a = list_prefixed(run_dir_a, "run_inprogress_");
  CHECK_EQ(left_a.size(), 1);

  // run B: 通常どおり close_run で finalize される。
  CHECK(exists(run_dir_b + "/run0006.root"));
  CHECK_EQ(list_prefixed(run_dir_b, "run_inprogress_").size(), 0);

  CHECK_EQ(files.size(), 2);
  if (files.size() == 2 && left_a.size() == 1) {
    CHECK(files[0].path == run_dir_a + "/" + left_a[0]);
    CHECK_EQ(files[0].entries, 1);
    CHECK(files[1].path == run_dir_b + "/run0006.root");
    CHECK_EQ(files[1].entries, 1);
  }
  // run A の中身も捨てていない(inprogress でも読める)。
  if (left_a.size() == 1) {
    const std::vector<ReadEntry> entries = read_root_file(run_dir_a + "/" + left_a[0]);
    CHECK_EQ(entries.size(), 1);
  }
  remove_tree(dir);
}

// ---------------------------------------------------------------------------
// 2c. write() 連打だけで AutoSave が走る(tick() を呼ばない、TODO/024 R-P2-2)
// ---------------------------------------------------------------------------
//
// 手計算の出典: kAutoSaveIntervalMs = 30000。open_part() が last_autosave_ms_ を
// 最初の write() の now_ms(1000)で初期化するので、次の write() の now_ms が
// 1000 + 30000 = 31000 に達した瞬間(`now_ms < last_autosave_ms_ + interval` が
// false になる境界)で AutoSave が走るはず。**tick() は一度も呼ばない** —— 以前は
// これが原因でデータが途切れない run では AutoSave が一度も走らなかった。
//
// 観測方法: プロセス内で同じ inprogress パスをもう一度 `TFile::Open(..., "READ")`
// で開く(Recorder はまだ閉じていない)。AutoSave が書いたキーが無ければツリーは
// 見えず(peek_tree_entries が -1)、AutoSave が走った後は同じパスからエントリ数が
// 読める(peek_tree_entries はブランチを読まないので、書き手がまだ握っている
// バッファには触れない)。
void test_write_triggers_autosave_without_tick() {
  const std::string dir = scratch_dir("autosave");
  const uint32_t kRun = 9;
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &fixture_geometry();
    rootsink::Recorder rec(cfg);

    rootsink::BuiltEvent ev0;
    ev0.run_number = kRun;
    ev0.event_idx = 0;
    rootsink::OwnedFragment f0 = make_fragment(kRun, 0, 0, 0);
    push_item(f0.items, pack_item(0, 1, 2, 3));
    ev0.fragments.push_back(std::move(f0));
    rec.write(ev0, /*now_ms=*/1000);
    const std::string provisional_path = rec.provisional();
    CHECK(!provisional_path.empty());

    // AutoSave 前: まだキーが書かれていないので、別の読み手からはツリーが見えない。
    CHECK_EQ(peek_tree_entries(provisional_path, rootsink::kPEventTreeName), -1);

    rootsink::BuiltEvent ev1;
    ev1.run_number = kRun;
    ev1.event_idx = 1;
    rootsink::OwnedFragment f1 = make_fragment(kRun, 1, 0, 0);
    push_item(f1.items, pack_item(0, 1, 2, 4));
    ev1.fragments.push_back(std::move(f1));
    // 30 s の期限をまたぐ now_ms。tick() は一度も呼んでいない。
    rec.write(ev1, /*now_ms=*/1000 + 30000);

    // AutoSave 後: 同じパスを別の TFile で開くとツリーが読める(2 エントリ)。
    CHECK_EQ(peek_tree_entries(provisional_path, rootsink::kPEventTreeName), 2);

    rec.close_run(kRun, /*now_ms=*/1000 + 30010);
    CHECK(rec.fatal_reason() == nullptr);
    CHECK_EQ(rec.entries_written(), 2);
  }
  remove_tree(dir);
}

// ---------------------------------------------------------------------------
// 3. rollover(SPEC §6.5: サイズ超過で part ファイルへ)
// ---------------------------------------------------------------------------
//
// 手計算の出典: `max_root_bytes = 1` は TFile ヘッダ(約 100 B)だけで超えるので
// **1 イベント毎に 1 ファイル**。3 イベント → run0012.root / run0012_0001.root /
// run0012_0002.root の 3 本、各 1 エントリ。ROOT の自動分割(fgMaxTreeSize)は
// 無効化してあるので、命名は必ずこの規則になる(ROOT 任せにしない、発注書 §2)。
void test_rollover_splits_the_run_into_numbered_parts() {
  const std::string dir = scratch_dir("rollover");
  const uint32_t kRun = 12;
  std::vector<rootsink::RootFileRecord> files;
  uint64_t entries_written = 0;
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &fixture_geometry();
    cfg.max_root_bytes = 1;  // 事実上「毎エントリでロールオーバ」
    rootsink::Recorder rec(cfg);
    for (uint32_t i = 0; i < 3; ++i) {
      rootsink::BuiltEvent ev;
      ev.run_number = kRun;
      ev.event_idx = i;
      rootsink::OwnedFragment f = make_fragment(kRun, i, 0, 0);
      push_item(f.items, pack_item(0, 1, 2, 100 + i));
      ev.fragments.push_back(std::move(f));
      rec.write(ev, 1000 + i);
    }
    rec.close_run(kRun, 2000);
    CHECK(rec.fatal_reason() == nullptr);
    files = rec.files_snapshot();
    entries_written = rec.entries_written();
  }
  const std::string run_dir = dir + "/run0012";
  CHECK_EQ(entries_written, 3);
  CHECK_EQ(files.size(), 3);
  // ロールオーバの尻に空ファイルを作らない(次の書き込みで初めて開く)
  CHECK_EQ(list_prefixed(run_dir, "run_inprogress_").size(), 0);
  const char* expect_names[3] = {"run0012.root", "run0012_0001.root", "run0012_0002.root"};
  for (size_t i = 0; i < files.size() && i < 3; ++i) {
    CHECK(files[i].path == run_dir + "/" + expect_names[i]);
    CHECK_EQ(files[i].entries, 1);
    CHECK(exists(run_dir + "/" + expect_names[i]));
    const std::vector<ReadEntry> entries = read_root_file(run_dir + "/" + expect_names[i]);
    CHECK_EQ(entries.size(), 1);
    if (entries.size() == 1) {
      CHECK_EQ(entries[0].event_id, i);
      // event_time は make_fragment の式(イベント毎に 0x100 ずつずらす)
      CHECK_EQ(entries[0].timestamp, 0x0000'1234'5678'9abcULL + 0x100ULL * i);
    }
  }
  remove_tree(dir);
}

// ---------------------------------------------------------------------------
// 4. 範囲外の chan は黙って消さない(CLAUDE.md「silent failure を作らない」)
// ---------------------------------------------------------------------------
//
// item の chan フィールドは 7 bit(0–127)だが AGET は 68 ch。68 以上は作業マスに
// 置き場がないので落とすしかない —— **数えて JSON に出す**(計数の実体は Filler)。
void test_out_of_range_channel_is_counted_not_silently_dropped() {
  const std::string dir = scratch_dir("range");
  const uint32_t kRun = 3;
  uint64_t out_of_range = 0;
  std::string run_dir = dir + "/run0003";
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &fixture_geometry();
    rootsink::Recorder rec(cfg);
    rootsink::BuiltEvent ev;
    ev.run_number = kRun;
    ev.event_idx = 0;
    rootsink::OwnedFragment f = make_fragment(kRun, 0, 0, 0);
    push_item(f.items, pack_item(0, 68, 0, 11));   // 範囲外(68)
    push_item(f.items, pack_item(1, 127, 1, 22));  // 範囲外(最大値)
    push_item(f.items, pack_item(2, 67, 2, 33));   // 正常(境界の内側)
    ev.fragments.push_back(std::move(f));
    rec.write(ev, 1000);
    rec.close_run(kRun, 1010);
    out_of_range = rec.items_out_of_range();
  }
  CHECK_EQ(out_of_range, 2);  // chan 68 と chan 127 の 2 件だけ
  // 落としたのは item であってイベントではない —— エントリは普通に 1 本書かれる。
  const std::vector<ReadEntry> entries = read_root_file(run_dir + "/run0003.root");
  CHECK_EQ(entries.size(), 1);
  if (entries.size() == 1) CHECK_EQ(entries[0].event_id, 0);
  remove_tree(dir);
}

// ---------------------------------------------------------------------------
// 6. 圧縮設定(TODO/014): 既定 101(ZLIB-1)・明示指定(例 505=ZSTD-5)の反映
// ---------------------------------------------------------------------------
//
// SPEC §6.4(v1.5): Warsaw のオフライン解析機は DAQ 計算機と同一の旧 ROOT で
// ZSTD(505、ROOT 6.20+ 必須)を読めないため、既定は全時代互換の ZLIB-1(=101、
// 算出は algorithm*100+level。1=ZLIB, 5=ZSTD)。`RecorderConfig::compression` で
// 明示指定できることを固定する(root_sink.cxx の `--root-compression` はこれを配線するだけ)。
int read_compression_settings(const std::string& path) {
  TFile* in = TFile::Open(path.c_str(), "READ");
  if (in == nullptr || in->IsZombie()) {
    std::printf("read_compression_settings: cannot open %s\n", path.c_str());
    delete in;
    return -1;
  }
  const int settings = in->GetCompressionSettings();
  in->Close();
  delete in;
  return settings;
}

void write_one_entry_run(const std::string& dir, uint32_t run, int* compression) {
  rootsink::RecorderConfig cfg;
  cfg.output_root = dir;
  cfg.geometry = &fixture_geometry();
  if (compression != nullptr) cfg.compression = *compression;
  rootsink::Recorder rec(cfg);
  rootsink::BuiltEvent ev;
  ev.run_number = run;
  ev.event_idx = 0;
  rootsink::OwnedFragment f = make_fragment(run, 0, 0, 0);
  push_item(f.items, pack_item(0, 1, 0, 5));
  ev.fragments.push_back(std::move(f));
  rec.write(ev, 1000);
  rec.close_run(run, 1010);
  CHECK(rec.fatal_reason() == nullptr);
}

void test_default_compression_is_101_zlib1() {
  const std::string dir = scratch_dir("compdefault");
  const uint32_t kRun = 21;
  write_one_entry_run(dir, kRun, /*compression=*/nullptr);  // RecorderConfig 既定のまま
  const std::string path = dir + "/run0021/run0021.root";
  CHECK(exists(path));
  CHECK_EQ(read_compression_settings(path), 101);  // ZLIB-1(SPEC §6.4 既定、v1.5)
  remove_tree(dir);
}

void test_explicit_compression_setting_is_honored() {
  const std::string dir = scratch_dir("compexplicit");
  const uint32_t kRun = 22;
  int zstd5 = 505;
  write_one_entry_run(dir, kRun, &zstd5);
  const std::string path = dir + "/run0022/run0022.root";
  CHECK(exists(path));
  CHECK_EQ(read_compression_settings(path), 505);  // 明示指定が反映される
  remove_tree(dir);
}

// ---------------------------------------------------------------------------
// 7. モニタヒストの ROOT 書き出し(TODO/022、SPEC §5.2/§6.5/§12-9)
// ---------------------------------------------------------------------------
//
// **ROOT IO は Recorder スレッドだけ**(SPEC §5.1)—— 集計器(monitor_hist.hpp)は
// ROOT を知らず、run close で受け取った配列を Recorder が TH1D/TH2D にして
// `run{run:04}_monitor.root` に書く。ここはその書き手の照合。

// 既知の値を入れた 9 枚の受け皿。形は HistAccumulator が作るものと同じ
// (2D = Nstrip × 512 / 1D = 512)。値は非対称に、手で置く。
rsmon::HistSnapshot make_known_snapshot() {
  static const char* kNames[9] = {"StripTimeU", "StripTimeV", "StripTimeW",
                                  "ChargeU",    "ChargeV",    "ChargeW",
                                  "ChargeMaxU", "ChargeMaxV", "ChargeMaxW"};
  rsmon::HistSnapshot s;
  for (size_t i = 0; i < rsmon::kHistCount; ++i) {
    s[i].id = static_cast<uint8_t>(i + 1);
    s[i].name = kNames[i];
    if (i < 3) {
      s[i].nx = static_cast<uint32_t>(2 + i);  // U2 / V3 / W4(面ごとに違う)
      s[i].ny = rsmon::kBuckets;
    } else {
      s[i].nx = rsmon::kChargeBins;
      s[i].ny = 1;
    }
    s[i].bins.assign(static_cast<size_t>(s[i].nx) * s[i].ny, 0.0);
  }
  // 添字は SPEC §5.3: 2D = (strip-1)*512 + bucket
  s[0].bins[(1 - 1) * 512 + 3] = 100.0;   // StripTimeU strip1 bucket3
  s[0].bins[(2 - 1) * 512 + 7] = 4095.0;  // StripTimeU strip2 bucket7
  s[2].bins[(4 - 1) * 512 + 9] = 42.0;    // StripTimeW strip4 bucket9
  s[3].bins[31] = 2.0;                    // ChargeU  bin 31(波高 250 → 250/8)
  s[8].bins[511] = 1.0;                   // ChargeMaxW bin 511(波高 4095)
  return s;
}

void test_monitor_root_holds_the_nine_histograms() {
  const std::string dir = scratch_dir("monitorroot");
  const uint32_t kRun = 31;
  uint64_t bytes_written = 0;
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &fixture_geometry();  // TTree 側は本題ではない
    rootsink::Recorder rec(cfg);
    rootsink::BuiltEvent ev;
    ev.run_number = kRun;
    ev.event_idx = 0;
    rootsink::OwnedFragment f = make_fragment(kRun, 0, 0, 0);
    push_item(f.items, pack_item(0, 1, 0, 5));
    ev.fragments.push_back(std::move(f));
    rec.write(ev, 1000);
    // bytes_written は「書いた ROOT の実バイト数」(status の材料 — SPEC §5.3)
    CHECK(rec.bytes_written() > 0);
    rec.close_run(kRun, 1010);
    rec.write_monitor_root(kRun, make_known_snapshot());
    CHECK(rec.fatal_reason() == nullptr);
    bytes_written = rec.bytes_written();
  }

  const std::string path = dir + "/run0031/run0031_monitor.root";
  CHECK(exists(path));
  // TTree 側の run ファイルも無傷(モニタ書き出しが保存側を壊していない)
  CHECK(exists(dir + "/run0031/run0031.root"));
  CHECK_EQ(bytes_written, rootsink::file_size_bytes(dir + "/run0031/run0031.root"));

  TFile* in = TFile::Open(path.c_str(), "READ");
  CHECK(in != nullptr && !in->IsZombie());
  if (in == nullptr || in->IsZombie()) {
    delete in;
    remove_tree(dir);
    return;
  }

  // --- 9 枚が名前どおりに在る(SPEC §5.2 の表)---
  static const char* kNames[9] = {"StripTimeU", "StripTimeV", "StripTimeW",
                                  "ChargeU",    "ChargeV",    "ChargeW",
                                  "ChargeMaxU", "ChargeMaxV", "ChargeMaxW"};
  for (int i = 0; i < 9; ++i) CHECK(in->Get(kNames[i]) != nullptr);

  // --- 2D: ビン数・軸レンジ・ビン値 ---
  TH2D* stu = dynamic_cast<TH2D*>(in->Get("StripTimeU"));
  CHECK(stu != nullptr);
  if (stu != nullptr) {
    CHECK_EQ(stu->GetNbinsX(), 2);    // Nstrip(ジオメトリ由来)
    CHECK_EQ(stu->GetNbinsY(), 512);  // bucket
    CHECK(stu->GetXaxis()->GetXmin() == 1.0);  // x = strip 1..N+1
    CHECK(stu->GetXaxis()->GetXmax() == 3.0);
    CHECK(stu->GetYaxis()->GetXmin() == 0.0);  // y = bucket 0..512
    CHECK(stu->GetYaxis()->GetXmax() == 512.0);
    // ROOT のビン番号は 1 起点: strip s → x ビン s、bucket b → y ビン b+1
    CHECK(stu->GetBinContent(1, 4) == 100.0);
    CHECK(stu->GetBinContent(2, 8) == 4095.0);
    CHECK(stu->GetBinContent(1, 1) == 0.0);
    CHECK(stu->Integral() == 100.0 + 4095.0);
  }
  TH2D* stw = dynamic_cast<TH2D*>(in->Get("StripTimeW"));
  CHECK(stw != nullptr);
  if (stw != nullptr) {
    CHECK_EQ(stw->GetNbinsX(), 4);
    CHECK(stw->GetXaxis()->GetXmax() == 5.0);
    CHECK(stw->GetBinContent(4, 10) == 42.0);
    CHECK(stw->Integral() == 42.0);
  }
  TH2D* stv = dynamic_cast<TH2D*>(in->Get("StripTimeV"));
  CHECK(stv != nullptr);
  if (stv != nullptr) {
    CHECK_EQ(stv->GetNbinsX(), 3);
    CHECK(stv->Integral() == 0.0);  // 空でも 9 枚とも書く
  }

  // --- 1D: 512 ビン・[0,4096] 固定レンジ(オートレンジ禁止 — SPEC §5.2)---
  TH1D* chu = dynamic_cast<TH1D*>(in->Get("ChargeU"));
  CHECK(chu != nullptr);
  if (chu != nullptr) {
    CHECK_EQ(chu->GetNbinsX(), 512);
    CHECK(chu->GetXaxis()->GetXmin() == 0.0);
    CHECK(chu->GetXaxis()->GetXmax() == 4096.0);
    CHECK(chu->GetBinContent(32) == 2.0);  // 添字 31 → ROOT ビン 32
    CHECK(chu->Integral() == 2.0);
  }
  TH1D* cmw = dynamic_cast<TH1D*>(in->Get("ChargeMaxW"));
  CHECK(cmw != nullptr);
  if (cmw != nullptr) {
    CHECK(cmw->GetBinContent(512) == 1.0);  // 添字 511
    CHECK(cmw->Integral() == 1.0);
  }

  in->Close();
  delete in;
  remove_tree(dir);
}

// 0 イベントの run は monitor.root も作らない(SPEC §6.5 の遅延オープンと同じ理屈)。
void test_zero_event_run_writes_no_monitor_root() {
  const std::string dir = scratch_dir("monitorempty");
  const uint32_t kRun = 32;
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &fixture_geometry();
    rootsink::Recorder rec(cfg);
    rec.close_run(kRun, 1000);  // データが 1 件も来なかった run
    rec.write_monitor_root(kRun, make_known_snapshot());
    CHECK(rec.fatal_reason() == nullptr);
    CHECK_EQ(rec.bytes_written(), 0);
  }
  CHECK(!exists(dir + "/run0032/run0032_monitor.root"));
  CHECK(!exists(dir + "/run0032/run0032.root"));
  remove_tree(dir);
}

// ---------------------------------------------------------------------------
// inspect モード(E2E の entries 照合に使う)
// ---------------------------------------------------------------------------
int inspect(const std::string& path) {
  const std::vector<ReadEntry> entries = read_root_file(path);
  unsigned min_idx = 0, max_idx = 0;
  bool nondecreasing = true;
  for (size_t i = 0; i < entries.size(); ++i) {
    if (i == 0 || entries[i].event_id < min_idx) min_idx = entries[i].event_id;
    if (i == 0 || entries[i].event_id > max_idx) max_idx = entries[i].event_id;
    if (i > 0 && entries[i].event_id < entries[i - 1].event_id) nondecreasing = false;
  }
  std::printf("entries=%zu event_id=[%u,%u] nondecreasing=%d\n", entries.size(), min_idx,
              max_idx, nondecreasing ? 1 : 0);
  if (!entries.empty()) {
    std::printf("first: event_id=%u timestamp=%lu run_id=%ld\n", entries.front().event_id,
                entries.front().timestamp, entries.front().run_id);
  }
  return entries.empty() ? 1 : 0;
}

}  // namespace

int main(int argc, char** argv) {
  gROOT->SetBatch(kTRUE);
  if (argc > 1) return inspect(argv[1]);

  test_shutdown_without_eos_keeps_the_inprogress_name();
  test_mixed_run_defense_keeps_the_old_run_inprogress();
  test_write_triggers_autosave_without_tick();
  test_rollover_splits_the_run_into_numbered_parts();
  test_out_of_range_channel_is_counted_not_silently_dropped();
  test_default_compression_is_101_zlib1();
  test_explicit_compression_setting_is_honored();
  test_monitor_root_holds_the_nine_histograms();
  test_zero_event_run_writes_no_monitor_root();
  return tpccheck::report("test_recorder");
}
