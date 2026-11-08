# Badger — macOS build and install targets
#
# Usage:
#   make build    — compile release bundle (produces target/release/bundle/macos/Badger.app)
#   make install  — copy .app to /Applications (requires build to have run first)

.PHONY: build install

APP_NAME    := Badger
APP_BUNDLE  := target/release/bundle/macos/$(APP_NAME).app
INSTALL_DIR := /Applications

build:
	npm run tauri build

install:
	@if [ ! -d "$(APP_BUNDLE)" ]; then \
		echo "Error: $(APP_BUNDLE) not found — run 'make build' first"; \
		exit 1; \
	fi
	@if pgrep -x "$(APP_NAME)" > /dev/null 2>&1; then \
		echo "Error: $(APP_NAME) is running — quit it first, then re-run make install"; \
		exit 1; \
	fi
	@echo "Removing old $(APP_NAME).app from $(INSTALL_DIR)..."
	rm -rf "$(INSTALL_DIR)/$(APP_NAME).app"
	@echo "Copying $(APP_BUNDLE) to $(INSTALL_DIR)..."
	cp -r "$(APP_BUNDLE)" "$(INSTALL_DIR)/$(APP_NAME).app"
	@echo "Installed: $(INSTALL_DIR)/$(APP_NAME).app"
