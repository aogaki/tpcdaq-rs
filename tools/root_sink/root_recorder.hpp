// root_recorder.hpp — GDataFrame の TTree 書き出し(SPEC §6.4)+ ファイルライフサイクル
// (SPEC §6.5)。**ROOT に依存する唯一のヘッダ**。
//
// 出自: delila-rs `tools/root_sink/root_sink.cxx` の `Recorder` クラスの流儀
// (inprogress 名で開く → EOS で rename、AutoSave、開いている TFile を 1 スレッドに
// 隔離する)を踏襲。GDataFrame の充填は
//   * `reference/20190315_patched/CoBoFrameViewer/src/graw2root/graw2root.cpp`(本家)
//   * `~/test/get/tpcdaq/src/output/root_writer.cpp`(C++ 版 tpcdaq = 実験で使用中)
// の 2 つを正とした。移植にあたっての変更点は下の「delila-rs / 本家との差」を参照。
//
// **このヘッダを include してよいのは root_sink.cxx と test_recorder.cxx だけ**。
// tpc_wire / rs_core / eb_core の 3 ヘッダは ROOT 非依存のまま(発注書 §3)。
//
// --- delila-rs / 本家との差(意図的なもの)---------------------------------
//  1. **rollover を ROOT 任せにしない**。delila は `TTree::GetMaxTreeSize` 超過時の
//     ROOT の自動 ChangeFile(`<stem>_1.root` …)を後から rename して辻褄を合わせていた
//     (dangling TFile* を名前比較で回避する、という綱渡り)。ここでは
//     `TTree::SetMaxTreeSize` を実質無限にして自動分割を殺し、`--max-root-bytes` で
//     **自分で** finalize → 次 part を開く(SPEC §6.5 の命名規則を ROOT に握らせない)。
//  2. finalize 先が既にあるときは **rename しない**(delila は `_<unix_ns>` を足す)。
//     完成済みの run ファイルを黙って上書きする方が危険なので、inprogress のまま残して
//     エラーを出す —— 「異常終了は inprogress のまま」(SPEC §6.5)と同じ扱い。
//  3. **書き出し失敗を握り潰さない**。delila は open 失敗を ERROR ログ + false で流していた。
//     保存系はロスレス契約なので `fatal_reason()` を立て、呼び手(root_sink.cxx)が
//     カウンタ JSON を出して即死する(CLAUDE.md「silent failure を作らない」)。
//  4. 範囲外 chan(item の chan は 7bit = 0–127、AGET は 68 ch)は落とすしかないが
//     **数える**(`items_out_of_range()`)。本家は try/catch で黙って捨てていた。
//
// --- GET クラスの地雷(third_party/get は無改変が原則)---------------------
// `GDataFrame` の `fChannels`/`fSamples` は **static 共有**(fgChannels/fgSamples)で、
// `~GDataFrame()` が `Reset()` 経由でそれを **delete** する。したがって
// **プロセス内で GDataFrame を同時に 2 個生かしてはいけない**(片方の破棄で他方が
// dangling する)。Recorder は 1 個だけ持ち、デストラクタで消す。読み戻し(test_recorder /
// 012 の比較)は **Recorder を畳んでから**行うこと。

#ifndef TPCDAQ_ROOT_SINK_ROOT_RECORDER_HPP
#define TPCDAQ_ROOT_SINK_ROOT_RECORDER_HPP

#include <dirent.h>
#include <sys/stat.h>
#include <unistd.h>

#include <algorithm>
#include <atomic>
#include <cerrno>
#include <cinttypes>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <limits>
#include <mutex>
#include <string>
#include <utility>
#include <vector>

#include <TFile.h>
#include <TProcessID.h>
#include <TROOT.h>
#include <TTree.h>

#include "GDataChannel.h"
#include "GDataFrame.h"
#include "GDataSample.h"
#include "GFrameHeader.h"
#include "eb_core.hpp"

