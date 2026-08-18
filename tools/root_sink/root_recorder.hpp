// root_recorder.hpp — イベント TTree の書き出し(SPEC §6.4)+ ファイルライフサイクル
// (SPEC §6.5)。**ROOT に依存するヘッダ**(相方は pevent_fill.hpp)。
//
// 出自: delila-rs `tools/root_sink/root_sink.cxx` の `Recorder` クラスの流儀
// (inprogress 名で開く → EOS で rename、AutoSave、開いている TFile を 1 スレッドに
// 隔離する)を踏襲。移植にあたっての変更点は下の「delila-rs との差」を参照。
//
// **出力形式は 1 つ(SPEC §6.4 v1.17、TODO/054)**: TPCReco `grawToEventTPC` と
// 同一形式の PEventTPC —— ツリー `TPCData` / ブランチ `Event`(128000, splitlevel 2)/
// **1 エントリ = 1 ビルド済みイベント** / eventId は一回きり(遅延分は書かず数える)/
// `myChargeArray*` ブランチ無効 / 100 イベント毎 FlushBaskets / `Write("", kOverwrite)`。
// 出自 = `EventSources/src/ConvertGrawFile.cpp:40-45,84-92`。
// v1.7 までの GDataFrame 出力(ツリー `tree` / 1 エントリ = 1 フラグメント)と、その
// テスト専用モード `--format gdataframe` は **v1.17 で撤去**(ユーザー裁定 2026-08-15:
// GDataFrame は graw2root = GET 付属の別ツールの形式であり我々のチェーンには不要。
// v1.8 が定めた削除条件「PEventTPC の同 run 実データオラクルが閉じたら」は 021 の
// `compared 3852 events, 0 differences` で成立済み)。
//
// **このヘッダを include してよいのは root_sink.cxx / test_recorder.cxx /
// test_pevent.cxx だけ**。tpc_wire / rs_core / eb_core の 3 ヘッダは ROOT 非依存の
// まま(発注書 §3)。
//
// --- delila-rs との差(意図的なもの)---------------------------------------
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
//     **数える**(`items_out_of_range()` —— 計数の実体は Filler にある)。
//
// --- P1 並列化(TODO/064)-----------------------------------------------------
// `--recorder-workers N` で **worker 毎に TTree 一式を専有**する(裁定 = archive/055 案 P1)。
//   * `Recorder` = dispatcher。run ライフサイクル / 重複 eventId 判定 / round-robin 分配 /
//     カウンタ合算 / monitor.root。**ROOT の TFile には触らない**。
//   * `RecorderWorker` = TFile + TTree + PEventTPC + Filler を専有。part 命名と
//     ロールオーバ、AutoSave、Filler カウンタ。
//   * **N=1 は現行と完全同一**: worker はスレッドもキューも持たず、呼び手のスレッド上で
//     直接呼ばれる(出力名も `run{run:04}.root` のまま)。既存テストが無改変で通ることが
//     その証明。N>1 では worker k が `run{run:04}_w{k}.root` + `_w{k}_{part:04}.root`。
//   * ファイル横断の全順序は保証しない(worker 内は eventIdx 単調 —— round-robin なので
//     自然に成立)。イベントは自立しており、オラクル(compare_pevent)は eventId キーの
//     内容一致なので問題ない。

#ifndef TPCDAQ_ROOT_SINK_ROOT_RECORDER_HPP
#define TPCDAQ_ROOT_SINK_ROOT_RECORDER_HPP

#include <dirent.h>
#include <sys/stat.h>
#include <unistd.h>

#include <algorithm>
#include <atomic>
#include <cerrno>
#include <chrono>
#include <cinttypes>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <future>
#include <limits>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <utility>
#include <vector>

#include <TFile.h>
#include <TH1D.h>
#include <TH2D.h>
#include <TObject.h>
#include <TROOT.h>
#include <TTree.h>

#include "eb_core.hpp"
#include "monitor_hist.hpp"
#include "pevent_fill.hpp"
#include "rs_core.hpp"

