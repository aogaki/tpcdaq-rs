// eb_core.hpp — eventIdx イベントビルダ(SPEC §6.3)。ZMQ 非依存・ROOT 非依存。
//
// 出自: **新規**。delila-rs `tools/root_sink/eb_core.hpp` の Sorter / SeqReorder は
// TPC には使えない(SPEC §6.1「新規」欄・§14-1: ソース毎 FIFO も timestamp キーも
// 実在しない)。流用したのは Channel<T>(rs_core.hpp 側、008 の波)だけ。
//
// このヘッダがやること —— (run_number, event_idx) をキーに、設定で与えた期待
// フラグメント集合 {(cobo, asad)} が揃うまで貯め、揃ったら(または build_timeout_ms
// を過ぎたら)**event_idx 昇順**で吐き出す。「到着順連結で順序が狂う」現行 GET の
// 問題(PROPOSAL Q3)の構造的回避点がここ。
//
// 設計上の決めごと(すべて SPEC §6.3 と CLAUDE.md の絶対ルール由来):
//   * **何も捨てない**。揃わなかったイベントは incomplete フラグ付きで出す。emit 済みの
//     イベントに遅れて届いたフラグメントは LateFragment として呼び手に返す(呼び手が
//     書き出す責任を持つ ——「順序より可逆性優先」)。期待集合外・重複の (cobo,asad) も
//     イベントに載せたうえで別カウンタで可視化する(silent failure を作らない)。
//   * **時計を内部で読まない**。now_ms は必ず引数で注入する。タイムアウトの試験が
//     sleep 無しで決定的に書けることを優先した(テストは仕様書 — CLAUDE.md)。
//   * **head-of-line**: 先頭(最小 event_idx)が complete か timeout になるまで後続は
//     出さない。これが「昇順で出る」ことの実装手段。
//   * fragment の所有コピー(OwnedFragment): ビルダはバッチ(ZmqMsg)の寿命を超えて
//     保持するので参照では持てない。ゼロコピー化は P5 で実測してから(KISS)。
//
// **NDEBUG を定義しないこと**: 期待集合が空のビルダを止める assert が消える。

#ifndef TPCDAQ_ROOT_SINK_EB_CORE_HPP
#define TPCDAQ_ROOT_SINK_EB_CORE_HPP

#include <algorithm>
#include <cassert>
#include <cstdint>
#include <cstdio>
#include <iterator>
#include <map>
#include <optional>
#include <set>
#include <utility>
#include <vector>

#include "tpc_wire.hpp"

namespace rootsink {

// build_timeout_ms の既定(SPEC §6.3)。
constexpr uint64_t kDefaultBuildTimeoutMs = 1000;

// 期待フラグメント集合の要素 = (cobo, asad)。
using FragmentKey = std::pair<uint8_t, uint8_t>;

// ---------------------------------------------------------------------------
// 1. OwnedFragment — Fragment(SPEC §2.4)の所有コピー
// ---------------------------------------------------------------------------
//
// `tpcwire::FragmentView` は受信バッファへのスライス。ビルダはバッチの寿命を超えて
// 持つので、ヘッダ全フィールド + items バイト列を所有コピーする。
// `run_number` だけは Fragment 自身が持たないので Batch ヘッダ(§2.2)から渡す。
struct OwnedFragment {
  uint32_t run_number = 0;  // Batch ヘッダ由来(キー (run, event_idx) の片割れ)
  uint32_t event_idx = 0;
  uint64_t event_time = 0;  // 48bit 有効
  uint8_t cobo = 0;
  uint8_t asad = 0;
  uint8_t frame_type = 0;
  uint8_t revision = 0;
  uint16_t read_offset = 0;
  uint8_t status = 0;
  uint16_t mult[4] = {0, 0, 0, 0};
  uint32_t window_out = 0;
  uint16_t last_cell[4] = {0, 0, 0, 0};
  std::vector<uint8_t> items;  // u32 LE の連結(1 item = 1 サンプル)

