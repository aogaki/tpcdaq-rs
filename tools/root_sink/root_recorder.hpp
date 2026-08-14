// root_recorder.hpp — イベント TTree の書き出し(SPEC §6.4)+ ファイルライフサイクル
// (SPEC §6.5)。**ROOT に依存するヘッダ**(相方は pevent_fill.hpp)。
//
// 出自: delila-rs `tools/root_sink/root_sink.cxx` の `Recorder` クラスの流儀
// (inprogress 名で開く → EOS で rename、AutoSave、開いている TFile を 1 スレッドに
// 隔離する)を踏襲。GDataFrame の充填は
//   * `reference/20190315_patched/CoBoFrameViewer/src/graw2root/graw2root.cpp`(本家)
//   * `~/test/get/tpcdaq/src/output/root_writer.cpp`(C++ 版 tpcdaq = 実験で使用中)
// の 2 つを正とした。移植にあたっての変更点は下の「delila-rs / 本家との差」を参照。
//
// **出力形式は 2 つ(SPEC §6.4 v1.8、TODO/020)**:
//   * `OutputFormat::PEvent`(**既定**)= TPCReco `grawToEventTPC` と同一形式。
//     ツリー `TPCData` / ブランチ `Event`(128000, splitlevel 2)/ **1 エントリ =
//     1 ビルド済みイベント** / eventId は一回きり(遅延分は書かず数える)/
//     `myChargeArray*` ブランチ無効 / 100 イベント毎 FlushBaskets /
//     `Write("", kOverwrite)`。出自 = `EventSources/src/ConvertGrawFile.cpp:40-45,84-92`。
//   * `OutputFormat::GDataFrame` = v1.7 までの出力(ツリー `tree` / 1 エントリ =
//     1 フラグメント)。**テスト専用**(§12-3 の mini 実データ全値一致オラクルを
//     持つ唯一の回帰)—— PEventTPC の同 run 実データオラクルが閉じたら削除してよい。
//
// **このヘッダを include してよいのは root_sink.cxx / test_recorder.cxx /
// test_pevent.cxx だけ**。tpc_wire / rs_core / eb_core の 3 ヘッダは ROOT 非依存の
// まま(発注書 §3)。
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
#include <memory>
#include <mutex>
#include <string>
#include <utility>
#include <vector>

#include <TFile.h>
#include <TH1D.h>
#include <TH2D.h>
#include <TObject.h>
#include <TProcessID.h>
#include <TROOT.h>
#include <TTree.h>

#include "GDataChannel.h"
#include "GDataFrame.h"
#include "GDataSample.h"
#include "GFrameHeader.h"
#include "eb_core.hpp"
#include "monitor_hist.hpp"
#include "pevent_fill.hpp"

