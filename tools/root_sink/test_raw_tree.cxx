// test_raw_tree.cxx — raw ADC ツリー(SPEC §6.7 v1.28、TODO/075)の単体テスト。
//
// **ROOT だけにリンクする**(TPCReco の辞書・ソースは要らない —— raw ツリーは
// PEventTPC もジオメトリも使わない)。`make test-root` から走る。
//
// やっていること: 合成 BuiltEvent を RawTreeWriter に書かせ、**同プロセスで TFile を
// 開き直して読み戻し**、macros/ の契約(graw2root.C の出力形式)とファイルライフ
// サイクル(SPEC §6.5)を機械照合する。読み戻しは **analyzeUVW.C と同じ書き方**
// (`TTree* tree=(TTree*)in.Get("raw")` + `static Short_t adc[4][68][512]` への
// SetBranchAddress)—— 下流が実際にやることをそのままやる。
//
// 小道具(scratch_dir / remove_tree / pack_item / push_item / make_fragment)は
// test_recorder.cxx からの**コピー**(ヘッダ化はしない —— KISS)。

#include <sys/stat.h>
#include <sys/wait.h>

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <utility>
#include <vector>

#include <TFile.h>
#include <TLeaf.h>
#include <TObjArray.h>
#include <TROOT.h>
#include <TTree.h>

#include "check.hpp"
#include "eb_core.hpp"
#include "raw_tree.hpp"