  FragmentKey key() const { return FragmentKey(cobo, asad); }
  size_t item_count() const { return items.size() / 4; }

  // i 番目の item(u32 LE)。範囲外は呼ばないこと(呼び手が item_count で回す)。
  uint32_t item(size_t i) const {
    const uint8_t* p = items.data() + i * 4;
    return static_cast<uint32_t>(p[0]) | (static_cast<uint32_t>(p[1]) << 8) |
           (static_cast<uint32_t>(p[2]) << 16) | (static_cast<uint32_t>(p[3]) << 24);
  }

  static OwnedFragment from_view(const tpcwire::FragmentView& v, uint32_t run_number) {
    OwnedFragment f;
    f.run_number = run_number;
    f.event_idx = v.event_idx;
    f.event_time = v.event_time;
    f.cobo = v.cobo;
    f.asad = v.asad;
    f.frame_type = v.frame_type;
    f.revision = v.revision;
    f.read_offset = v.read_offset;
    f.status = v.status;
    for (int k = 0; k < 4; ++k) f.mult[k] = v.mult[k];
    f.window_out = v.window_out;
    for (int k = 0; k < 4; ++k) f.last_cell[k] = v.last_cell[k];
    f.items.assign(v.items, v.items + v.items_size);
    return f;
  }
};

// ---------------------------------------------------------------------------
// 2. 出力型
// ---------------------------------------------------------------------------

// 組み上がったイベント。fragments は **(cobo, asad) 昇順**(SPEC v1.3 §6.3)。
// 期待集合外・重複も落とさずここに入る(整列は期待集合の内外を問わない)。
struct BuiltEvent {
  uint32_t run_number = 0;
  uint32_t event_idx = 0;
  bool incomplete = false;  // 期待集合が揃わないまま出た(timeout / flush)
  // 取得の再開(restart)で +1 される世代(その run 内、0 始まり)。recorder は世代が
  // 変わったら「最後に書いた eventId」の高水位をリセットする(TODO/110)。
  uint32_t epoch = 0;
  std::vector<OwnedFragment> fragments;
};

// emit 済みの (run, event_idx) に遅れて届いたフラグメント。**捨てない** ——
// 呼び手が書き出す責任を持つ(SPEC §6.3「順序より可逆性優先」)。
struct LateFragment {
  OwnedFragment fragment;
  uint32_t emitted_upto = 0;  // その run で emit 済みの最大 event_idx(可視化用)
};

// ---------------------------------------------------------------------------
// 3. EventBuilder
// ---------------------------------------------------------------------------

class EventBuilder {
 public:
  // expected = 設定 + ジオメトリ由来の {(cobo, asad)}(mini: {(0,0)}、
  // ELITPC: {(0,0),(0,1),(1,0),(1,1)})。空集合は「全イベントが即 complete」という
  // 無意味な動作になるので設定ミスとして止める。
  explicit EventBuilder(std::set<FragmentKey> expected,
                        uint64_t build_timeout_ms = kDefaultBuildTimeoutMs)
      : expected_(std::move(expected)), build_timeout_ms_(build_timeout_ms) {
    assert(!expected_.empty() && "EventBuilder needs at least one expected (cobo,asad)");
  }

