// root_sink.cxx — root-sink 取り込み骨格(TODO/008)+ イベントビルダ(TODO/010)
//                 + GDataFrame TTree 書き出し(TODO/011)。
//
// 配線(SPEC §6.2 のロスレス化チェックリスト 1–3・8 の実装点):
//
//   decoder --PUSH--> [ZMQ_PULL bind, RCVHWM 有限] --> 受信ループ(main スレッド)
//        --> Channel<ZmqMsg>(有界・満杯なら push が待つ)--> 集計スレッド
//        --> EventBuilder((run, event_idx) で束ね、event_idx 昇順で emit)
//        --> Channel<RecordItem>(有界)--> **Recorder スレッド**(ROOT IO はここだけ)
//
// 満杯の Channel が受信ループを止め、RCVHWM が埋まり、送り手の PUSH send がブロックする。
// これが「保存系は絶対にデータを落とさない」の実装手段(CLAUDE.md / SPEC §1.4-2)。
// delila-rs の root_sink は SUB + HWM=0(無制限)だったので背圧が存在しなかった —— そこの反転。
//
// **ROOT IO を別スレッドに隔離した理由**(SPEC §6.2-8): run 境界の finalize / rename /
// 次ファイル open は数十 ms 単位で止まりうる。集計スレッドに置くと取り込みが止まり、
// run 境界でのブロッキング外部 IO 禁止に反する。有界 Channel を挟むことで、
// 「詰まったら背圧」(ロスレス)と「run 境界で intake を止めない」を両立させる。
//
// 終了:
//   * SIGINT / SIGTERM → 在庫を吸い切って(取りこぼさない)stdout に JSON 1 行、exit 0。
//     run が finalize されていなければ ROOT ファイルは **inprogress のまま**残す(SPEC §6.5)。
//   * malformed バッチ → 即 exit 2(SPEC §6.2-6。「warn して捨てる」をしない)。
//   * sequence_number のギャップ・巻き戻り → 即 exit 3(SPEC §6.2-5)。
//   * ROOT の書き出し失敗 → 即 exit 5(黙って書けないまま走り続けない)。
//   異常終了時も同じ JSON 1 行を出す("fatal" フィールドに理由が入る)。落ちた瞬間の
//   カウンタは事故調査で一番効くので捨てない(CLAUDE.md「silent failure を作らない」)。

#include <zmq.h>

#include <atomic>
#include <chrono>
#include <cinttypes>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <exception>
#include <memory>
#include <mutex>
#include <optional>
#include <set>
#include <string>
#include <thread>
#include <utility>
#include <vector>

#include "eb_core.hpp"
#include "monitor_hist.hpp"
#include "monitor_pub.hpp"
#include "root_recorder.hpp"
#include "rs_core.hpp"
#include "tpc_wire.hpp"

namespace {

// --------------------------------------------------------------------------
// 既定値(SPEC §3.2 のポート表 / §1.4-2 の HWM)
// --------------------------------------------------------------------------

constexpr const char* kDefaultBind = "tcp://*:47003";  // root-sink PULL bind
constexpr int kDefaultRcvHwm = 1000;                   // 有限(0 = 無制限は方針違反)
constexpr std::size_t kDefaultQueue = 1000;            // 受信 → 集計の有界 Channel
constexpr int kPollTimeoutMs = 100;                    // 停止フラグを見に行く間隔
constexpr int kDrainGraceMs = 200;                     // 停止後、在庫を待つ猶予
// ビルダに壁時計を注入する最大間隔。データが来なくても build_timeout は進まねば
// ならないので、集計スレッドはこの周期で必ず起きる(SPEC §6.3)。
constexpr int kBuilderTickMs = 100;
// Recorder スレッドの起床間隔(AutoSave の面倒を見るため。SPEC §6.1 の 30 s に対して
// 十分細かければよい)。
constexpr int kRecorderTickMs = 100;
// 集計 → Recorder の有界キュー(イベント単位)。フル読み出し 1 フレーム = 68ch × 4aget ×
// 512bucket × 4B ≈ 557 KiB なので、ELITPC(4 フラグメント/イベント)でも
// 16 × 4 × 557 KiB ≈ 35 MiB —— 詰まったら背圧(捨てない)。CLI で開けるノブは増やさない。
constexpr std::size_t kRecorderQueue = 16;

// --- モニタ PUB(TODO/022、SPEC §5.3)---
constexpr const char* kDefaultPubBind = "tcp://*:47004";  // SPEC §3.2
// スナップショットは ~2.4 MB 級(ELITPC の W 面)なので、溜め込ませない。
// モニタ系は「最新優先で落としてよい」側(CLAUDE.md の絶対ルール)。
constexpr int kPubSndHwm = 10;
// Publisher の起床上限。実際は次の配信期限まで寝る(status 1 Hz / snapshot_hz /
// event_publish_hz のうち一番早いもの)。
constexpr int kPubTickMs = 100;
constexpr uint64_t kStatusPeriodMs = 1000;  // status は 1 Hz 固定(run 外でも常時)
constexpr uint32_t kDefaultSnapshotHz = 1;
constexpr uint32_t kDefaultEventPublishHz = 20;

constexpr int kExitUsage = 1;
constexpr int kExitMalformed = 2;  // SPEC §6.2-6
constexpr int kExitSeq = 3;        // SPEC §6.2-5
constexpr int kExitZmq = 4;
constexpr int kExitRootIo = 5;     // ROOT の open / 書き出し失敗(011)

// --------------------------------------------------------------------------
// シグナル
// --------------------------------------------------------------------------

volatile sig_atomic_t g_stop = 0;
void on_signal(int) { g_stop = 1; }

// --------------------------------------------------------------------------
// ZmqMsg — zmq_msg_t の move-only RAII ラッパ
// --------------------------------------------------------------------------
//
// バッチは最大 8 MiB(SPEC §2.3)。受信スレッドから集計スレッドへ「値のコピー」で渡すと
// 毎バッチ数 MiB の memcpy になるので、zmq_msg_move で所有権だけ動かす。
class ZmqMsg {
 public:
  ZmqMsg() { zmq_msg_init(&m_); }
  ~ZmqMsg() { zmq_msg_close(&m_); }
  ZmqMsg(ZmqMsg&& o) noexcept {
    zmq_msg_init(&m_);
    zmq_msg_move(&m_, &o.m_);
  }
  ZmqMsg& operator=(ZmqMsg&& o) noexcept {
    if (this != &o) zmq_msg_move(&m_, &o.m_);
    return *this;
  }
  ZmqMsg(const ZmqMsg&) = delete;
  ZmqMsg& operator=(const ZmqMsg&) = delete;

  zmq_msg_t* raw() { return &m_; }
  const uint8_t* data() { return static_cast<const uint8_t*>(zmq_msg_data(&m_)); }
  std::size_t size() { return zmq_msg_size(&m_); }