namespace rootsink {

// SPEC §6.4 / §6.5 の定数(一字違わず合わせる)。
// **TPCReco `EventSourceROOT` がハード期待する形**
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
// `--root-imt` の既定(TODO/054-B)。ROOT のバスケット圧縮スレッド数。0 = 無効。
// 隔離プローブの実測(mini 実サイズ 200 イベント、macOS 14 コア / ROOT 6.36):
// off 17.13 / 2 本 16.44 / **4 本 16.06** / 8 本 16.08 / 14 本 16.09 ms/event ——
// 4 本で頭打ちなので既定を 4 にする。効きが小さい理由は root_sink.cxx の注記。
constexpr int kDefaultImtThreads = 4;
constexpr uint64_t kDefaultMaxRootBytes = 1ULL << 30;  // 1 GiB(--max-root-bytes 既定)
// `--recorder-workers` の既定(TODO/064)。**1 = 現行と完全同一**(スレッドもキューも
// 作らない)。N>1 で worker 毎 TTree の並列書き出しに入る。
constexpr int kDefaultRecorderWorkers = 1;
// worker 1 本あたりの有界キュー段数(N>1 のときだけ使う)。**満杯なら分配側が待つ** =
// 背圧(意図的ドロップ禁止 — CLAUDE.md)。ELITPC 2.2 MB/event × 4 段 × worker 数 が
// 在庫の上限。round-robin の粒度ゆらぎを吸うために 1 ではなく 4。
constexpr std::size_t kWorkerQueueDepth = 4;
// worker スレッドの起床間隔(AutoSave の面倒を見るため。root_sink.cxx の
// kRecorderTickMs と同じ 100 ms)。
constexpr int kWorkerTickMs = 100;
// AutoSave 間隔(delila-rs Recorder 由来 / SPEC §6.1)。run 中でも inprogress を
// ROOT で開けるようにしておく = 異常終了時にそこまでのデータが読める。
constexpr uint64_t kAutoSaveIntervalMs = 30000;

// モニタヒストの 1D 軸レンジ(SPEC §5.2「x レンジ 0–4096 固定。オートレンジ禁止 ——
// 飽和天井 4095 が常に見えること」)。TODO/022。
constexpr double kChargeAxisMax = 4096.0;

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
  // --run-id(TODO/021): EventInfo.runId の上書き。0 = run を開いた時刻から生成。
  long run_id_override = 0;
  // **必須**(strip 射影に要る)。呼び手が寿命を持つこと。
  const tpcgeo::Geometry* geometry = nullptr;       // --geometry
  tpcpevent::FillConfig fill;                       // --pedestal-remove / --*-cell
  // --recorder-workers(TODO/064)。1 = 現行と完全同一。**ジオメトリは worker 間で
  // read-only 共有**(Filler と PEventTPC だけが worker 専有)。
  int workers = kDefaultRecorderWorkers;
};

// `<output_root>/run{run:04}`(SPEC §6.5: run 毎ディレクトリ)。
inline std::string run_dir_of(const std::string& output_root, uint32_t run) {
  char buf[32];
  std::snprintf(buf, sizeof(buf), "run%04u", run);
  return output_root + "/" + buf;
}

// worker スレッドが AutoSave の期限判定に使う単調時計。**N=1 では読まない**
// (呼び手が now_ms を注入する現行の作法を崩さない —— テストが sleep 無しで書ける)。
// N>1 の worker はキューの向こう側で自走するので、自分で時計を読むしかない
// (root_sink.cxx の Recorder スレッドが tick() 用に now_ms() を読んでいたのと同じ)。
inline uint64_t steady_now_ms() {
  const auto since_epoch = std::chrono::steady_clock::now().time_since_epoch();
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::milliseconds>(since_epoch).count());
}

// JSON の root_files 要素(SPEC の「何をどこに何件書いたか」の可視化)。
struct RootFileRecord {
  std::string path;      // 最終パス。finalize しなかった場合は inprogress のまま
  uint64_t entries = 0;  // そのファイルのエントリ数(= フレーム数)
  uint64_t bytes = 0;    // close 直後の実サイズ
};

// ---------------------------------------------------------------------------
// RecorderWorker — TFile/TTree/PEventTPC/Filler を専有する 1 本の書き手
// ---------------------------------------------------------------------------
//
// **このオブジェクトの ROOT 資源に触るスレッドは常に 1 本**:
//   * N=1 … 呼び手のスレッド(現行と完全同一。スレッドもキューも作らない)
//   * N>1 … 自前の worker スレッド(有界キューの向こう側)
// run の切替(BeginRun)も finalize(CloseRun)も **in-band** でキューに流すので、
// キューに残ったイベントを追い越して閉じることはない(root_sink.cxx が RunClose
// マーカーを in-band にしてあるのと同じ理屈)。
class RecorderWorker;

// worker への荷物(N>1 のときだけキューに乗る)。
struct WorkerItem {
  enum class Kind : uint8_t { Event, BeginRun, CloseRun };
  Kind kind = Kind::Event;
  BuiltEvent event;   // Kind::Event
  uint32_t run = 0;   // Kind::BeginRun
  long run_id = 0;    // Kind::BeginRun
  uint64_t now_ms = 0;
  // Kind::CloseRun のとき、処理し終えたことを分配側へ知らせる(flush → join の同期)。
  std::shared_ptr<std::promise<void>> done;
};