  // フラグメントを 1 個食わせる。戻り値が入っていれば遅延到着(呼び手が書き出す)。
  //
  // **取得の再開(TODO/110)**: event_idx==0 のフレームが、同じ run・同じ世代で complete または
  // emit 済みのイベントがあり(2 枚目の板の event 0 が未着の間は誤検出しない。poll 前の
  // 同一バッチ内でも検出できるよう、emit だけでなく complete でも立てる)、より大きい idx
  // (> kRestartMinIdx)を見た後に、その世代で見た最小 event_time より小さい時刻で来たら、
  // ECC 不在で走り続けた zCoBo の古いフレームの後に新しい取得が始まったと見なす。
  // late 判定より前に見る。起きたら旧世代の pending を押し出し(take_restart_flush() で
  // 受け取る)、世代を +1 して高水位をリセットする。
  std::optional<LateFragment> feed(OwnedFragment f, uint64_t now_ms) {
    RunState& rs = runs_[f.run_number];
    if (f.event_idx == 0 && (rs.have_emitted || rs.have_complete) && rs.max_seen > kRestartMinIdx && rs.have_time &&
        f.event_time < rs.time_floor) {
      restart(f.run_number, rs);
    }
    if (rs.have_emitted && f.event_idx <= rs.emitted_upto) {
      ++late_fragments_;
      LateFragment late;
      late.emitted_upto = rs.emitted_upto;
      late.fragment = std::move(f);
      return late;
    }
    if (!rs.have_time || f.event_time < rs.time_floor) rs.time_floor = f.event_time;
    rs.have_time = true;
    if (f.event_idx > rs.max_seen) rs.max_seen = f.event_idx;

    Pending& ev = pending_[EventKey(f.run_number, f.event_idx)];
    if (ev.fragments.empty()) ev.first_arrival_ms = now_ms;
    const FragmentKey fk = f.key();
    if (expected_.count(fk) == 0) {
      ++unexpected_fragments_;  // complete 判定には数えないが、イベントには載せる
    } else if (!ev.seen.insert(fk).second) {
      ++duplicate_fragments_;  // 同じソースから 2 回 = 片肺での早出しを防ぐ
    }
    ev.fragments.push_back(std::move(f));
    if (is_complete(ev)) rs.have_complete = true;
    return std::nullopt;
  }

  // 出せるイベントを event_idx 昇順で返す。先頭が complete でも timeout でもない間は
  // 後続を出さない(head-of-line)。
  std::vector<BuiltEvent> poll(uint64_t now_ms) {
    std::vector<BuiltEvent> out;
    while (!pending_.empty()) {
      const auto it = pending_.begin();
      if (!is_complete(it->second) && !timed_out(it->second, now_ms)) break;
      emit_into(it, out);
      pending_.erase(it);
    }
    return out;
  }

  // feed() が restart を検出したときに押し出した旧世代のイベント(昇順)を渡して空にする。
  // 呼び手は **eb.poll() の結果より先に** 書き出すこと(旧世代 → 新世代の順)。
  std::vector<BuiltEvent> take_restart_flush() {
    std::vector<BuiltEvent> out;
    out.swap(restart_flush_);
    return out;
  }

  // EOS 時: 残りを全部昇順で出す。揃っていたものは complete のまま出す
  // (counts の意味を壊さない —— 「全部 incomplete」と嘘をつかない)。
  std::vector<BuiltEvent> flush() {
    std::vector<BuiltEvent> out;
    while (!pending_.empty()) {
      const auto it = pending_.begin();
      emit_into(it, out);
      pending_.erase(it);
    }
    return out;
  }

  // --- カウンタ(すべて累積。pending だけ瞬間値)---
  uint64_t events_complete() const { return events_complete_; }
  uint64_t events_incomplete() const { return events_incomplete_; }
  uint64_t late_fragments() const { return late_fragments_; }
  uint64_t unexpected_fragments() const { return unexpected_fragments_; }
  uint64_t duplicate_fragments() const { return duplicate_fragments_; }
  // 取得の再開(TODO/110)。累積。
  uint64_t acquisition_restarts() const { return acquisition_restarts_; }
  uint64_t events_before_restart() const { return events_before_restart_; }  // 旧世代で emit した数
  uint64_t restart_duplicate_events() const { return restart_duplicate_events_; }  // 新世代で捨てた数
  // run を閉じたら emit 済み番号の区間リストを解放する(EOS の flush 後に呼ぶ)。
  // 小さな状態(emitted_upto 等)は、EOS 後の迷子を late と判定するために残す。
  void close_run(uint32_t run) {
    const auto it = runs_.find(run);
    if (it == runs_.end()) return;
    it->second.epoch_ids.release();
    it->second.retired_ids.release();
  }
  // テスト・可視化用: その run が保持している区間数(現世代 + 旧世代)。
  size_t emitted_id_ranges(uint32_t run) const {
    const auto it = runs_.find(run);
    return it == runs_.end() ? 0 : it->second.epoch_ids.v.size() + it->second.retired_ids.v.size();
  }
  size_t pending() const { return pending_.size(); }

