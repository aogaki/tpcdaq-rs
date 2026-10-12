# 115 — 歩き戻しで閉じた前の run の `run_stop` をログブックに書く

**Status: IN PROGRESS**(2026-10-12 起票 Opus、仕様は 114 の一括レビューでの Fable 裁定。Opus implementer)

## 事実
- 実機の run 58 / 60(114 の crash 試験)は、controller が run 中に落ちた run。ログブック `logbook.jsonl` には `run_start` しか無く、`run_stop` が無い。
  歩き戻しはデータを EOS で閉じるが、台帳には何も書かない。
- `run_stop` は唯一の run 台帳(033-A、§9.2)。runs.yaml 変換(082)では、この run は「`no run_stop` 注記 + TS glob 後退」になる。
  run_id / files は拾えるが、counters と `eos_closed` の事実が残らない。

## 裁定(Fable、2026-10-12)
- **書く**。内容は次のとおり。
  - `reason: "abort:controller-restart"`、`ok: false`。
  - `forced_eos` / `eos_closed` は歩き戻しの結果。
  - `counters` / `files` は停止経路の第 3 段と同じく、**Stop の後・Reset の前**に `collect_status` で採る(graw-writer は Reset でファイル実績を手放すため)。
  - 一部だけ Running の経路でも `eos_closed: false` で書く(閉じ損ねた事実を台帳に残す)。
- **run 番号の出典 = コンポーネントの `GetStatus` 応答の `run_number`**(`CommandResponse.run_number`。歩き戻しの状態調べで手元にある)。
  - `Some` を返した全員が一致 → それを使う。
  - 一致しない → `warn!` を出し、audit の note に残し、`run_stop` は書かない(推測で台帳を汚さない)。
  - 状態ファイルは `next_run` しか持たず、`run/next` で手動変更され得るので、`next_run - 1` は使わない。
  - ログブックの最後の `run_start` は照合だけに使う(一致しなければ note)。
- `duration_s`: ログブックの同じ番号の `run_start.ts` から算出する。見つからなければ null(§9.2 の `duration_s` は、この経路に限り nullable)。
- 書く場所と順序: `post_run_start` が `RunStart`(新しい run)を書く**前**に、`RunStop`(前の run)を書く。
  歩き戻しの後に start が失敗した場合(Configure 拒否など)も、前の run は閉じているので書く(`StartFailure` 側に、閉じた run の情報を載せる。形は implementer に委ねる)。
- SPEC: §9.2 に reason `abort:controller-restart` と `duration_s` nullable、§1.3 の歩き戻し段落に「前の run の run_stop を書く」1 文。v1.40(Opus が文案、Fable が確定)。

## テスト
- 単体: 全員 Running(`run_number` 一致)→ `RunStop` が `RunStart` より前に、reason / ok / eos_closed / counters 付きで出る。
  run_number が食い違う → `run_stop` は書かれず、note に残る。全員 Idle → `run_stop` なし。
- 統合: 114 の `a_restarted_controller_closes_the_running_run_with_eos_before_the_next_start` を延長し、ログブックの JSONL が
  `run_start 1 → run_stop 1(abort:controller-restart)→ run_start 2 → run_stop 2` の順になること。
- runs.yaml: `logbook2runs` に通すと、run 1 に `no run_stop` 注記が出ないこと。