namespace rootsink {

// SPEC §6.4 / §6.5 の定数(一字違わず合わせる)。
constexpr const char* kTreeName = "tree";           // ツリー名
constexpr const char* kBranchName = "GDataFrame";   // 単一ブランチ
constexpr int kBranchBufferSize = 32000;            // Branch(..., 32000, 99)
constexpr int kBranchSplitLevel = 99;               // graw2root 互換のリーフ名
constexpr int kRootCompression = 505;               // ZSTD-5
constexpr uint64_t kDefaultMaxRootBytes = 1ULL << 30;  // 1 GiB(--max-root-bytes 既定)
// AutoSave 間隔(delila-rs Recorder 由来 / SPEC §6.1)。run 中でも inprogress を
// ROOT で開けるようにしておく = 異常終了時にそこまでのデータが読める。
constexpr uint64_t kAutoSaveIntervalMs = 30000;

// AGET の物理的な上限(GFrameHeader::MAX_CHANNELS と同じ 68)。
constexpr int kMaxAget = 4;
constexpr int kMaxChanPerAget = 68;

// ---------------------------------------------------------------------------
// 小道具(POSIX。ROOT の TSystem を使わないのは test 側でも素直に使えるように)
// ---------------------------------------------------------------------------

inline bool path_exists(const std::string& p) {
  struct stat st;
  return ::stat(p.c_str(), &st) == 0;
}

inline uint64_t file_size_bytes(const std::string& p) {
  struct stat st;
  if (::stat(p.c_str(), &st) != 0) return 0;
  return static_cast<uint64_t>(st.st_size);
}

// `mkdir -p`。既存は成功扱い。作れなければ false。
inline bool mkdir_p(const std::string& path) {
  if (path.empty()) return false;
  std::string acc;
  size_t i = 0;
  if (path[0] == '/') {
    acc = "/";
    i = 1;
  }
  while (i <= path.size()) {
    const size_t slash = path.find('/', i);
    const size_t end = (slash == std::string::npos) ? path.size() : slash;
    if (end > i) {
      acc.append(path, i, end - i);
      if (::mkdir(acc.c_str(), 0755) != 0 && errno != EEXIST) return false;
      acc.push_back('/');
    }
    if (slash == std::string::npos) break;
    i = slash + 1;
  }
  return true;
}

// dir 直下の名前(. と .. を除く)。開けなければ空。
inline std::vector<std::string> list_directory(const std::string& dir) {
  std::vector<std::string> out;
  DIR* d = ::opendir(dir.c_str());
  if (d == nullptr) return out;
  for (struct dirent* e = ::readdir(d); e != nullptr; e = ::readdir(d)) {
    const std::string name = e->d_name;
    if (name == "." || name == "..") continue;
    out.push_back(name);
  }
  ::closedir(d);
  return out;
}

// ---------------------------------------------------------------------------
// 設定と出力の台帳
// ---------------------------------------------------------------------------

struct RecorderConfig {
  std::string output_root = ".";                    // --output-root
  uint64_t max_root_bytes = kDefaultMaxRootBytes;   // --max-root-bytes
};

// JSON の root_files 要素(SPEC の「何をどこに何件書いたか」の可視化)。
struct RootFileRecord {
  std::string path;      // 最終パス。finalize しなかった場合は inprogress のまま
  uint64_t entries = 0;  // そのファイルのエントリ数(= フレーム数)
  uint64_t bytes = 0;    // close 直後の実サイズ
};

// ---------------------------------------------------------------------------
// Recorder — ROOT IO はこのクラス(= 呼び手の 1 スレッド)に閉じ込める
// ---------------------------------------------------------------------------
//
// SPEC §6.2-8「run 境界でブロッキング外部 IO 禁止」の実装手段: finalize / rename /
// 次ファイルの open はすべてこのクラスの中で起き、それは Recorder スレッドの中だけ。
// 取り込み(ZMQ 受信 → 集計)は有界 Channel の向こう側なので止まらない。
class Recorder {
 public:
  explicit Recorder(RecorderConfig cfg) : cfg_(std::move(cfg)) {
    if (cfg_.output_root.empty()) cfg_.output_root = ".";
    // 自動分割を殺す(発注書 §2: 命名規則を ROOT 任せにしない)。
    TTree::SetMaxTreeSize(std::numeric_limits<Long64_t>::max());
    frame_ = new GET::GDataFrame();
    for (int a = 0; a < kMaxAget; ++a) {
      for (int c = 0; c < kMaxChanPerAget; ++c) grid_[a][c].reserve(64);
    }
  }

  ~Recorder() {
    shutdown();
    delete frame_;  // static TClonesArray も道連れ(ヘッダ冒頭の地雷)
    frame_ = nullptr;
  }

  Recorder(const Recorder&) = delete;
  Recorder& operator=(const Recorder&) = delete;

