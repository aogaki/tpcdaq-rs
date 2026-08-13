// test_eb_core.cpp — eb_core.hpp(eventIdx イベントビルダ)と rs_core.hpp の
// SeqCheck 厳格モードの単体テスト(ZMQ 不要・ROOT 不要)。
//
//   g++ -std=c++17 -O2 -Wall -Wextra test_eb_core.cpp -o test_eb_core && ./test_eb_core
//
// 仕様は SPEC §6.3(イベントビルダ)と §2.2(seq は run 開始で 0 リセット)。
// **時刻はすべて引数注入**なので、タイムアウト系も sleep なしで決定的に走る。
//
// **assert は必要**: 期待集合が空の EventBuilder は abort する。死亡テストを含むので
// NDEBUG なしでビルドすること(Makefile がそうしている)。

#include <sys/wait.h>
#include <unistd.h>

#include <csignal>
#include <cstdint>
#include <cstdio>
#include <set>
#include <utility>
#include <vector>

#include "check.hpp"
#include "eb_core.hpp"
#include "rs_core.hpp"

using namespace rootsink;

// ---------------------------------------------------------------------------
// テスト用の合成 fragment
// ---------------------------------------------------------------------------

static void push_u32_le(std::vector<uint8_t>& v, uint32_t w) {
  v.push_back(static_cast<uint8_t>(w & 0xff));
  v.push_back(static_cast<uint8_t>((w >> 8) & 0xff));
  v.push_back(static_cast<uint8_t>((w >> 16) & 0xff));
  v.push_back(static_cast<uint8_t>((w >> 24) & 0xff));
}

// items の 1 語目に (event_idx, cobo, asad) を織り込む。所有コピーの取り違え
// (別 fragment の items を掴む)がそのまま値の不一致として出る。
//   word0 = 0xAB00_0000 | event_idx<<8 | cobo<<4 | asad
static uint32_t marker_word(uint32_t event_idx, uint8_t cobo, uint8_t asad) {
  return 0xAB000000u | (event_idx << 8) | (static_cast<uint32_t>(cobo) << 4) | asad;
}

// 非対称な合成 fragment(全フィールドが違う値 —— 取り違えが見えるように)。
static OwnedFragment frag(uint32_t run, uint32_t event_idx, uint8_t cobo, uint8_t asad) {
  OwnedFragment f;
  f.run_number = run;
  f.event_idx = event_idx;
  f.event_time = 0x0000123456780000ull + event_idx;
  f.cobo = cobo;
  f.asad = asad;
  f.frame_type = 2;
  f.revision = 5;
  f.read_offset = 136;
  f.status = 0;
  f.mult[0] = 68;
  f.mult[1] = 0;
  f.mult[2] = 17;
  f.mult[3] = 3;
  f.window_out = 512;
  f.last_cell[0] = 7;
  f.last_cell[1] = 9;
  f.last_cell[2] = 11;
  f.last_cell[3] = 13;
  push_u32_le(f.items, marker_word(event_idx, cobo, asad));
  push_u32_le(f.items, 0x5A5A0000u | event_idx);
  return f;
}

// ---------------------------------------------------------------------------
// 1. OwnedFragment — FragmentView(元バッファへの参照)からの所有コピー
// ---------------------------------------------------------------------------