class RecorderWorker {
 public:
  // `suffixed` = 出力名に `_w{index}` を挟むか(N>1 のときだけ true)。
  RecorderWorker(const RecorderConfig& cfg, int index, bool suffixed)
      : cfg_(cfg), index_(index), suffixed_(suffixed) {
    pevent_ = new PEventTPC();
    filler_.reset(new tpcpevent::Filler(*cfg_.geometry, cfg_.fill));
  }

  ~RecorderWorker() {
    stop_thread();
    shutdown();
    delete pevent_;
    pevent_ = nullptr;
  }

  RecorderWorker(const RecorderWorker&) = delete;
  RecorderWorker& operator=(const RecorderWorker&) = delete;

  // --- N>1 のときだけ使う自走モード -----------------------------------------
  void start_thread() {
    chan_.reset(new Channel<WorkerItem>(kWorkerQueueDepth));
    thread_ = std::thread([this] { loop(); });
  }

  // 満杯なら**待つ** = 背圧(捨てない)。
  void submit(WorkerItem&& item) { chan_->push(std::move(item)); }

  // キューを流し切ってからスレッドを畳む(残りは書く —— ロスレス)。
  void stop_thread() {
    if (!thread_.joinable()) return;
    chan_->close();
    thread_.join();
  }

  // --- 実処理(N=1 では呼び手のスレッドから直接、N>1 では loop() から)-------

  // run が変わった = 前の run は close_run 済みのはず。part を 0 に戻す。
  //
  // **ここに来るのは本来 root_sink.cxx の consume() が run_number_mismatch の
  // 増分を検知して fatal(exit 6)にする経路(SPEC §6.2-5 v1.10、R-P2-1)** —— この
  // 分岐は到達不能になったはずの防御。それでも万一(上流のプロトコル違反が
  // fatal 化をすり抜けた等)ここに来たら、**finalize しない**(旧 run を
  // 完成 run 名に化けさせない。§6.5「異常終了は inprogress のまま」と同じ扱い)。
  void begin_run(uint32_t run, long run_id) {
    if (file_ != nullptr) {
      std::fprintf(stderr,
                   "root_sink: PROTOCOL VIOLATION recorder saw run %u while run %u "
                   "was still open (this should have been fatal upstream, SPEC "
                   "6.2-5 v1.10) — keeping %u inprogress, NOT finalizing it\n",
                   run, run_, run_);
      close_part(/*finalize=*/false);
    }
    run_ = run;
    run_id_ = run_id;
    next_part_ = 0;
    run_entries_.store(0, std::memory_order_relaxed);
  }

  // **1 エントリ = 1 ビルド済みイベント**(SPEC §6.4)。分配側(Recorder)が
  // fatal / 空フラグメント / 重複 eventId を既に弾いている。
  void write(const BuiltEvent& ev, uint64_t now_ms) {
    if (fatal_.load(std::memory_order_acquire) != nullptr) return;
    if (file_ == nullptr && !open_part(now_ms)) return;

    fill_pevent(ev);
    // この run に何か書いたか(0 イベント run の判定 — SPEC §6.5)。
    run_entries_.fetch_add(1, std::memory_order_relaxed);

    // `bytes_written`(status の材料、SPEC §5.3)。GetEND() = 現在のファイル末尾
    // オフセットなので、閉じた part の合計 + 開いている part の現在サイズ。
    // **読むのは Publisher スレッド**なので atomic に置く(ROOT には触らせない)。
    if (file_ != nullptr) {
      bytes_written_.store(closed_bytes_ + static_cast<uint64_t>(file_->GetEND()),
                           std::memory_order_relaxed);
    }

    // R-P2-2: write() 単独でも AutoSave の期限を見る(呼び手の tick() だけに頼ると、
    // データが途切れない run では 1 度も走らなかった —— P2_REVIEW.md R-P2-2)。
    maybe_autosave(now_ms);

    // ロールオーバ判定は **イベント単位**(フレーム単位ではなく)—— 1 イベントの
    // フラグメントが 2 ファイルに割れないようにする。GetEND() = 現在のファイル末尾
    // オフセット = 実ファイルサイズ。
    if (file_ != nullptr && static_cast<uint64_t>(file_->GetEND()) > cfg_.max_root_bytes) {
      close_part(/*finalize=*/true);
      // 次の part はここでは開かない。**次の write() まで開かない**ので、
      // run の最後がロールオーバで終わっても空ファイルが残らない。
    }
  }

