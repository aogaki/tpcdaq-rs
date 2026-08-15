# 仮想 zCoBo — 調査と設計方針(フォーク B: 実 ECC + 仮想 zCoBo)

- **status**: v1.2(2026-08-15 — **トラック完了**。039/040/041/042 全ユニット green。
  統合デモは実 .graw で run 3 本 + 歩き戻し実証 + 異常系観測まで完了(041 結果節が正)。
  起動レシピ = `reference/_spike/demo/`。発見 3 件は SPEC v1.14 + TODO/043 + 033 追記で処理)
- **前提文書**: [SPEC_ja.md](SPEC_ja.md)(実装の正本、v1.12)/ [TODO/038](../TODO/038_real_ecc_local_spike.md)(本調査の発注書)
- **決定の記録**: フォーク B 採用 = 2026-08-14 ユーザー裁定([TODO/CURRENT.md](../TODO/CURRENT.md) 裁定節)

## 1. 目的と位置づけ

**graw ファイルをデータソースとする仮想 zCoBo(CoBo を板ごと偽るソフト)を作り、
実 ECC の配下に置く。** これにより検出器・実ハード無しで、UI の Run 制御から
データ保存・モニタまでの**全経路が本物**のデモ/開発環境が成立する。

- **用途 1 — デモ改良**: Run 制御 UI を「完成形レイアウト + 全 disabled」から**実配線**へ
  進める鍵。「モック禁止」裁定(2026-08-13)は **UI の偽装の禁止**であり、仮想 zCoBo は
  境界の反対端(ハード)の置き換え = UI から見える経路は全て本物になるため両立する。
- **用途 2 — ELI-NP 実機テストの予行**: 出張から帰って実機(1 CoBo mini TPC)に触った時、
  デバッグ→運用が最短で立ち上がるように、制御プレーンの実挙動をローカルで毎日回しておく。
  実験と同一版の ECC バイナリ(reference/20190315_patched)がデモで回る = 制御面の忠実度最大。
  「テストダブルが実機より甘いと誤実装が green で通る」(036 の教訓)への構造的な答え。
- **用途 3 — 負荷ハーネスのソース**(031 と接続): ペーシング可変にすれば §12-5/12-6 の
  負荷試験のデータ源そのものになる。

### フォーク A(不採用)との比較

fake_ecc 拡張(ECC ごと偽る)は安いが、ECC の状態機械・タイムアウト・エラー文言が
本物でない。032/034/036 で「実 ECC でしか出ない挙動」に二度足を掬われた経緯から、
**制御面は本物を使う**(B)をユーザー裁定で採用。fake_ecc は引き続き CI の e2e
テストダブルとして維持する(用途が違う — B は CI を置き換えない)。

## 2. 全体像

```
                     ┌──────────── tpcdaq-rs(全て本物)────────────┐
 [Web UI] ──REST──▶ controller ──JSON REQ/REP──▶ ecc-bridge(Ice client, enc 1.1)
                        │                             │
                        │                        Ice(GetEcc 面)
                        ▼                             ▼
                    receiver ◀──TCP データリンク── ┌─────────────┐      ┌───────────────┐
                    (listen-before-start)          │ 仮想 zCoBo  │◀─Ice─│ 実 ECC サーバ  │
                        │                          │ (本文書の対象)│      │ (20190315 版) │
                        ▼                          └─────────────┘      └───────────────┘
              decoder → {graw-writer, root-sink, monitor}      データソース = 実 .graw ファイル
```

- ecc-bridge → ECC の面(describe/prepare/configure/start/stop/breakup/reset/status)は
  現行実装のまま。**仮想 zCoBo の導入で我々の Rust/C++ 側は 1 行も変わらない**のが理想形。
- 仮想 zCoBo が受けるのは「ECC がハードノードに向かって呼ぶ Ice 面」+
  「DataLinkSet に従い receiver へ TCP を張ってフレームを流すデータ面」。

## 3. 既に確定している事実(032/034/036 + SPEC v1.12 — 本調査の前提)

- **データリンクを張るのは CoBo 自身**。ECC は Ice で指示するだけで probe 接続を張らない。
  接続確立は **configure の時点**(GET 純正 DataRouter は接続確立後 listen を閉じるので、
  probe があれば純正が動かない — 決定打)。
- **ecc stop はデータリンクを close しない**(MDaq `DaqCtrlNodeI.cpp:419-435` の daqStop =
  割り込み停止 + flush)。close は breakup か次の configure の resetDataSender。
  → 実機停止では EOF が来ない = **強制 EOS が正規経路**(receiver は「リンク保持 + 送信静止」
  を正常系として扱う — 032/033)。
- **実 ECC の reset は EV_UNDO = 1 段戻し**(`GetBench/src/get/rc/BackEnd.cpp:924`)。
  Active(Ready/Running)からは無音で無視。configure は `ST_PREPARED` ガードで黙って
  スキップ(`BackEnd.cpp:955-962`)。→ Ready → Idle は **breakup → reset → reset** の歩き戻し。
- **DataLinkSet の実機形式**: DataSender id は `CoBo[k]` 形式、flowType は大文字 `TCP`
  (FW 資料 `ZC706_20181031_ELINP/README_SCRIPTS.txt` でも確認済み)。Ice は encoding 1.1 固定。
