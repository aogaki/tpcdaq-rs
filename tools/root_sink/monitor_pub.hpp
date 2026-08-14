// monitor_pub.hpp — モニタ PUB のワイヤ直列化(SPEC §5.3 v1.10 + §2.2 + §2.4)。TODO/022。
// status に `pending_events`(11 キー目、`late_fragments` の次)を追加(TODO/024、R-P2-5)。
//
// **純ヘッダ。ZMQ 非依存・ROOT 非依存** —— ここは「バイト列を組む」だけで、送るのは
// root_sink.cxx の Publisher スレッド。直列化と zmq send は必ず `hist_mutex` の**外**で
// 行う(SPEC §5.1: 保存への非干渉)。
//
// 既存の tpc_wire.hpp は**読み側**しか持たない(root-sink は受信専用だった)。
// ここに置くのは必要最小限の**書き側** —— map / array / str / uint / bool / bin の
// 6 種だけ(KISS。DOM も汎用シリアライザも作らない)。
//
// --- ワイヤ形式(SPEC §5.3 v1.9 が正本。ここでは再定義しない)-----------------
//   エンベロープ = §2.2 と同形:
//     fixmap(1){ "Data": array(5)[ source_id=101, run_number, sequence_number,
//                                  created_ns, payload ] }
//   sequence_number は **全 kind 単一系列・run リセット無し**(§2.2 の 0 リセット規則の
//   例外。モニタリンクでの用途は「ギャップ = ドロップ数」の可視化だけ)。
//   payload は **map 形式**(`to_vec_named` 相当の自己記述)で `kind` で 3 種を判別:
//     "status" / "hist_snapshot" / "built_event"
//   `hist_snapshot.bins` は **f64 LE の生バイト列**(msgpack float64 の連打は 9 B/ビンで
//   2 倍以上太る —— SPEC がそう定めた理由)。
//   `built_event.fragments` は §2.4 の positional array(12) の再直列化。
//
// 整数は**最小幅**で書く(rmp-serde の compact 出力と同じ規約 —— 読み手はどの幅でも
// 受けられるが、期待バイト列を機械照合できるように送り側を一意に決めておく)。

#ifndef TPCDAQ_ROOT_SINK_MONITOR_PUB_HPP
#define TPCDAQ_ROOT_SINK_MONITOR_PUB_HPP

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <vector>

#include "eb_core.hpp"
#include "monitor_hist.hpp"

