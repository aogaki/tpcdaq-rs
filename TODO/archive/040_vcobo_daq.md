# 040 — vcobo-daq(仮想 zCoBo 本体: HwNode + DaqCtrlNode + graw 送出)

**Status: COMPLETED**(2026-08-15 implementer/Opus → 発注側(Fable)レビュー PASS)

## 結果

- **オラクル照合 = 完全合格**: 実 getEccServer(20190315 版)相手にフルウォーク
  **8/8 遷移 `result: 0`**(describe→prepare→configure→start→stop→breakup→reset×2 →
  最終 `IDLE / NO_ERR`)。**configure xcfg は checkPowerSupply=true の完全無改変版**
  (`reference/_spike/run040/configs/` — run039 比 diff は当該 1 行のみ)。
  alarm=!powerON のモデル化が実 ECC の電源チェックを通過(`Powering on AsAd board no. 0`
  例外なし)。VDD=3.01254 V / AGET 0x201×4 を ECC ログで確認。daqStart 後 **46005 に
  204,800 B 到達**。**ウォーク中の WARN 0 件**(ECC が触った全レジスタが NodeConfig 宣言内)。
- **テスト**: `test_vcobo_core` **92 passed** / `vcobo_ice_client(full)`(**encoding 1.0** で
  46001+46004 全面 + stop→start + 背圧)**57 passed** / heartbeat(3 s 端数 flush)**6 passed** /
  replay(実 .graw 任意回帰)**6 passed** — 全 0 failed。`cargo test --lib` 264 passed =
  Rust 無影響。`make -j8` 警告 0(自作 TU は -Wall -Wextra)。
- **バイト一致**: 合成 2,060,288 B を ①通常 ②stop→start 後 ③背圧 1.5 s の 3 シナリオとも
  完全一致。実 .graw(109 フレーム / 30,108,684 B)完全一致・端数 0。
- **実装**: `tools/vcobo/` 計 2,764 行(コア = Ice/socket 非依存の `vcobo_core.hpp` に分離、
  ホットパス事前確保、§4.3 の 11 項を逐条実装 + ソース参照コメント)。Ice 3.7 keg /
  slice はビルド時生成・リポ非同梱。ビルド・テスト手順は Makefile(`test`/`ci`/`oracle`
  ターゲット、前提欠落は明示 skip)。
- **逸脱の裁定(発注側)**: ①**daqStart のリプレイ巻き戻し = 受理**(我々の controller は
  pause/resume 不使用 + v1.12 毎 run 完全リセットにより同一リンク再開はほぼ発生せず、
  毎回同一データはデモとして望ましい決定性)②ThreadPool=4(daqStop の実機準拠 10 s 待ちの
  巻き添え回避)③未知フィールド write は全レジスタ扱い + warn ④execBatch は CmdException
  (batchProcessing=false 固定 — 静かに壊れない)⑤設定は TOML 部分集合の自前パース
  (C++ tools に TOML 依存を足さない — 受理)。既知の限界(全ファイル事前ロード、glob 無し)は
  結果節記録のみで受理。
- 実行環境・日付: macOS Darwin 25.5.0(arm64)、Ice 3.7.11 keg、2026-08-15。
  reference/ 正本 2 ディレクトリ無変更(mtime 検証)。

---

(以下、起票時の発注書)