// ビルダはバッチの寿命を超えて fragment を持つ。元バッファが書き換わっても
// (= ZmqMsg が破棄されて再利用されても)保持中の値が変わらないこと。
static void test_owned_fragment_deep_copies_the_view() {
  std::vector<uint8_t> buffer;
  push_u32_le(buffer, 0x11223344u);
  push_u32_le(buffer, 0x55667788u);

  tpcwire::FragmentView v;
  v.event_idx = 42;
  v.event_time = 0x0000123456789abcull;
  v.cobo = 1;
  v.asad = 3;
  v.frame_type = 2;
  v.revision = 5;
  v.read_offset = 136;
  v.status = 7;
  v.mult[0] = 68;
  v.mult[1] = 0;
  v.mult[2] = 17;
  v.mult[3] = 3;
  v.window_out = 512;
  v.last_cell[0] = 7;
  v.last_cell[1] = 9;
  v.last_cell[2] = 11;
  v.last_cell[3] = 13;
  v.items = buffer.data();
  v.items_size = buffer.size();

  const OwnedFragment f = OwnedFragment::from_view(v, /*run_number=*/7);
  CHECK_EQ(f.run_number, 7);  // run は Batch ヘッダ由来(Fragment 自身は持たない)
  CHECK_EQ(f.event_idx, 42);
  CHECK_EQ(f.event_time, 0x0000123456789abcull);
  CHECK_EQ(f.cobo, 1);
  CHECK_EQ(f.asad, 3);
  CHECK_EQ(f.frame_type, 2);
  CHECK_EQ(f.revision, 5);
  CHECK_EQ(f.read_offset, 136);
  CHECK_EQ(f.status, 7);
  CHECK_EQ(f.mult[0], 68);
  CHECK_EQ(f.mult[3], 3);
  CHECK_EQ(f.window_out, 512);
  CHECK_EQ(f.last_cell[2], 11);
  CHECK_EQ(f.item_count(), 2);
  CHECK_EQ(f.item(0), 0x11223344u);
  CHECK_EQ(f.item(1), 0x55667788u);
  CHECK(f.key() == FragmentKey(1, 3));

  // 元バッファを壊す —— コピーは無傷であるべき
  buffer.assign(buffer.size(), 0xff);
  CHECK_EQ(f.item(0), 0x11223344u);
  CHECK_EQ(f.item(1), 0x55667788u);
}

// ---------------------------------------------------------------------------
// 2. 完全ビルド(多ソース)
// ---------------------------------------------------------------------------

// 2 ソース(mini ではなく ELITPC 相当の一部)。片方だけでは出さない。
// **到着は cobo1 → cobo0 の逆順**に入れて、出口が (cobo,asad) 昇順であることを見る
// (SPEC v1.3 §6.3)。
static void test_two_sources_build_one_complete_event() {
  EventBuilder eb({{0, 0}, {1, 0}}, 1000);
  CHECK(!eb.feed(frag(7, 0, 1, 0), 100).has_value());
  CHECK_EQ(eb.poll(100).size(), 0);  // 片方だけ = まだ complete ではない
  CHECK_EQ(eb.pending(), 1);

  CHECK(!eb.feed(frag(7, 0, 0, 0), 150).has_value());
  const std::vector<BuiltEvent> out = eb.poll(150);
  CHECK_EQ(out.size(), 1);
  CHECK_EQ(out[0].run_number, 7);
  CHECK_EQ(out[0].event_idx, 0);
  CHECK(!out[0].incomplete);
  CHECK_EQ(out[0].fragments.size(), 2);
  // 到着順(1,0)→(0,0)ではなく **(cobo,asad) 昇順**で並ぶ。
  // items は所有コピーなので、並べ替えても中身が付いて回る。
  CHECK_EQ(out[0].fragments[0].cobo, 0);
  CHECK_EQ(out[0].fragments[1].cobo, 1);
  CHECK_EQ(out[0].fragments[0].item(0), marker_word(0, 0, 0));  // 0xab000000
  CHECK_EQ(out[0].fragments[1].item(0), marker_word(0, 1, 0));  // 0xab000010

  CHECK_EQ(eb.events_complete(), 1);
  CHECK_EQ(eb.events_incomplete(), 0);
  CHECK_EQ(eb.late_fragments(), 0);
  CHECK_EQ(eb.pending(), 0);
}

