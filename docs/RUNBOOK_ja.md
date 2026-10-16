# mini eTPC 運転手順書(RUNBOOK)— v0.2 骨子(2026-09-23: Parameters / Slow control を追記)

**位置づけ**: TODO/080 B-5。現場のオペレータが読む。**正本は SPEC ではなく現物** — 手順が現物と違ったら本書を直す。
出典: SURVEY §4(ログブック 5 年分の運用知見)、072 §4(チューニングの実務ループ)、069(実機テスト計画)、079/080。
`[TODO]` は未確定。空欄のまま実験に入らない。

## 0. 連絡先・場所
- DAQ 機: `[TODO ホスト名 / IP / 置き場]`、zCoBo: `[TODO 4.81 or 4.84]`、UI: `http://<DAQ>:8080/`。
- 担当: DAQ `[TODO]`、ガス `[TODO]`、HV `[TODO]`、シフトリーダー `[TODO]`。

## 1. 電源投入の順序(mini)
1. LV(R&S HMP2020、AsAd 電源)ON → 電圧 `[TODO 定格]`、電流 `[TODO 平常値]` を目視。
2. zCoBo(ZC706)電源 ON → 約 `[TODO]` 秒で getHwServer 常駐(`ping 192.168.4.8x`)。
3. ガス: 流通運転(Bronkhorst MFC → チャンバー → BPC)。圧が設定値(`[TODO mbar]`)で安定するまで待つ。
4. HV(HiVolta):`[TODO 手順。ランプ設定、TRIP 閾値]`。**設定はローカル(前面パネル / ベンダー GUI)— DAQ からは読むだけ**。
5. DAQ スタック起動: `daq_up.sh`(root_sink → monitor → graw_writer → decoder → receiver → ECC → bridge → controller)。
   `curl http://localhost:8080/api/status` で全 alive。
- 電源事情の教訓(SURVEY §4): NIM クレート投入が圧力計に干渉した事例あり。**NIM は圧力が落ち着いてから**。

## 2. 停止の順序
逆順: run 停止(UI)→ `daq_down.sh` → HV OFF(ランプダウン)→ ガス(実験ごとの規則 `[TODO]`)→ zCoBo → LV。

## 3. ラン手順(標準セット)
| 順 | ラン種別 | config_id / プリセット | 確認するもの |
|---|---|---|---|
| 1 | pedestal(内部 periodically) | `pedestal`(`[TODO 実 id]`) | ch 毎 baseline/noise → `calib pedestal`(081) |
| 2 | pulser(AsAd 内蔵、必要なら振幅ランプ) | `pulser` | 相対ゲイン → `calib pulser`(081)、FPN 飽和は正常 |
| 3 | physics(外部トリガ) | `physics-<条件>` | 下記 §4 の目視 |
- **run 開始前チェック(072 §4 のループ)**: ①イベント表示でトラックが窓の端で切れていない ②トリガ前ベースラインが
  数十セル残っている(delay の配分 — 080 §0 の式: 帯位置 = (窓 − delay − L)/40 ns)③圧・HV・流量が記録されている。
- ラン番号は controller が振る。**既存 run の上書きは設計上できない**(p.204 の事故対策)。
- 条件を変えたら logbook にコメント(UI の Logbook タブ)。**何を変えたかを書かない run は解析で捨てられる**。

### 3.1 パラメータ(UI の Parameters タブ、SPEC §3.3 — 078)
- xcfg を手で編集しない。**Parameters タブでノブを変えて Save** すると `configure-<id>.xcfg` / `prepare-<id>.xcfg` が生成され、
  次の run/start がその `<id>` で prepare / configure する(describe は変わらない)。`<id>` は logbook の `run_start.params` と audit に残る。
- 常用ノブ = `triggerDelay`(10 ns 単位)。実験切替級 = `writingClockFrequency_MHz` / `gain` / `peakingTime_ns`、ラン種別 = `triggerMode` / `readout`、
  較正 = `pulser.*` / `globalThreshold` / `lsbThreshold`。**`readout = partial` にすると frameType が 1 に変わる**(解析側に周知)。
- 特定 ch だけ変える(ノイズ ch の閾値など)は **Exceptions** に行を足す。件数は status バーの `exceptions N` に常に出る —
  **N ≠ 0 のまま physics run に入るときは logbook にその理由を書く**。
