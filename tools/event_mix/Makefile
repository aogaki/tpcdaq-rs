# tools/event_mix/Makefile — 実イベントの重ね合わせ(TODO/128)。ROOT のみ(TPCReco・ZMQ 不要)。
#
#   make        event_mix をビルド
#   make test   合成データのテスト(test_event_mix)をビルドして実行
#   make clean
CXX      ?= g++
CXXFLAGS ?= -std=c++17 -O2 -g -Wall -Wextra
ROOT_CONFIG ?= $(shell command -v root-config 2>/dev/null || echo /opt/ROOT/bin/root-config)
ROOT_CFLAGS := $(shell $(ROOT_CONFIG) --cflags)
ROOT_LIBS   := $(shell $(ROOT_CONFIG) --libs)

.PHONY: all test clean
all: event_mix

event_mix: event_mix.cxx event_mix.hpp
	$(CXX) $(CXXFLAGS) $(ROOT_CFLAGS) event_mix.cxx $(ROOT_LIBS) -o $@

test_event_mix: test_event_mix.cxx event_mix.hpp ../root_sink/check.hpp
	$(CXX) $(CXXFLAGS) $(ROOT_CFLAGS) test_event_mix.cxx $(ROOT_LIBS) -o $@

test: test_event_mix
	./test_event_mix

clean:
	rm -f event_mix test_event_mix