- **我々の ecc-bridge の契約**(SPEC §8.2): JSON REQ/REP、configure に `config_id` + `links`
  (receiver が**実際に bind したポート**を渡す)。DataLinkSet XML は links から生成。
- **listen-before-start**(SPEC §1.3): ecc start の前に receiver が listen 済みであること。
- ELITPC は 1 論理 CoBo × 4 AsAd(2 枚の zCoBo を 1 CoBo として扱う)。mini は 1 CoBo × 1 AsAd。
  **AsAd 数・CoBo 数は設定駆動**(ch 数焼き込み禁止の不変条件と同根)。
- フレームは **frameType 2(compact rev 5、blkSize256/big-endian)**が実機の現実(2022 時点で既に)。
  仮想 zCoBo のデータソースは実 .graw のリプレイなので、**ワイヤ上のバイト列は構成上本物**。

## 4. 調査結果(038 スパイク、2026-08-14 実走)

> 4 レーン並列調査: A) 実 ECC のローカルビルド成立性 B) ECC→ハード制御面の棚卸し
> C) データリンク挙動 D) 実 getHwServer の流用可能性。結果は判明し次第ここに反映。

### 4.1 実 ECC のビルド・起動成立性(レーン A — 完了、**macOS 成立**)

**結論: 実 ECC サーバは macOS(arm64, Darwin 25)でビルド・起動し、実 Ice トラフィックを
喋る。ソースパッチは実質 2 行。** 全 7 パッケージ(TinyXml → Utilities → CompoundConfig →
StateMachine → MultiFrame → MDaq → GetBench)のフルビルドは `-j8` で約 10 分、
`reference/_spike/build_all.sh` で無人再現可能。

- **本体の特定**: ターゲット `getEccServer`(`GetBench/Makefile.am:347`、
  main = `GetBench/src/main/getEccServer.cpp:85`)。Ice identity **`Ecc`**、port **46002**。
  MultiFrame は必須(configure.ac:241)、DB(ConfigDatabase/soci)は `--disable-database-support`
  で不要。GUI 無関係。
- **鍵は Ice 3.7**: 手元既定の Ice 3.8.2 は IceUtil/C++98 マッピング廃止で移植不能級。
  `zeroc-ice/tap` の **`ice@3.7`(3.7.11)に arm64 用ビルド済み bottle があり**、
  **unlinked keg** として導入(システム既定は 3.8 のまま — `tools/ecc_bridge` 無影響を実証)。
- **パッチ 2 行 + 機械的修正**: `Factory.hpp` の include 1 行(Boost 1.90)/
  `PeriodicTask.cpp` の `io_service`→`io_context` 1 行(Boost 1.87)。他は configure の
  Ice パス再標的(GET の m4 マクロの潜在バグ)と環境変数(`-std=gnu++14`、libboost_system
  スタブ、`-Wl,-undefined,dynamic_lookup`、`--host=aarch64-apple-darwin`(configure.ac:137 の
  arm ガード誤爆回避)、**Ice 3.7 libdir を LDFLAGS 先頭に**)。全記録 =
  `reference/_spike/{patches.diff,env.sh}`。
- **起動・実通信の実証**:
  - `getEccServer --config-repo-url <dir>` → 0.0.0.0:46002 listen、`getEccClient` で
    `sm-status` = `IDLE / NO_ERR`。
  - 実 xcfg(`describe-zCobo-ZC706` + `configure-pulser`)で `config-list` が実トリプレットを
    返す。ハード無し `sm-describe` は ConnectTimeout → `WHEN_DESCRIBE` で健全にエラー着地
    (クラッシュ・ハング無し)。
  - **我々の ecc_bridge(Ice 3.8.2)→ 実 ECC(3.7.11)の実通信成功**
    (`{"action":"status"}` → `{"ok":true,"state":"Idle"}`、encoding 1.1)。
    Ice 3.7/3.8 の wire 互換をテストダブルでなく実物で実証。
  - **ボーナス(039 の前倒し)**: `getHwServer` も `--enable-cobo` でビルドでき、
    **実 ECC ↔ ローカル getHwServer の接続まで実証**
    (`Connecting to 'CoBo' node '0' at 127.0.0.1:46001` → `Added HW node`)。
- **運用上の罠 2 件(実測)**: ①**1 プロセスに Ice runtime 2 つが混ざると無言 SIGSEGV**
  (`OutputStream::writeSize` — LDFLAGS の順序で 3.7/3.8 が混線した時の症状。診断出力なし)
  ②getHwServer は **`--Ice.IPv6=0` 必須**(無指定だと IPv6-only bind になり、IPv4 で来る
  ECC と繋がらない。getEccServer 自身は内部で IPv6=0 を強制している)。
