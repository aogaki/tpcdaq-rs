# 118 — HV(HiVolta)/ LV(HMP2020)の読み出しを ELITPC で実機稼働(SPEC §11 の範囲内 = 監視のみ)

**Status: IN PROGRESS**(2026-10-12 起票 Opus。ユーザー「そろそろ HV についてやりたい」→ 現地で USB 接続)

## 経緯
- 方針(10-09 ユーザー回答、Fable キュー): web からは on/off と V/I の読み出しだけ、上限・ランプはローカル(CAEN のアプリ)。
  on/off は SPEC §11(v1.29 安全境界)の改訂が要るので Fable 行き。**本票は読み出しだけ**(§11 の範囲内、改訂不要)。
- 085(コーデック)/ 089(Driver)は 2026-09 に完成済みで、実機との突き合わせ(現地 Phase 0)が残っていた。

## 実施(2026-10-12、ユーザー現地 + Opus 遠隔)
- ユーザーが 2 台を ELITPC に USB で接続した。HiVolta はそれまで GDED に USB で繋がっていたのを付け替えた。HMP2020 は何にも繋がっていなかった。
  - HiVolta: `/dev/serial/by-id/usb-CAEN_SPA_DT1415_HV_Power_Supply-if00`(cdc_acm、ttyACM0)。
  - HMP2020: `/dev/serial/by-id/usb-HAMEG_HAMEG_HO720_100031452875-if00-port0`(HO720 = FTDI VCP、ttyUSB0)。
  - daq は dialout グループに入っているので sudo は不要。
- 読み出し専用プローブ(`~/hvprobe.py` / `~/lvprobe.py`。MON / `*IDN?` / `SYST:ERR?` / `MEAS?` / `OUTP?` のみ):
  - **HiVolta**: `BDNAME=DT1415ET`、`BDNCH=8`、`BDFREL=1.14`、`BDSNUM=165`、**`BDCTR=REMOTE`**、`BDILK=NO`、`BDILKM=UNDRIVEN`、`BDALARM=00000`。
    VSET = 350 / 250 / 350 / 250 / 350 / 250 / 600 / 600 V、VMON ≈ VSET(**ch0 だけ 341.06**)、IMON 9.59 / 8.87 / 9.08 / 9.07 / 9.04 / 8.91 / 3.75 / 5.93 µA = **HV は印加中**。
  - **HMP2020**: 最初は `*IDN?` を書いても送り出されない / 応答が空だった。**本体の Interface が RS-232 になっていた** → ユーザーが USB VCP に変更 →
    `HAMEG,HMP2020,101852,HW50010002/SW2.62`、`SYST:ERR? = 0,"No error"`、ch1 = OUTP 1 / 3.600 V / 0.997 A(AsAd 電源)、ch2 = OFF。
    変更直後の 1 発目は、前の試行で受信バッファに残ったゴミと混ざって `-100 Command error` になった(2 発目からは正常)。
- `/opt/tpcdaq/daq.toml`(バックアップ `daq.toml.bak-2026-10-12-slowctl`)に `[slowctl]` を追加した。
  - hv / lv の 2 台。`expect_serial` は 165 / 101852 で、open 時に照合する。
  - `[monitor] slow_sub = "tcp://127.0.0.1:47006"` を有効にした。
  - その後 daq_down / up(Idle)し、**slowctl を手で起動した**(`$LOGS/slowctl.{log,pid}`。daq_up / daq_down にはまだ入っていない)。
- 結果:
  - slowctl は 2 台とも識別に成功した(`hivolta identified … snum=165` / `hmp2020 identified … serial=101852`)。monitor は 47006 を購読している。
  - ログブックに `slow` の VALUE が入る(hv は ch 0–7 × vmon / imon / status / vset + ボードの bdalarm / bdilk、lv は ch1–2 × volt / curr / outp + IDENT)。
  - **HV ch0 は機器自身が UNV(不足電圧、STATUS ビット 5)を立てている**: status 33 = ON + UNV、Vmon 341.08 V / Vset 350 V / 9.58 µA。
    slowctl は ALARM として記録した。ほかの 7 ch は status 1(ON のみ)。設定には一切触れていない。
  - slowctl がシリアルポートを専有するので、ほかのプロセスは同じポートを開けない(プローブで確認)。

## 発見: HiVolta のポーリングが偽トリガを作る(2026-10-12 11:17–11:23)
- ユーザー: 「何も信号がないのにイベントを取ることが多くなった」。ch0 の UNV は「ほかの ch に引っ張られて下がる」既知の挙動(異常ではない)。
- run 69(ユーザーが 11:15:02 に開始、25 MHz / trigDelay 1500 = run 57 と同じ設定)は 2.5 分で graw を 5 本(≈ 4.7 GB)書いていた。
  receiver の frames から **≈ 52 Hz**。run 57(一晩、HV / LV の USB 接続より前)は 4.86 Hz。
- A/B 試験(receiver の frames の 30 s 差分):

  | slowctl | レート |
  |---|---|
  | 2 台とも読む | 51.9 Hz → (停止)**6.53 Hz** → (再開)52.7 Hz |
  | HV だけ | **54.5 Hz** |
  | LV だけ | 6.87 Hz |
  | 読まない | 6.57 Hz |

  → **HiVolta(DT1415ET)の USB シリアル通信が偽トリガを作る**。HMP2020 は無関係。
  - 1 周 = 34 往復(8 ch × vmon / imon / status / vset + bdalarm / bdilk)で、余分 ≈ 48 Hz なので、1 往復あたり ≈ 1.4 発。
  - ユーザーも HiVolta の USB アクセスランプが激しく点滅しているのを見ていた。
  - 機序の推定(未検証): コマンド処理のデジタルノイズが、HV 出力(検出器の電極に直結)か USB のグラウンドを経由して CFD を叩く。
- **11:23:02 から slowctl は LV だけ**(`/opt/tpcdaq/slowctl_only_lv.toml`)→ 20 s で 3.2 Hz に戻った。
  run 69 のうち 11:15:02–11:23:00 は偽トリガが大半(HV を読んでいた区間: 〜11:18:50 と 11:19:20–11:21:3x と 11:22:27–11:23:00)。
- 切り分けの候補:
  - (a) Ethernet(TCP :1470、トランスで絶縁)で読んでも出るか → 出なければ USB のグラウンド経路、出れば機器内部のノイズ。
  - (b) USB アイソレータ。
  - (c) ソフト側の緩和: 全 ch 一括の問い合わせ(`CH:8`)で 1 周 6 往復にし、HV だけ周期を長くする(例 30 s)。偽トリガは ≈ 0.3 Hz まで減るが 0 にはならない。

## 残り
1. **daq_up / daq_down に slowctl を入れる**(下流から 2 番目あたり。止めるときは SIGINT)。手順書(NEW_DAQ_SETUP)と `config/daq_example.toml` に USB の by-id の例を足す。
2. UI の Slow control 画面でユーザーが目視する。
3. ログブックの量: hv は 1 回の記録で 34 行(ch ごと・値ごとに 1 行)。60 s ごとだと約 4.9 万行/日 → `log_every_s` か記録の形を見直すかは様子を見て決める。
4. 設定変更(Vset・ランプ)の経路: USB は slowctl が専有する。前面パネル(CONTROL を LOCAL に)か、GECO2020 を Ethernet(TCP :1470)から同時に使えるか、を試す → on/off の設計(Fable キュー、SPEC §11)の入力になる。
