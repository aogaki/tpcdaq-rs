# 209 — ディスクが詰まった graw-writer は GetStatus にも答えず、204 の「graw を時計で切らない」守りが効かない

**Status: TODO(ユーザー判断: 11/9 の前にやるか)**(2026-11-04 起票 Opus。出どころ = 188 の Fable フェーズ境界レビュー 指摘 2)

## 何が起きるか
- graw-writer の `respond()`(GetStatus など、`src/graw_writer.rs:1073-1087`)と `latch_error()`(`:976-995`)は RunWriter の Mutex を **blocking の `lock()`** で取る。書き込みスレッドは `handle_batch` の中(= `write(2)` の中)でそのロックを握る。
- → **実ディスクが詰まっている間(ZFS の sync のスパイク、満杯の手前など)、graw-writer は GetStatus に答えない = controller からは「不達」**に見える。
- 204 のロスレスの守り(SPEC v1.54 §1.3 停止 2)は「graw-writer が**到達可能** ∧ `files_open > 0` ∧ Error でない ∧ …」の間だけ静止でも打ち切らない。不達なら守りは Off → 停止は静止 ≥ `eos_timeout`(5 s)で切れ、Reset が receiver の graw リンクの送出待ち(PUSH HWM 1,000 + 内部キュー)を `messages_abandoned` に数えて捨てる = **graw の喪失(数えてはいる)**。
- ユーザー裁定(2026-11-04、204 B-4)は「ディスクが固まったときは graw-writer を手で落とすまで停止が終わらない(= 待つ)」だった。今の作りではその場面で待たない。SIGSTOP の試験(188 §3)も同じ形(不達)になる。

## 直し方の案(Fable)
- graw-writer の GetStatus を `try_lock` にし、取れないときは「直近に書いた状態」(state Running、files_open、frames の最新値をアトミックか別の小さな Mutex に複写)を返す → 詰まり中も「到達可能・files_open > 0・Running」に見え、守りが Holds になる。永久待ちの出口 = 運転員が graw-writer を落とす(RUNBOOK §5「停止が終わらない」と整合)。
- 188 の監視から見ても、詰まり中の graw-writer は「到達可能で Running」になり、receiver の溢れ(Error)が先に引き金を引く(今と同じ)。

## 決めること
- 11/9 の前に入れるか(graw_writer は保存系の要。テスト = 書き込みスレッドを人工的に止めた graw-writer に GetStatus → 即応答・files_open > 0、204 の停止が Holds のまま、詰まりが解けたら閉じる)。モデル = Opus。