- **判明したギャップ(ELITPC プロファイルのみ)**: 実 zCoBo の hardwareDescription は
  `registerAccess=zCobo` を要求するが、SimServerDevices の登録は
  `Nested/MemBus/AGetBus/AsAdBus/LocalMap` のみ(`zCobo` 型は ARM/VxWorks ビルド専用)。
  LocalMap 代用はフラットアドレス空間の衝突(`already declared at 0x10`)で不成立。
  **mini プロファイル(fullCoBoStandAlone = MemBus/AGetBus/AsAdBus)は Sim 登録で充足**
  するため、当面のデモ(mini 相当)に影響なし(§6-R8)。
- **Linux/Docker の見立て**: ほぼ確実(本スナップショット自体が Ubuntu 16.04 で実ビルドされた
  痕跡あり。今回のパッチは全て modern Homebrew(Boost 1.90 / Ice 3.8 既定)起因)。
  **再現可能なデモ基盤としては Docker、日常の高速イテレーションは macOS ネイティブ**の使い分け。
- **成果物**(`reference/_spike/`、gitignore 済み): `build_all.sh` / `env.sh` /
  `patch_ice_prefix.sh` / `patches.diff`(108 行、実ソース変更は 2 行)/ `src/`(パッチ済み
  コピー)/ `build/*/` ログ / `prefix/bin/`(getEccServer, getEccClient, getHwServer,
  dataRouter ほか)/ `run/`(実行ログ)。**reference/ の正本 2 ディレクトリは無変更を検証済み**。

### 4.2 ECC → ハードノードの Ice 面(レーン B — 完了)

**⚠ 最重要の訂正事項: ECC → ハード側の Ice encoding は 1.0。**
ECC はハードノードへの全プロキシに `-e 1.0` を焼き込む(`MDaq/src/mdaq/EccBackEnd.cpp:59-65`、
`HardwareNode.cpp:134-139`、`CoBoNode.cpp:133-137` — 出荷 .so 内の文字列でも確認)。
CLAUDE.md の「encoding 1.1 固定」は **ecc-bridge → ECC のレッグの話**であり、このレッグは別。
仮想 zCoBo(の自作部分)は **encoding 1.0 のリクエストを受ける**こと。
プロキシ形式: `"<Identity> -e 1.0 :default -h <IP> -p <PORT>"`。

**エンドポイント発見機構(仮想 zCoBo がどこで listen するか)**:
- describe xcfg の `Setup/Node[CoBo]/Instance[<id>]/Endpoint`(IP 4 オクテット + `<port>`)。
  port=0 なら既定 **46001**(`hwServerCtrlPortNum`)。実例 = `describe-mono-node.xcfg`
  (port 46001、`HardwareConfigId=fullCoBoStandAlone`、`batchProcessing=false`)。
- **DaqCtrlNode は「同じ IP の port 46004」にハードコード**(`HardwareNode.cpp:133,139` —
  xcfg で変えられない)。
- ノード名は describe 中に ECC が `setName("CoBo[<instanceId>]")` で上書きし、DataLinkSet の
  `DataSender id` と突合される(不一致は configure が throw)。

**状態遷移ごとの呼び出し(要点)**:
- **describe**: `HwNode` identity へ checkedCast(`ice_isA`)→ `name`/`destroy`/
  **`create(NodeConfig)`**(ハード記述 xcfg 全体 = 6594 行を 1 メッセージで投入。デバイス
  `ctrl`/`asad`/`aget`/`zeroSuppress*`/`pll` 等を生成させる)→ `getMapOfDevices`
  (**デバイス名を identity とする `Device` プロキシ群**を返す — 以後のレジスタ操作は全部
  このプロキシ経由)→ `setName` → ctrl へ mutantConfig 初期化 writeField×2。
- **prepare**: 大量の `writeField`/`writeRegister`/`readField`/`readRegister`(ctrl/asad/aget)+
  `LedManager::setLEDs/modifyLED`(**例外は握り潰される — no-op 可**)+
  `AlarmService::reset/subscribe`。Mutant 0 台なら slot/alignment 系は**丸ごとスキップ**。
- **configure**: 数百の register 書き込み + `setDataLinks`(ECC 内ローカル)+ `daqConnect` =
  ①46004 へ `ice_isA` ②**`DaqCtrlNode::connect(flowType, "ip:port")`**
  ③`setCircularBuffersEnabled(asadMask)`。
- **start**: asad 温度 read → ctrl レジスタ操作(resetTime/pipeCtrl/readPtr)→
  **`DaqCtrlNode::daqStart()`** → pipeCtrl 有効化。
  **stop**: pulser 停止(例外握り潰し)→ 温度 read → pipeCtrl 無効化(+ECC 側 500 ms sleep)→
  **`DaqCtrlNode::daqStop()`**。pause/resume は stop/start とほぼ同一。
- **breakup**: stop 全シーケンス → **`DaqCtrlNode::disconnect()`** → mutantConfig リセット。
- **reset(EV_UNDO)**: Prepared→Described(asadEnable 落とし + `AlarmService::unsubscribe`)/
  Described→Idle(ノード破棄。デストラクタが initDone=0 を書く)。Active からは何もしない。

**レジスタプリミティブは 4 つだけ**(+batchProcessing=true 時のみ `execBatch`。既定 false):
`writeRegister` / `writeField` / `readRegister` / `readField`。