 private:
  zmq_msg_t m_;
};

// --------------------------------------------------------------------------
// カウンタ(集計スレッドが所有。読むのは join 後の main だけ)
// --------------------------------------------------------------------------

struct Counts {
  uint64_t batches = 0;
  uint64_t fragments = 0;
  uint64_t items = 0;
  uint64_t eos = 0;
  uint64_t runs = 0;
  // イベントビルダ(SPEC §6.3)
  uint64_t events_complete = 0;
  uint64_t events_incomplete = 0;    // 揃わないまま出た(timeout / EOS flush)—— 捨ててはいない
  uint64_t late_fragments = 0;       // emit 済みイベントへの遅延到着(捨てない)
  uint64_t unexpected_fragments = 0; // 期待 (cobo,asad) 集合の外から来た
  uint64_t duplicate_fragments = 0;  // 同一イベントに同じ (cobo,asad) が 2 回
  uint64_t pending_events = 0;       // 終了時点で組み上げ中だったイベント(瞬間値)
  // 以下は「黙って通さない」ための可視化カウンタ(CLAUDE.md)
  uint64_t heartbeats = 0;
  uint64_t unknown = 0;              // 知らない variant 名(前方互換で無視するが数える)
  uint64_t stale_eos = 0;            // idle 中の EOS
  uint64_t unexpected_sources = 0;   // 期待ソース集合外からの Data/EOS
  uint64_t run_number_mismatch = 0;  // run 中に食い違う run_number
  uint64_t recorder_queue = 0;       // 集計 → Recorder の在庫(瞬間値、011)
  // CoBo 毎のフラグメント数(status の材料 — SPEC §5.3。Fragment.cobo は u8)。
  uint64_t frames_per_cobo[rsmon::kMaxCobo] = {};
};

// --------------------------------------------------------------------------
// モニタ配線(TODO/022、SPEC §5.1 のスレッド規約)
// --------------------------------------------------------------------------
//
// **ロック規約**:
//   * `hist_mutex` を触るのは **集計スレッドと Publisher だけ**。Recorder(TTree)
//     スレッドとは一切共有しない —— これが「モニタが保存を邪魔しない」の実装手段。
//     ロック中に許すのは配列 fill / memcpy / 数個の u64 コピーのみ。
//     **msgpack 直列化と zmq send は必ずロックの外**。
//   * `slot_mutex` は built event の受け渡しだけ(最新優先の 1 枠)。
//
// **最新優先の作り**(100 Hz × ~2 MB/event を毎回コピーしないため):
//   Publisher が配信間隔ごとに `hungry` を立てる → 集計スレッドは emit 時に hungry で
//   あればそのイベントを**コピー**して slot に置き、hungry を下ろす。hungry でなければ
//   `publish_drops` を数えるだけ(モニタ系のドロップは silent にしない)。
struct Monitor {
  std::mutex hist_mutex;
  std::unique_ptr<rsmon::HistAccumulator> hist;  // --geometry があるときだけ
  rsmon::Status status;                          // hist_mutex 下で更新

  std::atomic<bool> hungry{false};
  std::mutex slot_mutex;
  rootsink::BuiltEvent slot;
  bool slot_full = false;

  std::atomic<uint64_t> publish_drops{0};
  std::atomic<uint64_t> snapshots_published{0};
  std::atomic<uint64_t> events_published{0};
  std::atomic<bool> pub_bind_failed{false};
  std::atomic<bool> stop{false};

  uint32_t snapshot_hz = kDefaultSnapshotHz;
  uint32_t event_hz = kDefaultEventPublishHz;

