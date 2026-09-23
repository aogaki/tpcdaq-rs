# 087 — vcobo `loop = true` で eventIdx / eventTime を付け替える(2 周目以降がビルダに捨てられる)

**Status: COMPLETED**(2026-09-22 起票 Fable。Sonnet 1 日、C++ tools/vcobo)

## 0. 事実(077 目視 第 2 弾で判明)
- `tools/vcobo/vcobo_link.hpp` のループは `cursor_ = 0` に戻して**同じバイト列**を再送する。eventIdx が 1 周目と同じなので root-sink の
  イベントビルダは 2 周目以降を **late fragments** として捨てる(demo 実測: events_built 108 / late_fragments 8866、frames 8974)。
  → monitor の 0x02/0x03 は 1 周目の 108 イベントで止まり、後から繋いだクライアントには何も届かない。demo の「ずっと流れる」用途が成立していない。

## 1. 発注
- ループで先頭に戻るたびに **eventIdx オフセット**(= セット内の最大 eventIdx + 1)× 周回数を、送出する各データフレームの共通 MFM ヘッダ
  eventIdx(offset 22..26、エンディアンはフレームの `little` フラグに従う — `vcobo_core.hpp::peek_event_idx` と同じ読み方)に加算して送る。
  eventTime(offset 16..22、6 バイト)にもセットの時間幅(最大 − 最小 eventTime + 1 フレーム分)× 周回数を加算して単調増加を保つ。
  制御フレーム(eventIdx を持たない)は無改変。
- 実装は `vcobo_core.hpp` に純関数 `patch_frame_header(uint8_t* frame, size_t len, uint32_t idx_offset, uint64_t time_offset)`(戻り値 bool、
  制御フレームは false)を置き、`vcobo_link.hpp` のホットパスは pending_ にコピーした**後に**そのコピーへ適用する(元バッファは不変、heap 確保なし)。
  `loop = false` のときは呼ばない(1 周目はバイト一致のまま = 既存テストが証拠)。
- テスト(`test_vcobo_core.cpp`): ①合成フレーム(little / big 両方)で eventIdx / eventTime の加算値と他バイトの不変を検査 ②制御フレームは無改変
  ③2 周分の送出列で eventIdx が単調増加し重複が無いこと(既存のループテストがあれば拡張)。`make -j` で green、件数を結果節に。
- 受け入れ(demo、**Fable がマージ後に実施 — 実装レーンは demo スタックを立てない**(ユーザーのスタックとポートが衝突する)): `TPCDAQ_DEMO_LOOP=true` で run を 3 分回し、`/api/status` の events_built が周回に応じて増え、late_fragments が 0 のまま。
  monitor に後から繋いだ WS クライアントに 0x02/0x03 が届く(`reference/_spike/demo/ws_probe.py --seconds 20` で 0x02 が数えられる)。
- 注意: graw_writer が書く .graw も付け替え後のヘッダになる(demo 用途なので可。docs/VIRTUAL_ZCOBO_ja.md に 1 行追記)。

## 2. 参照
077 結果 第 2 弾、docs/VIRTUAL_ZCOBO_ja.md、SPEC §6.3(ビルダは順序に依存しない / late fragment)、tools/vcobo/vcobo_core.hpp(peek_event_idx)。

## 結果(2026-09-23、Sonnet レーン)

**実装**: `tools/vcobo/vcobo_core.hpp` に純関数 `write_header_uint`(`read_header_uint` の逆)と
`patch_frame_header(frame, len, idx_offset, time_offset)` を追加(ゲートは `peek_event_idx` と同一 —
28B 未満 / frameType ∉ {1,2} は無改変・false)。`tools/vcobo/vcobo_link.hpp`:
`GrawSet` に `max_event_idx` / `has_data_frame` を追加(`load_graw_set` が既存の eventIdx スキャンで
そのまま計算、二度手間なし)。`DataLink` に `idx_offset_per_lap_`(`start()` で 1 回計算 =
`has_data_frame ? max_event_idx+1 : 0`)と `lap_`(`daqStart` で 0 に戻る。`worker()` が
`cursor_` を先頭へ戻すたびに `++lap_`)を追加。ホットパスでは `pending_` へコピーした**後**の
領域に対してのみ `patch_frame_header(..., idx_offset_per_lap_ * lap_, /*time_offset=*/0)` を呼ぶ
(`loop_ && lap_for_frame != 0` の時だけ — `loop=false` および 1 周目は一切呼ばないので既存の
バイト一致 CI(`run_vcobo_ci.sh`)は無改造で green)。`span` と同様に `lap_` もロック内でスナップ
ショットしてからロック外(ホットパス)で使う(データ競合なし)。heap 確保・ログ整形は追加していない。