**タイミング制約: 事実上なし。** ECC → ハードの Ice 呼び出しにタイムアウト設定は無い
(唯一の 5 s は rebootHardware 専用)。ECC 側が自分で sleep(200 ms〜1 s)を挟む。
ただし checkedCast(`ice_isA`)がアクセサ利用のたびに再発行されるので応答は軽くすること。
**応答しないと ECC スレッドが永久ブロック**するため、仮想側は必ず応答を返すこと。

**チェック付き read-back(素通しでは prepare が throw する値)**:
| デバイス/レジスタ | 要求値 | 意味 |
|---|---|---|
| `asad`/`monitorID` | **0x41**('A') | AsAd 実在確認(CoBoNode.cpp:1157-1166) |
| AsAd AIM user field | 1+asadIdx | AsAd 識別(:1170-1186。ECC 自身が書いてから読む可能性あり — 実走で確認) |
| `aget`/`reg5` | 0x201/0x202/0x203 | AGET チップ版数(:64, :1756-1765) |
| `ctrl`/`asadConnection.PLG<i>` | 1(active AsAd) | 物理接続検出(:274, :1044-1048) |
| `ctrl`/`asadStatus.alarm` bit | 期待値(xcfg の checkPowerSupply=false で緩和可) | 電源アラーム |
| `asad`/`VDD` | 閾値以上(10 回リトライ内) | 電源電圧(:2554-2580) |
| `ctrl`/`memStart*` | 読めれば可(0 でも整合) | start 毎に readPtr へ書き戻される |

**種付け(seed)の成立性 — 追加確認済み(2026-08-14 主対話)**: ハード記述 xcfg に初期値
`<value>` は**皆無**(grep 0 件)なのでシミュレータのレジスタは全ゼロ起動 = 上表の値は
**外部から仕込む必要がある**。一方、サーバ実装(`MDaq/src/mdaq/hw/server/`)は xcfg の
`readOnly` を**強制していない**(該当コード無し — GUI 向けメタデータ)。
→ **describe 後に Ice クライアントとして `Device::writeRegister/writeField` で魔法の値を
書き込む「シーダ」が成立する**。タイミング(describe 完了〜prepare 開始の窓)にレースが
あるため、実走検証と §5 の設計判断の対象。

**ECC 側の設定リポジトリ**: `getEccServer --config-repo-url <dir>`(既定 `.`)の
**フラットなディレクトリだけで動く**(DB は configure フラグ無効時は完全不要)。
命名 = `describe-<id>.xcfg` / `configure-<id>.xcfg` / `hardwareDescription_<id>.xcfg`
(注意: prepare 用の読み込みも実装は **configure-** プレフィクスを読む)。Ice 面の
configId は素の文字列(複合 XML 形式は SOAP 専用)。

**Mutant 無し・単一 CoBo で不要な面**: MutantLinkManager(呼ばれない)/ AlignmentServer /
`Node::reboot/shutdown/testConnectionToHardware`(CLI 専用)/ `setAlwaysFlushData`(CLI 専用)。

**仮想 zCoBo の最小 Ice 面(結論)**: 46001 に `HwNode`(CtrlNode 継承ツリーの ice_isA に
応答)+ `AlarmService` + デバイス servant 群(create で動的生成、identity = デバイス名)、
46004 に `DaqCtrlNode`(connect/disconnect/setCircularBuffersEnabled/daqStart/daqStop の 5 op)。
LedManager は no-op 可、AsAdPulserMgr は stopPeriodicPulser のみ必須(例外握り潰しあり)。

### 4.3 データリンクのライフサイクルとフレーム送出(レーン C — 完了)

**ライフサイクル(全てソース追跡済み)**:

```
configure ──▶ setDataLinks(XML パース) ──▶ daqConnect() = CoBo が TCP connect
              SystemManager.cpp:499-503        (ブロッキング・単発・リトライなし。
                                                失敗は即 ECC へエラー返却)
start  ──▶ daqStart = 割り込み監視開始(heartBeat 3000ms)。ソケット操作なし
stop   ──▶ daqStop  = 監視停止(最大 10 s 待ち)+ flushData。ソケットは開いたまま
breakup ──▶ disconnect = resetDataSender → DataSender 破棄 → shutdown(both)+close(正常 FIN)
(次の configure も先に旧接続を破棄してから新規 connect — 二重接続は残らない)
```

- 送出は**割り込み(データ到着)駆動**で「今溜まっている全バイト」を送る
  (`MemRead.cpp:220-263`)。固定周期のパケット化はしない。
- **3 秒ごとのハートビート flush**: 新規割り込みが無くても 3 s タイマーでバッファ端数を
  吐き出す(`DaqCtrlNodeI.cpp:407-410`, `MemRead.cpp:301-325`)。
- **背圧 = 完全ブロッキング**(`TcpDataSender.hpp:131-166`)。受信側が詰まれば送信が止まる。
  ドロップ機構は存在しない。**送信エラー時も自動再接続しない**(plain TCP。ログのみ —
  `MemRead.cpp:194-215`。自動再接続を持つのは GANIL FDT センダーだけ)。
