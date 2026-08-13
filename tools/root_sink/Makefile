# tools/root_sink/Makefile — root-sink(TODO/008 取り込み骨格 + 010 イベントビルダ
#                             + 011 GDataFrame TTree 書き出し)。
#
#   make            root_sink をビルド(libzmq + ROOT)
#   make test       **ROOT 非依存**の単体テスト 4 本をビルドして実行
#   make test GOLDEN=/path/to/golden_stream.bin
#                   適合性テストにフィクスチャを渡す(未指定なら SKIP)
#   make test-root  Recorder のテスト(ROOT リンク)—— test とは別立て
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
TESTS     = test_tpc_wire test_rs_core test_eb_core test_conformance
ROOTTESTS = test_recorder

.PHONY: all test test-root compare clean

all: root_sink

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

root_sink: root_sink.cxx tpc_wire.hpp rs_core.hpp eb_core.hpp root_recorder.hpp \
           $(GET_DICT) $(GET_SRC)
	$(CXX) $(CXXFLAGS) $(ZMQ_CFLAGS) $(ROOT_CFLAGS) -I$(GET_DIR) \
	  root_sink.cxx $(GET_DICT) $(GET_SRC) \
	  $(ZMQ_LIBS) $(ROOT_LIBS) $(LDFLAGS) -pthread -o $@

test_tpc_wire: test_tpc_wire.cpp tpc_wire.hpp check.hpp
	$(CXX) $(CXXFLAGS) $< -o $@

test_rs_core: test_rs_core.cpp rs_core.hpp check.hpp
	$(CXX) $(CXXFLAGS) -pthread $< -o $@

test_eb_core: test_eb_core.cpp eb_core.hpp rs_core.hpp tpc_wire.hpp check.hpp
	$(CXX) $(CXXFLAGS) $< -o $@

test_conformance: test_conformance.cpp tpc_wire.hpp check.hpp
	$(CXX) $(CXXFLAGS) $< -o $@

test_recorder: test_recorder.cxx root_recorder.hpp eb_core.hpp tpc_wire.hpp check.hpp \
               $(GET_DICT) $(GET_SRC)
	$(CXX) $(CXXFLAGS) $(ROOT_CFLAGS) -I$(GET_DIR) \
	  test_recorder.cxx $(GET_DICT) $(GET_SRC) $(ROOT_LIBS) $(LDFLAGS) -o $@

# GDataFrame TTree の全値比較(TODO/012、SPEC §12-3「比較スクリプトを tools/ に置く」)。
# 本体とは独立に使う道具なので `make` の既定ターゲットには入れない —— `make compare`。
compare_gdataframe: compare_gdataframe.cxx $(GET_DICT) $(GET_SRC)
	$(CXX) $(CXXFLAGS) $(ROOT_CFLAGS) -I$(GET_DIR) \
	  compare_gdataframe.cxx $(GET_DICT) $(GET_SRC) $(ROOT_LIBS) $(LDFLAGS) -o $@

compare: compare_gdataframe

test: $(TESTS)
	./test_tpc_wire
	./test_rs_core
	./test_eb_core
	./test_conformance $(GOLDEN)

test-root: $(ROOTTESTS)
	./test_recorder

clean:
	rm -f root_sink $(TESTS) $(ROOTTESTS) compare_gdataframe \
	      $(GET_DICT) getdict_rdict.pcm libGET.rootmap
	rm -rf *.dSYM