  // ヒスト集計と built_event 配信は **--geometry があるときだけ**(SPEC §5.2 の
  // フィル規則がジオメトリ無しでは定義できない)。status は常時。
  bool data_enabled() const { return hist != nullptr; }
};

Monitor g_mon;

// Recorder は別スレッドが回すが、fatal 時は join できないまま JSON を出す
// (落ちた瞬間のカウンタを捨てない)。カウンタは atomic + mutex 保護のスナップショットで
// 読めるようにしてある(root_recorder.hpp)。スレッド開始前に代入するので、
// print_counts_json から見えないタイミングは無い。
std::atomic<rootsink::Recorder*> g_recorder{nullptr};

// JSON 文字列値のエスケープ(パスに " や \ が入っても壊れた JSON を吐かない)。
std::string json_escape(const std::string& s) {
  std::string out;
  out.reserve(s.size() + 8);
  for (const char ch : s) {
    switch (ch) {
      case '"': out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\n': out += "\\n"; break;
      case '\r': out += "\\r"; break;
      case '\t': out += "\\t"; break;
      default:
        if (static_cast<unsigned char>(ch) < 0x20) {
          char buf[8];
          std::snprintf(buf, sizeof(buf), "\\u%04x", static_cast<unsigned>(ch) & 0xFFu);
          out += buf;
        } else {
          out.push_back(ch);
        }
    }
  }
  return out;
}

void print_counts_json(const Counts& c, const char* fatal) {
  rootsink::Recorder* rec = g_recorder.load(std::memory_order_acquire);
  const uint64_t entries_written = (rec != nullptr) ? rec->entries_written() : 0;
  const uint64_t items_out_of_range = (rec != nullptr) ? rec->items_out_of_range() : 0;
  // PEventTPC 出力の可視化カウンタ(TODO/020)。GDataFrame モードでは 0 のまま。
  const uint64_t duplicate_event_ids = (rec != nullptr) ? rec->duplicate_event_ids() : 0;
  const uint64_t channels_without_strip = (rec != nullptr) ? rec->channels_without_strip() : 0;
  const uint64_t charge_keys_out_of_range =
      (rec != nullptr) ? rec->charge_keys_out_of_range() : 0;
  const uint64_t frames_outside_geometry =
      (rec != nullptr) ? rec->frames_outside_geometry() : 0;
  std::string files_json = "[";
  if (rec != nullptr) {
    const std::vector<rootsink::RootFileRecord> files = rec->files_snapshot();
    for (std::size_t i = 0; i < files.size(); ++i) {
      char buf[64];
      if (i != 0) files_json += ",";
      files_json += "{\"path\":\"";
      files_json += json_escape(files[i].path);
      std::snprintf(buf, sizeof(buf), "\",\"entries\":%" PRIu64 ",\"bytes\":%" PRIu64 "}",
                    files[i].entries, files[i].bytes);
      files_json += buf;
    }
  }
  files_json += "]";

  std::printf(
      "{\"batches\":%" PRIu64 ",\"fragments\":%" PRIu64 ",\"items\":%" PRIu64
      ",\"eos\":%" PRIu64 ",\"runs\":%" PRIu64 ",\"events_complete\":%" PRIu64
      ",\"events_incomplete\":%" PRIu64 ",\"late_fragments\":%" PRIu64
      ",\"unexpected_fragments\":%" PRIu64 ",\"duplicate_fragments\":%" PRIu64
      ",\"pending_events\":%" PRIu64 ",\"heartbeats\":%" PRIu64
      ",\"unknown\":%" PRIu64 ",\"stale_eos\":%" PRIu64 ",\"unexpected_sources\":%" PRIu64
      ",\"run_number_mismatch\":%" PRIu64 ",\"entries_written\":%" PRIu64
      ",\"items_out_of_range\":%" PRIu64 ",\"recorder_queue\":%" PRIu64
      ",\"duplicate_event_ids\":%" PRIu64 ",\"channels_without_strip\":%" PRIu64
      ",\"charge_keys_out_of_range\":%" PRIu64 ",\"frames_outside_geometry\":%" PRIu64
      ",\"snapshots_published\":%" PRIu64 ",\"events_published\":%" PRIu64
      ",\"publish_drops\":%" PRIu64 ",\"pub_bind_failed\":%d"
      ",\"root_files\":%s,\"fatal\":\"%s\"}\n",
      c.batches, c.fragments, c.items, c.eos, c.runs, c.events_complete,
      c.events_incomplete, c.late_fragments, c.unexpected_fragments,
      c.duplicate_fragments, c.pending_events, c.heartbeats, c.unknown, c.stale_eos,
      c.unexpected_sources, c.run_number_mismatch, entries_written, items_out_of_range,
      c.recorder_queue, duplicate_event_ids, channels_without_strip,
      charge_keys_out_of_range, frames_outside_geometry,
      g_mon.snapshots_published.load(std::memory_order_relaxed),
      g_mon.events_published.load(std::memory_order_relaxed),
      g_mon.publish_drops.load(std::memory_order_relaxed),
      g_mon.pub_bind_failed.load(std::memory_order_relaxed) ? 1 : 0, files_json.c_str(),
      fatal);
  std::fflush(stdout);
}

// カウンタ付きで即死する。std::exit ではなく std::_Exit —— 受信スレッドが走ったまま
// 静的デストラクタを回すと二次災害になる(先に fflush する)。
[[noreturn]] void fatal(const Counts& c, int code, const char* kind,
                        const std::string& detail) {
  std::fprintf(stderr, "root_sink: FATAL %s: %s\n", kind, detail.c_str());
  std::fflush(stderr);
  print_counts_json(c, kind);
  std::_Exit(code);
}

// --------------------------------------------------------------------------
// オプション
// --------------------------------------------------------------------------

struct Options {
  std::string bind = kDefaultBind;
  int rcvhwm = kDefaultRcvHwm;
  std::size_t queue = kDefaultQueue;
  int throttle_ms = 0;
  // 期待フラグメント集合(SPEC §6.3)。既定 = mini eTPC の 1 CoBo・1 AsAd。
  // ジオメトリからの導出は設定配線の波(ここでは CLI で与える)。
  std::set<rootsink::FragmentKey> expect{{0, 0}};
  uint64_t build_timeout_ms = rootsink::kDefaultBuildTimeoutMs;
  // TTree 書き出し(TODO/011)
  std::string output_root = ".";
  uint64_t max_root_bytes = rootsink::kDefaultMaxRootBytes;
  int root_compression = rootsink::kDefaultCompression;  // TODO/014
  bool no_root = false;
  // PEventTPC 出力(TODO/020、SPEC §6.4 v1.8)
  rootsink::OutputFormat format = rootsink::OutputFormat::PEvent;
  std::string geometry;  // --geometry(PEvent モードでは必須)
  tpcpevent::FillConfig fill;
  // --run-id(TODO/021): EventInfo.runId の上書き。0 = run を開いた時刻から生成(既定)。
  // 実データ照合(同 run の grawToEventTPC 出力と比較)と、P4 で controller が正式な
  // run TS を配る経路(SPEC §6.4 実装注記)の受け口。
  long run_id = 0;
  // モニタ PUB(TODO/022、SPEC §5.3/§3.2)
  std::string pub = kDefaultPubBind;
  uint32_t snapshot_hz = kDefaultSnapshotHz;      // 0 = off
  uint32_t event_publish_hz = kDefaultEventPublishHz;  // 0 = off
};

void usage(const char* argv0) {
  std::printf(
      "usage: %s [--bind ENDPOINT] [--rcvhwm N] [--queue N] [--throttle-ms N]\n"
      "       [--expect COBO:ASAD[,COBO:ASAD...]] [--build-timeout-ms N]\n"
      "       [--output-root DIR] [--max-root-bytes N] [--root-compression N] [--no-root]\n"
      "       [--format pevent|gdataframe] [--geometry FILE.dat] [--pedestal-remove 0|1]\n"
      "       [--min-pedestal-cell N] [--max-pedestal-cell N]\n"
      "       [--min-signal-cell N] [--max-signal-cell N]\n"
      "       [--pub ENDPOINT] [--snapshot-hz N] [--event-publish-hz N]\n"
      "\n"
      "  --bind ENDPOINT   PULL bind endpoint (default %s)\n"
      "  --rcvhwm N        receive high-water mark, must be > 0 (default %d)\n"
      "  --queue N         bounded receive->aggregate queue depth (default %zu)\n"
      "  --throttle-ms N   sleep N ms per message in the aggregate thread\n"
      "                    (test hook for observing back-pressure. default 0)\n"
      "  --expect LIST     expected (cobo,asad) fragments per event, e.g.\n"
      "                    0:0,0:1,1:0,1:1 for ELITPC (default 0:0 = mini eTPC)\n"
      "  --build-timeout-ms N\n"
      "                    emit an event as incomplete this long after its first\n"
      "                    fragment (never dropped, default %" PRIu64 ")\n"
      "  --output-root DIR run files go to DIR/run{run:04}/ (default \".\")\n"
      "  --max-root-bytes N\n"
      "                    roll the run file over past this size (default %" PRIu64 ")\n"
      "  --root-compression N\n"
      "                    ROOT TFile compression settings, non-negative\n"
      "                    (default %d = ZLIB-1, readable by all ROOT eras;\n"
      "                    e.g. 505 = ZSTD-5, requires ROOT 6.20+)\n"
      "  --no-root         do not write a TTree, only count (fast path for tests)\n"
      "  --format FORMAT   pevent (default) writes PEventTPC events the way TPCReco's\n"
      "                    grawToEventTPC does (tree TPCData, one entry per built\n"
      "                    event); gdataframe writes the pre-v1.8 GDataFrame tree\n"
      "                    (one entry per fragment, kept for regression tests only)\n"
      "  --geometry FILE   TPCReco geometry .dat, REQUIRED for --format pevent\n"
      "  --pedestal-remove 0|1\n"
      "                    subtract the per-event pedestal (default 1)\n"
      "  --min-pedestal-cell N / --max-pedestal-cell N\n"
      "                    pedestal time window (default %d..%d)\n"
      "  --min-signal-cell N / --max-signal-cell N\n"
      "                    signal time window (default %d..%d)\n"
      "  --run-id N        override EventInfo.runId (YYYYmmddHHMMSS as one number;\n"
      "                    default 0 = derive from the wall clock when the run opens)\n"
      "  --pub ENDPOINT    monitor PUB bind endpoint (default %s). status is always\n"
      "                    published at 1 Hz, even while idle\n"
      "  --snapshot-hz N   histogram snapshot rate, 0 = off (default %u)\n"
      "  --event-publish-hz N\n"
      "                    upper bound on the latest-built-event rate, 0 = off\n"
      "                    (default %u). Histograms and built events need --geometry\n"
      "\n"
      "SIGINT/SIGTERM: drain, print one JSON line of counters to stdout, exit 0.\n"
      "A run that never saw its EndOfStream keeps the run_inprogress_<unixtime>.root\n"
      "name (an unfinalized run must not masquerade as a complete one).\n",
      argv0, kDefaultBind, kDefaultRcvHwm, kDefaultQueue,
      rootsink::kDefaultBuildTimeoutMs, rootsink::kDefaultMaxRootBytes,
      rootsink::kDefaultCompression, tpcpevent::kDefaultMinPedestalCell,
      tpcpevent::kDefaultMaxPedestalCell, tpcpevent::kDefaultMinSignalCell,
      tpcpevent::kDefaultMaxSignalCell, kDefaultPubBind, kDefaultSnapshotHz,
      kDefaultEventPublishHz);
}

// 半端な既定値で走らない(SPEC §3.2「設定パースエラーは起動失敗」)。
long parse_positive(const char* flag, const char* text) {
  char* endp = nullptr;
  const long v = std::strtol(text, &endp, 10);
  if (endp == text || *endp != '\0' || v <= 0) {
    std::fprintf(stderr, "root_sink: %s expects a positive integer, got '%s'\n", flag, text);
    std::exit(kExitUsage);
  }
  return v;
}

long parse_nonnegative(const char* flag, const char* text) {
  char* endp = nullptr;
  const long v = std::strtol(text, &endp, 10);
  if (endp == text || *endp != '\0' || v < 0) {
    std::fprintf(stderr, "root_sink: %s expects a non-negative integer, got '%s'\n", flag,
                 text);
    std::exit(kExitUsage);
  }
  return v;
}

// "0" 〜 "255" の 1 個を読む(--expect の COBO / ASAD)。
uint8_t parse_u8(const std::string& text) {
  char* endp = nullptr;
  const long v = std::strtol(text.c_str(), &endp, 10);
  if (text.empty() || endp == text.c_str() || *endp != '\0' || v < 0 || v > 255) {
    std::fprintf(stderr,
                 "root_sink: --expect expects COBO:ASAD with 0..255 values, got '%s'\n",
                 text.c_str());
    std::exit(kExitUsage);
  }
  return static_cast<uint8_t>(v);
}

// "0:0,0:1,1:0,1:1" → {(0,0),(0,1),(1,0),(1,1)}。半端な指定で走らない
// (SPEC §3.2「設定パースエラーは起動失敗」)。
std::set<rootsink::FragmentKey> parse_expect(const std::string& text) {
  std::set<rootsink::FragmentKey> out;
  std::size_t pos = 0;
  for (;;) {
    const std::size_t comma = text.find(',', pos);
    const std::string token =
        text.substr(pos, comma == std::string::npos ? std::string::npos : comma - pos);
    const std::size_t colon = token.find(':');
    if (colon == std::string::npos) {
      std::fprintf(stderr, "root_sink: --expect entry '%s' is not COBO:ASAD\n",
                   token.c_str());
      std::exit(kExitUsage);
    }
    const rootsink::FragmentKey key(parse_u8(token.substr(0, colon)),
                                    parse_u8(token.substr(colon + 1)));
    if (!out.insert(key).second) {
      std::fprintf(stderr, "root_sink: --expect lists %u:%u twice\n",
                   static_cast<unsigned>(key.first), static_cast<unsigned>(key.second));
      std::exit(kExitUsage);
    }
    if (comma == std::string::npos) break;
    pos = comma + 1;
  }
  return out;  // 空にはならない(トークンが 1 個も無ければ上で exit している)
}

Options parse_args(int argc, char** argv) {
  Options opt;
  for (int i = 1; i < argc; ++i) {
    const std::string a = argv[i];
    const bool has_value = (i + 1 < argc);
    if (a == "--help" || a == "-h") {
      usage(argv[0]);
      std::exit(0);
    } else if (a == "--bind" && has_value) {
      opt.bind = argv[++i];
    } else if (a == "--rcvhwm" && has_value) {
      opt.rcvhwm = static_cast<int>(parse_positive("--rcvhwm", argv[++i]));
    } else if (a == "--queue" && has_value) {
      opt.queue = static_cast<std::size_t>(parse_positive("--queue", argv[++i]));
    } else if (a == "--throttle-ms" && has_value) {
      opt.throttle_ms = static_cast<int>(parse_nonnegative("--throttle-ms", argv[++i]));
    } else if (a == "--expect" && has_value) {
      opt.expect = parse_expect(argv[++i]);
    } else if (a == "--build-timeout-ms" && has_value) {
      opt.build_timeout_ms =
          static_cast<uint64_t>(parse_positive("--build-timeout-ms", argv[++i]));
    } else if (a == "--output-root" && has_value) {
      opt.output_root = argv[++i];
    } else if (a == "--max-root-bytes" && has_value) {
      opt.max_root_bytes =
          static_cast<uint64_t>(parse_positive("--max-root-bytes", argv[++i]));
    } else if (a == "--root-compression" && has_value) {
      opt.root_compression =
          static_cast<int>(parse_nonnegative("--root-compression", argv[++i]));
    } else if (a == "--no-root") {
      opt.no_root = true;
    } else if (a == "--format" && has_value) {
      const std::string v = argv[++i];
      if (v == "pevent") {
        opt.format = rootsink::OutputFormat::PEvent;
      } else if (v == "gdataframe") {
        opt.format = rootsink::OutputFormat::GDataFrame;
      } else {
        std::fprintf(stderr, "root_sink: --format expects pevent|gdataframe, got '%s'\n",
                     v.c_str());
        std::exit(kExitUsage);
      }
    } else if (a == "--geometry" && has_value) {
      opt.geometry = argv[++i];
    } else if (a == "--pedestal-remove" && has_value) {
      const long v = parse_nonnegative("--pedestal-remove", argv[++i]);
      if (v > 1) {
        std::fprintf(stderr, "root_sink: --pedestal-remove expects 0 or 1, got '%s'\n",
                     argv[i]);
        std::exit(kExitUsage);
      }
      opt.fill.remove_pedestal = (v != 0);
    } else if (a == "--min-pedestal-cell" && has_value) {
      opt.fill.min_pedestal_cell =
          static_cast<int>(parse_nonnegative("--min-pedestal-cell", argv[++i]));
    } else if (a == "--max-pedestal-cell" && has_value) {
      opt.fill.max_pedestal_cell =
          static_cast<int>(parse_nonnegative("--max-pedestal-cell", argv[++i]));
    } else if (a == "--min-signal-cell" && has_value) {
      opt.fill.min_signal_cell =
          static_cast<int>(parse_nonnegative("--min-signal-cell", argv[++i]));
    } else if (a == "--max-signal-cell" && has_value) {
      opt.fill.max_signal_cell =
          static_cast<int>(parse_nonnegative("--max-signal-cell", argv[++i]));
    } else if (a == "--run-id" && has_value) {
      opt.run_id = parse_nonnegative("--run-id", argv[++i]);
    } else if (a == "--pub" && has_value) {
      opt.pub = argv[++i];
    } else if (a == "--snapshot-hz" && has_value) {
      opt.snapshot_hz = static_cast<uint32_t>(parse_nonnegative("--snapshot-hz", argv[++i]));
    } else if (a == "--event-publish-hz" && has_value) {
      opt.event_publish_hz =
          static_cast<uint32_t>(parse_nonnegative("--event-publish-hz", argv[++i]));
    } else {
      std::fprintf(stderr, "root_sink: unknown or incomplete option '%s'\n", a.c_str());
      usage(argv[0]);
      std::exit(kExitUsage);
    }
  }
  // 半端な設定で走らない(SPEC §3.2「設定パースエラーは起動失敗」)。
  if (!opt.no_root && opt.format == rootsink::OutputFormat::PEvent && opt.geometry.empty()) {
    std::fprintf(stderr,
                 "root_sink: --format pevent needs --geometry FILE.dat "
                 "(strip projection is impossible without it)\n");
    std::exit(kExitUsage);
  }
  std::string why;
  if (!opt.fill.validate(&why)) {
    std::fprintf(stderr, "root_sink: %s\n", why.c_str());
    std::exit(kExitUsage);
  }
  return opt;
}

// --------------------------------------------------------------------------
// 集計スレッド
// --------------------------------------------------------------------------

// 集計スレッドの壁時計(単調)。ビルダには**必ずここから注入する**——
// ビルダ自身は時計を読まない(SPEC §6.3 / テストが sleep 無しで書けるように)。
uint64_t now_ms() {
  const auto since_epoch = std::chrono::steady_clock::now().time_since_epoch();
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::milliseconds>(since_epoch).count());
}

