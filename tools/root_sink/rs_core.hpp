// rs_core.hpp — root-sink の純ロジック(ZMQ 非依存・ROOT 非依存)。
//
// 出自: `Channel<T>` は **delila-rs `tools/root_sink/eb_core.hpp` の
// `rootsink::eb::Channel<T>` からの移植**(BSD-3-Clause、同じ作者のプロジェクト)。
// 移植にあたっての変更点(SPEC §6.2 のロスレス化チェックリスト由来):
//   * **容量 0(無制限)を廃止**。delila-rs は記録経路を無制限にして「絶対に落とさない」を
//     実現しているが、tpcdaq-rs は有限容量 + ブロックで背圧を作る(SPEC §1.4-2、
//     src/zmq_helper.rs の同じ理屈 —— 無制限は OOM = 最悪の形のデータ喪失)。
//   * **closed への push は silent discard をやめて assert で落とす**(SPEC §6.2-7)。
//     delila-rs は「shutdown 中の生産者には行き場がない」として黙って捨てていたが、
//     ロスレス契約では捨てた事実が見えないことの方が危険。
//   * 表示 tee 用の `try_push` / `pop_for` は**まだ**移植しない(このユニットに用途がない
//     —— KISS。モニタ tee = SPEC §6.2-4 は PUB を足す波で必要になったら持ってくる)。
//
// `RunState` は delila-rs `tools/root_sink/sink_core.hpp` の同名クラスの**作り直し**:
// delila の「Data を出した source を seen に貯めて、seen 全部の EOS で閉じる」方式は、
// 一度も Data を出さなかったソースがあると閉じ条件が変わってしまう。tpcdaq-rs では
// **期待ソース集合を設定から与える**(SPEC §2.3「decoder のソース性」: P2 の期待集合は
// {decoder = 100} 一つ)。コールバック(open/finalize/stale)も戻り値の enum に置き換えた
// —— 呼び手が 1 箇所しかないのにテンプレート引数を 2 つ取るのは過剰(KISS)。
//
// `SeqCheck` は新規(delila-rs は sequence_number の検査を明示的に skip している)。
// ロスレスリンクではギャップ = プロトコル違反 = fatal(SPEC §6.2-5 / §1.4-3)。
//
// **NDEBUG を定義しないこと**: §6.2-7 の assert が消える(Makefile 冒頭の注記)。

#ifndef TPCDAQ_ROOT_SINK_RS_CORE_HPP
#define TPCDAQ_ROOT_SINK_RS_CORE_HPP

#include <cassert>
#include <chrono>
#include <condition_variable>
#include <cstddef>
#include <cstdint>
#include <deque>
#include <map>
#include <mutex>
#include <set>
#include <utility>

namespace rootsink {

// decoder の source_id(SPEC §3.2 の表。receiver = cobo_id、decoder = 100、psu = 200)。
constexpr uint32_t kDecoderSourceId = 100;

// Channel<T>::pop_for の結果。「値が来た」「時間切れ(まだ生きている)」
// 「closed かつ drain 済み(= 消費側スレッドの終了合図)」を取り違えないための 3 値。
enum class PopResult { Value, Timeout, Closed };

// ---------------------------------------------------------------------------
// 1. Channel<T> — スレッド間の有界キュー(背圧の実体)
// ---------------------------------------------------------------------------
//
// 受信スレッド → 集計スレッドの唯一の受け渡し口。満杯なら push が**待つ**ので、
// ZMQ の受信ループが止まり、RCVHWM が埋まり、送り手(decoder)の PUSH send が
// ブロックする —— これが「保存系は絶対にデータを落とさない」の実装手段(SPEC §6.2-2/3)。
//
// メッセージ粒度はバッチ(8 MiB or 10 ms)なので、ロック回数は毎秒せいぜい数百回。
// lock-free 構造を持ち込む理由がない(KISS)。
template <class T>
class Channel {
 public:
  // capacity は **1 以上**。0(無制限)は方針違反なので受け付けない。
  explicit Channel(std::size_t capacity) : cap_(capacity) {
    assert(capacity > 0 && "Channel capacity 0 (unbounded) violates SPEC 1.4-2");
  }

