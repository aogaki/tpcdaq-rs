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
#include <sys/wait.h>

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
// 10. P1 並列 Recorder(TODO/064)—— worker 毎 TTree
// ---------------------------------------------------------------------------
//
// 骨子(発注書 064 §1〜7): ビルダは単一のまま、**組み上がったイベントを round-robin で
// N worker へ**分配する。worker は自分の Filler + TFile/TTree を専有し、fill + Fill を
// 丸ごと並列に回す。**N=1 は現行と完全同一**(コードパスも出力名も)—— それを証明する
// のは上の 1〜9 群(無改変で green のまま)であり、ここは N>1 の性質だけを見る。

// P1 群が使い回す合成イベント。**event_idx で全部の値がずれる**ので、worker 間の
// 取り違え(どの worker がどのイベントを書いたか)が値で見える。
//
// **bucket は 100**(既定の signal 窓 5..506 の内側 — pevent_fill.hpp)。窓の外
// (このファイルの他のライフサイクル試験が使う bucket 2 など)だと item が落ちて
// **chargeMap が空**になり、compare_pevent 系の照合が「両側とも空 = 一致」で
// 素通りする(2026-08-17 に 10i のネガティブコントロールが捕獲)。
constexpr uint32_t kSignalBucket = 100;

rootsink::BuiltEvent make_event(uint32_t run, uint32_t event_idx) {
  rootsink::BuiltEvent ev;
  ev.run_number = run;
  ev.event_idx = event_idx;
  rootsink::OwnedFragment f = make_fragment(run, event_idx, 0, 0);
  push_item(f.items, pack_item(0, 1, kSignalBucket, 3 + event_idx));
  ev.fragments.push_back(std::move(f));
  return ev;
}

// ある .root の event_id 集合(順不同の照合用)。
std::vector<unsigned> event_ids_of(const std::string& path) {
  std::vector<unsigned> ids;
  for (const ReadEntry& e : read_root_file(path)) ids.push_back(e.event_id);
  return ids;
}

// 10a. round-robin で worker 毎の TTree に分かれる(骨子 §1/§3)
//
// 手計算の出典: 分配は write() の呼び出し順に worker 0,1,0,1,… なので、
// 6 イベント(event_idx 0..5)を N=2 に流すと
//   worker 0 = {0, 2, 4} / worker 1 = {1, 3, 5}
// worker 内は eventIdx 単調(round-robin なので自然に成立 — 骨子 §3)。
void test_round_robin_splits_events_into_per_worker_trees() {
  const std::string dir = scratch_dir("p1split");
  const uint32_t kRun = 40;
  uint64_t entries_written = 0;
  std::vector<rootsink::RootFileRecord> files;
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &fixture_geometry();
    cfg.workers = 2;
    rootsink::Recorder rec(cfg);
    for (uint32_t i = 0; i < 6; ++i) rec.write(make_event(kRun, i), 1000 + i);
    rec.close_run(kRun, 2000);
    CHECK(rec.fatal_reason() == nullptr);
    entries_written = rec.entries_written();
    files = rec.files_snapshot();
  }
  const std::string run_dir = dir + "/run0040";
  CHECK_EQ(entries_written, 6);
  // N>1 では **`_w{k}` 付きの名前だけ**(素の run0040.root は作らない)。
  CHECK(!exists(run_dir + "/run0040.root"));
  CHECK(exists(run_dir + "/run0040_w0.root"));
  CHECK(exists(run_dir + "/run0040_w1.root"));
  CHECK_EQ(list_prefixed(run_dir, "run_inprogress_").size(), 0);  // 全部 finalize 済み

  const std::vector<unsigned> w0 = event_ids_of(run_dir + "/run0040_w0.root");
  const std::vector<unsigned> w1 = event_ids_of(run_dir + "/run0040_w1.root");
  CHECK_EQ(w0.size(), 3);
  CHECK_EQ(w1.size(), 3);
  if (w0.size() == 3) {
    CHECK_EQ(w0[0], 0);
    CHECK_EQ(w0[1], 2);
    CHECK_EQ(w0[2], 4);
  }
  if (w1.size() == 3) {
    CHECK_EQ(w1[0], 1);
    CHECK_EQ(w1[1], 3);
    CHECK_EQ(w1[2], 5);
  }
  // ④終了 JSON の root_files 台帳に**全パート**が載る(移送チェックリスト)。
  CHECK_EQ(files.size(), 2);
  uint64_t ledger_entries = 0;
  for (const rootsink::RootFileRecord& r : files) {
    ledger_entries += r.entries;
    CHECK(r.path.find("_w") != std::string::npos);
    CHECK(exists(r.path));
  }
  CHECK_EQ(ledger_entries, 6);
  remove_tree(dir);
}

