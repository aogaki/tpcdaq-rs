// LinkDefTPCReco.h — PEventTPC / eventraw::EventInfo の ROOT 辞書生成指示(TODO/020)。
//
// **このファイルだけが我々のもの**。クラス定義そのもの(TPCReco)は
// `TPCDAQ_TPCRECO_DIR` から **ビルド時に参照**する —— TPCReco はライセンス無指定
// (= all rights reserved)なのでコピーを公開リポに置けない(SPEC §6.4 / §6.6 v1.8)。
// Warsaw の再配布許諾が得られたら `third_party/tpcreco/` へ昇格し NOTICE を付す。
//
// pragma は TPCReco `DataFormats/LinkDef.h` の該当 3 行と同じ(そこから我々が使う
// クラスだけを抜いたもの)。`nestedclasses` が無いと `EventInfo::global_properties`
// の辞書が出ず、splitlevel 2 のブランチが割れない。
//
// **streamer checksum は受け入れテストで固定**(test_pevent.cxx):
//   PEventTPC v1 0xf71c32cf / eventraw::EventInfo v1 0xfea093e4 /
//   eventraw::EventInfo::global_properties v1 0x49e6428c
// コピー元は HIGS2026_online 固定 —— myChargeArray が出入りした他スナップショットの
// ヘッダを掴むと checksum が割れて実機ファイルと非互換になる。

#ifdef __CINT__
#pragma link off all globals;
#pragma link off all classes;
#pragma link off all functions;
#pragma link C++ nestedclasses;

#pragma link C++ class PEventTPC+;
#pragma link C++ class eventraw::EventInfo+;
#pragma link C++ class eventraw::EventInfo::global_properties+;

#endif
