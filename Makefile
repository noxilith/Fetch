CARGO ?= cargo
PREFIX ?= /usr/local
PYTHON ?= python3

BIN := target/release/fetch

# Statically linked build (glibc linked in): about half the resident memory, a
# bigger file. Needs the static glibc libraries (libc.a; `glibc-static` on Fedora).
TRIPLE := $(shell rustc -vV | sed -n 's/^host: //p')
STATIC_BIN := target/static/$(TRIPLE)/release/fetch

all: $(BIN)

$(BIN): Cargo.toml $(wildcard src/*.rs src/gather/*.rs)
	$(CARGO) build --release

install: $(BIN)
	install -d $(DESTDIR)$(PREFIX)/bin
	install -m 755 $(BIN) $(DESTDIR)$(PREFIX)/bin/fetch

static:
	RUSTFLAGS="-C target-feature=+crt-static" $(CARGO) build --release --target $(TRIPLE) --target-dir target/static

install-static: static
	install -d $(DESTDIR)$(PREFIX)/bin
	install -m 755 $(STATIC_BIN) $(DESTDIR)$(PREFIX)/bin/fetch

clean:
	$(CARGO) clean

test: $(BIN)
	$(CARGO) test
	$(PYTHON) tests/posix.py

.PHONY: all install static install-static clean test
