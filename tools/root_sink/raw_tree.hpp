// raw_tree.hpp — raw ADC ツリー `run{run:04}_raw.root` の書き出し(SPEC §6.7 v1.28、
// TODO/075)。**ROOT に依存するヘッダ**(相方は root_recorder.hpp)。
//
// 何のためにあるか: mini eTPC のスタンドアロン解析チェーン(macros/ ——
// graw2root.C → analyzeUVW.C → makeTracks.C。素の ROOT のみ・TPCReco 不要)を
// **下流無変更**で DAQ に直結する。root-sink がこのツリーを書けば、graw2root.C
// (生 .graw → ROOT の変換ステップ)が要らなくなる。
//
// **契約の現物は macros/graw2root.C**(凍結されたオラクル)。ツリー `raw` /
// 1 エントリ = 1 ビルド済みイベント / ブランチ `eventId`(UInt_t)・
// `eventTime`(ULong64_t)・`adc`(Short_t、生 ADC = FPN 込み・減算なし)。
// 下流 analyzeUVW.C が `SetBranchAddress` する名前と型がそのまま凍結対象。
//
// **adc のリーフリストは焼き込まない**(SPEC §6.7 v1.28): 期待フラグメント集合
// (§6.3 `--expect`)の要素数 nAsad から起動時に組む —— `adc[{4×nAsad}][68][512]/S`。
// mini(`0:0` の 1 要素)で macros 契約と完全一致する。ブランチは最初のイベントより
// 前に要るので「観測した AsAd 数」では作れない(先頭が incomplete でも配列が
// 縮んではならない)。AsAd ブロックの順 = 期待集合の (cobo,asad) 昇順の順位。
//
// **PEventTPC(§6.4)とは別物・別スレッド・別ファイル**。こちらは生 ADC、
// あちらはペデスタル減算済み + ジオメトリ適用済み。ロックもキューも共有しない。
//
// ライフサイクルの規則は root_recorder.hpp の `RecorderWorker` と**同じ**
// (名前空間だけ分ける): 遅延オープン / `run_raw_inprogress_<unixtime>.root` で書いて
// finalize で rename / 異常終了は inprogress のまま / 既存の完成ファイルは上書きしない /
// AutoSave 30 s / 100 エントリ毎 FlushBaskets / `Write("", kOverwrite)`。
// **ロールオーバだけは無い**(SPEC §6.7 v1.28: 下流が単一ファイル前提)。
//
// **このヘッダを include してよいのは root_sink.cxx / test_raw_tree.cxx だけ**
// (root_recorder.hpp と同じ規約)。ROOT 非依存ヘッダ(tpc_wire / rs_core / eb_core)は
// ROOT 非依存のまま。TPCReco には依存しない —— PEventTPC もジオメトリも要らない
// (root_recorder.hpp を include するのは POSIX の小道具と定数の再利用のため)。

#ifndef TPCDAQ_ROOT_SINK_RAW_TREE_HPP
#define TPCDAQ_ROOT_SINK_RAW_TREE_HPP

#include <algorithm>
#include <atomic>
#include <cerrno>
#include <cinttypes>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <limits>
#include <map>
#include <mutex>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include <TFile.h>
#include <TObject.h>
#include <TTree.h>

#include "eb_core.hpp"
#include "root_recorder.hpp"