  // 満杯なら空くまで待つ(捨てない)。closed への push は assert で落ちる(§6.2-7)。
  void push(T v) {
    std::unique_lock<std::mutex> lk(m_);
    cv_push_.wait(lk, [&] { return q_.size() < cap_ || closed_; });
    assert(!closed_ && "push on a closed Channel (SPEC 6.2-7: no silent discard)");
    q_.push_back(std::move(v));
    cv_pop_.notify_one();
  }

  // 値が来るまで待つ。false は「closed かつ drained」= 消費側スレッドの終了合図。
  // close 後も残った要素は取り出せる(drain-then-false)。
  bool pop(T& out) {
    std::unique_lock<std::mutex> lk(m_);
    cv_pop_.wait(lk, [&] { return !q_.empty() || closed_; });
    if (q_.empty()) return false;
    out = std::move(q_.front());
    q_.pop_front();
    cv_push_.notify_one();
    return true;
  }

  // タイムアウト付きの pop。**delila-rs eb_core.hpp の `Channel<T>::pop_for` から移植**。
  // 移植にあたっての変更点: delila は bool を返し「timeout も closed も false」だったが、
  // それでは呼び手が終了判定のために別途 closed を見に行くしかなく、
  //   pop_for が空で戻る → 生産者が push + close → 呼び手が closed を見て break
  // という順序で**最後の 1 通を落とす**。返り値を 3 値にして、キューの状態と closed を
  // 同じロックの中で判定する(SPEC §6.2-7 と同じ「黙って捨てない」の徹底)。
  //
  // 用途はビルダの壁時計注入 —— データが来なくても build_timeout を進めるため、
  // 集計スレッドが最大 timeout_ms 毎に起きる必要がある(SPEC §6.3)。
  PopResult pop_for(T& out, int timeout_ms) {
    std::unique_lock<std::mutex> lk(m_);
    cv_pop_.wait_for(lk, std::chrono::milliseconds(timeout_ms),
                     [&] { return !q_.empty() || closed_; });
    if (q_.empty()) return closed_ ? PopResult::Closed : PopResult::Timeout;
    out = std::move(q_.front());
    q_.pop_front();
    cv_push_.notify_one();
    return PopResult::Value;
  }

  // 待っている生産者・消費者を全部起こす。残った要素は pop できるまま。
  void close() {
    std::lock_guard<std::mutex> lk(m_);
    closed_ = true;
    cv_pop_.notify_all();
    cv_push_.notify_all();
  }

  // 瞬間値(本質的に racy。ステータス表示とテスト用)。
  std::size_t size() const {
    std::lock_guard<std::mutex> lk(m_);
    return q_.size();
  }

  std::size_t capacity() const { return cap_; }

 private:
  mutable std::mutex m_;
  std::condition_variable cv_pop_, cv_push_;
  std::deque<T> q_;
  std::size_t cap_;
  bool closed_ = false;
};

// ---------------------------------------------------------------------------
// 2. RunState — 期待ソース集合ベースの run open/close
// ---------------------------------------------------------------------------

// Data 1 通を食わせた結果。
enum class DataAction {
  Opened,     // Idle → Writing(この 1 通が run を開けた)
  Continued,  // 既に開いている run の続き
  Unexpected  // 期待ソース集合の外から来た(数えて可視化する)
};

// EndOfStream 1 通を食わせた結果。
enum class EosAction {
  Closed,   // 期待集合の全 EOS が揃った → Writing → Idle
  Pending,  // まだ揃っていない
  Ignored   // idle 中の stale EOS、または期待外ソースの EOS
};

// **期待ソース集合**(設定由来)で run の開閉を決める。
// P2 の root-sink では期待集合 = {kDecoderSourceId} の 1 要素(SPEC §2.3: decoder は
// 全 CoBo をまとめた単一ストリームとして送り、EOS も 1 本しか出さない)。
//
// 異常はどれも「黙って通す」ことをせず、カウンタとして残す(CLAUDE.md)。
class RunState {
 public:
  explicit RunState(std::set<uint32_t> expected_sources)
      : expected_(std::move(expected_sources)) {}

  bool is_open() const { return open_; }
  // 開いている run の番号(run 中に来た食い違う値では書き換えない)。
  uint32_t run_number() const { return run_number_; }