namespace rsmon {

// root-sink のモニタ PUB の source_id(SPEC §3.2 v1.9)。
constexpr uint32_t kMonitorSourceId = 101;
// Fragment.cobo は u8 —— status の frames_per_cobo はこの幅の固定配列で持つ。
constexpr size_t kMaxCobo = 256;

// ---------------------------------------------------------------------------
// status のフィールド(SPEC §5.3)
// ---------------------------------------------------------------------------
//
// 集計スレッドが `hist_mutex` 下で更新し、Publisher が同じロックの下でコピーして
// (コピーだけ)、ロック外で直列化する。
struct Status {
  uint32_t run = 0;        // 走行中はその run、idle なら直近クローズした run(無ければ 0)
  bool running = false;    // ワイヤ上は "running" / "idle"
  uint64_t events_built = 0;       // emit 済みイベント総数(complete + incomplete)
  uint64_t events_incomplete = 0;
  uint64_t late_fragments = 0;
  // ビルダ組み上げ中の瞬間値(SPEC §5.3 v1.10、R-P2-5)。pending ≈ レート ×
  // build_timeout に比例するため常時可視化する(hard limit は設けない — 下の
  // kPendingWarnThreshold は warn するだけ)。
  uint64_t pending_events = 0;
  uint64_t frames_per_cobo[kMaxCobo] = {};
  uint64_t bytes_written = 0;      // Recorder の atomic(--no-root なら 0)
  Saturation saturation[kPlaneCount] = {};
  uint64_t publish_drops = 0;      // built event の間引き数(送り手側の意図的間引き)
};

// pending_events の警告閾値(SPEC §5.3 v1.10、R-P2-5)。hard limit はロスレス契約と
// 衝突するので設けない —— 超過を可視化して warn するだけ。呼び手(root_sink.cxx の
// 集計スレッド)が「一度だけ」のラッチを持ち、この純関数は閾値判定だけを切り出す
// (単体テスト対象)。
constexpr uint64_t kPendingWarnThreshold = 1000;

inline bool pending_events_exceeds_warn_threshold(uint64_t pending_events) {
  return pending_events > kPendingWarnThreshold;
}

// ---------------------------------------------------------------------------
// MessagePack 書き側(必要な 6 種だけ)
// ---------------------------------------------------------------------------

namespace mpw {

inline void be(std::vector<uint8_t>& o, uint64_t v, int bytes) {
  for (int k = bytes - 1; k >= 0; --k) o.push_back(static_cast<uint8_t>(v >> (8 * k)));
}

inline void map_len(std::vector<uint8_t>& o, uint32_t n) {
  if (n < 16) {
    o.push_back(static_cast<uint8_t>(0x80u | n));
  } else if (n <= 0xFFFF) {
    o.push_back(0xde);
    be(o, n, 2);
  } else {
    o.push_back(0xdf);
    be(o, n, 4);
  }
}

inline void array_len(std::vector<uint8_t>& o, uint32_t n) {
  if (n < 16) {
    o.push_back(static_cast<uint8_t>(0x90u | n));
  } else if (n <= 0xFFFF) {
    o.push_back(0xdc);
    be(o, n, 2);
  } else {
    o.push_back(0xdd);
    be(o, n, 4);
  }
}

inline void str(std::vector<uint8_t>& o, const char* s, size_t n) {
  if (n < 32) {
    o.push_back(static_cast<uint8_t>(0xa0u | n));
  } else if (n <= 0xFF) {
    o.push_back(0xd9);
    o.push_back(static_cast<uint8_t>(n));
  } else if (n <= 0xFFFF) {
    o.push_back(0xda);
    be(o, n, 2);
  } else {
    o.push_back(0xdb);
    be(o, n, 4);
  }
  o.insert(o.end(), s, s + n);
}

inline void str(std::vector<uint8_t>& o, const char* s) { str(o, s, std::strlen(s)); }

// 符号なし整数を**最小幅**で(positive fixint → uint8 → uint16 → uint32 → uint64)。
inline void uint_(std::vector<uint8_t>& o, uint64_t v) {
  if (v <= 0x7F) {
    o.push_back(static_cast<uint8_t>(v));
  } else if (v <= 0xFF) {
    o.push_back(0xcc);
    o.push_back(static_cast<uint8_t>(v));
  } else if (v <= 0xFFFF) {
    o.push_back(0xcd);
    be(o, v, 2);
  } else if (v <= 0xFFFFFFFFULL) {
    o.push_back(0xce);
    be(o, v, 4);
  } else {
    o.push_back(0xcf);
    be(o, v, 8);
  }
}

inline void boolean(std::vector<uint8_t>& o, bool v) { o.push_back(v ? 0xc3 : 0xc2); }

inline void bin_header(std::vector<uint8_t>& o, size_t n) {
  if (n <= 0xFF) {
    o.push_back(0xc4);
    o.push_back(static_cast<uint8_t>(n));
  } else if (n <= 0xFFFF) {
    o.push_back(0xc5);
    be(o, n, 2);
  } else {
    o.push_back(0xc6);
    be(o, n, 4);
  }
}

inline void bin(std::vector<uint8_t>& o, const uint8_t* p, size_t n) {
  bin_header(o, n);
  o.insert(o.end(), p, p + n);
}

// double 配列を **f64 リトルエンディアンの bin** として書く(SPEC §5.3)。
// ホストのバイト順に依存しないよう、ビットパターンを取り出して明示的に並べる。
inline void bin_f64_le(std::vector<uint8_t>& o, const double* v, size_t n) {
  bin_header(o, n * 8);
  const size_t at = o.size();
  o.resize(at + n * 8);
  uint8_t* p = o.data() + at;
  for (size_t i = 0; i < n; ++i) {
    uint64_t bits = 0;
    std::memcpy(&bits, &v[i], sizeof(bits));
    for (int k = 0; k < 8; ++k) p[i * 8 + k] = static_cast<uint8_t>(bits >> (8 * k));
  }
}

}  // namespace mpw

// ---------------------------------------------------------------------------
// Encoder — 3 種のメッセージを組む(seq の採番もここ)
// ---------------------------------------------------------------------------
//
// 出力は呼び手のバッファに書く(`out` は毎回 clear されるが**容量は保たれる** ——
// スナップショットは ~2.4 MB 級なので、毎秒 heap を確保し直さない)。
class Encoder {
 public:
  explicit Encoder(uint32_t source_id = kMonitorSourceId) : source_id_(source_id) {}

  Encoder(const Encoder&) = delete;
  Encoder& operator=(const Encoder&) = delete;

  // 次に使う sequence_number(テストと可視化用)。
  uint64_t sequence() const { return seq_; }

  void status(const Status& s, uint64_t created_ns, std::vector<uint8_t>& out) {
    open_envelope(out, s.run, created_ns);
    mpw::map_len(out, 11);
    mpw::str(out, "kind");
    mpw::str(out, "status");
    mpw::str(out, "run");
    mpw::uint_(out, s.run);
    mpw::str(out, "state");
    mpw::str(out, s.running ? "running" : "idle");
    mpw::str(out, "events_built");
    mpw::uint_(out, s.events_built);
    mpw::str(out, "events_incomplete");
    mpw::uint_(out, s.events_incomplete);
    mpw::str(out, "late_fragments");
    mpw::uint_(out, s.late_fragments);
    mpw::str(out, "pending_events");
    mpw::uint_(out, s.pending_events);
    mpw::str(out, "frames_per_cobo");
    write_frames_per_cobo(out, s);
    mpw::str(out, "bytes_written");
    mpw::uint_(out, s.bytes_written);
    mpw::str(out, "saturation");
    mpw::map_len(out, 3);
    for (size_t p = 0; p < kPlaneCount; ++p) {
      mpw::str(out, tpcgeo::plane_as_str(static_cast<tpcgeo::Plane>(p)));
      mpw::map_len(out, 2);
      mpw::str(out, "saturated");
      mpw::uint_(out, s.saturation[p].saturated);
      mpw::str(out, "counted");
      mpw::uint_(out, s.saturation[p].counted);
    }
    mpw::str(out, "publish_drops");
    mpw::uint_(out, s.publish_drops);
  }