- ソケットオプション: `SO_KEEPALIVE` 有効、**`TCP_NODELAY` は未設定(Nagle 有効)**、
  送信バッファは OS 既定。→ 小フレームは TCP 上で結合し得る(receiver のフレーミングは
  任意分割前提であること — 既存実装は満たしている)。
- `daqStop` の .ice コメント「closes data link」は**実装と食い違う**(閉じない)。
  `DataSender::stop()` の `isStopped_` フラグは**どこからも読まれない死んだフラグ**
  (grep 網羅確認)。→ コメントではなく実装が正、の再確認。
- DataLinkSet XML の正確な形: 属性名は **`type`**(コード内変数名 flowType と混同注意)、
  値は**大文字小文字区別の完全一致**(`"ICE"|"TCP"|"ZBUF"|"FDT"|"Debug"|"DebugFrames"` —
  `DaqCtrlNodeI.cpp:268-296`)。`type="TCP"` → `StdDataSender`(plain TCP)。
- `sendTopology` は FDT 専用。**plain TCP では daqStart 時に追加送信は無い**。

**要確認事項(⚠)**: ZC706 FW 資料(2018 Warsaw)の DataLinkSet 実例は**全て `type="FDT"`**
(GANIL Narval 運用)で、`type="TCP"` の実例はローカル資料に無い。「TCP 大文字」の根拠は
C++ 版 tpcdaq(ELI-NP mini 実運用)由来のはずで、**TCP パス自体はコードに実在**するため
矛盾ではないが、C++ 版 EccController の実運用値で裏取りするのが望ましい(§6 に登録)。

**仮想 zCoBo が再現すべき挙動(チェックリスト、各項ソース行参照はレーン C 報告どおり)**:
①connect は configure 末尾で CoBo 自身が 1 回だけ(リトライを勝手に足さない)
②configure 毎に旧接続破棄→新規 connect ③start/stop はソケット無関係(stop→start を
接続維持のまま繰り返せる)④stop は flush のみ・打ち切らない ⑤close は breakup/次 configure
のみ・正常 FIN ⑥送出はデータ駆動 + 3 s 端数 flush ⑦背圧でブロック(ドロップ禁止)
⑧エラー時自動再接続なし ⑨Nagle 有効・KEEPALIVE 有効 ⑩`type` 完全一致パース
⑪plain TCP に topology フレーム無し。

### 4.4 実 getHwServer の流用可能性(レーン D — 完了)

**結論: 実 getHwServer をそのまま流用できる。しかもハード無し動作は上流の設計に最初から
入っている。** 自作するのはデータ送出プロセスだけでよい見込み。

- **PC ビルドは既定でシミュレータ**: `getHwServer` のビルド(`GetBench/Makefile.am:484-521`)は
  `!VXWORKS && !ARM` 分岐でソースに **`SimServerDevices.cpp`** を使う(`ServerDevices.cpp` =
  実ハード版は VxWorks 分岐のみ)。SimServerDevices は `MemBus`/`AGetBus`/`AsAdBus`/`LocalMap`
  の全 registerAccess 種を**メモリ上のレジスタ**(`NestedStoragePolicy` = ただのメンバ変数、
  `ControllerDeviceSimulator` = std::map)で実装済み。実 CoBo の describe xcfg
  (`GetBench/data/config/hardwareDescription_fullCoBoStandAlone.xcfg`)が要求する
  registerAccess を**全て充足**する。
- **必要なのはビルドフラグ 1 個**: このスナップショットの configure は `--enable-cobo` 無し
  だったため getHwServer 自体は未生成(`GetBench/build/config.log`)。ただし**構造的に同一
  経路の `MDaq/build/hwServer` は x86_64 Linux バイナリとして実ビルド済み**の痕跡がツリー内に
  ある(ホスト daqula2)= このレシピが通る直接証拠。ソース改変は不要の見込み。
- **servant 構成**(`GetBench/src/GetHwServer.cpp:66-97`): 1 プロセス・1 アダプタに
  `HwNode`(CtrlNodeI、LedManager/AsAdPulser 同居)/ `MutantLinkManager` / `AlarmService` の
  3 identity。**既定ポート 46001**(`MDaq/src/mdaq/DefaultPortNums.h` — ecc=46002、
  dataRouterCtrl=46003、dataSenderCtrl=46004、dataFlow=46005)。
- **ハード依存は全て #ifdef で退化**(XGPIO → no-op、VxWorks task API → PC パスあり、
  Zedboard /dev/mem → ZEDBOARD 分岐のみ)。Qt/GUI 依存なし(grep 網羅)。
- **データ送出の切れ目は上流が既に引いている**: PC ビルドの getHwServer は `get/daq/*`
  (DaqCtrlNodeI・MemRead 等)を**一切リンクしない**(それらは VxWorks/ARM 分岐のみ)。
  VxWorks では GetHwServer.cpp:78-82 が **DaqCtrlNode の第 2 Ice サーバ(port 46004)**を
  起動するが、PC ではそのブロック自体が存在しない。
  → **帰結(重要)**: ECC はデータリンク操作(connect/daqStart/daqStop/disconnect)を
  **port 46004 の DaqCtrlNode servant** に向けて呼ぶ(§4.3)。PC 版 getHwServer にはこれが
  居ないので、**我々の graw リプレイ送出プロセスがこの DaqCtrlNode Ice 面を名乗って 46004 で
  listen する** — これが「仮想 zCoBo のうち自作する部分」の正確な定義になる。
