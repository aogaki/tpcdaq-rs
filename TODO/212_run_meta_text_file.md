# 212 — run ごとの meta ファイル(テキスト): params・HV / LV・ガス(種類と混合比・圧力・流量)

**Status: TODO(Fable キュー: SPEC の改訂 — run/start の API・新しい成果物)**(2026-11-25 起票 Opus。研究者からの要望をユーザーが伝達)

## 要望(ユーザー 2026-11-25)
- graw と同じ run のディレクトリに、HV・ガス圧・ガスの情報を meta ファイルとして残したい(研究者がオフラインで読む)。
- **テキスト**がよい。
- ガス = **種類(混合比込み)・圧力・流量**。どれも **Unknown** を選べるようにする。
- HV は slowctl で既に読めている(SPEC §9.2 v1.31 の `run_start.slow` / `run_stop.slow`)。**問題はガス** — ELITPC はまだガスを読めていない(160: ドライバは実装済み、現地の配線待ち)。

## 主対話の案(Fable が確定する)
1. **ファイル**: `/data/tpcdaq/run{NNNN}/run{NNNN}_meta.txt`。controller が **run 開始時に書き、停止時に追記**する(停止の節を足す)。形式 = 1 行 1 項目の `key = value`、`#` はコメント、節の見出しは `[start]` / `[stop]` / `[gas]` など(TOML の部分集合 = 人にも機械にも読める)。単位は key に含める(`pressure_mbar`、`flow_mln_min`)。
2. **中身**:
   - run: 番号、開始・停止の時刻(EET と UTC)、operator、comment、params id と**ノブの全値**(peaking・sampling・delay・トリガの種類・周期・readout…)、geometry。
   - HV / LV: slowctl の直近値(ch ごとの設定値と読み値、状態)を開始時と停止時。
   - **ガス**: `mixture`(自由文字列、例 `CO2 100%`、`Ar/CO2 90/10`)、`pressure_mbar`、`flow_mln_min`。各値は数値か `unknown`。各項目に `source = manual | auto`、入力した人、時刻。
   - トリガ(N1068GE)の設定 — 205 ができたら。
   - 停止時: run_stop の要約(ok・reason・事象数・graw の数・root_complete・落とし)と、**run 中のログブックの comment**(流量を変えた等、途中の変化はここに残る)。
3. **ガスの入力(run 開始)**: UI の run 開始の画面にガスの 3 項目を出す。**前回の値を入れておき、確認のチェックを必須**にする(毎回打たせない — 慣れて確かめずに通すのを避ける。値を変えたときだけ打つ)。各項目に「Unknown」の選択肢。API: `POST /api/run/start` に `gas: {mixture, pressure_mbar, flow_mln_min}`(各 `string | number | "unknown"`)を足す。無ければ 400(UI 以外の呼び手 = scan.py などにも明示を求める)か、前回の値 + `source = carried`(要決定)。
4. **自動の値(160 の後)**: Bronkhorst から slowctl で圧力・流量が読めるようになったら、手入力より自動の値を優先し(`source = auto`)、開始時と停止時の両方を書く。手入力の欄は混合比だけ残る。
5. ログブックの `run_start` / `run_stop` にも同じガスの節(JSON)を入れる(meta.txt はその写し — 正は 1 つ)。

## 決めること(Fable)
- `gas` が無い run/start を拒むか、前回の値を持ち越すか。
- meta.txt の形式(上の TOML の部分集合でよいか)と、停止時の追記で原子的に書く方法(一時ファイル → rename)。
- SPEC の節: §8.1(run/start)、§9.2(run_start / run_stop の `gas`)、§6.5 か §7(run ディレクトリの成果物に meta.txt)、§11(UI)。

## 関連
- 160(ガスを ELITPC から読む — 現地の配線)、205(N1068GE の設定の記録)、131。