  // EOS: 開いていれば finalize(最終名へ rename)。
  void close_run() {
    if (file_ != nullptr) close_part(/*finalize=*/true);
  }

  // 停止: 開いていれば **inprogress のまま**閉じる(SPEC §6.5)。
  void shutdown() {
    if (file_ != nullptr) close_part(/*finalize=*/false);
  }

  void tick(uint64_t now_ms) { maybe_autosave(now_ms); }

  // --- 状態・カウンタ(分配側 / Publisher スレッドから読まれる = すべて atomic)---
  // `file_` そのものは worker スレッドの持ち物なので、**外から見る開閉は別の atomic**
  // (生ポインタを他スレッドから読むと data race になる)。
  bool is_open() const { return open_.load(std::memory_order_relaxed); }
  uint64_t run_entries() const { return run_entries_.load(std::memory_order_relaxed); }
  uint64_t entries_written() const { return entries_written_.load(std::memory_order_relaxed); }
  uint64_t bytes_written() const { return bytes_written_.load(std::memory_order_relaxed); }
  uint64_t items_out_of_range() const {
    return items_out_of_range_.load(std::memory_order_relaxed);
  }
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
  const char* fatal_reason() const { return fatal_.load(std::memory_order_acquire); }
  // `fatal_reason()` が非 nullptr を返した後にだけ読むこと(detail は fatal_ より
  // 先に書かれる = acquire/release で見える)。
  const std::string& fatal_detail() const { return fatal_detail_; }

 private:
  void loop() {
    WorkerItem item;
    for (;;) {
      const PopResult pr = chan_->pop_for(item, kWorkerTickMs);
      if (pr == PopResult::Value) {
        switch (item.kind) {
          case WorkerItem::Kind::BeginRun:
            begin_run(item.run, item.run_id);
            break;
          case WorkerItem::Kind::CloseRun:
            close_run();
            break;
          case WorkerItem::Kind::Event:
            write(item.event, item.now_ms);
            break;
        }
        // **必ず**知らせる(fatal でも)—— 分配側を待たせたまま死なせない。
        if (item.done) item.done->set_value();
        item.done.reset();
      } else {
        // データが来なくても AutoSave は進む(自走モードでは自分で時計を読む)。
        tick(steady_now_ms());
      }
      if (pr == PopResult::Closed) break;
    }
    // 停止時に run が finalize されていなければ inprogress のまま閉じる(SPEC §6.5)。
    shutdown();
  }

  // `run{run:04}.root`(part 0)/ `run{run:04}_{part:04}.root`(part 1〜)。
  // N>1 では worker 番号を挟む: `run{run:04}_w{k}.root` / `run{run:04}_w{k}_{part:04}.root`。
  std::string final_path(uint32_t run, uint32_t part) const {
    char buf[80];
    if (suffixed_) {
      if (part == 0) {
        std::snprintf(buf, sizeof(buf), "run%04u_w%d.root", run, index_);
      } else {
        std::snprintf(buf, sizeof(buf), "run%04u_w%d_%04u.root", run, index_, part);
      }
    } else if (part == 0) {
      std::snprintf(buf, sizeof(buf), "run%04u.root", run);
    } else {
      std::snprintf(buf, sizeof(buf), "run%04u_%04u.root", run, part);
    }
    return run_dir_of(cfg_.output_root, run) + "/" + buf;
  }

  // 書き込み中の名前 `run_inprogress_<unixtime>.root`(SPEC §6.5)。
  // 同じ秒に 2 個目が要る場合(小さい --max-root-bytes での連続ロールオーバや、
  // 前回の異常終了の残骸)は `_2`, `_3` … を足す —— **既存ファイルを RECREATE で
  // 踏み潰さない**(それは黙ってデータを捨てるのと同じ)。
  // N>1 では worker 番号を先に挟む —— 同じ秒に別 worker が開く衝突を、名前空間を
  // 分けることで**構造的に**避ける(`_2` の探索は同一スレッド内でしか安全でない)。
  std::string pick_provisional(uint32_t run) const {
    const std::string dir = run_dir_of(cfg_.output_root, run);
    char buf[80];
    if (suffixed_) {
      std::snprintf(buf, sizeof(buf), "/run_inprogress_w%d_%lld", index_,
                    static_cast<long long>(std::time(nullptr)));
    } else {
      std::snprintf(buf, sizeof(buf), "/run_inprogress_%lld",
                    static_cast<long long>(std::time(nullptr)));
    }
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
    const std::string dir = run_dir_of(cfg_.output_root, run_);
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
    open_.store(true, std::memory_order_relaxed);  // 以降は close_part() が false に戻す
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
    part_ = next_part_++;
    part_entries_ = 0;
    last_autosave_ms_ = now_ms;
    std::fprintf(stderr, "root_sink: recording run %u part %u -> %s\n", run_, part_,
                 provisional_.c_str());
    return true;
  }

