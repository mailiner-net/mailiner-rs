build-classes:
	DIOXUS_CLASS_BUILD_PATH="$$PWD/css/classes.rs" cargo test --features "build-classes"
	cargo build
	cd css && cargo build

serve:
	~/.cargo/bin/dx serve

clear-buffer:
	echo -e -n "\\0033c" && tmux clear-history

watch-classes:
	cargo watch -w src/ -s "make clear-buffer && cargo rustc -- -Awarnings && make build-classes"

test-browser:
	wasm-pack test --chrome