  void hist_snapshot(uint32_t run, const HistSnapshot& hists, uint64_t created_ns,
                     std::vector<uint8_t>& out) {
    open_envelope(out, run, created_ns);
    mpw::map_len(out, 3);
    mpw::str(out, "kind");
    mpw::str(out, "hist_snapshot");
    mpw::str(out, "run");
    mpw::uint_(out, run);
    mpw::str(out, "hists");
    mpw::array_len(out, static_cast<uint32_t>(hists.size()));
    for (const HistBuffer& h : hists) {
      mpw::map_len(out, 5);
      mpw::str(out, "id");
      mpw::uint_(out, h.id);
      mpw::str(out, "name");
      mpw::str(out, h.name);
      mpw::str(out, "nx");
      mpw::uint_(out, h.nx);
      mpw::str(out, "ny");
      mpw::uint_(out, h.ny);
      mpw::str(out, "bins");
      mpw::bin_f64_le(out, h.bins.data(), h.bins.size());
    }
  }

  void built_event(const rootsink::BuiltEvent& ev, uint64_t created_ns,
                   std::vector<uint8_t>& out) {
    open_envelope(out, ev.run_number, created_ns);
    mpw::map_len(out, 5);
    mpw::str(out, "kind");
    mpw::str(out, "built_event");
    mpw::str(out, "run");
    mpw::uint_(out, ev.run_number);
    mpw::str(out, "event_idx");
    mpw::uint_(out, ev.event_idx);
    mpw::str(out, "complete");
    mpw::boolean(out, !ev.incomplete);
    mpw::str(out, "fragments");
    mpw::array_len(out, static_cast<uint32_t>(ev.fragments.size()));
    for (const rootsink::OwnedFragment& f : ev.fragments) write_fragment(out, f);
  }

 private:
  // fixmap(1){"Data": array(5)[...]} の 4 フィールド目までを書く(SPEC §2.2)。
  void open_envelope(std::vector<uint8_t>& out, uint32_t run, uint64_t created_ns) {
    out.clear();  // 容量は保たれる(ホットパスで確保し直さない)
    mpw::map_len(out, 1);
    mpw::str(out, "Data");
    mpw::array_len(out, 5);
    mpw::uint_(out, source_id_);
    mpw::uint_(out, run);
    mpw::uint_(out, seq_++);
    mpw::uint_(out, created_ns);
  }

  // 0 件の CoBo は載せない(毎秒 256 個のゼロを流さない)。キーは 10 進文字列・cobo 昇順。
  static void write_frames_per_cobo(std::vector<uint8_t>& out, const Status& s) {
    uint32_t n = 0;
    for (size_t c = 0; c < kMaxCobo; ++c) {
      if (s.frames_per_cobo[c] != 0) ++n;
    }
    mpw::map_len(out, n);
    char key[8];
    for (size_t c = 0; c < kMaxCobo; ++c) {
      if (s.frames_per_cobo[c] == 0) continue;
      const int len = std::snprintf(key, sizeof(key), "%zu", c);
      mpw::str(out, key, static_cast<size_t>(len < 0 ? 0 : len));
      mpw::uint_(out, s.frames_per_cobo[c]);
    }
  }

  // SPEC §2.4 の positional array(12)。フィールド順は tpc_wire.hpp の読み手と対。
  static void write_fragment(std::vector<uint8_t>& out, const rootsink::OwnedFragment& f) {
    mpw::array_len(out, tpcwire::kFragmentFields);
    mpw::uint_(out, f.event_idx);
    mpw::uint_(out, f.event_time);
    mpw::uint_(out, f.cobo);
    mpw::uint_(out, f.asad);
    mpw::uint_(out, f.frame_type);
    mpw::uint_(out, f.revision);
    mpw::uint_(out, f.read_offset);
    mpw::uint_(out, f.status);
    mpw::array_len(out, 4);
    for (int k = 0; k < 4; ++k) mpw::uint_(out, f.mult[k]);
    mpw::uint_(out, f.window_out);
    mpw::array_len(out, 4);
    for (int k = 0; k < 4; ++k) mpw::uint_(out, f.last_cell[k]);
    mpw::bin(out, f.items.data(), f.items.size());
  }

  uint32_t source_id_;
  uint64_t seq_ = 0;
};

}  // namespace rsmon

#endif  // TPCDAQ_ROOT_SINK_MONITOR_PUB_HPP
