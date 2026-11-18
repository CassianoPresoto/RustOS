# Atalhos para o cargo e o `cargo xtask`. `make help` lista tudo.

ARCH ?= x86_64
PROFILE ?= dev
TARGETS := x86_64-unknown-none aarch64-unknown-none-softfloat
XTASK := cargo xtask

.DEFAULT_GOAL := help
.PHONY: help fmt fmt-check clippy layers-check comments-check docs-check check test \
        build user image run smoke smoke-fast smoke-panic screenshot font-preview initramfs size pr clean

help: ## lista os alvos
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "}; {printf "  %-15s %s\n", $$1, $$2}'
	@echo "  variáveis: ARCH=x86_64|aarch64|all  PROFILE=dev|release|release-small"

fmt: ## formata o código
	cargo fmt --all

fmt-check: ## confere a formatação
	cargo fmt --all --check

clippy: ## lint no host e nos dois alvos bare-metal (kernel com e sem `selftest` e as features de depuração; `user/` e `arch/*-user`); aviso é erro
	cargo clippy --all-targets -- -D warnings
	cargo clippy --package rustos-shell --no-default-features --all-targets -- -D warnings
	cargo clippy --package rustos-user-rt --all-targets -- -D warnings
	@for target in $(TARGETS); do \
		for features in "" selftest debug-addrs,panic-tests selftest,debug-addrs,panic-tests; do \
			cargo clippy --package rustos-kernel --target $$target $${features:+--features $$features} -- -D warnings || exit 1; \
		done; \
		cargo clippy --package rustos-user-rt --package rustos-user-init --package rustos-user-test-big \
			--package rustos-arch-$${target%%-*}-user --target $$target -- -D warnings || exit 1; \
	done

layers-check: ## dependências entre camadas, crates externos, unsafe e asm no lugar certo
	$(XTASK) check-layers

comments-check: ## comentários de no máximo 4 linhas e DEBT-NNNN que existe
	$(XTASK) check-comments

docs-check: ## índices, frontmatter das dívidas e links de docs/
	$(XTASK) check-docs

check: fmt-check clippy layers-check comments-check docs-check ## todos os gates (não sobe a QEMU)

test: ## testes de unidade no host (todos os crates menos o kernel), o shell sem o `bench` e o `user/rt`
	cargo test
	cargo test --package rustos-shell --no-default-features
	cargo test --package rustos-user-rt

build: ## compila o kernel
	$(XTASK) build --arch $(ARCH) --profile $(PROFILE)

user: ## compila os programas de `user/` (init e os de teste) em target/user/<arch>
	$(XTASK) user --arch $(ARCH) --profile $(PROFILE)

image: ## monta a partição EFI em target/esp/<arch>
	$(XTASK) image --arch $(ARCH) --profile $(PROFILE)

run: ## sobe na QEMU com a serial no terminal (Ctrl-A X sai)
	$(XTASK) run --arch $(ARCH) --profile $(PROFILE)

smoke: ## sobe na QEMU no modo de teste (autotestes e benchmarks), espera "[rustos] boot ok" e sai
	$(XTASK) smoke --arch $(ARCH) --profile $(PROFILE)

smoke-fast: ## o mesmo no modo normal (o boot do usuário), com o tempo de boot do kernel
	$(XTASK) smoke-fast --arch $(ARCH) --profile $(PROFILE)

smoke-panic: ## derruba o kernel de propósito e confere que o pânico aparece (cedo, console preso, aninhado)
	$(XTASK) smoke-panic --arch $(ARCH) --profile $(PROFILE)

screenshot: ## sobe sem janela, digita CMDS ("fetch;mem") no shell e grava a tela em PNG
	$(XTASK) screenshot --arch $(ARCH) --profile $(PROFILE) -- "$(CMDS)"

font-preview: ## desenha todos os glifos da fonte própria em target/font-preview.png
	$(XTASK) font-preview

initramfs: ## empacota FILES (`[nome=]arquivo ...`) em OUT ou target/initramfs/<isa>.cpio, sem o `cpio` do host
	$(XTASK) initramfs --arch $(ARCH) $(if $(OUT),--out $(OUT)) $(FILES)

size: ## tamanho das seções do kernel
	$(XTASK) size --arch $(ARCH) --profile $(PROFILE)

pr: check test ## checklist antes do PR: gates, testes, smoke, smoke-fast e smoke-panic nas duas ISAs, mais a imagem enxuta
	$(XTASK) smoke --arch all --profile $(PROFILE)
	$(XTASK) smoke-fast --arch all --profile $(PROFILE)
	$(XTASK) smoke-fast --arch all --profile release-small
	$(XTASK) smoke-panic --arch all --profile $(PROFILE)
	$(XTASK) smoke-panic --arch all --profile release-small
	RUSTOS_ACCEL=tcg RUSTOS_CPU=a64fx $(XTASK) smoke --arch aarch64 --profile $(PROFILE)

clean: ## apaga target/ (inclusive o Limine baixado)
	cargo clean
