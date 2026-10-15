// eb_core.hpp — eventIdx イベントビルダ(SPEC §6.3)。ZMQ 非依存・ROOT 非依存。
//
// 出自: **新規**。delila-rs `tools/root_sink/eb_core.hpp` の Sorter / SeqReorder は
// TPC には使えない(SPEC §6.1「新規」欄・§14-1: ソース毎 FIFO も timestamp キーも
// 実在しない)。流用したのは Channel<T>(rs_core.hpp 側、008 の波)だけ。
//
// このヘッダがやること —— (run_number, 世代, event_idx) をキーに、設定で与えた期待
// フラグメント集合 {(cobo, asad)} が揃うまで貯め、揃ったら(または build_timeout_ms
// を過ぎたら)**世代内で event_idx 昇順**で吐き出す。世代は取得の再開(TODO/110)で
// ソース(cobo, asad)ごとに +1 され、再開前後のフレームを混ぜない(TODO/133)。「到着順連結で順序が狂う」現行 GET の
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
#include <tuple>
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
  // 取得の再開(restart)で +1 される世代(その run 内、0 始まり)。世代はソース(cobo, asad)
  // ごとに持ち、イベントの鍵は (run, epoch, event_idx)(TODO/133)。recorder は世代が
  // 変わったら「最後に書いた eventId」の高水位をリセットする(TODO/110)。
  uint32_t epoch = 0;
  std::vector<OwnedFragment> fragments;
};

// emit 済みの (run, event_idx) に遅れて届いたフラグメント。**捨てない** ——
// 呼び手が書き出す責任を持つ(SPEC §6.3「順序より可逆性優先」)。
struct LateFragment {
  OwnedFragment fragment;
  uint32_t emitted_upto = 0;  // そのフラグメントの世代で emit 済みの最大 event_idx(可視化用)
};

// ---------------------------------------------------------------------------
// 3. EventBuilder
// ---------------------------------------------------------------------------

class EventBuilder {
 public:
  // 再開判定の時刻後退幅の下限(TODO/134)。CoBo の event_time は 10 ns 刻みなので 1e7 ticks = 100 ms。
  // 板どうしの時計ずれは µs 級、本物の取得再開は秒〜数十秒戻る(実機 run 73: 79.6 s → 0.28 s)。
  // 閾値未満の後退は再開ではなく通常の遅延到着として扱う(偽陽性で新イベントを捨てない)。
  static constexpr uint64_t kRestartMinTimeGap = 10000000;

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
  // **取得の再開(TODO/110、133 でソースごとに)**: そのソース(cobo, asad)の現世代で
  // より大きい idx(> kRestartMinIdx)を見た後に、event_idx==0 のフレームが、そのソースの
  // 現世代で見た最小 event_time より kRestartMinTimeGap(100 ms)以上前の時刻で来たら、
  // ECC 不在で走り続けた zCoBo の古いフレームの後に新しい取得が始まったと見なし、
  // **そのソースの**世代を +1 する(板のずれ = µs は再開にしない — 134)。
  // late 判定より前に見る。フラグメントはそのソースの現世代の鍵 (run, epoch, idx) に入る
  // ので、まだ再開していない AsAd の古いフレームが新世代に混ざらない(133 §1b)。
  // 期待集合の全ソースが次の世代に進んだら(= run の世代が上がったら)旧世代の pending を
  // 押し出し(take_restart_flush() で受け取る)、acquisition_restarts を +1 する。
  std::optional<LateFragment> feed(OwnedFragment f, uint64_t now_ms) {
    const uint32_t run = f.run_number;
    const FragmentKey fk = f.key();
    RunState& rs = runs_[run];
    SourceState& src = rs.sources[fk];
    if (f.event_idx == 0 && src.max_seen > kRestartMinIdx && src.have_time &&
        f.event_time + kRestartMinTimeGap < src.time_floor) {
      restart_source(run, rs, src, fk);
    }
    const EpochState& es = rs.epochs[src.epoch];
    if (es.have_emitted && f.event_idx <= es.emitted_upto) {
      ++late_fragments_;
      LateFragment late;
      late.emitted_upto = es.emitted_upto;
      late.fragment = std::move(f);
      return late;
    }
    if (!src.have_time || f.event_time < src.time_floor) src.time_floor = f.event_time;
    src.have_time = true;
    if (f.event_idx > src.max_seen) src.max_seen = f.event_idx;

    Pending& ev = pending_[EventKey{run, src.epoch, f.event_idx}];
    if (ev.fragments.empty()) ev.first_arrival_ms = now_ms;
    if (expected_.count(fk) == 0) {
      ++unexpected_fragments_;  // complete 判定には数えないが、イベントには載せる
    } else if (!ev.seen.insert(fk).second) {
      ++duplicate_fragments_;  // 同じソースから 2 回 = 片肺での早出しを防ぐ
    }
    ev.fragments.push_back(std::move(f));
    return std::nullopt;
  }