 private:
  using EventKey = std::pair<uint32_t, uint32_t>;  // (run_number, event_idx)

  struct Pending {
    uint64_t first_arrival_ms = 0;
    std::set<FragmentKey> seen;  // 期待集合のうち到着済み(重複は入らない)
    std::vector<OwnedFragment> fragments;
  };

  bool is_complete(const Pending& ev) const { return ev.seen.size() == expected_.size(); }

  bool timed_out(const Pending& ev, uint64_t now_ms) const {
    // 巻き戻った時計(テストの注入も含む)を「即タイムアウト」にしない。
    if (now_ms <= ev.first_arrival_ms) return false;
    return (now_ms - ev.first_arrival_ms) > build_timeout_ms_;
  }

  // restart の最小 idx ゲート(decoder と同じ。隣接混在 idx 1→0 を誤検出しない)。
  static constexpr uint32_t kRestartMinIdx = 8;

  // emit 済み event_idx の区間 [lo, hi] リスト(昇順)。世代内の emit は昇順なので
  // 末尾の延長か追加だけで済み、欠番が無ければ 1 区間(常駐プロセスで RSS を伸ばさない)。
  struct IdRanges {
    std::vector<std::pair<uint32_t, uint32_t>> v;
    void add(uint32_t idx) {
      if (!v.empty() && idx <= v.back().second + 1ull && idx >= v.back().first) {
        if (idx > v.back().second) v.back().second = idx;
      } else {
        v.emplace_back(idx, idx);
      }
    }
    bool contains(uint32_t idx) const {
      auto it = std::upper_bound(v.begin(), v.end(), idx,
                                 [](uint32_t x, const std::pair<uint32_t, uint32_t>& r) {
                                   return x < r.first;
                                 });
      return it != v.begin() && idx <= std::prev(it)->second;
    }
    void absorb(const IdRanges& o) {  // 併合(昇順に整え、隣接・重複を結ぶ)
      v.insert(v.end(), o.v.begin(), o.v.end());
      std::sort(v.begin(), v.end());
      std::vector<std::pair<uint32_t, uint32_t>> m;
      for (const auto& r : v) {
        if (!m.empty() && r.first <= m.back().second + 1ull) {
          if (r.second > m.back().second) m.back().second = r.second;
        } else {
          m.push_back(r);
        }
      }
      v.swap(m);
    }
    void release() { std::vector<std::pair<uint32_t, uint32_t>>().swap(v); }
  };

  // run 毎の状態。世代(epoch)内で emit は昇順。
  struct RunState {
    bool have_emitted = false;
    bool have_complete = false;     // 現世代に期待フラグメントが揃ったイベントがある
    uint32_t emitted_upto = 0;      // 現世代の emit 済み最大 event_idx
    uint32_t max_seen = 0;          // 現世代で見た最大 event_idx(pending 含む)
    bool have_time = false;
    uint64_t time_floor = 0;        // 現世代で見た最小 event_time
    uint32_t epoch = 0;
    bool dup_logged = false;        // restart_duplicate の stderr は run 毎に 1 回
    uint64_t epoch_count = 0;       // 現世代で emit した数
    IdRanges epoch_ids;             // 現世代で emit した event_idx
    IdRanges retired_ids;           // 旧世代で emit した event_idx
  };