  // **1 エントリ = 1 ビルド済みイベント**(SPEC §6.3/§6.4)。フラグメントは
  // (cobo,asad) 昇順に並んでいる(eb_core が保証)—— Filler が順に**直読**して
  // chargeMap に足し込む(`AddValByStrip` は `+=`。v1.17 で GDataFrame 中間表現を撤去)。
  void fill_pevent(const BuiltEvent& ev) {
    pevent_->Clear();
    uint64_t event_time = 0;
    for (const OwnedFragment& f : ev.fragments) {
      filler_->add_fragment(f, *pevent_);
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
    // 充填カウンタを他スレッドから読める場所へ積み替える(イベント毎に 1 回)。
    items_out_of_range_.store(filler_->items_out_of_range(), std::memory_order_relaxed);
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
    // 「最新版のツリーだけを残す」(ConvertGrawFile.cpp:92 と同じ)。AutoSave が
    // 書いたキーの隣に ;2 を作らない。
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
    if (fatal_.load(std::memory_order_relaxed) != nullptr) return;
    fatal_detail_ = detail;  // **理由より先に**書く(読み手は acquire で両方見える)
    fatal_.store(reason, std::memory_order_release);
    std::fprintf(stderr, "root_sink: FATAL %s: %s\n", reason, detail.c_str());
  }

  // AutoSave の期限判定(write()/tick() 共通、R-P2-2)。**write() 側からも呼ぶのが
  // 本ユニットの修正点**(P2_REVIEW.md R-P2-2)—— 以前は呼び手の tick() 頼みで、
  // データが途切れない run では AutoSave が一度も走らなかった(kill -9 / 電源断時の
  // inprogress 回復性が下がる。生 graw がバックストップなのでデータ喪失ではないが、
  // run.root だけでも途中まで読めた方が安全 — SPEC §6.1)。
  void maybe_autosave(uint64_t now_ms) {
    if (file_ == nullptr || tree_ == nullptr) return;
    if (now_ms < last_autosave_ms_ + kAutoSaveIntervalMs) return;
    tree_->AutoSave("SaveSelf");  // ツリーとキーを書くがファイルは閉じない
    last_autosave_ms_ = now_ms;
  }

  const RecorderConfig cfg_;
  const int index_;
  const bool suffixed_;

  TFile* file_ = nullptr;
  TTree* tree_ = nullptr;
  PEventTPC* pevent_ = nullptr;                // ブランチバッファ
  std::unique_ptr<tpcpevent::Filler> filler_;  // Fragment → chargeMap の充填器

  std::unique_ptr<Channel<WorkerItem>> chan_;  // N>1 のときだけ
  std::thread thread_;                         // N>1 のときだけ

  std::string provisional_;
  uint32_t run_ = 0;
  long run_id_ = 0;         // run 開始時刻の %Y%m%d%H%M%S(EventInfo::runId)
  uint32_t part_ = 0;       // 開いている part
  uint32_t next_part_ = 0;  // 次に開く part
  uint64_t part_entries_ = 0;
  uint64_t closed_bytes_ = 0;  // 閉じた part のバイト合計(bytes_written の土台)
  uint64_t last_autosave_ms_ = 0;
  std::atomic<bool> open_{false};  // file_ != nullptr の外向きの影(上の is_open() 参照)
  std::atomic<uint64_t> run_entries_{0};
  std::atomic<uint64_t> entries_written_{0};
  std::atomic<uint64_t> bytes_written_{0};
  std::atomic<uint64_t> items_out_of_range_{0};
  std::atomic<uint64_t> channels_without_strip_{0};
  std::atomic<uint64_t> charge_keys_out_of_range_{0};
  std::atomic<uint64_t> frames_outside_geometry_{0};
  mutable std::mutex files_mu_;
  std::vector<RootFileRecord> files_;
  std::atomic<const char*> fatal_{nullptr};
  std::string fatal_detail_;
};

// ---------------------------------------------------------------------------
// Recorder — 分配役。run ライフサイクルと重複判定を一元化し、ROOT には触らない
// ---------------------------------------------------------------------------
//
// SPEC §6.2-8「run 境界でブロッキング外部 IO 禁止」の実装手段: finalize / rename /
// 次ファイルの open はすべて worker の中で起き、それは worker のスレッドの中だけ。
// 取り込み(ZMQ 受信 → 集計)は有界 Channel の向こう側なので止まらない。
class Recorder {
 public:
  explicit Recorder(RecorderConfig cfg) : cfg_(std::move(cfg)) {
    if (cfg_.output_root.empty()) cfg_.output_root = ".";
    if (cfg_.workers < 1) cfg_.workers = 1;
    // 自動分割を殺す(発注書 §2: 命名規則を ROOT 任せにしない)。
    TTree::SetMaxTreeSize(std::numeric_limits<Long64_t>::max());
    if (cfg_.geometry == nullptr) {
      // ジオメトリ無しで PEventTPC は書けない(strip 射影ができない)。
      // 呼び手が CLI で弾くのが本筋だが、黙って空を書かないための最後の砦。
      set_fatal("pevent-no-geometry", "PEventTPC output needs a geometry (--geometry)");
      return;
    }
    // TFile/TTree を複数スレッドが同時に触る(worker 専有だが ROOT のグローバル
    // 資源 —— TClass / gDirectory / TROOT のリスト —— は共有)。**N=1 では呼ばない**
    // (現行と完全同一の挙動を保つ。root_sink.cxx は従来どおり自前で呼ぶ)。
    const bool parallel = cfg_.workers > 1;
    if (parallel) ROOT::EnableThreadSafety();
    for (int k = 0; k < cfg_.workers; ++k) {
      workers_.emplace_back(new RecorderWorker(cfg_, k, /*suffixed=*/parallel));
    }
    if (parallel) {
      for (auto& w : workers_) w->start_thread();
      std::fprintf(stderr, "root_sink: recorder workers=%d (per-worker TTree, queue=%zu)\n",
                   cfg_.workers, kWorkerQueueDepth);
    }
  }