  // 出せるイベントを (世代, event_idx) 昇順で返す。head-of-line は **(run, 世代) ごと**:
  // 世代内の先頭が complete でも timeout でもない間はその世代の後続を出さないが、別の世代は
  // 待たない(死んだ AsAd の旧世代が新世代を塞がない — 133 §4-3)。run を跨いだ順序は従来どおり
  // (前の run に塞がった世代が残っている間は次の run を出さない)。
  std::vector<BuiltEvent> poll(uint64_t now_ms) {
    std::vector<BuiltEvent> out;
    auto it = pending_.begin();
    if (it == pending_.end()) return out;
    uint32_t run = it->first.run;
    bool run_blocked = false;
    while (it != pending_.end()) {
      if (it->first.run != run) {
        if (run_blocked) break;
        run = it->first.run;
      }
      if (!is_complete(it->second) && !timed_out(it->second, now_ms)) {
        run_blocked = true;
        it = pending_.lower_bound(EventKey{run, it->first.epoch + 1, 0});  // 次の世代へ
        continue;
      }
      emit_into(it, out);
      it = pending_.erase(it);
    }
    return out;
  }

  // run の世代が上がったときに押し出した旧世代のイベント(昇順)を渡して空にする。
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
  // 取得の再開(TODO/110)。累積。acquisition_restarts は **run の世代**(期待集合の全ソースの
  // 世代の最小値)が上がった回数 = 4 AsAd 全部が再開して初めて 1 回(TODO/133)。
  uint64_t acquisition_restarts() const { return acquisition_restarts_; }
  uint64_t events_before_restart() const { return events_before_restart_; }  // 旧世代で emit した数
  uint64_t restart_duplicate_events() const { return restart_duplicate_events_; }  // 新世代で捨てた数
  // ソース(cobo, asad)ごとの再開の回数(累積、TODO/133)。再開したソースだけが載る。
  const std::map<FragmentKey, uint64_t>& source_restarts() const { return source_restarts_; }
  // その run で、世代が最小(= run の世代)に留まり他のソースより遅れている期待ソースの数。
  // 全ソースが同じ世代なら 0(TODO/133 §4-5: 死んだ AsAd の可視化 — status PUB)。
  uint64_t sources_behind(uint32_t run) const {
    const auto it = runs_.find(run);
    if (it == runs_.end()) return 0;
    uint32_t lo = UINT32_MAX;
    uint32_t hi = 0;
    for (const FragmentKey& k : expected_) {
      const uint32_t e = it->second.source_epoch(k);
      lo = std::min(lo, e);
      hi = std::max(hi, e);
    }
    if (lo == hi) return 0;
    uint64_t n = 0;
    for (const FragmentKey& k : expected_) {
      if (it->second.source_epoch(k) == lo) ++n;
    }
    return n;
  }
  // run を閉じたら emit 済み番号の区間リストを解放する(EOS の flush 後に呼ぶ)。
  // 小さな状態(emitted_upto 等)は、EOS 後の迷子を late と判定するために残す。
  void close_run(uint32_t run) {
    const auto it = runs_.find(run);
    if (it == runs_.end()) return;
    for (auto& e : it->second.epochs) e.second.ids.release();
  }
  // テスト・可視化用: その run が保持している区間数(全世代の合計)。
  size_t emitted_id_ranges(uint32_t run) const {
    const auto it = runs_.find(run);
    if (it == runs_.end()) return 0;
    size_t n = 0;
    for (const auto& e : it->second.epochs) n += e.second.ids.v.size();
    return n;
  }
  size_t pending() const { return pending_.size(); }

