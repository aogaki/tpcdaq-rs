# tools/root_sink/Makefile — root-sink(TODO/008 取り込み骨格 + 010 イベントビルダ
#                             + 011 GDataFrame TTree 書き出し)。
#
#   make            root_sink をビルド(libzmq + ROOT + TPCReco 参照)
#   make test       **ROOT 非依存**の単体テスト 5 本をビルドして実行
#   make test GOLDEN=/path/to/golden_stream.bin
#                   適合性テストにフィクスチャを渡す(未指定なら SKIP)
#   make test-root  Recorder / PEventTPC のテスト(ROOT リンク)—— test とは別立て
#   TPCDAQ_REAL_PEVENT=/path/PEventTPC_....root make test-root
#                   実機オラクルとの構造一致も見る(未指定なら SKIP)
#   make compare    比較ツール(compare_gdataframe / compare_pevent)
#   make clean
#
# 純ヘッダ(tpc_wire.hpp / rs_core.hpp / eb_core.hpp)は ZMQ 非依存・**ROOT 非依存** ——
# `make test` の 4 本は g++ 一発で通る(ヘッダテストに ROOT を混ぜない、TODO/011 §3)。
# ROOT が要るのは `make`(本体)と `make test-root` だけ。
#
# **NDEBUG を定義しないこと**: closed channel への push の assert(SPEC §6.2-7)を
# 殺さないため。-O2 は assert を残す。

CXX      ?= g++
CXXFLAGS ?= -std=c++17 -O2 -g -Wall -Wextra
LDFLAGS  ?=

ZMQ_CFLAGS := $(shell pkg-config --cflags libzmq)
ZMQ_LIBS   := $(shell pkg-config --libs libzmq)

# ROOT 6.36(/opt/ROOT)。PATH に無ければ /opt/ROOT/bin/root-config を使う。
ROOT_CONFIG ?= $(shell command -v root-config 2>/dev/null || echo /opt/ROOT/bin/root-config)
ROOT_CFLAGS := $(shell $(ROOT_CONFIG) --cflags)
ROOT_LIBS   := $(shell $(ROOT_CONFIG) --glibs)
ROOTCLING   ?= $(shell dirname $(ROOT_CONFIG))/rootcling

# TPCReco 由来クラス(PEventTPC / eventraw::EventInfo)。**ライセンス無指定 =
# all rights reserved なのでコピーしない** —— ビルド時参照(SPEC §6.4/§6.6 v1.8、
# TODO/020)。別スナップショットを使いたければ環境変数で差し替える:
#
#   TPCDAQ_TPCRECO_DIR=/path/to/TPCReco make
#
# **コピー元は HIGS2026_online 固定**が既定 —— myChargeArray が出入りした他
# スナップショットのヘッダを掴むと streamer checksum が割れて実機ファイルと非互換
# になる(test_pevent.cxx が checksum を固定して検出する)。
TPCDAQ_TPCRECO_DIR ?= ../../reference/TPCReco/TPCReco-HIGS2026_online
TPCRECO_DIR  := $(TPCDAQ_TPCRECO_DIR)
TPCRECO_INC  := -I$(abspath $(TPCRECO_DIR)/DataFormats/include) \
                -I$(abspath $(TPCRECO_DIR)/Utilities/include)
# リンクに要る最小集合。PEventTPC.cpp(AddValByStrip/Clear)が `operator<<` 経由で
# EventInfo.cpp を、EventInfo.cpp が `eventAbsoluteTime` 経由で RunIdParser.cpp を、
# それが CoBoClock.cpp(static epoch)を引く —— 呼ばなくてもリンカは解決を要求する。
TPCRECO_SRC  := $(TPCRECO_DIR)/DataFormats/src/PEventTPC.cpp \
                $(TPCRECO_DIR)/DataFormats/src/EventInfo.cpp \
                $(TPCRECO_DIR)/Utilities/src/RunIdParser.cpp \
                $(TPCRECO_DIR)/Utilities/src/CoBoClock.cpp
TPCRECO_DICT := tpcrecodict.cxx
TPCRECO_HDR  := $(TPCRECO_DIR)/DataFormats/include/TPCReco/PEventTPC.h

# boost(CoBoClock.h が boost/date_time を include する。ヘッダのみ = リンク不要)。
# Homebrew / /usr/local / システムの順に探す。見つからなければ空 —— `tpcreco-check`
# が明示メッセージで止める(黙ってコンパイルエラーの山を出さない)。
BOOST_INC_DIR ?= $(shell for d in /opt/homebrew/include /usr/local/include /usr/include; do \
                   if [ -f $$d/boost/date_time/posix_time/posix_time.hpp ]; then echo $$d; break; fi; done)
