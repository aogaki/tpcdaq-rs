// test_recorder.cxx — Recorder(GDataFrame TTree 書き出し)の単体テスト(TODO/011)。
//
// **`make test` からは外してある**(`make test-root`)。tpc_wire / rs_core / eb_core /
// conformance の 4 本は ROOT 非依存のまま —— ヘッダテストに ROOT を混ぜない(発注書 §3)。
//
// 試験方式は既存 3 本と同じ「素の CHECK + main」(check.hpp)。やっていることは 1 つ:
//
//   合成 BuiltEvent を書く → **同プロセスで TFile を開き直して読み戻し**、
//   エントリ数・ヘッダ全フィールド・チャンネル・サンプル値・bucket 順を機械照合する。
//
// 「書けた」ではなく「**標準の ROOT リーダで読める形で書けた**」を確かめるのが目的
// (graw2root 互換 = SPEC §6.4 の存在理由)。
//
// 引数を 1 つ与えると **inspect モード**: その .root を読み戻して要約を印字する。
// E2E(graw_replay → receiver → decoder → root_sink)の entries=108 照合はこれを使う
// (発注書 §4「test_recorder 側の読み戻しユーティリティ」)。
//
//   ./test_recorder                      # 単体テスト
//   ./test_recorder /path/run0000.root   # 読み戻して要約(entries=... を印字)
//
// **GET クラスの地雷**(third_party/get は無改変なので回避するしかない):
// `GDataFrame` の TClonesArray は **static 共有**(fgChannels/fgSamples)で、
// `~GDataFrame()` がそれを **delete する**。同時に 2 個生かしてはいけない ——
// 各テストは Recorder を畳んでから読み戻す(読み手の GDataFrame は TTree が所有する)。

#include <sys/stat.h>

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <utility>
#include <vector>

#include <TClonesArray.h>
#include <TFile.h>
#include <TROOT.h>
#include <TRefArray.h>
#include <TTree.h>

#include "GDataChannel.h"
#include "GDataFrame.h"
#include "GDataSample.h"
#include "GFrameHeader.h"
#include "check.hpp"
#include "eb_core.hpp"
#include "root_recorder.hpp"