// SPEC v1.3 §6.3: イベント内のフラグメント順は **(cobo, asad) 昇順で決定的**。
// 到着順は run 毎に揺れるので、そのまま並べると §12-4 の TTree 比較が順序で
// 偽陰性になる。4 ソースを昇順の逆(1:1 → 1:0 → 0:1 → 0:0)で投入して確かめる。
static void test_fragments_inside_an_event_are_sorted_by_cobo_asad() {
  EventBuilder eb({{0, 0}, {0, 1}, {1, 0}, {1, 1}}, 1000);
  eb.feed(frag(7, 4, 1, 1), 0);
  eb.feed(frag(7, 4, 1, 0), 0);
  eb.feed(frag(7, 4, 0, 1), 0);
  eb.feed(frag(7, 4, 0, 0), 0);

  const std::vector<BuiltEvent> out = eb.poll(0);
  CHECK_EQ(out.size(), 1);
  CHECK_EQ(out[0].fragments.size(), 4);
  const FragmentKey want[4] = {{0, 0}, {0, 1}, {1, 0}, {1, 1}};
  for (size_t k = 0; k < 4; ++k) {
    CHECK(out[0].fragments[k].key() == want[k]);
    // 並べ替えで items が入れ替わっていないこと(所有コピーが付いて回る)
    CHECK_EQ(out[0].fragments[k].item(0),
             marker_word(4, want[k].first, want[k].second));
  }
}

// flush 経路(EOS)でも同じ順序規則が効く。incomplete なイベントも整列して出す。
static void test_flush_also_sorts_fragments_by_cobo_asad() {
  EventBuilder eb({{0, 0}, {0, 1}, {1, 0}, {1, 1}}, 1000);
  eb.feed(frag(7, 0, 1, 0), 0);  // 3 個だけ = incomplete のまま
  eb.feed(frag(7, 0, 0, 1), 0);
  eb.feed(frag(7, 0, 1, 1), 0);

  const std::vector<BuiltEvent> out = eb.flush();
  CHECK_EQ(out.size(), 1);
  CHECK(out[0].incomplete);
  CHECK_EQ(out[0].fragments.size(), 3);
  CHECK(out[0].fragments[0].key() == FragmentKey(0, 1));
  CHECK(out[0].fragments[1].key() == FragmentKey(1, 0));
  CHECK(out[0].fragments[2].key() == FragmentKey(1, 1));
}

// ---------------------------------------------------------------------------
// 3. 昇順 emit / 先頭が塞ぐ(SPEC §6.3「emit は event_idx 昇順」)
// ---------------------------------------------------------------------------

// 到着順は e2 → e0 → e1 → e2 → e1 → e0 とばらばら。先頭(最小 event_idx)が
// 揃うまで後続は 1 個も出さず、揃った瞬間に 3 個まとめて昇順で出る。
static void test_shuffled_arrival_emits_in_event_idx_order() {
  EventBuilder eb({{0, 0}, {0, 1}}, 1000);
  eb.feed(frag(7, 2, 0, 0), 10);
  CHECK_EQ(eb.poll(10).size(), 0);
  eb.feed(frag(7, 0, 0, 1), 11);
  CHECK_EQ(eb.poll(11).size(), 0);
  eb.feed(frag(7, 1, 0, 0), 12);
  CHECK_EQ(eb.poll(12).size(), 0);

  // e2 が揃った。しかし先頭は e0(未完)なので **何も出さない**。
  eb.feed(frag(7, 2, 0, 1), 13);
  CHECK_EQ(eb.poll(13).size(), 0);
  CHECK_EQ(eb.events_complete(), 0);
  // e1 も揃った。それでも先頭 e0 が塞いでいる。
  eb.feed(frag(7, 1, 0, 1), 14);
  CHECK_EQ(eb.poll(14).size(), 0);
  CHECK_EQ(eb.pending(), 3);

  // e0 が揃った瞬間に e0,e1,e2 が昇順で出る。
  eb.feed(frag(7, 0, 0, 0), 15);
  const std::vector<BuiltEvent> out = eb.poll(15);
  CHECK_EQ(out.size(), 3);
  CHECK_EQ(out[0].event_idx, 0);
  CHECK_EQ(out[1].event_idx, 1);
  CHECK_EQ(out[2].event_idx, 2);
  CHECK(!out[0].incomplete);
  CHECK(!out[1].incomplete);
  CHECK(!out[2].incomplete);
  CHECK_EQ(eb.events_complete(), 3);
  CHECK_EQ(eb.pending(), 0);
}