namespace {

// ---------------------------------------------------------------------------
// テスト用の小道具(test_recorder.cxx からのコピー)
// ---------------------------------------------------------------------------

bool exists(const std::string& p) {
  struct stat st;
  return ::stat(p.c_str(), &st) == 0;
}

// dir 直下で prefix 始まりのものを名前順に返す(inprogress の残骸検出用)。
std::vector<std::string> list_prefixed(const std::string& dir, const std::string& prefix) {
  std::vector<std::string> hits;
  for (const std::string& name : rootsink::list_directory(dir)) {
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
  std::string base =
      (tmp != nullptr && tmp[0] != '\0') ? std::string(tmp) : std::string("/tmp");
  if (!base.empty() && base.back() == '/') base.pop_back();
  char suffix[64];
  std::snprintf(suffix, sizeof(suffix), "/tpcdaq_test_raw_tree_%d_%s",
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

// 1 イベント = 1 フラグメント(mini)を組む。items は呼び手が足す。
rootsink::BuiltEvent make_event(uint32_t run, uint32_t event_idx, uint8_t cobo,
                                uint8_t asad) {
  rootsink::BuiltEvent ev;
  ev.run_number = run;
  ev.event_idx = event_idx;
  ev.fragments.push_back(make_fragment(run, event_idx, cobo, asad));
  return ev;
}

// 外部コマンドの実行(compare_raw の判定に使う)。stdout+stderr を全部拾う。
struct CmdResult {
  int status = -1;
  std::string out;
};

CmdResult run_cmd(const std::string& cmd) {
  CmdResult r;
  FILE* p = ::popen((cmd + " 2>&1").c_str(), "r");
  if (p == nullptr) return r;
  char buf[512];
  while (std::fgets(buf, sizeof(buf), p) != nullptr) r.out += buf;
  const int wait_status = ::pclose(p);  // WEXITSTATUS は lvalue しか取れない
  r.status = WEXITSTATUS(wait_status);
  return r;
}

// ---------------------------------------------------------------------------
// 読み戻し(任意の adc 長。ELITPC 形の 8 ブロックも読む)
// ---------------------------------------------------------------------------

struct RawEntry {
  UInt_t event_id = 0;
  ULong64_t event_time = 0;
  std::vector<Short_t> adc;
};

// `path` の tree "raw" を全部読む。adc_len_out には adc リーフの要素数を返す。
std::vector<RawEntry> read_raw_file(const std::string& path, int* adc_len_out) {
  std::vector<RawEntry> out;
  if (adc_len_out != nullptr) *adc_len_out = 0;
  TFile* in = TFile::Open(path.c_str(), "READ");
  if (in == nullptr || in->IsZombie()) {
    std::printf("read_raw_file: cannot open %s\n", path.c_str());
    delete in;
    return out;
  }
  TTree* tree = dynamic_cast<TTree*>(in->Get(rootsink::kRawTreeName));
  if (tree == nullptr) {
    std::printf("read_raw_file: no TTree named \"%s\" in %s\n", rootsink::kRawTreeName,
                path.c_str());
    in->Close();
    delete in;
    return out;
  }
  TLeaf* adc_leaf = tree->GetLeaf("adc");
  const int adc_len = (adc_leaf == nullptr) ? 0 : adc_leaf->GetLenStatic();
  if (adc_len_out != nullptr) *adc_len_out = adc_len;
  UInt_t event_id = 0;
  ULong64_t event_time = 0;
  std::vector<Short_t> adc(static_cast<std::size_t>(adc_len), 0);
  tree->SetBranchAddress("eventId", &event_id);
  tree->SetBranchAddress("eventTime", &event_time);
  tree->SetBranchAddress("adc", adc.data());
  const Long64_t entries = tree->GetEntries();
  for (Long64_t i = 0; i < entries; ++i) {
    tree->GetEntry(i);
    RawEntry re;
    re.event_id = event_id;
    re.event_time = event_time;
    re.adc = adc;
    out.push_back(std::move(re));
  }
  in->Close();
  delete in;
  return out;
}

// adc 全要素の総和(「置いた値以外は 0」を 1 個の数字で見るため)。
long long adc_sum(const std::vector<Short_t>& adc) {
  long long sum = 0;
  for (const Short_t v : adc) sum += v;
  return sum;
}

// mini 形(4 ブロック)の添字。graw2root.C の adc[aget][chan][bucket] と同じ並び。
std::size_t adc_index(int block_aget, int chan, int bucket) {
  return (static_cast<std::size_t>(block_aget) * 68 + static_cast<std::size_t>(chan)) *
             512 +
         static_cast<std::size_t>(bucket);
}

// ---------------------------------------------------------------------------
// 1. 契約一致(mini, expect={0:0})—— 下流 analyzeUVW.C が読む形そのもの
// ---------------------------------------------------------------------------
//
// 手計算の出典: 置いた item は下の kItems 表。エントリ毎の adc 総和は置いた ADC の
// 合計に一致するはず(= 他の要素は全部 0)。eventTime は make_fragment の式
// `0x0000'1234'5678'9abc + 0x100 * event_idx`(cobo=asad=0)。
void test_mini_tree_matches_the_graw2root_contract() {
  const std::string dir = scratch_dir("contract");
  const uint32_t kRun = 21;
  // {event_idx, aget, chan, bucket, adc}
  struct Item {
    uint32_t event_idx, aget, chan, bucket, adc;
  };
  const std::vector<Item> kItems = {
      {5, 0, 0, 0, 100},     {5, 3, 67, 511, 4095}, {6, 1, 11, 7, 1},
      {6, 2, 5, 256, 2048},  {6, 0, 63, 1, 7},      {7, 2, 22, 100, 333},
  };
  std::string leaflist;
  {
    rootsink::RawTreeConfig cfg;
    cfg.output_root = dir;
    cfg.expect = {{0, 0}};
    rootsink::RawTreeWriter w(cfg);
    leaflist = w.leaflist();
    for (uint32_t idx = 5; idx <= 7; ++idx) {
      rootsink::BuiltEvent ev = make_event(kRun, idx, 0, 0);
      for (const Item& it : kItems) {
        if (it.event_idx != idx) continue;
        push_item(ev.fragments[0].items, pack_item(it.aget, it.chan, it.bucket, it.adc));
      }
      w.write(ev, 1000 + idx);
    }
    w.close_run(kRun);
    CHECK(w.fatal_reason() == nullptr);
    CHECK_EQ(w.entries_written(), 3);
    CHECK_EQ(w.run_entries(), 3);
    CHECK_EQ(w.items_out_of_range(), 0);
    CHECK_EQ(w.unexpected_fragments(), 0);
    const std::vector<rootsink::RootFileRecord> files = w.files_snapshot();
    CHECK_EQ(files.size(), 1);
    if (files.size() == 1) {
      CHECK(files[0].path == dir + "/run0021/run0021_raw.root");
      CHECK_EQ(files[0].entries, 3);
      CHECK(files[0].bytes > 0);
    }
  }
  if (leaflist != "adc[4][68][512]/S") std::printf("leaflist = %s\n", leaflist.c_str());
  CHECK(leaflist == "adc[4][68][512]/S");

  // --- 下流(analyzeUVW.C)と同じ読み方 -------------------------------------
  const std::string path = dir + "/run0021/run0021_raw.root";
  CHECK(exists(path));
  {
    TFile in(path.c_str());
    TTree* tree = dynamic_cast<TTree*>(in.Get("raw"));
    CHECK(tree != nullptr);
    if (tree != nullptr) {
      CHECK_EQ(tree->GetEntries(), 3);
      CHECK_EQ(tree->GetListOfBranches()->GetEntries(), 3);  // eventId / eventTime / adc
      TLeaf* id_leaf = tree->GetLeaf("eventId");
      TLeaf* time_leaf = tree->GetLeaf("eventTime");
      TLeaf* adc_leaf = tree->GetLeaf("adc");
      CHECK(id_leaf != nullptr && time_leaf != nullptr && adc_leaf != nullptr);
      if (id_leaf != nullptr) CHECK(std::string(id_leaf->GetTypeName()) == "UInt_t");
      if (time_leaf != nullptr) {
        CHECK(std::string(time_leaf->GetTypeName()) == "ULong64_t");
      }
      if (adc_leaf != nullptr) {
        CHECK(std::string(adc_leaf->GetTypeName()) == "Short_t");
        CHECK_EQ(adc_leaf->GetLenStatic(), 4 * 68 * 512);
      }

      UInt_t eventId = 0;
      ULong64_t eventTime = 0;
      static Short_t adc[4][68][512];
      tree->SetBranchAddress("eventId", &eventId);
      tree->SetBranchAddress("eventTime", &eventTime);
      tree->SetBranchAddress("adc", adc);
      for (Long64_t i = 0; i < tree->GetEntries(); ++i) {
        tree->GetEntry(i);
        const uint32_t expect_idx = 5 + static_cast<uint32_t>(i);
        CHECK_EQ(eventId, expect_idx);
        // make_fragment の式(cobo=asad=0)。
        CHECK_EQ(eventTime, 0x0000'1234'5678'9abcULL + 0x100ULL * expect_idx);
        long long sum = 0;
        long long want_sum = 0;
        for (int a = 0; a < 4; ++a) {
          for (int c = 0; c < 68; ++c) {
            for (int b = 0; b < 512; ++b) sum += adc[a][c][b];
          }
        }
        for (const Item& it : kItems) {
          if (it.event_idx != expect_idx) continue;
          want_sum += it.adc;
          CHECK_EQ(adc[it.aget][it.chan][it.bucket], it.adc);
        }
        // 置いていない要素は 0(総和が置いた分ちょうどであることで示す)。
        CHECK_EQ(sum, want_sum);
      }
    }
  }
  remove_tree(dir);
}

// ---------------------------------------------------------------------------
// 2. ELITPC 形(expect={0:0,0:1})—— 配列は期待集合から、欠損 AsAd は 0
// ---------------------------------------------------------------------------
void test_two_asad_expectation_sets_the_block_layout() {
  const std::string dir = scratch_dir("elitpc");
  const uint32_t kRun = 22;
  std::string leaflist;
  {
    rootsink::RawTreeConfig cfg;
    cfg.output_root = dir;
    cfg.expect = {{0, 0}, {0, 1}};
    rootsink::RawTreeWriter w(cfg);
    leaflist = w.leaflist();

    // イベント 1: asad1 だけ(incomplete)。ブロック 1 = adc[4..7][..][..]。
    rootsink::BuiltEvent lone = make_event(kRun, 1, 0, 1);
    lone.incomplete = true;
    push_item(lone.fragments[0].items, pack_item(0, 1, 2, 555));
    w.write(lone, 1000);

    // イベント 2: 両方揃う(eb_core が (cobo,asad) 昇順に並べた形で渡す)。
    rootsink::BuiltEvent both;
    both.run_number = kRun;
    both.event_idx = 2;
    both.fragments.push_back(make_fragment(kRun, 2, 0, 0));
    both.fragments.push_back(make_fragment(kRun, 2, 0, 1));
    push_item(both.fragments[0].items, pack_item(3, 10, 20, 111));
    push_item(both.fragments[1].items, pack_item(1, 30, 40, 222));
    w.write(both, 1001);

    w.close_run(kRun);
    CHECK(w.fatal_reason() == nullptr);
  }
  if (leaflist != "adc[8][68][512]/S") std::printf("leaflist = %s\n", leaflist.c_str());
  CHECK(leaflist == "adc[8][68][512]/S");

  int adc_len = 0;
  const std::vector<RawEntry> entries =
      read_raw_file(dir + "/run0022/run0022_raw.root", &adc_len);
  CHECK_EQ(adc_len, 8 * 68 * 512);
  CHECK_EQ(entries.size(), 2);
  if (entries.size() == 2) {
    // --- イベント 1(asad1 のみ)---
    CHECK_EQ(entries[0].event_id, 1);
    // eventTime = 届いた唯一のフラグメント(asad1)のもの: 式 + cobo*16 + asad。
    CHECK_EQ(entries[0].event_time, 0x0000'1234'5678'9abcULL + 0x100ULL * 1 + 1);
    // ブロック 1(= adc[4+aget]) の (aget0, chan1, bucket2) に 555。
    CHECK_EQ(entries[0].adc[adc_index(4 + 0, 1, 2)], 555);
    CHECK_EQ(adc_sum(entries[0].adc), 555);  // ブロック 0 は全部 0
    // --- イベント 2(両方)---
    CHECK_EQ(entries[1].event_id, 2);
    // eventTime = 先頭フラグメント = asad0 のもの(+0)。
    CHECK_EQ(entries[1].event_time, 0x0000'1234'5678'9abcULL + 0x100ULL * 2);
    CHECK_EQ(entries[1].adc[adc_index(0 + 3, 10, 20)], 111);  // asad0 → ブロック 0
    CHECK_EQ(entries[1].adc[adc_index(4 + 1, 30, 40)], 222);  // asad1 → ブロック 1
    CHECK_EQ(adc_sum(entries[1].adc), 333);                   // 111 + 222
  }
  remove_tree(dir);
}

// ---------------------------------------------------------------------------
// 3. ライフサイクル(SPEC §6.5 —— RecorderWorker と同じ規則、名前空間だけ別)
// ---------------------------------------------------------------------------
void test_inprogress_name_until_close_run() {
  const std::string dir = scratch_dir("lifecycle");
  const uint32_t kRun = 23;
  const std::string run_dir = dir + "/run0023";
  {
    rootsink::RawTreeConfig cfg;
    cfg.output_root = dir;
    cfg.expect = {{0, 0}};
    rootsink::RawTreeWriter w(cfg);
    // 0 イベントの run はファイルを作らない(遅延オープン)。
    CHECK(!w.is_open());
    CHECK(!exists(run_dir));

    rootsink::BuiltEvent ev = make_event(kRun, 0, 0, 0);
    push_item(ev.fragments[0].items, pack_item(0, 0, 0, 42));
    w.write(ev, 1000);
    CHECK(w.is_open());
    CHECK_EQ(list_prefixed(run_dir, "run_raw_inprogress_").size(), 1);
    CHECK(!exists(run_dir + "/run0023_raw.root"));
    CHECK(w.provisional().find("/run_raw_inprogress_") != std::string::npos);

    w.close_run(kRun);
    CHECK(!w.is_open());
    CHECK_EQ(list_prefixed(run_dir, "run_raw_inprogress_").size(), 0);
    CHECK(exists(run_dir + "/run0023_raw.root"));
  }
  remove_tree(dir);
}

void test_shutdown_without_eos_keeps_the_inprogress_name() {
  const std::string dir = scratch_dir("noeos");
  const uint32_t kRun = 24;
  const std::string run_dir = dir + "/run0024";
  std::vector<rootsink::RootFileRecord> files;
  {
    rootsink::RawTreeConfig cfg;
    cfg.output_root = dir;
    cfg.expect = {{0, 0}};
    rootsink::RawTreeWriter w(cfg);
    rootsink::BuiltEvent ev = make_event(kRun, 0, 0, 0);
    push_item(ev.fragments[0].items, pack_item(0, 0, 0, 42));
    w.write(ev, 1000);
    w.shutdown();  // EOS 無しの停止
    files = w.files_snapshot();
  }
  // 未完了 run が完全 run に化けない(SPEC §6.5)。
  CHECK(!exists(run_dir + "/run0024_raw.root"));
  CHECK_EQ(list_prefixed(run_dir, "run_raw_inprogress_").size(), 1);
  CHECK_EQ(files.size(), 1);
  if (files.size() == 1) {
    CHECK(files[0].path.find("/run_raw_inprogress_") != std::string::npos);
    CHECK_EQ(files[0].entries, 1);
  }
  remove_tree(dir);
}

void test_zero_event_run_writes_no_file() {
  const std::string dir = scratch_dir("zeroev");
  const uint32_t kRun = 25;
  {
    rootsink::RawTreeConfig cfg;
    cfg.output_root = dir;
    cfg.expect = {{0, 0}};
    rootsink::RawTreeWriter w(cfg);
    w.close_run(kRun);  // 1 件も write しないまま EOS
    CHECK(w.files_snapshot().empty());
    CHECK_EQ(w.entries_written(), 0);
    CHECK(w.fatal_reason() == nullptr);
  }
  CHECK(!exists(dir + "/run0025"));  // run ディレクトリごと作らない
  remove_tree(dir);
}

void test_existing_final_file_is_not_overwritten() {
  const std::string dir = scratch_dir("exists");
  const uint32_t kRun = 26;
  const std::string run_dir = dir + "/run0026";
  const std::string target = run_dir + "/run0026_raw.root";
  CHECK(rootsink::mkdir_p(run_dir));
  {
    // 完成済みファイルの代役(中身は何でもよい —— 上書きされないことを見る)。
    FILE* f = std::fopen(target.c_str(), "wb");
    CHECK(f != nullptr);
    if (f != nullptr) {
      std::fputs("not a root file", f);
      std::fclose(f);
    }
  }
  const uint64_t before = rootsink::file_size_bytes(target);
  std::vector<rootsink::RootFileRecord> files;
  {
    rootsink::RawTreeConfig cfg;
    cfg.output_root = dir;
    cfg.expect = {{0, 0}};
    rootsink::RawTreeWriter w(cfg);
    rootsink::BuiltEvent ev = make_event(kRun, 0, 0, 0);
    push_item(ev.fragments[0].items, pack_item(0, 0, 0, 42));
    w.write(ev, 1000);
    w.close_run(kRun);
    files = w.files_snapshot();
  }
  CHECK_EQ(rootsink::file_size_bytes(target), before);  // 踏み潰していない
  CHECK_EQ(list_prefixed(run_dir, "run_raw_inprogress_").size(), 1);  // 維持
  CHECK_EQ(files.size(), 1);
  if (files.size() == 1) {
    CHECK(files[0].path.find("/run_raw_inprogress_") != std::string::npos);
  }
  remove_tree(dir);
}

// ---------------------------------------------------------------------------
// 4. 防御カウンタ(黙って捨てない —— CLAUDE.md)
// ---------------------------------------------------------------------------
//
// 手計算の出典: chan は 7bit(0–127)だが AGET は 68 ch。chan=68 と chan=127 の
// 2 件は置き場が無いので落として数える。期待集合 {0:0} の外から来た (1,0) の
// フラグメントも同様。残る「正しい」item は (aget1, chan2, bucket3, adc=77) だけ
// なので、adc 総和は 77 になる。
void test_out_of_range_and_unexpected_are_counted_not_written() {
  const std::string dir = scratch_dir("defence");
  const uint32_t kRun = 27;
  {
    rootsink::RawTreeConfig cfg;
    cfg.output_root = dir;
    cfg.expect = {{0, 0}};
    rootsink::RawTreeWriter w(cfg);
    rootsink::BuiltEvent ev;
    ev.run_number = kRun;
    ev.event_idx = 0;
    ev.fragments.push_back(make_fragment(kRun, 0, 0, 0));
    push_item(ev.fragments[0].items, pack_item(1, 2, 3, 77));    // 正常
    push_item(ev.fragments[0].items, pack_item(0, 68, 4, 1000));  // 範囲外
    push_item(ev.fragments[0].items, pack_item(0, 127, 5, 2000)); // 範囲外
    // 期待集合の外((cobo,asad) = (1,0))。eb_core は (cobo,asad) 昇順なので後ろ。
    ev.fragments.push_back(make_fragment(kRun, 0, 1, 0));
    push_item(ev.fragments[1].items, pack_item(0, 0, 0, 3000));
    w.write(ev, 1000);
    w.close_run(kRun);
    CHECK_EQ(w.items_out_of_range(), 2);
    CHECK_EQ(w.unexpected_fragments(), 1);
    CHECK(w.fatal_reason() == nullptr);
  }
  int adc_len = 0;
  const std::vector<RawEntry> entries =
      read_raw_file(dir + "/run0027/run0027_raw.root", &adc_len);
  CHECK_EQ(adc_len, 4 * 68 * 512);
  CHECK_EQ(entries.size(), 1);
  if (entries.size() == 1) {
    CHECK_EQ(entries[0].adc[adc_index(1, 2, 3)], 77);
    CHECK_EQ(adc_sum(entries[0].adc), 77);  // 範囲外・期待外は 1 つも書かれていない
  }
  remove_tree(dir);
}

// ---------------------------------------------------------------------------
// 5. compare_raw の負性対照(比較器が中身を見ていることの証明)
// ---------------------------------------------------------------------------
//
// 手計算の出典: 同じ 3 イベントを 2 回書き、片方だけ最後のイベント(event_idx 2)の
// ADC を 1 だけずらす(`100 + 2 = 102` → `103`)。adc の 1 要素だけが違うので
// compare_raw は **exit 1** かつ「1 differences」でなければならない。
void test_compare_raw_detects_a_one_adc_difference() {
  const char* env_bin = std::getenv("TPCDAQ_COMPARE_RAW");
  const std::string cmp_bin = (env_bin != nullptr && env_bin[0] != '\0')
                                  ? std::string(env_bin)
                                  : std::string("./compare_raw");
  if (!exists(cmp_bin)) {
    std::printf("SKIP compare_raw tests: %s not built (make compare)\n", cmp_bin.c_str());
    return;
  }
  const std::string dir = scratch_dir("cmpraw");
  const uint32_t kRun = 28;
  // variant 0/1 = 同内容、variant 2 = 1 要素だけ違う、variant 3 = エントリ数が違う。
  for (int variant : {0, 1, 2, 3}) {
    rootsink::RawTreeConfig cfg;
    cfg.output_root = dir + "/v" + std::to_string(variant);
    cfg.expect = {{0, 0}};
    rootsink::RawTreeWriter w(cfg);
    const uint32_t n_events = (variant == 3) ? 2 : 3;
    for (uint32_t idx = 0; idx < n_events; ++idx) {
      rootsink::BuiltEvent ev = make_event(kRun, idx, 0, 0);
      uint32_t adc = 100 + idx;
      if (variant == 2 && idx == 2) adc += 1;  // 1 要素だけずらす
      push_item(ev.fragments[0].items, pack_item(idx % 4, idx, idx, adc));
      w.write(ev, 1000 + idx);
    }
    w.close_run(kRun);
    CHECK(w.fatal_reason() == nullptr);
  }
  const std::string a = dir + "/v0/run0028/run0028_raw.root";
  const std::string b = dir + "/v1/run0028/run0028_raw.root";
  const std::string c = dir + "/v2/run0028/run0028_raw.root";
  const std::string d = dir + "/v3/run0028/run0028_raw.root";

  const CmdResult same = run_cmd(cmp_bin + " '" + a + "' '" + b + "'");
  CHECK_EQ(same.status, 0);
  CHECK(same.out.find("compared 3 events, 0 differences") != std::string::npos);

  const CmdResult one_adc = run_cmd(cmp_bin + " '" + a + "' '" + c + "'");
  CHECK(one_adc.status != 0);
  CHECK(one_adc.out.find("1 differences") != std::string::npos);
  if (one_adc.status == 0 || one_adc.out.find("1 differences") == std::string::npos) {
    std::printf("compare_raw(one adc) status=%d out=%s\n", one_adc.status,
                one_adc.out.c_str());
  }

  const CmdResult short_file = run_cmd(cmp_bin + " '" + a + "' '" + d + "'");
  CHECK(short_file.status != 0);
  CHECK(short_file.out.find("entries A=3 B=2") != std::string::npos);
  remove_tree(dir);
}

}  // namespace

// 取得の再開(TODO/110): raw ツリーは変更不要 —— ビルダが旧世代と同じ番号を除いた後の
// ストリームをそのまま書く(重複判定を持たない)ことの確認。
// 手計算の出典: 旧世代 2038..2056(19 件)+ 新世代 0..2060 から 2038..2056 を除いた
// 2042 件 = 2061 エントリ(PEventTPC 側 test_recorder と同じ数)。
void test_raw_tree_writes_the_epoch_filtered_stream_as_is() {
  const std::string dir = scratch_dir("epoch");
  const uint32_t kRun = 29;
  uint64_t entries = 0;
  {
    rootsink::RawTreeConfig cfg;
    cfg.output_root = dir;
    cfg.expect = {{0, 0}};
    rootsink::RawTreeWriter w(cfg);
    rootsink::EventBuilder eb({{0, 0}}, 1000);
    auto feed = [&](uint32_t idx, uint64_t time) {
      rootsink::BuiltEvent src = make_event(kRun, idx, 0, 0);
      src.fragments[0].event_time = time;
      eb.feed(std::move(src.fragments[0]), 0);
      for (rootsink::BuiltEvent& ev : eb.take_restart_flush()) w.write(ev, 1000);
    };
    for (uint32_t i = 2038; i <= 2056; ++i) feed(i, 406000000ULL + i * 10ULL);
    for (rootsink::BuiltEvent& ev : eb.poll(0)) w.write(ev, 1000);  // 旧世代を emit 済みに
    for (uint32_t i = 0; i <= 2060; ++i) feed(i, 16000ULL + i * 10ULL);
    for (rootsink::BuiltEvent& ev : eb.poll(0)) w.write(ev, 1001);
    for (rootsink::BuiltEvent& ev : eb.flush()) w.write(ev, 1002);
    CHECK_EQ(eb.restart_duplicate_events(), 19);
    w.close_run(kRun);
    CHECK(w.fatal_reason() == nullptr);
    entries = w.entries_written();
  }
  CHECK_EQ(entries, 2061);
  remove_tree(dir);
}

int main() {
  gROOT->SetBatch(kTRUE);

  test_mini_tree_matches_the_graw2root_contract();
  test_two_asad_expectation_sets_the_block_layout();
  test_inprogress_name_until_close_run();
  test_shutdown_without_eos_keeps_the_inprogress_name();
  test_zero_event_run_writes_no_file();
  test_existing_final_file_is_not_overwritten();
  test_out_of_range_and_unexpected_are_counted_not_written();
  test_compare_raw_detects_a_one_adc_difference();
  test_raw_tree_writes_the_epoch_filtered_stream_as_is();
  return tpccheck::report("test_raw_tree");
}