**ビルド・テストコマンド**:
```
make -C tools/vcobo test
make -C tools/vcobo -j8 TPCDAQ_ICE_DIR=/Users/aogaki/Workspace/tpcdaq-rs/reference/20190315_patched
make -C tools/vcobo ci TPCDAQ_ICE_DIR=/Users/aogaki/Workspace/tpcdaq-rs/reference/20190315_patched
```

**テスト数**: `test_vcobo_core` 148 passed → **188 passed, 0 failed**(新規 4 関数、+40 CHECK)。
新規テスト名:
- `test_patch_frame_header_adds_offsets_and_preserves_other_bytes`(①: little/big 両方、
  eventIdx 100+250=350・eventTime 1,000,000+2,500,000=3,500,000 の手計算、[16,26) 外は
  `std::equal` で丸ごと不変照合)
- `test_patch_frame_header_leaves_control_frames_untouched`(②: frameType 7 と 28B 未満の
  2 パターン、バイト列を丸ごと `==` 照合)
- `test_loop_renumbering_is_monotonic_and_duplicate_free`(③: 非対称セット eventIdx {2,5,9}
  を 2 周、idx_offset_per_lap=10 で `{2,5,9,12,15,19}` — 全体で単調増加・重複なしを
  `std::adjacent_find` で確認)
- `test_patch_frame_header_is_a_noop_with_zero_offsets`(offset 0/0 はバイト不変 —
  `loop_ && lap_for_frame != 0` ゲートの根拠)

既存 148 テストは無改造のまま green。`make -j8`(Ice ビルド、`vcobo_daq`/`vcobo_ice_client`)は
警告ゼロで通過。`make ci`(`run_vcobo_ci.sh`)も green(`vcobo_ice_client(full): 57 passed`,
`vcobo_ice_client(heartbeat): 6 passed`、バイト一致チェック含め全 PASS)。

**承認が要る逸脱(発注書からの逸脱)**: eventTime(offset 16..22)の周回オフセットは
**未配線**(`vcobo_link.hpp` の呼び出し側で `time_offset=0` 固定)。発注書 §1 は
「セットの時間幅(最大−最小 eventTime)+ 1 フレーム分」× 周回数を加算せよと書いているが、
「1 フレーム分」の定量化(平均イベント間隔から逆算するのか / 固定 tick 幅にするのか /
1 フレームあたりの eventIdx 増分 1 と同じ意味で「+1」なのか)が発注書だけでは一意に決まらず、
CLAUDE.md の「設計判断はしない」原則に従い実装せず報告する(呼び出し元の指示でも名指しで
同じ懸念が挙がっていた)。`patch_frame_header` 自体は `time_offset` を汎用的に受け取り、
①のテストで(任意の合成オフセット値により)加算ロジックそのものは検証済み — 残るのは
「ループ 1 周ごとに実際いくつ足すか」という運用ポリシーの決定のみ。root-sink のイベント
ビルダは eventIdx のみで late fragment を判定し eventTime には依存しない
(TODO/archive/031_load_harness.md)ため、077 で観測された late_fragments 化はこの
eventIdx 側の付け替えだけで解消する(eventTime 未配線は demo の「ずっと流れる」用途の
成立を妨げない)。docs/VIRTUAL_ZCOBO_ja.md §5.2 にもこの未実装を明記した。

**demo 受け入れ未実施**: 発注書の指示どおり、実装レーンは demo スタックを立てていない
(ユーザーのスタックとポートが衝突するため)。`TPCDAQ_DEMO_LOOP=true` での 3 分run・
`/api/status` の events_built 増加・late_fragments=0・後から繋いだ WS クライアントへの
0x02/0x03 到達(`ws_probe.py`)は **Fable がマージ後に実施**。

**実行環境**: macOS(Darwin 25.6.0)、g++ (`c++ -std=c++17 -O2`)、Ice 3.7 keg
(`/opt/homebrew/opt/ice@3.7`)、`TPCDAQ_ICE_DIR=reference/20190315_patched`、2026-09-23。

**スキップしたテスト**: `run_vcobo_oracle.sh`(実 ECC 相手)は `TPCDAQ_SPIKE_PREFIX` 未設定のため
未実行(元々オンデマンド、本チケットの変更範囲外)。`run_vcobo_ci.sh` 内の実データ回帰は
`TPCDAQ_REAL_GRAW_DIR` 未設定でスキップ(既存の任意回帰、本チケット無関係)。

