# 120 — Slow control 画面を「機器 × ch を 1 行」の表に + HV の Iset / LV の Vset・Iset を読む

**Status: COMPLETED**(2026-10-12 起票・同日完了 Opus。ユーザー「device ch vmon vset imon iset status ON/OFF みたいにしてください。今は同じチャンネルが縦に伸びているのでわかりづらい」。Sonnet 2 レーン)

## 今
- `ui/src/app/slow/slow-view.html` の Latest 表は、**1 行 = 1 サンプル**(device / node / param / value / unit / age / alarm)。
  HV は 1 ch あたり 4 行(vmon / imon / status / vset)が縦に並ぶ。status は数値のビットマスク(例 33)のまま。
- slowctl が出す param(SPEC §2.3 kind=`slow`、wire は変えない):
  - hivolta(`device = "hv"`、node = ch 0–7): `vmon` V / `imon` µA / `status` bitmask / `vset` V。node なし: `bdalarm` bitmask / `bdilk` flag。
  - hmp2020(`device = "lv"`、node = ch 1–2): `volt` V / `curr` A / `outp` on(0/1)。
  - **Iset(HV)と Vset・Iset(LV)は読んでいない**。

## レーン A(Rust、Sonnet): 読み出しの追加(読み出し専用の問い合わせだけ。設定コマンドは作らない)
1. hivolta: `ChPar::Iset` の全 ch 一括 `$CMD:MON,CH:8,PAR:ISET` を `poll` に足す(**7 往復/周期**)→ ch ごとに param `iset`、unit `µA`。
   並びは ch ごとに vmon, imon, status, vset, **iset**。alarm の立て方は既存の 4 つと同じ(その ch の STATUS から)。
   応答の値の数 ≠ nch は Protocol エラー(既存の `bulk` ヘルパ)。`ChPar::Iset` のトークンはコーデックに既にあるはず(無ければマニュアル p.22 の表から)。
2. hmp2020: コーデック `hmp.rs` に読み出しの問い合わせ `volt_query() -> "VOLT?"` と `curr_query() -> "CURR?"` を足す。
   HMP SCPI マニュアル p.23 `[SOURce:]VOLTage…? → VOLT?, Response: 10.000`、p.24 `CURR?, Response: 5.0000` = **設定値の読み出し**。
   `poll` で ch ごとに(既存の INST:NSEL の後で)読み、param `vset`(V)と `iset`(A)として出す。並びは ch ごとに volt, curr, outp, **vset, iset**。
   - `VOLT <値>` / `CURR <値>` / `OUTP ON` 等の**設定コマンドを作る関数は作らない**(SPEC §11、085 と同じ安全境界)。
3. テスト: hivolta は「poll の送信行がちょうど 7 本」に更新し、ISET の応答から `iset` サンプルが ch ごとに出ること。
   hmp は fake で VOLT? / CURR? に応答させ、`vset` / `iset` が ch1 / ch2 に出ること。既存は green のまま。
4. ゲート: `cargo fmt && cargo clippy --tests -- -D warnings && cargo clippy --tests --features slowctl-fake -- -D warnings && cargo test`。

## レーン B(UI、Sonnet): Latest 表の作り替え
1. 純関数(例 `slow/slow-table.ts`)で、最新サンプルの集合 → 表の行に変換する:
   - **ch の行**(node が数値のサンプル): `(device, node)` ごとに 1 行。列は次のとおり。

     | 列 | 中身 |
     |---|---|
     | **Device** | device 名 |
     | **Ch** | node |
     | **Vmon** | `vmon` または `volt` |
     | **Vset** | `vset` |
     | **Imon** | `imon` または `curr` |
     | **Iset** | `iset` |
     | **Status** | 下の規則で文字にする |
     | **ON/OFF** | 下の規則 |
     | **Age** | その行のサンプルで一番古いもの(stale 判定は今と同じ) |

   - 値は今の `formatValue` と同じ書式で、単位を小さく添える(HV は µA、LV は A なので、単位はセルごとに持つ)。無い値は `—`。
   - **Status**: param `status` かつ unit `bitmask` は DT1415ET の STATUS ビットとして文字にする(複数あれば空白区切り。値 0 は `OFF`)。
     ビット割り当ては 0 ON, 1 UP, 2 DOWN, 3 OVC, 4 OVV, 5 UNV, 6 TRIP, 7 OVP, 8 TWN, 9 OVT, 10 KILL, 11 INTLK, 12 DIS, 13 FAIL, 14 LOCK
     (`src/slowctl/hivolta.rs` の `ChStatus` と同じ)。例: 33 → `ON UNV`、1 → `ON`。`status` が無い機器(LV)は `—`。
   - **ON/OFF**: `outp`(1 → ON / 0 → OFF)、無ければ `status` のビット 0。どちらも無ければ `—`。ON は緑、OFF は灰色。
   - 行の alarm: その行のサンプルに 1 つでも alarm があれば、今の `.alarm` の行強調と ALARM バッジ。
   - 並び: device 名 → ch の昇順。
   - **それ以外**(node が null のサンプル = `bdalarm` / `bdilk` など、上の列に写らない param、ガス・環境など別種の機器): **消さない**。
     下に今の形式の小さな表(device / node / param / value / unit / age / alarm)として残す(見出し例 `Other`)。