- **ライセンス**: 調査した全ファイルが CeCILL ヘッダ(CEA)。third_party/ 隔離 + 表示の
  既定方針どおりで扱える。
- **残る不確実性(実走でしか閉じない)**: ECC の describe→configure 実行中に読み返される
  ステータス/レディビット類が、「書いた値がそのまま読める」だけの NestedStoragePolicy で
  ECC のポーリングを満足するか。実 FPGA が非同期に立てるビットがあると詰まる可能性。
  → レーン A の ECC 起動と組み合わせた**実 ecc 歩き(describe→…→start)での検証が必須**
  (一次資料主義: 静的読解で断定しない)。

### 4.5 実走検証の結果(039、2026-08-15 — 全成果物 `reference/_spike/run039/`)

**フルウォーク完走**: mini プロファイル(hardwareDescription **無改変**)で
Idle → Described → Prepared → Ready → Running → … → Idle 全遷移 `result: 0`。
自作は 46004 スタブ 120 行のみ。実測所要: describe 0.10 s / **prepare 4.85 s** /
**configure 2.21 s** / start 0.22 s — **run 開始レイテンシの下限 ≈ 7 s**(ECC 内部 sleep 支配)。

- **R2 解決**: シード必須は **4 書き込みだけ**(`PLG0`=1 / `monitorID`=0x41 / `VDD`=0xCD /
  `reg5`=0x201 — 1 レジスタで 4 チップ分)。AIM は ECC 自身が書く。温度・memStart・版数は
  チェックされない。**唯一の例外 = `asadStatus.alarm`**: power-off 後 1 / power-on 後 0 を
  同一ビットに要求 = 静的メモリでは原理的に両立不能(`checkPowerSupply=false` で回避可、
  ただし §5 改訂により不要になった — 下記裁定)。
- **R1 解決**: 外部シーダのポーリングは **5/5 敗北**(describe 0.10 s、割り込む窓は構造的に
  ゼロ)。失敗後リトライは 5/5 成功。**シードは describe 毎に消える**(destroy+create)が、
  prepare 再試行・breakup・reset(Prepared→Described)は跨いで生存。
- **46004 の実測契約**(§4.2 の机上表を上書きする一次データ): encoding 1.0(ワイヤ実測)/
  `ice_isA` は **configure 毎 1 回のみ**(DaqCtrlNode プロキシは ECC 側でキャッシュ —
  §4.2 の「アクセサ毎に再発行」は 46001 HwNode 側のみの話)/
  `connect("TCP", "127.0.0.1:46005")` — 第 2 引数は素の ip:port /
  **`daqStop` は breakup で 2 度目が来る = 冪等必須** / Prepared からの再 configure は
  `disconnect` 無しで `connect` = **旧センダ破棄は connect 側の責務** /
  46004 不在の configure は `PREPARED/WHEN_CONFIGURE` に健全着地 → 再発行で回復(ハング無し)。
- **prepare の設定読み込みは実測でも `configure-` プレフィクス**(SPEC v1.13 注記と一致)。

**裁定(Fable 2026-08-15)— 構成の改訂**: 039 が示したのは「外部シーダはレースに構造的に
勝てず、我々の run シーケンスは毎 run describe し直すためシードが毎回死ぬ」という事実。
race-free にする唯一の外置き案 = controller への post-describe フックは、実機で誤発火すると
**実レジスタに書く footgun** になるため不採用。よって **vcobo-daq が 46001
(HwNode/AlarmService/Device 面)も自分で名乗る** — 文字通り「板ごと偽る」に改訂する。
シーダは消滅(魔法値は servant に焼き込み)、`alarm` ビットは powerON 書き込みの追跡で
**正しく**モデル化でき(xcfg 無改変・checkPowerSupply=true のまま)、プロセスは 1 個減り、
R8(registerAccess=zCobo)も構造的に消える。実 getHwServer + シード経路は**検証済み
オラクル**として _spike に保持(vcobo-daq の 46001 実装は同じ ECC ウォークで照合できる)。

## 5. 設計方針(v1.1 — 039 の実測と裁定を反映)

### 5.1 プロセス構成 — 「実 ECC + vcobo-daq の 2 プロセス」(v1.1 改訂)

```
[実 getEccServer]──Ice(-e 1.0)──▶ ┌────────── vcobo-daq(自作)──────────┐
   ▲                              │ port 46001: HwNode/AlarmService/     │
   │                              │   Device 群(レジスタ = メモリ +      │
 Ice(enc 1.1)                     │   魔法値焼き込み + alarm=!powerON)   │
   │                              │ port 46004: DaqCtrlNode(5 op)       │
[ecc-bridge(既存・無変更)]        │ graw リプレイ送出 ──TCP──▶ receiver │
                                  └──────────────────────────────────────┘
```

1. **実 getEccServer** — `--config-repo-url` にフラットなデモ用 xcfg ディレクトリを渡すだけ。
   DB 不要。**制御の忠実度はここが担う**(状態機械・シーケンス・エラー文言・タイムアウトが
   実験と同一版)。
