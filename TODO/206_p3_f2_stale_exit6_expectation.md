# 206 — `p3_error_paths` の F2 が 186 以来落ちている(root_sink の exit 6 を期待する古いテスト)

**Status: TODO**(2026-11-04 起票 Opus。出どころ = 204 の implementer が env ゲートの実走で発見、main 663b00d でも同じく落ちることを確認)

## 何が起きているか
- `tests/p3_error_paths.rs::f1_eos_timeout_leaves_the_run_open_and_f2_the_next_run_hits_a_late_fatal` は env ゲート(`TPCDAQ_ROOT_SINK_BIN` など)のときだけ走る。前半 F1(停止中に receiver を殺す → `error:eos-timeout`)は通るが、**後半 F2 が「次の run の Data で root_sink が exit 6 で死ぬ」ことを期待**していて、`root_sink did not exit within 30s` で落ちる。
- 186(SPEC v1.53)で root_sink の exit 6 の経路は撤去され、今は「別 run_number の Data → 旧 run を inprogress のまま閉じて新 run を開く」(`runs_closed_without_eos` + 1)。テストの期待が古いまま。186 の受け入れでは env ゲートのこのテストを回していなかった(見逃し)。

## 直すこと
- F2 の期待を 186 の挙動へ: 次の run の Data で root_sink は落ちず、旧 run は inprogress・monitor.root 無し・close 行 `closed without EOS (incomplete)`、`runs_closed_without_eos == 1`、新 run は EOS で finalize。テストの名前も意味に合わせて改める(例 `f1_eos_timeout_leaves_the_run_open_and_f2_the_next_run_closes_it_as_incomplete`)。
- 186 の archive の結果節に、env ゲートの F2 を見逃していたことを 1 行追記。
- モデル: Sonnet(テスト 1 本の期待の書き換え)。204 / 188 のマージの後。

## 受け入れ
- env ゲートで `cargo test --test p3_error_paths` が全部 green。