BOOST_CFLAGS  := $(if $(BOOST_INC_DIR),-I$(BOOST_INC_DIR),)

# GET 由来クラス(CeCILL)。**無改変**でここにコピーしてある — third_party/get/README.md。
GET_DIR  = ../../third_party/get
GET_SRC  = $(GET_DIR)/GDataFrame.cpp $(GET_DIR)/GFrameHeader.cpp \
           $(GET_DIR)/GDataChannel.cpp $(GET_DIR)/GDataSample.cpp
GET_HDR  = $(GET_DIR)/GDataFrame.h $(GET_DIR)/GFrameHeader.h \
           $(GET_DIR)/GDataChannel.h $(GET_DIR)/GDataSample.h
GET_DICT = getdict.cxx

# 適合性フィクスチャ(run_conformance.sh が生成して渡す)。既定は「渡さない」= SKIP。
GOLDEN ?=

HEADERS   = tpc_wire.hpp rs_core.hpp eb_core.hpp check.hpp
TESTS     = test_tpc_wire test_rs_core test_eb_core test_conformance test_geo
ROOTTESTS = test_recorder test_pevent

.PHONY: all test test-root compare clean tpcreco-check

all: root_sink

# TPCReco 参照の存在確認。**黙って劣化させない**(TODO/020 §1)—— 見つからなければ
# ここで止め、何をすればよいかを出す。ROOT を使うターゲットの **order-only** 前提に
# 付けてある(毎回走るが、リンクをやり直させはしない)。
tpcreco-check:
	@if [ ! -f "$(TPCRECO_HDR)" ]; then \
	  echo "ERROR: TPCReco not found at $(TPCRECO_DIR)"; \
	  echo "  PEventTPC output (SPEC 6.4 v1.8) compiles against TPCReco headers."; \
	  echo "  TPCReco is NOT redistributable (license unspecified) so it is never"; \
	  echo "  copied into this repository - point the build at a local checkout:"; \
	  echo "    make TPCDAQ_TPCRECO_DIR=/path/to/TPCReco-HIGS2026_online"; \
	  echo "  (expected header: $(TPCRECO_HDR))"; \
	  exit 1; \
	fi
	@if [ -z "$(BOOST_INC_DIR)" ]; then \
	  echo "ERROR: boost headers not found (TPCReco/CoBoClock.h includes boost/date_time)."; \
	  echo "  Install boost (brew install boost) or pass the include dir:"; \
	  echo "    make BOOST_INC_DIR=/path/to/include"; \
	  exit 1; \
	fi

# ROOT 辞書。GDataFrame 系を TTree に書く(splitlevel 99)ために必須。
# 生成される getdict_rdict.pcm は実行時に要る —— 実行ファイルと同じ場所に置いておく
# (rootcling が絶対パスを埋めるので cwd には依存しない)。
#
# **-I は絶対パスで渡すこと**: rootcling はここで見たパスを辞書に焼き込むので、
# 相対パスだと cwd が違う実行(cargo test から起動される root_sink)で
# 「Missing FileEntry for GDataFrame.h」を出す。
$(GET_DICT): $(GET_HDR) $(GET_DIR)/LinkDefGET.h
	$(ROOTCLING) -f $@ -rmf libGET.rootmap -I$(abspath $(GET_DIR)) \
	  GDataSample.h GDataChannel.h GFrameHeader.h GDataFrame.h $(abspath $(GET_DIR))/LinkDefGET.h

# TPCReco 辞書(PEventTPC / eventraw::EventInfo)。**LinkDef は我々のもの**、
# クラス定義は TPCDAQ_TPCRECO_DIR から参照する(コピーしない)。
# GET 辞書と同じ理由で **-I は絶対パス**(rootcling がパスを辞書に焼き込む)。
$(TPCRECO_DICT): LinkDefTPCReco.h $(TPCRECO_HDR) | tpcreco-check
	$(ROOTCLING) -f $@ -rmf libTPCReco.rootmap $(TPCRECO_INC) $(BOOST_CFLAGS) \
	  TPCReco/PEventTPC.h $(abspath LinkDefTPCReco.h)

