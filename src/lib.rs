//! tpcdaq — GET ベース TPC DAQ の Rust コンポーネント群(共有ライブラリ)。
//! 正本: docs/SPEC_ja.md。

/// 設定スキーマ(TOML)の読込・検証。SPEC §3。
pub mod config;

/// ジオメトリ抽象(TPCReco `.dat` パーサ、ChannelRole 解決)。SPEC §4。(002 で実装)
pub mod geometry;

/// ZMQ メッセージ核(Message/Batch/Fragment、直列化)。SPEC §2。(003 で実装)
pub mod msg;

/// コマンドチャネル(JSON REQ/REP、ComponentState 遷移)。SPEC §2.6。(003 で実装)
pub mod command;

/// ZMQ 配線ヘルパ(有限 HWM 方針の単一チョークポイント)。SPEC §1.4。(003 で実装)
pub mod zmq_helper;

/// MFM フレーマ(バイトストリーム → フレーム境界)。純コア、IO なし。SPEC §2.4。(004 で実装)
pub mod framer;

/// CoBo フレームバイト列 → msg::Fragment デコーダ(純コア、IO なし)。SPEC §2.4。(004 で実装)
pub mod decode;

/// receiver コンポーネント(CoBo 毎の TCP 受信、never-stop)。SPEC §1.1/§1.3/§1.4。(006 で実装)
pub mod receiver;

/// graw-writer コンポーネント(AsAd 毎ファイル、バイト一致 append)。SPEC §1.1/§6.5/§7。(007 で実装)
pub mod graw_writer;

/// decoder コンポーネント(Fragments 単一ストリーム + EOS 集約)。SPEC §1.1/§2.3。(009 で実装)
pub mod decoder;

/// JSONL ログブック(単一書き手の追記器 + リーダ)。SPEC §9。(015 で実装)
pub mod logbook;

/// `tpcdaq_state.json` の `next_run` 永続化。SPEC §8.1。(015 で実装)
pub mod state;

/// controller コンポーネント(REST + run シーケンス + ログブック)。SPEC §8.1/§1.3。(016 で実装)
pub mod controller;

/// monitor コンポーネント(root-sink PUB 購読 → 表示変換 → WS 配信)。SPEC §5.3/§5.4/§10。(026 で実装)
pub mod monitor;
