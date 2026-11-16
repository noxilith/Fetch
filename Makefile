CARGO ?= cargo
PREFIX ?= /usr/local
PYTHON ?= python3

BIN := target/release/fetch

all: $(BIN)

$(BIN): Cargo.toml $(wildcard src/*.rs src/gather/*.rs)
	$(CARGO) build --release

install: $(BIN)
	install -d $(DESTDIR)$(PREFIX)/bin
	install -m 755 $(BIN) $(DESTDIR)$(PREFIX)/bin/fetch

clean:
	$(CARGO) clean

test: $(BIN)
	$(CARGO) test
	$(PYTHON) tests/posix.py

.PHONY: all install clean test