namespace rootsink {

// 下流 macros/ の契約(graw2root.C:119-125)。ツリー名とブランチ名は**凍結**。
constexpr const char* kRawTreeName = "raw";
constexpr const char* kRawEventIdBranch = "eventId";
constexpr const char* kRawEventTimeBranch = "eventTime";
constexpr const char* kRawAdcBranch = "adc";
// 1 AsAd = 4 AGET × 68 ch(FPN 込みの raw ch)× 512 time bucket(SPEC §2.4)。
constexpr int kRawAgetChips = 4;
constexpr int kRawChPerAget = 68;
constexpr int kRawBuckets = 512;

struct RawTreeConfig {
  std::string output_root = ".";            // --output-root(PEventTPC と同じ run dir)
  int compression = kDefaultCompression;    // --root-compression と同じ値を渡す
  // 期待フラグメント集合(SPEC §6.3、--expect)。**空は設定ミス** —— ctor で fatal。
  std::set<FragmentKey> expect;
};

// ---------------------------------------------------------------------------
// RawTreeWriter — TFile/TTree を専有する単一ライタ
// ---------------------------------------------------------------------------
//
// **このオブジェクトの ROOT 資源に触るスレッドは常に 1 本**(root_sink.cxx の
// raw スレッド)。run の切替は write() の中で検知し(遅延オープン)、finalize は
// in-band の RunClose(close_run)で起きる —— キューに残ったイベントを追い越して
// 閉じることはない。
class RawTreeWriter {
 public:
  explicit RawTreeWriter(RawTreeConfig cfg) : cfg_(std::move(cfg)) {
    if (cfg_.output_root.empty()) cfg_.output_root = ".";
    // 自動分割を殺す(命名規則を ROOT 任せにしない。root_recorder.hpp と同じ理由)。
    // **冪等** —— Recorder が先に呼んでいても同じ値を入れるだけ。
    TTree::SetMaxTreeSize(std::numeric_limits<Long64_t>::max());
    if (cfg_.expect.empty()) {
      // 期待集合が空だと adc 配列の段数が決まらない。黙って 0 段で走らない。
      set_fatal("raw-no-expect",
                "the raw tree needs at least one expected (cobo,asad) — see --expect "
                "(SPEC 6.3)");
      return;
    }
    // AsAd ブロック index = 期待集合を昇順に並べたときの順位(std::set は既に昇順)。
    int block = 0;
    for (const FragmentKey& k : cfg_.expect) block_of_[k] = block++;
    const int blocks = static_cast<int>(cfg_.expect.size());
    char buf[64];
    std::snprintf(buf, sizeof(buf), "%s[%d][%d][%d]/S", kRawAdcBranch,
                  kRawAgetChips * blocks, kRawChPerAget, kRawBuckets);
    leaflist_ = buf;
    adc_.assign(static_cast<std::size_t>(kRawAgetChips) * static_cast<std::size_t>(blocks) *
                    static_cast<std::size_t>(kRawChPerAget) *
                    static_cast<std::size_t>(kRawBuckets),
                0);
  }

  ~RawTreeWriter() { shutdown(); }

  RawTreeWriter(const RawTreeWriter&) = delete;
  RawTreeWriter& operator=(const RawTreeWriter&) = delete;

  // 1 エントリ = 1 ビルド済みイベント。run の切替は**内部で検知**する(遅延オープン
  // —— 0 イベントの run はファイルを作らない、SPEC §6.5)。
  void write(const BuiltEvent& ev, uint64_t now_ms) {
    if (fatal_.load(std::memory_order_acquire) != nullptr) return;
    // 中身が無いイベントは書かない(ビルダは空イベントを出さないので防御)。
    if (ev.fragments.empty()) return;
    if (!run_active_ || run_ != ev.run_number) begin_run(ev.run_number);
    if (file_ == nullptr && !open_run(now_ms)) return;

    fill_entry(ev);
    run_entries_.fetch_add(1, std::memory_order_relaxed);
    // 開いている part の現在サイズ(閉じた分は closed_bytes_)。**読むのは別スレッド**
    // なので atomic に置く(ROOT には触らせない)。
    bytes_written_.store(closed_bytes_ + static_cast<uint64_t>(file_->GetEND()),
                         std::memory_order_relaxed);
    // データが途切れない run でも AutoSave の期限を見る(root_recorder.hpp R-P2-2)。
    maybe_autosave(now_ms);
  }

  // EOS: 開いていれば finalize(`run{run:04}_raw.root` へ rename)。
  void close_run(uint32_t run) {
    if (file_ != nullptr) {
      if (run != run_) {
        // 開いている run と閉じろと言われた run が違う。ファイルは開いている run の
        // ものなのでそちらの名前で閉じる —— 黙って別 run の名前に化けさせない。
        std::fprintf(stderr,
                     "root_sink: raw tree asked to close run %u while run %u is open — "
                     "finalizing %u\n",
                     run, run_, run_);
      }
      close_part(/*finalize=*/true);
    } else {
      // データが 1 件も来なかった run。黙って通さない(CLAUDE.md)。
      std::fprintf(stderr, "root_sink: run %u closed with no raw ROOT file (no events)\n",
                   run);
    }
    run_active_ = false;
  }

  // 停止: 開いていれば **inprogress のまま**閉じる(SPEC §6.5)。何度呼んでも安全。
  void shutdown() {
    if (file_ != nullptr) close_part(/*finalize=*/false);
  }