**Status(起票時): READY**(v2 確定 2026-08-15 Fable — 039 の実測と §4.5 裁定を反映して
スコープ改訂: **46001 の HwNode/Device 面も自作に含める**。シーダは消滅)
**仕様の正**: [docs/VIRTUAL_ZCOBO_ja.md](../docs/VIRTUAL_ZCOBO_ja.md) **v1.1** —
§5.1(構成)/ §4.2(46001 の最小 Ice 面 + 魔法値表)/ §4.5(46004 の実測契約 +
alarm 意味論)/ §4.3(データリンク挙動 11 項)
**オラクル**: `reference/_spike/run039/`(実 ECC + 実 getHwServer での完走ログ・スクリプト)
**発注先想定**: implementer/**Opus**

---

## 目的

仮想 zCoBo の本体 = **唯一の自作プロセス**。実 ECC(20190315 版)の配下で CoBo を
「板ごと」名乗る: 46001 で HwNode/Device/AlarmService、46004 で DaqCtrlNode、
DataLinkSet に従って receiver へ plain TCP を張り実 .graw をリプレイ送出する。

## 構成・ビルド

- 置き場: `tools/vcobo/`(C++17 可の範囲で。Ice は **3.7 keg**(`ice@3.7`、unlinked)を使う —
  **encoding 1.0 の実 ECC クライアントを受けることが実証済みのツールチェーン**
  (039 の `run039/stub/build_stub.sh` が動く実例。`-lIce` のみ、`-lIceUtil` は 3.7.11 keg に
  存在しない。3.7/3.8 runtime を 1 プロセスに混ぜると無言 SIGSEGV — docs §4.1 の罠)。
- slice: `tools/ecc_bridge` と同じ流儀 — **`TPCDAQ_ICE_DIR`(= reference/20190315_patched)から
  ビルド時に slice2cpp 生成**、生成物はリポに入れない(.gitignore)。必要 .ice:
  `GetBench/src/get/cobo/CtrlNode.ice`(継承ツリーごと — これを継承すれば `ice_isA` は自動)/
  `GetBench/src/get/mt/AlarmService.ice` / `MDaq/src/mdaq/hw/Control.ice` +
  `HardwareTypes.ice` / `MDaq/src/mdaq/DaqControl.ice`。
- Makefile: `make -j` 前提。`ICE_HOME`(既定 = ice@3.7 keg パス)と `TPCDAQ_ICE_DIR` を変数に。

## 機能仕様

### 1. 46001 — HwNode 面(identity `HwNode`、encoding 1.0 を受ける)

- `create(NodeConfig)`: 受け取った記述からデバイス servant 群を**動的生成**
  (identity = デバイス名: `ctrl`/`asad`/`aget`/`zeroSuppress*`/`pll`/…)。レジスタは
  メモリ格納(名前 → 64bit 値)、フィールドは NodeConfig の offset/width から
  ビット抽出・挿入。`destroy` で全破棄。`name`/`setName`/`getMapOfDevices` は
  docs §4.2 の describe シーケンスどおり。
- Device の 4 op: `writeRegister` / `writeField` / `readRegister` / `readField`。
  未知レジスタ/フィールドへの read は 0、write は受理して格納(**ただし warn ログ 1 回/名前** —
  silent failure 禁止)。`execBatch` は実装しない(batchProcessing=false 固定)。
- **魔法値の焼き込み(create 直後に設定)**: `asad`/`monitorID`=0x41、`aget`/`reg5`=0x201、
  `asad`/`VDD`=0xCD、`ctrl`/`asadConnection.PLG{0..3}`=1(039 実測の必須 4 種)。
- **`ctrl`/`asadStatus.alarm` の意味論**: `ctrl`/`asadEnable.powerON{i}` への write を追跡し、
  **alarm ビット i = !powerON{i}**(実 FPGA の意味論。これにより configure xcfg は
  `checkPowerSupply=true` のまま無改変で通る — 039 §4.5)。
- `AlarmService`(identity `AlarmService`): `reset`/`subscribe`/`unsubscribe` を受理
  (callback は呼ばない)。LedManager / AsAdPulserMgr の各 op は**受理して no-op**
  (`stopPeriodicPulser` は stop 毎に来る)。

### 2. 46004 — DaqCtrlNode 面(identity `DaqCtrlNode`、encoding 1.0)

039 の実測契約(docs §4.5)どおり:
- `connect(type, "ip:port")`: `"TCP"` 厳密一致のみ受理(他は例外)。**既存センダがあれば
  自分で破棄してから**新規 TCP connect(ブロッキング・単発・リトライ無し、失敗は例外で ECC へ)。
- `disconnect()`: shutdown(both)+close(正常 FIN)。
- `daqStart()`/`daqStop()`: ソケット無関係(送出の開始/停止 + flush)。**daqStop は冪等**
  (breakup で 2 度目が来る)。stop→start をリンク維持のまま繰り返せること。
- `setCircularBuffersEnabled(mask)`: AsAd マスクとして保持。

### 3. 送出エンジン(§4.3 の 11 項準拠)

- 実 .graw をフレーム単位に読み**バイト無変換**で送出。ペーシング: 固定 Hz(既定 ~100)/
  全速、設定で切替。**3 秒端数 flush** 再現。背圧 = ブロッキング(ドロップ禁止)。
  `SO_KEEPALIVE` 有効・`TCP_NODELAY` 設定しない。送信エラーで自動再接続しない(ログのみ)。
- ファイル群を送り切ったら送出静止(リンク保持)。ループ再生は設定で。

### 4. 設定(TOML か JSON、既存 tools の流儀に合わせる)

graw ファイルセット / CoBo id / AsAd 数(mini 1 / ELITPC 4)/ ペーシング(Hz・全速)/
listen(既定 0.0.0.0:46001 + 46004)/ ループ有無。**ch 数・AsAd 数の焼き込み禁止**。

## 受け入れ

- **CI 可能(必須)**: ①レジスタ/フィールド格納のユニット(ビット抽出・alarm=!powerON)
  ②encoding **1.0** の Ice テストクライアントで 46001(create→魔法値 read→
  powerON write→alarm read)と 46004(connect→start→stop→start→stop→stop→disconnect —
  冪等含む)を機械照合 ③TCP 受け側でリプレイ出力が**ソース .graw とバイト一致**
  (合成フィクスチャ)④stop→start のリンク維持 ⑤背圧でブロックしドロップしない。
- **オラクル照合(ローカル、env ゲート)**: `TPCDAQ_SPIKE_PREFIX` が指す実 ECC
  (`reference/_spike/prefix/bin/getEccServer` + `run039/configs/`)相手に、
  **039 と同じフルウォーク**(describe→prepare→configure→start→stop→breakup→reset×2)が
  全遷移 `result: 0` で完走すること。**configure xcfg は checkPowerSupply=true に戻した
  無改変版**を使う(alarm 意味論の実証)。046005 側は簡易 TCP リスナで受け、
  daqStart 後にデータが届くことまで確認。
- 実 .graw のバイト一致リプレイは `TPCDAQ_REAL_GRAW_DIR` の任意回帰として追加。
- `cargo fmt/clippy/test` 無影響(Rust 非接触)。C++ は `make -j` 警告なし。
- 結果節: テスト数と green / オラクル照合の遷移ログ / バイト一致実測 / 環境と日付。

## 非スコープ

- 我々の Rust チェーンとの統合 run(→ 041)/ Run 制御 UI 実配線(P4)/
  実 getHwServer 経路の保守(オラクルとして _spike に凍結)。