  void restart(uint32_t run, RunState& rs) {
    // 旧世代の pending を昇順に押し出す(complete / incomplete のまま — 捨てない)。
    const auto first = pending_.lower_bound(EventKey(run, 0));
    auto it = first;
    while (it != pending_.end() && it->first.first == run) {
      emit_into(it, restart_flush_);
      ++it;
    }
    pending_.erase(first, it);

    events_before_restart_ += rs.epoch_count;
    rs.retired_ids.absorb(rs.epoch_ids);
    rs.epoch_ids.release();
    rs.epoch_count = 0;
    rs.have_emitted = false;
    rs.have_complete = false;
    rs.emitted_upto = 0;
    rs.max_seen = 0;
    rs.have_time = false;
    rs.time_floor = 0;
    ++rs.epoch;
    ++acquisition_restarts_;
  }

  // emit して out に積む。新世代で、旧世代が emit 済みの番号と同じイベントは
  // 書かずに数える(restart_duplicate_events — ユーザー裁定 2026-10-13)。
  void emit_into(std::map<EventKey, Pending>::iterator it, std::vector<BuiltEvent>& out) {
    RunState& rs = runs_[it->first.first];
    const uint32_t idx = it->first.second;
    // 世代内の昇順は保たれる(drop しても高水位は進める — 以降の late 判定のため)。
    rs.have_emitted = true;
    rs.emitted_upto = idx;
    if (rs.epoch > 0 &&
        rs.retired_ids.contains(idx)) {
      ++restart_duplicate_events_;
      if (!rs.dup_logged) {  // run 毎に 1 回(ホットパスで毎回整形しない)
        rs.dup_logged = true;
        std::fprintf(stderr,
                     "root_sink: run %u epoch %u: event_idx=%u was already emitted before "
                     "the acquisition restart — not written, counted (raw graw keeps the "
                     "data)\n",
                     it->first.first, rs.epoch, idx);
      }
      return;
    }
    rs.epoch_ids.add(idx);
    ++rs.epoch_count;
    BuiltEvent ev = emit(it);
    ev.epoch = rs.epoch;
    out.push_back(std::move(ev));
  }

  BuiltEvent emit(std::map<EventKey, Pending>::iterator it) {
    BuiltEvent ev;
    ev.run_number = it->first.first;
    ev.event_idx = it->first.second;
    ev.incomplete = !is_complete(it->second);
    ev.fragments = std::move(it->second.fragments);
    // **(cobo, asad) 昇順で決定的にする**(SPEC v1.3 §6.3)。到着順は run 毎に揺れるので、
    // そのまま TTree に流すと §12-4 の 2 ソースビルド一致比較が順序で偽陰性になる。
    // 期待集合は高々数要素なので、emit 時に 1 回並べ替えるだけで足りる(KISS)。
    // **stable_sort**: 同キー(= 重複フラグメント)同士は到着順を保つ —— 捨てないと決めた
    // ものの前後関係を勝手に作り変えない。
    std::stable_sort(ev.fragments.begin(), ev.fragments.end(),
                     [](const OwnedFragment& a, const OwnedFragment& b) {
                       return a.key() < b.key();
                     });
    if (ev.incomplete) {
      ++events_incomplete_;
    } else {
      ++events_complete_;
    }
    return ev;
  }

  std::set<FragmentKey> expected_;
  uint64_t build_timeout_ms_;
  std::map<EventKey, Pending> pending_;      // キー順 = (run, event_idx) 昇順
  // run → 状態。run が閉じても消さない(EOS 後に来た迷子を pending に戻さないため)。
  std::map<uint32_t, RunState> runs_;
  std::vector<BuiltEvent> restart_flush_;  // take_restart_flush() 待ち

  uint64_t events_complete_ = 0;
  uint64_t events_incomplete_ = 0;
  uint64_t late_fragments_ = 0;
  uint64_t unexpected_fragments_ = 0;
  uint64_t duplicate_fragments_ = 0;
  uint64_t acquisition_restarts_ = 0;
  uint64_t events_before_restart_ = 0;
  uint64_t restart_duplicate_events_ = 0;
};

}  // namespace rootsink

#endif  // TPCDAQ_ROOT_SINK_EB_CORE_HPP