// ---------------------------------------------------------------------------
// 4. タイムアウト(時刻注入、sleep なし)
// ---------------------------------------------------------------------------

// 判定は `now - first_arrival > build_timeout_ms`(等しいうちはまだ出さない)。
// 手計算: first_arrival = 1_000_000、timeout = 1000 →
//   now = 1_001_000 は経過 1000 ms = 境界(出さない)
//   now = 1_001_001 は経過 1001 ms > 1000(出す)
static void test_timeout_emits_an_incomplete_event_without_dropping_it() {
  EventBuilder eb({{0, 0}, {0, 1}}, 1000);
  eb.feed(frag(7, 5, 0, 0), 1000000);
  CHECK_EQ(eb.poll(1001000).size(), 0);  // 境界ちょうどでは出さない

  const std::vector<BuiltEvent> out = eb.poll(1001001);
  CHECK_EQ(out.size(), 1);
  CHECK_EQ(out[0].event_idx, 5);
  CHECK(out[0].incomplete);
  CHECK_EQ(out[0].fragments.size(), 1);  // 揃わなかった側は無いが、来た分は捨てない
  CHECK_EQ(out[0].fragments[0].item(0), marker_word(5, 0, 0));
  CHECK_EQ(eb.events_incomplete(), 1);
  CHECK_EQ(eb.events_complete(), 0);
  CHECK_EQ(eb.pending(), 0);
}

// タイムアウトした先頭は後続の門を開ける(先頭が永久に塞ぎ続けない)。
static void test_a_timed_out_head_unblocks_the_events_behind_it() {
  EventBuilder eb({{0, 0}, {0, 1}}, 100);
  eb.feed(frag(7, 0, 0, 0), 1000);  // e0 は片方しか来ない
  eb.feed(frag(7, 1, 0, 0), 1010);
  eb.feed(frag(7, 1, 0, 1), 1020);  // e1 は揃っている
  CHECK_EQ(eb.poll(1050).size(), 0);

  // now=1101: e0 は 101 ms 経過 > 100 → incomplete で出る。続けて e1 が complete で出る。
  const std::vector<BuiltEvent> out = eb.poll(1101);
  CHECK_EQ(out.size(), 2);
  CHECK_EQ(out[0].event_idx, 0);
  CHECK(out[0].incomplete);
  CHECK_EQ(out[1].event_idx, 1);
  CHECK(!out[1].incomplete);
  CHECK_EQ(eb.events_incomplete(), 1);
  CHECK_EQ(eb.events_complete(), 1);
}

// ---------------------------------------------------------------------------
// 5. 遅延到着 = LateFragment(捨てない — SPEC §6.3「順序より可逆性優先」)
// ---------------------------------------------------------------------------

static void test_late_arrival_is_returned_not_dropped() {
  EventBuilder eb({{0, 0}}, 1000);
  eb.feed(frag(7, 3, 0, 0), 10);
  CHECK_EQ(eb.poll(10).size(), 1);  // 単一ソース = 即 complete

  // 同じ (run, event_idx) への到着 = 遅延。呼び手に返す(= 書き出す責任は呼び手)。
  const std::optional<LateFragment> late = eb.feed(frag(7, 3, 0, 0), 20);
  CHECK(late.has_value());
  CHECK(late.has_value() && late->fragment.event_idx == 3);
  CHECK(late.has_value() && late->fragment.item(0) == marker_word(3, 0, 0));
  CHECK(late.has_value() && late->emitted_upto == 3);
  CHECK_EQ(eb.late_fragments(), 1);
  CHECK_EQ(eb.pending(), 0);  // 遅延を pending に化けさせない(復活イベントを作らない)

  // emit 済みより若い event_idx も遅延扱い。
  const std::optional<LateFragment> late2 = eb.feed(frag(7, 1, 0, 0), 30);
  CHECK(late2.has_value());
  CHECK_EQ(eb.late_fragments(), 2);
  CHECK_EQ(eb.pending(), 0);

  // 新しい event_idx は普通に受ける(遅延判定が未来を巻き込まない)。
  CHECK(!eb.feed(frag(7, 4, 0, 0), 40).has_value());
  CHECK_EQ(eb.poll(40).size(), 1);
  CHECK_EQ(eb.late_fragments(), 2);
  CHECK_EQ(eb.events_complete(), 2);  // 出たのは e3 と e4 の 2 個だけ(遅延は数えない)
}

