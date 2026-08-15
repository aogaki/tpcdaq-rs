# 038 — 実 ECC ローカル起動の成立性スパイク(仮想 zCoBo フォーク B の前提調査)

**Status: COMPLETED**(2026-08-14 — Fable 主対話オーケストレーション + 4 サブエージェント並列。
**成果の正本 = [docs/VIRTUAL_ZCOBO_ja.md](../../docs/VIRTUAL_ZCOBO_ja.md) v1.0**)

## 結果

- **実行形態(逸脱 1 件・ユーザー裁定)**: 起票時は「起票のみ・実装なし」だったが、同日の
  ユーザー指示(「出張中に仕事を煮詰める最重要部分。docs 以下に徹底的に文章化して方針を
  固める。Fable を利用できる部分はガンガン」)により**スパイク実走に格上げ**。
  Fable が統合・設計判断・文書化、調査の脚は 4 レーン並列サブエージェント
  (A=ビルド/Opus、B=制御面/Opus、C=データ面/Sonnet、D=getHwServer 解剖/Sonnet)。
- **Q1 ビルド成立性 = macOS 成立**: 全 7 パッケージ、**実ソースパッチ 2 行**
  (Boost 1.90/1.87 起因)+ 環境設定。鍵は Ice 3.7 keg(3.7.11 arm64 bottle、unlinked —
  システム既定 3.8.2 と ecc_bridge は無影響を実証)。フルビルド ~10 分(-j8)、
  無人再現 = `reference/_spike/build_all.sh`。打ち切り基準には一度も接近せず。
  Linux/Docker は「ほぼ確実 + パッチ不要」の見立て。
- **Q2 起動成立性 = 成立 + 前倒し実証**: getEccServer 0.0.0.0:46002 listen /
  `sm-status` = IDLE / 実 xcfg で config-list / ハード無し describe は
  ConnectTimeout → `WHEN_DESCRIBE` の健全着地 / **我々の ecc_bridge(Ice 3.8.2)からの
  実通信成功**(status → `{"ok":true,"state":"Idle"}`、encoding 1.1、Ice 3.7↔3.8 wire 互換を
  実物で実証)/ **ボーナス: getHwServer(--enable-cobo、Sim デバイス)もビルドし、
  実 ECC ↔ getHwServer の接続(describe の "Added HW node")まで実証** = 039 の入口を前倒し。
- **Q3 Ice 面の棚卸し = 完了**(詳細は docs §4.2/4.3): **ECC→ハードは encoding 1.0**
  (`-e 1.0` 焼き込み — CLAUDE.md の 1.1 は ecc-bridge→ECC のレッグ)/ 最小 Ice 面 =
  46001 の HwNode+AlarmService+Device 群、**46004(ハードコード)の DaqCtrlNode 5 op** /
  チェック付き read-back 表(monitorID=0x41 等 — Sim は全ゼロ起動なのでシーダ必須、
  readOnly 非強制で書き込み可)/ タイムアウト事実上なし / Mutant 無しで不要な面 /
  データリンク挙動 11 項チェックリスト(stop で close しない・背圧ブロッキング・
  Nagle 有効・3 s 端数 flush 等)。
- **追加発見**: ①R8 = ELITPC プロファイルの `registerAccess=zCobo` は Sim 未登録
  (mini プロファイルは充足 — 当面のデモに影響なし)②**R9 = SPEC ギャップ: 実 ECC の
  ConfigId は 3 組**(describe/prepare/configure 別名運用が実在)— ecc-bridge JSON の
  単一 `config_id` では表現不能。**裁定済み**(3 相個別 id 対応 + 後方互換、SPEC diff は
  040 と同時)③運用の罠: Ice runtime 2 つ同居 → 無言 SIGSEGV / getHwServer は
  `--Ice.IPv6=0` 必須 ④小粒: ecc_bridge --help の既定 proxy が stale(`GetEcc:` →
  実 identity は `Ecc`。fake_ecc は正しい)。
- **検証・テスト**: 調査スパイクにつきリポのコード変更なし(cargo/C++ テスト対象外)。
  変更は md のみ。ビルドは `reference/_spike/` のコピーで完結し、
  **reference/ 正本 2 ディレクトリの無変更を find -newermt で検証済み**。
  R4(type="TCP" の実運用裏取り)は C++ 版 `ecc_controller.hpp:33` の既定値で同日クローズ。
- **実行環境・日付**: macOS Darwin 25.5.0(arm64)、2026-08-14。
  サブエージェント 4 本の所要 約 5〜47 分(ビルドレーンが最長)。
**起票**: 2026-08-14(ユーザー裁定「フォーク B 採用。実装は行わず起票だけ」)
**仕様**: SPEC への影響なし(調査スパイク)。ただし Q3 の結果は 039(仮想 zCoBo 本体)の
発注書の一次材料になる
**関連**: [CURRENT.md](CURRENT.md) デモ改良トラック /
[archive/036_ecc_walk_back.md](archive/036_ecc_walk_back.md)(実 ECC 状態機械の意味論)/
[archive/032_receiver_stale_link.md](archive/032_receiver_stale_link.md)(データリンクは
CoBo 側が configure で張る — `DaqCtrlNodeI.cpp` 追跡済み)/
CLAUDE.md「やらないこと」(制御プレーン改変禁止 — 本件は**ローカルコピーの起動**であり
実験系の改変ではない)

