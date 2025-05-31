CC = clang
CFLAGS = -Wall -Wextra -std=c11
TARGET = hello
SOURCE = hello.c
PREFIX ?= /usr/local

all: $(TARGET)

$(TARGET): $(SOURCE)
	$(CC) $(CFLAGS) -o $(TARGET) $(SOURCE)

install: $(TARGET)
	mkdir -p $(DESTDIR)$(PREFIX)/bin
	cp $(TARGET) $(DESTDIR)$(PREFIX)/bin/

clean:
	rm -f $(TARGET)

.PHONY: all install clean