  ~Recorder() { shutdown(); }

  Recorder(const Recorder&) = delete;
  Recorder& operator=(const Recorder&) = delete;

  int workers() const { return static_cast<int>(workers_.size()); }

  // 組み上がったイベントを書く。**1 エントリ = 1 ビルド済みイベント**(SPEC §6.4)。
  // 既に書いた eventId(= 遅延到着)は**書かずに数える**(SPEC §6.3 v1.8)。
  void write(const BuiltEvent& ev, uint64_t now_ms) {
    if (!admit(ev, now_ms)) return;
    if (workers_.size() == 1) {
      workers_[0]->write(ev, now_ms);  // N=1: 呼び手のスレッドで直接(現行と同一)
      return;
    }
    WorkerItem item;
    item.kind = WorkerItem::Kind::Event;
    item.event = ev;  // const 参照からはコピーするしかない(rvalue 版は下)
    item.now_ms = now_ms;
    dispatch(std::move(item));
  }

  // ホットパス用の move 版(root_sink.cxx はこちらを使う)。N>1 でもコピーしない。
  void write(BuiltEvent&& ev, uint64_t now_ms) {
    if (!admit(ev, now_ms)) return;
    if (workers_.size() == 1) {
      workers_[0]->write(ev, now_ms);
      return;
    }
    WorkerItem item;
    item.kind = WorkerItem::Kind::Event;
    item.event = std::move(ev);
    item.now_ms = now_ms;
    dispatch(std::move(item));
  }

  // 全ソース EOS(run が閉じた)→ 全 worker を flush してから finalize。
  void close_run(uint32_t run_number, uint64_t now_ms) {
    (void)now_ms;
    const bool wrote_something = close_run_all();
    if (!wrote_something && (!run_active_ || run_ != run_number)) {
      // データが 1 件も来なかった run。黙って通さない(CLAUDE.md)。
      std::fprintf(stderr, "root_sink: run %u closed with no ROOT file (no events)\n",
                   run_number);
    }
    run_active_ = false;
  }

