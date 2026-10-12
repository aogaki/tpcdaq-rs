# 118 — HV(HiVolta)/ LV(HMP2020)の読み出しを ELITPC で実機稼働(SPEC §11 の範囲内 = 監視のみ)

**Status: COMPLETED**(2026-10-12 起票・同日完了 Opus。ユーザー「そろそろ HV についてやりたい」→ 現地で USB 接続。Fable 一括レビュー承認)

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

## 決着: 原因は不良 USB ケーブル(2026-10-12 11:58–12:06)
- 119 の対策(一括読み出し・オンデマンドで開く)の実機確認のあと、ユーザーが HiVolta の **USB ケーブルを交換**した。
- 同じ「開くだけ」試験(`~/hvopen.py`、30 s ずつ、LV のみ読み出し中):

  | 状態 | 旧ケーブル | **新ケーブル** |
  |---|---|---|
  | ポート閉 | 6.40 Hz | 5.33 Hz |
  | 開・DTR / RTS 高 | 24.5 Hz | **5.03 Hz** |
  | 閉 | 6.37 Hz | 5.20 Hz |
  | 開・DTR / RTS 低 | 44.1 Hz | **4.83 Hz** |
  | 閉 | 6.77 Hz | 5.07 Hz |

- 最悪条件(`~/hvpersist.py`: 開きっぱなしで ch ごとに 34 往復を毎秒 = 最初に偽トリガが出たのと同じ使い方、34 s で 1156 往復): **5.03 Hz**(前後のベースラインは 5.07 / 4.27 Hz)。
- → **偽トリガは旧ケーブルを伝わった USB の雑音だった**(シールド不良か劣化)。DT1415ET の本質的な性質ではない。Ethernet での試験は不要になった(しても可)。
- 12:05:12 から slowctl は全機器(HV は `poll_s = 5`・オンデマンド、LV は 1 s)。60 s で 5.1 Hz。
- **run 69 の汚染区間**: 11:15:02–11:23:00(開きっぱなしの 1 Hz 読み出し + A/B 試験)、11:33:22–11:37:16(開きっぱなし + 30 s 周期)、
  11:37:49–11:39:33(開くだけの試験)、11:41:05–11:41:35(開く → 読む → 閉じる × 6、`hvcycle.py` の配置は 11:40:34)、≈ 11:28 の STATUS 一括読み出し 1 回、11:55:47–11:58(オンデマンド 30 s、≈ 12 発/回)。12:00 以降はケーブル交換後でクリーン。
- 教訓: 検出器に直結した機器の USB は、雑音の経路になり得る。新しい機器を繋いだら、まずトリガレートを A/B で確かめる。

## 残り
1. **daq_up / daq_down に slowctl を入れる**(下流から 2 番目あたり。止めるときは SIGINT)。手順書(NEW_DAQ_SETUP)と `config/daq_example.toml` に USB の by-id の例を足す。
2. UI の Slow control 画面でユーザーが目視する。
3. ログブックの量: hv は 1 回の記録で 34 行(ch ごと・値ごとに 1 行)。60 s ごとだと約 4.9 万行/日 → `log_every_s` か記録の形を見直すかは様子を見て決める。
4. 設定変更(Vset・ランプ)の経路: USB はオンデマンドになったので、読み出しの合間なら別プロセスも開ける(ただし同時に開くと slowctl の読み出しが 1 回失敗して数えられる)。前面パネル(CONTROL を LOCAL に)か、GECO2020 を Ethernet(TCP :1470)から同時に使えるか、を試す → on/off の設計(Fable キュー、SPEC §11)の入力になる。

## 結果(2026-10-12、ELITPC 現地コミッショニング)
- 環境: ELITPC(Ubuntu 26.04.1、daq)。HiVolta DT1415ET(S/N 165、FW 1.14)と HMP2020(S/N 101852、SW 2.62)を USB で接続、zCoBo 直結、α 線源 91 mbar。
  プローブは `~/hvprobe.py` / `~/lvprobe.py` / `~/hvstat.py` / `~/hvopen.py` / `~/hvcycle.py` / `~/hvpersist.py`(すべて読み出しのみ)。
- 読み出し: 2 台とも識別に成功。HMP の Interface を RS-232 から USB VCP に変更。
- 偽トリガ: A/B 表(上の「発見」「決着」節)。**原因は不良 USB ケーブルで、交換して解消**。最悪条件(開きっぱなしで毎秒 34 往復)でも 5.03 Hz。最終状態は全機器を読み出して 5.1 Hz。
- run 69 の汚染区間はログブックの comment(opus、2 件。2 件目は 11:41:05–11:41:35 の訂正)に記録した。
- コードのゲートは 119〜122 が担保(本票は現地コミッショニング)。
- 「残り」の扱い:
  - 1(daq_up)と 3(ログブックの量)は 121 で完了。
  - 2(UI の目視)はユーザー待ち(表の形はユーザーの要望で 120 に作り替え済み)。
  - 4(設定変更の経路)は Fable キュー(SPEC §11 HV / LV on/off)へ移した。オンデマンドで開くので、合間には GECO2020 / ローカル CLI も同じ USB を使える(Fable 2(f))。
- RUNBOOK v0.3 に「検出器まわりに機器を繋いだら、トリガレートを繋ぐ前後で比べる」を追記した。