2. Trend と Counters は変えない。
3. テスト(vitest、既存 spec の流儀): 変換関数の単体。hv ch0–7(ch0 は status 33)+ lv ch1–2(outp 1 / 0)+ bdalarm / bdilk の入力で、
   - 行数 = 10
   - ch0 の Status が `ON UNV` で alarm
   - lv ch2 が OFF
   - lv の Vmon が `volt` から来ること
   - Iset が無い入力では `—`
   - bdalarm / bdilk が Other に出ること
   
   ほかに component の描画 spec を 1 本。既存 spec は green のまま。
4. ゲート: `cd ui && npm test -- --watch=false && npm run build`。

## 結果(2026-10-12)
- レーン A(Sonnet): hivolta は `2865d78`、hmp は `cd73c50`。merge は `51a9a17` / `5b77435`。
  - **1 回目の報告は hmp 部分が誤りだった**: 編集スクリプトが実行を拒否されていたのに、実行済みと思い込んで報告していた。
    Opus がコミットの stat と実機のログブックで気づき、差し戻した。教訓: サブエージェントの報告は `git show --stat` と実機で確かめる。
  - hivolta: 全 ch 一括 ISET を足して 7 往復/周期。param `iset` µA。実機の応答 `#CMD:OK,VAL:0015.00;…`(8 値)をゴールデンにした。
  - hmp: 問い合わせ `VOLT?` / `CURR?`(SCPI マニュアル p.23–24)を足し、param `vset` V / `iset` A。設定コマンドを作る関数は無い。
- レーン B(Sonnet `0162d77` → merge `de989c1`): `ui/src/app/slow/slow-table.ts`(`buildSlowTable` / `formatStatusBits`)。
  - Latest を「機器 × ch が 1 行」の表にした(Vmon ← vmon|volt、Imon ← imon|curr、Status は DT1415ET のビット名、ON/OFF は outp かビット 0)。
  - 写らないサンプルは Other 表に残す。新規テストは 14 本(変換 12 + 描画 1 + ビット 2 — 内訳は spec)。
- ゲート(main `5b77435`、Mac): fmt OK / clippy 0(feature 有無とも)/ **cargo test 904 passed / 0 failed / 1 ignored**(既存)。UI **324 passed / 5 skipped**、build OK。
- 実機(ELITPC、run 69 実行中、12:23 UI 差し替え・slowctl 入れ替え、12:28 HMP 版):
  - hv ch 0–7: iset 15.0 µA、vset 350 / 250 / 350 / 250 / 350 / 250 / 600 / 600 V。
  - lv: ch1 は volt 3.6 / curr 1.235 / outp 1 / **vset 3.6 V / iset 2.5 A**、ch2 は outp 0 / **vset 3.5 V / iset 1.253 A**。
- **運用の知見(ユーザー)**: LV ch1(AsAd)の電流は、**Configure 後で ≈ 1.2 A が普通**。1 A 未満は GET の Load HW 前後の値。
  今日の記録も 11:11(Idle、Configure 前)0.997 A → run 中 1.234 A。LV の電流で AsAd の状態が読める。
- UI の目視確認はユーザー待ち。