---

## 目的

デモ改良の目玉 = **graw ファイルをデータソースとする仮想 zCoBo(フォーク B: 実 ECC +
仮想 zCoBo)** の成立性を判定する材料を集める。**判定そのものは発注側(Fable/ユーザー)が
行う** — 本チケットは材料の持ち帰りまで。

B の価値: 実験と同一版の ECC バイナリがデモで毎日回る = 制御面の忠実度最大。
「テストダブルが実機より甘いと誤実装が green で通る」(036 の教訓)への構造的な答え。
唯一のブロッカーが「実 ECC がローカルでビルド・起動できるか」なので、それを先に潰す。

## やること(すべて調査。Rust/C++ 本体・SPEC に触らない)

### Q1 — ビルド成立性

- 対象: `reference/20190315_patched/` 内の **ECC サーバ**(eccserver / GetBench 系。
  **どのディレクトリ・ターゲットが ECC サーバ本体かの特定から**が仕事)。
- **macOS(このマシン)優先**。Ice 3.6.3 は ecc-bridge で使用中 = ツールチェーン導入済みを活用。
- macOS で不成立なら **Linux(Docker)で判定**。ELI-NP 持ち込み箱は現代 Linux なので、
  **「macOS 不可・Linux 可」でも B は成立扱い**(デモの走らせ方が Docker になるだけ)。
  どちらで成立したかを明記。
- ビルドを通すための**最小パッチは可**。条件: ①意味論を変えない ②パッチ全文を結果節に記録
  ③`reference/` 内で完結(成果物・パッチをリポに持ち込まない。third_party/ 昇格の判断は 039)。
- **深追い禁止(打ち切り基準)**: OS レベルの依存を 3 つ以上ソースから手ビルドする状況、
  またはビルドシステム自体の大改造が必要になったら**打ち切り**、ブロッカーを記録して
  持ち帰る(フォーク A へのフォールバック判断は発注側)。make は必ず `-j`。

### Q2 — 起動成立性(ビルドが通った場合のみ)

- ECC サーバの起動に何が要るか: 設定ファイル、設定リポジトリ(xcfg 置き場)、ポート、
  ロガー等。起動コマンドラインを記録。
- 我々の **ecc-bridge(Ice クライアント、encoding 1.1)から疎通するか** — describe 相当の
  最初の一往復が返れば十分。**ハード(getHwServer)不在でどこまで進み、どこでどう失敗するか
  の記録自体が 039 の材料**(仮想 zCoBo が何を提供すれば次へ進めるかの逆引き)。
  run まで回すことは求めない。

### Q3 — ECC → ハード側 Ice 面の棚卸し(**ビルド可否と独立に必ずやる** — 039 の発注書材料)

- **一次資料 = `reference/20190315_patched/` の実ソースのみ**。テストダブル
  (fake_ecc / ecc_core.hpp)のコメントを根拠にしない(036 で SPEC を誤りかけた教訓)。
- describe → prepare → configure → start / stop / breakup / reset の歩きで、ECC が
  ハード側に**実際に呼ぶ** Ice インターフェース・メソッドの一覧
  (.ice ファイルパス + 呼び出し元 .cpp の行参照つき)。呼ばれないものは列挙しない(KISS —
  仮想 zCoBo は呼ばれる面だけ実装する)。
- **ECC がハードをどう見つけるか**: エンドポイントの指定方法(describe xcfg のハードウェア
  ノード記述? 固定ポート?)。仮想 zCoBo が「どこで listen して何を名乗るか」を決める材料。
- **データリンク側の模倣対象の列挙**: CoBo が configure で receiver へ接続を張る実装箇所
  (032 の `DaqCtrlNodeI.cpp` 追跡を起点に)、start でのフレーム送出開始、stop で close
  しない挙動、breakup / resetDataSender での close。仮想 zCoBo が再現すべき挙動として
  ソース行参照つきで列挙。

### 非スコープ

- 仮想 zCoBo の設計・実装(→ 039。**本スパイクの結果を受けて Fable が起票**)。
- GET 由来コードのリポ取り込み・third_party/ 判断(→ 039)。
- ECC・FW の改変(Q1 の最小ビルドパッチを除く)。実験系(ELI-NP/Warsaw の稼働系)には
  一切触らない。

## 受け入れ

- 結果節に: ①ビルド可否 + 環境 + 完全な再現手順(パッチ全文)②起動可否 + 必要物 +
  ecc-bridge 疎通の記録 ③Q3 棚卸し表(Ice 面 / 発見機構 / データリンク挙動、全てソース行参照)
  ④ブロッカーと代替案(B 不成立時のみ)。
- `git status` クリーン(リポ内変更は本 md のみ。ビルドは reference/ 内 or scratch で完結)。
- **発注先想定**: implementer/**Opus**(ビルド泥沼のアドリブ + ソース読解の判断が要り、
  テストで縛れない)。Q3 のみ切り出して Sonnet 調査という分割も可(発注時に判断)。
