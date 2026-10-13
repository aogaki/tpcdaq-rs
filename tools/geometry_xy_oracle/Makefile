# tools/geometry_xy_oracle/Makefile — TPCReco GeometryTPC を正とする strip 線分 / MatchCrossPoint
# のオラクル(TODO/125 レーン A-4)。流儀は tools/root_sink/Makefile と同じ:
# TPCReco のソースは**コピーしない**(ライセンス無指定)—— TPCDAQ_TPCRECO_DIR から直コンパイル。
#
#   make -j                          # 既定の TPCDAQ_TPCRECO_DIR を使う
#   make -j TPCDAQ_TPCRECO_DIR=/abs/path/to/TPCReco-HIGS2026_online
#   make fixtures                    # 合成ジオメトリの出力をリポのフィクスチャへ書き直す
#   ./geometry_xy_oracle GEOMETRY.dat SEGMENTS.json [MATCH_CASES.json]
#
# **~/Workspace/Higs/TPCReco/build のライブラリは使わない**(GeometryTPC.cpp が reference と
# 違うローカル版 — TODO/125)。ROOT 辞書は要らない(GeometryTPC は TObject ではない)。

CXX      ?= c++
CXXFLAGS ?= -std=c++17 -O2 -g -Wall -Wextra
LDFLAGS  ?=

ROOT_CONFIG ?= $(shell command -v root-config 2>/dev/null || echo /opt/ROOT/bin/root-config)
ROOT_CFLAGS := $(shell $(ROOT_CONFIG) --cflags)
ROOT_LIBS   := $(shell $(ROOT_CONFIG) --libs)

TPCDAQ_TPCRECO_DIR ?= ../../reference/TPCReco/TPCReco-HIGS2026_online
TPCRECO_DIR := $(TPCDAQ_TPCRECO_DIR)
TPCRECO_INC := -I$(abspath $(TPCRECO_DIR)/DataFormats/include) \
               -I$(abspath $(TPCRECO_DIR)/Utilities/include)
# GeometryTPC と、それがリンク時に要求する最小集合。
TPCRECO_SRC := $(TPCRECO_DIR)/DataFormats/src/GeometryTPC.cpp \
               $(TPCRECO_DIR)/DataFormats/src/StripTPC.cpp \
               $(TPCRECO_DIR)/DataFormats/src/GeometryStats.cpp \
               $(TPCRECO_DIR)/DataFormats/src/RunConditions.cpp \
               $(TPCRECO_DIR)/Utilities/src/UtilsMath.cpp
TPCRECO_HDR := $(TPCRECO_DIR)/DataFormats/include/TPCReco/GeometryTPC.h

# boost(MultiKey.h → PEventTPC.h → CoBoClock.h が boost/date_time を include する。ヘッダのみ)。
BOOST_INC_DIR ?= $(shell for d in /opt/homebrew/include /usr/local/include /usr/include; do \
                   if [ -f $$d/boost/date_time/posix_time/posix_time.hpp ]; then echo $$d; break; fi; done)
BOOST_CFLAGS  := $(if $(BOOST_INC_DIR),-I$(BOOST_INC_DIR),)

SYNTH_DAT   := ../../tests/fixtures/geometry_xy_synth.dat
FIXTURE_DIR := ../../tests/fixtures/geometry_xy_oracle

.PHONY: all fixtures clean tpcreco-check

all: geometry_xy_oracle

tpcreco-check:
	@if [ ! -f "$(TPCRECO_HDR)" ]; then \
	  echo "ERROR: TPCReco not found at $(TPCRECO_DIR)"; \
	  echo "  TPCReco is NOT redistributable (license unspecified) so it is never"; \
	  echo "  copied into this repository - point the build at a local checkout:"; \
	  echo "    make TPCDAQ_TPCRECO_DIR=/path/to/TPCReco-HIGS2026_online"; \
	  exit 1; \
	fi
	@if [ -z "$(BOOST_INC_DIR)" ]; then \
	  echo "ERROR: boost headers not found (TPCReco/CoBoClock.h includes boost/date_time)."; \
	  echo "    make BOOST_INC_DIR=/path/to/include"; \
	  exit 1; \
	fi

# `-include sscanf_guard.h`: GeometryTPC::LoadAnalog の sscanf スタック溢れの当て物(ヘッダ参照)。
# TPCReco のソースは書き換えない。
geometry_xy_oracle: geometry_xy_oracle.cxx sscanf_guard.h | tpcreco-check
	$(CXX) $(CXXFLAGS) $(ROOT_CFLAGS) $(TPCRECO_INC) $(BOOST_CFLAGS) -include sscanf_guard.h \
	  geometry_xy_oracle.cxx $(TPCRECO_SRC) $(ROOT_LIBS) $(LDFLAGS) -o $@

# 合成ジオメトリ(make_synth_geometry.py の出力)のオラクル JSON をフィクスチャへ書く。
fixtures: geometry_xy_oracle
	python3 make_synth_geometry.py > $(SYNTH_DAT)
	./geometry_xy_oracle $(SYNTH_DAT) $(FIXTURE_DIR)/segments.json $(FIXTURE_DIR)/match_cases.json

clean:
	rm -f geometry_xy_oracle
	rm -rf *.dSYM
