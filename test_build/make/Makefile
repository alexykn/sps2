CC=gcc
CFLAGS=-O2 -Wall

hello: hello.c
	$(CC) $(CFLAGS) -o hello hello.c

install: hello
	mkdir -p $(DESTDIR)/test-make-1.0.0/bin
	cp hello $(DESTDIR)/test-make-1.0.0/bin/

clean:
	rm -f hello

.PHONY: install clean