// 10b. ロールオーバは worker 毎に独立して番号を進める(骨子 §3)
//
// 手計算の出典: `max_root_bytes = 1` = 事実上「毎エントリでロールオーバ」。
// 4 イベントを N=2 に流すと各 worker が 2 イベント = 2 パート:
//   worker 0 -> run0041_w0.root, run0041_w0_0001.root
//   worker 1 -> run0041_w1.root, run0041_w1_0001.root
void test_rollover_numbers_parts_per_worker() {
  const std::string dir = scratch_dir("p1rollover");
  const uint32_t kRun = 41;
  std::vector<rootsink::RootFileRecord> files;
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &fixture_geometry();
    cfg.workers = 2;
    cfg.max_root_bytes = 1;
    rootsink::Recorder rec(cfg);
    for (uint32_t i = 0; i < 4; ++i) rec.write(make_event(kRun, i), 1000 + i);
    rec.close_run(kRun, 2000);
    CHECK(rec.fatal_reason() == nullptr);
    files = rec.files_snapshot();
  }
  const std::string run_dir = dir + "/run0041";
  static const char* kExpect[4] = {"run0041_w0.root", "run0041_w0_0001.root",
                                   "run0041_w1.root", "run0041_w1_0001.root"};
  for (const char* name : kExpect) CHECK(exists(run_dir + "/" + name));
  CHECK_EQ(list_prefixed(run_dir, "run_inprogress_").size(), 0);
  CHECK_EQ(files.size(), 4);
  // worker 0 は {0,2}、worker 1 は {1,3} を 1 イベント/パートで持つ。
  CHECK_EQ(event_ids_of(run_dir + "/run0041_w0.root").size(), 1);
  if (!event_ids_of(run_dir + "/run0041_w0.root").empty()) {
    CHECK_EQ(event_ids_of(run_dir + "/run0041_w0.root")[0], 0);
    CHECK_EQ(event_ids_of(run_dir + "/run0041_w0_0001.root")[0], 2);
    CHECK_EQ(event_ids_of(run_dir + "/run0041_w1.root")[0], 1);
    CHECK_EQ(event_ids_of(run_dir + "/run0041_w1_0001.root")[0], 3);
  }
  remove_tree(dir);
}

// 10c. カウンタは全 worker 合算(骨子 §4 / 移送チェックリスト①)
//
// 手計算の出典: item の chan は 7bit(0–127)だが AGET は 68 ch。
// 各イベントに `chan = 100` の item を **1 個**混ぜるので、6 イベントで
// items_out_of_range = 6。N=3 なら worker 毎の Filler は 2 ずつ数え、合算で 6。
void test_counters_are_summed_across_workers() {
  const std::string dir = scratch_dir("p1counters");
  const uint32_t kRun = 42;
  uint64_t items_oor = 0;
  uint64_t entries = 0;
  uint64_t bytes = 0;
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &fixture_geometry();
    cfg.workers = 3;
    rootsink::Recorder rec(cfg);
    for (uint32_t i = 0; i < 6; ++i) {
      rootsink::BuiltEvent ev = make_event(kRun, i);
      push_item(ev.fragments[0].items, pack_item(0, 100, 2, 7));  // chan 100 >= 68
      rec.write(std::move(ev), 1000 + i);
    }
    rec.close_run(kRun, 2000);
    CHECK(rec.fatal_reason() == nullptr);
    items_oor = rec.items_out_of_range();
    entries = rec.entries_written();
    bytes = rec.bytes_written();
  }
  CHECK_EQ(items_oor, 6);
  CHECK_EQ(entries, 6);
  // bytes_written は全 worker の実ファイルサイズ合計。
  const std::string run_dir = dir + "/run0042";
  uint64_t on_disk = 0;
  for (const std::string& name : rootsink::list_directory(run_dir)) {
    on_disk += rootsink::file_size_bytes(run_dir + "/" + name);
  }
  CHECK_EQ(bytes, on_disk);
  remove_tree(dir);
}

