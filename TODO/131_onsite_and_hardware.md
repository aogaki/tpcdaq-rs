# 131 — 現地・機器・ELITPC 実機でやること(069 / 073 / 079 / 084 / 101 / 107 の残りを 1 本に)

**Status: READY**(2026-10-13 起票 Opus。ユーザー指示「130 は後回しにして、69 などのスタックしているのを確認して解決していきたい」→
棚卸しで、止まっていたチケットの残りはほぼ全部「人が現地にいる / 機器を触る / ELITPC で run を止める」ものだった。
ユーザー裁定(同日): **1 本にまとめ、元のチケットは処置を書いて閉じる**)

各項目は独立。やれるときに 1 つずつ。終わったら ✔・日付・結果(数値)をこの md に書く。全部済むか、残りが不要になったら archive。

## A. ELITPC で run を止めてやること(遠隔で可、ユーザーの了解を取ってから)

| # | 項目 | やり方 | 元 |
|---|---|---|---|
| A-1 | **Arm の bind 競合リトライの実測** | 10-08 以降の ELITPC のログブック(audit `run/start`)から `ArmRetry{attempts, waited_ms}` を全 run 分集計するだけ(run は止めなくてよい)。仮想の実測(最悪 4 試行 / 157 ms)から桁で外れないこと | 069 #8 |
| A-2 | **run 開始失敗時に `next_run` が戻るか** | わざと 1 回失敗させる(例: params の render ができない current.json、または ECC を止めた状態で run/start)→ 番号が消費されないこと、500 が連発しないこと。統合テスト(`a_failed_start_rolls_back_and_leaves_no_half_started_run`、034)は済んでいるので実機の確認だけ | 069 #9 |
| A-3 | **24 h soak(ハード込み)** | periodic トリガ(例 10 Hz)で 24 h。031 と同じ様式で RSS / fd / カウンタを定期サンプリングし、後半半分で単調増加が無いこと。13.5 h の α run(107 の run 2)は成立済み — 足りないのはサンプリング付きの 24 h | 069 #21 |
| A-4 | **systemd ユニットと ufw** | 今は `daq_up.sh` / `daq_down.sh` の手動起動。docs/NEW_DAQ_SETUP_ja.md には方針(systemd ユーザーユニット、`After=` で順序、root_sink は `KillSignal=SIGINT`)だけで、ユニットの実体は未作成。ユニットの文案はこちらで書ける、有効化と ufw は sudo なのでユーザー作業 | 107 残り 7 |

## B. 機器・配線(現地)

| # | 項目 | 中身 | 元 |
|---|---|---|---|
| B-1 | **ガス(Bronkhorst FLOW-BUS)の読み出し** | Windows 側で FlowSuite / FlowPlot と LabVIEW の両方が動いている。COM ポートは排他なので仲介役(FlowDDE 等)がいるはず → ①Windows のポートと仲介役を確認 ②Windows 上の追加クライアントで読むか、別機器の D-sub を第 2 の local host にして読むだけにするか(079 §1.5 案 1)を決める ③node scan で各ノードの識別群 + 計測値のゴールデンを reference/ に。ドライバ(ProPar codec)は 079 Phase 1–2 で実装済み | 079 Phase 0 |
| B-2 | **Arduino MKR1000 + ENV shield の設置** | スケッチとドライバは 079 Phase 3 で実装済み。設置場所(検出器近傍? 室温・大気圧の用途)を決めて USB で ELITPC へ、slowctl の device に足す | 079 Phase 0、080 C-2 |
| B-3 | **GEM 信号トリガの測定**(現行 = CFD のまま、ユーザー確認 2026-10-13) | archive/084 §4 の 4 項目: ①フィルタボックス出力の波形(オシロ、数十イベント)で立ち上がり時間の分布 ②閾値 2〜3 点の turn-on 曲線 ③トリガ遅れ L ④12.5 MHz + delay 3600 で窓に収まるか。あわせて MSCF-16 の型番(F / LN / PMT)・CFD/LE ジャンパ・遠隔制御の接続を確認(084 §6)。CFD は µs の信号に原理的に合わない(084 §2)ので、作り直すなら A 案(積分 + LE)が候補 | 084 |
| B-4 | **前面スイッチ OFF での HV ON 拒否(DIS)** | 任意。124 の実機受け入れで残した 1 項目 | 124 |

## C. 運転条件の判断(ユーザー)

| # | 項目 | 中身 | 元 |
|---|---|---|---|
| C-1 | **取得を 25 MHz に戻すか** | 今の mini は 100 MHz(`delay_test.xcfg`)で、時間窓 5.12 µs ≈ ドリフト 24 mm。|θ| > 10° の飛跡は窓からはみ出て電荷が欠ける(101 追記 3)。25 MHz なら窓 20.48 µs ≈ 98 mm。戻すときは triggerDelay も窓の位置に合わせて決め直す(080 §0 の式 c = (窓 − delay − L) / cell 幅)。手段は Parameters の Apply & restart(111 / 112)。macros 側の samplingMHz はユーザーの macros の話 | 101 |

## D. 人に頼むもの(任意、あれば嬉しい)

- physics run 追加数本のコピー(別日・別 trigDelay)— 073 #2。
- MAGBOLTZ22 の出力(運転点表の元、D_L / D_T の一次値)— 073 #3。拡散は熱限界一致まで導出済みなので照合用。
- HIGS の DCS / InfluxDB エクスポート(physics run 日の圧力・温度・HiVolta 電流)— 073 #4。
- ログブック原本の写真(p.030、p.123–126、p.254/257–259)— 073 #6。

## 参照
archive/069(Phase 2〜4)、archive/073、archive/079(§1.5、Phase 0)、archive/084、archive/101(追記 3)、archive/107(残り)、124。