  // モニタヒスト 9 枚を `run{run:04}_monitor.root` に書く(SPEC §5.2/§6.5、TODO/022)。
  //
  // **呼ぶのは close_run() の直後**(TTree finalize 後 = 全 worker が静止した後)。
  // 集計は monitor_hist.hpp が持ち、ROOT 化はここ = 分配側スレッドだけ ——
  // **worker 数によらず run 毎に 1 つ**(移送チェックリスト③)。
  // R10(EOS から 10 s 以内)は即時書きで満たす。
  //
  // **0 イベントの run には書かない**(§6.5 の遅延オープンと同じ理屈)。run が
  // 閉じないまま停止した場合は呼ばれない(RunClose マーカー経由でしか来ない)ので、
  // 未完了 run のヒストが完全 run に化けることもない。
  void write_monitor_root(uint32_t run, const rsmon::HistSnapshot& hists) {
    if (fatal_reason() != nullptr) return;
    if (run_ != run || run_entries() == 0) {
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

  // 停止(SIGTERM / 異常)。全 worker のキューを流し切ってから、開いているパートを
  // **inprogress のまま**閉じる —— finalize していない run が完全 run に化けない
  // (SPEC §6.5)。在庫は捨てない(保存系ロスレス — CLAUDE.md)。
  // **何度呼んでも安全**(デストラクタからも呼ぶ)。
  void shutdown() {
    if (workers_.size() == 1) {
      workers_[0]->shutdown();
    } else {
      // close → drain → close_part(false) → join(worker の loop() 末尾)。
      for (auto& w : workers_) w->stop_thread();
    }
    run_active_ = false;
  }

  // データが来ない間も呼ばれる(呼び手の tick)。AutoSave の面倒だけ見る。
  // **write() 側からも同じ期限判定を呼ぶ**(R-P2-2、maybe_autosave() 参照)ので、
  // ここは薄い委譲になっている。**N>1 では no-op** —— worker が自分のキューの
  // タイムアウトで自走する(そちらは自分で時計を読む)。
  void tick(uint64_t now_ms) {
    if (workers_.size() == 1) workers_[0]->tick(now_ms);
  }

  // --- 状態・カウンタ ---
  //
  // カウンタは **他スレッドから読まれる**(root_sink.cxx は fatal 時に Recorder
  // スレッドを join できないまま JSON を出す —— 「落ちた瞬間のカウンタを捨てない」)。
  // worker はローカルの atomic に持ち、ここで**全 worker 合算**して返す(発注書 §4)。
  // ロックは**ファイルを閉じるときだけ**なので、ホットパス(fill)には乗らない。
  bool is_open() const {
    for (const auto& w : workers_) {
      if (w->is_open()) return true;
    }
    return false;
  }
  uint64_t entries_written() const {
    return sum([](const RecorderWorker& w) { return w.entries_written(); });
  }
  // イベント ROOT に書いた実バイト数(閉じた part の合計 + 開いている part の現在サイズ)。
  // status の `bytes_written`(SPEC §5.3)—— **Publisher スレッドから読まれる**ので
  // atomic。monitor.root の分は含めない(保存系のスループットを表す数字なので)。
  uint64_t bytes_written() const {
    return sum([](const RecorderWorker& w) { return w.bytes_written(); });
  }
  uint64_t items_out_of_range() const {
    return sum([](const RecorderWorker& w) { return w.items_out_of_range(); });
  }
  // 「既に書いた eventId」を弾いた回数(遅延到着・重複)。**分配前の一元判定**
  // (worker に散らした後では「以前に書いた番号」が見えない — 移送チェックリスト②)。
  uint64_t duplicate_event_ids() const {
    return duplicate_event_ids_.load(std::memory_order_relaxed);
  }
  // 充填カウンタ(AGET の 68 ch を外れた item / strip でないチャンネル /
  // PEventTPC の固定長配列をはみ出す key / ジオメトリ外フレーム)。
  // **Filler 側は素の整数**(per-sample の atomic はホットパスに置かない)。
  // イベント毎に 1 回 worker の atomic へ積み替え、ここで合算する。
  uint64_t channels_without_strip() const {
    return sum([](const RecorderWorker& w) { return w.channels_without_strip(); });
  }
  uint64_t charge_keys_out_of_range() const {
    return sum([](const RecorderWorker& w) { return w.charge_keys_out_of_range(); });
  }
  uint64_t frames_outside_geometry() const {
    return sum([](const RecorderWorker& w) { return w.frames_outside_geometry(); });
  }
  // 全 worker のパート台帳を worker 順に連結(移送チェックリスト④)。
  std::vector<RootFileRecord> files_snapshot() const {
    std::vector<RootFileRecord> out;
    for (const auto& w : workers_) {
      const std::vector<RootFileRecord> part = w->files_snapshot();
      out.insert(out.end(), part.begin(), part.end());
    }
    return out;
  }
  // 非 nullptr なら ROOT IO が失敗している。呼び手はカウンタを出して即死すること。
  // 自分 → worker 0,1,… の順に最初の 1 件(reason/detail で走査順を揃える)。
  const char* fatal_reason() const {
    if (fatal_ != nullptr) return fatal_;
    for (const auto& w : workers_) {
      const char* r = w->fatal_reason();
      if (r != nullptr) return r;
    }
    return nullptr;
  }
  const std::string& fatal_detail() const {
    if (fatal_ == nullptr) {
      for (const auto& w : workers_) {
        if (w->fatal_reason() != nullptr) return w->fatal_detail();
      }
    }
    return fatal_detail_;
  }

  // `<output_root>/run{run:04}`(SPEC §6.5: run 毎ディレクトリ)。
  std::string run_dir(uint32_t run) const { return run_dir_of(cfg_.output_root, run); }

  // 開いている inprogress のパス(N=1 前提のテスト用 —— worker 0 のもの)。
  const std::string& provisional() const {
    static const std::string kNone;
    return workers_.empty() ? kNone : workers_[0]->provisional();
  }

 private:
  template <typename F>
  uint64_t sum(F get) const {
    uint64_t total = 0;
    for (const auto& w : workers_) total += get(*w);
    return total;
  }

  // この run に(全 worker 合わせて)何件書いたか。0 イベント run の判定 —— SPEC §6.5。
  uint64_t run_entries() const {
    return sum([](const RecorderWorker& w) { return w.run_entries(); });
  }

  // 分配の前段。run ライフサイクルと重複 eventId を**ここ 1 か所**で見る。
  // false = 書かない(fatal / 空 / 重複)。
  bool admit(const BuiltEvent& ev, uint64_t now_ms) {
    (void)now_ms;
    if (fatal_reason() != nullptr) return false;  // 既に死んでいる(呼び手が exit する)
    if (ev.fragments.empty()) return false;
    if (!run_active_ || run_ != ev.run_number) {
      run_ = ev.run_number;
      run_active_ = true;
      have_last_event_id_ = false;
      // runId = **run 開始時刻**の %Y%m%d%H%M%S(TPCReco RunIdParser と同じ導出、
      // SPEC §6.4)。run 中は一定 —— ここで 1 回だけ決める。--run-id 指定時は
      // その値(実データ照合と P4 controller 経路の受け口、TODO/021)。
      run_id_ = cfg_.run_id_override != 0 ? cfg_.run_id_override : tpcpevent::run_id_now();
      begin_run_all();  // **in-band**(キューに残ったイベントを追い越さない)
      std::fprintf(stderr, "root_sink: run %u runId=%ld\n", run_, run_id_);
    }

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
      return false;
    }
    last_event_id_ = ev.event_idx;
    have_last_event_id_ = true;
    return true;
  }

  // round-robin(発注書 §1)。**満杯なら push が待つ** = 背圧。
  void dispatch(WorkerItem&& item) {
    workers_[next_worker_]->submit(std::move(item));
    next_worker_ = (next_worker_ + 1) % workers_.size();
  }

  void begin_run_all() {
    if (workers_.size() == 1) {
      workers_[0]->begin_run(run_, run_id_);
      return;
    }
    for (auto& w : workers_) {
      WorkerItem item;
      item.kind = WorkerItem::Kind::BeginRun;
      item.run = run_;
      item.run_id = run_id_;
      w->submit(std::move(item));
    }
  }

  // 全 worker のキューを流し切って(in-band CloseRun)全パートを finalize する。
  // 戻り値 = この run に 1 件でも書いた worker があったか。
  bool close_run_all() {
    if (workers_.size() == 1) {
      workers_[0]->close_run();
    } else {
      std::vector<std::future<void>> waits;
      waits.reserve(workers_.size());
      for (auto& w : workers_) {
        WorkerItem item;
        item.kind = WorkerItem::Kind::CloseRun;
        item.done = std::make_shared<std::promise<void>>();
        waits.push_back(item.done->get_future());
        w->submit(std::move(item));
      }
      // 在庫を吐き切るまで待つ(捨てない = ロスレス)。
      for (std::future<void>& f : waits) f.wait();
    }
    return run_entries() > 0;
  }

  void set_fatal(const char* reason, const std::string& detail) {
    if (fatal_ != nullptr) return;
    fatal_ = reason;
    fatal_detail_ = detail;
    std::fprintf(stderr, "root_sink: FATAL %s: %s\n", reason, detail.c_str());
  }

  RecorderConfig cfg_;
  std::vector<std::unique_ptr<RecorderWorker>> workers_;
  std::size_t next_worker_ = 0;

  uint32_t run_ = 0;
  bool run_active_ = false;
  long run_id_ = 0;             // run 開始時刻の %Y%m%d%H%M%S(EventInfo::runId)
  uint32_t last_event_id_ = 0;  // 最後に**分配した** eventId
  bool have_last_event_id_ = false;
  std::atomic<uint64_t> duplicate_event_ids_{0};
  const char* fatal_ = nullptr;  // 分配側自身の fatal(ジオメトリ無し)
  std::string fatal_detail_;
};

}  // namespace rootsink

#endif  // TPCDAQ_ROOT_SINK_ROOT_RECORDER_HPP