// 10d. 重複 eventId の判定は**分配前に一元化**(移送チェックリスト②)
//
// worker に散らした後では「以前に書いた番号」が worker 毎にしか見えないので、
// dispatcher 側で 1 か所だけ判定する。
// 手計算の出典: 5, 6 を書いた後の 3 は `3 <= last(6)` なので弾かれる。
// N=2 でも duplicate_event_ids = 1 / entries_written = 2。
void test_duplicate_event_id_is_rejected_before_distribution() {
  const std::string dir = scratch_dir("p1dup");
  const uint32_t kRun = 43;
  uint64_t dups = 0;
  uint64_t entries = 0;
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &fixture_geometry();
    cfg.workers = 2;
    rootsink::Recorder rec(cfg);
    rec.write(make_event(kRun, 5), 1000);
    rec.write(make_event(kRun, 6), 1001);
    rec.write(make_event(kRun, 3), 1002);  // 遅延・重複 —— 書かずに数える
    rec.close_run(kRun, 2000);
    dups = rec.duplicate_event_ids();
    entries = rec.entries_written();
  }
  CHECK_EQ(dups, 1);
  CHECK_EQ(entries, 2);
  remove_tree(dir);
}

// 10e. EOS 無しの停止でも**キュー在庫は書き切る**、ただし finalize しない(骨子 §6)
//
// 保存系はロスレス —— worker のキューに積んだイベントを停止で捨てない。
// 一方で run は閉じていないので、パートは inprogress のまま(SPEC §6.5 の意味論不変)。
void test_stop_without_eos_flushes_workers_but_keeps_inprogress() {
  const std::string dir = scratch_dir("p1stop");
  const uint32_t kRun = 44;
  std::vector<rootsink::RootFileRecord> files;
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &fixture_geometry();
    cfg.workers = 2;
    rootsink::Recorder rec(cfg);
    for (uint32_t i = 0; i < 4; ++i) rec.write(make_event(kRun, i), 1000 + i);
    rec.shutdown();  // EOS 無しの停止
    CHECK(rec.fatal_reason() == nullptr);
    CHECK_EQ(rec.entries_written(), 4);  // 在庫は捨てずに書き切った
    files = rec.files_snapshot();
  }
  const std::string run_dir = dir + "/run0044";
  CHECK(!exists(run_dir + "/run0044_w0.root"));  // 完成 run に化けない
  CHECK(!exists(run_dir + "/run0044_w1.root"));
  const std::vector<std::string> left = list_prefixed(run_dir, "run_inprogress_");
  CHECK_EQ(left.size(), 2);  // worker 毎に 1 本
  CHECK_EQ(files.size(), 2);
  uint64_t total = 0;
  for (const rootsink::RootFileRecord& r : files) {
    total += r.entries;
    CHECK(r.path.find("run_inprogress_") != std::string::npos);
  }
  CHECK_EQ(total, 4);
  // 中身も読める(4 イベントすべてが 2 本のどちらかに在る)。
  size_t read_back = 0;
  for (const std::string& name : left) read_back += event_ids_of(run_dir + "/" + name).size();
  CHECK_EQ(read_back, 4);
  remove_tree(dir);
}