  void tick(uint64_t now_ms) { maybe_autosave(now_ms); }

  // --- 状態・カウンタ(別スレッドから読まれるものは atomic)---
  bool is_open() const { return open_.load(std::memory_order_relaxed); }
  uint64_t entries_written() const { return entries_written_.load(std::memory_order_relaxed); }
  uint64_t run_entries() const { return run_entries_.load(std::memory_order_relaxed); }
  uint64_t bytes_written() const { return bytes_written_.load(std::memory_order_relaxed); }
  uint64_t items_out_of_range() const {
    return items_out_of_range_.load(std::memory_order_relaxed);
  }
  uint64_t unexpected_fragments() const {
    return unexpected_fragments_.load(std::memory_order_relaxed);
  }
  std::vector<RootFileRecord> files_snapshot() const {
    std::lock_guard<std::mutex> lk(files_mu_);
    return files_;
  }
  // 例 "adc[4][68][512]/S"(テスト用 —— 契約の現物と 1 文字も違わないこと)。
  const std::string& leaflist() const { return leaflist_; }
  // 開いている inprogress のパス(テスト用)。
  const std::string& provisional() const { return provisional_; }
  const char* fatal_reason() const { return fatal_.load(std::memory_order_acquire); }
  // `fatal_reason()` が非 nullptr を返した後にだけ読むこと(detail は fatal_ より
  // 先に書かれる = acquire/release で見える)。
  const std::string& fatal_detail() const { return fatal_detail_; }

 private:
  // run が変わった。**前の run は close_run 済みのはず** —— 開いたままなら上流の
  // プロトコル違反(root_recorder.hpp の同名防御と同じ扱い): finalize しない
  // (旧 run を完成 run 名に化けさせない。SPEC §6.5)。
  void begin_run(uint32_t run) {
    if (file_ != nullptr) {
      std::fprintf(stderr,
                   "root_sink: PROTOCOL VIOLATION raw tree saw run %u while run %u was "
                   "still open (this should have been fatal upstream, SPEC 6.2-5 v1.10) — "
                   "keeping %u inprogress, NOT finalizing it\n",
                   run, run_, run_);
      close_part(/*finalize=*/false);
    }
    run_ = run;
    run_active_ = true;
    run_entries_.store(0, std::memory_order_relaxed);
  }

  std::string final_path(uint32_t run) const {
    char buf[64];
    std::snprintf(buf, sizeof(buf), "run%04u_raw.root", run);
    return run_dir_of(cfg_.output_root, run) + "/" + buf;
  }

  // 書き込み中の名前 `run_raw_inprogress_<unixtime>.root`(SPEC §6.5)。同じ秒に
  // 2 個目が要る場合(前回の異常終了の残骸など)は `_2`, `_3` … を足す ——
  // **既存ファイルを RECREATE で踏み潰さない**(root_recorder.hpp の pick_provisional
  // と同じ規則。名前空間だけ `raw` で分ける)。
  std::string pick_provisional(uint32_t run) const {
    const std::string dir = run_dir_of(cfg_.output_root, run);
    char buf[64];
    std::snprintf(buf, sizeof(buf), "/run_raw_inprogress_%lld",
                  static_cast<long long>(std::time(nullptr)));
    const std::string stem = dir + buf;
    if (!path_exists(stem + ".root")) return stem + ".root";
    for (int n = 2; n < 10000; ++n) {
      const std::string cand = stem + "_" + std::to_string(n) + ".root";
      if (!path_exists(cand)) {
        std::fprintf(stderr, "root_sink: %s exists — using %s instead\n",
                     (stem + ".root").c_str(), cand.c_str());
        return cand;
      }
    }
    return stem + ".root";  // ここまで来たら諦める(下の open が失敗を検出する)
  }