  // idle 中に届いた EOS の数(delila-rs の stale EOS 事故 — SPEC §2.2)。
  uint64_t stale_eos() const { return stale_eos_; }
  // 期待集合外の source_id から届いた Data / EOS の数。
  uint64_t unexpected_sources() const { return unexpected_sources_; }
  // 開いている run と違う run_number を載せたメッセージの数(プロトコル違反)。
  uint64_t run_number_mismatch() const { return run_number_mismatch_; }
  // 閉じた run の数。
  uint64_t runs_closed() const { return runs_closed_; }

  DataAction on_data(uint32_t source_id, uint32_t run_number) {
    if (expected_.count(source_id) == 0) {
      ++unexpected_sources_;
      return DataAction::Unexpected;
    }
    if (!open_) {
      open_ = true;
      run_number_ = run_number;
      eos_.clear();
      return DataAction::Opened;
    }
    if (run_number != run_number_) ++run_number_mismatch_;
    return DataAction::Continued;
  }

  EosAction on_eos(uint32_t source_id, uint32_t run_number) {
    if (expected_.count(source_id) == 0) {
      ++unexpected_sources_;
      return EosAction::Ignored;
    }
    if (!open_) {
      ++stale_eos_;
      return EosAction::Ignored;
    }
    // run_number の食い違いは数えるが、EOS 自体は受け取る(EOS を捨てると
    // writing のまま固まる —— delila-rs の実地事故そのもの。SPEC §2.2)。
    if (run_number != run_number_) ++run_number_mismatch_;
    eos_.insert(source_id);
    if (eos_.size() < expected_.size()) return EosAction::Pending;
    open_ = false;
    eos_.clear();
    ++runs_closed_;
    return EosAction::Closed;
  }

 private:
  std::set<uint32_t> expected_;
  std::set<uint32_t> eos_;
  bool open_ = false;
  uint32_t run_number_ = 0;
  uint64_t stale_eos_ = 0;
  uint64_t unexpected_sources_ = 0;
  uint64_t run_number_mismatch_ = 0;
  uint64_t runs_closed_ = 0;
};

// ---------------------------------------------------------------------------
// 3. SeqCheck — sequence_number の連続性(SPEC §6.2-5)
// ---------------------------------------------------------------------------

enum class SeqAction {
  First,      // そのソースの初回(基準値になる)
  Ok,         // 直前 + 1
  Gap,        // 飛んだ = ロスレス契約違反(呼び手は fatal 扱い)
  Regression  // 巻き戻り or 重複(再送・ソース再起動の取り違えを黙って通さない)
};

// ソース毎に直前の sequence_number を覚えるだけ。run 境界では reset() すること
// (SPEC §2.2: run 開始で 0 リセット)。
//
// **厳格モード**(`SeqCheck(true)`、008 レビュー申し送りの解消): 各ソースの初回
// sequence_number は **0 のみ受理**し、≠0 は Gap(= 呼び手は fatal)にする。
// SPEC §2.2 が「run 開始で 0 リセット」を規定しているロスレスリンク(decoder →
// root-sink)では、初回を無条件に基準値にすると**先頭バッチの喪失が不可視**になる。
//
// 既定は非厳格(初回を基準にする)。途中から繋いだ検査ツールや、0 起点を約束していない
// リンクではこちらを使う —— 既存の呼び手・テストの挙動は変えない。
class SeqCheck {
 public:
  explicit SeqCheck(bool expect_zero_start = false)
      : expect_zero_start_(expect_zero_start) {}

  SeqAction check(uint32_t source_id, uint64_t sequence_number) {
    auto it = last_.find(source_id);
    if (it == last_.end()) {
      last_.emplace(source_id, sequence_number);
      // 初回が 0 でない = そのソースの先頭バッチを取りこぼしている(黙って通さない)。
      // 基準値は進めておく(呼び手が続行を選んでも状態が壊れない)。
      if (expect_zero_start_ && sequence_number != 0) return SeqAction::Gap;
      return SeqAction::First;
    }
    const uint64_t last = it->second;
    if (sequence_number <= last) return SeqAction::Regression;
    it->second = sequence_number;
    return (sequence_number == last + 1) ? SeqAction::Ok : SeqAction::Gap;
  }

  void reset() { last_.clear(); }

 private:
  std::map<uint32_t, uint64_t> last_;
  bool expect_zero_start_;
};

}  // namespace rootsink

#endif  // TPCDAQ_ROOT_SINK_RS_CORE_HPP