namespace {

// ---------------------------------------------------------------------------
// 読み戻しユーティリティ(inspect モードと全テストが共有)
// ---------------------------------------------------------------------------

struct ReadSample {
  int bucket = 0;
  int adc = 0;
};

struct ReadChannel {
  int aget = 0;
  int chan = 0;
  std::vector<ReadSample> samples;
};

// TTree の 1 エントリ = 1 CoBo フレーム(SPEC §6.4)を素の C++ に落としたもの。
struct ReadFrame {
  unsigned data_source = 0;
  unsigned revision = 0;
  unsigned long event_time = 0;
  unsigned event_idx = 0;
  unsigned cobo = 0;
  unsigned asad = 0;
  unsigned read_offset = 0;
  unsigned status = 0;
  unsigned window_out = 0;
  unsigned mult[4] = {0, 0, 0, 0};
  unsigned last_cell[4] = {0, 0, 0, 0};
  int nchannels_field = 0;  // GDataFrame::GetNchannels()(TClonesArray の実数と突き合わせる)
  std::vector<ReadChannel> channels;
};

// `path` の "tree" を読み切る。開けなければ空を返す(呼び手が CHECK で落ちる)。
std::vector<ReadFrame> read_root_file(const std::string& path) {
  std::vector<ReadFrame> out;
  TFile* in = TFile::Open(path.c_str(), "READ");
  if (in == nullptr || in->IsZombie()) {
    std::printf("read_root_file: cannot open %s\n", path.c_str());
    delete in;
    return out;
  }
  TTree* tree = dynamic_cast<TTree*>(in->Get("tree"));
  if (tree == nullptr) {
    std::printf("read_root_file: no TTree named \"tree\" in %s\n", path.c_str());
    in->Close();
    delete in;
    return out;
  }
  GET::GDataFrame* frame = nullptr;
  tree->SetBranchAddress("GDataFrame", &frame);
  const Long64_t entries = tree->GetEntries();
  for (Long64_t i = 0; i < entries; ++i) {
    tree->GetEntry(i);
    ReadFrame rf;
    const GET::GFrameHeader& h = frame->fHeader;
    rf.data_source = h.fDataSource;
    rf.revision = h.fRevision;
    rf.event_time = h.fEventTime;
    rf.event_idx = h.fEventIdx;
    rf.cobo = h.fCoboIdx;
    rf.asad = h.fAsadIdx;
    rf.read_offset = h.fReadOffset;
    rf.status = h.fStatus;
    rf.window_out = h.fWindowOut;
    for (int k = 0; k < 4; ++k) {
      rf.mult[k] = h.fMult[k];
      rf.last_cell[k] = h.fLastCellIdx[k];
    }
    rf.nchannels_field = frame->GetNchannels();
    TClonesArray* chans = frame->GetChannels();
    const int nchan = (chans == nullptr) ? 0 : static_cast<int>(chans->GetEntriesFast());
    for (int c = 0; c < nchan; ++c) {
      GET::GDataChannel* ch = static_cast<GET::GDataChannel*>(chans->At(c));
      if (ch == nullptr) continue;
      ReadChannel rc;
      rc.aget = ch->fAgetIdx;
      rc.chan = ch->fChanIdx;
      TRefArray& hits = ch->GetHits();
      for (int s = 0; s < ch->GetNhit(); ++s) {
        GET::GDataSample* smp = static_cast<GET::GDataSample*>(hits.At(s));
        if (smp == nullptr) continue;
        rc.samples.push_back({static_cast<int>(smp->fBuckIdx), static_cast<int>(smp->fValue)});
      }
      rf.channels.push_back(std::move(rc));
    }
    out.push_back(std::move(rf));
  }
  in->Close();
  delete in;
  return out;
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
// 1. 1 エントリ = 1 フラグメント + 全フィールド読み戻し照合
// ---------------------------------------------------------------------------
//
// 手計算の出典(このテストが仕様書):
//
//   event 41: フラグメント (cobo0,asad0) と (cobo1,asad1)
//   event 42: フラグメント (cobo0,asad1) と (cobo1,asad0)
//   → **4 エントリ**(1 エントリ = 1 CoBo フレーム、SPEC §6.4)。
//
//   最初のフラグメントの items は **わざと** (aget,chan) 混在・bucket 逆順で入れる:
//     (aget1,chan5,  bucket3, adc100)
//     (aget0,chan67, bucket1, adc200)
//     (aget1,chan5,  bucket1, adc101)
//     (aget0,chan67, bucket0, adc201)
//     (aget1,chan5,  bucket2, adc102)
//   → チャンネルは (aget,chan) 昇順で 2 本 = (0,67) と (1,5)。
//     (0,67) は bucket 0,1     = adc 201,200(**bucket 昇順**、SPEC §6.4)
//     (1,5)  は bucket 1,2,3   = adc 101,102,100
//     サンプル合計 5。
void test_writes_one_entry_per_fragment_and_reads_back() {
  const std::string dir = scratch_dir("roundtrip");
  const uint32_t kRun = 7;
  std::vector<rootsink::RootFileRecord> files;
  uint64_t entries_written = 0;

  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    rootsink::Recorder rec(cfg);

    rootsink::BuiltEvent ev41;
    ev41.run_number = kRun;
    ev41.event_idx = 41;
    rootsink::OwnedFragment f00 = make_fragment(kRun, 41, 0, 0);
    push_item(f00.items, pack_item(1, 5, 3, 100));
    push_item(f00.items, pack_item(0, 67, 1, 200));
    push_item(f00.items, pack_item(1, 5, 1, 101));
    push_item(f00.items, pack_item(0, 67, 0, 201));
    push_item(f00.items, pack_item(1, 5, 2, 102));
    ev41.fragments.push_back(std::move(f00));
    rootsink::OwnedFragment f11 = make_fragment(kRun, 41, 1, 1);
    push_item(f11.items, pack_item(3, 0, 9, 3000));
    ev41.fragments.push_back(std::move(f11));

    rootsink::BuiltEvent ev42;
    ev42.run_number = kRun;
    ev42.event_idx = 42;
    rootsink::OwnedFragment f01 = make_fragment(kRun, 42, 0, 1);
    push_item(f01.items, pack_item(2, 33, 5, 777));
    ev42.fragments.push_back(std::move(f01));
    rootsink::OwnedFragment f10 = make_fragment(kRun, 42, 1, 0);
    push_item(f10.items, pack_item(0, 0, 0, 1));
    push_item(f10.items, pack_item(0, 0, 1, 2));
    ev42.fragments.push_back(std::move(f10));

    rec.write(ev41, /*now_ms=*/1000);
    // run 途中では inprogress のまま(完全 run に化けない、SPEC §6.5)
    CHECK(rec.is_open());
    CHECK(!exists(dir + "/run0007/run0007.root"));
    CHECK(list_prefixed(dir + "/run0007", "run_inprogress_").size() == 1);

    rec.write(ev42, /*now_ms=*/1010);
    rec.close_run(kRun, /*now_ms=*/1020);
    CHECK(rec.fatal_reason() == nullptr);
    CHECK(!rec.is_open());
    files = rec.files_snapshot();
    entries_written = rec.entries_written();
    CHECK_EQ(rec.items_out_of_range(), 0);
  }  // Recorder を畳んでから読み戻す(static TClonesArray の共有 —— 冒頭の注意)

  // --- ファイルライフサイクル(SPEC §6.5)---
  const std::string run_dir = dir + "/run0007";
  CHECK_EQ(files.size(), 1);
  CHECK_EQ(entries_written, 4);
  if (files.size() == 1) {
    CHECK(files[0].path == run_dir + "/run0007.root");
    CHECK_EQ(files[0].entries, 4);
    CHECK(files[0].bytes > 0);
  }
  CHECK(exists(run_dir + "/run0007.root"));
  CHECK_EQ(list_prefixed(run_dir, "run_inprogress_").size(), 0);  // rename 済み

  // --- 読み戻し(標準の ROOT リーダと同じ手順)---
  const std::vector<ReadFrame> frames = read_root_file(run_dir + "/run0007.root");
  CHECK_EQ(frames.size(), 4);
  if (frames.size() != 4) return;

  // エントリ順 = イベント昇順 × イベント内 (cobo,asad) 昇順(SPEC §6.3)
  const unsigned expect_idx[4] = {41, 41, 42, 42};
  const unsigned expect_cobo[4] = {0, 1, 0, 1};
  const unsigned expect_asad[4] = {0, 1, 1, 0};
  for (int i = 0; i < 4; ++i) {
    const ReadFrame& f = frames[i];
    CHECK_EQ(f.event_idx, expect_idx[i]);
    CHECK_EQ(f.cobo, expect_cobo[i]);
    CHECK_EQ(f.asad, expect_asad[i]);
    // ヘッダ全フィールド(make_fragment の式と一致)
    CHECK_EQ(f.revision, 5);
    CHECK_EQ(f.read_offset, 136 + expect_cobo[i]);
    CHECK_EQ(f.status, 3 + expect_asad[i]);
    CHECK_EQ(f.event_time, 0x0000'1234'5678'9abcULL + 0x100ULL * expect_idx[i] +
                               expect_cobo[i] * 16 + expect_asad[i]);
    CHECK_EQ(f.window_out, 512 + expect_idx[i]);
    CHECK_EQ(f.mult[0], 68);
    CHECK_EQ(f.mult[1], 0);
    CHECK_EQ(f.mult[2], 17);
    CHECK_EQ(f.mult[3], 3 + expect_idx[i]);
    CHECK_EQ(f.last_cell[0], 7);
    CHECK_EQ(f.last_cell[1], 9);
    CHECK_EQ(f.last_cell[2], 11);
    CHECK_EQ(f.last_cell[3], 13 + expect_cobo[i]);
    // Fragment(§2.4)は dataSource を運ばない —— C++ 版 RootWriter と同じく 0
    CHECK_EQ(f.data_source, 0);
    CHECK_EQ(f.nchannels_field, static_cast<int>(f.channels.size()));
  }

  // --- エントリ 0: チャンネル分解と bucket 昇順(このユニットの核心)---
  const ReadFrame& e0 = frames[0];
  CHECK_EQ(e0.channels.size(), 2);
  if (e0.channels.size() == 2) {
    CHECK_EQ(e0.channels[0].aget, 0);
    CHECK_EQ(e0.channels[0].chan, 67);
    CHECK_EQ(e0.channels[0].samples.size(), 2);
    if (e0.channels[0].samples.size() == 2) {
      CHECK_EQ(e0.channels[0].samples[0].bucket, 0);  // 投入は bucket 1 が先 = 並べ替え済み
      CHECK_EQ(e0.channels[0].samples[0].adc, 201);
      CHECK_EQ(e0.channels[0].samples[1].bucket, 1);
      CHECK_EQ(e0.channels[0].samples[1].adc, 200);
    }
    CHECK_EQ(e0.channels[1].aget, 1);
    CHECK_EQ(e0.channels[1].chan, 5);
    CHECK_EQ(e0.channels[1].samples.size(), 3);
    if (e0.channels[1].samples.size() == 3) {
      CHECK_EQ(e0.channels[1].samples[0].bucket, 1);
      CHECK_EQ(e0.channels[1].samples[0].adc, 101);
      CHECK_EQ(e0.channels[1].samples[1].bucket, 2);
      CHECK_EQ(e0.channels[1].samples[1].adc, 102);
      CHECK_EQ(e0.channels[1].samples[2].bucket, 3);
      CHECK_EQ(e0.channels[1].samples[2].adc, 100);
    }
  }

  // --- 残り 3 エントリ(混線していないこと)---
  CHECK_EQ(frames[1].channels.size(), 1);
  if (frames[1].channels.size() == 1) {
    CHECK_EQ(frames[1].channels[0].aget, 3);
    CHECK_EQ(frames[1].channels[0].chan, 0);
    CHECK_EQ(frames[1].channels[0].samples.size(), 1);
    CHECK_EQ(frames[1].channels[0].samples[0].bucket, 9);
    CHECK_EQ(frames[1].channels[0].samples[0].adc, 3000);
  }
  CHECK_EQ(frames[2].channels.size(), 1);
  if (frames[2].channels.size() == 1) {
    CHECK_EQ(frames[2].channels[0].aget, 2);
    CHECK_EQ(frames[2].channels[0].chan, 33);
    CHECK_EQ(frames[2].channels[0].samples.size(), 1);
    CHECK_EQ(frames[2].channels[0].samples[0].bucket, 5);
    CHECK_EQ(frames[2].channels[0].samples[0].adc, 777);
  }
  CHECK_EQ(frames[3].channels.size(), 1);
  if (frames[3].channels.size() == 1) {
    CHECK_EQ(frames[3].channels[0].aget, 0);
    CHECK_EQ(frames[3].channels[0].chan, 0);
    CHECK_EQ(frames[3].channels[0].samples.size(), 2);
    CHECK_EQ(frames[3].channels[0].samples[0].adc, 1);
    CHECK_EQ(frames[3].channels[0].samples[1].adc, 2);
  }

  remove_tree(dir);
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
    const std::vector<ReadFrame> frames = read_root_file(run_dir + "/" + left[0]);
    CHECK_EQ(frames.size(), 1);
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
    const std::vector<ReadFrame> frames = read_root_file(run_dir + "/" + expect_names[i]);
    CHECK_EQ(frames.size(), 1);
    if (frames.size() == 1) {
      CHECK_EQ(frames[0].event_idx, i);
      CHECK_EQ(frames[0].channels.size(), 1);
      if (frames[0].channels.size() == 1 && frames[0].channels[0].samples.size() == 1) {
        CHECK_EQ(frames[0].channels[0].samples[0].adc, 100 + i);
      }
    }
  }
  remove_tree(dir);
}