2. **vcobo-daq(自作、唯一の新規コード)** — 「板ごと偽る」を 1 プロセスで:
   - **46001: HwNode 面**(describe の `create(NodeConfig)` でデバイス servant 群を動的生成、
     `getMapOfDevices`、Device の 4 op(write/read × Register/Field)、AlarmService の
     reset/subscribe/unsubscribe、LedManager/AsAdPulserMgr は no-op)。レジスタは単純な
     メモリ格納 + **魔法値の焼き込み**(monitorID=0x41 / reg5=0x201 / VDD=0xCD / PLG=1 —
     039 実測の必須 4 種)+ **`asadStatus.alarm` = powerON 書き込みの追跡で反転**
     (= 実 FPGA の意味論を正しくモデル化。xcfg は checkPowerSupply=true のまま無改変)。
   - **46004: DaqCtrlNode 面**(§4.5 の実測契約どおり: connect は旧センダ自己破棄 +
     単発 TCP connect、daqStop 冪等、setCircularBuffersEnabled 保持)。
   - **データ送出**: §4.3 準拠の plain TCP(stop で close しない / 背圧ブロッキング /
     Nagle 有効 / 3 s 端数 flush)。データソースは**実 .graw のリプレイ**(バイト列は本物)。
   - どちらの servant も **encoding 1.0 のクライアント(実 ECC)を受ける**。
3. **我々の本体(controller / ecc-bridge / receiver 以降)は 1 行も変えない** — これが
   フォーク B の存在意義。ecc-bridge の接続先を fake_ecc から実 ECC に切り替えるだけ。
4. **実 getHwServer + CLI シード経路(039 で完走実証済み)は開発オラクル** — vcobo-daq の
   46001 実装は「同じ ECC ウォークが同じ結果になる」ことで照合する。

### 5.2 実装上の決定

- **言語と置き場**: vcobo-daq は C++ / `tools/vcobo/`(Ice が必須のため。ROOT/Ice は tools/
  に閉じ込める既定ルールどおり)。slice は `tools/ecc_bridge/slice/` の前例に倣い必要分のみ
  生成(`DaqControl.ice` / `hw/Control.ice` / `GetEcc.ice` 派生 — 同梱可否は ecc_bridge と
  同じ扱いの前例あり)。
- **encoding**: servant 側は Ice の仕様上 1.0/1.1 の両リクエストを受けられるが、
  **encoding 1.0 クライアント(実 ECC)相手の受信テストを明示的に持つ**こと。
- **ペーシング**: 設定で「実時間風(固定 Hz、既定 ~100 Hz)」と「全速(031 負荷ハーネス
  モード)」を切替。3 s 端数 flush(§4.3-⑧)も再現。
- **設定駆動**: AsAd 数(mini 1 / ELITPC 4)・CoBo 数・graw ファイルセット・ペーシングは
  設定ファイル。ch 数焼き込み禁止の不変条件をここでも守る。
- **xcfg セット**: デモ用 describe / configure / hardwareDescription は
  `GetBench/data/config/` の実例(describe-mono-node / fullCoBoStandAlone)から派生。
  **当面はローカル(reference/ 参照)で運用し、リポに入れない**(CeCILL 由来 + 実験設定は
  内部情報の疑い。リポ同梱は third_party/ 整理とあわせて後日判断 — §6-R6)。
- **fake_ecc は廃止しない**: CI の e2e はこれまでどおり fake_ecc(実機準拠化済み)。
  仮想 zCoBo スタックは**対話デモ・開発・負荷用**であり、CI 必須依存にしない
  (ビルド済み GET バイナリを CI に要求しないため)。

### 5.3 ELI-NP 実機テストへの接続(出張の成果物として)

この構成の最大の価値: **実機テスト初日に「ソフト側は全部検証済み」の状態で臨める**。
実機で初めて本物になるのは ①getHwServer が実レジスタを叩くこと(=我々からは不可視)
②データが FW 由来であること ③タイミングの実測、の 3 点だけに絞られる。
032/036 の現地確認項目(`extra_connections`/`peer`/`ecc_walk_back`/所要時間)は
仮想スタックで毎日リハーサル可能になる。

## 6. リスク・未決事項