// 送信時 unix ns(SPEC §2.2 の created_ns —— レイテンシ計測用なので壁時計)。
uint64_t now_unix_ns() {
  const auto since_epoch = std::chrono::system_clock::now().time_since_epoch();
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::nanoseconds>(since_epoch).count());
}

// 集計スレッド → Recorder スレッドの荷物。イベントに **in-band 制御マーカー**
// (RunClose)を混ぜる delila-rs の流儀(SPEC §6.1)—— run の finalize を別経路の
// フラグでやると、キューに残ったイベントより先に閉じてしまう。
struct RecordItem {
  enum class Kind : uint8_t { Event, RunClose };
  Kind kind = Kind::Event;
  uint32_t run_number = 0;         // Kind::RunClose のとき有効
  rootsink::BuiltEvent event;      // Kind::Event のとき有効
  // Kind::RunClose のとき、その run の最終ヒスト(TODO/022)。**run 毎 1 回のコピー**で
  // Recorder スレッドに渡し、TTree finalize の後に `run{run:04}_monitor.root` へ書く
  // (ROOT IO は Recorder スレッドだけ —— SPEC §5.1)。--geometry 無しなら nullptr。
  std::unique_ptr<rsmon::HistSnapshot> hists;
};

using RecordChannel = rootsink::Channel<RecordItem>;

