PREFIX ?= /usr/local
CC ?= cc
CFLAGS ?= -O2

all: hello

hello: hello.c
	$(CC) $(CFLAGS) -o hello hello.c

install: hello
	mkdir -p $(DESTDIR)$(PREFIX)/bin
	cp hello $(DESTDIR)$(PREFIX)/bin/hello
	chmod 755 $(DESTDIR)$(PREFIX)/bin/hello

clean:
	rm -f hello

.PHONY: all install clean