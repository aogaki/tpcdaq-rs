// test_rs_core.cpp — rs_core.hpp の単体テスト(ZMQ 不要・ROOT 不要)。
//
//   g++ -std=c++17 -O2 -Wall -Wextra -pthread test_rs_core.cpp -o test_rs_core && ./test_rs_core
//
// 有界 Channel(背圧の実体)、RunState(期待ソース集合ベースの run open/close)、
// SeqCheck(連続性 = ロスレスリンクの検算)。SPEC §6.2 チェックリスト 3/5/7。
//
// **assert は必要**: closed への push は abort する(§6.2-7)。それを確かめる死亡テストを
// 含むので、このテストは NDEBUG なしでビルドすること(Makefile がそうしている)。

#include <sys/wait.h>
#include <unistd.h>

#include <atomic>
#include <chrono>
#include <csignal>
#include <cstdint>
#include <cstdio>
#include <set>
#include <thread>
#include <vector>

#include "check.hpp"
#include "rs_core.hpp"

using namespace rootsink;

// ---------------------------------------------------------------------------
// 1. Channel<T> — 有界キュー(delila-rs eb_core.hpp から移植)
// ---------------------------------------------------------------------------

static void test_channel_is_fifo() {
  Channel<int> ch(4);
  ch.push(1);
  ch.push(2);
  ch.push(3);
  CHECK_EQ(ch.size(), 3);
  int out = 0;
  CHECK(ch.pop(out) && out == 1);
  CHECK(ch.pop(out) && out == 2);
  CHECK(ch.pop(out) && out == 3);
  CHECK_EQ(ch.size(), 0);
}

static void test_pop_blocks_until_a_push_arrives() {
  Channel<int> ch(4);
  std::atomic<int> got{0};
  std::thread consumer([&] {
    int v = 0;
    if (ch.pop(v)) got.store(v);
  });
  std::this_thread::sleep_for(std::chrono::milliseconds(20));
  CHECK_EQ(got.load(), 0);  // まだ何も来ていない = ブロックしている
  ch.push(77);
  consumer.join();
  CHECK_EQ(got.load(), 77);
}

// **このユニットの存在理由**: cap を超える push は「捨てる」のではなく「待つ」。
// これが受信スレッド → ZMQ ソケットへ伝わって背圧になる(§6.2-2/3)。
static void test_push_blocks_when_full_and_resumes_after_pop() {
  Channel<int> ch(2);
  ch.push(1);
  ch.push(2);
  CHECK_EQ(ch.size(), 2);

  std::atomic<bool> third_done{false};
  std::thread producer([&] {
    ch.push(3);
    third_done.store(true);
  });
  std::this_thread::sleep_for(std::chrono::milliseconds(30));
  CHECK(!third_done.load());  // 満杯なのでブロックしている(= 落としていない)

  int out = 0;
  CHECK(ch.pop(out) && out == 1);
  producer.join();
  CHECK(third_done.load());
  CHECK(ch.pop(out) && out == 2);
  CHECK(ch.pop(out) && out == 3);  // ブロックした値は失われない
}

static void test_close_drains_then_reports_end() {
  Channel<int> ch(4);
  ch.push(10);
  ch.push(20);
  ch.close();
  int out = 0;
  CHECK(ch.pop(out) && out == 10);  // close 後も残りは取り出せる
  CHECK(ch.pop(out) && out == 20);
  CHECK(!ch.pop(out));  // closed かつ drained = 終了合図
}

static void test_close_wakes_a_blocked_pop() {
  Channel<int> ch(4);
  std::atomic<bool> returned{false};
  std::thread consumer([&] {
    int v = 0;
    ch.pop(v);
    returned.store(true);
  });
  std::this_thread::sleep_for(std::chrono::milliseconds(20));
  CHECK(!returned.load());
  ch.close();
  consumer.join();
  CHECK(returned.load());
}