### 追補(2026-09-23、Fable 裁定反映 — eventTime 配線)

上記「承認が要る逸脱」を Fable がレビューし裁定。以下の式で eventTime も配線した(追加コミット、
同 worktree):

```
time_offset_per_lap = (max_event_time − min_event_time) + step
step = (max_event_time − min_event_time) / (n_data_frames − 1)   ※整数除算・データフレーム 2 枚以上のとき
n_data_frames <= 1 のときは step = 0 → time_offset_per_lap = 0
```

**実装**: `tools/vcobo/vcobo_link.hpp` の `GrawSet` に `min_event_time` / `max_event_time` /
`n_data_frames` を追加(`load_graw_set` の既存 eventIdx スキャンへ相乗り — `peek_event_idx` が
true を返した各データフレームで `read_header_uint(fp+16, 6, little)` も同時に読む。二度手間なし)。
`DataLink` に `time_offset_per_lap_`(`start()` で上式により 1 回計算)を追加。ホットパスの
`patch_frame_header` 呼び出しは `time_offset=0` 固定をやめ、
`time_offset_per_lap_ * uint64_t(lap_for_frame)` を渡すよう変更(`idx_offset` と対称)。
48-bit 桁あふれは考慮しない(裁定どおり、demo 用途)。

**テスト**: `test_loop_renumbering_is_monotonic_and_duplicate_free` を eventTime にも拡張
(既存の eventIdx {2,5,9} のフレームに eventTime {1000,2000,4000} を持たせ、
span=4000−1000=3000・step=3000/(3−1)=1500・time_offset_per_lap=3000+1500=4500 を手計算、
2 周目は eventTime {5500,6500,8500} になることと、6 個を通して eventTime も単調増加する
ことを確認)。新規 1 本追加: `test_time_offset_per_lap_is_zero_with_at_most_one_data_frame`
(データフレーム 1 枚のセットは time_offset_per_lap=0 → eventTime 無改変、eventIdx は
通常どおり付け替わる、を確認)。

**ビルド・テストコマンド**(前節と同一):
```
make -C tools/vcobo test
make -C tools/vcobo -j8 TPCDAQ_ICE_DIR=/Users/aogaki/Workspace/tpcdaq-rs/reference/20190315_patched
make -C tools/vcobo ci TPCDAQ_ICE_DIR=/Users/aogaki/Workspace/tpcdaq-rs/reference/20190315_patched
```

**テスト数**: `test_vcobo_core` **188 passed → 204 passed, 0 failed**(新規 1 関数 + 既存 1 関数の
拡張、+16 CHECK)。`make -j8`(Ice ビルド)は警告ゼロで通過。`make ci` も green
(`vcobo_ice_client(full): 57 passed`, `vcobo_ice_client(heartbeat): 6 passed`、バイト一致
チェック含め全 PASS、`loop=false` パスは無改造のまま)。

**docs**: `docs/VIRTUAL_ZCOBO_ja.md` §5.2 の「eventTime は未実装」を上式へ書き換え。

**残る逸脱・懸念**: なし(前節の懸念は本追補で解消)。demo 受け入れは前節のとおり未実施のまま
(引き続き Fable がマージ後に実施)。

**実行環境**: 前節と同一(macOS Darwin 25.6.0、Ice 3.7 keg、2026-09-23)。

### demo 受け入れ(Fable、2026-09-23、main `5f5b4ea` の vcobo をビルドして実施)
- `TPCDAQ_DEMO_LOOP=true start_demo.sh` → `run_once.sh 900`。後接続の WS クライアント(`ws_tally.py`、15 s)を run 開始 45 s 後と 2 分後に 2 回:
  - A: 0x02 **307** / 0x03 **102** / status 15、`events_built 3977、late_fragments 0、events_incomplete 0、monitorGaps 0`
  - B: 0x02 **301** / 0x03 **100**、`events_built 5134、late_fragments 0`(1 周 108 イベントを何十周も越えて増え続ける)
- 修正前(09-22、同じデータ)は `events_built 108 / late_fragments 8866` で後接続クライアントには 0x02/0x03 が 0 通だった(077 結果 第 2 弾)。→ **受け入れ ①②③ 合格**。
- 副作用: 1 周目はバイト一致のまま(`loop = false` 経路は無改変、vcobo ci のバイト一致チェック green)。

