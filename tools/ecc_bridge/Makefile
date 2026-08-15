# tools/ecc_bridge/Makefile — ecc-bridge(C++/Ice)+ fake-ECC(TODO/017、SPEC §8.2/§8.3)。
#
#   make test       **Ice 非依存**の単体テストをビルドして実行(g++ 一発、常に走る)
#   make            ecc_bridge / fake_ecc / ecc_e2e_client をビルド(Ice + libzmq が要る)
#   make e2e        統合テスト run_ecc_e2e.sh(fake-ECC ↔ ecc_bridge ↔ ZMQ REQ)
#   make clean
#
# **Ice スタブは .ice から毎回生成し、コミットしない**(.gitignore)。`.ice` 自体も
# reference/ から**コピーしない** —— ビルド時にだけ参照する。参照パスは環境変数で渡す:
#
#   make TPCDAQ_ICE_DIR=$(cd ../.. && pwd)/reference/20190315_patched
#
# 未設定なら Ice を使うターゲットは**明示的に skip**(黙って劣化させない)。
# `make test` は Ice 無しでも走る —— 核(ecc_core.hpp / json_min.hpp)は Ice も ZMQ も知らない。

CXX      ?= g++
CXXFLAGS ?= -std=c++17 -O2 -g -Wall -Wextra
LDFLAGS  ?=

ZMQ_CFLAGS := $(shell pkg-config --cflags libzmq 2>/dev/null)
ZMQ_LIBS   := $(shell pkg-config --libs libzmq 2>/dev/null)

# ZeroC Ice(Homebrew = 3.8.2)。ICE_HOME / -DICE_PREFIX で上書き可。
ICE_PREFIX ?= $(shell for d in "$$ICE_HOME" /opt/homebrew/opt/ice /usr/local/opt/ice /usr/local /usr; do \
                if [ -n "$$d" ] && [ -f "$$d/include/Ice/Ice.h" ]; then echo $$d; break; fi; done)
SLICE2CPP  ?= $(shell command -v slice2cpp 2>/dev/null || \
                (test -x "$(ICE_PREFIX)/bin/slice2cpp" && echo $(ICE_PREFIX)/bin/slice2cpp))

# GET / MDaq の .ice(実験使用中と同一版 = reference/20190315_patched)。
TPCDAQ_ICE_DIR ?=
MDAQ_SRC := $(TPCDAQ_ICE_DIR)/MDaq/src
GET_SRC  := $(TPCDAQ_ICE_DIR)/GetBench/src

# GetEcc.ice(get::rc::StateMachine)とその include 閉包。
# 流用元 ~/test/get/tpcdaq/CMakeLists.txt の _slices と同じ集合。
SLICE_UNITS := mdaq/Ecc mdaq/hw/Control mdaq/hw/HardwareTypes mdaq/utl/CmdException \
               get/GetEcc get/cobo/LedManager get/cobo/MutantLinkManager \
               get/mt/AlarmService get/mutant/AlignmentServer
SLICE_CPP   := $(addprefix slice/,$(addsuffix .cpp,$(SLICE_UNITS)))
SLICE_OBJ   := $(SLICE_CPP:.cpp=.o)

# Ice を使うターゲットを作れるか(3 条件が揃ったときだけ)。
ICE_OK := $(strip $(if $(TPCDAQ_ICE_DIR),$(if $(SLICE2CPP),$(if $(ICE_PREFIX),yes))))

ICE_CFLAGS := -I$(ICE_PREFIX)/include -Islice
ICE_LIBS   := -L$(ICE_PREFIX)/lib -lIce

HEADERS := ecc_core.hpp json_min.hpp check.hpp

# 049: ECC 状態機械の **共有 golden 遷移表**。Rust 側(src/controller.rs の
# `the_ecc_state_machine_matches_the_shared_golden_table`)が読むのと**同じ 1 ファイル**を
# test_ecc_bridge にも渡す —— 片側だけの修正(ドリフト)を落とすのが目的なので、
# ここを別ファイルに向けたらテストの意味が消える。
ECC_TRANSITIONS ?= ../../tests/fixtures/ecc_transitions.txt