// SPEC §6.2-7: closed への push は「silent discard」ではなく assert で明示化する。
// 死亡テスト: 子プロセスで実行して SIGABRT を確認する(親は無傷)。
static void test_push_after_close_aborts() {
  pid_t pid = fork();
  if (pid == 0) {
    // 子: assert のメッセージは捨てる(テスト出力を汚さない)
    if (std::freopen("/dev/null", "w", stderr) == nullptr) _exit(3);
    Channel<int> ch(4);
    ch.close();
    ch.push(1);  // ここで abort する想定
    _exit(0);    // 到達したら「silent discard のまま」= テスト失敗
  }
  CHECK(pid > 0);
  int status = 0;
  CHECK(waitpid(pid, &status, 0) == pid);
  CHECK(WIFSIGNALED(status));
  CHECK(WIFSIGNALED(status) && WTERMSIG(status) == SIGABRT);
}

// ---------------------------------------------------------------------------
// 2. RunState — 期待ソース集合ベース(SPEC §2.3「decoder のソース性」)
// ---------------------------------------------------------------------------

// P2 の期待集合は {decoder = 100} のみ。EOS 1 本で run が閉じる。
static void test_single_source_run_opens_on_data_and_closes_on_eos() {
  RunState rs({kDecoderSourceId});
  CHECK(!rs.is_open());
  CHECK(rs.on_data(kDecoderSourceId, 7) == DataAction::Opened);
  CHECK(rs.is_open());
  CHECK_EQ(rs.run_number(), 7);
  CHECK(rs.on_data(kDecoderSourceId, 7) == DataAction::Continued);
  CHECK(rs.on_eos(kDecoderSourceId, 7) == EosAction::Closed);
  CHECK(!rs.is_open());
}

static void test_multi_source_run_waits_for_every_expected_eos() {
  RunState rs({0, 1});
  CHECK(rs.on_data(0, 3) == DataAction::Opened);
  CHECK(rs.on_eos(0, 3) == EosAction::Pending);  // 1 本目では閉じない
  CHECK(rs.is_open());
  CHECK(rs.on_eos(1, 3) == EosAction::Closed);
  CHECK(!rs.is_open());
}

// EOS を出していないソースからの Data が来ても、閉じ条件は「期待集合の全 EOS」。
// 一度も Data を出さないソースがあっても run は開く(delila の seen ベースとの差分)。
static void test_run_closes_even_if_a_source_sent_no_data() {
  RunState rs({0, 1});
  rs.on_data(0, 12);
  CHECK(rs.on_eos(1, 12) == EosAction::Pending);
  CHECK(rs.on_eos(0, 12) == EosAction::Closed);
}

// idle 中の EOS は数えて無視(delila-rs の stale EOS 事故 — SPEC §2.2)。
static void test_stale_eos_while_idle_is_counted_and_ignored() {
  RunState rs({kDecoderSourceId});
  CHECK(rs.on_eos(kDecoderSourceId, 7) == EosAction::Ignored);
  CHECK(!rs.is_open());
  CHECK_EQ(rs.stale_eos(), 1);
  // 次の run は普通に開く
  CHECK(rs.on_data(kDecoderSourceId, 8) == DataAction::Opened);
  CHECK_EQ(rs.run_number(), 8);
}

// 期待していない source からの Data は数える(黙って混ぜない — CLAUDE.md)。
static void test_unexpected_source_is_counted() {
  RunState rs({kDecoderSourceId});
  CHECK(rs.on_data(kDecoderSourceId, 7) == DataAction::Opened);
  CHECK(rs.on_data(0, 7) == DataAction::Unexpected);
  CHECK_EQ(rs.unexpected_sources(), 1);
  CHECK(rs.on_eos(0, 7) == EosAction::Ignored);
  CHECK_EQ(rs.unexpected_sources(), 2);
  // 期待ソースの EOS だけで閉じる
  CHECK(rs.on_eos(kDecoderSourceId, 7) == EosAction::Closed);
}