 private:
  // イベントの鍵 = (run_number, ソースの世代, event_idx)(TODO/133)。map の順 = この辞書順。
  struct EventKey {
    uint32_t run;
    uint32_t epoch;
    uint32_t idx;
    bool operator<(const EventKey& o) const {
      return std::tie(run, epoch, idx) < std::tie(o.run, o.epoch, o.idx);
    }
  };

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
    void release() { std::vector<std::pair<uint32_t, uint32_t>>().swap(v); }
  };

  // ソース(cobo, asad)ごとの再開判定の材料(TODO/133)。値はそのソースの現世代のもの。
  struct SourceState {
    uint32_t epoch = 0;
    uint32_t max_seen = 0;    // 現世代で見た最大 event_idx(pending 含む)
    bool have_time = false;
    uint64_t time_floor = 0;  // 現世代で見た最小 event_time
  };

  // 世代ごとの emit の状態。世代内で emit は昇順(late 判定の高水位はここ)。
  struct EpochState {
    bool have_emitted = false;
    uint32_t emitted_upto = 0;  // この世代の emit 済み最大 event_idx
    uint64_t count = 0;         // この世代で emit した数
    IdRanges ids;               // この世代で emit した event_idx
  };

  // run 毎の状態。
  struct RunState {
    uint32_t epoch = 0;       // run の世代 = 期待集合の全ソースの世代の最小値
    bool dup_logged = false;  // restart_duplicate の stderr は run 毎に 1 回
    std::map<FragmentKey, SourceState> sources;
    std::map<uint32_t, EpochState> epochs;

    uint32_t source_epoch(const FragmentKey& k) const {
      const auto it = sources.find(k);
      return it == sources.end() ? 0 : it->second.epoch;  // まだ来ていないソースは世代 0
    }
  };

  // ソース 1 つの再開。そのソースの判定材料を消して世代を +1 する。期待集合の全ソースが
  // 次の世代に進んだ(= run の世代が上がった)ら旧世代を締める。
  void restart_source(uint32_t run, RunState& rs, SourceState& src, const FragmentKey& fk) {
    src.max_seen = 0;
    src.have_time = false;
    src.time_floor = 0;
    ++src.epoch;
    ++source_restarts_[fk];
    uint32_t lo = UINT32_MAX;
    for (const FragmentKey& k : expected_) lo = std::min(lo, rs.source_epoch(k));
    if (lo > rs.epoch) retire_epoch(run, rs);  // 1 フラグメントで上がるのは高々 1
  }

  // run の世代を締める: 旧世代(以下)の pending を昇順に押し出す(complete / incomplete の
  // まま — 捨てない)→ 旧世代で emit した数を events_before_restart へ → 世代 +1。
  void retire_epoch(uint32_t run, RunState& rs) {
    const auto first = pending_.lower_bound(EventKey{run, 0, 0});
    const auto last = pending_.lower_bound(EventKey{run, rs.epoch + 1, 0});
    for (auto it = first; it != last; ++it) emit_into(it, restart_flush_);
    pending_.erase(first, last);
    events_before_restart_ += rs.epochs[rs.epoch].count;
    ++rs.epoch;
    ++acquisition_restarts_;
  }

  // emit して out に積む。**別の世代が emit 済みの番号**と同じイベントは書かずに数える
  // (restart_duplicate_events — ユーザー裁定 2026-10-13。eventId は run の中で一回きり)。
  void emit_into(std::map<EventKey, Pending>::iterator it, std::vector<BuiltEvent>& out) {
    const uint32_t run = it->first.run;
    const uint32_t epoch = it->first.epoch;
    const uint32_t idx = it->first.idx;
    RunState& rs = runs_[run];
    EpochState& es = rs.epochs[epoch];
    // 世代内の昇順は保たれる(drop しても高水位は進める — 以降の late 判定のため)。
    es.have_emitted = true;
    es.emitted_upto = idx;
    if (emitted_in_another_epoch(rs, epoch, idx)) {
      ++restart_duplicate_events_;
      if (!rs.dup_logged) {  // run 毎に 1 回(ホットパスで毎回整形しない)
        rs.dup_logged = true;
        std::fprintf(stderr,
                     "root_sink: run %u epoch %u: event_idx=%u was already emitted before "
                     "the acquisition restart — not written, counted (raw graw keeps the "
                     "data)\n",
                     run, epoch, idx);
      }
      return;
    }
    es.ids.add(idx);
    ++es.count;
    BuiltEvent ev = emit(it);
    ev.epoch = epoch;
    out.push_back(std::move(ev));
  }

  // 世代は run 内で数個なので素直に回す(世代が 1 つなら 1 回で抜ける)。
  static bool emitted_in_another_epoch(const RunState& rs, uint32_t epoch, uint32_t idx) {
    for (const auto& e : rs.epochs) {
      if (e.first != epoch && e.second.ids.contains(idx)) return true;
    }
    return false;
  }

  BuiltEvent emit(std::map<EventKey, Pending>::iterator it) {
    BuiltEvent ev;
    ev.run_number = it->first.run;
    ev.event_idx = it->first.idx;
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
  std::map<EventKey, Pending> pending_;      // キー順 = (run, 世代, event_idx) 昇順
  // run → 状態。run が閉じても消さない(EOS 後に来た迷子を pending に戻さないため)。
  std::map<uint32_t, RunState> runs_;
  std::vector<BuiltEvent> restart_flush_;  // take_restart_flush() 待ち(run の世代が上がった時の旧世代)

  uint64_t events_complete_ = 0;
  uint64_t events_incomplete_ = 0;
  uint64_t late_fragments_ = 0;
  uint64_t unexpected_fragments_ = 0;
  uint64_t duplicate_fragments_ = 0;
  uint64_t acquisition_restarts_ = 0;
  uint64_t events_before_restart_ = 0;
  uint64_t restart_duplicate_events_ = 0;
  std::map<FragmentKey, uint64_t> source_restarts_;
};

}  // namespace rootsink

#endif  // TPCDAQ_ROOT_SINK_EB_CORE_HPP