// ---------------------------------------------------------------------------
// 6. flush(EOS 時)
// ---------------------------------------------------------------------------

// 残りを全部昇順で出す。**揃っていたものを incomplete と嘘をつかない**
// (counts の意味が壊れる)。
static void test_flush_emits_everything_left_in_order() {
  EventBuilder eb({{0, 0}, {0, 1}}, 1000);
  eb.feed(frag(7, 0, 0, 0), 0);  // e0 は片方だけ
  eb.feed(frag(7, 1, 0, 0), 0);
  eb.feed(frag(7, 1, 0, 1), 0);  // e1 は揃っている(が e0 が塞いでいる)
  CHECK_EQ(eb.poll(10).size(), 0);
  CHECK_EQ(eb.pending(), 2);

  const std::vector<BuiltEvent> out = eb.flush();
  CHECK_EQ(out.size(), 2);
  CHECK_EQ(out[0].event_idx, 0);
  CHECK(out[0].incomplete);
  CHECK_EQ(out[1].event_idx, 1);
  CHECK(!out[1].incomplete);
  CHECK_EQ(eb.events_incomplete(), 1);
  CHECK_EQ(eb.events_complete(), 1);
  CHECK_EQ(eb.pending(), 0);
  CHECK_EQ(eb.flush().size(), 0);  // 二度目は空(再入で二重計上しない)
}

// flush 後も遅延判定は生きている(EOS の後に来た迷子を pending に戻さない)。
static void test_late_detection_survives_flush() {
  EventBuilder eb({{0, 0}, {0, 1}}, 1000);
  eb.feed(frag(7, 2, 0, 0), 0);
  CHECK_EQ(eb.flush().size(), 1);
  const std::optional<LateFragment> late = eb.feed(frag(7, 2, 0, 1), 5);
  CHECK(late.has_value());
  CHECK_EQ(eb.late_fragments(), 1);
  CHECK_EQ(eb.pending(), 0);
}

// ---------------------------------------------------------------------------
// 7. run_number が別なら別イベント(キーは (run, event_idx))
// ---------------------------------------------------------------------------

static void test_same_event_idx_in_a_different_run_is_a_different_event() {
  EventBuilder eb({{0, 0}, {0, 1}}, 1000);
  eb.feed(frag(7, 0, 0, 0), 0);
  eb.feed(frag(8, 0, 0, 1), 0);  // 同じ event_idx=0 でも run が違う
  CHECK_EQ(eb.pending(), 2);
  CHECK_EQ(eb.poll(0).size(), 0);

  eb.feed(frag(7, 0, 0, 1), 0);  // run 7 が揃った
  const std::vector<BuiltEvent> out = eb.poll(0);
  CHECK_EQ(out.size(), 1);
  CHECK_EQ(out[0].run_number, 7);
  CHECK_EQ(out[0].event_idx, 0);
  CHECK_EQ(eb.pending(), 1);  // run 8 のイベントは残る

  // 遅延判定も run 毎。run 7 の e0 を出した後でも run 8 の e0 は遅延ではない。
  const std::optional<LateFragment> not_late = eb.feed(frag(8, 0, 0, 0), 0);
  CHECK(!not_late.has_value());
  const std::vector<BuiltEvent> out8 = eb.poll(0);
  CHECK_EQ(out8.size(), 1);
  CHECK_EQ(out8[0].run_number, 8);
  CHECK_EQ(eb.late_fragments(), 0);
}

// ---------------------------------------------------------------------------
// 8. 単一 CoBo 構成 = 実質素通し(SPEC §6.3)
// ---------------------------------------------------------------------------