// run 中に run_number が変わるのはプロトコル違反。数えて可視化する(open 中は初回値を保つ)。
static void test_run_number_change_mid_run_is_counted() {
  RunState rs({kDecoderSourceId});
  rs.on_data(kDecoderSourceId, 7);
  CHECK(rs.on_data(kDecoderSourceId, 8) == DataAction::Continued);
  CHECK_EQ(rs.run_number(), 7);
  CHECK_EQ(rs.run_number_mismatch(), 1);
}

static void test_two_runs_in_a_row() {
  RunState rs({kDecoderSourceId});
  CHECK(rs.on_data(kDecoderSourceId, 1) == DataAction::Opened);
  CHECK(rs.on_eos(kDecoderSourceId, 1) == EosAction::Closed);
  CHECK(rs.on_data(kDecoderSourceId, 2) == DataAction::Opened);
  CHECK_EQ(rs.run_number(), 2);
  CHECK(rs.on_eos(kDecoderSourceId, 2) == EosAction::Closed);
  CHECK_EQ(rs.stale_eos(), 0);
}

// ---------------------------------------------------------------------------
// 3. SeqCheck — sequence_number 連続性(SPEC §6.2-5、ギャップ = fatal)
// ---------------------------------------------------------------------------

static void test_first_sequence_number_is_the_baseline() {
  SeqCheck sc;
  // ロスレスリンクでは run 開始 = 0 だが、値そのものを決め打ちせず初回を基準にする。
  CHECK(sc.check(100, 0) == SeqAction::First);
  CHECK(sc.check(100, 1) == SeqAction::Ok);
  CHECK(sc.check(100, 2) == SeqAction::Ok);
}

static void test_gap_is_detected() {
  SeqCheck sc;
  CHECK(sc.check(100, 0) == SeqAction::First);
  CHECK(sc.check(100, 1) == SeqAction::Ok);
  CHECK(sc.check(100, 3) == SeqAction::Gap);  // 2 が消えた = ロスレス契約違反
}

// 巻き戻り・重複も連続性違反(再送やソース再起動の取り違えを黙って通さない)。
static void test_regression_and_duplicate_are_detected() {
  SeqCheck sc;
  sc.check(100, 5);
  CHECK(sc.check(100, 5) == SeqAction::Regression);
  CHECK(sc.check(100, 4) == SeqAction::Regression);
}

static void test_sources_are_tracked_independently() {
  SeqCheck sc;
  CHECK(sc.check(100, 0) == SeqAction::First);
  CHECK(sc.check(200, 41) == SeqAction::First);  // 別ソースは別カウンタ
  CHECK(sc.check(100, 1) == SeqAction::Ok);
  CHECK(sc.check(200, 42) == SeqAction::Ok);
  CHECK(sc.check(100, 2) == SeqAction::Ok);
}

// run 境界で 0 に戻る(SPEC §2.2)。リセットしないと次 run の 0 が Regression に見える。
static void test_reset_clears_the_baselines() {
  SeqCheck sc;
  sc.check(100, 0);
  sc.check(100, 1);
  sc.reset();
  CHECK(sc.check(100, 0) == SeqAction::First);
  CHECK(sc.check(100, 1) == SeqAction::Ok);
}

int main() {
  test_channel_is_fifo();
  test_pop_blocks_until_a_push_arrives();
  test_push_blocks_when_full_and_resumes_after_pop();
  test_close_drains_then_reports_end();
  test_close_wakes_a_blocked_pop();
  test_push_after_close_aborts();

  test_single_source_run_opens_on_data_and_closes_on_eos();
  test_multi_source_run_waits_for_every_expected_eos();
  test_run_closes_even_if_a_source_sent_no_data();
  test_stale_eos_while_idle_is_counted_and_ignored();
  test_unexpected_source_is_counted();
  test_run_number_change_mid_run_is_counted();
  test_two_runs_in_a_row();

  test_first_sequence_number_is_the_baseline();
  test_gap_is_detected();
  test_regression_and_duplicate_are_detected();
  test_sources_are_tracked_independently();
  test_reset_clears_the_baselines();
  return tpccheck::report("test_rs_core");
}
