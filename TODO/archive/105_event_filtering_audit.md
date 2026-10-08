# 105 — イベント選別の総点検(multiplicity 以外も含む、検出器 → 解析の全段)

**Status: COMPLETED**(2026-10-08 起票・調査 Opus。ユーザー「multiplicity 以外でも何らかの形でイベントのフィルタリングが行われていないか精査して」)

## 方法
- .graw の全フレームヘッダを直接読む(`reference/_spike/stop_0929/grawscan.py`、CoboFormats-Rev-5-Compact.xcfg のオフセット)。対象 6 run(09-25 / 09-29 / 09-30 / 10-01 / 10-02 / 10-07)、計 162,948 フレーム。
- GET ソース(`reference/20190315_patched/GetBench/src/get/rc/CoBo{Node,ConfigParser}.cpp`)で設定値がどのレジスタに効くかを確認。
- 解析マクロ(graw2root / analyzeUVW / makeTracks)の事象数の保存。

## 結果(段ごと)
| 段 | 選別の可能性 | 判定 | 根拠 |
|---|---|---|---|
| 検出器 → フィルタ箱 → CFD(Vth)→ ゲートジェネレータ | 波高(電荷)閾値、出力幅・再トリガ禁止、veto 入力 | **ここが唯一の内容依存の選別** | 総電荷の下限 3.1e5 → 1.5e5(Vth 500 → 300、103)。幅・veto 配線はデータから見えない → ユーザー確認 |
| CoBo のトリガ受付 | モードの選択、busy 中の取りこぼし | 内容非依存の **約 1 s の不感時間のみ**(104) | ソース: `setTriggerMode` は選んだモード 1 つのマスクだけ立てる。`external` なら multiplicity / timing / slowControl は無効。`triggerPeriod` はレジスタに書かれるが、不感時間の端が 0.84〜1.0 s でばらつく(10 ns 精度の時刻で 1.000 s ちょうどの壁ではない)ので固定のファーム hold-off ではなく、転送・保存側の可変な待ちと判断 |
| AsAd / AGET の読み出し | 部分読み出し(readIfHit / readAlways マスク)、ゼロサプレッション、FPN 省略 | **無し** | 全フレームが frameType 1、itemCount 139,264 = 272 ch × 512 セル(全読み出し)、readOffset 0、status 0、windowOut 0xFFFFFFFF。AGET の hitPat は 16〜46 本と事象ごとに変わるが読み出しには使われていない。multip 欄は AGET 0〜3 で 1 / 2 / 4 / 8 の定数(実際の多重度ではない) |
| CoBo → dataRouter → 保存(VM) | フレームの破棄・間引き | **無し** | eventIdx は全 run で 0 から欠番なし(チャンク境界も連続)、ファイル末尾の切れ無し、トポロジーフレームは run 先頭の 1 個だけ |
| graw2root.C | データ以外のフレームと item サイズ異常のみ捨てる(警告付き) | **無し** | 09-30 / 10-01 / 10-02 / 10-07 はフレーム数 = raw の事象数。09-25(94,800 対 93,953)と 09-29(19,224 対 14,203)は run 継続中に変換した分で、末尾チャンクが未変換なだけ(連番の途中で切れている)|
| analyzeUVW / makeTracks | — | **無し**(事象は全部ツリーに残る。再構成失敗は ok = 0) | tracks の事象数 = raw の事象数 |
| 解析マクロのカット(stoppedAlphas 等) | 線源方向・左壁・停止判定 | 明示的なカットのみ | 各マクロ先頭の定数 |

## 内容に依存しない選別だが物理に効くもの
- **約 1 s の不感時間は時間相関のある崩壊を選択的に落とす**: ²²⁰Rn → ²¹⁶Po(T½ 0.145 s)は 99.2 % が 1 s 以内に崩壊 → ガス中の Rn–Po 遅延同時計数(083 で狙っていたもの)は現行 DAQ では原理的に見えない。²¹²Bi → ²¹²Po(0.3 µs)は同じ 20 µs の窓に入るので別問題(同一フレーム内の 2 本)。