  // 組み上がったイベントを書く。**1 フラグメント = 1 エントリ**(SPEC §6.4)。
  // 遅延到着フラグメントは呼び手が「1 フラグメントの BuiltEvent」として渡す
  // (SPEC §6.3「emit 後に遅延到着したフラグメントも必ず TTree に書く」)。
  void write(const BuiltEvent& ev, uint64_t now_ms) {
    if (fatal_ != nullptr) return;  // 既に死んでいる(呼び手が exit する)
    if (ev.fragments.empty()) return;
    if (!run_active_ || run_ != ev.run_number) {
      // run が変わった = 前の run は close_run 済みのはず。part を 0 に戻す。
      if (run_active_ && file_ != nullptr) {
        std::fprintf(stderr,
                     "root_sink: recorder saw run %u while run %u was still open — "
                     "finalizing the old one\n",
                     ev.run_number, run_);
        close_part(/*finalize=*/true);
      }
      run_ = ev.run_number;
      run_active_ = true;
      next_part_ = 0;
    }
    if (file_ == nullptr && !open_part(now_ms)) return;

    for (const OwnedFragment& f : ev.fragments) fill(f);

    // ロールオーバ判定は **イベント単位**(フレーム単位ではなく)—— 1 イベントの
    // フラグメントが 2 ファイルに割れないようにする。GetEND() = 現在のファイル末尾
    // オフセット = 実ファイルサイズ。
    if (file_ != nullptr && static_cast<uint64_t>(file_->GetEND()) > cfg_.max_root_bytes) {
      close_part(/*finalize=*/true);
      // 次の part はここでは開かない。**次の write() まで開かない**ので、
      // run の最後がロールオーバで終わっても空ファイルが残らない。
    }
  }

  // 全ソース EOS(run が閉じた)→ finalize して `run{run:04}.root` へ rename。
  void close_run(uint32_t run_number, uint64_t now_ms) {
    (void)now_ms;
    if (file_ != nullptr) {
      close_part(/*finalize=*/true);
    } else if (!run_active_ || run_ != run_number) {
      // データが 1 件も来なかった run。黙って通さない(CLAUDE.md)。
      std::fprintf(stderr, "root_sink: run %u closed with no ROOT file (no events)\n",
                   run_number);
    }
    run_active_ = false;
  }

  // 停止(SIGTERM / 異常)。開いていれば **inprogress のまま**閉じる ——
  // finalize していない run が完全 run に化けない(SPEC §6.5)。
  void shutdown() {
    if (file_ != nullptr) close_part(/*finalize=*/false);
    run_active_ = false;
  }

  // データが来ない間も呼ばれる(呼び手の tick)。AutoSave の面倒だけ見る。
  void tick(uint64_t now_ms) {
    if (file_ == nullptr || tree_ == nullptr) return;
    if (now_ms < last_autosave_ms_ + kAutoSaveIntervalMs) return;
    tree_->AutoSave("SaveSelf");  // ツリーとキーを書くがファイルは閉じない
    last_autosave_ms_ = now_ms;
  }

  // --- 状態・カウンタ ---
  //
  // カウンタ 3 種は **他スレッドから読まれる**(root_sink.cxx は fatal 時に Recorder
  // スレッドを join できないまま JSON を出す —— 「落ちた瞬間のカウンタを捨てない」)。
  // スカラは atomic、可変長の files_ は mutex でスナップショットを取る。
  // ロックは**ファイルを閉じるときだけ**なので、ホットパス(fill)には乗らない。
  bool is_open() const { return file_ != nullptr; }
  uint64_t entries_written() const { return entries_written_.load(std::memory_order_relaxed); }
  uint64_t items_out_of_range() const {
    return items_out_of_range_.load(std::memory_order_relaxed);
  }
  std::vector<RootFileRecord> files_snapshot() const {
    std::lock_guard<std::mutex> lk(files_mu_);
    return files_;
  }
  const std::string& provisional() const { return provisional_; }
  // 非 nullptr なら ROOT IO が失敗している。呼び手はカウンタを出して即死すること。
  const char* fatal_reason() const { return fatal_; }
  const std::string& fatal_detail() const { return fatal_detail_; }

  // `<output_root>/run{run:04}`(SPEC §6.5: run 毎ディレクトリ)。
  std::string run_dir(uint32_t run) const {
    char buf[32];
    std::snprintf(buf, sizeof(buf), "run%04u", run);
    return cfg_.output_root + "/" + buf;
  }