// 組み上がったイベントを Recorder へ渡す(`rec` が nullptr = --no-root なら数えるだけ)。
// **push は満杯なら待つ** = 背圧。ここで捨てたらロスレス契約が崩れる(CLAUDE.md)。
void absorb(std::vector<rootsink::BuiltEvent>&& built, const rootsink::EventBuilder& eb,
            Counts& c, RecordChannel* rec) {
  for (rootsink::BuiltEvent& ev : built) {
    // --- モニタ集計(TODO/022)---------------------------------------------
    // ロック中にやるのは配列加算だけ(SPEC §5.1)。直列化も送信もここではしない。
    if (g_mon.data_enabled()) {
      {
        std::lock_guard<std::mutex> lk(g_mon.hist_mutex);
        g_mon.hist->on_event(ev);
      }
      // 最新優先の built event スロット。Publisher が hungry を立てたときだけコピー ——
      // 100 Hz × ~2 MB/event を毎回コピーしない(コピーは高々 event_publish_hz 回/秒)。
      if (g_mon.event_hz > 0) {
        if (g_mon.hungry.exchange(false, std::memory_order_acq_rel)) {
          std::lock_guard<std::mutex> lk(g_mon.slot_mutex);
          g_mon.slot = ev;
          g_mon.slot_full = true;
        } else {
          // モニタ系のドロップは落としてよいが **silent にしない**(CLAUDE.md)。
          g_mon.publish_drops.fetch_add(1, std::memory_order_relaxed);
        }
      }
    }
    // --- 保存系(ロスレス)。push は満杯なら待つ = 背圧 --------------------
    if (rec != nullptr) {
      RecordItem item;
      item.kind = RecordItem::Kind::Event;
      item.event = std::move(ev);
      rec->push(std::move(item));
    }
  }
  if (rec != nullptr) {
    c.recorder_queue = rec->size();
  }
  c.events_complete = eb.events_complete();
  c.events_incomplete = eb.events_incomplete();
  c.late_fragments = eb.late_fragments();
  c.unexpected_fragments = eb.unexpected_fragments();
  c.duplicate_fragments = eb.duplicate_fragments();
  c.pending_events = eb.pending();
}

// 1 メッセージを解釈してカウンタを進める。プロトコル違反はここで即死(戻らない)。
void consume(const uint8_t* data, std::size_t size, Counts& c, rootsink::RunState& run,
             rootsink::SeqCheck& seq, rootsink::EventBuilder& eb,
             std::vector<tpcwire::FragmentView>& frags, const Options& opt,
             RecordChannel* rec) {
  using namespace rootsink;
  using tpcwire::MsgKind;

  tpcwire::Envelope env;
  try {
    env = tpcwire::parse_envelope(data, size);
  } catch (const std::exception& e) {
    fatal(c, kExitMalformed, "malformed-message", e.what());
  }

  switch (env.kind) {
    case MsgKind::Data: {
      tpcwire::BatchHeader h;
      try {
        h = tpcwire::parse_batch_header(env.payload, env.payload_size);
      } catch (const std::exception& e) {
        fatal(c, kExitMalformed, "malformed-batch", e.what());
      }

      const DataAction da = run.on_data(h.source_id, h.run_number);
      if (da == DataAction::Unexpected) {
        // 期待していないソースのデータは混ぜない。数えて可視化する(CLAUDE.md)。
        std::fprintf(stderr,
                     "root_sink: Data from unexpected source_id=%u (run=%u seq=%" PRIu64
                     ") — counted, not mixed in\n",
                     h.source_id, h.run_number, h.sequence_number);
        c.unexpected_sources = run.unexpected_sources();
        break;
      }
      if (da == DataAction::Opened) {
        seq.reset();  // run 開始で sequence_number は 0 に戻る(SPEC §2.2)
        // ヒストは run start でリセット(SPEC §5.2)。CoBo 毎フレーム数も run 単位。
        if (g_mon.data_enabled()) {
          std::lock_guard<std::mutex> lk(g_mon.hist_mutex);
          g_mon.hist->reset();
        }
        std::memset(c.frames_per_cobo, 0, sizeof(c.frames_per_cobo));
        std::fprintf(stderr, "root_sink: run %u opened (source_id=%u)\n", h.run_number,
                     h.source_id);
      }

      const SeqAction sa = seq.check(h.source_id, h.sequence_number);
      if (sa == SeqAction::Gap || sa == SeqAction::Regression) {
        char buf[192];
        std::snprintf(buf, sizeof(buf),
                      "source_id=%u run=%u sequence_number=%" PRIu64 " (%s)", h.source_id,
                      h.run_number, h.sequence_number,
                      sa == SeqAction::Gap ? "gap" : "regression/duplicate");
        fatal(c, kExitSeq, "sequence-break", buf);
      }

      try {
        tpcwire::read_fragments(h.payload, h.payload_size, frags);
      } catch (const std::exception& e) {
        fatal(c, kExitMalformed, "malformed-fragments", e.what());
      }
      ++c.batches;
      c.fragments += frags.size();
      const uint64_t tick = now_ms();
      for (const tpcwire::FragmentView& f : frags) {
        c.items += f.item_count();
        ++c.frames_per_cobo[f.cobo];  // cobo は u8 —— 配列長 256 で必ず収まる
        std::optional<rootsink::LateFragment> late =
            eb.feed(rootsink::OwnedFragment::from_view(f, h.run_number), tick);
        if (late.has_value()) {
          if (c.late_fragments == 0) {
            // ログはこの 1 回だけ —— ホットパスで per-frame のログ整形をしない
            // (CLAUDE.md)。以降は late_fragments カウンタが可視化を担う。
            std::fprintf(stderr,
                         "root_sink: late fragment run=%u event_idx=%u cobo=%u asad=%u "
                         "(already emitted up to %u) — kept, counted\n",
                         late->fragment.run_number, late->fragment.event_idx,
                         late->fragment.cobo, late->fragment.asad, late->emitted_upto);
          }
          // **PEventTPC(既定)では TTree に書かない**(SPEC §6.3 v1.8): eventId は
          // 一回きりで書き切る形式なので、emit 済みイベントへの遅延分は
          // late_fragments として数えるだけ(データ自体は生 graw に必ず在る)。
          // GDataFrame モード(テスト専用)は従来どおり 1 フラグメント = 1 エントリで追記。
          if (rec != nullptr && opt.format == rootsink::OutputFormat::GDataFrame) {
            RecordItem item;
            item.kind = RecordItem::Kind::Event;
            item.event.run_number = late->fragment.run_number;
            item.event.event_idx = late->fragment.event_idx;
            item.event.fragments.push_back(std::move(late->fragment));
            rec->push(std::move(item));
          }
        }
        c.late_fragments = eb.late_fragments();
      }
      absorb(eb.poll(tick), eb, c, rec);
      c.run_number_mismatch = run.run_number_mismatch();
      break;
    }

    case MsgKind::EndOfStream: {
      ++c.eos;
      const EosAction ea = run.on_eos(env.source_id, env.run_number);
      if (ea == EosAction::Closed) {
        c.runs = run.runs_closed();
        // 組み上げ中の残りを全部出す(揃わなかったものは incomplete で —— 捨てない)。
        absorb(eb.flush(), eb, c, rec);
        // 残りを流し切った**あとに** finalize マーカーを流す(in-band = 追い越さない)。
        if (rec != nullptr) {
          RecordItem item;
          item.kind = RecordItem::Kind::RunClose;
          item.run_number = run.run_number();
          if (g_mon.data_enabled()) {
            // 最終ヒストを同梱する(run 毎 1 回のコピー)。**確保はロック外**で、
            // ロック中は memcpy だけ(SPEC §5.1)。
            item.hists.reset(new rsmon::HistSnapshot());
            g_mon.hist->prepare(*item.hists);
            std::lock_guard<std::mutex> lk(g_mon.hist_mutex);
            g_mon.hist->copy_into(*item.hists);
          }
          // push は満杯なら**待つ**(背圧)。ロックは上のスコープで既に手放している ——
          // 保存側の詰まりで Publisher を巻き込まない(SPEC §5.1)。
          rec->push(std::move(item));
          c.recorder_queue = rec->size();
        }
        std::fprintf(stderr,
                     "root_sink: run %u closed (batches=%" PRIu64 " events_complete=%" PRIu64
                     " events_incomplete=%" PRIu64 " late_fragments=%" PRIu64 ")\n",
                     env.run_number, c.batches, c.events_complete, c.events_incomplete,
                     c.late_fragments);
      } else if (ea == EosAction::Ignored) {
        std::fprintf(stderr,
                     "root_sink: ignored EndOfStream source_id=%u run=%u "
                     "(idle or unexpected source)\n",
                     env.source_id, env.run_number);
      }
      c.stale_eos = run.stale_eos();
      c.unexpected_sources = run.unexpected_sources();
      c.run_number_mismatch = run.run_number_mismatch();
      break;
    }

    case MsgKind::Heartbeat:
      ++c.heartbeats;
      break;

    case MsgKind::Unknown:
      // 知らない variant は前方互換のため無視するが、黙ってはいない(CLAUDE.md)。
      ++c.unknown;
      std::fprintf(stderr, "root_sink: unknown message variant '%s' (ignored)\n",
                   env.variant.c_str());
      break;
  }

  if (opt.throttle_ms > 0) {
    std::this_thread::sleep_for(std::chrono::milliseconds(opt.throttle_ms));
  }
}

