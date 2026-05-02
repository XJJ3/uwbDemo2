CXX = clang++
CXXFLAGS = -std=c++17 -O2 -Wall -Wextra
LDFLAGS = -framework IOKit

all: dt_delay_test dt_bidi_test

dt_delay_test: dt_delay_test.cpp nlink_utils.hpp
	$(CXX) $(CXXFLAGS) -o $@ dt_delay_test.cpp $(LDFLAGS)

dt_bidi_test: dt_bidi_test.cpp nlink_utils.hpp
	$(CXX) $(CXXFLAGS) -o $@ dt_bidi_test.cpp $(LDFLAGS)

clean:
	rm -f dt_delay_test dt_bidi_test
