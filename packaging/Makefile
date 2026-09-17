# Shared install logic. Every package format below uses this, so there's
# only one place that decides where files land.
#
#   make build            build release binaries
#   sudo make install     install to /usr/local (plain manual install)
#   sudo make uninstall   remove again
#
# Package builds override PREFIX and DESTDIR:
#   make install DESTDIR=/tmp/pkgroot PREFIX=/usr

PREFIX  ?= /usr/local
DESTDIR ?=

BINDIR     = $(DESTDIR)$(PREFIX)/bin
APPDIR     = $(DESTDIR)$(PREFIX)/share/applications
DOCDIR     = $(DESTDIR)$(PREFIX)/share/doc/hypr-xp-magnifier
LICENSEDIR = $(DESTDIR)$(PREFIX)/share/licenses/hypr-xp-magnifier

# Run from the project root (one level up from packaging/)
ROOT = $(CURDIR)/..

.PHONY: build install uninstall clean

build:
	cd $(ROOT) && cargo build --release

install:
	install -Dm755 $(ROOT)/target/release/hypr-xp-magnifier $(BINDIR)/hypr-xp-magnifier
	install -Dm755 $(ROOT)/target/release/magnifier-settings $(BINDIR)/magnifier-settings
	install -Dm644 $(CURDIR)/hypr-xp-magnifier.desktop $(APPDIR)/hypr-xp-magnifier.desktop
	install -Dm644 $(CURDIR)/magnifier-settings.desktop $(APPDIR)/magnifier-settings.desktop
	install -Dm644 $(ROOT)/README.md $(DOCDIR)/README.md
	install -Dm644 $(CURDIR)/COMPATIBILITY.md $(DOCDIR)/COMPATIBILITY.md

uninstall:
	rm -f $(BINDIR)/hypr-xp-magnifier
	rm -f $(BINDIR)/magnifier-settings
	rm -f $(APPDIR)/hypr-xp-magnifier.desktop
	rm -f $(APPDIR)/magnifier-settings.desktop
	rm -rf $(DOCDIR)

clean:
	cd $(ROOT) && cargo clean
