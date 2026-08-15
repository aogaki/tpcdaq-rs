# tools/vcobo/Makefile — vcobo-daq(仮想 zCoBo 本体)。TODO/040、docs/VIRTUAL_ZCOBO_ja.md v1.1。
#
#   make test    **Ice 非依存**の単体テスト(g++ 一発。常に走る)
#   make         vcobo_daq / vcobo_ice_client / make_graw_fixture をビルド(Ice 3.7 が要る)
#   make ci      CI 可能な統合テスト(run_vcobo_ci.sh: 46001/46004 の機械照合 + バイト一致)
#   make oracle  実 ECC 相手のフルウォーク(TPCDAQ_SPIKE_PREFIX がある時だけ)
#   make clean
#
# **Ice は 3.7 keg(ice@3.7、unlinked)を使う** —— encoding 1.0 の実 ECC クライアントを
# 受けることが実証済みのツールチェーン(reference/_spike/run039/stub/build_stub.sh)。
# 3.7 と 3.8 の runtime を 1 プロセスに混ぜると **無言 SIGSEGV**(docs §4.1)。
# `-lIce` のみ(`-lIceUtil` は 3.7.11 keg に存在しない)。
#
# **Ice スタブは .ice から毎回生成し、コミットしない**(.gitignore)。`.ice` 自体も
# reference/ からコピーしない —— ビルド時にだけ参照する:
#
#   make TPCDAQ_ICE_DIR=$(cd ../.. && pwd)/reference/20190315_patched
#
# 未設定なら Ice を使うターゲットは**明示的に skip**(黙って劣化させない)。

CXX      ?= g++
CXXFLAGS ?= -std=c++17 -O2 -g -Wall -Wextra
LDFLAGS  ?=

# Ice 3.7 keg。ICE_HOME で上書き可。
ICE_PREFIX ?= $(shell for d in "$$ICE_HOME" /opt/homebrew/opt/ice@3.7 /usr/local/opt/ice@3.7; do \
                if [ -n "$$d" ] && [ -f "$$d/include/Ice/Ice.h" ]; then echo $$d; break; fi; done)
SLICE2CPP  ?= $(shell test -x "$(ICE_PREFIX)/bin/slice2cpp" && echo $(ICE_PREFIX)/bin/slice2cpp)

# GET / MDaq の .ice(実験使用中と同一版 = reference/20190315_patched)。
TPCDAQ_ICE_DIR ?=
MDAQ_SRC := $(TPCDAQ_ICE_DIR)/MDaq/src
GET_SRC  := $(TPCDAQ_ICE_DIR)/GetBench/src

# CtrlNode.ice の include 閉包 —— これを継承すれば ice_isA は自動で立つ(docs §4.2)。
SLICE_UNITS := mdaq/utl/CmdException mdaq/hw/HardwareTypes mdaq/hw/Control mdaq/DaqControl \
               get/mt/AlarmService get/cobo/LedManager get/cobo/CtrlNode
SLICE_CPP   := $(addprefix slice/,$(addsuffix .cpp,$(SLICE_UNITS)))
SLICE_OBJ   := $(SLICE_CPP:.cpp=.o)

ICE_OK := $(strip $(if $(TPCDAQ_ICE_DIR),$(if $(SLICE2CPP),$(if $(ICE_PREFIX),yes))))

ICE_CFLAGS := -I$(ICE_PREFIX)/include -Islice
ICE_LIBS   := -L$(ICE_PREFIX)/lib -lIce

HEADERS := vcobo_core.hpp vcobo_link.hpp check.hpp

.PHONY: all test ci oracle clean ice-skip

ifeq ($(ICE_OK),yes)
all: vcobo_daq vcobo_ice_client make_graw_fixture
else
all: ice-skip
endif

ice-skip:
	@echo "vcobo: SKIP (Ice ビルドの前提が揃っていない)"
	@echo "  TPCDAQ_ICE_DIR = '$(TPCDAQ_ICE_DIR)'  (.ice の場所。例 <repo>/reference/20190315_patched)"
	@echo "  slice2cpp      = '$(SLICE2CPP)'"
	@echo "  Ice 3.7 prefix = '$(ICE_PREFIX)'"
	@echo "  例: make TPCDAQ_ICE_DIR=\$$(cd ../.. && pwd)/reference/20190315_patched"
	@echo "  (単体テストは Ice 不要: make test)"

# --- Ice スタブ生成 --------------------------------------------------------
# 相互 include があるので **全部まとめて 1 回**生成し、スタンプで依存を張る。
slice/.stamp:
	@if [ ! -f "$(GET_SRC)/get/cobo/CtrlNode.ice" ]; then \
	  echo "ERROR: $(GET_SRC)/get/cobo/CtrlNode.ice が無い (TPCDAQ_ICE_DIR を確認)"; exit 1; fi
	@for u in $(SLICE_UNITS); do \
	  sub=`dirname $$u`; dir=slice/$$sub; mkdir -p $$dir; \
	  case $$u in mdaq/*) root=$(MDAQ_SRC);; *) root=$(GET_SRC);; esac; \
	  echo "slice2cpp $$u.ice"; \
	  $(SLICE2CPP) -I$(MDAQ_SRC) -I$(GET_SRC) --include-dir $$sub \
	    --output-dir $$dir $$root/$$u.ice || exit 1; \
	done
	@touch $@

$(SLICE_CPP): slice/.stamp

# 生成コードは我々のコードではないので警告を抑制(-w)。
slice/%.o: slice/%.cpp
	$(CXX) $(CXXFLAGS) -w -I$(ICE_PREFIX)/include -Islice -c $< -o $@

# --- 実行ファイル ----------------------------------------------------------
vcobo_daq: vcobo_daq.cpp $(HEADERS) $(SLICE_OBJ)
	$(CXX) $(CXXFLAGS) $(ICE_CFLAGS) -Wno-deprecated-declarations vcobo_daq.cpp $(SLICE_OBJ) \
	  $(ICE_LIBS) $(LDFLAGS) -pthread -o $@

vcobo_ice_client: vcobo_ice_client.cpp $(HEADERS) $(SLICE_OBJ)
	$(CXX) $(CXXFLAGS) $(ICE_CFLAGS) -Wno-deprecated-declarations vcobo_ice_client.cpp $(SLICE_OBJ) \
	  $(ICE_LIBS) $(LDFLAGS) -pthread -o $@

# 合成 .graw フィクスチャ生成(Ice 不要だが all で一緒に作る)。
make_graw_fixture: make_graw_fixture.cpp vcobo_core.hpp
	$(CXX) $(CXXFLAGS) make_graw_fixture.cpp -o $@

test_vcobo_core: test_vcobo_core.cpp $(HEADERS)
	$(CXX) $(CXXFLAGS) $< -o $@

# --- テスト ----------------------------------------------------------------
test: test_vcobo_core
	./test_vcobo_core

ifeq ($(ICE_OK),yes)
ci: all test
	./run_vcobo_ci.sh
oracle: all
	./run_vcobo_oracle.sh
else
ci: ice-skip
oracle: ice-skip
endif

clean:
	rm -f vcobo_daq vcobo_ice_client make_graw_fixture test_vcobo_core
	rm -rf slice *.dSYM
