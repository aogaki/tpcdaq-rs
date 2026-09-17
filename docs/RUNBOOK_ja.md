# mini eTPC 運転手順書(RUNBOOK)— v0.1 骨子

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

## 4. 監視すべきもの(UI)
- status: receiver `overflow_frames` / `extra_connections`、decoder `malformed`、graw-writer `write_errors`、root-sink
  `events_incomplete` — **全部 0 が平常**。増えたら §5。
- 窓端クリップ率(B-4 実装後)、slowctl のガス圧・流量・HV(079/C-3 実装後)、ALARM バッジ。
- レートスパイクはビーム由来が大半(SURVEY §4)。制御室に連絡。

## 5. 復旧
| 症状 | 手当 |
|---|---|
| zCoBo に ping が通らない / ECC が `WHEN_DESCRIBE` | `ssh root@192.168.4.8x`、`cd /etc/init.d; ./S99gethwserver stop; ./S99gethwserver start`(SURVEY p.230)。ダメなら電源再投入(§1-2) |
| ECC が Off/halt に見える(`onUnPrepare` 失敗) | controller の reset を 2 回(Described → Idle)。改善しなければ getEccServer 再起動 |
| データが来ない・止まる(制御は通る) | ジャンボ経路を疑う: `ping -M do -s 8972`(073 #15)。receiver ログ `CoBo connected` の有無 |
| `extra_connections > 0` | 余分な TCP 接続(旧 dataRouter / 別 PC)。相手を止めてから run |
| 停電・UPS 放電 | §1 の順序でやり直す。logbook の最終行破損は許容(§9.1)— リーダが警告する |
| HV TRIP | DAQ は止めない(既定)。HV 側で原因を見てから復帰。logbook に自動記録(C-3 実装後) |

## 6. ログブック規約
- run_start/run_stop は自動。人が書くのは **comment**: 条件変更、異常、ビーム状態、線源の出し入れ。
- 圧・HV・流量は slowctl が自動記録(実装後)。それまでは **run 開始時に comment に数値を書く**。

## 7. チェックリスト(印刷用)
- [ ] 時刻同期 / ディスク空き / ネットワーク DF ping
- [ ] LV → zCoBo → ガス安定 → HV → DAQ
- [ ] pedestal → pulser → physics の順
- [ ] 窓・ベースライン・圧・HV の記録
- [ ] 停止は逆順、logbook にコメント

## 8. 変更履歴
- v0.1(2026-09-17、Fable): 骨子。`[TODO]` は現地で埋める。069 0-3 のリハーサル記録を追記予定。