.PHONY: all test e2e clean ice-skip

ifeq ($(ICE_OK),yes)
all: ecc_bridge fake_ecc ecc_e2e_client
else
all: ice-skip
endif

ice-skip:
	@echo "ecc_bridge: SKIP (Ice ビルドの前提が揃っていない)"
	@echo "  TPCDAQ_ICE_DIR = '$(TPCDAQ_ICE_DIR)'  (.ice の場所。例 <repo>/reference/20190315_patched)"
	@echo "  slice2cpp      = '$(SLICE2CPP)'"
	@echo "  Ice prefix     = '$(ICE_PREFIX)'"
	@echo "  例: make TPCDAQ_ICE_DIR=\$$(cd ../.. && pwd)/reference/20190315_patched"
	@echo "  (単体テストは Ice 不要: make test)"

# --- Ice スタブ生成 --------------------------------------------------------
# 相互 include があるので **全部まとめて 1 回**生成し、スタンプで依存を張る。
slice/.stamp:
	@if [ ! -f "$(GET_SRC)/get/GetEcc.ice" ]; then \
	  echo "ERROR: $(GET_SRC)/get/GetEcc.ice が無い (TPCDAQ_ICE_DIR を確認)"; exit 1; fi
	@for u in $(SLICE_UNITS); do \
	  dir=slice/`dirname $$u`; mkdir -p $$dir; \
	  case $$u in mdaq/*) root=$(MDAQ_SRC);; *) root=$(GET_SRC);; esac; \
	  echo "slice2cpp $$u.ice"; \
	  $(SLICE2CPP) -I$(MDAQ_SRC) -I$(GET_SRC) --output-dir $$dir $$root/$$u.ice || exit 1; \
	done
	@touch $@

$(SLICE_CPP): slice/.stamp

# 生成コードは我々のコードではないので警告を抑制(-w)。
slice/%.o: slice/%.cpp
	$(CXX) $(CXXFLAGS) -w -I$(ICE_PREFIX)/include -Islice -c $< -o $@

# --- 実行ファイル ----------------------------------------------------------
ecc_bridge: ecc_bridge.cpp $(HEADERS) $(SLICE_OBJ)
	$(CXX) $(CXXFLAGS) $(ICE_CFLAGS) $(ZMQ_CFLAGS) ecc_bridge.cpp $(SLICE_OBJ) \
	  $(ICE_LIBS) $(ZMQ_LIBS) $(LDFLAGS) -pthread -o $@

fake_ecc: fake_ecc.cpp $(HEADERS) $(SLICE_OBJ)
	$(CXX) $(CXXFLAGS) $(ICE_CFLAGS) fake_ecc.cpp $(SLICE_OBJ) \
	  $(ICE_LIBS) $(LDFLAGS) -pthread -o $@

# e2e クライアントは Ice を要らない(ZMQ REQ + 生 TCP listen だけ)。
ecc_e2e_client: ecc_e2e_client.cpp $(HEADERS)
	$(CXX) $(CXXFLAGS) $(ZMQ_CFLAGS) ecc_e2e_client.cpp $(ZMQ_LIBS) $(LDFLAGS) -pthread -o $@

test_ecc_bridge: test_ecc_bridge.cpp $(HEADERS)
	$(CXX) $(CXXFLAGS) $< -o $@

# --- テスト ----------------------------------------------------------------
test: test_ecc_bridge
	./test_ecc_bridge $(ECC_TRANSITIONS)

ifeq ($(ICE_OK),yes)
e2e: all
	./run_ecc_e2e.sh
else
e2e: ice-skip
endif

clean:
	rm -f ecc_bridge fake_ecc ecc_e2e_client test_ecc_bridge
	rm -rf slice *.dSYM