- Save は操作権 token が要る(Run control で acquire)。Running 中は保存できない(次の run の前に変える)。
- **注意**: 保存済みの params(`current.json`)がある限り、**TOML の config_id より params が優先**される(ラン種別を TOML で切り替えても効かない)。run 種別を変えるときは Parameters を変えるか、status バーの `params <id>` が意図した template か確認する(2026-09-23 に pulser 設定のまま「physics」run を始めた事故)。
- テンプレートは ECC の config repo(`ecc_config_dir`)にある正典 xcfg。Warsaw から新しい xcfg が来たら repo に置き、Parameters の template 名を変えて Save し直す(JSON は変えない)。

## 4. 監視すべきもの(UI)
- status: receiver `overflow_frames` / `extra_connections`、decoder `malformed`、graw-writer `write_errors`、root-sink
  `events_incomplete` — **全部 0 が平常**。増えたら §5。
- 窓端クリップ率(B-4 実装後)。
- **Slow control タブ(079 / 089)**: ガス圧・流量(Bronkhorst)、温湿度(Arduino)、HV の VMON / IMON / STATUS(HiVolta)、AsAd LV(HMP2020)の
  最新値とトレンド(30 min)。status バーの `slow` = ALARM 中の項目数、**0 が平常**。`—` は slowctl が動いていないか monitor の `slow_sub` 未設定。
  `counters` の `poll_err` / `identity_mismatch` が増え続けるなら配線・by-id の取り違え(§5)。**このタブから機器は動かせない**(SPEC §11 — 作動は機器側 GUI)。
- ALARM は DAQ を止めない(既定)。logbook に `slow` の ALARM / RECOVER が自動で残る。
  **定期値はログブックに書かない**(run の開始・停止時の値だけ `run_start` / `run_stop` に載る)。全サンプルは InfluxDB(`http://<host>:8086`、Data Explorer、bucket `slowctl`)— 122。
  表は機器 × ch(Vmon / Vset / Imon / Iset / Status / ON/OFF)。HV ch0 の UNV は「ほかの ch に引っ張られる」既知の挙動なので alarm から外してある(Status 欄には `ON UNV` が出る、121)。
- **検出器まわりに機器(USB / 電源 / 計測器)を繋いだら、繋ぐ前後でトリガレートを比べる**。不良 USB ケーブル 1 本で、HV 電源のポートを開くだけで偽トリガが 24–54 Hz 出た(2026-10-12、118)。
- レートスパイクはビーム由来が大半(SURVEY §4)。制御室に連絡。