| # | 内容 | 解消手段 |
|---|---|---|
| R1 | ~~シーダの describe→prepare 窓のレース~~ **解消(2026-08-15、039 実測 → §4.5 裁定)**: 外部シーダは構造的に敗北(5/5)、リトライは決定的に成功(5/5)。**構成改訂で問題自体が消滅**(vcobo-daq が 46001 を名乗り、魔法値は servant に焼き込み) | 閉じた(§4.5) |
| R2 | ~~Sim の read-back 意味論の落とし穴~~ **解消(2026-08-15、039 実測)**: シード必須は 4 値のみ、AIM は ECC 自身が書く。唯一の原理的例外 `asadStatus.alarm`(同一ビットに矛盾要求)は、自作 servant では **powerON 追跡で正しくモデル化**でき xcfg 無改変で通せる | 閉じた(§4.5) |
| R3 | ~~macOS ビルド成立性~~ **解消(2026-08-14 レーン A)**: macOS 成立。パッチ 2 行、Ice 3.7 keg(unlinked)。Docker は「ほぼ確実 + パッチ不要」の見立て | 閉じた(§4.1) |
| R4 | ~~`type="TCP"` の実運用実例が ZC706 資料に無い(全例 FDT)~~ **解消(2026-08-14)**: C++ 版 tpcdaq の `EccController::Config` 既定値が `flow_type = "TCP"`(「大文字 TCP 必須、factory は case-sensitive」の注釈付き — `~/test/get/tpcdaq/include/tpcdaq/control/ecc_controller.hpp:33`)。ELI-NP mini 実運用値で確定。ZC706 の FDT 例は 2018 Warsaw の GANIL Narval 運用文脈 | 閉じた |
| R5 | ~~Ice 版数差~~ **解消(2026-08-14 レーン A)**: ECC ローカルビルド = 3.7.11 keg、我々の ecc_bridge = 3.8.2、**実通信で wire 互換を実証**(encoding 1.1)。なお CLAUDE.md の「Ice 3.6.3」は要更新(手元実態は 3.8 系) | 閉じた(§4.1) |
| R8 | ~~ELITPC プロファイルの registerAccess=zCobo が Sim に無い~~ **構造的に解消(v1.1 構成改訂)**: vcobo-daq の 46001 servant は registerAccess 名を見ない(任意の NodeConfig を受けてメモリ格納)。実 getHwServer 経路(オラクル)に限る話に縮退 | 閉じた(§4.5 裁定の副産物) |
| R9 | **SPEC ギャップ(レーン A の発見)**: 実 ECC の ConfigId は **describe/prepare/configure の 3 組**で、実運用は別名を使う(例: `describe=zCobo-ZC706, configure=pulser`)。**正確化(2026-08-15)**: ecc-bridge JSON は既にアクション毎 `config_id` を取れる — 単一 id の焼き込みは **controller 設定(config.rs `config_id: String`)・run シーケンス・logbook run_start** にある | **裁定(Fable 2026-08-14、15 正確化)**: controller 設定を「文字列(3 相同値の略記)または {describe, prepare, configure} テーブル」の両対応にし、run シーケンスが相ごとの id を渡す。logbook run_start は非同値時のみ `config_ids` オブジェクトを追加(nullable 規律に整合)。**SPEC v1.13 として適用、実装は 042**。なお仮想 zCoBo デモ自体は xcfg 名を揃えれば単一 id で回るため 040/041 をブロックしない |
| R6 | ライセンス・内部情報: GET 由来 xcfg・ビルド成果物・FW 情報はリポ持ち込み禁止のまま運用開始。将来 CI に入れたくなったら third_party/ 整理 | 運用ルールで回避(当面リポ外) |
| R7 | ~~ECC の応答不能時の見え方~~ **観測完了(2026-08-15、041 D-1/D-2)**: vcobo 不在の run/start は 0.04 s で健全失敗 + next_run 巻き戻り + 全コンポーネント Idle 復帰(ECC 着地は `WHEN_DESCRIBE` — v1.1 構成の帰結)。ただし **ECC のエラーフラグが /api/status に出ない**(→ SPEC v1.14 / TODO/043)。CoBo 突然死は normal クローズに化ける(→ SPEC v1.14 §9.2 注記 / 033 追記) | 閉じた(残件は 043/033 へ移管) |

## 7. チケット分割案(詳細起票は 1 フェーズ先ルールに従う)

- **038(本スパイク)**: 本日 4 レーンで消化・完了(→ archive)。ビルド成果と再現スクリプトは
  `reference/_spike/`。**実 ECC ↔ getHwServer の接続(describe の入口)まで実証済み**。
- **039 — 実 ECC 歩きの実走検証: 完了(2026-08-15)** — フルウォーク完走、R1/R2 実測クローズ、
  §4.5 に反映。設計裁定(46001 も自作)を誘発。
- **040 — vcobo-daq 実装: 完了(2026-08-15)** — `tools/vcobo/` 2,764 行。オラクル照合
  8/8 遷移 green(**xcfg 完全無改変**)、テスト 161 本 green、バイト一致(合成 + 実 30 MB)。
- **041 — 統合デモ: 完了(2026-08-15)** — 実 .graw で run 3 本(graw sha256 一致、
  TTree 108 entries = P1 オラクル、WS 実測照合)、**実 ECC 歩き戻しの初実証**、
  listen-before-start の機械実証(非既定ポート)、異常系 2 種観測。
  run/start ≈ 7 s(実 ECC 支配)/ run/stop 5.7 s(033-E で秒未満化の見込み)。
- **043 — ECC エラーフラグ可視化(起票済み・READY)**: 041 発見② → SPEC v1.14。
  **P4(Run 制御 UI 実配線)の前提**。
- **この後**: Run 制御 UI の disabled 解除 = P4 チケット群(次の起票対象)。
  031 負荷ハーネスは vcobo-daq の全速モードをソースにできる。
- **042 — ConfigId 3 相化(SPEC v1.13): 完了(2026-08-15)** — リポゲート 402 → 415 passed。