// --------------------------------------------------------------------------
// Publisher スレッド(TODO/022 / SPEC §5.3)
// --------------------------------------------------------------------------

// 集計スレッドが status の材料を置く(hist_mutex 下。コピーだけ)。
void update_status_material(const rootsink::RunState& run, const Counts& c) {
  std::lock_guard<std::mutex> lk(g_mon.hist_mutex);
  // idle のときの run は「直近クローズした run」(RunState は閉じても番号を持つ)。
  g_mon.status.run = run.run_number();
  g_mon.status.running = run.is_open();
  g_mon.status.events_built = c.events_complete + c.events_incomplete;
  g_mon.status.events_incomplete = c.events_incomplete;
  g_mon.status.late_fragments = c.late_fragments;
  std::memcpy(g_mon.status.frames_per_cobo, c.frames_per_cobo,
              sizeof(g_mon.status.frames_per_cobo));
}

// 落としてよい送信(PUB)。EAGAIN = 購読側が遅れて HWM に当たった = 意図した間引き。
bool pub_send(void* pub, const std::vector<uint8_t>& msg) {
  return zmq_send(pub, msg.data(), msg.size(), ZMQ_DONTWAIT) >= 0;
}

// PUB を所有する唯一のスレッド。**直列化と送信は必ず hist_mutex の外**(SPEC §5.1)。
void publisher_thread(void* ctx, Options opt, rootsink::Recorder* recorder) {
  void* pub = zmq_socket(ctx, ZMQ_PUB);
  if (pub == nullptr) {
    std::fprintf(stderr, "root_sink: ERROR monitor PUB zmq_socket failed: %s\n",
                 zmq_strerror(zmq_errno()));
    g_mon.pub_bind_failed.store(true, std::memory_order_relaxed);
    return;
  }
  zmq_setsockopt(pub, ZMQ_SNDHWM, &kPubSndHwm, sizeof(kPubSndHwm));
  const int linger = 0;
  zmq_setsockopt(pub, ZMQ_LINGER, &linger, sizeof(linger));
  if (zmq_bind(pub, opt.pub.c_str()) != 0) {
    // **保存系は落とさない**(CLAUDE.md の絶対ルール: 保存 > モニタ)。モニタ PUB が
    // 立たないことは大声で出し、終了 JSON にも残す —— silent にはしない。
    std::fprintf(stderr,
                 "root_sink: ERROR monitor PUB zmq_bind(%s) failed: %s — monitoring is "
                 "OFF for this process (storage continues, pub_bind_failed=1)\n",
                 opt.pub.c_str(), zmq_strerror(zmq_errno()));
    g_mon.pub_bind_failed.store(true, std::memory_order_relaxed);
    zmq_close(pub);
    return;
  }

  rsmon::Encoder enc;
  std::vector<uint8_t> buf;      // 使い回す(スナップショットは ~2.4 MB 級)
  rsmon::HistSnapshot snap;      // 二重バッファの片側
  rootsink::BuiltEvent event;    // slot から swap で受け取る作業バッファ
  if (g_mon.data_enabled()) {
    // ビン数は構築時に決まって以後変わらない —— 受け皿の確保はロック外で 1 回だけ。
    g_mon.hist->prepare(snap);
  }

  const uint64_t snapshot_period =
      g_mon.snapshot_hz > 0 ? std::max<uint64_t>(1, 1000 / g_mon.snapshot_hz) : 0;
  const uint64_t event_period =
      g_mon.event_hz > 0 ? std::max<uint64_t>(1, 1000 / g_mon.event_hz) : 0;
  const bool snapshots_on = snapshot_period > 0 && g_mon.data_enabled();
  const bool events_on = event_period > 0 && g_mon.data_enabled();

  uint64_t next_status = now_ms();
  uint64_t next_snapshot = next_status;
  uint64_t next_event = next_status;

  while (!g_mon.stop.load(std::memory_order_relaxed)) {
    const uint64_t now = now_ms();

    if (now >= next_status) {
      rsmon::Status s;
      {
        std::lock_guard<std::mutex> lk(g_mon.hist_mutex);
        s = g_mon.status;  // 小さな POD のコピーだけ
        if (g_mon.data_enabled()) {
          for (size_t p = 0; p < rsmon::kPlaneCount; ++p) {
            s.saturation[p] = g_mon.hist->saturation(static_cast<tpcgeo::Plane>(p));
          }
        }
      }
      s.bytes_written = (recorder != nullptr) ? recorder->bytes_written() : 0;
      s.publish_drops = g_mon.publish_drops.load(std::memory_order_relaxed);
      enc.status(s, now_unix_ns(), buf);  // ← ロック外
      pub_send(pub, buf);
      next_status = now + kStatusPeriodMs;
    }

    if (snapshots_on && now >= next_snapshot) {
      uint32_t run = 0;
      {
        std::lock_guard<std::mutex> lk(g_mon.hist_mutex);
        run = g_mon.status.run;
        g_mon.hist->copy_into(snap);  // memcpy だけ
      }
      enc.hist_snapshot(run, snap, now_unix_ns(), buf);  // ← ロック外
      if (pub_send(pub, buf)) {
        g_mon.snapshots_published.fetch_add(1, std::memory_order_relaxed);
      }
      next_snapshot = now + snapshot_period;
    }

    if (events_on && now >= next_event) {
      bool have = false;
      {
        std::lock_guard<std::mutex> lk(g_mon.slot_mutex);
        if (g_mon.slot_full) {
          // swap: Publisher 側の古いバッファを slot に返す(次のコピーが容量を再利用)。
          std::swap(event, g_mon.slot);
          g_mon.slot_full = false;
          have = true;
        }
      }
      if (have) {
        enc.built_event(event, now_unix_ns(), buf);  // ← ロック外
        if (pub_send(pub, buf)) {
          g_mon.events_published.fetch_add(1, std::memory_order_relaxed);
        } else {
          // 送れなかった分も間引きとして数える(published + drops = 全イベント)。
          // PUB は遅い購読者向けにソケット層で捨てるので実際にはまず起きないが、
          // 数え落ちを作らない(CLAUDE.md)。
          g_mon.publish_drops.fetch_add(1, std::memory_order_relaxed);
        }
      } else {
        // 次の emit でコピーしてもらう(集計スレッドはこの旗が立っている時だけ払う)。
        g_mon.hungry.store(true, std::memory_order_release);
      }
      next_event = now + event_period;
    }

    // 次の配信期限まで寝る(上限 kPubTickMs = 停止に気づく間隔)。
    uint64_t next = next_status;
    if (snapshots_on && next_snapshot < next) next = next_snapshot;
    if (events_on && next_event < next) next = next_event;
    const uint64_t t = now_ms();
    uint64_t sleep_ms = (next > t) ? (next - t) : 0;
    if (sleep_ms > static_cast<uint64_t>(kPubTickMs)) sleep_ms = kPubTickMs;
    if (sleep_ms == 0) sleep_ms = 1;
    std::this_thread::sleep_for(std::chrono::milliseconds(sleep_ms));
  }

  zmq_close(pub);
}

}  // namespace