  bool open_run(uint64_t now_ms) {
    const std::string dir = run_dir_of(cfg_.output_root, run_);
    if (!mkdir_p(dir)) {
      set_fatal("raw-mkdir", dir + ": " + std::strerror(errno));
      return false;
    }
    provisional_ = pick_provisional(run_);
    file_ = TFile::Open(provisional_.c_str(), "RECREATE", "", cfg_.compression);
    if (file_ == nullptr || file_->IsZombie()) {
      delete file_;
      file_ = nullptr;
      set_fatal("raw-open", provisional_);
      return false;
    }
    open_.store(true, std::memory_order_relaxed);  // 以降は close_part() が false に戻す
    tree_ = new TTree(kRawTreeName, "raw ADC waveforms");
    tree_->SetDirectory(file_);
    // **名前・型は graw2root.C と同じ**(下流 analyzeUVW.C の SetBranchAddress 契約)。
    tree_->Branch(kRawEventIdBranch, &event_id_, "eventId/i");
    tree_->Branch(kRawEventTimeBranch, &event_time_, "eventTime/l");
    tree_->Branch(kRawAdcBranch, adc_.data(), leaflist_.c_str());
    part_entries_ = 0;
    last_autosave_ms_ = now_ms;
    std::fprintf(stderr, "root_sink: recording raw run %u -> %s\n", run_,
                 provisional_.c_str());
    return true;
  }

  // 生 ADC をイベント単位で展開する。**毎イベント 0 クリア**してから届いた
  // フラグメントだけ埋める —— incomplete イベントの欠損 AsAd は 0 のまま
  // (graw2root.C の memset 既定と同じ意味論。SPEC §6.7)。
  void fill_entry(const BuiltEvent& ev) {
    std::fill(adc_.begin(), adc_.end(), static_cast<Short_t>(0));
    event_id_ = ev.event_idx;
    // eventTime = イベント先頭フラグメント((cobo,asad) 昇順の先頭 —— eb_core が
    // 整列済み)の 48bit CoBo タイムスタンプ(SPEC §6.7)。
    event_time_ = ev.fragments.front().event_time;

    for (const OwnedFragment& f : ev.fragments) {
      const auto it = block_of_.find(f.key());
      if (it == block_of_.end()) {
        // 期待集合の外 = 置き場が無い。書かずに数える(SPEC §6.7)。
        const uint64_t n = unexpected_fragments_.fetch_add(1, std::memory_order_relaxed) + 1;
        if (n == 1) {
          std::fprintf(stderr,
                       "root_sink: raw tree: fragment (cobo=%u asad=%u) is not in the "
                       "expected set — not written, counted\n",
                       static_cast<unsigned>(f.cobo), static_cast<unsigned>(f.asad));
        }
        continue;
      }
      const std::size_t block = static_cast<std::size_t>(it->second);
      const std::size_t n_items = f.item_count();
      for (std::size_t i = 0; i < n_items; ++i) {
        // item の 32bit パック(SPEC §2.4 / pevent_fill.hpp scatter_items と同じ)。
        const uint32_t w = f.item(i);
        const uint32_t aget = (w >> 30) & 0x3u;      // [31:30]
        const uint32_t chan = (w >> 23) & 0x7Fu;     // [29:23] raw 0–67(FPN 込み)
        const uint32_t bucket = (w >> 14) & 0x1FFu;  // [22:14] 0–511
        const uint32_t adc = w & 0xFFFu;             // [11:0] 生 ADC(減算なし)
        if (chan >= static_cast<uint32_t>(kRawChPerAget)) {
          // 7bit の chan は 127 まで表現できるが AGET は 68 ch。置き場が無いので
          // 落とすしかない —— **黙っては落とさない**(CLAUDE.md)。
          const uint64_t n = items_out_of_range_.fetch_add(1, std::memory_order_relaxed) + 1;
          if (n == 1) {
            std::fprintf(stderr,
                         "root_sink: raw tree: item chan=%u is outside the AGET's %d "
                         "channels — dropped, counted\n",
                         chan, kRawChPerAget);
          }
          continue;
        }
        // adc[block*4 + aget][chan][bucket](3 次元配列の行優先そのまま)。
        adc_[((block * kRawAgetChips + aget) * kRawChPerAget + chan) * kRawBuckets +
             bucket] = static_cast<Short_t>(adc);
      }
    }

    tree_->Fill();
    entries_written_.fetch_add(1, std::memory_order_relaxed);
    ++part_entries_;
    // 100 エントリ毎に basket を吐く(root_recorder.hpp と同じ間隔)。
    if (part_entries_ % kFlushBasketsEvery == 0) tree_->FlushBaskets();
  }