// ---------------------------------------------------------------------------
// 4. 範囲外の chan は黙って消さない(CLAUDE.md「silent failure を作らない」)
// ---------------------------------------------------------------------------
//
// item の chan フィールドは 7 bit(0–127)だが AGET は 68 ch。68 以上は
// GDataChannel に置き場がないので落とすしかない —— **数えて JSON に出す**。
void test_out_of_range_channel_is_counted_not_silently_dropped() {
  const std::string dir = scratch_dir("range");
  const uint32_t kRun = 3;
  uint64_t out_of_range = 0;
  std::vector<ReadFrame> frames;
  std::string run_dir = dir + "/run0003";
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
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
  CHECK_EQ(out_of_range, 2);
  frames = read_root_file(run_dir + "/run0003.root");
  CHECK_EQ(frames.size(), 1);
  if (frames.size() == 1) {
    CHECK_EQ(frames[0].channels.size(), 1);  // 生き残るのは (2,67) だけ
    if (frames[0].channels.size() == 1) {
      CHECK_EQ(frames[0].channels[0].aget, 2);
      CHECK_EQ(frames[0].channels[0].chan, 67);
      CHECK_EQ(frames[0].channels[0].samples.size(), 1);
      CHECK_EQ(frames[0].channels[0].samples[0].adc, 33);
    }
  }
  remove_tree(dir);
}