## ユーザー確認事項
1. ゲートジェネレータの出力幅・モード(再トリガ可否)と、veto / inhibit 入力に何か(CoBo の busy 等)が繋がっているか。LED が CFD 出力かゲート出力か。
2. VM の物理設定で `triggerMode` / `secondaryTriggerMode` / `readoutDepth` / `enableZeroSuppression` と、LEMO I/O の `function`(busy_output の割り当て)。

## 追記(2026-10-08、ユーザー「FPGA にアクセスできますか」)
- 経路: Mac → GDED(daq、鍵)→ zCoBo(root、GDED の `~/tpcdaq-rs/zcobo.py`)。2026-10-08 確認: Linux 4.9.0-xilinx(ARMv7、Zynq PS)、稼働 9 日、getHwServer 稼働中(VM の GET でデータ取得中)。
  FPGA は `/dev/mem` と UIO 3 個(gpio 0x41210000 / axidatagenerator 0x46000000 / udmabufconfig 0x54000000)、`busybox devmem` あり。**レジスタには未アクセス**(run 中のため)。
- レジスタ表 = `reference/ZC706_20181031_ELINP/configs_1asad/hardwareDescription_zCobo.xcfg`、`ctrl` の base 0x44000000。問題に直結する読み取り専用レジスタ:
  | レジスタ | アドレス | 意味(xcfg の説明) |
  |---|---|---|
  | extTriggerCnt | 0x440001CC | CoBo が受けた外部トリガの総数(pipeCtrl/reset でリセット) |
  | eventCounter | 0x44000090 | 循環バッファに書いた事象数 |
  | fifoHighWaterMark | 0x440001D0 | FPGA 内 FIFO の最高水位 [B](下流の詰まりの指標) |
  | triggerDelay / triggerPeriod_10ns | 0x44000068 / 0x44000084 | 実際に設定されている値(VM の設定ファイルを見ずに確認できる) |
  | firmwareDate | 0x440001D8 | FW のビルド日 |
  extTriggerCnt − eventCounter = busy 中に来て捨てられたトリガ = 不感時間の直接測定。
- 注意: 未定義アドレスへの AXI 読み出しは Zynq をハングさせ得る(= 取得中の run が止まる)。アドレスは zCoBo 自身の記述ファイル由来だが、run 中に読むかはユーザー判断。**書き込みは一切しない**。

## 追記 2(2026-10-08、ユーザー「リスクは承知で」→ zCoBo の FPGA レジスタを読み取りのみで読んだ。書き込みなし、zCoBo・取得とも正常継続)
`busybox devmem <addr> 32`(GDED 経由)。base の確認に firmwareDate を先に読んだ。
| レジスタ | アドレス | 値 | 解釈 |
|---|---|---|---|
| firmwareDate | 0x440001D8 | 0x20180129 | 2018-01-29(アドレスは正しい) |
| hardwareVersion | 0x440001D4 | 1 | Reduced CoBo(zCoBo) |
| **triggerMask** | 0x4400008C | **0x00000001** | **external のみ有効**(multiplicity / timing / manual / slowControl / 2p / _out すべて 0) |
| triggerDelay | 0x44000068 | 0x140 = 320 | 3.2 µs |
| triggerPeriod_10ns | 0x44000084 | 0x05F5E100 | 1 s(既定値。timing モードが無効なので不使用) |
| eventCounter / extTriggerCnt / fifoHighWaterMark / timeStamp | 0x90 / 0x1CC / 0x1D0 / 0x94・0x98 | **すべて 0** | 4 分あけて再読も 0。その間 .graw は 14 フレーム(7,802,368 B)増加 = **このファームでは未実装**(zCoBo の Trigger_Unit は CoBo とは別実装) |