## 5. 復旧
| 症状 | 手当 |
|---|---|
| zCoBo に ping が通らない / ECC が `WHEN_DESCRIBE` | `ssh root@192.168.4.8x`、`cd /etc/init.d; ./S99gethwserver stop; ./S99gethwserver start`(SURVEY p.230)。ダメなら電源再投入(§1-2) |
| ECC が Off/halt に見える(`onUnPrepare` / breakup 失敗、`ecc_error: WHEN_BREAKUP`) | **controller の reset では戻らない(Off のまま — 069 #14 実測)→ getEccServer を再起動**。Rust ブリッジは次の操作で自動再接続するので、そのまま run/start(歩き戻しは Idle から) |
| zCoBo の getHwServer が死んだ / 板の再起動 | run は `forced_eos:false` で自然終了に見える(受信 EOF)。run/stop は ECC の stop が 60 s 待って `abort:ecc-stop-failed` になるが受信データは無傷。板で `cd /etc/init.d && ./S99gethwserver start`(2 s で 46001 復帰)→ ECC 再起動 → run/start。zCoBo の ssh host key は起動ごとに変わる(仕様)ので `StrictHostKeyChecking=no` で |
| データが来ない・止まる(制御は通る) | ジャンボ経路を疑う: `ping -M do -s 8972`(073 #15)。receiver ログ `CoBo connected` の有無 |
| `extra_connections > 0` | 余分な TCP 接続(旧 dataRouter / 別 PC)。相手を止めてから run |
| 停電・UPS 放電 | §1 の順序でやり直す。logbook の最終行破損は許容(§9.1)— リーダが警告する |
| HV TRIP | DAQ は止めない(既定)。HV 側(CAEN GUI)で原因を見てから復帰。logbook に `slow` ALARM(STATUS の trip ビット)が自動記録 |
| Slow control が `—` / IDENT が出ない | `slowctl` プロセスと TOML の `port`(`/dev/serial/by-id/`)を確認。`identity_mismatch` が増えるなら別の機器を読んでいる(ケーブル差し替え後は by-id を見直す) |
| Parameters の Save が 503 | controller の TOML に `ecc_config_dir` が無い。409 = Running 中、400 = 値の範囲外(文言どおり直す) |

## 6. ログブック規約
- run_start/run_stop は自動。人が書くのは **comment**: 条件変更、異常、ビーム状態、線源の出し入れ。
- 圧・HV・流量・温湿度は slowctl が自動記録(`slow` レコード: IDENT / VALUE / ALARM / RECOVER / ERROR、run_start / run_stop に直近値)。
  slowctl を動かしていない run では **run 開始時に comment に数値を書く**。
- パラメータは `run_start.params.id` で追える。Exceptions を使った run はその理由を comment に。

## 7. チェックリスト(印刷用)
- [ ] 時刻同期 / ディスク空き / ネットワーク DF ping
- [ ] LV → zCoBo → ガス安定 → HV → DAQ
- [ ] pedestal → pulser → physics の順
- [ ] 窓・ベースライン・圧・HV の記録
- [ ] Parameters の `<id>` と exceptions N を確認(status バー)、Slow control の `slow` が `—` でない
- [ ] 停止は逆順、logbook にコメント

## 7.1 ソフトの反映チェックリスト(ELITPC、2026-10-14 — 110 の反映ミス 2 件から)
- [ ] run は Idle(`curl -s http://127.0.0.1:8080/api/status | grep phase`)。走っている run はユーザーの了解を取ってから止める
- [ ] コード: Mac の main を `git bundle` → ELITPC `~/tpcdaq-rs` で `git merge --ff-only` → `cargo build --release`
- [ ] root_sink: `make TPCDAQ_TPCRECO_DIR=/opt/tpcdaq/assets/TPCReco-HIGS2026_online -j`(reference/ はリポの外)。`make … -j test` も通す
- [ ] UI: `ui/dist/ui/browser` の**中身**を `/opt/tpcdaq/ui` へ(旧版は `ui.prevNNN` に退避)。dist/ui ごと置くと `/` が 404
- [ ] 停止・起動は **`~/tpcdaq-rs/tools/deploy/daq_{down,up}.sh`**(古い写しを使わない。slowctl も扱う版)
- [ ] 反映後: `/api/status` が 200 **と** `/` が 200(`curl -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/`)、`/srv/tpcdaq/logs/*.log` に WARN / ERROR なし、`pgrep -a -f target/release` に 8 プロセス + root_sink
- [ ] 試験で run を壊したら(stop せずに落とす等)、ログブックに comment を書き、`run_inprogress_*.root` の扱い(残す / 消す)を決める

## 8. 変更履歴
- v0.1(2026-09-17、Fable): 骨子。`[TODO]` は現地で埋める。069 0-3 のリハーサル記録を追記予定。
- v0.2(2026-09-23、Fable): §3.1 Parameters タブ(078)、§4 Slow control タブ(079 / 089)、§5 に slowctl / Parameters の症状、§6 の自動記録を反映。
- v0.4(2026-10-14、Fable): §7.1 反映チェックリスト。
- v0.3(2026-10-12、Opus): §4 に slowctl の記録先(InfluxDB)・機器 × ch の表・ch0 UNV・「機器を繋いだらトリガレートを A/B」(118–122)。

## HV の値を変えるとき(CAEN GECO2020、2026-10-15、archive/145)
1. `daq_down.sh`(slowctl が同じ USB ポート `/dev/ttyACM0` を使うため)。
2. デスクトップで `caengeco2020`(Wayland では事前に `xhost +SI:localuser:daq`、autostart 済み)。接続: **SMART HV / VCP USB / Port `ttyACM0`(`/dev/` を付けない)/ 9600 / LBus 0**。本体は REMOTE。
3. 設定後、GECO を閉じる(閉じないとポートの取り合い)。残った `/dev/shm/CAENV_LCK_0_0` は消してよい。
4. `daq_up.sh`。次の run_start.slow に新しい値が記録される。電圧値を web から変える経路は無い(SPEC §11)。
5. **LV(AsAd)を入れ直した直後の最初の run/start は configure(AGET レジスタの読み戻しが 0)で失敗することがある** — そのまま run/start をもう一度(archive/149、2026-10-15)。LV が OFF のままだと prepare が「AsAd が電源から切り離されている」で止まる(正常な拒否)。

### 7.1 の注意(2026-10-16 追記)
- 非対話 ssh / `setsid nohup` のスクリプトでは `~/.cargo/bin` が PATH に無い → `export PATH=$HOME/.cargo/bin:$PATH` を先頭に。無いと `cargo: command not found` でも先へ進み、古いバイナリのまま再起動してしまう(2026-10-16 に一度やった)。
- ELITPC に node は無い。UI は Mac で `cd ui && npx ng build` → `scp -r dist/ui/browser/. daq@…:/opt/tpcdaq/ui/`。`/opt/tpcdaq/ui` を先に消さない(cp の失敗で 404 になる)。

## ZS 運用(ゼロサプレッション、2026-10-16、TODO/155)
ZS を使うと event あたりのデータが大きく減るが、**ch ごとの閾値の表(pedestal)が電子回路の設定に依存する**。
表と設定がずれると、信号を捨てるかノイズを全部読むかのどちらかになる。

### 手順
1. **pedestal run** を取る: `triggerMode periodically` + `readout full`、150 events 程度(Parameters タブで Save → run/start)。
   ZS は **OFF**(full で全セルを読む)。peaking / gain は physics で使う値のままにする。
2. 表を作って ZS 付き params を生成(Mac / ELITPC のどちらでも。`cargo build --release` で `calib` が要る):
   ```
   tools/scan/zs_params.sh --ped-run /data/tpcdaq/run0123 --label 502ns \
       --base ~/test/scan/base.json --out ~/test/scan/zs_502ns.json
   ```
   要約の `base peakingTime_ns / gain` が pedestal run のときの設定と同じか**目で確かめる**(違えば base を直して再実行)。
   FPN を常に読みたい(共通モード減算用)ときは `--keep-fpn`。
3. 生成した JSON を Parameters タブ(または scan.py)で適用 → physics run。例外の件数は status バーの `exceptions N`
   (ch 数と同数になる。logbook に「ZS 閾値表 <label>」と書く)。
4. 表のファイルは `[system] ped_table` に置く(ZS データの下流再構成が使う)。**peaking / gain を変えたら表を取り直し、
   `ped_table` を更新して `daq_down.sh` → `daq_up.sh`**(再起動しないと古い表のまま)。

### カウンタの読み方
- decoder / root_sink / monitor のカウンタに ZS の項目が出る(名前は実装どおり、SPEC v1.49):
  decoder = run_stop.counters の `frames_type1` / `frames_type2` / `compact_size_mismatch`(ZS は type1 のみ、type2 と mismatch は 0 が正常)、
  root_sink = 終了 JSON の `pevent_zs_frames_table` / `pevent_zs_frames_no_table` / `pevent_table_missing_channels`、
  monitor = status JSON と停止ログの `uvwZsFragmentsRestored` / `uvwZsFragmentsNoTable` / `uvwTableMissingChannels`
  (run ごとに見える経路 = close 行 / status / run_stop は TODO/159)。**`pevent_zs_frames_no_table` または `uvwZsFragmentsNoTable` が 0 でない** =
  ZS のフレームが来ているのに表が読み込まれていない(`ped_table` の未設定・パス違い)。その間の ped は代用値で、波高・電荷は信用しない。
  表を直して `daq_down/up`、そのときの run は logbook に「ZS 表なし」と書く。
- `run_metrics.C` の指標 JSON の `ped_source` が `cells_1_20` で `zs_suspected` が立っていたら、ZS データを表なしで解析している
  (`metrics_for_run.sh --ped <表>` で解析し直す)。
- ZS を止めるときは base の `zeroSuppression.enabled` を false に戻して Save(閾値の例外は残ってよいが、`exceptions N` の理由を書く)。