// ---------------------------------------------------------------------------
// 5. 遅延到着フラグメント(単一フラグメントの BuiltEvent)も普通に書ける
// ---------------------------------------------------------------------------
//
// SPEC §6.3「emit 後に遅延到着したフラグメントも **必ず TTree に書く**(順序より
// 可逆性優先)」—— root_sink.cxx はこれを 1 フラグメントの BuiltEvent として流す。
// Recorder 側に特別扱いが無い(= 順序が前後するだけで必ず 1 エントリになる)ことを固定する。
void test_a_single_fragment_event_becomes_one_entry() {
  const std::string dir = scratch_dir("late");
  const uint32_t kRun = 5;
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    rootsink::Recorder rec(cfg);
    for (uint32_t idx : {7u, 8u, 6u}) {  // 6 は「遅れて来た」= 昇順ではない
      rootsink::BuiltEvent ev;
      ev.run_number = kRun;
      ev.event_idx = idx;
      rootsink::OwnedFragment f = make_fragment(kRun, idx, 0, 0);
      push_item(f.items, pack_item(0, 1, 0, 40 + idx));
      ev.fragments.push_back(std::move(f));
      rec.write(ev, 1000);
    }
    rec.close_run(kRun, 1100);
  }
  const std::vector<ReadFrame> frames = read_root_file(dir + "/run0005/run0005.root");
  CHECK_EQ(frames.size(), 3);
  if (frames.size() == 3) {
    // 書いた順にそのまま並ぶ(捨てない・並べ替えない)
    CHECK_EQ(frames[0].event_idx, 7);
    CHECK_EQ(frames[1].event_idx, 8);
    CHECK_EQ(frames[2].event_idx, 6);
  }
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
// inspect モード(E2E の entries 照合に使う)
// ---------------------------------------------------------------------------
int inspect(const std::string& path) {
  const std::vector<ReadFrame> frames = read_root_file(path);
  unsigned long long samples = 0;
  unsigned long long channels = 0;
  unsigned min_idx = 0, max_idx = 0;
  for (size_t i = 0; i < frames.size(); ++i) {
    channels += frames[i].channels.size();
    for (const ReadChannel& c : frames[i].channels) samples += c.samples.size();
    if (i == 0 || frames[i].event_idx < min_idx) min_idx = frames[i].event_idx;
    if (i == 0 || frames[i].event_idx > max_idx) max_idx = frames[i].event_idx;
  }
  std::printf("entries=%zu channels=%llu samples=%llu event_idx=[%u,%u]\n", frames.size(),
              channels, samples, min_idx, max_idx);
  if (!frames.empty()) {
    const ReadFrame& f = frames.front();
    std::printf("first: cobo=%u asad=%u event_idx=%u event_time=%lu revision=%u "
                "read_offset=%u status=%u window_out=%u channels=%zu\n",
                f.cobo, f.asad, f.event_idx, f.event_time, f.revision, f.read_offset,
                f.status, f.window_out, f.channels.size());
  }
  return frames.empty() ? 1 : 0;
}

}  // namespace

int main(int argc, char** argv) {
  gROOT->SetBatch(kTRUE);
  if (argc > 1) return inspect(argv[1]);

  test_writes_one_entry_per_fragment_and_reads_back();
  test_shutdown_without_eos_keeps_the_inprogress_name();
  test_rollover_splits_the_run_into_numbered_parts();
  test_out_of_range_channel_is_counted_not_silently_dropped();
  test_a_single_fragment_event_becomes_one_entry();
  test_default_compression_is_101_zlib1();
  test_explicit_compression_setting_is_honored();
  return tpccheck::report("test_recorder");
}