PEVENT_CFLAGS = $(TPCRECO_INC) $(BOOST_CFLAGS)
PEVENT_DEPS   = pevent_fill.hpp geo.hpp $(TPCRECO_DICT)

root_sink: root_sink.cxx tpc_wire.hpp rs_core.hpp eb_core.hpp root_recorder.hpp \
           $(PEVENT_DEPS) $(GET_DICT) $(GET_SRC) | tpcreco-check
	$(CXX) $(CXXFLAGS) $(ZMQ_CFLAGS) $(ROOT_CFLAGS) $(PEVENT_CFLAGS) -I$(GET_DIR) \
	  root_sink.cxx $(GET_DICT) $(GET_SRC) $(TPCRECO_DICT) $(TPCRECO_SRC) \
	  $(ZMQ_LIBS) $(ROOT_LIBS) $(LDFLAGS) -pthread -o $@

test_tpc_wire: test_tpc_wire.cpp tpc_wire.hpp check.hpp
	$(CXX) $(CXXFLAGS) $< -o $@

test_rs_core: test_rs_core.cpp rs_core.hpp check.hpp
	$(CXX) $(CXXFLAGS) -pthread $< -o $@

test_eb_core: test_eb_core.cpp eb_core.hpp rs_core.hpp tpc_wire.hpp check.hpp
	$(CXX) $(CXXFLAGS) $< -o $@

test_conformance: test_conformance.cpp tpc_wire.hpp check.hpp
	$(CXX) $(CXXFLAGS) $< -o $@

test_geo: test_geo.cpp geo.hpp check.hpp
	$(CXX) $(CXXFLAGS) $< -o $@

test_recorder: test_recorder.cxx root_recorder.hpp eb_core.hpp tpc_wire.hpp check.hpp \
               $(PEVENT_DEPS) $(GET_DICT) $(GET_SRC) | tpcreco-check
	$(CXX) $(CXXFLAGS) $(ROOT_CFLAGS) $(PEVENT_CFLAGS) -I$(GET_DIR) \
	  test_recorder.cxx $(GET_DICT) $(GET_SRC) $(TPCRECO_DICT) $(TPCRECO_SRC) \
	  $(ROOT_LIBS) $(LDFLAGS) -o $@

test_pevent: test_pevent.cxx root_recorder.hpp eb_core.hpp tpc_wire.hpp check.hpp \
             $(PEVENT_DEPS) $(GET_DICT) $(GET_SRC) | tpcreco-check
	$(CXX) $(CXXFLAGS) $(ROOT_CFLAGS) $(PEVENT_CFLAGS) -I$(GET_DIR) \
	  test_pevent.cxx $(GET_DICT) $(GET_SRC) $(TPCRECO_DICT) $(TPCRECO_SRC) \
	  $(ROOT_LIBS) $(LDFLAGS) -o $@

# GDataFrame TTree の全値比較(TODO/012、SPEC §12-3「比較スクリプトを tools/ に置く」)。
# 本体とは独立に使う道具なので `make` の既定ターゲットには入れない —— `make compare`。
compare_gdataframe: compare_gdataframe.cxx $(GET_DICT) $(GET_SRC)
	$(CXX) $(CXXFLAGS) $(ROOT_CFLAGS) -I$(GET_DIR) \
	  compare_gdataframe.cxx $(GET_DICT) $(GET_SRC) $(ROOT_LIBS) $(LDFLAGS) -o $@

# PEventTPC TTree の全値比較(TODO/020、SPEC §12-3 v1.8 の実データ照合に使う)。
compare_pevent: compare_pevent.cxx $(PEVENT_DEPS) | tpcreco-check
	$(CXX) $(CXXFLAGS) $(ROOT_CFLAGS) $(PEVENT_CFLAGS) \
	  compare_pevent.cxx $(TPCRECO_DICT) $(TPCRECO_SRC) $(ROOT_LIBS) $(LDFLAGS) -o $@

compare: compare_gdataframe compare_pevent

test: $(TESTS)
	./test_tpc_wire
	./test_rs_core
	./test_eb_core
	./test_conformance $(GOLDEN)
	./test_geo

test-root: $(ROOTTESTS)
	./test_recorder
	./test_pevent

clean:
	rm -f root_sink $(TESTS) $(ROOTTESTS) compare_gdataframe compare_pevent \
	      $(GET_DICT) getdict_rdict.pcm libGET.rootmap \
	      $(TPCRECO_DICT) tpcrecodict_rdict.pcm libTPCReco.rootmap
	rm -rf *.dSYM
