CXX = clang++
CXXFLAGS = -std=c++17 -O2 -Wall -Wextra
LDFLAGS = -framework IOKit

TARGET = dt_delay_test
SRCS = dt_delay_test.cpp

all: $(TARGET)

$(TARGET): $(SRCS) nlink_utils.hpp
	$(CXX) $(CXXFLAGS) -o $@ $(SRCS) $(LDFLAGS)

clean:
	rm -f $(TARGET)

run: $(TARGET)
	./$(TARGET) -m /dev/cu.wchusbserial585C0089431 -s /dev/cu.wchusbserial5AB50010561 --slave-id 0 -i 10 -c 100