// 10f. `run{N}_monitor.root` は N>1 でも **1 つだけ**(移送チェックリスト③)
void test_monitor_root_is_single_with_parallel_workers() {
  const std::string dir = scratch_dir("p1monitor");
  const uint32_t kRun = 45;
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &fixture_geometry();
    cfg.workers = 4;
    rootsink::Recorder rec(cfg);
    for (uint32_t i = 0; i < 8; ++i) rec.write(make_event(kRun, i), 1000 + i);
    rec.close_run(kRun, 2000);
    rec.write_monitor_root(kRun, make_known_snapshot());
    CHECK(rec.fatal_reason() == nullptr);
  }
  const std::string run_dir = dir + "/run0045";
  CHECK(exists(run_dir + "/run0045_monitor.root"));
  CHECK_EQ(list_prefixed(run_dir, "run0045_monitor").size(), 1);
  // 8 イベントが 4 worker に 2 つずつ。
  for (int k = 0; k < 4; ++k) {
    char name[64];
    std::snprintf(name, sizeof(name), "/run0045_w%d.root", k);
    CHECK_EQ(event_ids_of(run_dir + name).size(), 2);
  }
  remove_tree(dir);
}

// 10g. 新しい run で part 番号は worker 毎に 0 へ戻る(骨子 §6 / §3)
void test_new_run_resets_part_numbering_in_every_worker() {
  const std::string dir = scratch_dir("p1tworuns");
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &fixture_geometry();
    cfg.workers = 2;
    rootsink::Recorder rec(cfg);
    for (uint32_t i = 0; i < 2; ++i) rec.write(make_event(46, i), 1000 + i);
    rec.close_run(46, 1100);
    for (uint32_t i = 0; i < 2; ++i) rec.write(make_event(47, i), 2000 + i);
    rec.close_run(47, 2100);
    CHECK(rec.fatal_reason() == nullptr);
    CHECK_EQ(rec.entries_written(), 4);
  }
  CHECK(exists(dir + "/run0046/run0046_w0.root"));
  CHECK(exists(dir + "/run0046/run0046_w1.root"));
  CHECK(exists(dir + "/run0047/run0047_w0.root"));   // part 0 に戻っている
  CHECK(exists(dir + "/run0047/run0047_w1.root"));
  CHECK(!exists(dir + "/run0047/run0047_w0_0001.root"));
  remove_tree(dir);
}

// 10h. **全パートのユニオン = N=1 の 1 本**(受け入れ 4 の仕組みを合成データで先に閉じる)
//
// compare_pevent を複数ファイル(カンマ区切り)対応に拡張した —— その拡張自体の
// テスト。同じ 6 イベントを N=1 と N=2 で書き、`w0,w1` の**ユニオン**が N=1 の 1 本と
// 全イベント全 key 一致することを compare_pevent 自身に判定させる。
// **runId は固定**(--run-id 相当の run_id_override)—— 既定は「run を開いた時刻」なので、
// 2 回の Recorder で値が変わると EventInfo 差分になる。
void test_compare_pevent_matches_the_union_of_worker_parts() {
  const char* env_bin = std::getenv("TPCDAQ_COMPARE_PEVENT");
  const std::string cmp_bin = (env_bin != nullptr && env_bin[0] != '\0')
                                  ? std::string(env_bin)
                                  : std::string("./compare_pevent");
  if (!exists(cmp_bin)) {
    std::printf("SKIP compare_pevent union test: %s not built (make compare)\n",
                cmp_bin.c_str());
    return;
  }
  const std::string dir = scratch_dir("p1union");
  const uint32_t kRun = 48;
  const long kRunId = 20260817123456L;  // 固定 runId(2 回の Recorder で同じ値にする)
  for (int workers : {1, 2}) {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir + "/n" + std::to_string(workers);
    cfg.geometry = &fixture_geometry();
    cfg.run_id_override = kRunId;
    cfg.workers = workers;
    rootsink::Recorder rec(cfg);
    for (uint32_t i = 0; i < 6; ++i) rec.write(make_event(kRun, i), 1000 + i);
    rec.close_run(kRun, 2000);
    CHECK(rec.fatal_reason() == nullptr);
  }
  const std::string single = dir + "/n1/run0048/run0048.root";
  const std::string parts =
      dir + "/n2/run0048/run0048_w0.root," + dir + "/n2/run0048/run0048_w1.root";
  CHECK(exists(single));
  const std::string cmd = cmp_bin + " '" + parts + "' '" + single + "'";
  const int rc = std::system(cmd.c_str());
  CHECK_EQ(WEXITSTATUS(rc), 0);  // 0 = 完全一致
  remove_tree(dir);
}