static void test_single_source_is_a_pass_through() {
  EventBuilder eb({{0, 0}}, 1000);
  for (uint32_t e = 0; e < 3; ++e) {
    CHECK(!eb.feed(frag(7, e, 0, 0), e).has_value());
    const std::vector<BuiltEvent> out = eb.poll(e);
    CHECK_EQ(out.size(), 1);  // 1 個入れたら 1 個出る = バッファに溜めない
    CHECK_EQ(out[0].event_idx, e);
    CHECK(!out[0].incomplete);
    CHECK_EQ(out[0].fragments.size(), 1);
  }
  CHECK_EQ(eb.events_complete(), 3);
  CHECK_EQ(eb.events_incomplete(), 0);
  CHECK_EQ(eb.pending(), 0);
}

// ---------------------------------------------------------------------------
// 9. 期待外・重複フラグメント(捨てない + 数える — CLAUDE.md)
// ---------------------------------------------------------------------------

// 期待集合外の (cobo, asad) は complete 判定に数えないが、イベントには載せる。
static void test_unexpected_fragment_is_counted_and_still_emitted() {
  EventBuilder eb({{0, 0}, {0, 1}}, 1000);
  eb.feed(frag(7, 0, 3, 3), 0);  // 設定にない (3,3)
  CHECK_EQ(eb.unexpected_fragments(), 1);
  CHECK_EQ(eb.poll(0).size(), 0);  // 期待集合はまだ埋まっていない

  eb.feed(frag(7, 0, 0, 0), 0);
  eb.feed(frag(7, 0, 0, 1), 0);
  const std::vector<BuiltEvent> out = eb.poll(0);
  CHECK_EQ(out.size(), 1);
  CHECK(!out[0].incomplete);
  CHECK_EQ(out[0].fragments.size(), 3);  // 期待外の 1 個も一緒に出る(ロスレス)
  // 整列は期待集合の内外を問わない —— (3,3) は最後に来る(先頭到着でも)。
  CHECK(out[0].fragments[0].key() == FragmentKey(0, 0));
  CHECK(out[0].fragments[1].key() == FragmentKey(0, 1));
  CHECK(out[0].fragments[2].key() == FragmentKey(3, 3));
}

// 同じ (cobo, asad) が 2 回来ても complete にはしない(片肺での早出しを防ぐ)。
// 同キー同士の並びは**到着順を保つ**(安定ソート —— 重複の前後関係を作り変えない)。
static void test_duplicate_fragment_is_counted_and_still_emitted() {
  EventBuilder eb({{0, 0}, {0, 1}}, 1000);
  OwnedFragment first = frag(7, 0, 0, 0);
  OwnedFragment second = frag(7, 0, 0, 0);
  second.event_time = 0xdead;  // 2 個目だけ見分けが付くようにする
  eb.feed(std::move(first), 0);
  eb.feed(std::move(second), 0);
  CHECK_EQ(eb.duplicate_fragments(), 1);
  CHECK_EQ(eb.poll(0).size(), 0);

  eb.feed(frag(7, 0, 0, 1), 0);
  const std::vector<BuiltEvent> out = eb.poll(0);
  CHECK_EQ(out.size(), 1);
  CHECK_EQ(out[0].fragments.size(), 3);  // 重複も捨てない
  CHECK(out[0].fragments[0].key() == FragmentKey(0, 0));
  CHECK(out[0].fragments[1].key() == FragmentKey(0, 0));
  CHECK(out[0].fragments[2].key() == FragmentKey(0, 1));
  // 手計算: 1 個目の event_time = 0x0000123456780000 + event_idx(0)、2 個目は 0xdead。
  CHECK_EQ(out[0].fragments[0].event_time, 0x0000123456780000ull);
  CHECK_EQ(out[0].fragments[1].event_time, 0xdead);
  CHECK_EQ(eb.events_complete(), 1);
}

