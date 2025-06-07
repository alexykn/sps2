CC=gcc
CFLAGS=-O2 -Wall

hello: hello.c
	$(CC) $(CFLAGS) -o hello hello.c

install: hello
	mkdir -p $(DESTDIR)/bin
	cp hello $(DESTDIR)/bin/

clean:
	rm -f hello

.PHONY: install clean