// 10i. **ネガティブコントロール**: 中身が違えば compare_pevent は必ず落ちる
//
// 10h(ユニオン一致)だけだと、比較器が **chargeMap を 1 つも読んでいなくても**
// 「0 differences」で緑になってしまう(両側とも空 map = 一致)。TChain 化(複数ファイル
// 対応)でブランチ有効化を壊していないことを、**差分が出ることの側**から固定する。
//
// 手計算の出典: 同じ 6 イベントを書くが、片方だけ最後のイベント(event_idx 5)の
// ADC を 1 だけずらす(`3 + 5 = 8` → `9`)。chargeMap の値が 1 つ違うので
// compare_pevent は **exit 1**(差分あり)でなければならない。
void test_compare_pevent_detects_a_one_adc_difference() {
  const char* env_bin = std::getenv("TPCDAQ_COMPARE_PEVENT");
  const std::string cmp_bin = (env_bin != nullptr && env_bin[0] != '\0')
                                  ? std::string(env_bin)
                                  : std::string("./compare_pevent");
  if (!exists(cmp_bin)) {
    std::printf("SKIP compare_pevent negative control: %s not built (make compare)\n",
                cmp_bin.c_str());
    return;
  }
  const std::string dir = scratch_dir("p1negctl");
  const uint32_t kRun = 49;
  const long kRunId = 20260817123456L;
  for (int variant : {0, 1}) {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir + "/v" + std::to_string(variant);
    cfg.geometry = &fixture_geometry();
    cfg.run_id_override = kRunId;
    cfg.workers = 2;  // ユニオン側でも検出できること
    rootsink::Recorder rec(cfg);
    for (uint32_t i = 0; i < 6; ++i) {
      rootsink::BuiltEvent ev = make_event(kRun, i);
      if (variant == 1 && i == 5) {
        // 最後のイベントの ADC を 1 だけ増やす(pack_item の adc = 3 + i = 8 -> 9)
        ev.fragments[0].items.clear();
        push_item(ev.fragments[0].items, pack_item(0, 1, kSignalBucket, 3 + i + 1));
      }
      rec.write(std::move(ev), 1000 + i);
    }
    rec.close_run(kRun, 2000);
    CHECK(rec.fatal_reason() == nullptr);
  }
  const std::string a = dir + "/v0/run0049/run0049_w0.root," + dir + "/v0/run0049/run0049_w1.root";
  const std::string b = dir + "/v1/run0049/run0049_w0.root," + dir + "/v1/run0049/run0049_w1.root";
  const int rc = std::system((cmp_bin + " '" + a + "' '" + b + "'").c_str());
  CHECK_EQ(WEXITSTATUS(rc), 1);  // 1 = 差分あり(0 なら比較器が中身を読んでいない)
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
  // --- P1 並列 Recorder(TODO/064)---
  test_round_robin_splits_events_into_per_worker_trees();
  test_rollover_numbers_parts_per_worker();
  test_counters_are_summed_across_workers();
  test_duplicate_event_id_is_rejected_before_distribution();
  test_stop_without_eos_flushes_workers_but_keeps_inprogress();
  test_monitor_root_is_single_with_parallel_workers();
  test_new_run_resets_part_numbering_in_every_worker();
  test_compare_pevent_matches_the_union_of_worker_parts();
  test_compare_pevent_detects_a_one_adc_difference();
  return tpccheck::report("test_recorder");
}