- zCoBo ファームの README(`reference/ZC706_20181031_ELINP/docs/README.txt` §5): **「現行ファームは MULTIPLICITY でのトリガも 2 陽子モードも扱えない」** → multiplicity による選別は zCoBo では原理的に無い。
- トリガ受付数はレジスタでは測れない。代わりにハードで測れる: FMC-to-AsAd アダプタの SMA **OUT1 = trig_pulse_nodelay**(時刻と事象番号をラッチした瞬間 = 受け付けたトリガ 1 回につき 1 発)、OUT2 = triggerDelay 遅れの実停止。
  **IN1(ゲートジェネレータからの入力)と OUT1 をスケーラーかオシロで数えれば、CoBo の受付率と不感時間が直接分かる**。

## 追記 3(2026-10-08、ユーザー「GET 以外にも FPGA のシステム上に問題になりそうなものがないか」)— zCoBo の Linux を読み取りのみで点検(取得は継続)
| 項目 | 結果 | 判定 |
|---|---|---|
| CPU / 負荷 | 100 % アイドル、load 0.00。プロセスは init / syslogd / klogd / dropbear / getHwServer のみ | ○ |
| メモリ | 1,030 MB 中 727 MB 空き、swap 無し。getHwServer の RSS 144 MB(**VmHWM = VmRSS** = 今が最大値。増え続けているかは 1 日後に再確認) | ○(要再確認) |
| ファイルシステム | ルートは RAM ディスク、/var/log → /tmp(tmpfs、使用 24 kB)。ログで RAM が埋まる心配無し | ○ |
| カーネルログ | 起動時のメッセージのみ。エラーは USB OTG の probe 失敗だけ(無関係) | ○ |
| データ用 NIC(eth1、AXI Ethernet) | 1 Gbps 全二重、flow control off。TCP: OutSegs 535 万、**再送 0**、エラー・リセット 0。VM への接続の送信キュー 0 | ○ |
| DMA バッファ | udmabuf 128 MB × 2(フレーム 557 kB なら約 240 個分)。zCoBo 側で詰まっている兆候無し | ○ |
| getHwServer | スレッド 7。データ用とみられる 1 本が `poll_schedule_timeout`(9 日で sys 11.5 s)、他は futex / epoll 待ち。UIO の割り込みは /proc/interrupts に出ていない(ポーリング動作) | ○(下記) |
| 温度(XADC) | 41 °C | ○ |
| ファーム | SD カード(読み取り専用でマウント → 解除)の BOOT.bin / devicetree.dtb / uImage / uramdisk の md5 が `reference/ZC706_20181031_ELINP/SD_image` と**完全一致**(2018-10-31 版)。firmwareDate レジスタの 2018-01-29 はビットストリームに焼かれた値 | ○ |
| 時計 | 1970-01-10(RTC / NTP 無し)。ログの時刻は無意味。事象の時刻は FPGA のカウンタなので**データには無影響** | △(実害なし) |
| その他 | 別の対話セッションで `top` が動いていた(点検の約 2 分前に開始、こちらの操作ではない → 触らず)。SD カードに macOS のメタデータ(2025-03-31)が残っている(無害) | — |
- 不感時間との関係: zCoBo 側は CPU・NIC・DMA バッファとも余裕で、送信キューも空 = **書き出しの詰まりは zCoBo 側では起きていない**。約 1 s の不感時間は、トリガ受付後の FPGA と getHwServer の間の受け渡し(ポーリング周期や再アーム待ち)か、VM 側の受信のどちらか。zCoBo の getHwServer は CoBo 版の移植でソースが手元に無いので、内部は確認できない。
  切り分けは、run の無い時に AsAd パルサーを 2 Hz / 5 Hz で入れて記録の頭打ちを見る、か OUT1 と IN1 を数える(追記 2)。
- ユーザー回答(2026-10-08): 時計 1970 は電源断で全リセットされる仕様(既知)/ getHwServer のメモリ増加は仕方ない(許容)/ `top` のセッションはユーザー本人。→ 3 点とも対応不要。