 private:
  // `run{run:04}.root`(part 0)/ `run{run:04}_{part:04}.root`(part 1〜)。
  std::string final_path(uint32_t run, uint32_t part) const {
    char buf[64];
    if (part == 0) {
      std::snprintf(buf, sizeof(buf), "run%04u.root", run);
    } else {
      std::snprintf(buf, sizeof(buf), "run%04u_%04u.root", run, part);
    }
    return run_dir(run) + "/" + buf;
  }

  // 書き込み中の名前 `run_inprogress_<unixtime>.root`(SPEC §6.5)。
  // 同じ秒に 2 個目が要る場合(小さい --max-root-bytes での連続ロールオーバや、
  // 前回の異常終了の残骸)は `_2`, `_3` … を足す —— **既存ファイルを RECREATE で
  // 踏み潰さない**(それは黙ってデータを捨てるのと同じ)。
  std::string pick_provisional(uint32_t run) const {
    const std::string dir = run_dir(run);
    char buf[64];
    std::snprintf(buf, sizeof(buf), "/run_inprogress_%lld",
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

  bool open_part(uint64_t now_ms) {
    const std::string dir = run_dir(run_);
    if (!mkdir_p(dir)) {
      set_fatal("root-mkdir", dir + ": " + std::strerror(errno));
      return false;
    }
    provisional_ = pick_provisional(run_);
    file_ = TFile::Open(provisional_.c_str(), "RECREATE", "", kRootCompression);
    if (file_ == nullptr || file_->IsZombie()) {
      delete file_;
      file_ = nullptr;
      set_fatal("root-open", provisional_);
      return false;
    }
    tree_ = new TTree(kTreeName, "tpcdaq GET data frames");
    tree_->SetDirectory(file_);
    // splitlevel 99 が graw2root 互換のリーフ名を作る(SPEC §6.4)。
    tree_->Branch(kBranchName, &frame_, kBranchBufferSize, kBranchSplitLevel);
    part_ = next_part_++;
    part_entries_ = 0;
    last_autosave_ms_ = now_ms;
    std::fprintf(stderr, "root_sink: recording run %u part %u -> %s\n", run_, part_,
                 provisional_.c_str());
    return true;
  }

  // 1 フラグメント = 1 エントリ。充填の正は graw2root.cpp と C++ 版 tpcdaq RootWriter。
  void fill(const OwnedFragment& f) {
    frame_->Clear();
    // TRefArray(GDataChannel::fSamples)が使うオブジェクト番号を毎エントリ巻き戻す。
    // graw2root.cpp と ROOT の JetEvent チュートリアル由来 —— これをやらないと
    // TProcessID のオブジェクト表が単調増加して、長い run で溢れる。
    const Int_t object_count = TProcessID::GetObjectCount();

    GET::GFrameHeader& h = frame_->fHeader;
    // Fragment(SPEC §2.4)は dataSource を運ばない(MFM ヘッダの 1 バイト)。
    // C++ 版 tpcdaq RootWriter も 0 を入れている。frame_type も GFrameHeader に
    // 置き場が無い(GET クラスは無改変が原則)—— どちらも生 graw がバックストップ。
    h.fDataSource = 0;
    h.fRevision = f.revision;
    h.fEventTime = f.event_time;
    h.fEventIdx = f.event_idx;
    h.fCoboIdx = f.cobo;
    h.fAsadIdx = f.asad;
    h.fReadOffset = f.read_offset;
    h.fStatus = f.status;
    h.fWindowOut = f.window_out;
    for (int a = 0; a < 4; ++a) {
      h.fMult[a] = f.mult[a];
      h.fLastCellIdx[a] = f.last_cell[a];
    }
    // fHitPatterns は未充填(SPEC §6.4。C++ 版 RootWriter も同じ —— 生 graw が
    // 可逆バックストップなので、36 バイトをワイヤに載せる方を選ばなかった)。

    // items を (aget, chan) のマスに配る。grid_ は使い回し(clear は容量を残すので
    // ホットパスで heap 確保が起きない —— CLAUDE.md)。
    const size_t n = f.item_count();
    for (size_t i = 0; i < n; ++i) {
      const uint32_t w = f.item(i);
      const uint32_t aget = (w >> 30) & 0x3u;    // [31:30]
      const uint32_t chan = (w >> 23) & 0x7Fu;   // [29:23] raw 0–67(FPN 込み)
      const uint32_t bucket = (w >> 14) & 0x1FFu;  // [22:14]
      const uint32_t adc = w & 0xFFFu;             // [11:0] 生 ADC(減算なし)
      if (chan >= static_cast<uint32_t>(kMaxChanPerAget)) {
        // 7bit の chan は 127 まで表現できるが AGET は 68 ch。置き場が無いので
        // 落とすしかない —— **黙っては落とさない**(CLAUDE.md)。
        items_out_of_range_.fetch_add(1, std::memory_order_relaxed);
        continue;
      }
      grid_[aget][chan].push_back(Sample{static_cast<uint16_t>(bucket),
                                         static_cast<uint16_t>(adc)});
    }

    // チャンネルの並びは **(aget, chan) 昇順**で決定的にする。実データの item 順
    // (bucket 外側 → aget → chan)での初出順と一致するので、C++ 版 tpcdaq の
    // ROOT 出力とも同じ並びになる(§12-3 の TTree 比較は 012 の仕事)。
    for (int a = 0; a < kMaxAget; ++a) {
      for (int c = 0; c < kMaxChanPerAget; ++c) {
        std::vector<Sample>& cell = grid_[a][c];
        if (cell.empty()) continue;
        // チャンネル内は bucket 昇順で連続 AddSample(標準リーダの前提、SPEC §6.4)。
        // 実データは既に昇順なので、判定だけして普段は並べ替えない。
        if (!std::is_sorted(cell.begin(), cell.end(), by_bucket)) {
          std::stable_sort(cell.begin(), cell.end(), by_bucket);
        }
        GET::GDataChannel* ch = frame_->AddChannel(static_cast<UShort_t>(a),
                                                   static_cast<UShort_t>(c));
        for (const Sample& s : cell) {
          GET::GDataSample* smp = frame_->AddSample();
          smp->Set(s.bucket, s.adc);
          ch->AddSample(smp);
        }
        cell.clear();  // 容量は残る
      }
    }

    TProcessID::SetObjectCount(object_count);
    tree_->Fill();
    entries_written_.fetch_add(1, std::memory_order_relaxed);
    ++part_entries_;
  }

  // 現在の part を書いて閉じる。finalize=true なら最終名へ rename。
  void close_part(bool finalize) {
    if (file_ == nullptr) return;
    file_->cd();
    tree_->Write();
    const std::string written = provisional_;
    file_->Close();
    delete file_;  // TFile が TTree を所有(delete で tree_ も消える)
    file_ = nullptr;
    tree_ = nullptr;

    const uint64_t bytes = file_size_bytes(written);
    std::string recorded = written;
    if (finalize) {
      const std::string target = final_path(run_, part_);
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
        std::fprintf(stderr, "root_sink: finalized %s (%" PRIu64 " entries, %" PRIu64
                             " bytes)\n",
                     target.c_str(), part_entries_, bytes);
      }
    } else {
      std::fprintf(stderr,
                   "root_sink: stopped mid-run — kept %s (%" PRIu64
                   " entries, NOT finalized)\n",
                   written.c_str(), part_entries_);
    }
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
    if (fatal_ != nullptr) return;
    fatal_ = reason;
    fatal_detail_ = detail;
    std::fprintf(stderr, "root_sink: FATAL %s: %s\n", reason, detail.c_str());
  }

  struct Sample {
    uint16_t bucket;
    uint16_t adc;
  };
  static bool by_bucket(const Sample& a, const Sample& b) { return a.bucket < b.bucket; }

  RecorderConfig cfg_;
  TFile* file_ = nullptr;
  TTree* tree_ = nullptr;
  GET::GDataFrame* frame_ = nullptr;  // ブランチのバッファ(プロセス内で 1 個だけ)

  std::string provisional_;
  uint32_t run_ = 0;
  bool run_active_ = false;
  uint32_t part_ = 0;       // 開いている part
  uint32_t next_part_ = 0;  // 次に開く part
  uint64_t part_entries_ = 0;
  std::atomic<uint64_t> entries_written_{0};
  std::atomic<uint64_t> items_out_of_range_{0};
  uint64_t last_autosave_ms_ = 0;
  mutable std::mutex files_mu_;
  std::vector<RootFileRecord> files_;
  const char* fatal_ = nullptr;
  std::string fatal_detail_;

  // (aget, chan) の作業マス。1 フレーム分だけ使って毎回 clear する。
  std::vector<Sample> grid_[kMaxAget][kMaxChanPerAget];
};

}  // namespace rootsink

#endif  // TPCDAQ_ROOT_SINK_ROOT_RECORDER_HPP