int main(int argc, char** argv) {
  const Options opt = parse_args(argc, argv);

  std::signal(SIGINT, on_signal);
  std::signal(SIGTERM, on_signal);

  // ROOT 作法(SPEC §6.1)。バッチモード = GUI を開かない、スレッド安全化は
  // Recorder スレッドが TFile を触るため。--no-root では ROOT に一切触らない。
  // ジオメトリは **PEventTPC 出力とモニタ集計の共有物**(TODO/022)。--no-root でも
  // --geometry があれば読む —— ヒストは ROOT を書かなくても PUB で配れる(SPEC §5.3)。
  std::unique_ptr<rootsink::Recorder> recorder;
  std::unique_ptr<RecordChannel> rec_chan;
  std::unique_ptr<tpcgeo::Geometry> geometry;  // Recorder / HistAccumulator より長生き
  if (!opt.geometry.empty()) {
    try {
      geometry.reset(new tpcgeo::Geometry(tpcgeo::load(opt.geometry)));
    } catch (const std::exception& e) {
      std::fprintf(stderr, "root_sink: --geometry %s: %s\n", opt.geometry.c_str(), e.what());
      return kExitUsage;
    }
    // ジオメトリの素性を必ず出す(黙って半端な .dat で走らない —— CLAUDE.md)。
    std::fprintf(stderr,
                 "root_sink: geometry %s: cobos=%zu max_strip=U%u/V%u/W%u "
                 "duplicates=%zu malformed_lines=%zu\n",
                 opt.geometry.c_str(), geometry->cobo_count(),
                 static_cast<unsigned>(geometry->max_strip[0]),
                 static_cast<unsigned>(geometry->max_strip[1]),
                 static_cast<unsigned>(geometry->max_strip[2]),
                 geometry->duplicate_warnings().size(), geometry->malformed_lines().size());
    g_mon.hist.reset(new rsmon::HistAccumulator(*geometry));
  }
  g_mon.snapshot_hz = opt.snapshot_hz;
  g_mon.event_hz = opt.event_publish_hz;

  if (!opt.no_root) {
    gROOT->SetBatch(kTRUE);
    ROOT::EnableThreadSafety();
    rootsink::RecorderConfig rcfg;
    rcfg.output_root = opt.output_root;
    rcfg.max_root_bytes = opt.max_root_bytes;
    rcfg.compression = opt.root_compression;
    rcfg.format = opt.format;
    rcfg.geometry = geometry.get();
    rcfg.fill = opt.fill;
    rcfg.run_id_override = opt.run_id;
    recorder.reset(new rootsink::Recorder(rcfg));
    // スレッドを起こす前に公開する(fatal 時の JSON から必ず見える)。
    g_recorder.store(recorder.get(), std::memory_order_release);
    rec_chan.reset(new RecordChannel(kRecorderQueue));
  }

  void* ctx = zmq_ctx_new();
  if (ctx == nullptr) {
    std::fprintf(stderr, "root_sink: zmq_ctx_new failed: %s\n", zmq_strerror(zmq_errno()));
    return kExitZmq;
  }
  void* pull = zmq_socket(ctx, ZMQ_PULL);
  if (pull == nullptr) {
    std::fprintf(stderr, "root_sink: zmq_socket failed: %s\n", zmq_strerror(zmq_errno()));
    zmq_ctx_term(ctx);
    return kExitZmq;
  }
  // HWM は bind の前に設定しないと効かない。**有限**であることが背圧の前提(SPEC §6.2-2)。
  if (zmq_setsockopt(pull, ZMQ_RCVHWM, &opt.rcvhwm, sizeof(opt.rcvhwm)) != 0) {
    std::fprintf(stderr, "root_sink: ZMQ_RCVHWM failed: %s\n", zmq_strerror(zmq_errno()));
    zmq_close(pull);
    zmq_ctx_term(ctx);
    return kExitZmq;
  }
  const int linger = 0;
  zmq_setsockopt(pull, ZMQ_LINGER, &linger, sizeof(linger));
  if (zmq_bind(pull, opt.bind.c_str()) != 0) {
    std::fprintf(stderr, "root_sink: zmq_bind(%s) failed: %s\n", opt.bind.c_str(),
                 zmq_strerror(zmq_errno()));
    zmq_close(pull);
    zmq_ctx_term(ctx);
    return kExitZmq;
  }

  rootsink::Channel<ZmqMsg> chan(opt.queue);
  Counts counts;

  std::fprintf(stderr,
               "root_sink: PULL bind %s (rcvhwm=%d queue=%zu throttle_ms=%d). "
               "SIGINT/SIGTERM to stop.\n",
               opt.bind.c_str(), opt.rcvhwm, opt.queue, opt.throttle_ms);
  if (opt.no_root) {
    std::fprintf(stderr, "root_sink: --no-root: counting only, no TTree is written\n");
  } else if (opt.format == rootsink::OutputFormat::PEvent) {
    std::fprintf(stderr,
                 "root_sink: writing PEventTPC TTree \"%s\" under %s (max-root-bytes=%" PRIu64
                 " compression=%d pedestal_remove=%d pedestal_cells=%d..%d "
                 "signal_cells=%d..%d)\n",
                 rootsink::kPEventTreeName, opt.output_root.c_str(), opt.max_root_bytes,
                 opt.root_compression, opt.fill.remove_pedestal ? 1 : 0,
                 opt.fill.min_pedestal_cell, opt.fill.max_pedestal_cell,
                 opt.fill.min_signal_cell, opt.fill.max_signal_cell);
  } else {
    std::fprintf(stderr,
                 "root_sink: writing GDataFrame TTree \"%s\" under %s (max-root-bytes=%" PRIu64
                 " compression=%d)\n",
                 rootsink::kTreeName, opt.output_root.c_str(), opt.max_root_bytes,
                 opt.root_compression);
  }
  // モニタ PUB の素性(TODO/022)。何を配るのかを黙って決めない —— CLAUDE.md。
  std::fprintf(stderr,
               "root_sink: monitor PUB bind %s (snapshot_hz=%u event_publish_hz=%u "
               "sndhwm=%d histograms=%s)\n",
               opt.pub.c_str(), opt.snapshot_hz, opt.event_publish_hz, kPubSndHwm,
               g_mon.data_enabled() ? "on" : "off (no --geometry: status only)");
  std::fflush(stderr);

  // Recorder スレッド(ROOT IO はここだけ。SPEC §6.2-8)。
  std::thread record_thread;
  if (recorder) {
    record_thread = std::thread([&] {
      RecordItem item;
      for (;;) {
        const rootsink::PopResult pr = rec_chan->pop_for(item, kRecorderTickMs);
        if (pr == rootsink::PopResult::Value) {
          if (item.kind == RecordItem::Kind::RunClose) {
            recorder->close_run(item.run_number, now_ms());
            // TTree finalize の**後**にモニタヒストを書く(SPEC §6.5 / R10 = §12-9)。
            if (item.hists) recorder->write_monitor_root(item.run_number, *item.hists);
          } else {
            recorder->write(item.event, now_ms());
          }
          if (recorder->fatal_reason() != nullptr) {
            // 書けないまま走り続けない。counts は集計スレッドの持ち物だが、
            // これは _Exit 直前の診断スナップショット(既存の fatal と同じ扱い)。
            fatal(counts, kExitRootIo, recorder->fatal_reason(),
                  recorder->fatal_detail());
          }
        } else {
          recorder->tick(now_ms());  // データが来なくても AutoSave は進む
        }
        if (pr == rootsink::PopResult::Closed) break;
      }
      // 停止時に run が finalize されていなければ inprogress のまま閉じる(SPEC §6.5)。
      recorder->shutdown();
    });
  }

  // Publisher スレッド(PUB を所有。TODO/022 / SPEC §5.1)。
  std::thread publish_thread(publisher_thread, ctx, opt, recorder.get());

  // 集計スレッド。P2 の期待ソース集合は {decoder} のみ(SPEC §2.3)。
  std::thread aggregate([&] {
    rootsink::RunState run({rootsink::kDecoderSourceId});
    // decoder リンクは run 開始で sequence_number が 0 に戻る(SPEC §2.2)。
    // **厳格モード** = 初回 0 以外は Gap 扱い —— 先頭バッチの喪失を不可視にしない。
    rootsink::SeqCheck seq(/*expect_zero_start=*/true);
    rootsink::EventBuilder eb(opt.expect, opt.build_timeout_ms);
    std::vector<tpcwire::FragmentView> frags;  // 使い回す(毎バッチの heap 確保を避ける)
    ZmqMsg msg;
    for (;;) {
      const rootsink::PopResult pr = chan.pop_for(msg, kBuilderTickMs);
      if (pr == rootsink::PopResult::Value) {
        consume(msg.data(), msg.size(), counts, run, seq, eb, frags, opt, rec_chan.get());
      } else {
        // データが来なくても壁時計は進む —— アイドル中の build_timeout はここで働く。
        absorb(eb.poll(now_ms()), eb, counts, rec_chan.get());
      }
      // status の材料を置き直す(Publisher が 1 Hz で読む。ロック中はコピーだけ)。
      update_status_material(run, counts);
      if (pr == rootsink::PopResult::Closed) break;
    }
    // 停止時に組み上げ中だったイベントも数に出す(黙って消さない)。**書きもする** ——
    // run が閉じていないので finalize はしない = inprogress のまま残る(SPEC §6.5)。
    absorb(eb.flush(), eb, counts, rec_chan.get());
  });

  // 受信ループ(main スレッド)。ソケットを触るのはこのスレッドだけ。
  auto drain_ready_messages = [&] {
    for (;;) {
      ZmqMsg msg;
      if (zmq_msg_recv(msg.raw(), pull, ZMQ_DONTWAIT) < 0) break;  // EAGAIN = 在庫切れ
      chan.push(std::move(msg));  // 満杯ならここで待つ = 背圧(捨てない)
    }
  };

  zmq_pollitem_t items[1];
  items[0].socket = pull;
  items[0].fd = 0;
  items[0].events = ZMQ_POLLIN;
  items[0].revents = 0;

  while (g_stop == 0) {
    const int rc = zmq_poll(items, 1, kPollTimeoutMs);
    if (rc < 0) {
      if (zmq_errno() == EINTR) continue;  // 自分のシグナルで起こされただけ
      std::fprintf(stderr, "root_sink: zmq_poll error: %s\n", zmq_strerror(zmq_errno()));
      break;
    }
    if ((items[0].revents & ZMQ_POLLIN) != 0) drain_ready_messages();
  }

  // 停止後の猶予: 飛んでいる途中のメッセージを吸い切る(終了時に取りこぼさない)。
  for (;;) {
    const int rc = zmq_poll(items, 1, kDrainGraceMs);
    if (rc <= 0) break;  // タイムアウト or エラー = もう来ない
    if ((items[0].revents & ZMQ_POLLIN) != 0) drain_ready_messages();
  }

  chan.close();
  aggregate.join();
  // 集計が終わってから Recorder を閉じる(順番が逆だと最後のイベントの行き場が消える)。
  if (rec_chan) {
    counts.recorder_queue = rec_chan->size();
    rec_chan->close();
  }
  if (record_thread.joinable()) record_thread.join();
  // Publisher は最後に畳む(Recorder の atomic を読むので、Recorder より先に止める)。
  g_mon.stop.store(true, std::memory_order_relaxed);
  if (publish_thread.joinable()) publish_thread.join();

  print_counts_json(counts, "");

  zmq_close(pull);
  zmq_ctx_term(ctx);
  return 0;
}