namespace rootsink {

// 出力形式(SPEC §6.4 v1.8)。既定は PEventTPC —— GDataFrame はテスト専用。
enum class OutputFormat { PEvent, GDataFrame };

// SPEC §6.4 / §6.5 の定数(一字違わず合わせる)。
constexpr const char* kTreeName = "tree";           // ツリー名(GDataFrame モード)
constexpr const char* kBranchName = "GDataFrame";   // 単一ブランチ(同上)
constexpr int kBranchBufferSize = 32000;            // Branch(..., 32000, 99)
constexpr int kBranchSplitLevel = 99;               // graw2root 互換のリーフ名

// PEventTPC モード。**TPCReco `EventSourceROOT` がハード期待する形**
// (ConvertGrawFile.cpp:40-45 / EventSourceROOT.cpp:25,74)。
constexpr const char* kPEventTreeName = "TPCData";
constexpr const char* kPEventBranchName = "Event";
constexpr int kPEventBufferSize = 128000;
constexpr int kPEventSplitLevel = 2;
// 実運用 `disabledBranches` と同一(float[3][3][256][512] ≈ 4.7 MB/イベントの節約)。
constexpr const char* kDisabledBranchPattern = "myChargeArray*";
// ConvertGrawFile.cpp:85 と同じ間隔。
constexpr uint64_t kFlushBasketsEvery = 100;
// ZLIB-1(既定、TODO/014・SPEC §6.4 v1.5)。Warsaw はオフライン解析も DAQ 計算機の
// 同一(旧)ROOT で行うため ZSTD(505、ROOT 6.20+ 必須)は読めない。ZLIB は全時代互換
// で C++ 版の「ROOT 既定」とも一致。`--root-compression` で上書き可(例 505=ZSTD-5)。
constexpr int kDefaultCompression = 101;
constexpr uint64_t kDefaultMaxRootBytes = 1ULL << 30;  // 1 GiB(--max-root-bytes 既定)
// AutoSave 間隔(delila-rs Recorder 由来 / SPEC §6.1)。run 中でも inprogress を
// ROOT で開けるようにしておく = 異常終了時にそこまでのデータが読める。
constexpr uint64_t kAutoSaveIntervalMs = 30000;

// モニタヒストの 1D 軸レンジ(SPEC §5.2「x レンジ 0–4096 固定。オートレンジ禁止 ——
// 飽和天井 4095 が常に見えること」)。TODO/022。
constexpr double kChargeAxisMax = 4096.0;

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
  int compression = kDefaultCompression;            // --root-compression(TODO/014)
  OutputFormat format = OutputFormat::PEvent;       // --format(既定 = PEventTPC)
  // --run-id(TODO/021): EventInfo.runId の上書き。0 = run を開いた時刻から生成。
  long run_id_override = 0;
  // PEvent モードでは **必須**(strip 射影に要る)。呼び手が寿命を持つこと。
  const tpcgeo::Geometry* geometry = nullptr;       // --geometry
  tpcpevent::FillConfig fill;                       // --pedestal-remove / --*-cell
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
    if (cfg_.format == OutputFormat::PEvent) {
      if (cfg_.geometry == nullptr) {
        // ジオメトリ無しで PEventTPC は書けない(strip 射影ができない)。
        // 呼び手が CLI で弾くのが本筋だが、黙って空を書かないための最後の砦。
        set_fatal("pevent-no-geometry", "PEventTPC output needs a geometry (--geometry)");
        return;
      }
      pevent_ = new PEventTPC();
      filler_.reset(new tpcpevent::Filler(*cfg_.geometry, cfg_.fill));
    }
  }

  ~Recorder() {
    shutdown();
    delete pevent_;
    pevent_ = nullptr;
    delete frame_;  // static TClonesArray も道連れ(ヘッダ冒頭の地雷)
    frame_ = nullptr;
  }

  Recorder(const Recorder&) = delete;
  Recorder& operator=(const Recorder&) = delete;

  // 組み上がったイベントを書く。
  //   * PEvent モード: **1 エントリ = 1 ビルド済みイベント**(SPEC §6.4 v1.8)。
  //     既に書いた eventId(= 遅延到着)は**書かずに数える**(SPEC §6.3 v1.8)。
  //   * GDataFrame モード: **1 フラグメント = 1 エントリ**(v1.7 まで)。遅延到着
  //     フラグメントは呼び手が「1 フラグメントの BuiltEvent」として渡す。
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
      run_events_ = 0;
      have_last_event_id_ = false;
      if (cfg_.format == OutputFormat::PEvent) {
        // runId = **run 開始時刻**の %Y%m%d%H%M%S(TPCReco RunIdParser と同じ導出、
        // SPEC §6.4)。run 中は一定 —— ここで 1 回だけ決める。--run-id 指定時は
        // その値(実データ照合と P4 controller 経路の受け口、TODO/021)。
        run_id_ = cfg_.run_id_override != 0 ? cfg_.run_id_override : tpcpevent::run_id_now();
        std::fprintf(stderr, "root_sink: run %u runId=%ld\n", run_, run_id_);
      }
    }

    if (cfg_.format == OutputFormat::PEvent) {
      // eventId は一回きり(grawToEventTPC の eventIdMap と同じ意味論)。ビルダは
      // event_idx 昇順で emit するので、「以前に書いた番号以下」= 遅延・重複。
      if (have_last_event_id_ && ev.event_idx <= last_event_id_) {
        duplicate_event_ids_.fetch_add(1, std::memory_order_relaxed);
        if (duplicate_event_ids_.load(std::memory_order_relaxed) == 1) {
          std::fprintf(stderr,
                       "root_sink: event_idx=%u already written (last=%u) — not written to "
                       "the TTree, counted (raw graw keeps the data)\n",
                       ev.event_idx, last_event_id_);
        }
        return;
      }
    }

    if (file_ == nullptr && !open_part(now_ms)) return;

    if (cfg_.format == OutputFormat::PEvent) {
      fill_pevent(ev);
    } else {
      for (const OwnedFragment& f : ev.fragments) fill(f);
    }
    ++run_events_;  // この run に何か書いたか(0 イベント run の判定 — SPEC §6.5)

    // `bytes_written`(status の材料、SPEC §5.3)。GetEND() = 現在のファイル末尾
    // オフセットなので、閉じた part の合計 + 開いている part の現在サイズ。
    // **読むのは Publisher スレッド**なので atomic に置く(ROOT には触らせない)。
    if (file_ != nullptr) {
      bytes_written_.store(closed_bytes_ + static_cast<uint64_t>(file_->GetEND()),
                            std::memory_order_relaxed);
    }

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

  // モニタヒスト 9 枚を `run{run:04}_monitor.root` に書く(SPEC §5.2/§6.5、TODO/022)。
  //
  // **呼ぶのは close_run() の直後**(TTree finalize 後)。集計は monitor_hist.hpp が
  // 持ち、ROOT 化はここ = Recorder スレッドだけ —— 「ROOT IO は 1 スレッド」の原則を
  // モニタ経路でも崩さない(SPEC §5.1)。R10(EOS から 10 s 以内)は即時書きで満たす。
  //
  // **0 イベントの run には書かない**(§6.5 の遅延オープンと同じ理屈)。run が
  // 閉じないまま停止した場合は呼ばれない(RunClose マーカー経由でしか来ない)ので、
  // 未完了 run のヒストが完全 run に化けることもない。
  void write_monitor_root(uint32_t run, const rsmon::HistSnapshot& hists) {
    if (fatal_ != nullptr) return;
    if (run_ != run || run_events_ == 0) {
      std::fprintf(stderr, "root_sink: run %u had no events — no monitor.root\n", run);
      return;
    }
    const std::string dir = run_dir(run);
    if (!mkdir_p(dir)) {
      set_fatal("monitor-root-mkdir", dir + ": " + std::strerror(errno));
      return;
    }
    char name[64];
    std::snprintf(name, sizeof(name), "/run%04u_monitor.root", run);
    const std::string path = dir + name;
    TFile* out = TFile::Open(path.c_str(), "RECREATE", "", cfg_.compression);
    if (out == nullptr || out->IsZombie()) {
      delete out;
      set_fatal("monitor-root-open", path);
      return;
    }
    out->cd();
    for (const rsmon::HistBuffer& h : hists) {
      if (h.nx == 0) {
        // ジオメトリにその面のストリップが 1 本も無い(Nstrip = 0)。ビン 0 本の
        // ヒストは作れないので飛ばす —— 黙ってではなく 1 行出す(CLAUDE.md)。
        std::fprintf(stderr, "root_sink: monitor hist %s has 0 bins (no strips) — skipped\n",
                     h.name);
        continue;
      }
      if (h.ny > 1) {
        // x = strip(1..N+1)、y = bucket(0..512)。SPEC §5.2 の軸。
        TH2D* th = new TH2D(h.name, h.name, static_cast<Int_t>(h.nx), 1.0,
                            1.0 + static_cast<double>(h.nx), static_cast<Int_t>(h.ny), 0.0,
                            static_cast<double>(h.ny));
        for (uint32_t x = 0; x < h.nx; ++x) {
          for (uint32_t y = 0; y < h.ny; ++y) {
            // 添字は (strip-1)*512 + bucket(SPEC §5.3)。ROOT のビンは 1 起点。
            th->SetBinContent(static_cast<Int_t>(x) + 1, static_cast<Int_t>(y) + 1,
                              h.bins[static_cast<size_t>(x) * h.ny + y]);
          }
        }
        th->SetDirectory(out);
      } else {
        // x = 波高 [0, 4096) 固定(オートレンジ禁止 — SPEC §5.2)。
        TH1D* th = new TH1D(h.name, h.name, static_cast<Int_t>(h.nx), 0.0, kChargeAxisMax);
        for (uint32_t x = 0; x < h.nx; ++x) {
          th->SetBinContent(static_cast<Int_t>(x) + 1, h.bins[x]);
        }
        th->SetDirectory(out);
      }
    }
    out->Write();
    out->Close();
    delete out;  // ヒストは TFile のディレクトリが所有(道連れで消える)
    std::fprintf(stderr, "root_sink: wrote %s (%" PRIu64 " bytes)\n", path.c_str(),
                 file_size_bytes(path));
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
  // イベント ROOT に書いた実バイト数(閉じた part の合計 + 開いている part の現在サイズ)。
  // status の `bytes_written`(SPEC §5.3)—— **Publisher スレッドから読まれる**ので
  // atomic。monitor.root の分は含めない(保存系のスループットを表す数字なので)。
  uint64_t bytes_written() const { return bytes_written_.load(std::memory_order_relaxed); }
  uint64_t items_out_of_range() const {
    return items_out_of_range_.load(std::memory_order_relaxed);
  }
  // PEvent モードで「既に書いた eventId」を弾いた回数(遅延到着・重複)。
  uint64_t duplicate_event_ids() const {
    return duplicate_event_ids_.load(std::memory_order_relaxed);
  }
  // PEvent モードの充填カウンタ(strip でないチャンネル / PEventTPC の固定長配列を
  // はみ出す key / ジオメトリ外フレーム)。GDataFrame モードでは常に 0。
  // **Filler 側は素の整数**(per-sample の atomic はホットパスに置かない)。
  // イベント毎に 1 回だけここへ積み替えて、他スレッドから読めるようにする。
  uint64_t channels_without_strip() const {
    return channels_without_strip_.load(std::memory_order_relaxed);
  }
  uint64_t charge_keys_out_of_range() const {
    return charge_keys_out_of_range_.load(std::memory_order_relaxed);
  }
  uint64_t frames_outside_geometry() const {
    return frames_outside_geometry_.load(std::memory_order_relaxed);
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
    file_ = TFile::Open(provisional_.c_str(), "RECREATE", "", cfg_.compression);
    if (file_ == nullptr || file_->IsZombie()) {
      delete file_;
      file_ = nullptr;
      set_fatal("root-open", provisional_);
      return false;
    }
    if (cfg_.format == OutputFormat::PEvent) {
      // **タイトルは空文字**(ConvertGrawFile.cpp:41 `TTree aTree(treeName, "")`)。
      tree_ = new TTree(kPEventTreeName, "");
      tree_->SetDirectory(file_);
      tree_->Branch(kPEventBranchName, &pevent_, kPEventBufferSize, kPEventSplitLevel);
      // myChargeArray は書かない(実運用 disabledBranches と同一)。found==0 なら
      // クラス定義が変わったということ —— 黙って 4.7 MB/イベントを書き始めない。
      UInt_t found = 0;
      tree_->SetBranchStatus(kDisabledBranchPattern, false, &found);
      if (found == 0) {
        set_fatal("pevent-branch", std::string("no branch matches ") + kDisabledBranchPattern);
        return false;
      }
    } else {
      tree_ = new TTree(kTreeName, "tpcdaq GET data frames");
      tree_->SetDirectory(file_);
      // splitlevel 99 が graw2root 互換のリーフ名を作る(SPEC §6.4)。
      tree_->Branch(kBranchName, &frame_, kBranchBufferSize, kBranchSplitLevel);
    }
    part_ = next_part_++;
    part_entries_ = 0;
    last_autosave_ms_ = now_ms;
    std::fprintf(stderr, "root_sink: recording run %u part %u -> %s\n", run_, part_,
                 provisional_.c_str());
    return true;
  }

  // OwnedFragment を `frame_`(GDataFrame)に展開する。**両モードの共通入口** ——
  // PEventTPC 充填も「GDataFrame を読む」形に揃えることで、TPCReco
  // `fillEventFromFrame` との意味論の等価性を構造的に担保する(SPEC §6.4 v1.8)。
  // 充填の正は graw2root.cpp と C++ 版 tpcdaq RootWriter。
  void build_frame(const OwnedFragment& f) {
    frame_->Clear();
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
  }

  // GDataFrame モード: **1 フラグメント = 1 エントリ**(v1.7 まで、テスト専用)。
  void fill(const OwnedFragment& f) {
    // TRefArray(GDataChannel::fSamples)が使うオブジェクト番号を毎エントリ巻き戻す。
    // graw2root.cpp と ROOT の JetEvent チュートリアル由来 —— これをやらないと
    // TProcessID のオブジェクト表が単調増加して、長い run で溢れる。
    const Int_t object_count = TProcessID::GetObjectCount();
    build_frame(f);
    TProcessID::SetObjectCount(object_count);
    tree_->Fill();
    entries_written_.fetch_add(1, std::memory_order_relaxed);
    ++part_entries_;
  }

  // PEvent モード: **1 エントリ = 1 ビルド済みイベント**(SPEC §6.3/§6.4 v1.8)。
  // フラグメントは (cobo,asad) 昇順に並んでいる(eb_core が保証)—— 順に
  // GDataFrame へ展開して chargeMap に足し込む(`AddValByStrip` は `+=`)。
  void fill_pevent(const BuiltEvent& ev) {
    pevent_->Clear();
    uint64_t event_time = 0;
    for (const OwnedFragment& f : ev.fragments) {
      const Int_t object_count = TProcessID::GetObjectCount();
      build_frame(f);
      filler_->add_frame(*frame_, *pevent_);
      TProcessID::SetObjectCount(object_count);
      // timestamp は最後のフラグメント(= (cobo,asad) 最大)のもの。
      // TPCReco も「フラグメントを回して最後に立った値」を採る —— 我々は並びが
      // 決定的(SPEC §6.3 v1.3)なので、同じイベントなら常に同じ値になる。
      event_time = f.event_time;
    }

    eventraw::EventInfo info;
    info.SetEventId(ev.event_idx);
    info.SetEventTimestamp(static_cast<ULong_t>(event_time));
    info.SetRunId(run_id_);
    info.SetPedestalSubtracted(cfg_.fill.remove_pedestal);
    // eventType / global_properties は 0 のまま(実運用の grawToEventTPC と同一 ——
    // 埋めるのは解析側)。
    pevent_->SetEventInfo(info);

    tree_->Fill();
    entries_written_.fetch_add(1, std::memory_order_relaxed);
    ++part_entries_;
    last_event_id_ = ev.event_idx;
    have_last_event_id_ = true;
    // 充填カウンタを他スレッドから読める場所へ積み替える(イベント毎に 1 回)。
    channels_without_strip_.store(filler_->channels_without_strip(),
                                  std::memory_order_relaxed);
    charge_keys_out_of_range_.store(filler_->keys_out_of_range(), std::memory_order_relaxed);
    frames_outside_geometry_.store(filler_->frames_outside_geometry(),
                                   std::memory_order_relaxed);
    // 100 イベント毎に basket を吐く(ConvertGrawFile.cpp:85 と同じ)。
    if (part_entries_ % kFlushBasketsEvery == 0) tree_->FlushBaskets();
  }

  // 現在の part を書いて閉じる。finalize=true なら最終名へ rename。
  void close_part(bool finalize) {
    if (file_ == nullptr) return;
    file_->cd();
    if (cfg_.format == OutputFormat::PEvent) {
      // 「最新版のツリーだけを残す」(ConvertGrawFile.cpp:92 と同じ)。AutoSave が
      // 書いたキーの隣に ;2 を作らない。
      tree_->Write("", TObject::kOverwrite);
    } else {
      tree_->Write();
    }
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
  GET::GDataFrame* frame_ = nullptr;  // 取り込みの内部表現(プロセス内で 1 個だけ)
  PEventTPC* pevent_ = nullptr;       // PEvent モードのブランチバッファ
  std::unique_ptr<tpcpevent::Filler> filler_;  // PEvent モードの充填器

  std::string provisional_;
  uint32_t run_ = 0;
  bool run_active_ = false;
  long run_id_ = 0;                // run 開始時刻の %Y%m%d%H%M%S(EventInfo::runId)
  uint32_t last_event_id_ = 0;     // PEvent モードで最後に書いた eventId
  bool have_last_event_id_ = false;
  uint32_t part_ = 0;       // 開いている part
  uint32_t next_part_ = 0;  // 次に開く part
  uint64_t part_entries_ = 0;
  uint64_t run_events_ = 0;    // この run に書いたイベント数(0 なら monitor.root も作らない)
  uint64_t closed_bytes_ = 0;  // 閉じた part のバイト合計(bytes_written の土台)
  std::atomic<uint64_t> entries_written_{0};
  std::atomic<uint64_t> bytes_written_{0};
  std::atomic<uint64_t> items_out_of_range_{0};
  std::atomic<uint64_t> duplicate_event_ids_{0};
  std::atomic<uint64_t> channels_without_strip_{0};
  std::atomic<uint64_t> charge_keys_out_of_range_{0};
  std::atomic<uint64_t> frames_outside_geometry_{0};
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