  // 開いているファイルを書いて閉じる。finalize=true なら最終名へ rename。
  void close_part(bool finalize) {
    if (file_ == nullptr) return;
    file_->cd();
    // 「最新版のツリーだけを残す」—— AutoSave が書いたキーの隣に ;2 を作らない。
    tree_->Write("", TObject::kOverwrite);
    const std::string written = provisional_;
    file_->Close();
    delete file_;  // TFile が TTree を所有(delete で tree_ も消える)
    file_ = nullptr;
    tree_ = nullptr;
    open_.store(false, std::memory_order_relaxed);

    const uint64_t bytes = file_size_bytes(written);
    std::string recorded = written;
    if (finalize) {
      const std::string target = final_path(run_);
      if (path_exists(target)) {
        // 完成済みの run ファイルを黙って上書きしない。inprogress のまま残す。
        std::fprintf(stderr,
                     "root_sink: ERROR %s already exists — keeping %s (NOT finalized)\n",
                     target.c_str(), written.c_str());
      } else if (std::rename(written.c_str(), target.c_str()) != 0) {
        std::fprintf(stderr, "root_sink: ERROR rename %s -> %s failed (%s) — file kept\n",
                     written.c_str(), target.c_str(), std::strerror(errno));
      } else {
        recorded = target;
        std::fprintf(stderr,
                     "root_sink: finalized %s (%" PRIu64 " entries, %" PRIu64 " bytes)\n",
                     target.c_str(), part_entries_, bytes);
      }
    } else {
      std::fprintf(stderr,
                   "root_sink: stopped mid-run — kept %s (%" PRIu64 " entries, NOT "
                   "finalized)\n",
                   written.c_str(), part_entries_);
    }
    closed_bytes_ += bytes;
    bytes_written_.store(closed_bytes_, std::memory_order_relaxed);
    RootFileRecord rec;
    rec.path = recorded;
    rec.entries = part_entries_;
    rec.bytes = bytes;
    {
      std::lock_guard<std::mutex> lk(files_mu_);
      files_.push_back(std::move(rec));
    }
    provisional_.clear();
    part_entries_ = 0;
  }

  void set_fatal(const char* reason, const std::string& detail) {
    if (fatal_.load(std::memory_order_relaxed) != nullptr) return;
    fatal_detail_ = detail;  // **理由より先に**書く(読み手は acquire で両方見える)
    fatal_.store(reason, std::memory_order_release);
    std::fprintf(stderr, "root_sink: FATAL %s: %s\n", reason, detail.c_str());
  }

  // AutoSave の期限判定(write()/tick() 共通)。run 中でも inprogress を ROOT で
  // 開けるようにしておく = 異常終了時にそこまでのデータが読める(SPEC §6.1)。
  void maybe_autosave(uint64_t now_ms) {
    if (file_ == nullptr || tree_ == nullptr) return;
    if (now_ms < last_autosave_ms_ + kAutoSaveIntervalMs) return;
    tree_->AutoSave("SaveSelf");  // ツリーとキーを書くがファイルは閉じない
    last_autosave_ms_ = now_ms;
  }

  RawTreeConfig cfg_;
  std::map<FragmentKey, int> block_of_;  // (cobo,asad) → AsAd ブロック index
  std::string leaflist_;

  TFile* file_ = nullptr;
  TTree* tree_ = nullptr;
  UInt_t event_id_ = 0;         // ブランチバッファ(graw2root.C と同じ型)
  ULong64_t event_time_ = 0;    // 同上
  std::vector<Short_t> adc_;    // 4×nAsad × 68 × 512

  std::string provisional_;
  uint32_t run_ = 0;
  bool run_active_ = false;
  uint64_t part_entries_ = 0;
  uint64_t closed_bytes_ = 0;
  uint64_t last_autosave_ms_ = 0;
  std::atomic<bool> open_{false};  // file_ != nullptr の外向きの影
  std::atomic<uint64_t> run_entries_{0};
  std::atomic<uint64_t> entries_written_{0};
  std::atomic<uint64_t> bytes_written_{0};
  std::atomic<uint64_t> items_out_of_range_{0};
  std::atomic<uint64_t> unexpected_fragments_{0};
  mutable std::mutex files_mu_;
  std::vector<RootFileRecord> files_;
  std::atomic<const char*> fatal_{nullptr};
  std::string fatal_detail_;
};

}  // namespace rootsink

#endif  // TPCDAQ_ROOT_SINK_RAW_TREE_HPP