// 期待集合が空のビルダは「全イベントが即 complete」という無意味な動作になる。
// 設定ミスを黙って通さない(死亡テスト: 子プロセスで SIGABRT を確認)。
static void test_empty_expected_set_aborts() {
  pid_t pid = fork();
  if (pid == 0) {
    if (std::freopen("/dev/null", "w", stderr) == nullptr) _exit(3);
    EventBuilder eb({}, 1000);
    (void)eb.pending();
    _exit(0);  // 到達したら設定ミスが素通りしている = テスト失敗
  }
  CHECK(pid > 0);
  int status = 0;
  CHECK(waitpid(pid, &status, 0) == pid);
  CHECK(WIFSIGNALED(status) && WTERMSIG(status) == SIGABRT);
}

// ---------------------------------------------------------------------------
// 10. SeqCheck 厳格モード(SPEC §2.2: run 開始で 0 リセット)
// ---------------------------------------------------------------------------

// 厳格モードでは初回 sequence_number は 0 のみ受理。
static void test_strict_seqcheck_accepts_zero_as_the_first() {
  SeqCheck sc(true);
  CHECK(sc.check(kDecoderSourceId, 0) == SeqAction::First);
  CHECK(sc.check(kDecoderSourceId, 1) == SeqAction::Ok);
  CHECK(sc.check(kDecoderSourceId, 2) == SeqAction::Ok);
}

// 初回が 0 でない = 先頭バッチを取りこぼしている。Gap = fatal で可視化する
// (008 レビューの申し送り: 先頭喪失を不可視にしない)。
static void test_strict_seqcheck_treats_a_nonzero_first_as_a_gap() {
  SeqCheck sc(true);
  CHECK(sc.check(kDecoderSourceId, 3) == SeqAction::Gap);
  // Gap を返した後も基準値は進む(呼び手が続行を選んでも状態が壊れない)
  CHECK(sc.check(kDecoderSourceId, 4) == SeqAction::Ok);
}

// 既定(非厳格)は従来どおり「初回を基準にする」。既存の呼び手・テストは無改変。
static void test_default_seqcheck_is_unchanged() {
  SeqCheck sc;
  CHECK(sc.check(kDecoderSourceId, 3) == SeqAction::First);
  CHECK(sc.check(kDecoderSourceId, 4) == SeqAction::Ok);
  SeqCheck explicit_default(false);
  CHECK(explicit_default.check(kDecoderSourceId, 9) == SeqAction::First);
}

// ソース毎に効き、reset(run 境界)の後もまた 0 を要求する。
static void test_strict_seqcheck_applies_per_source_and_after_reset() {
  SeqCheck sc(true);
  CHECK(sc.check(100, 0) == SeqAction::First);
  CHECK(sc.check(200, 7) == SeqAction::Gap);  // 別ソースにも同じ規則
  sc.reset();
  CHECK(sc.check(100, 5) == SeqAction::Gap);  // 次の run も 0 から
  sc.reset();
  CHECK(sc.check(100, 0) == SeqAction::First);
}

int main() {
  test_owned_fragment_deep_copies_the_view();

  test_two_sources_build_one_complete_event();
  test_fragments_inside_an_event_are_sorted_by_cobo_asad();
  test_flush_also_sorts_fragments_by_cobo_asad();
  test_shuffled_arrival_emits_in_event_idx_order();

  test_timeout_emits_an_incomplete_event_without_dropping_it();
  test_a_timed_out_head_unblocks_the_events_behind_it();

  test_late_arrival_is_returned_not_dropped();

  test_flush_emits_everything_left_in_order();
  test_late_detection_survives_flush();

  test_same_event_idx_in_a_different_run_is_a_different_event();
  test_single_source_is_a_pass_through();

  test_unexpected_fragment_is_counted_and_still_emitted();
  test_duplicate_fragment_is_counted_and_still_emitted();
  test_empty_expected_set_aborts();

  test_strict_seqcheck_accepts_zero_as_the_first();
  test_strict_seqcheck_treats_a_nonzero_first_as_a_gap();
  test_default_seqcheck_is_unchanged();
  test_strict_seqcheck_applies_per_source_and_after_reset();
  return tpccheck::report("test_eb_core");
}
